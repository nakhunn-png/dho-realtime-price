//! 무역 상인 '남만무역정보' 창 읽기 (3.4).
//!
//! 창 왼쪽 위 모서리는 교역 창과 같은 틀이라 `Recognizer::find_dialog` 로 찾는다. 그 모서리 기준으로:
//! - 문화권 칸 (조선 · 일본 · 대만 섬 · 화남)
//! - 도시 목록 5줄: 청록 바탕 줄 = 고른 도시, 이름 앞 16×16 아이콘 = 그 도시의 상태 (평상이면 없음)
//! - 교역품 분류 목록 5줄: 청록 바탕 줄 = 고른 분류
//! - '도시 상태 [아이콘] 가뭄' 글자
//! - '재고 변동' 꺾은선: 흰 점 11개 (24px 간격). 세로축 맨 아래 0틱 ~ 맨 위 100틱 (161px)
//!
//! 문화권 공헌도와 NPC 대사는 읽지 않는다 (사용자 결정).
use crate::cv::{Gray, Mask, Rgb, match_max};
use crate::font::Gulim;
use crate::pngio;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Instant;

/// 지도·남만 표의 문화권 순서 (2026-11-18 사용자)
pub const CULTURES: [&str; 4] = ["일본", "화남", "조선", "대만 섬"];
/// 문화권별 도시 (게임 목록 순서)
pub const CITIES: [(&str, &[&str]); 4] = [
    ("일본", &["에도", "사카이", "나가사키"]),
    ("화남", &["마카오", "항주", "운대산", "천주", "중경"]),
    ("조선", &["한양", "부산", "포항"]),
    ("대만 섬", &["안평", "단수이"]),
];
/// 교역품 분류 (게임 목록 순서)
pub const KINDS: [&str; 20] = [
    "식료품", "조미료", "주류", "기호품", "향신료", "의약품", "섬유", "직물", "염료", "귀금속", "광석", "향료", "보석", "가축", "무기류", "총포류", "미술품", "공예품",
    "공업품", "잡화",
];
/// 도시 상태 (화면 글자). '보통' = 평상, 아이콘 없음. 전염병은 패치로 없어졌지만 글자가 나오면 그대로 읽는다
pub const STATES: [&str; 10] = ["보통", "전쟁", "전염병", "가뭄", "수해", "냉해", "불경기", "호경기", "조선 특수수요", "축제"];
/// 아이콘 파일 이름(영문) ↔ 상태
pub const ICON_FILES: [(&str, &str); 9] = [
    ("war", "전쟁"),
    ("plague", "전염병"),
    ("drought", "가뭄"),
    ("flood", "수해"),
    ("cold", "냉해"),
    ("slump", "불경기"),
    ("boom", "호경기"),
    ("joseon", "조선 특수수요"),
    ("festival", "축제"),
];

pub fn cities_of(culture: &str) -> &'static [&'static str] {
    CITIES.iter().find(|c| c.0 == culture).map_or(&[], |c| c.1)
}

pub fn culture_of(city: &str) -> Option<&'static str> {
    CITIES.iter().find(|c| c.1.contains(&city)).map(|c| c.0)
}

// 모서리 기준 위치 (게임 UI는 해상도와 관계없이 픽셀 크기가 같다)
const CULTURE_BOX: (i64, i64, i64, i64) = (22, 38, 224, 62);
const CITY_ROW0: i64 = 101;
const KIND_ROW0: i64 = 241;
const ROW_H: i64 = 20;
const ROWS: i64 = 5;
const LIST_X: (i64, i64) = (28, 233);
const ICON_X: i64 = 30;
const ICON_DY: i64 = 2;
const ICON: i64 = 16;
const NAME_X0: i64 = 47;
const STATE_BOX: (i64, i64, i64, i64) = (328, 58, 600, 82);
const AXIS_X: i64 = 331;
/// 0틱(바닥선)과 100틱(세로축 맨 위)의 y
const Y0: i64 = 285;
const Y100: i64 = 124;
const DOT_X0: i64 = 337;
const DOT_STEP: i64 = 24;
pub const POINTS: usize = 11;
/// 남만무역정보 창 너비 (모서리에서 오른쪽 금테 끝까지, 캡처 40: 655 → 1262)
pub const WIN_W: i64 = 607;
/// 틱 → 그래프 점의 세로 위치 (모서리 기준). 틱 HUD를 마지막 점 높이에 띄울 때 쓴다
pub fn tick_y(t: u8) -> i64 {
    Y0 - (t as i64 * (Y0 - Y100) + 50) / 100
}

