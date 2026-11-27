//! 남만무역정보 창 화면을 언제 쓰나 — 감지기 gamji/src/collector.rs `NbTrack` 을 그대로 옮겼다 (0.10.0, 2026-11-23 사용자 "방식은 exe 와 같아").
//!
//! ① 도시를 누르면 왼쪽 목록은 바로 바뀌지만 오른쪽('도시 상태'·그래프)은 조금 뒤에 바뀐다 → 도시가 바뀌면 화면이 NB_SETTLE_SEC 동안
//!    그대로이고 오른쪽이 바꾸기 직전과 달라야 쓴다 (똑같으면 NB_STALE_SEC 까지 기다림).
//! ② 같은 도시에서 분류를 바꾸면 그래프는 바로 바뀐다. 도시가 자리 잡은 뒤엔 분류가 바뀐 첫 화면을 바로 쓴다.
//!    단 그래프가 앞 분류와 똑같으면 (잘못 읽었을 수도) 자리 잡을 때까지 기다린다
use recog::nanban::{KINDS, NanbanScreen, POINTS};
use std::collections::HashMap;

pub const NB_SETTLE_SEC: f64 = 0.6;
pub const NB_STALE_SEC: f64 = 2.0;
pub const NB_CITY_LAG_SEC: f64 = 1.0;

#[derive(Default)]
pub struct NbTrack {
    prev: Option<NanbanScreen>,
    since: f64,
    city: Option<(String, Option<String>)>,
    city_at: f64,
    city_before: Option<(Option<[u8; POINTS]>, Option<String>)>,
    city_ok: bool,
    ok: Option<NanbanScreen>,
    /// 이 도시에서 쓴 그래프 → 분류 (다른 분류 이름으로 같은 그래프가 보이면 게임이 그래프를 늦게 그린 것)
    used: HashMap<[u8; POINTS], String>,
}

/// 분류가 목록에서 한 칸만 움직였나 (위·아래, 끝에서 처음으로 넘어가는 것 포함)
fn one_step(a: Option<&str>, b: Option<&str>) -> bool {
    let ix = |k: Option<&str>| k.and_then(|k| KINDS.iter().position(|x| *x == k));
    let (Some(i), Some(j)) = (ix(a), ix(b)) else {
        return false;
    };
    let n = KINDS.len();
    (i + 1) % n == j || (j + 1) % n == i
}

