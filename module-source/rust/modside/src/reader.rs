//! 화면 한 장 → 교역 창·도시 읽기. 감지기(gamji/collector.rs)의 규칙을 시세 업로드에 필요한 만큼만 옮겼다.
//!
//! - 교역 창이 보이면 4줄을 읽고, 연속 2번 같은 값만 인정한다 (감지기와 같음)
//! - 매각 창·시세 창만. 구입 창은 버린다. 길드 개척도시는 수집하지 않는다
//! - 교역 창이 없을 때 1.5초마다 왼쪽 위 지명을 읽어 도시를 안다
//! - 같은 값은 다시 알리지 않는다. 창을 계속 열어 두면 60초마다 보이는 값을 다시 알린다
//!
//! - 0.9.0 (2026-11-22 사용자 "인근 시세도 올려야지"): 시세 창에서 품목을 누르면 오른쪽 '인근 도시 시세' 5줄도 읽는다 (감지기와 같은 규칙:
//!   지금 도시·길드 개척도시는 빼고, 연속 2번 같은 값만, 가격 없이 %만). 지금 도시를 몰라도 인근 시세는 올릴 수 있다
//!
//! - 0.10.0 (2026-11-23 사용자 "교역소 대사, 남만무역 그래프 + 몇 틱 HUD, 방식은 exe 와 같아"): 감지기 collector.rs 의
//!   `news_step`(3초마다 채팅창의 교역소 주인·도제 대사, 대사별 보이는 횟수가 늘었을 때만)과
//!   `nanban_step`(무역 상인 '남만무역정보' 창: 도시 상태 + 고른 도시·분류의 점 11개, 자리 잡은 화면만 — nb.rs)을 옮겼다.
//!   남만 창이 열려 있으면 그 자리만 빠르게 읽는다 (교역 창 읽기 건너뜀). `tick` = 틱 HUD 자리 (캡처 좌표, 마지막 점 높이)
//!
//! 지금 범위 밖 (감지기에만 있음): 목록에 없는 도시, 캐릭터 별, 네비.
use crate::nb::NbTrack;
use recog::nanban::{NanbanReader, NanbanScreen, POINTS, WIN_W, tick_y};
use recog::text::is_guild_city;
use recog::{Pct, Recognizer, Rgb, TradeScreen};
use serde_json::{Value, json};
use std::collections::HashMap;

/// 교역소 대사를 읽는 간격 (감지기와 같음)
pub const NEWS_EVERY: f64 = 3.0;

pub const CITY_EVERY: f64 = 1.5;
pub const RESEND_SEC: f64 = 60.0;

/// 시세 창 '인근 도시 시세' 한 줄 (가격 없이 %)
#[derive(Clone, Debug, PartialEq)]
pub struct NearItem {
    pub city: String,
    pub good: String,
    pub pct: i64,
    pub tr: &'static str,
    pub arrow: Option<(String, String)>,
}

impl NearItem {
    pub fn json(&self) -> Value {
        json!({ "city": self.city, "pct": self.pct, "tr": self.tr })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub price: i64,
    pub pct: Pct,
    pub tr: &'static str,
    /// 화살표 원래 값 (종류 up|flat|down|vup|vdown, 색 #rrggbb) — 공유 서버에 그대로 올린다
    pub arrow: Option<(String, String)>,
}

impl Row {
    pub fn json(&self) -> Value {
        let pct = match &self.pct {
            Pct::Num(n) => json!(n),
            Pct::Tier(t) => json!(t),
        };
        json!({ "name": self.name, "price": self.price, "pct": pct, "tr": self.tr })
    }
}

/// 감지기 collector.rs `mark` 와 같은 표시
pub fn mark(a: &Option<(String, String)>) -> &'static str {
    match a.as_ref().map(|a| a.0.as_str()) {
        Some("up") => "↗",
        Some("flat") => "→",
        Some("down") => "↘",
        Some("vup") => "↑",
        Some("vdown") => "↓",
        Some(_) => "?",
        None => "",
    }
}

/// 화면 한 장을 읽은 결과로 생긴 일
#[derive(Debug, PartialEq)]
pub enum Event {
    /// 도시가 바뀜 (None = 도시 밖)
    City(Option<String>),
    /// 연속 2번 같게 읽힌 새 시세 (kind: "sell" | "market")
    Trade { kind: String, city: Option<String>, rows: Vec<Row> },
    /// 연속 2번 같게 읽힌 새 인근 도시 시세 (시세 창에서 고른 품목 하나)
    Near { good: String, items: Vec<NearItem> },
    /// 알릴 만한 상황 (같은 글은 창을 다시 열 때까지 한 번만)
    Note(String),
    /// 채팅창에 새로 나온 교역소 대사 (가장 최근이 마지막)
    News { city: String, texts: Vec<String> },
    /// 남만무역정보 창이 자리 잡은 화면 (같은 화면은 60초에 한 번). states = 목록 도시마다 상태('보통'은 '평상'),
    /// seen = 도시·분류를 새로 읽었다 (HUD '나가사키의 식료품 재고 인식')
    Nanban { culture: String, city: Option<String>, kind: Option<String>, pts: Option<[u8; POINTS]>, states: Vec<(String, String)>, seen: bool },
}