/// 글자 맞추기 기준 (흰 글자 그림 vs 굴림 14px)
const TEXT_MIN: f32 = 0.75;
const TEXT_MARGIN: f32 = 0.05;
/// 아이콘: 16×16 평균 밝기 차이(0~255)가 이보다 작으면 같은 아이콘
const ICON_DIFF: f64 = 18.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NanbanScreen {
    pub culture: String,
    /// 고른 도시 (목록에서 청록 줄이 안 보이거나 못 읽으면 None)
    pub city: Option<String>,
    /// 고른 분류 (밀려서 안 보이면 None)
    pub kind: Option<String>,
    /// 지금 도시의 상태 글자 ('보통' 등)
    pub state: Option<String>,
    /// 점 11개 (틱 0~100, 왼쪽이 과거 → 마지막이 지금). 하나라도 못 읽으면 None
    pub pts: Option<[u8; POINTS]>,
    /// 목록에 보이는 도시마다 (도시, 상태). 아이콘을 모르면 상태 None
    pub list: Vec<(String, Option<String>)>,
}

pub struct NanbanReader {
    tpl: HashMap<String, Vec<Gray>>,
    /// (상태, 16×16 RGB) — 실행 파일에 넣은 것 + 이번에 배운 것
    icons: Vec<(String, Rgb)>,
    icon_dir: Option<std::path::PathBuf>,
    /// 마지막 read 에서 고른 도시 줄의 모르는 아이콘
    pending: Option<Rgb>,
    /// 빠른 읽기(read_fast): 마지막으로 다 읽은 화면 + 그때 왼쪽 위(문화권·도시 목록·도시 상태) 모양 + 시각
    full: Option<(NanbanScreen, u64, Instant)>,
    /// 고른 분류 줄의 글자 모양 → 분류 이름 (한 번 읽은 모양은 다시 맞추지 않는다)
    kind_cache: HashMap<u64, String>,
    pub log: Vec<String>,
}

fn white(rgb: &Rgb, th: u8) -> Mask {
    let g = rgb.min_channel();
    g.gt(th as f32)
}

fn teal_ratio(rgb: &Rgb) -> f64 {
    if rgb.is_empty() {
        return 0.0;
    }
    let n = rgb.data.chunks_exact(3).filter(|p| p[1] > 120 && p[2] > 110 && p[0] < 90).count();
    n as f64 / (rgb.w * rgb.h) as f64
}

/// 아이콘 그림의 점인가 (바탕이 아닌가): 남색 바탕·청록(고른 줄) 바탕이 아니고, 색이 진하거나 밝은 점.
/// 전쟁 아이콘(칼 두 자루)은 회색·흰색이라 '밝은 점'으로 잡는다 (2026-11-19: 색만 보다가 전쟁을 '평상'으로 읽었음)
fn icon_px(p: &[u8]) -> bool {
    let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
    let teal = p[0] < 110 && p[1] > 110 && p[2] > 110 && p[1] as i32 - p[0] as i32 > 30;
    !teal && (mx - mn > 60 || mn > 110)
}

/// 두 아이콘의 차이: 어느 한쪽이라도 그림 점인 곳만 비교 (바탕이 남색이든 청록이든 같게 본다)
fn icon_diff(a: &Rgb, b: &Rgb) -> f64 {
    if a.w != b.w || a.h != b.h || a.is_empty() {
        return f64::MAX;
    }
    let (mut s, mut n) = (0u64, 0u64);
    for (x, y) in a.data.chunks_exact(3).zip(b.data.chunks_exact(3)) {
        if icon_px(x) || icon_px(y) {
            s += x.iter().zip(y).map(|(u, v)| (*u as i32 - *v as i32).unsigned_abs() as u64).sum::<u64>();
            n += 3;
        }
    }
    if n == 0 { f64::MAX } else { s as f64 / n as f64 }
}

/// 아이콘 칸에 아이콘이 있나: 그림 점이 30개 넘게
fn has_icon(rgb: &Rgb) -> bool {
    rgb.data.chunks_exact(3).filter(|p| icon_px(p)).count() > 30
}