impl NbTrack {
    pub fn settle(&mut self, scr: &NanbanScreen, t: f64) -> bool {
        let pane = (scr.pts, scr.state.clone());
        let ck = (scr.culture.clone(), scr.city.clone());
        if self.city.as_ref() != Some(&ck) {
            self.city_before = self.prev.as_ref().map(|p| (p.pts, p.state.clone()));
            self.city = Some(ck);
            self.city_at = t;
            self.city_ok = false;
            self.ok = None;
            self.used.clear();
        }
        let prev = self.prev.clone();
        if prev.as_ref() != Some(scr) {
            self.prev = Some(scr.clone());
            self.since = t;
        }
        if self.ok.as_ref() == Some(scr) {
            return true; // 이미 쓴 화면이 그대로
        }
        let stable = t - self.since >= NB_SETTLE_SEC - 1e-9;
        let good = if !self.city_ok {
            // 도시를 바꾼 뒤 0.6초 그대로이거나, 1초가 지났고 오른쪽이 바꾸기 직전과 다르면 (방향키를 누르고 있어 그대로일 틈이 없을 때)
            let fresh =
                !(self.city_before.as_ref() == Some(&pane) && t - self.city_at < NB_STALE_SEC);
            let since_city = t - self.city_at;
            let ok = fresh
                && ((since_city >= NB_SETTLE_SEC - 1e-9 && stable)
                    || (since_city >= NB_CITY_LAG_SEC && self.city_before.as_ref() != Some(&pane)));
            self.city_ok = ok;
            ok
        } else {
            // 분류가 바뀐 첫 화면을 바로 쓰는 조건 (2026-11-19 녹화 1700장): 목록이 한 칸만 움직였고 (두 칸 건너뛴 화면은 그래프가 가운데 분류 것이었음),
            // 그래프가 앞 화면과 다르고, 이 도시에서 다른 분류로 쓴 그래프가 아니다
            let fast = prev.as_ref().is_some_and(|p| {
                p.list == scr.list
                    && one_step(p.kind.as_deref(), scr.kind.as_deref())
                    && p.pts != scr.pts
            }) && scr.pts.is_some_and(|p| {
                self.used
                    .get(&p)
                    .is_none_or(|k| Some(k) == scr.kind.as_ref())
            });
            stable || fast
        };
        if good {
            self.ok = Some(scr.clone());
            if let (Some(p), Some(k)) = (scr.pts, &scr.kind) {
                self.used.insert(p, k.clone());
            }
        }
        good
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn scr(city: &str, pts: u8, state: &str) -> NanbanScreen {
        NanbanScreen {
            culture: "화남".into(),
            city: Some(city.into()),
            kind: Some("잡화".into()),
            state: Some(state.into()),
            pts: Some([pts; POINTS]),
            list: vec![
                ("운대산".into(), Some("보통".into())),
                ("중경".into(), Some("전쟁".into())),
            ],
        }
    }

    /// 0.1초마다 화면을 읽을 때 처음 자리 잡는 시각
    fn first_ok(tr: &mut NbTrack, frames: &[(f64, NanbanScreen)]) -> Option<f64> {
        frames.iter().find(|(t, s)| tr.settle(s, *t)).map(|f| f.0)
    }

    #[test]
    fn nanban_kind_scroll_is_instant() {
        // 도시가 자리 잡은 뒤 아래 방향키: 분류가 0.03초마다 바뀌고 화면은 0.016초마다 → 분류가 바뀐 첫 화면을 바로 쓴다
        let mut tr = NbTrack::default();
        let mut t = 0.0;
        while t < 1.0 {
            tr.settle(&scr("중경", 5, "전쟁"), t);
            t += 0.016;
        }
        let kinds = ["식료품", "조미료", "주류", "기호품", "향신료", "의약품"];
        let mut got = vec![];
        for (i, k) in kinds.iter().enumerate() {
            for j in 0..2 {
                let mut s = scr("중경", 10 + i as u8, "전쟁");
                s.kind = Some(k.to_string());
                if tr.settle(&s, t) && j == 0 {
                    got.push(*k);
                }
                t += 0.016;
            }
        }
        assert_eq!(got, kinds);
        // 앞 분류와 그래프가 똑같으면 바로 쓰지 않는다
        let mut s = scr("중경", 15, "전쟁");
        s.kind = Some("섬유".into());
        assert!(!tr.settle(&s, t));
    }

    /// 실제 녹화(2026-11-19, 57장/초, 천주→운대산을 누르고 아래 방향키): 녹화 화면을 그대로 넣어 본다.
    /// 운대산을 누른 뒤 0.7초 동안 천주 그래프가 남아 있었고, '광석'에 귀금속 그래프가 붙은 화면(염료→광석 두 칸 건너뜀)이 있었다
    #[test]
    fn nanban_replay_recording() {
        let mut tr = NbTrack::default();
        let mut got: Vec<(String, String, String)> = vec![];
        for l in include_str!("testdata/nbrec_unsan.txt").lines() {
            let t: f64 = l.trim_start().split('s').next().unwrap().parse().unwrap();
            let Some((head, pts)) = l
                .split_once(") ")
                .map(|x| x.1)
                .and_then(|x| x.split_once(" | "))
            else {
                continue;
            };
            let f: Vec<&str> = head.split(' ').collect();
            let p: Vec<u8> = pts.split(' ').map(|v| v.parse().unwrap()).collect();
            let opt = |v: &str| (v != "?").then(|| v.to_string());
            let scr = NanbanScreen {
                culture: "화남".into(),
                city: opt(f[0]),
                kind: opt(f[1]),
                state: opt(f[2]),
                pts: Some(p.clone().try_into().unwrap()),
                list: vec![],
            };
            if tr.settle(&scr, t) {
                let e = (f[0].to_string(), f[1].to_string(), pts.to_string());
                if got.last() != Some(&e) {
                    got.push(e);
                }
            }
        }
        // 운대산 이름으로 천주 그래프를 쓰지 않는다
        let cheonju = "18 16 16 21 18 16 21 18 16 16 25";
        assert!(
            !got.iter().any(|e| e.0 == "운대산" && e.2 == cheonju),
            "{got:?}"
        );
        // 광석에 귀금속 그래프를 붙이지 않는다
        assert!(
            !got.iter()
                .any(|e| e.1 == "광석" && e.2 == "7 10 9 8 8 7 7 10 9 8 8"),
            "{got:?}"
        );
        // 한 분류에 그래프는 하나 (방향키 두 바퀴)
        let mut by: HashMap<String, HashSet<String>> = HashMap::new();
        for e in got.iter().filter(|e| e.0 == "운대산") {
            by.entry(e.1.clone()).or_default().insert(e.2.clone());
        }
        assert!(by.values().all(|v| v.len() == 1), "{by:?}");
        assert!(by.len() >= 19, "운대산 분류 {}개만 읽음: {by:?}", by.len());
    }

    #[test]
    fn nanban_waits_for_right_pane() {
        let mut tr = NbTrack::default();
        let ticks = |a: f64, b: f64| {
            (0..)
                .map(move |i| a + i as f64 * 0.1)
                .take_while(move |t| *t < b - 1e-9)
        };
        // 중경(전쟁, 5틱): 0.6초 뒤에야 씀
        let f: Vec<_> = ticks(0.0, 1.0)
            .map(|t| (t, scr("중경", 5, "전쟁")))
            .collect();
        assert!((first_ok(&mut tr, &f).unwrap() - 0.6).abs() < 1e-6);
        // 운대산을 누름: 오른쪽은 0.8초 동안 중경 것 그대로 → 쓰지 않음, 바뀐 뒤 0.6초 지나서 씀
        let mut f: Vec<_> = ticks(1.0, 1.8)
            .map(|t| (t, scr("운대산", 5, "전쟁")))
            .collect();
        f.extend(ticks(1.8, 3.0).map(|t| (t, scr("운대산", 40, "보통"))));
        assert!((first_ok(&mut tr, &f).unwrap() - 2.0).abs() < 1e-6);
        // 다시 중경: 오른쪽이 정말로 운대산과 같은 값이면 2초 기다린 뒤 씀
        let f: Vec<_> = ticks(3.0, 6.0)
            .map(|t| (t, scr("중경", 40, "보통")))
            .collect();
        assert!((first_ok(&mut tr, &f).unwrap() - 5.0).abs() < 1e-6);
    }
}