pub struct Reader {
    pub r: Recognizer,
    pub city: Option<String>,
    t_city: f64,
    pub t_trade: f64,
    prev: Option<Vec<Row>>,
    sent: Vec<Row>,
    prev_near: Option<Vec<NearItem>>,
    sent_near: Vec<NearItem>,
    t_resend: f64,
    noted: Option<String>,
    /// 바다 지명을 본 뒤 지명을 연속으로 못 읽은 횟수 (1.0.1, 감지기 '도시를 인식중입니다'와 같음)
    /// 바다 지명을 읽음 = 도시 밖 (1.0.2: 이때는 게임 화면 HUD 를 숨긴다)
    pub sea: bool,
    city_miss: u32,
    // ---- 교역소 대사 (0.10.0) ----
    /// 서버를 알 때만 읽는다 (main 이 정함, 감지기와 같음)
    pub news_on: bool,
    t_news: f64,
    news: HashMap<String, usize>,
    news_seq: Option<Vec<String>>,
    // ---- 남만무역정보 (0.10.0) ----
    nb: NanbanReader,
    nb_track: NbTrack,
    /// 남만 창이 열려 있는 자리 (창 모서리). 있으면 교역 창 읽기를 건너뛰고 빠르게 읽는다
    pub nb_corner: Option<(i64, i64)>,
    nb_last: Option<(NanbanScreen, f64)>,
    nb_seen: Option<(String, String)>,
    /// 틱 HUD: (캡처 x, 캡처 y = 마지막 점 높이, 틱). 남만 창이 자리 잡았을 때만 Some
    pub tick: Option<(i64, i64, u8)>,
}

impl Reader {
    pub fn new(r: Recognizer) -> Self {
        Self::with_icons(r, None)
    }

    /// icon_dir: 남만 도시 상태 아이콘을 새로 배우면 저장할 폴더
    pub fn with_icons(r: Recognizer, icon_dir: Option<std::path::PathBuf>) -> Self {
        Reader {
            r,
            city: None,
            t_city: 0.0,
            t_trade: 0.0,
            prev: None,
            sent: vec![],
            prev_near: None,
            sent_near: vec![],
            t_resend: 0.0,
            noted: None,
            sea: false,
            city_miss: 0,
            news_on: false,
            t_news: 0.0,
            news: HashMap::new(),
            news_seq: None,
            nb: NanbanReader::new(icon_dir),
            nb_track: NbTrack::default(),
            nb_corner: None,
            nb_last: None,
            nb_seen: None,
            tick: None,
        }
    }

    /// 대상 창이 바뀌면 읽던 것을 버린다 (도시도 다시 읽는다)
    pub fn reset(&mut self) {
        self.city = None;
        self.sea = false;
        self.t_city = 0.0;
        self.prev = None;
        self.sent.clear();
        self.prev_near = None;
        self.sent_near.clear();
        self.noted = None;
        self.t_news = 0.0;
        self.news.clear();
        self.news_seq = None;
        self.nb_reset();
    }

    fn nb_reset(&mut self) {
        self.nb_track = NbTrack::default();
        self.nb_corner = None;
        self.nb_last = None;
        self.nb_seen = None;
        self.tick = None;
    }

    pub fn take_log(&mut self) -> Vec<String> {
        let mut v = std::mem::take(&mut self.r.log);
        v.append(&mut self.nb.log);
        v
    }