impl NanbanReader {
    pub fn new(icon_dir: Option<std::path::PathBuf>) -> Self {
        let mut font = Gulim::new(14).expect("굴림 글꼴을 만들 수 없음");
        let mut tpl = HashMap::new();
        let names = CULTURES.iter().chain(KINDS.iter()).chain(STATES.iter()).copied().chain(CITIES.iter().flat_map(|c| c.1.iter().copied()));
        for n in names {
            // PC마다 게임이 글자를 1~2px 더 띄워 그리기도 하고, 이 창은 빈칸도 넓게 그려서 간격별로 그려 둔다.
            // '조선 특수수요'는 빈칸 폭이 그때그때 달라 '특수수요'만으로도 맞춘다
            let mut parts = vec![n];
            if n == "조선 특수수요" {
                parts.push("특수수요");
            }
            let mut v: Vec<Gray> = vec![];
            for p in parts {
                for e in [0, 1, 2] {
                    for sp in if p.contains(' ') { &[0, 2, 4, 6][..] } else { &[0][..] } {
                        if let Some(t) = font.render(p, e, *sp) {
                            v.push(t.gt(100.0).to_gray());
                        }
                    }
                }
            }
            tpl.insert(n.to_string(), v);
        }
        let mut icons = vec![];
        for (file, png) in crate::assets::NANBAN {
            let stem = file.trim_end_matches(".png");
            if let (Some(state), Some(im)) = (ICON_FILES.iter().find(|f| f.0 == stem).map(|f| f.1), pngio::load_rgb(png)) {
                icons.push((state.to_string(), im));
            }
        }
        if let Some(d) = &icon_dir {
            // 실행하면서 배운 아이콘 (data/nanban_icons/{영문}_{번호}.png)
            if let Ok(rd) = std::fs::read_dir(d) {
                for e in rd.flatten() {
                    let p = e.path();
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").split('_').next().unwrap_or("").to_string();
                    if let (Some(state), Some(im)) = (ICON_FILES.iter().find(|f| f.0 == stem).map(|f| f.1), std::fs::read(&p).ok().and_then(|b| pngio::load_rgb(&b))) {
                        icons.push((state.to_string(), im));
                    }
                }
            }
        }
        NanbanReader { tpl, icons, icon_dir, pending: None, full: None, kind_cache: HashMap::new(), log: vec![] }
    }

    /// 흰 글자 그림에서 후보 중 가장 잘 맞는 이름 (확실하지 않으면 None)
    fn best(&self, m: &Mask, cands: &[&str]) -> Option<String> {
        let img = m.to_gray();
        let mut sc: Vec<(f32, &str)> = vec![];
        for &c in cands {
            let Some(v) = self.tpl.get(c) else { continue };
            let mut b: Option<f32> = None;
            for t in v {
                if t.h <= img.h && t.w <= img.w {
                    let s = match_max(&img, t);
                    b = Some(b.map_or(s, |x: f32| x.max(s)));
                }
            }
            if let Some(b) = b {
                sc.push((b, c));
            }
        }
        sc.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let (s0, n0) = *sc.first()?;
        let s1 = sc.get(1).map_or(0.0, |x| x.0);
        (s0 >= TEXT_MIN && s0 - s1 >= TEXT_MARGIN).then(|| n0.to_string())
    }

    /// 창이 남만무역정보 모양인가: 세로축 선(어두운 세로줄)이 0틱~100틱 사이에 이어져 있다
    pub fn looks_like(rgb: &Rgb, cx: i64, cy: i64) -> bool {
        let col = rgb.crop(cx + AXIS_X - 1, cy + Y100 + 4, cx + AXIS_X + 2, cy + Y0 - 4);
        if col.is_empty() || col.h < 100 {
            return false;
        }
        // 세 줄 중 가장 어두운 줄이 거의 전부 어두워야 한다 (축 선은 거의 검정)
        (0..3).any(|x| (0..col.h).filter(|&y| col.px(x, y).iter().map(|&v| v as u32).sum::<u32>() < 3 * 40).count() as f64 / col.h as f64 > 0.85)
    }

