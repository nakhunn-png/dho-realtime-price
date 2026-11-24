//! '(108%)' 옆 시세 화살표 (Python `ArrowReader`). 게임이 쓰는 그림이 정해져 있으므로 그림째로 맞춘다.
use crate::cv::{Rgb, sqdiff_masked_min};
use crate::pngio;

/// (종류 up/flat/down/vup(↑)/vdown(↓)/?, 색 #rrggbb)
pub type Arrow = (String, String);

pub fn arrow_color(kind: &str) -> &'static str {
    match kind {
        "up" => "#d17821",
        "flat" => "#92d61f",
        "down" => "#16d0b7",
        _ => "#ffffff",
    }
}

struct Tpl {
    kind: String,
    w: usize,
    h: usize,
    rgb: Vec<[f32; 3]>,
    mask: Vec<bool>,
    n: f64,
}

pub struct ArrowReader {
    tpl: Vec<Tpl>,
}

impl Default for ArrowReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ArrowReader {
    pub fn new() -> Self {
        let tpl = crate::assets::ARROWS
            .iter()
            .filter_map(|(f, b)| {
                let (w, h, px) = pngio::load_rgba(b)?;
                let mask: Vec<bool> = px.iter().map(|p| p[3] > 0).collect();
                Some(Tpl {
                    kind: f.trim_end_matches(".png").to_string(),
                    w,
                    h,
                    rgb: px.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect(),
                    n: mask.iter().filter(|&&m| m).count() as f64,
                    mask,
                })
            })
            .collect();
        ArrowReader { tpl }
    }

    /// (종류, 색). 처음 보는 화살표는 '?'. 화살표가 없으면 None
    pub fn read(&self, region: &Rgb) -> Option<Arrow> {
        let (mut best, mut kind) = (1e9f64, None);
        for t in &self.tpl {
            if t.h > region.h || t.w > region.w {
                continue;
            }
            let r = sqdiff_masked_min(region, &t.rgb, t.w, t.h, &t.mask);
            let v = (r.max(0.0) / (t.n * 3.0)).sqrt();
            if v < best {
                best = v;
                kind = Some(&t.kind);
            }
        }
        if let Some(k) = kind {
            if best < 35.0 {
                return Some((k.clone(), arrow_color(k).to_string()));
            }
        }
        // 처음 보는 화살표(크기는 화살표만 함, 선택 행 바탕은 훨씬 넓음): 색만 기록
        let sat: Vec<[u8; 3]> = region
            .data
            .chunks_exact(3)
            .filter(|p| {
                let (mx, mn) = (p[0].max(p[1]).max(p[2]) as i32, p[0].min(p[1]).min(p[2]) as i32);
                mx - mn > 70 && mx > 110
            })
            .map(|p| [p[0], p[1], p[2]])
            .collect();
        if (25..=150).contains(&sat.len()) {
            let med = |c: usize| -> i64 {
                let mut v: Vec<i64> = sat.iter().map(|p| p[c] as i64).collect();
                v.sort();
                let n = v.len();
                if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2 }
            };
            let (r, g, b) = (med(0), med(1), med(2));
            // 270% 넘는 시세: 게임은 똑바로 선 빨간 ↑ · 파란 ↓ 를 쓴다. 그림을 잘라 넣지 않고 색으로 가린다 (2026-11-17 사용자)
            // 실제로 본 색: 빨강 #d44b51, 파랑 #1695de. 다른 색(노랑 #fee247 등)은 그대로 '?'
            let kind = if r > 150 && g < 130 && b < 130 {
                "vup"
            } else if b > 150 && r < 100 {
                "vdown"
            } else {
                "?"
            };
            return Some((kind.into(), format!("#{r:02x}{g:02x}{b:02x}")));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 그림 틀에 없는 화살표: 빨강이면 ↑(vup), 파랑이면 ↓(vdown), 다른 색은 '?'
    #[test]
    fn vertical_by_color() {
        let r = ArrowReader::new();
        let make = |c: [u8; 3]| {
            let (w, h) = (30usize, 26usize);
            let mut d = vec![20u8; w * h * 3];
            for y in 6..18 {
                for x in 12..17 {
                    d[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&c);
                }
            }
            Rgb::new(w, h, d)
        };
        assert_eq!(r.read(&make([0xd4, 0x4b, 0x51])).unwrap().0, "vup");
        assert_eq!(r.read(&make([0x16, 0x95, 0xde])).unwrap().0, "vdown");
        assert_eq!(r.read(&make([0xfe, 0xe2, 0x47])).unwrap().0, "?");
    }
}
