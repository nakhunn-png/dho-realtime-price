//! 가격·% 숫자 읽기 (게임 숫자 글자 틀과 글자 단위 비교, Python `DigitReader`)
use crate::cv::{Gray, Rgb, ncc};
use crate::pngio;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    /// ',' '.' 'W'(화폐 기호 Ɖ 또는 %) 'P'(괄호) 'd'(숫자)
    pub kind: char,
    pub x0: usize,
    pub x1: usize,
    pub y0: usize,
    pub y1: usize,
}

pub struct DigitReader {
    /// 무손실 캡처에서 맞게 읽은 숫자는 0.99 이상, 잘못 읽은 숫자는 0.5~0.7 이었다
    pub min_score: f64,
    tpl: Vec<(char, Gray)>,
}

impl Default for DigitReader {
    fn default() -> Self {
        Self::new()
    }
}

impl DigitReader {
    pub fn new() -> Self {
        let tpl = crate::assets::DIGITS
            .iter()
            .filter_map(|(f, b)| Some((f.chars().next()?, pngio::load_l(b)?)))
            .collect();
        DigitReader { min_score: 0.8, tpl }
    }

    pub fn segments(strip: &Rgb) -> (Gray, Vec<Seg>) {
        let (w, h) = (strip.w, strip.h);
        // 빨간 글자(기준보다 비싼 가격)도 흰 글자처럼: R이 G·B보다 확실히 밝은 곳
        let data: Vec<f32> = strip
            .data
            .chunks_exact(3)
            .map(|p| {
                let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
                let red = if r - g.max(b) > 60.0 { r } else { 0.0 };
                r.min(g).min(b).max(red)
            })
            .collect();
        let g = Gray { w, h, data };
        let m = |x: usize, y: usize| g.at(x, y) > 140.0;
        if !(0..h).any(|y| (0..w).any(|x| m(x, y))) {
            return (g, vec![]);
        }
        let cols: Vec<bool> = (0..w).map(|x| (0..h).any(|y| m(x, y))).collect();
        let mut blobs = vec![];
        let mut x = 0;
        while x < w {
            if cols[x] {
                let s = x;
                while x < w && cols[x] {
                    x += 1;
                }
                let ys: Vec<usize> = (0..h).filter(|&y| (s..x).any(|xx| m(xx, y))).collect();
                blobs.push((s, x, ys[0], ys[ys.len() - 1] + 1));
            } else {
                x += 1;
            }
        }
        // 기준 높이는 키 큰 글자(숫자·괄호)로만: 쉼표가 줄 아래로 처져도 흔들리지 않게
        let hmax = blobs.iter().map(|b| b.3 - b.2).max().unwrap() as f64;
        let tall: Vec<(usize, usize)> = blobs.iter().filter(|b| (b.3 - b.2) as f64 > hmax * 0.6).map(|b| (b.2, b.3)).collect();
        let top = tall.iter().map(|t| t.0).min().unwrap() as f64;
        let bot = (tall.iter().map(|t| t.1).max().unwrap() - 1) as f64;
        let hh = bot - top + 1.0;
        let segs = blobs
            .into_iter()
            .map(|(s, x, y0, y1)| {
                let (bh, bw) = ((y1 - y0) as f64, x - s);
                let kind = if bh <= hh * 0.45 {
                    if y0 as f64 > top + hh * 0.5 { ',' } else { '.' }
                } else if bw >= 9 {
                    'W'
                } else if bw <= 3 && bh >= hh * 0.9 {
                    'P'
                } else {
                    'd'
                };
                Seg { kind, x0: s, x1: x, y0, y1 }
            })
            .collect();
        (g, segs)
    }

    pub fn classify(&self, glyph: &Gray) -> (char, f64) {
        let (mut best, mut bd) = (-1.0, '?');
        for (d, t) in &self.tpl {
            let s = ncc(glyph, t);
            if s > best {
                best = s;
                bd = *d;
            }
        }
        (bd, best)
    }

    pub fn glyph(g: &Gray, s: &Seg) -> Gray {
        g.crop(s.x0 as i64, s.y0 as i64, s.x1 as i64, s.y1 as i64)
    }