    /// 화면 한 장. 교역 창(남만 창 포함)이 보였는지와 생긴 일을 돌려준다
    pub fn frame(&mut self, rgb: &Rgb, off: (i64, i64), t: f64) -> (bool, Vec<Event>) {
        let mut out = vec![];
        // 남만무역정보 창이 열려 있으면 그 자리만 빠르게 읽는다. 창이 닫히거나 움직이면 원래대로 (감지기와 같음)
        if let Some((cx, cy)) = self.nb_corner {
            if let Some(scr) = self.nb.read_fast(rgb, cx, cy) {
                self.t_trade = t;
                self.nb_step(scr, (cx, cy), t, &mut out);
                return (true, out);
            }
            self.nb_reset();
        }
        let trade = self.r.read_trade(rgb, off);
        let open = trade.is_some();
        match trade {
            None => {
                self.prev = None;
                self.sent.clear();
                self.prev_near = None;
                self.sent_near.clear();
                self.noted = None;
                self.nb_reset();
                self.t_resend = t;
                if t - self.t_city >= CITY_EVERY {
                    self.t_city = t;
                    self.city_step(rgb, off, &mut out);
                }
            }
            // 무역 상인 '남만무역정보' 창 (교역 창과 같은 모서리 틀, 품목 줄이 없고 그래프 세로축이 보임)
            Some(tr) if tr.rows.is_empty() && NanbanReader::looks_like(rgb, tr.corner.0, tr.corner.1) => {
                self.t_trade = t;
                let (cx, cy) = tr.corner;
                if let Some(scr) = self.nb.read(rgb, cx, cy) {
                    self.nb_corner = Some((cx, cy));
                    self.nb_step(scr, (cx, cy), t, &mut out);
                }
            }
            Some(tr) => {
                self.nb_reset();
                self.t_trade = t;
                self.trade_step(&tr, t, &mut out);
            }
        }
        // 교역소 대사: 서버·도시를 알고 길드 개척도시가 아니면 3초마다
        if self.news_on && t - self.t_news >= NEWS_EVERY && self.city.as_ref().is_some_and(|c| !is_guild_city(c)) {
            self.t_news = t;
            self.news_step(rgb, off, &mut out);
        }
        (open, out)
    }

    /// 채팅창에 새로 나온 교역소 대사만: 대사별로 보이는 횟수가 늘었을 때 (감지기 collector.rs `news_step` 과 같음).
    /// 채팅창에는 예전 도시의 대사도 남아 있어서, 횟수가 그대로인 대사는 지금 도시 것으로 알리지 않는다
    fn news_step(&mut self, rgb: &Rgb, off: (i64, i64), out: &mut Vec<Event>) {
        let seq = self.r.read_market_news(rgb, off);
        let mut cnt: HashMap<String, usize> = HashMap::new();
        for s in &seq {
            *cnt.entry(s.clone()).or_default() += 1;
        }
        let old = std::mem::take(&mut self.news);
        let mut uniq: Vec<String> = vec![];
        for s in &seq {
            if !uniq.contains(s) {
                uniq.push(s.clone());
            }
        }
        let mut news: Vec<String> = uniq.into_iter().filter(|s| cnt[s] > old.get(s).copied().unwrap_or(0)).collect();
        // 가장 최근에 나온 대사가 마지막 (지도에서 맨 위)
        news.sort_by_key(|s| seq.iter().rposition(|x| x == s).unwrap());
        // 같은 결과를 두 번 연속 읽었을 때만 횟수를 줄인다 (한 줄을 잠깐 놓쳤다가 다시 읽으면 새 대사로 착각하지 않게)
        self.news = if Some(&seq) == self.news_seq.as_ref() {
            cnt
        } else {
            let mut m = old.clone();
            for (k, v) in cnt {
                let e = m.entry(k).or_default();
                *e = (*e).max(v);
            }
            m
        };
        self.news_seq = Some(seq);
        if let (false, Some(c)) = (news.is_empty(), self.city.clone()) {
            out.push(Event::News { city: c, texts: news });
        }
    }

    /// 남만무역정보 창 (감지기 collector.rs `nanban_step` 의 읽기 부분). 올리기는 upload.rs
    fn nb_step(&mut self, scr: NanbanScreen, corner: (i64, i64), t: f64, out: &mut Vec<Event>) {
        if !self.nb_track.settle(&scr, t) {
            // 도시·분류를 바꾸는 중 (오른쪽이 아직 앞 것일 수 있음): 앞 도시의 틱 HUD도 지운다
            self.tick = None;
            return;
        }
        // 틱 HUD: 창 오른쪽 바깥, 마지막 점(지금 값) 높이에 'N틱' (감지기와 같은 자리)
        self.tick = scr.pts.map(|p| (corner.0 + WIN_W + 10, corner.1 + tick_y(p[POINTS - 1]), p[POINTS - 1]));
        if self.nb_last.as_ref().is_some_and(|(s, at)| *s == scr && t - at < RESEND_SEC) {
            return;
        }
        self.nb_last = Some((scr.clone(), t));
        // 도시 상태: 모두 목록 아이콘 (고른 도시도). 고른 도시 아이콘을 모르면 '도시 상태' 글자로 배운다
        let mut states = vec![];
        for (c, s) in &scr.list {
            let s = if s.is_none() && Some(c) == scr.city.as_ref() { scr.state.clone().filter(|v| self.nb.learn_selected(v)) } else { s.clone() };
            let Some(s) = s else { continue };
            states.push((c.clone(), if s == "보통" { "평상".to_string() } else { s }));
        }
        let key = scr.city.clone().zip(scr.kind.clone()).filter(|_| scr.pts.is_some());
        let seen = key.is_some() && key != self.nb_seen;
        if seen {
            self.nb_seen = key;
        }
        out.push(Event::Nanban { culture: scr.culture, city: scr.city, kind: scr.kind, pts: scr.pts, states, seen });
    }

