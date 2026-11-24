//! 측량 좌표 읽기 (네비) — collector/nav.py 의 Rust 판.
//!
//! 측량을 켜면 게임 화면 오른쪽 아래(오른쪽에서 약 10px, 아래에서 약 270px)에 '14351,2850' 처럼 배의 좌표가
//! 흰 글자(검은 테두리)로 나온다. 좌표가 읽히면 측량이 켜진 것으로 본다.
//! 글자는 9px 높이의 픽셀 글꼴이라 모양이 늘 같다: 흰 점만 골라 덩어리로 나눈 뒤 글자 모양과 비교한다.
//! 나침반(노란 N 장식)으로 카메라 방향, 측량 지도의 초록 삼각형으로 뱃머리 방향도 읽는다.
//! 상수·문턱은 모두 nav.py 와 같다 (그쪽 주석에 잰 날짜·이유가 있다).
use crate::cv::Rgb;

const GLYPHS_SRC: [(char, &str); 11] = [
    ('0', ".##.|#..#|#..#|#..#|#..#|#..#|#..#|#..#|.##."),
    ('1', ".#|##|.#|.#|.#|.#|.#|.#|.#"),
    ('2', ".##.|#..#|#..#|...#|..#.|.#..|.#..|#...|####"),
    ('3', ".##.|#..#|#..#|...#|.##.|...#|#..#|#..#|.##."),
    ('4', "...#.|..##.|..##.|.#.#.|.#.#.|#..#.|#####|...#.|...#."),
    ('5', "####|#...|#...|###.|#..#|...#|#..#|#..#|.##."),
    ('6', ".##.|#..#|#..#|#...|###.|#..#|#..#|#..#|.##."),
    ('7', "####|...#|...#|..#.|..#.|..#.|.#..|.#..|.#.."),
    ('8', ".##.|#..#|#..#|#..#|.##.|#..#|#..#|#..#|.##."),
    ('9', ".##.|#..#|#..#|#..#|.###|...#|#..#|#..#|.##."),
    (',', "###|..#|##."),
];
const DIGIT_H: usize = 9;
const COMMA_DY: usize = 7;
const AREA_W: i64 = 220;
const AREA_FROM_BOTTOM: (i64, i64) = (310, 230);
const WHITES: [u8; 3] = [200, 160, 130];
const MAX_DIFF: usize = 2;
const CHAR_GAP: i64 = 5;
const MAX_X: u32 = 16384;
const MAX_Y: u32 = 8192;

const COMPASS_CX: f64 = -94.05;
const COMPASS_CY: f64 = -71.23;
const COMPASS_RX: f64 = 52.0;
const COMPASS_RY: f64 = 43.42;
const COMPASS_BOX: (usize, usize) = (200, 160);
const COMPASS_MIN_PX: usize = 20;

const BOW_CX: i64 = -81;
const BOW_CY: i64 = -215;
const BOW_HALF: i64 = 22;
const BOW_MIN_PX: usize = 8;

/// (글자, 폭, 높이, 점들)
fn glyphs() -> &'static Vec<(char, usize, usize, Vec<bool>)> {
    static G: std::sync::OnceLock<Vec<(char, usize, usize, Vec<bool>)>> = std::sync::OnceLock::new();
    G.get_or_init(|| {
        GLYPHS_SRC
            .iter()
            .map(|(ch, s)| {
                let rows: Vec<&str> = s.split('|').collect();
                let w = rows[0].len();
                (*ch, w, rows.len(), rows.iter().flat_map(|r| r.chars().map(|c| c == '#')).collect())
            })
            .collect()
    })
}

/// 덩어리 하나 → 글자 (같은 크기 글자와 MAX_DIFF 점 이하로 다르면). 같은 차이면 앞 글자
fn match_glyph(w: usize, h: usize, mask: &[bool]) -> Option<char> {
    let mut best = None;
    let mut best_d = MAX_DIFF + 1;
    for (ch, gw, gh, g) in glyphs() {
        if *gw != w || *gh != h {
            continue;
        }
        let d = g.iter().zip(mask).filter(|(a, b)| a != b).count();
        if d < best_d {
            best = Some(*ch);
            best_d = d;
        }
    }
    best
}

/// 덩어리 하나의 테두리 상자와 점 수 (OpenCV connectedComponentsWithStats 의 stats 한 줄)
struct Comp {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    area: usize,
}

