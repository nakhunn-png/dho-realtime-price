//! 후보 이름을 굴림체로 그려 두고 화면 글자와 비교한다 (Python `NameMatcher`).
use crate::cv::{Gray, ncc};
use crate::font::Gulim;
use std::collections::HashMap;

/// 넓게 그린 빈칸의 추가 폭(px) ('타네가시마 총')
pub const SPACE_WIDE: i32 = 2;

pub struct NameMatcher {
    font: Gulim,
    /// 글자 사이 추가 간격(px). PC마다 게임이 글자를 2px 더 띄워 그리기도 한다
    pub extra: i32,
    pub names: Vec<String>,
    index: HashMap<String, usize>,
    /// 이름 순서대로 (아무것도 안 그려지면 None)
    pub tpl: Vec<Option<Gray>>,
    /// 띄어쓰기가 있는 이름만: 빈칸을 2px 넓게 그린 틀
    pub wide: Vec<Option<Gray>>,
}

/// 점수 높은 순 [(점수, 이름)]
pub type Ranked = Vec<(f64, String)>;

impl NameMatcher {
    pub fn new<S: AsRef<str>>(names: &[S]) -> Self {
        let font = Gulim::new(14).expect("굴림 글꼴을 만들 수 없음");
        let mut m = NameMatcher { font, extra: 0, names: vec![], index: HashMap::new(), tpl: vec![], wide: vec![] };
        m.set_names(names);
        m
    }

    pub fn set_names<S: AsRef<str>>(&mut self, names: &[S]) {
        self.names.clear();
        self.index.clear();
        for n in names.iter().map(|s| s.as_ref()).filter(|s| !s.is_empty()) {
            if !self.index.contains_key(n) {
                self.index.insert(n.to_string(), self.names.len());
                self.names.push(n.to_string());
            }
        }
        self.render_all();
    }

    pub fn set_extra(&mut self, extra: i32) {
        if extra != self.extra {
            self.extra = extra;
            self.render_all();
        }
    }

    fn render_all(&mut self) {
        let (names, e) = (std::mem::take(&mut self.names), self.extra);
        self.tpl = names.iter().map(|n| self.font.render(n, e, 0)).collect();
        self.wide = names.iter().map(|n| if n.contains(' ') { self.font.render(n, e, SPACE_WIDE) } else { None }).collect();
        self.names = names;
    }

    /// 간격을 정해 한 번 그린 틀 (시세 단어처럼 여러 간격을 미리 그려 둘 때)
    pub fn render(&mut self, t: &str, extra: i32) -> Option<Gray> {
        self.font.render(t, extra, 0)
    }

    pub fn get(&self, name: &str) -> Option<&Gray> {
        self.index.get(name).and_then(|&i| self.tpl[i].as_ref())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// glyph: 글자 영역만 잘라낸 밝기 그림 → 점수 높은 순
    pub fn ranked(&self, glyph: &Gray) -> Ranked {
        let w = glyph.w as f64;
        let lim = f64::max(4.0, 0.15 * w);
        let mut scores: Ranked = Vec::new();
        for (i, n) in self.names.iter().enumerate() {
            let mut best: Option<f64> = None;
            for t in [&self.tpl[i], &self.wide[i]].into_iter().flatten() {
                if (t.w as f64 - w).abs() > lim {
                    continue;
                }
                let v = ncc(glyph, t);
                best = Some(best.map_or(v, |b| b.max(v)));
            }
            if let Some(b) = best {
                scores.push((b, n.clone()));
            }
        }
        sort_ranked(&mut scores);
        scores
    }

    /// (이름, 점수, 2위와의 차이)
    pub fn best(&self, glyph: &Gray) -> (Option<String>, f64, f64) {
        let s = self.ranked(glyph);
        match s.first() {
            None => (None, 0.0, 0.0),
            Some((sc, n)) => (Some(n.clone()), *sc, sc - s.get(1).map_or(0.0, |x| x.0)),
        }
    }
}

/// Python `scores.sort(reverse=True)`: 점수 높은 순, 같으면 이름이 큰 쪽 먼저
pub fn sort_ranked(s: &mut Ranked) {
    s.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| b.1.cmp(&a.1)));
}