    /// 왼쪽 위(문화권·도시 목록·고른 도시 줄·아이콘 유무)와 '도시 상태' 칸의 모양. 이것이 같으면 그 부분은 다시 읽지 않는다
    fn left_sig(rgb: &Rgb, cx: i64, cy: i64) -> u64 {
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        for (x0, y0, x1, y1) in [CULTURE_BOX, (LIST_X.0, CITY_ROW0, LIST_X.1, CITY_ROW0 + ROWS * ROW_H), STATE_BOX] {
            white(&rgb.crop(cx + x0, cy + y0, cx + x1, cy + y1), 170).data.hash(&mut hs);
        }
        for k in 0..ROWS {
            let y = cy + CITY_ROW0 + k * ROW_H;
            (teal_ratio(&rgb.crop(cx + LIST_X.0, y, cx + LIST_X.1, y + ROW_H)) > 0.4).hash(&mut hs);
            has_icon(&rgb.crop(cx + ICON_X, y + ICON_DY, cx + ICON_X + ICON, y + ICON_DY + ICON)).hash(&mut hs);
        }
        hs.finish()
    }

    /// 고른 분류 (청록 줄) 이름. 같은 글자 모양은 기억해 두고 다시 맞추지 않는다
    fn read_kind(&mut self, rgb: &Rgb, cx: i64, cy: i64) -> Option<String> {
        for k in 0..ROWS {
            let y = cy + KIND_ROW0 + k * ROW_H;
            if teal_ratio(&rgb.crop(cx + LIST_X.0, y, cx + LIST_X.1, y + ROW_H)) > 0.4 {
                let m = white(&rgb.crop(cx + NAME_X0, y, cx + LIST_X.1, y + ROW_H), 170);
                let mut hs = std::collections::hash_map::DefaultHasher::new();
                (m.w, m.h, &m.data).hash(&mut hs);
                let key = hs.finish();
                if let Some(v) = self.kind_cache.get(&key) {
                    return Some(v.clone());
                }
                let v = self.best(&m, &KINDS)?;
                if self.kind_cache.len() > 2000 {
                    self.kind_cache.clear();
                }
                self.kind_cache.insert(key, v.clone());
                return Some(v);
            }
        }
        None
    }

    /// 빠른 읽기 (2026-11-19, 아래 방향키를 누르고 있으면 분류가 0.03초마다 바뀐다): 왼쪽 위 모양이 마지막으로 다 읽었을 때와 같고
    /// 3초가 안 지났으면 문화권·도시·상태는 그대로 쓰고, 고른 분류 한 줄과 점 11개만 읽는다 (다 읽기 약 15ms → 1ms 아래)
    pub fn read_fast(&mut self, rgb: &Rgb, cx: i64, cy: i64) -> Option<NanbanScreen> {
        if !Self::looks_like(rgb, cx, cy) {
            return None;
        }
        let sig = Self::left_sig(rgb, cx, cy);
        let base = match &self.full {
            Some((scr, s, at)) if *s == sig && at.elapsed().as_secs_f64() < 3.0 => scr.clone(),
            _ => return self.read(rgb, cx, cy),
        };
        Some(NanbanScreen { kind: self.read_kind(rgb, cx, cy), pts: Self::read_points(rgb, cx, cy), ..base })
    }

    pub fn read(&mut self, rgb: &Rgb, cx: i64, cy: i64) -> Option<NanbanScreen> {
        if !Self::looks_like(rgb, cx, cy) {
            return None;
        }
        let sig = Self::left_sig(rgb, cx, cy);
        let cb = rgb.crop(cx + CULTURE_BOX.0, cy + CULTURE_BOX.1, cx + CULTURE_BOX.2, cy + CULTURE_BOX.3);
        let culture = self.best(&white(&cb, 170), &CULTURES)?;
        let cities = cities_of(&culture);
        let mut scr = NanbanScreen { culture, city: None, kind: None, state: None, pts: None, list: vec![] };
        // 도시 목록
        let mut sel_icon: Option<Rgb> = None;
        for k in 0..ROWS {
            let y = cy + CITY_ROW0 + k * ROW_H;
            let row = rgb.crop(cx + LIST_X.0, y, cx + LIST_X.1, y + ROW_H);
            let name_part = rgb.crop(cx + NAME_X0, y, cx + LIST_X.1, y + ROW_H);
            let m = white(&name_part, 170);
            if m.count() < 15 {
                continue;
            }
            let Some(name) = self.best(&m, cities) else { continue };
            let icon = rgb.crop(cx + ICON_X, y + ICON_DY, cx + ICON_X + ICON, y + ICON_DY + ICON);
            let state = if has_icon(&icon) { self.icon_state(&icon) } else { Some("보통".to_string()) };
            if teal_ratio(&row) > 0.4 {
                scr.city = Some(name.clone());
                if has_icon(&icon) {
                    sel_icon = Some(icon);
                }
            }
            scr.list.push((name, state));
        }
        // 분류 목록
        scr.kind = self.read_kind(rgb, cx, cy);
        // 도시 상태 글자
        let sb = rgb.crop(cx + STATE_BOX.0, cy + STATE_BOX.1, cx + STATE_BOX.2, cy + STATE_BOX.3);
        scr.state = self.best(&white(&sb, 170), &STATES);
        // 고른 도시의 아이콘을 모르면 들고 있다가, 화면이 자리 잡은 뒤 상태 글자로 배운다 (learn_selected).
        // 바로 배우지 않는 까닭: 도시를 누른 직후엔 오른쪽 '도시 상태'가 아직 앞 도시 것이다 (2026-11-19)
        self.pending = sel_icon.filter(|i| self.icon_state(i).is_none());
        scr.pts = Self::read_points(rgb, cx, cy);
        self.full = Some((scr.clone(), sig, Instant::now()));
        Some(scr)
    }