/// 8방향 이웃으로 덩어리 나누기. 번호는 1부터 (가로줄 순서로 처음 만난 순, OpenCV와 같은 순서)
fn components(m: &[bool], w: usize, h: usize) -> (Vec<u32>, Vec<Comp>) {
    let mut lab = vec![0u32; w * h];
    let mut comps = vec![];
    let mut stack = vec![];
    for start in 0..w * h {
        if !m[start] || lab[start] != 0 {
            continue;
        }
        let id = comps.len() as u32 + 1;
        lab[start] = id;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut area) = (usize::MAX, usize::MAX, 0, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            area += 1;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if m[j] && lab[j] == 0 {
                        lab[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        comps.push(Comp { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1, area });
    }
    (lab, comps)
}

/// 덩어리 `id` 의 상자 안 모양 (`lab[y:y+h, x:x+w] == id`)
fn comp_mask(lab: &[u32], w: usize, c: &Comp, id: u32) -> Vec<bool> {
    let mut v = Vec::with_capacity(c.w * c.h);
    for y in c.y..c.y + c.h {
        for x in c.x..c.x + c.w {
            v.push(lab[y * w + x] == id);
        }
    }
    v
}

/// 게임 화면(클라이언트) 부분: `rgb[oy:, ox:]`
fn client(rgb: &Rgb, off: (i64, i64)) -> Rgb {
    rgb.crop(off.0, off.1, rgb.w as i64, rgb.h as i64)
}

/// 게임 화면에서 측량 좌표 (x, y)를 읽는다. 측량이 꺼져 있거나 못 읽으면 None.
pub fn read_coords(rgb: &Rgb, off: (i64, i64)) -> Option<(u32, u32)> {
    let cl = client(rgb, off);
    let (w, h) = (cl.w as i64, cl.h as i64);
    let y0 = (h - AREA_FROM_BOTTOM.0).max(0);
    let y1 = (h - AREA_FROM_BOTTOM.1).max(0);
    let x0 = (w - AREA_W).max(0);
    let reg = cl.crop(x0, y0, w, y1);
    if reg.is_empty() {
        return None;
    }
    let lo: Vec<u8> = reg.data.chunks_exact(3).map(|p| p[0].min(p[1]).min(p[2])).collect();
    WHITES.iter().find_map(|&white| read_at(&lo, reg.w, reg.h, white))
}

fn read_at(lo: &[u8], w: usize, h: usize, white: u8) -> Option<(u32, u32)> {
    let m: Vec<bool> = lo.iter().map(|&v| v > white).collect();
    if !m.iter().any(|&b| b) {
        return None;
    }
    let (lab, comps) = components(&m, w, h);
    // 숫자(높이 9)와 쉼표(높이 3) 덩어리만 글자로 본다
    let mut digits = vec![]; // (x, y, 폭, 글자)
    let mut commas = vec![]; // (x, y, 폭)
    for (k, c) in comps.iter().enumerate() {
        let id = k as u32 + 1;
        if c.h == DIGIT_H && (2..=5).contains(&c.w) {
            if let Some(ch) = match_glyph(c.w, c.h, &comp_mask(&lab, w, c, id)) {
                if ch != ',' {
                    digits.push((c.x, c.y, c.w, ch));
                }
            }
        } else if c.h == 3 && c.w == 3 && match_glyph(3, 3, &comp_mask(&lab, w, c, id)) == Some(',') {
            commas.push((c.x, c.y, c.w));
        }
    }
    if digits.is_empty() || commas.is_empty() {
        return None;
    }
    // 같은 줄(윗줄이 같은) 숫자끼리 묶고 그 줄의 쉼표를 넣어 왼쪽부터 읽는다. 가장 오른쪽 줄 하나만 본다
    let mut tops: Vec<usize> = digits.iter().map(|d| d.1).collect();
    tops.sort_unstable();
    tops.dedup();
    let mut best: Option<(usize, String)> = None;
    for top in tops {
        let mut row: Vec<(usize, usize, char)> = digits.iter().filter(|d| d.1 == top).map(|d| (d.0, d.2, d.3)).collect();
        row.extend(commas.iter().filter(|c| c.1 == top + COMMA_DY).map(|c| (c.0, c.2, ',')));
        row.sort();
        // 오른쪽 끝에서부터 글자 사이가 CHAR_GAP px 이하로 이어진 것만
        let mut chain = vec![*row.last().unwrap()];
        for g in row[..row.len() - 1].iter().rev() {
            if chain[0].0 as i64 - (g.0 + g.1) as i64 > CHAR_GAP {
                break;
            }
            chain.insert(0, *g);
        }
        let text: String = chain.iter().map(|c| c.2).collect();
        let right = chain.last().unwrap().0;
        if best.as_ref().is_none_or(|b| right > b.0) {
            best = Some((right, text));
        }
    }
    let text = best?.1;
    let (a, b) = text.split_once(',')?;
    if b.contains(',') || !(1..=5).contains(&a.len()) || !(1..=4).contains(&b.len()) {
        return None;
    }
    let (x, y) = (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?);
    if x >= MAX_X || y >= MAX_Y {
        return None;
    }
    Some((x, y))
}

/// Python round(v, 1) (가까운 쪽, 딱 가운데는 짝수 쪽 — 실수에서는 거의 없다)
fn round1(v: f64) -> f64 {
    let r = (v * 10.0).round_ties_even() / 10.0;
    if r == 0.0 { 0.0 } else { r }
}

/// 나침반의 노란 N 장식 → 배(카메라)가 바라보는 방향 (도, 북 0·동 90). 못 읽으면 None.
pub fn read_heading(rgb: &Rgb, off: (i64, i64)) -> Option<f64> {
    let cl = client(rgb, off);
    let (bw, bh) = COMPASS_BOX;
    if cl.h < bh || cl.w < bw {
        return None;
    }
    let a = cl.crop((cl.w - bw) as i64, (cl.h - bh) as i64, cl.w as i64, cl.h as i64);
    let (mut sx, mut sy, mut n) = (0f64, 0f64, 0usize);
    for y in 0..a.h {
        for x in 0..a.w {
            let [r, g, b] = a.px(x, y);
            if r > 180 && g > 160 && b < 110 {
                sx += x as f64;
                sy += y as f64;
                n += 1;
            }
        }
    }
    if n < COMPASS_MIN_PX {
        return None;
    }
    let dx = (sx / n as f64 - bw as f64 - COMPASS_CX) / COMPASS_RX;
    let dy = (sy / n as f64 - bh as f64 - COMPASS_CY) / COMPASS_RY;
    let r = (dx * dx + dy * dy).sqrt();
    if !(0.7..=1.3).contains(&r) {
        return None; // 테 위가 아니면 (다른 노란 것) 버림
    }
    let phi = dx.atan2(-dy).to_degrees();
    Some(round1((360.0 - phi).rem_euclid(360.0)))
}

/// 측량 지도의 초록 삼각형 → 뱃머리 방향 (도, 북 0·동 90). 못 읽으면 None.
pub fn read_bow(rgb: &Rgb, off: (i64, i64)) -> Option<f64> {
    let cl = client(rgb, off);
    let (w, h) = (cl.w as i64, cl.h as i64);
    let (x0, y0) = (w + BOW_CX - BOW_HALF, h + BOW_CY - BOW_HALF);
    if x0 < 0 || y0 < 0 {
        return None;
    }
    let a = cl.crop(x0, y0, x0 + 2 * BOW_HALF, y0 + 2 * BOW_HALF);
    if a.is_empty() {
        return None;
    }
    let px: Vec<(i32, i32, i32)> = a.data.chunks_exact(3).map(|p| (p[0] as i32, p[1] as i32, p[2] as i32)).collect();
    let m: Vec<bool> = px.iter().map(|&(r, g, b)| g > 100 && g > r + 40 && g > b + 15).collect();
    let (lab, comps) = components(&m, a.w, a.h);
    if comps.is_empty() {
        return None;
    }
    // 가장 큰 초록 덩어리 (같으면 앞 번호)
    let mut k = 0;
    for (i, c) in comps.iter().enumerate() {
        if c.area > comps[k].area {
            k = i;
        }
    }
    let id = k as u32 + 1;
    let pts: Vec<(f64, f64, f64)> = (0..a.w * a.h)
        .filter(|&i| lab[i] == id)
        .map(|i| {
            let (r, g, b) = px[i];
            ((i % a.w) as f64, (i / a.w) as f64, (g - r.max(b)) as f64)
        })
        .collect();
    if pts.len() < BOW_MIN_PX {
        return None;
    }
    let tw: f64 = pts.iter().map(|p| p.2).sum();
    let (mx, my) = pts.iter().fold((0.0, 0.0), |(sx, sy), p| (sx + p.0 * p.2 / tw, sy + p.1 * p.2 / tw));
    let (mut cxx, mut cyy, mut cxy) = (0.0, 0.0, 0.0);
    for p in &pts {
        let (dx, dy, wt) = (p.0 - mx, p.1 - my, p.2 / tw);
        cxx += wt * dx * dx;
        cyy += wt * dy * dy;
        cxy += wt * dx * dy;
    }
    let th = 0.5 * (2.0 * cxy).atan2(cxx - cyy); // 긴 축
    let (mut ux, mut uy) = (th.cos(), th.sin());
    let skew: f64 = pts.iter().map(|p| (p.2 / tw) * ((p.0 - mx) * ux + (p.1 - my) * uy).powi(3)).sum();
    if skew < 0.0 {
        ux = -ux; // 꼭짓점 쪽(뾰족한 쪽)으로
        uy = -uy;
    }
    Some(round1(ux.atan2(-uy).to_degrees().rem_euclid(360.0)))
}
