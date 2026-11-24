//! 교역소 주인 대사를 굴림 14 글자 모양으로 읽는다 (Windows 글자 인식보다 먼저).
//!
//! 채팅 글자도 굴림 14 비트맵이라, 정해진 대사(`text::owner_canon`)를 그려서 화면과 점 단위로 맞추면
//! OCR처럼 '어서 오세요'를 '어서 왜요'로 읽는 일이 없다 (2026-11-21 사그레스 스크린샷: 맞춤 0.99).
//! 빈칸 폭은 PC마다 달라서 낱말 단위로 맞추고, 낱말 사이는 조금 떨어져 있어도 된다.
use crate::cv::{Gray, Rgb, match_ccoeff_normed, max_loc};
use crate::font::Gulim;
use crate::text::owner_canon;
use std::collections::HashMap;

/// 말한 사람 (낱말들)
const SPEAKERS: [&[&str]; 2] = [&["교역소주인"], &["교역소의", "도제"]];
/// 주인이 하는 말이지만 시세 소식이 아닌 것 (맞으면 그 줄은 조용히 넘긴다)
const NOT_NEWS: [&str; 1] = ["고맙네."];
/// 말한 사람 이름이 이만큼 맞으면 그 줄은 주인 말
const SPEAKER_MIN: f32 = 0.9;
/// 대사 낱말이 모두 이만큼 맞아야 그 대사
const WORD_MIN: f32 = 0.88;
/// 말한 사람 이름은 채팅 줄 맨 앞에 있다 (시작 x 가 이 안)
const SPEAKER_X_MAX: usize = 200;
/// 낱말 끝 → 다음 낱말 시작 사이 (빈칸, 떼어 낸 '..' 포함)
const WORD_GAP: i64 = 30;
/// 이름 끝 → 대사 시작 사이 (' : ' 가 멀리 떨어지기도 한다)
const SAY_GAP: i64 = 220;

/// 한 줄 결과: (줄 위치 y, 맞춘 정식 대사 — 못 맞추면 None)
pub type Row = (i64, Option<String>);

pub struct OwnerFont {
    font: Gulim,
    /// (정식 대사, 문장부호를 뺀 낱말들, 시세 소식인가)
    canon: Vec<(String, Vec<String>, bool)>,
    tpl: HashMap<(i32, String), Option<Gray>>,
}

fn words_of(s: &str) -> Vec<String> {
    s.split(' ').map(|w| w.chars().filter(|c| !".!?,".contains(*c)).collect::<String>()).filter(|w| !w.is_empty()).collect()
}

impl OwnerFont {
    pub fn new() -> Option<Self> {
        let mut canon: Vec<(String, Vec<String>, bool)> = owner_canon().into_iter().map(|c| { let w = words_of(&c); (c, w, true) }).collect();
        canon.extend(NOT_NEWS.iter().map(|c| (c.to_string(), words_of(c), false)));
        Some(OwnerFont { font: Gulim::new(14)?, canon, tpl: HashMap::new() })
    }

    fn tpl(&mut self, extra: i32, w: &str) -> Option<Gray> {
        let font = &mut self.font;
        self.tpl.entry((extra, w.to_string())).or_insert_with(|| font.render(w, extra, 0).map(|t| t.gt(100.0).to_gray())).clone()
    }

    /// 채팅창 그림에서 주인 말 줄을 위→아래로 (소식이 아닌 말은 뺀다). 주인 이름을 못 찾으면 None.
    /// 글자 간격은 `extras` 순서로 보고, 주인 이름이 보인 첫 간격을 쓴다
    pub fn read(&mut self, crop: &Rgb, extras: &[i32]) -> Option<Vec<Row>> {
        // 주인 말은 흰 글자 (시스템 안내는 하늘색이라 빠진다)
        let m = crop.min_channel().gt(150.0).to_gray();
        for &e in extras {
            if let Some(rows) = self.read_at(&m, e) {
                return Some(rows);
            }
        }
        None
    }