    /// 마지막 read 에서 고른 도시의 아이콘을 몰랐으면 이 상태로 배운다. 배웠으면 true
    pub fn learn_selected(&mut self, state: &str) -> bool {
        match self.pending.take() {
            Some(icon) if state != "보통" && ICON_FILES.iter().any(|f| f.1 == state) => {
                self.learn_icon(state, &icon);
                true
            }
            _ => false,
        }
    }

    fn icon_state(&self, icon: &Rgb) -> Option<String> {
        let mut best: Option<(f64, &str)> = None;
        for (s, t) in &self.icons {
            let d = icon_diff(icon, t);
            if d < ICON_DIFF && best.is_none_or(|b| d < b.0) {
                best = Some((d, s));
            }
        }
        best.map(|b| b.1.to_string())
    }

    fn learn_icon(&mut self, state: &str, icon: &Rgb) {
        let Some(file) = ICON_FILES.iter().find(|f| f.1 == state).map(|f| f.0) else { return };
        self.icons.push((state.to_string(), icon.clone()));
        if let Some(d) = &self.icon_dir {
            let _ = std::fs::create_dir_all(d);
            let n = self.icons.iter().filter(|i| i.0 == state).count();
            let _ = pngio::save_rgb(&d.join(format!("{file}_{n}.png")), icon);
        }
        self.log.push(format!("[남만] 도시 상태 아이콘을 새로 배움: {state}"));
    }

    /// 점 11개 → 틱. 점마다 흰 화소(가장 어두운 색 칸 > 200)의 무게중심 높이를 잰다
    pub fn read_points(rgb: &Rgb, cx: i64, cy: i64) -> Option<[u8; POINTS]> {
        let mut out = [0u8; POINTS];
        for (i, o) in out.iter_mut().enumerate() {
            let x = cx + DOT_X0 + DOT_STEP * i as i64;
            let (y0, y1) = (cy + Y100 - 6, cy + Y0 + 6);
            let c = rgb.crop(x - 4, y0, x + 5, y1);
            if c.w < 9 || c.h as i64 != y1 - y0 {
                return None;
            }
            let (mut sw, mut sy) = (0.0f64, 0.0f64);
            let (mut top, mut bot) = (usize::MAX, 0usize);
            for yy in 0..c.h {
                let mut roww = 0.0;
                for xx in 0..c.w {
                    let p = c.px(xx, yy);
                    let v = p[0].min(p[1]).min(p[2]) as f64 - 200.0;
                    if v >= 20.0 {
                        roww += v;
                    }
                }
                if roww > 0.0 {
                    sw += roww;
                    sy += roww * yy as f64;
                    top = top.min(yy);
                    bot = bot.max(yy);
                }
            }
            // 점은 지름 8px 안팎: 흰 것이 위아래로 길게 퍼져 있으면(뒤의 흰 글자 등) 못 읽은 것으로 본다
            if sw < 800.0 || bot - top > 16 {
                return None;
            }
            let ym = y0 as f64 + sy / sw;
            let t = ((cy + Y0) as f64 - ym) / (Y0 - Y100) as f64 * 100.0;
            *o = t.round().clamp(0.0, 100.0) as u8;
        }
        Some(out)
    }
}
