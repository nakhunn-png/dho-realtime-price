//! 확실히 읽은 행의 품목 아이콘을 모아 두고, 이름이 애매할 때 아이콘으로 가린다
//! (Python `IconBook`, PC마다 data/icons)
use crate::cv::{Rgb, match_ccoeff_normed_ch, max_loc};
use crate::pngio;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 대화상자 모서리 기준, 첫 행 아이콘의 왼쪽 위 (오른쪽 아래 수량 숫자는 뺌)
pub const ICON_BOX: (i64, i64, i64, i64) = (34, 53, 70, 83);
pub const ICON_PAD: i64 = 2;
/// 이 점수 이상으로 이름을 읽은 (선택 안 된) 행의 아이콘만 저장
pub const ICON_SURE: f64 = 0.95;
/// 저장한 아이콘과 이 이상 맞아야 같은 품목
pub const ICON_MATCH: f64 = 0.8;

pub struct IconBook {
    folder: Option<PathBuf>,
    pub icons: BTreeMap<String, Rgb>,
}

impl IconBook {
    pub fn new(folder: Option<PathBuf>) -> Self {
        let mut icons = BTreeMap::new();
        if let Some(rd) = folder.as_ref().and_then(|f| std::fs::read_dir(f).ok()) {
            for e in rd.flatten() {
                let p = e.path();
                let Some(name) = p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".png")) else {
                    continue;
                };
                // 깨진 파일은 무시
                if let Some(im) = std::fs::read(&p).ok().and_then(|b| pngio::load_rgb(&b)) {
                    icons.insert(name.to_string(), im);
                }
            }
        }
        IconBook { folder, icons }
    }

    pub fn crop(rgb: &Rgb, cx: i64, cy: i64, dy: i64, pad: i64) -> Rgb {
        let (x0, y0, x1, y1) = ICON_BOX;
        rgb.crop(cx + x0 - pad, cy + y0 + dy - pad, cx + x1 + pad, cy + y1 + dy + pad)
    }

    pub fn learn(&mut self, name: &str, icon: &Rgb) {
        let (x0, y0, x1, y1) = ICON_BOX;
        if self.icons.contains_key(name) || icon.h as i64 != y1 - y0 || icon.w as i64 != x1 - x0 {
            return;
        }
        self.icons.insert(name.to_string(), icon.clone());
        if let Some(f) = &self.folder {
            if !name.chars().any(|c| "\\/:*?\"<>|".contains(c)) {
                let _ = std::fs::create_dir_all(f);
                let _ = pngio::save_rgb(&f.join(format!("{name}.png")), icon);
            }
        }
    }

    pub fn score(&self, name: &str, area: &Rgb) -> Option<f64> {
        let t = self.icons.get(name)?;
        if area.h < t.h || area.w < t.w {
            return None;
        }
        let a = [area.channel(0), area.channel(1), area.channel(2)];
        let b = [t.channel(0), t.channel(1), t.channel(2)];
        let r = match_ccoeff_normed_ch(&[&a[0], &a[1], &a[2]], &[&b[0], &b[1], &b[2]]);
        Some(max_loc(&r).0 as f64)
    }

    /// 저장한 아이콘과 맞는 이름 하나. 없거나 둘 이상 맞으면 None
    pub fn pick(&self, names: &[String], area: &Rgb) -> Option<String> {
        let hits: Vec<&String> = names.iter().filter(|n| self.score(n, area).is_some_and(|s| s >= ICON_MATCH)).collect();
        (hits.len() == 1).then(|| hits[0].clone())
    }
}