    fn city_step(&mut self, rgb: &Rgb, off: (i64, i64), out: &mut Vec<Event>) {
        let (name, conf) = self.r.read_city(rgb, off);
        let now = match name {
            Some(n) if conf >= 0.6 && !n.is_empty() => Some(n),
            Some(_) => None, // 목록에 없는 지명(바다 등) = 도시 밖. 목록 밖 도시 인정은 아직 안 함
            None => {
                // 못 읽음: 그대로 둔다. 바다에 있다가 두 번 연속 못 읽으면 알린다 (감지기와 같음)
                if self.city.is_none() && self.sea {
                    self.city_miss += 1;
                    if self.city_miss == 2 {
                        out.push(Event::Note("도시를 인식중입니다".into()));
                    }
                }
                return;
            }
        };
        self.city_miss = 0;
        self.sea = now.is_none();
        if now != self.city {
            self.city = now.clone();
            out.push(Event::City(now));
        }
    }

    fn note(&mut self, msg: String, out: &mut Vec<Event>) {
        if self.noted.as_ref() != Some(&msg) {
            self.noted = Some(msg.clone());
            out.push(Event::Note(msg));
        }
    }

    fn trade_step(&mut self, tr: &TradeScreen, t: f64, out: &mut Vec<Event>) {
        if let Some(c) = self.city.clone().filter(|c| is_guild_city(c)) {
            self.note(format!("{c} · 길드 개척도시는 수집하지 않음"), out);
            return;
        }
        if tr.kind == "buy" {
            self.note("구입 창은 수집하지 않음 (매각 창만)".into(), out);
            return;
        }
        if !matches!(tr.kind.as_str(), "sell" | "market") {
            return;
        }
        if t - self.t_resend >= RESEND_SEC {
            self.t_resend = t;
            self.sent.clear();
            self.sent_near.clear();
        }
        self.near_step(tr, out);
        if tr.rows.is_empty() {
            return;
        }
        let rows: Vec<Row> =
            tr.rows.iter().map(|r| Row { name: r.name.clone(), price: r.price, pct: r.pct.clone(), tr: mark(&r.arrow), arrow: r.arrow.clone() }).collect();
        let prev = self.prev.replace(rows.clone()).unwrap_or_default();
        let stable: Vec<Row> = rows.into_iter().filter(|r| prev.contains(r)).collect(); // 연속 2번 같은 값만
        let new: Vec<Row> = stable.into_iter().filter(|r| !self.sent.contains(r)).collect();
        if new.is_empty() {
            return;
        }
        // 1.0.7 (2026-11-25 사용자 "업로드가 안 됨"): 도시를 아직 못 읽었으면 보낸 것으로 치지 않는다.
        // 전에는 '도시를 모름'으로 버리고 sent 에 넣어서, 몇 초 뒤 도시를 읽어도 10분 동안 다시 안 올렸다 (교역소에 들어가자마자 매각 창을 열 때)
        if self.city.is_none() {
            self.note("도시를 인식중입니다 · 인식되면 시세를 올립니다".into(), out);
            return;
        }
        self.sent.extend(new.iter().cloned());
        out.push(Event::Trade { kind: tr.kind.clone(), city: self.city.clone(), rows: new });
    }

    /// 시세 창 오른쪽 '인근 도시 시세' (감지기 collector.rs trade_step 의 near 부분과 같음)
    fn near_step(&mut self, tr: &TradeScreen, out: &mut Vec<Event>) {
        let Some(good) = tr.selected.clone().filter(|g| !g.is_empty()) else {
            self.prev_near = None;
            return;
        };
        let items: Vec<NearItem> = tr
            .near
            .iter()
            .filter(|n| Some(&n.city) != self.city.as_ref() && !is_guild_city(&n.city))
            .map(|n| NearItem { city: n.city.clone(), good: good.clone(), pct: n.pct, tr: mark(&n.arrow), arrow: n.arrow.clone() })
            .collect();
        let prev = self.prev_near.replace(items.clone()).unwrap_or_default();
        let new: Vec<NearItem> = items.into_iter().filter(|r| prev.contains(r) && !self.sent_near.contains(r)).collect();
        if new.is_empty() {
            return;
        }
        self.sent_near.extend(new.iter().cloned());
        out.push(Event::Near { good, items: new });
    }
}