    /// 가격 숫자의 '1'은 폭이 좁아 괄호(P)로 분류될 때가 있다 ('10,500' → 'P0,500').
    /// 화폐 기호(W) 앞에는 괄호가 올 수 없으므로, 그 자리의 P가 숫자 1 모양이면 숫자로 고친다.
    pub fn fix_ones(&self, g: &Gray, segs: Vec<Seg>) -> Vec<Seg> {
        let mut seen_w = false;
        segs.into_iter()
            .map(|mut s| {
                if s.kind == 'W' {
                    seen_w = true;
                } else if s.kind == 'P' && !seen_w && self.classify(&Self::glyph(g, &s)).0 == '1' {
                    s.kind = 'd';
                }
                s
            })
            .collect()
    }

    /// 숫자 칸은 읽은 숫자로, 나머지는 종류 글자로 이은 문자열과 가장 낮은 숫자 일치도
    fn spell(&self, g: &Gray, segs: &[Seg], stop_after_parens: bool) -> (String, f64) {
        let (mut out, mut low, mut parens) = (String::new(), 1.0f64, 0);
        for s in segs {
            if stop_after_parens && parens >= 2 {
                break; // ')' 뒤의 시세 화살표 아이콘 등은 무시
            }
            if s.kind == 'P' {
                parens += 1;
            }
            if s.kind == 'd' {
                let (d, sc) = self.classify(&Self::glyph(g, s));
                low = low.min(sc);
                out.push(d);
            } else {
                out.push(s.kind);
            }
        }
        (out, low)
    }

    /// '2,097 Ɖ (100%)' → (2097, 100). 못 읽으면 None. min_score: 숫자 일치도 기준 (기본 self.min_score)
    pub fn read(&self, strip: &Rgb, min_score: Option<f64>) -> Option<(i64, i64)> {
        let (g, segs) = Self::segments(strip);
        let segs = self.fix_ones(&g, segs);
        let (s, low) = self.spell(&g, &segs, true);
        let (price, pct) = parse_price_pct(&s)?;
        // 애매한 숫자는 틀리게 읽느니 안 읽는다 (다음 화면에서 다시 읽음)
        if low < min_score.unwrap_or(self.min_score) {
            return None;
        }
        (0 < price && price < 10_000_000 && (1..=999).contains(&pct)).then_some((price, pct))
    }

    /// '(103%)' → 103. 못 읽으면 None
    pub fn read_pct(&self, strip: &Rgb) -> Option<i64> {
        let (g, segs) = Self::segments(strip);
        let (s, low) = self.spell(&g, &segs, false);
        // P(\d+)W+P
        let c: Vec<char> = s.chars().collect();
        if c.len() < 4 || c[0] != 'P' || *c.last()? != 'P' {
            return None;
        }
        let body = &c[1..c.len() - 1];
        let nd = body.iter().take_while(|c| c.is_ascii_digit()).count();
        if nd == 0 || nd == body.len() || !body[nd..].iter().all(|&c| c == 'W') {
            return None;
        }
        let v: i64 = body[..nd].iter().collect::<String>().parse().ok()?;
        (low >= self.min_score && (1..=999).contains(&v)).then_some(v)
    }
}

/// 정규식 `([\d,]+)W+P(\d+)W+P?` 전체 일치 → (가격, %)
pub fn parse_price_pct(s: &str) -> Option<(i64, i64)> {
    let c: Vec<char> = s.chars().collect();
    let mut i = c.iter().take_while(|c| c.is_ascii_digit() || **c == ',').count();
    if i == 0 {
        return None;
    }
    let price: String = c[..i].iter().filter(|&&c| c != ',').collect();
    let w = c[i..].iter().take_while(|&&c| c == 'W').count();
    if w == 0 {
        return None;
    }
    i += w;
    if c.get(i) != Some(&'P') {
        return None;
    }
    i += 1;
    let nd = c[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    if nd == 0 {
        return None;
    }
    let pct: String = c[i..i + nd].iter().collect();
    i += nd;
    let w = c[i..].iter().take_while(|&&c| c == 'W').count();
    if w == 0 {
        return None;
    }
    i += w;
    if c.get(i) == Some(&'P') {
        i += 1;
    }
    if i != c.len() {
        return None;
    }
    Some((price.parse().ok()?, pct.parse().ok()?))
}

/// `\d{1,3}(,\d{3})*` 전체 일치
pub fn is_grouped_number(s: &str) -> bool {
    let parts: Vec<&str> = s.split(',').collect();
    let dig = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
    dig(parts[0]) && parts[0].len() <= 3 && parts[1..].iter().all(|p| dig(p) && p.len() == 3)
}
