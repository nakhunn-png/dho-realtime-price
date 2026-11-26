//! 교역 창 · 도시 이름 · 채팅 시장 소식 · 캐릭터 이름 읽기 (Python `Recognizer`)
//!
//! 입력은 창 테두리·제목 표시줄이 포함된 캡처 프레임(RGB). 클라이언트 영역 시작점(client_off)은 부르는 쪽이 넘긴다.
use crate::arrows::{Arrow, ArrowReader};
use crate::cv::{Gray, Mask, Rgb, bright, match_ccoeff_normed, match_max, max_loc, resize_linear, text_mask, trim, trim_mask};
use crate::digits::{DigitReader, is_grouped_number};
use crate::icons::{ICON_PAD, ICON_SURE, IconBook};
use crate::names::{NameMatcher, Ranked, sort_ranked};
use crate::ocr::ocr_lines;
use crate::owner_font::OwnerFont;
use crate::pngio;
use crate::text::{close_match, close_match_jamo, ends_sentence, guild_city_prefix, is_hangul, is_owner, match_owner_line, round_half_even};
use std::collections::HashMap;

// 대화상자 왼쪽 위 모서리 기준 오프셋 (게임 UI는 해상도와 관계없이 픽셀 크기가 고정)
pub const ROW_PITCH: i64 = 56;
/// 시세 % 대신 괄호 안에 나오는 단어
pub const TIERS: [&str; 5] = ["최저가", "염가", "보통가", "고가", "폭등가"];
pub const ROWS_VISIBLE: i64 = 4;
/// 시세 창에서 선택한 행의 가격·%를 쓸 때 숫자 일치도 기준 (평소 0.8)
pub const SEL_DIGIT_SCORE: f64 = 0.95;
pub const NAME_BOX: (i64, i64, i64, i64) = (81, 50, 278, 70);
pub const PRICE_BOX: (i64, i64, i64, i64) = (116, 74, 263, 94);
/// 시세 창 오른쪽 '인근 도시 시세' 칸: 왼쪽 칸에서 이만큼 오른쪽
pub const NEAR_DX: i64 = 363;
pub const NEAR_PCT_BOX: (i64, i64, i64, i64) = (520, 74, 615, 94);
pub const NEAR_ROWS: i64 = 5;
/// 첫 행의 '(108%)' 바로 뒤
pub const ARROW_BOX: (i64, i64, i64, i64) = (252, 70, 282, 96);
/// 클라이언트 기준: 위쪽 파란 안내 막대
pub const BAR_BOX: (i64, i64, i64, i64) = (188, 4, 900, 30);
/// 클라이언트 기준: 왼쪽 위 도시 이름
pub const CITY_BOX: (i64, i64, i64, i64) = (4, 5, 240, 24);
/// x0, 아래에서 위로 y0, x1, 아래에서 y1
pub const CHAT_BOX_FROM_BOTTOM: (i64, i64, i64, i64) = (6, 335, 600, 30);
/// 클라이언트 기준: 왼쪽 위 함대 목록 첫 줄(자기 캐릭터) 이름
pub const PARTY_NAME_BOX: (i64, i64, i64, i64) = (42, 47, 198, 71);
const SEL_MIN: f64 = 0.55;
const SEL_CLOSE: f64 = 0.03;
const SPACINGS: [i32; 5] = [0, 2, 1, 3, -1];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pct {
    Num(i64),
    /// '최저가/염가/보통가/고가/폭등가' (% 대신 단어가 나오는 캐릭터)
    Tier(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeRow {
    pub name: String,
    pub price: i64,
    pub pct: Pct,
    pub score: f64,
    pub arrow: Option<Arrow>,
    /// 시세 창에서 클릭해 고른 행 (청록 바탕)
    pub selected: bool,
}

/// 시세 창 '인근 도시 시세' 한 줄 (가격 없이 %만 나옴)
#[derive(Clone, Debug, PartialEq)]
pub struct NearRow {
    pub city: String,
    pub pct: i64,
    pub arrow: Option<Arrow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeScreen {
    /// "buy" | "sell" | "market"(시세 창) | "?"
    pub kind: String,
    pub corner: (i64, i64),
    pub rows: Vec<TradeRow>,
    pub unknown_rows: usize,
    /// 시세 창: 선택한 품목의 인근 도시 시세
    pub near: Vec<NearRow>,
    /// 시세 창: 선택한 품목 (보이는 행 중에 없으면 None)
    pub selected: Option<String>,
}

pub struct Recognizer {
    pub icons: IconBook,
    pub goods: NameMatcher,
    pub cities: NameMatcher,
    /// % 대신 나오는 시세 단어
    pub tiers: NameMatcher,
    /// 이 PC의 캐릭터 이름 (Mail 폴더 + 사용자가 알려준 이름)
    pub chars: NameMatcher,
    pub digits: DigitReader,
    arrows: ArrowReader,
    corner: Gray,
    corner_half: Gray,
    dlg_last: Option<((i64, i64), f32)>,
    city_bin: Option<Vec<Option<Gray>>>,
    city_cache: HashMap<Vec<u8>, (Option<String>, f64)>,
    news_cache: HashMap<Vec<u8>, Vec<String>>,
    kind_cache: HashMap<Vec<u8>, String>,
    rank_cache: HashMap<(i32, usize, usize, Vec<u8>), Ranked>,
    tier_key: Vec<i32>,
    tier_bin: Vec<(String, Vec<Gray>)>,
    spacing_fixed: bool,
    ocr_warned: bool,
    /// 교역소 주인 대사를 굴림 글자 모양으로 (굴림을 못 만들면 None → 글자 인식만)
    owner_font: Option<OwnerFont>,
    /// 이미 기록한 '정해진 대사와 다른 주인 대사' (같은 것을 계속 남기지 않게)
    news_odd: std::collections::HashSet<String>,
    /// 글자 간격을 새로 맞췄을 때 알림 (Python 은 print)
    pub log: Vec<String>,
}

fn sub(rgb: &Rgb, ox: i64, oy: i64, b: (i64, i64, i64, i64)) -> Rgb {
    rgb.crop(ox + b.0, oy + b.1, ox + b.2, oy + b.3)
}

impl Recognizer {
    pub fn new<S: AsRef<str>>(goods: &[S], cities: &[S], icon_dir: Option<std::path::PathBuf>) -> Self {
        let corner = pngio::load_l(crate::assets::DIALOG_CORNER).expect("dialog_corner.png");
        Recognizer {
            icons: IconBook::new(icon_dir),
            goods: NameMatcher::new(goods),
            cities: NameMatcher::new(cities),
            tiers: NameMatcher::new(&TIERS),
            chars: NameMatcher::new::<&str>(&[]),
            digits: DigitReader::new(),
            arrows: ArrowReader::new(),
            corner_half: corner.half(),
            corner,
            dlg_last: None,
            city_bin: None,
            city_cache: HashMap::new(),
            news_cache: HashMap::new(),
            kind_cache: HashMap::new(),
            rank_cache: HashMap::new(),
            tier_key: vec![],
            tier_bin: vec![],
            spacing_fixed: false,
            ocr_warned: false,
            owner_font: OwnerFont::new(),
            news_odd: Default::default(),
            log: vec![],
        }
    }

    // ------------------------------------------------------------ 교역 창
    pub fn find_dialog(&mut self, rgb: &Rgb) -> Option<(i64, i64)> {
        let g = rgb.to_gray_cv();
        let (th, tw) = (self.corner.h as i64, self.corner.w as i64);
        let at = |x: i64, y: i64, pad: i64| -> (f32, Option<(i64, i64)>) {
            let (x0, y0) = ((x - pad).max(0), (y - pad).max(0));
            let s = g.crop(x0, y0, x0 + tw + 2 * pad, y0 + th + 2 * pad);
            if (s.h as i64) < th || (s.w as i64) < tw {
                return (-1.0, None);
            }
            let (mx, lx, ly) = max_loc(&match_ccoeff_normed(&s, &self.corner));
            (mx, Some((x0 + lx as i64, y0 + ly as i64)))
        };
        // 1) 지난번 자리만 먼저 확인 (창은 보통 같은 자리에 열린다). 처음 찾았을 때만큼 잘 맞아야 인정
        if let Some(((lx, ly), ls)) = self.dlg_last {
            let (mx, loc) = at(lx, ly, 4);
            if mx >= f32::max(0.75, ls - 0.02) {
                return loc;
            }
        }
        // 2) 화면을 절반으로 줄여 후보를 고르고, 후보 몇 곳만 원래 크기로 확인.
        //    시세 창은 오른쪽 칸 모서리도 비슷하게 생겨서, 후보 중 원래 크기에서 가장 잘 맞는 곳을 고른다
        let half = g.half();
        if half.h < self.corner_half.h || half.w < self.corner_half.w {
            return None; // 창이 너무 작게 잡힌 프레임 (최소화 직후 등)
        }
        let mut r = match_ccoeff_normed(&half, &self.corner_half);
        let mut best: (f32, Option<(i64, i64)>) = (-1.0, None);
        for _ in 0..4 {
            let (mx, lx, ly) = max_loc(&r);
            if mx < 0.55 {
                break;
            }
            for y in ly.saturating_sub(6)..(ly + 7).min(r.h) {
                for x in lx.saturating_sub(6)..(lx + 7).min(r.w) {
                    r.data[y * r.w + x] = -1.0;
                }
            }
            let cand = at(lx as i64 * 2, ly as i64 * 2, 6);
            if cand.0 > best.0 {
                best = cand;
            }
        }
        if best.0 < 0.75 {
            return None;
        }
        self.dlg_last = Some((best.1?, best.0));
        best.1
    }

    pub fn trade_kind(&mut self, rgb: &Rgb, client_off: (i64, i64)) -> String {
        let crop = sub(rgb, client_off.0, client_off.1, BAR_BOX);
        if crop.is_empty() {
            return "?".into();
        }
        let key: Vec<u8> = resize_linear(&crop.to_gray_cv(), 64, 4).data.iter().map(|&v| v as u8).collect();
        if let Some(k) = self.kind_cache.get(&key) {
            return k.clone();
        }
        let text: String = match ocr_lines(&crop, 2) {
            Ok(l) => l.iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(" ").replace(' ', ""),
            Err(e) => {
                self.warn_ocr(&e);
                String::new()
            }
        };
        // OCR이 '매각말', '묻품들' 처럼 글자를 조금씩 틀리므로 짧은 핵심어로 판단
        let has = |ks: &[&str]| ks.iter().any(|k| text.contains(k));
        let kind = if has(&["시세", "열람", "표시합"]) {
            "market"
        } else if has(&["구입", "입할"]) {
            "buy"
        } else if has(&["매각", "각할", "선택하"]) {
            "sell"
        } else {
            "?"
        };
        if kind != "?" {
            if self.kind_cache.len() > 50 {
                self.kind_cache.clear();
            }
            self.kind_cache.insert(key, kind.into());
        }
        kind.into()
    }

    pub fn read_trade(&mut self, rgb: &Rgb, client_off: (i64, i64)) -> Option<TradeScreen> {
        let (cx, cy) = self.find_dialog(rgb)?;
        let kind = self.trade_kind(rgb, client_off);
        let mut scr = TradeScreen { kind, corner: (cx, cy), rows: vec![], unknown_rows: 0, near: vec![], selected: None };
        for i in 0..ROWS_VISIBLE {
            let dy = i * ROW_PITCH;
            let (nx0, ny0, nx1, ny1) = NAME_BOX;
            let b = bright(&rgb.crop(cx + nx0, cy + ny0 + dy, cx + nx1, cy + ny1 + dy));
            let m = text_mask(&b);
            if m.count() < 15 {
                continue; // 빈 행
            }
            let ranked = self.rank_goods(&trim(&b, &m).unwrap(), &trim_mask(&m).unwrap());
            let (mut name, score) = ranked.first().map_or((None, 0.0), |r| (Some(r.1.clone()), r.0));
            let margin = ranked.first().map_or(0.0, |r| r.0 - ranked.get(1).map_or(0.0, |x| x.0));
            let (px0, py0, px1, py1) = PRICE_BOX;
            let strip = rgb.crop(cx + px0, cy + py0 + dy, cx + px1, cy + py1 + dy);
            let mut pr = self.digits.read(&strip, None).map(|(p, c)| (p, Pct::Num(c)));
            let (ax0, ay0, ax1, ay1) = ARROW_BOX;
            let arrow = self.arrows.read(&rgb.crop(cx + ax0, cy + ay0 + dy, cx + ax1, cy + ay1 + dy));
            let bg = rgb.crop(cx + 20, cy + 48 + dy, cx + 280, cy + 100 + dy);
            let teal = bg.data.chunks_exact(3).filter(|p| p[1] > 120 && p[2] > 110 && p[0] < 90).count();
            let sel = !bg.is_empty() && teal as f64 / (bg.w * bg.h) as f64 > 0.4;
            let mut name_ok = name.is_some() && score >= 0.6 && margin >= 0.08;
            if !sel && name_ok && score >= ICON_SURE {
                let icon = IconBook::crop(rgb, cx, cy, dy, 0);
                self.icons.learn(name.as_deref().unwrap(), &icon);
            }
            if sel && !name_ok {
                // 선택 행은 글자가 굵어져 점수가 낮게 나온다: 기준을 0.55로 낮추되, 1·2등이 0.03 이내면 아이콘으로 가린다
                name = self.pick_selected(&ranked, &IconBook::crop(rgb, cx, cy, dy, ICON_PAD));
                name_ok = name.is_some();
            }
            if sel && name_ok {
                scr.selected = name.clone(); // 선택 행은 가격 글자가 굵게 바뀌어 못 읽을 때가 있어도 품목명은 정확하다
            }
            if pr.is_none() && !sel {
                pr = self.read_price_tier(&strip);
            }
            if sel {
                // 매각 창의 선택 행은 글자가 굵어져 숫자를 잘못 읽으므로(2,950→2060) 쓰지 않는다.
                // 시세 창의 선택 행(인근 시세를 보려고 누른 품목)은 숫자가 그대로라 읽을 수 있다: 아주 확실할 때만 쓴다
                pr = if scr.kind == "market" { self.digits.read(&strip, Some(SEL_DIGIT_SCORE)).map(|(p, c)| (p, Pct::Num(c))) } else { None };
            }
            match (name_ok, pr) {
                (true, Some((price, pct))) => scr.rows.push(TradeRow {
                    name: name.unwrap(),
                    price,
                    pct,
                    score: (score * 1000.0).round() / 1000.0,
                    arrow,
                    selected: sel,
                }),
                _ => scr.unknown_rows += 1,
            }
        }
        if scr.kind == "market" && scr.selected.is_some() {
            scr.near = self.read_near(rgb, cx, cy);
        }
        Some(scr)
    }

    /// 품목 이름 비교 (662개). 글자 모양이 전에 본 그대로면 다시 비교하지 않는다
    fn rank_goods(&mut self, glyph: &Gray, mask: &Mask) -> Ranked {
        let key = (self.goods.extra, mask.w, mask.h, mask.packbits());
        if let Some(r) = self.rank_cache.get(&key) {
            return r.clone();
        }
        if self.rank_cache.len() > 512 {
            self.rank_cache.clear();
        }
        let r = self.goods.ranked(glyph);
        self.rank_cache.insert(key, r.clone());
        r
    }

    pub fn pick_selected(&self, ranked: &Ranked, icon_area: &Rgb) -> Option<String> {
        let cands: Vec<&(f64, String)> = ranked.iter().filter(|r| r.0 >= SEL_MIN).collect();
        if cands.is_empty() {
            return None;
        }
        if cands.len() == 1 || cands[0].0 - cands[1].0 > SEL_CLOSE {
            return Some(cands[0].1.clone());
        }
        let close: Vec<String> = cands.iter().filter(|r| cands[0].0 - r.0 <= SEL_CLOSE).map(|r| r.1.clone()).collect();
        self.icons.pick(&close, icon_area)
    }

    /// '26,500 Ɖ (염가)' → (26500, '염가'). % 대신 시세 단어가 나오는 행. 못 읽으면 None
    fn read_price_tier(&mut self, strip: &Rgb) -> Option<(i64, Pct)> {
        let (g, segs) = DigitReader::segments(strip);
        let segs = self.digits.fix_ones(&g, segs);
        let (mut out, mut low, mut end) = (String::new(), 1.0f64, None);
        for s in &segs {
            match s.kind {
                'd' => {
                    let (d, sc) = self.digits.classify(&DigitReader::glyph(&g, s));
                    low = low.min(sc);
                    out.push(d);
                }
                ',' => out.push(','),
                'W' => {
                    // 화폐 기호 Ɖ (뒤의 '(' 와 붙어 보일 때도 있음)
                    end = Some(s.x1);
                    break;
                }
                _ => return None,
            }
        }
        let end = end?;
        if !is_grouped_number(&out) || low < self.digits.min_score {
            return None;
        }
        let price: i64 = out.replace(',', "").parse().ok()?;
        if !(0 < price && price < 10_000_000) {
            return None;
        }
        let tier = self.match_tier(&g.crop(end as i64, 0, g.w as i64, g.h as i64))?;
        Some((price, Pct::Tier(tier)))
    }

    /// 괄호 안 밝은 글자 → '염가' 등. 확실하지 않으면 None
    pub fn match_tier(&mut self, g: &Gray) -> Option<String> {
        let m = g.gt(140.0).to_gray();
        // 시세 단어는 글자 간격이 창마다 다르게 그려진다 ('염가'가 2px 넓게): 간격별로 그려 두고 가장 잘 맞는 것
        let mut spacings = vec![];
        for e in [self.tiers.extra, 0, 1, 2, 3] {
            if !spacings.contains(&e) {
                spacings.push(e);
            }
        }
        if self.tier_key != spacings {
            self.tier_key = spacings.clone();
            let names = self.tiers.names.clone();
            self.tier_bin = names
                .into_iter()
                .map(|n| {
                    let v = spacings.iter().filter_map(|&e| self.tiers.render(&n, e)).map(|t| t.gt(100.0).to_gray()).collect();
                    (n, v)
                })
                .collect();
        }
        let mut scores: Ranked = vec![];
        for (n, variants) in &self.tier_bin {
            let mut best: Option<f64> = None;
            for tb in variants {
                if tb.h <= m.h && tb.w <= m.w {
                    let v = match_max(&m, tb) as f64;
                    best = Some(best.map_or(v, |b| b.max(v)));
                }
            }
            if let Some(b) = best {
                scores.push((b, n.clone()));
            }
        }
        sort_ranked(&mut scores);
        if scores.is_empty() || scores[0].0 < 0.8 || (scores.len() > 1 && scores[0].0 - scores[1].0 < 0.05) {
            return None;
        }
        Some(scores[0].1.clone())
    }

    fn read_near(&self, rgb: &Rgb, cx: i64, cy: i64) -> Vec<NearRow> {
        let mut out = vec![];
        for i in 0..NEAR_ROWS {
            let dy = i * ROW_PITCH;
            let (nx0, ny0, nx1, ny1) = NAME_BOX;
            let b = bright(&rgb.crop(cx + nx0 + NEAR_DX, cy + ny0 + dy, cx + nx1 + NEAR_DX, cy + ny1 + dy));
            let m = text_mask(&b);
            if m.count() < 15 {
                break;
            }
            let (name, score, margin) = self.cities.best(&trim(&b, &m).unwrap());
            let (px0, py0, px1, py1) = NEAR_PCT_BOX;
            let pct = self.digits.read_pct(&rgb.crop(cx + px0, cy + py0 + dy, cx + px1, cy + py1 + dy));
            let (ax0, ay0, ax1, ay1) = ARROW_BOX;
            let arrow = self.arrows.read(&rgb.crop(cx + ax0 + NEAR_DX, cy + ay0 + dy, cx + ax1 + NEAR_DX, cy + ay1 + dy));
            if let (Some(city), Some(pct)) = (name, pct) {
                if score >= 0.6 && margin >= 0.08 {
                    out.push(NearRow { city, pct, arrow });
                }
            }
        }
        out
    }

    // ------------------------------------------------------------ 도시 이름 (교역 창이 없을 때 왼쪽 위)
    /// (도시명, 확신도). 확신도 1=목록과 일치, 0.9=길드 개척도시, 0.6=비슷한 이름, 0.2=목록에 없음(OCR 원문)
    pub fn read_city(&mut self, rgb: &Rgb, client_off: (i64, i64)) -> (Option<String>, f64) {
        let crop = sub(rgb, client_off.0, client_off.1, CITY_BOX);
        let mn = crop.min_channel();
        let m = mn.gt(170.0);
        if m.count() < 30 {
            return (None, 0.0);
        }
        let mut key = b"city".to_vec();
        key.extend(m.packbits());
        if let Some(v) = self.city_cache.get(&key) {
            return v.clone(); // 글자 모양이 전에 본 그대로면 OCR 생략
        }
        // 도시명은 굴림 14 그대로라 목록의 도시는 글자 모양 비교로 정확히 읽는다 (OCR은 '파루'를 '마루'로 읽음)
        let mut hit = self.city_by_font(&m, &mn);
        if hit.is_some() {
            self.spacing_fixed = true; // 지금 간격으로 도시명이 맞음 → 간격 확정
        } else {
            hit = self.calibrate_spacing(&m, &mn);
        }
        if let Some(h) = hit {
            let v = (Some(h), 1.0);
            self.remember_city(key, v.clone());
            return v;
        }
        // 흰 바탕 검은 글자로 바꿔 OCR
        let (w, h) = (m.w + 20, m.h + 20);
        let mut img = vec![255u8; w * h * 3];
        for y in 0..m.h {
            for x in 0..m.w {
                if m.data[y * m.w + x] {
                    let i = ((y + 10) * w + x + 10) * 3;
                    img[i..i + 3].fill(0);
                }
            }
        }
        let lines = match ocr_lines(&Rgb::new(w, h, img), 2) {
            Ok(l) => l,
            Err(e) => {
                // 한국어 글자 인식이 없는 PC: 목록에 있는 도시만 읽는다
                self.warn_ocr(&e);
                vec![]
            }
        };
        let raw = lines.first().map_or(String::new(), |l| l.0.trim().to_string());
        // '보스턴 (동맹국 🚩)' → 국적 표시 떼기, '보스턴 항구' → 항구 구역도 그 도시
        let mut raw = clean_city(&raw);
        for suf in ["항구", "교외"] {
            if let Some(s) = raw.strip_suffix(suf) {
                raw = s.trim_end().to_string();
                break;
            }
        }
        let raw = raw.trim().to_string();
        let val = if self.cities.contains(&raw) {
            (Some(raw), 1.0)
        } else if let Some(g) = guild_city_prefix(&raw).filter(|_| !raw.is_empty()) {
            // 길드 개척도시: 시세가 항상 100%라 수집하지 않는다
            (Some(format!("{g} 개척도시")), 0.9)
        } else if !raw.is_empty() {
            if let Some(c) = close_match(&raw, &self.cities.names, 0.75).or_else(|| close_match_jamo(&raw, &self.cities.names, 0.85)) {
                (Some(c.clone()), 0.6)
            } else if (2..=20).contains(&raw.chars().count()) && raw.chars().all(|c| is_hangul(c) || c == ' ') {
                (Some(raw), 0.2)
            } else {
                (None, 0.0)
            }
        } else {
            (None, 0.0)
        };
        self.remember_city(key, val.clone());
        val
    }

    /// 목록 도시명이 안 맞으면 글자 간격을 바꿔 다시 맞춰 본다. 맞는 간격을 찾으면 모든 이름 비교에 쓴다
    fn calibrate_spacing(&mut self, m: &Mask, mn: &Gray) -> Option<String> {
        if self.spacing_fixed {
            return None;
        }
        let cur = self.cities.extra;
        for e in SPACINGS {
            if e == cur {
                continue;
            }
            self.cities.set_extra(e);
            self.city_bin = None;
            if let Some(hit) = self.city_by_font(m, mn) {
                self.spacing_fixed = true; // 한 번 맞으면 그 PC의 간격으로 고정
                self.goods.set_extra(e);
                self.chars.set_extra(e);
                self.tiers.set_extra(e);
                self.log.push(format!("[글자 간격] 이 PC는 글자 간격 +{e}px로 읽습니다"));
                return Some(hit);
            }
        }
        self.cities.set_extra(cur);
        self.city_bin = None;
        None
    }

    /// 흰 글자 마스크에서 목록 도시명을 찾는다. 거의 똑같이 맞는 이름 중 가장 긴 것 ('파루' < '파루항' 대비)
    fn city_by_font(&mut self, m: &Mask, mn: &Gray) -> Option<String> {
        if self.city_bin.is_none() {
            self.city_bin = Some(self.cities.tpl.iter().map(|t| t.as_ref().map(|t| t.gt(100.0).to_gray())).collect());
        }
        let mm = m.to_gray();
        // 게임 글자는 흰 글자 + 검은 테두리: 테두리 옆 흰 점만 '글자'로 본다 (밝은 배경은 무시)
        let txt = m.and(&mn.lt(70.0).dilate3());
        let mut best: Vec<(f64, &String)> = vec![];
        for (t, n) in self.city_bin.as_ref().unwrap().iter().zip(&self.cities.names) {
            let Some(t) = t else { continue };
            if t.h > mm.h || t.w > mm.w {
                continue;
            }
            let (s, x, _) = max_loc(&match_ccoeff_normed(&mm, t));
            // 이름 뒤에 글자가 더 있으면('리스본 앞바다' 등) 그 도시가 아니다
            let e = (x + t.w) as i64;
            if s >= 0.9 && txt.crop(e + 2, 0, e + 12, txt.h as i64).count() < 3 {
                best.push((s as f64, n));
            }
        }
        let top = best.iter().map(|b| b.0).fold(f64::NEG_INFINITY, f64::max);
        // max(key=len): 길이가 같으면 먼저 나온 이름
        let mut pick: Option<&String> = None;
        for (s, n) in &best {
            if *s >= top - 0.02 && pick.is_none_or(|p| n.chars().count() > p.chars().count()) {
                pick = Some(n);
            }
        }
        pick.cloned()
    }

    fn remember_city(&mut self, key: Vec<u8>, val: (Option<String>, f64)) {
        if self.city_cache.len() > 64 {
            self.city_cache.clear();
        }
        self.city_cache.insert(key, val);
    }

    fn warn_ocr(&mut self, e: &str) {
        if !self.ocr_warned {
            self.ocr_warned = true;
            self.log.push(format!("[글자 인식] Windows 한국어 글자 인식을 쓸 수 없음: {e}"));
        }
    }

    // ------------------------------------------------------------ 채팅: 교역소 주인 대사만
    pub fn read_market_news(&mut self, rgb: &Rgb, client_off: (i64, i64)) -> Vec<String> {
        let (x0, yb0, x1, yb1) = CHAT_BOX_FROM_BOTTOM;
        let hh = rgb.h as i64;
        let crop = rgb.crop(client_off.0 + x0, hh - yb0, client_off.0 + x1, hh - yb1);
        if crop.is_empty() {
            return vec![];
        }
        // 채팅창 그림이 지난번과 점 하나까지 똑같을 때만 OCR 생략 (교역소 주인 대사는 놓치면 안 된다)
        let mut key = b"chat".to_vec();
        key.extend(exact_key(&crop));
        if let Some(v) = self.news_cache.get(&key) {
            return v.clone();
        }
        let out = self.read_news(&crop);
        if self.news_cache.len() > 64 {
            self.news_cache.clear();
        }
        self.news_cache.insert(key, out.clone());
        out
    }

    /// 주인 대사: 먼저 굴림 글자 모양으로 정해진 대사와 맞추고, 못 맞춘 줄이 있으면 글자 인식(OCR)으로 읽는다
    fn read_news(&mut self, crop: &Rgb) -> Vec<String> {
        let cur = self.cities.extra;
        let mut extras = vec![cur];
        if !self.spacing_fixed {
            extras.extend(SPACINGS.iter().copied().filter(|&e| e != cur));
        }
        let rows = self.owner_font.as_mut().and_then(|f| f.read(crop, &extras));
        if let Some(rows) = &rows {
            if rows.iter().all(|r| r.1.is_some()) {
                return rows.iter().filter_map(|r| r.1.clone()).collect();
            }
        }
        let (news, raw) = self.read_news_ocr(crop);
        // 여기까지 왔으면 글자 모양으로 못 맞춘 주인 말이 있다 → 정해진 대사와 얼마나 다른지 기록해 둔다 (사용자가 알고 싶어 함)
        for (r, m) in raw {
            let key = format!("{r}→{m:?}");
            if self.news_odd.len() < 200 && self.news_odd.insert(key) {
                let how = m.map_or("버림 (정해진 대사 아님)".to_string(), |m| format!("'{m}'(으)로 맞춤"));
                self.log.push(format!("[대사] 글자 모양으로 못 맞춘 주인 대사: 글자 인식 '{r}' → {how}"));
            }
        }
        news
    }

    /// 글자 인식으로 읽은 주인 대사 (정식 대사들, [(인식 원문, 맞춘 정식 대사)])
    fn read_news_ocr(&mut self, crop: &Rgb) -> (Vec<String>, Vec<(String, Option<String>)>) {
        let mut lines = match ocr_lines(crop, 2) {
            Ok(l) => l,
            Err(e) => {
                self.warn_ocr(&e);
                return (vec![], vec![]);
            }
        };
        lines.sort_by(|a, b| {
            (round_half_even(a.2 / 8.0), a.1).partial_cmp(&(round_half_even(b.2 / 8.0), b.1)).unwrap_or(std::cmp::Ordering::Equal)
        });
        // 같은 줄(높이가 같은) 조각 합치기
        let mut merged: Vec<String> = vec![];
        let mut cur_y: Option<f64> = None;
        for (t, _, y) in lines {
            match cur_y {
                // 2026-11-21 사그레스: '교역소주인          : 어서 오세요.' 처럼 이름과 ':' 사이가 멀면 ':' 쪽 조각이 12px쯤 아래로 따로 나온다
                // → ':'로 시작하는 조각은 바로 위 줄이 ':' 없는 이름이면 16px 안까지 붙인다
                Some(cy)
                    if (y - cy).abs() < 8.0
                        || ((y - cy).abs() < 16.0
                            && t.trim_start().starts_with([':', '：'])
                            && merged.last().is_some_and(|l| !l.contains([':', '：']))) =>
                {
                    let last = merged.last_mut().unwrap();
                    last.push(' ');
                    last.push_str(&t);
                }
                _ => {
                    merged.push(t);
                    cur_y = Some(y);
                }
            }
        }
        // 도시 상태 = 교역소 주인이 하는 말만. 줄이 넘어가면 다음 줄(이름 없음)을 이어 붙인다
        let (mut news, mut buf): (Vec<String>, Option<String>) = (vec![], None);
        for t in merged {
            // '말한 사람 : 대사'
            if let Some(p) = t.find([':', '：']) {
                if let Some(b) = buf.take() {
                    news.push(b);
                }
                let colon_len = t[p..].chars().next().unwrap().len_utf8();
                let (who, say) = (t[..p].trim(), t[p + colon_len..].trim());
                buf = is_owner(who).then(|| say.to_string());
            } else if buf.as_ref().is_some_and(|b| !ends_sentence(b)) {
                let b = buf.take().unwrap();
                buf = Some(format!("{b} {}", t.trim()).trim().to_string());
            } else if let Some(b) = buf.take() {
                news.push(b);
            }
        }
        if let Some(b) = buf {
            news.push(b);
        }
        let raw: Vec<(String, Option<String>)> = news.into_iter().map(|n| { let m = match_owner_line(&n); (n, m) }).collect();
        (raw.iter().filter_map(|r| r.1.clone()).collect(), raw)
    }

    // ------------------------------------------------------------ 캐릭터와 서버
    /// (이름, 확실한가). 이름 목록에 있으면 (이름, true), 없으면 글자 인식 결과 (글자, false), 못 읽으면 (None, false)
    pub fn read_character(&mut self, rgb: &Rgb, client_off: (i64, i64)) -> (Option<String>, bool) {
        let crop = sub(rgb, client_off.0, client_off.1, PARTY_NAME_BOX);
        let m = crop.min_channel().gt(190.0); // 흰 글자만 (테두리·바탕 제외)
        match trim_mask(&m) {
            Some(g) if g.h >= 9 => {}
            _ => return (None, false),
        }
        // 이름 모양(흰 글자 위치)을 통째로 맞춘다. 바탕의 밝은 점이 몇 개 섞여도 되도록 미끄러뜨려 비교
        let mp = m.to_gray().pad(2);
        if let Some(n) = self.char_by_font(&mp) {
            return (Some(n), true);
        }
        // 2026-11-26: 글자 간격은 도시명을 읽어야 정해져서, 바다에서 켜면 기본 간격으로 비교해 적힌 이름이 '다름'으로 나왔다
        // → 간격이 아직 안 정해졌으면 이름으로도 간격을 찾는다 (맞으면 그 PC 간격으로 고정, calibrate_spacing 과 같음)
        if !self.spacing_fixed && !self.chars.names.is_empty() {
            let cur = self.chars.extra;
            for e in SPACINGS.into_iter().filter(|&e| e != cur) {
                self.chars.set_extra(e);
                if let Some(n) = self.char_by_font(&mp) {
                    self.spacing_fixed = true;
                    self.cities.set_extra(e);
                    self.city_bin = None;
                    self.goods.set_extra(e);
                    self.tiers.set_extra(e);
                    self.log.push(format!("[글자 간격] 이 PC는 글자 간격 +{e}px로 읽습니다 (캐릭터 이름으로 찾음)"));
                    return (Some(n), true);
                }
            }
            self.chars.set_extra(cur);
        }
        let (w, h) = (m.w + 40, m.h + 36);
        let mut img = vec![255u8; w * h * 3];
        for y in 0..m.h {
            for x in 0..m.w {
                if m.data[y * m.w + x] {
                    let i = ((y + 18) * w + x + 20) * 3;
                    img[i..i + 3].fill(0);
                }
            }
        }
        match ocr_lines(&Rgb::new(w, h, img), 3) {
            Ok(l) => {
                let txt: String = l.iter().flat_map(|t| t.0.chars()).filter(|&c| is_hangul(c) || c.is_ascii_alphanumeric()).collect();
                ((!txt.is_empty()).then_some(txt), false)
            }
            Err(e) => {
                // 한국어 글자 인식이 없는 PC: 이름만 못 읽고 나머지는 계속
                self.warn_ocr(&e);
                (None, false)
            }
        }
    }

    /// 이름 칸 마스크(pad 2)와 적힌 이름 글자 그림이 90% 넘게, 다음 후보보다 2% 넘게 맞으면 그 이름
    fn char_by_font(&self, mp: &Gray) -> Option<String> {
        let mut scores: Ranked = vec![];
        for (t, n) in self.chars.tpl.iter().zip(&self.chars.names) {
            if let Some(t) = t {
                let tb = t.gt(100.0).to_gray();
                if tb.h <= mp.h && tb.w <= mp.w {
                    scores.push((match_max(mp, &tb) as f64, n.clone()));
                }
            }
        }
        sort_ranked(&mut scores);
        (!scores.is_empty() && scores[0].0 >= 0.9 && (scores.len() < 2 || scores[0].0 - scores[1].0 >= 0.02)).then(|| scores[0].1.clone())
    }

    /// 머리 위 이름표에서 국기 왼쪽 위 하늘색 별을 본다. Some(true)=다른 서버, Some(false)=원래 서버, None=이름표를 못 찾음
    pub fn away_from_home(&self, rgb: &Rgb, name: &str) -> Option<bool> {
        let t = self.chars.get(name)?;
        let (hh, ww) = (rgb.h as i64, rgb.w as i64);
        let (y0, x0) = ((hh / 2 - 300).max(0), (ww / 2 - 350).max(0));
        let reg = rgb.crop(x0, y0, ww / 2 + 350, hh / 2 + 150).channel(1); // 초록·흰 이름 모두 G가 밝다
        if reg.h < t.h || reg.w < t.w {
            return None;
        }
        let (mx, lx, ly) = max_loc(&match_ccoeff_normed(&reg, t));
        if mx < 0.7 {
            return None;
        }
        // 그 자리 이름이 정말 내 캐릭터인지: 아는 다른 이름(한 글자 차이)이 더 잘 맞으면 남의 이름표
        let (lx, ly) = (lx as i64, ly as i64);
        let spot = reg.crop((lx - 2).max(0), ly, lx + t.w as i64 + 2, ly + t.h as i64 + 2);
        for (o, n) in self.chars.tpl.iter().zip(&self.chars.names) {
            if let Some(o) = o {
                if n != name && o.h <= spot.h && o.w <= spot.w && match_max(&spot, o) >= mx - 0.005 {
                    return None;
                }
            }
        }
        let (x, y) = (lx + x0, ly + y0);
        let bx = rgb.crop((x - 32).max(0), (y - 6).max(0), x - 16, y + 10);
        let star = bx.data.chunks_exact(3).filter(|p| {
            let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
            b > 200 && g > 170 && r < 200 && b - r > 40
        });
        Some(star.count() >= 8)
    }
}

/// `re.split(r"\s*[(\[{（]", raw)[0].strip()`
fn clean_city(raw: &str) -> String {
    let end = raw.find(['(', '[', '{', '（']).unwrap_or(raw.len());
    raw[..end].trim().to_string()
}

/// 채팅창 그림 그대로의 지문 (캐시 열쇠). 점 하나만 달라도 다른 열쇠가 된다.
/// 줄이거나 밝은 점만 보면 새 대사가 나와도 같은 화면으로 보는 일이 있어 (창이 흐려지거나 줄 하나만 늘 때) 그림 전체를 쓴다.
fn exact_key(im: &Rgb) -> Vec<u8> {
    use std::hash::{BuildHasher, Hasher};
    let mut key = Vec::with_capacity(24);
    key.extend((im.w as u32).to_le_bytes());
    key.extend((im.h as u32).to_le_bytes());
    for seed in [0u64, 0x9e37_79b9_7f4a_7c15] {
        let mut h = std::hash::BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default().build_hasher();
        h.write_u64(seed);
        h.write(&im.data);
        key.extend(h.finish().to_le_bytes());
    }
    key
}
