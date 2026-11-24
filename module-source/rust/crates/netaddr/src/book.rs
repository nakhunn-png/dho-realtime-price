//! 주소표: 접속 주소 → 서버 (Python `AddrBook`)
//!
//! 공유 서버와 주고받는 부분(sync·learn 의 올리기)은 5단계(올리기)에서 붙인다. 여기서는 `accept`로 받은 내용을 고르는 규칙까지.
use crate::log;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

pub fn ip_of(addr: &str) -> &str {
    addr.rsplit_once(':').map_or(addr, |(ip, _)| ip)
}

/// 공유 서버 키 (점은 키에 못 쓴다): 1.2.3.4:9067 → 1_2_3_4:9067
pub fn addr_key(addr: &str) -> String {
    addr.replace('.', "_")
}

pub fn key_addr(key: &str) -> String {
    key.replace('_', ".")
}

fn octet(s: &str) -> Option<u32> {
    (!s.is_empty() && s.len() <= 3 && s.chars().all(|c| c.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// `\d{1,3}(\.\d{1,3}){3}:\d{1,5}`
fn is_addr(a: &str) -> bool {
    let Some((ip, port)) = a.rsplit_once(':') else { return false };
    let parts: Vec<&str> = ip.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| octet(p).is_some()) && !port.is_empty() && port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit())
}

/// {"1.2.3.10-19": "헬레네"} → [("1.2.3.", 10, 19, "헬레네")]. 마지막 자리만 범위로 쓴다
pub fn parse_ranges(ranges: &[(String, String)], servers: &[String]) -> Vec<(String, u32, u32, String)> {
    let mut out = vec![];
    for (k, s) in ranges {
        if !servers.contains(s) {
            continue;
        }
        let k = k.trim();
        let Some((head, last)) = k.rsplit_once('.') else { continue };
        let hp: Vec<&str> = head.split('.').collect();
        if hp.len() != 3 || !hp.iter().all(|p| octet(p).is_some()) {
            continue;
        }
        let (lo, hi) = match last.split_once('-') {
            Some((a, b)) => (octet(a), octet(b)),
            None => (octet(last), octet(last)),
        };
        if let (Some(lo), Some(hi)) = (lo, hi) {
            out.push((format!("{head}."), lo, hi, s.clone()));
        }
    }
    out
}

#[derive(Default)]
struct Tables {
    shared: BTreeMap<String, String>,
    own: BTreeMap<String, String>,
    warned: HashSet<(String, String)>,
}

pub struct AddrBook {
    pub servers: Vec<String>,
    pub seeds: BTreeMap<String, String>,
    pub ranges: Vec<(String, u32, u32, String)>,
    path: Option<PathBuf>,
    t: Mutex<Tables>,
}

impl AddrBook {
    /// 공유 서버의 주소는 서로 다른 PC 이만큼이 같은 서버로 알려 와야 인정
    pub const NEED: usize = 2;
    pub const SYNC_SEC: u64 = 600;

    pub fn new(seeds: &[(String, String)], servers: &[String], path: Option<PathBuf>, ranges: &[(String, String)]) -> Self {
        let servers = servers.to_vec();
        let seeds = seeds.iter().filter(|(_, s)| servers.contains(s)).cloned().collect();
        let ranges = parse_ranges(ranges, &servers);
        let mut t = Tables::default();
        if let Some(Ok(v)) = path.as_ref().and_then(|p| std::fs::read(p).ok()).map(|b| serde_json::from_slice::<Value>(&b)) {
            let pick = |k: &str| -> BTreeMap<String, String> {
                v.get(k)
                    .and_then(|m| m.as_object())
                    .map(|m| {
                        m.iter()
                            .filter_map(|(a, s)| s.as_str().filter(|s| servers.iter().any(|x| x == s)).map(|s| (a.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            t.shared = pick("shared");
            t.own = pick("own");
        }
        AddrBook { servers, seeds, ranges, path, t: Mutex::new(t) }
    }

    pub fn shared(&self) -> BTreeMap<String, String> {
        self.t.lock().unwrap().shared.clone()
    }

    pub fn own(&self) -> BTreeMap<String, String> {
        self.t.lock().unwrap().own.clone()
    }

    /// 주소 → 서버. 정확한 주소 → 같은 IP(포트는 바뀐다) → IP 대역 순서로 찾는다
    pub fn lookup(&self, addr: &str) -> Option<String> {
        {
            let t = self.t.lock().unwrap();
            for table in [&self.seeds, &t.shared, &t.own] {
                if let Some(s) = table.get(addr) {
                    return Some(s.clone());
                }
            }
            let ip = ip_of(addr);
            for table in [&self.seeds, &t.shared, &t.own] {
                let found: HashSet<&String> = table.iter().filter(|(a, _)| ip_of(a) == ip).map(|(_, s)| s).collect();
                if found.len() == 1 {
                    return found.into_iter().next().cloned();
                }
            }
        }
        let (head, last) = ip_of(addr).rsplit_once('.')?;
        let last: u32 = last.parse().ok()?;
        self.ranges.iter().find(|(pre, lo, hi, _)| format!("{head}.") == *pre && *lo <= last && last <= *hi).map(|r| r.3.clone())
    }

    /// 연결 주소들 → 서버. 아는 주소가 가리키는 서버가 정확히 하나일 때만
    pub fn server_of<'a>(&self, addrs: impl IntoIterator<Item = &'a String>) -> Option<String> {
        let found: HashSet<String> = addrs.into_iter().filter_map(|a| self.lookup(a)).collect();
        if found.len() == 1 { found.into_iter().next() } else { None }
    }

    /// 이름과 별 표시로 확실히 알아낸 서버를 그 주소의 서버로 기억한다.
    /// 새로 배운 주소면 true (부르는 쪽이 공유 서버에 `addr/{addr_key}/{PC 번호}` 로 알린다)
    pub fn learn(&self, addr: &str, server: &str) -> bool {
        if !self.servers.iter().any(|s| s == server) || !is_addr(addr) {
            return false;
        }
        if let Some(known) = self.lookup(addr) {
            let mut t = self.t.lock().unwrap();
            if known != server && t.warned.insert((addr.into(), server.into())) {
                drop(t);
                log(&format!("[접속 주소] {addr} 는 {known} 로 알고 있는데 화면으로는 {server} · 주소표를 바꾸지 않습니다"));
            }
            return false;
        }
        {
            let mut t = self.t.lock().unwrap();
            t.own.insert(addr.into(), server.into());
            self.save(&t);
        }
        log(&format!("[접속 주소] {addr} = {server} (새로 알아냄)"));
        true
    }

    /// 이 PC가 화면(이름 + 별)으로 알아낸 주소를 지운다 (설정 초기화 때)
    pub fn reset_own(&self) {
        let had = {
            let mut t = self.t.lock().unwrap();
            let had = std::mem::take(&mut t.own);
            t.warned.clear();
            self.save(&t);
            had
        };
        if !had.is_empty() {
            log(&format!(
                "[접속 주소] 이 PC가 알아낸 주소를 지움: {}",
                had.iter().map(|(a, s)| format!("{a}={s}")).collect::<Vec<_>>().join(", ")
            ));
        }
    }

    fn save(&self, t: &Tables) {
        let Some(p) = &self.path else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let v = json!({ "shared": t.shared, "own": t.own });
        let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
    }

    /// 공유 서버 /addr 내용 → 인정할 주소표. 알려 온 PC들의 서버가 하나로 같고 NEED 대 이상일 때만
    pub fn accept(&self, data: &Value) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(m) = data.as_object() else { return out };
        for (key, reps) in m {
            let Some(reps): Option<&Map<String, Value>> = reps.as_object() else { continue };
            let srv: HashSet<Option<&str>> =
                reps.values().filter(|r| r.is_object()).map(|r| r.get("server").and_then(|s| s.as_str())).collect();
            if srv.len() == 1 && reps.len() >= Self::NEED {
                if let Some(Some(s)) = srv.into_iter().next() {
                    if self.servers.iter().any(|x| x == s) {
                        out.insert(key_addr(key), s.to_string());
                    }
                }
            }
        }
        out
    }

    /// 받은 공유 주소표로 바꾼다 (달라졌을 때만 저장·로그)
    pub fn set_shared(&self, shared: BTreeMap<String, String>) {
        let mut t = self.t.lock().unwrap();
        if shared != t.shared {
            t.shared = shared;
            self.save(&t);
            let s = t.shared.iter().map(|(a, s)| format!("{a}={s}")).collect::<Vec<_>>().join(", ");
            drop(t);
            log(&format!("[접속 주소] 공유 주소표: {}", if s.is_empty() { "(없음)".into() } else { s }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn lookup_rules() {
        let servers: Vec<String> = ["헬레네", "폴라리스"].iter().map(|x| x.to_string()).collect();
        let b = AddrBook::new(
            &s(&[("203.242.208.218:9067", "헬레네"), ("203.242.208.228:9066", "폴라리스"), ("1.1.1.1:1", "에이레네")]),
            &servers,
            None,
            &s(&[("203.242.208.216-219", "헬레네"), ("203.242.208.226-229", "폴라리스"), ("bad", "헬레네")]),
        );
        assert_eq!(b.seeds.len(), 2); // 모르는 서버는 뺀다
        assert_eq!(b.ranges.len(), 2);
        assert_eq!(b.lookup("203.242.208.218:9067").as_deref(), Some("헬레네")); // 정확한 주소
        assert_eq!(b.lookup("203.242.208.228:9999").as_deref(), Some("폴라리스")); // 같은 IP 다른 포트
        assert_eq!(b.lookup("203.242.208.217:1234").as_deref(), Some("헬레네")); // 대역
        assert_eq!(b.lookup("203.242.208.220:1234"), None);
        let a = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(b.server_of(&a(&["203.242.208.217:1", "8.8.8.8:53"])).as_deref(), Some("헬레네"));
        assert_eq!(b.server_of(&a(&["203.242.208.217:1", "203.242.208.227:1"])), None); // 서로 다른 서버
        assert!(!b.learn("203.242.208.217:5", "폴라리스")); // 아는 주소는 안 바꾼다
        assert!(b.learn("9.9.9.9:100", "폴라리스"));
        assert_eq!(b.lookup("9.9.9.9:100").as_deref(), Some("폴라리스"));
        assert!(!b.learn("9.9.9:100", "폴라리스"));
        let acc = b.accept(&json!({
            "1_2_3_4:5": {"pcA": {"server": "헬레네"}, "pcB": {"server": "헬레네"}},
            "1_2_3_5:5": {"pcA": {"server": "헬레네"}},
            "1_2_3_6:5": {"pcA": {"server": "헬레네"}, "pcB": {"server": "폴라리스"}},
            "1_2_3_7:5": {"pcA": {"server": "에이레네"}, "pcB": {"server": "에이레네"}},
        }));
        assert_eq!(acc, BTreeMap::from([("1.2.3.4:5".to_string(), "헬레네".to_string())]));
    }
}