    fn read_at(&mut self, m: &Gray, e: i32) -> Option<Vec<Row>> {
        // 1) 말한 사람: 첫 낱말을 줄 맨 앞에서 찾는다
        let mut hits: Vec<(i64, i64, usize)> = vec![]; // (x, y, 말한 사람 번호)
        for (si, sp) in SPEAKERS.iter().enumerate() {
            let Some(t) = self.tpl(e, sp[0]) else { continue };
            let left = m.crop(0, 0, (SPEAKER_X_MAX + t.w) as i64, m.h as i64);
            if t.w > left.w || t.h > left.h {
                continue;
            }
            let r = match_ccoeff_normed(&left, &t);
            let mut pts: Vec<(f32, i64, i64)> = vec![];
            for y in 0..r.h {
                for x in 0..r.w {
                    let s = r.at(x, y);
                    if s >= SPEAKER_MIN {
                        pts.push((s, x as i64, y as i64));
                    }
                }
            }
            pts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            for (_, x, y) in pts {
                if !hits.iter().any(|h| (h.1 - y).abs() < 8) {
                    hits.push((x, y, si));
                }
            }
        }
        if hits.is_empty() {
            return None;
        }
        hits.sort_by_key(|h| h.1);
        // 2) 줄마다: 이름 나머지 낱말 → 대사
        let mut rows = vec![];
        for (x, y, si) in hits {
            // 글자 윗줄 기준 위 4px ~ 아래 17px (위아래 줄 글자는 안 들어온다, 줄 간격 20px)
            let band = m.crop(0, y - 4, m.w as i64, y + 17);
            let mut memo = HashMap::new();
            let sp: Vec<String> = SPEAKERS[si].iter().map(|s| s.to_string()).collect();
            let Some((_, end)) = self.chain(&band, e, &sp, x - 1, x + 1, &mut memo) else { continue };
            let mut scored: Vec<(f32, usize)> = vec![];
            for i in 0..self.canon.len() {
                let words = self.canon[i].1.clone();
                if let Some((s, _)) = self.chain(&band, e, &words, end + 1, end + SAY_GAP, &mut memo) {
                    scored.push((s, i));
                }
            }
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            // 1·2등이 거의 같으면(다른 대사) 어느 쪽인지 모른다
            let pick = match scored.as_slice() {
                [] => None,
                [a] => Some(a.1),
                [a, b, ..] => (a.0 - b.0 >= 0.02).then_some(a.1),
            };
            match pick.map(|i| &self.canon[i]) {
                Some((_, _, false)) => {} // 소식이 아닌 말
                Some((c, _, true)) => rows.push((y, Some(c.clone()))),
                None => rows.push((y, None)),
            }
        }
        Some(rows)
    }

    /// 낱말들을 왼쪽부터 차례로 맞춘다. 첫 낱말 시작 x 는 [xa, xb]. (가장 낮은 낱말 점수, 마지막 낱말 끝 x)
    fn chain(&mut self, band: &Gray, e: i32, words: &[String], xa: i64, xb: i64, memo: &mut HashMap<(String, i64, i64), Option<(f32, i64)>>) -> Option<(f32, i64)> {
        let (mut lo, mut hi, mut low, mut end) = (xa, xb, 1.0f32, xa);
        for w in words {
            let key = (w.clone(), lo, hi);
            let found = match memo.get(&key) {
                Some(v) => *v,
                None => {
                    let v = self.tpl(e, w).and_then(|t| find(band, &t, lo, hi));
                    memo.insert(key, v);
                    v
                }
            };
            let (s, ex) = found?;
            if s < WORD_MIN {
                return None;
            }
            low = low.min(s);
            end = ex;
            (lo, hi) = (ex + 1, ex + WORD_GAP);
        }
        Some((low, end))
    }
}

/// 낱말 틀이 [xa, xb] 에서 시작하는 가장 잘 맞는 곳 (점수, 끝 x)
fn find(band: &Gray, t: &Gray, xa: i64, xb: i64) -> Option<(f32, i64)> {
    let x0 = xa.max(0);
    let x1 = (xb + t.w as i64).min(band.w as i64);
    if x1 - x0 < t.w as i64 || band.h < t.h {
        return None;
    }
    let r = match_ccoeff_normed(&band.crop(x0, 0, x1, band.h as i64), t);
    let (s, x, _) = max_loc(&r);
    Some((s, x0 + (x + t.w) as i64))
}
