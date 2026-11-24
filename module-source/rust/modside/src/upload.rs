//! 4단계: 읽은 매각 시세를 공유 서버(Firebase)에 올린다. 감지기 store.rs `report_trade` 의 매각 부분과 같은 모양으로 올린다.
//!
//! - 처음에는 시험 서버(`/servers/시험/...`)에만 올린다 (2026-11-22 사용자 결정). 웹 지도에는 '시험' 서버가 없어 다른 사람에게 안 보인다.
//!   확인이 끝나면 `TEST_SERVER` 를 None 으로 바꿔 진짜 서버에 올린다.
//! - 등록자(by)는 왼쪽 위 함대 목록에서 읽은 캐릭터 이름 (못 읽으면 '미확인') (사용자 결정)
//! - 올리는 일·로그인·실패 때 다시 보내기는 감지기 cloud.rs 를 그대로 쓴다 (익명 로그인, 토큰은 데이터 폴더 fb_auth.json)
use crate::cloud::{Cloud, fkey, log_key};
use crate::config;
use crate::reader::{NearItem, Row};
use recog::Pct;
use recog::text::is_guild_city;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Some(이름) 이면 진짜 서버 대신 이 이름의 서버 경로에 올린다
pub const TEST_SERVER: Option<&str> = None; // 0.6.0 (2026-11-22 사용자 "진짜 서버에도 올라가는지 확인"): 진짜 서버로. 다시 시험하려면 Some("시험")
/// 같은 값의 등록 기록(log)은 10분에 한 번 (감지기와 같음)
const LOG_SAME_SEC: i64 = 600;

pub struct Uploader {
    cloud: Option<Arc<Cloud>>,
    /// (올린 서버, 도시, 품목) → (시각 뺀 값, 마지막 기록 시각)
    last_log: HashMap<(String, String, String), (Map<String, Value>, i64)>,
    /// (올린 서버, 도시, 품목) → (%, 직접 본 가격, 시각). 인근 시세는 가격이 없어서, %가 그대로면 1시간 안에 본 가격을 그대로 둔다 (감지기 store.rs 와 같음)
    last_price: HashMap<(String, String, String), (i64, i64, i64)>,
    /// 남만무역정보: (서버, 도시, 분류 또는 "상태") → (올린 값, 시각)
    nb_sent: HashMap<(String, String, String), (String, i64)>,
    /// 교역소 대사: (서버, 도시) → 지금 목록 (최근이 앞)
    news: HashMap<(String, String), Vec<Value>>,
}

/// 남만무역정보: 같은 값은 10분 안에 다시 올리지 않는다 (감지기와 같음)
const NANBAN_RESEND_SEC: i64 = 600;
/// 교역소 대사: 도시마다 최근 2건, 6시간 지나면 숨김 (감지기 store.rs 와 같음)
const NEWS_KEEP: usize = 2;
const NEWS_TTL: i64 = 6 * 3600;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// 공유 서버 규칙: 1~20자, . $ # [ ] / 없음
pub fn by_ok(by: &str) -> bool {
    let n = by.chars().count();
    (1..=20).contains(&n) && !by.contains(['.', '$', '#', '[', ']', '/'])
}

impl Uploader {
    pub fn new(data: Option<&PathBuf>) -> Self {
        let auth = data.cloned().unwrap_or_else(std::env::temp_dir).join("fb_auth.json");
        let cloud = config::read_config().map(|c| Cloud::start(&c.api_key, &c.database_url, auth));
        Uploader { cloud, last_log: HashMap::new(), last_price: HashMap::new(), nb_sent: HashMap::new(), news: HashMap::new() }
    }

    pub fn ready(&self) -> bool {
        self.cloud.is_some()
    }

    /// 매각·시세 창에서 읽은 줄을 올린다. 올린 서버 경로 이름(시험이면 '시험')을 돌려준다
    pub fn trade(&mut self, server: &str, city: &str, rows: &[Row], by: &str) -> Option<String> {
        let cl = self.cloud.as_ref()?;
        let target = TEST_SERVER.unwrap_or(server).to_string();
        let t = now();
        let (mut up, mut log) = (Map::new(), Map::new());
        for r in rows {
            let (key, pv) = match &r.pct {
                Pct::Num(n) => ("pct", json!(n)),
                Pct::Tier(s) => ("tier", json!(s)),
            };
            let mut rec = Map::new();
            rec.insert(key.into(), pv.clone());
            rec.insert("price".into(), json!(r.price));
            if let Some((tr, col)) = &r.arrow {
                rec.insert("tr".into(), json!(tr));
                rec.insert("col".into(), json!(col));
            }
            if let Pct::Num(n) = &r.pct {
                self.last_price.insert((target.clone(), city.to_string(), r.name.clone()), (*n, r.price, t));
            }
            let mut sell = rec.clone();
            sell.insert("at".into(), json!(t));
            sell.insert("by".into(), json!(by));
            up.insert(format!("prices/{}/sell", fkey(&r.name)), Value::Object(sell));
            // 관리자 화면의 등록 기록 (같은 값은 10분에 한 번)
            let k = (target.clone(), city.to_string(), r.name.clone());
            let same = self.last_log.get(&k).is_some_and(|(v, at)| *v == rec && t - at <= LOG_SAME_SEC);
            if !same {
                let mut l = Map::new();
                l.insert("at".into(), json!(t));
                l.insert("by".into(), json!(by));
                l.insert("city".into(), json!(city));
                l.insert("good".into(), json!(r.name));
                l.insert(key.into(), pv);
                l.insert("price".into(), json!(r.price));
                log.insert(format!("log/{}/{}", fkey(&target), log_key()), Value::Object(l));
                self.last_log.insert(k, (rec, t));
            }
        }
        if up.is_empty() {
            return None;
        }
        up.insert("seen".into(), json!(t));
        cl.push(&target, city, up, log);
        Some(target)
    }

    /// 남만무역정보 (0.10.0, 감지기 store.rs `report_nanban` 과 같은 모양): `servers/{서버}/nanban/{도시}/state={v,at,by}`,
    /// `.../cats/{분류}={pts,at,by}`. 같은 값은 10분 안에 다시 올리지 않는다. 그래프는 캐릭터의 원래 서버에서만 보이므로 접속한 서버로 올린다.
    /// 올렸으면 (올린 서버, 올린 상태들, 그래프를 올렸나)
    pub fn nanban(
        &mut self,
        server: &str,
        states: &[(String, String)],
        cat: Option<(&str, &str, &[u8])>,
        by: &str,
    ) -> Option<(String, Vec<(String, String)>, bool)> {
        let cl = self.cloud.clone()?;
        let target = TEST_SERVER.unwrap_or(server).to_string();
        let t = now();
        let mut fresh = |k: (String, String, String), v: String| -> bool {
            let ok = self.nb_sent.get(&k).is_none_or(|(old, at)| *old != v || t - at >= NANBAN_RESEND_SEC);
            if ok {
                self.nb_sent.insert(k, (v, t));
            }
            ok
        };
        let mut up = Map::new();
        let mut sent = vec![];
        for (c, v) in states {
            if fresh((target.clone(), c.clone(), "상태".into()), v.clone()) {
                up.insert(format!("servers/{}/nanban/{}/state", fkey(&target), fkey(c)), json!({ "v": v, "at": t, "by": by }));
                sent.push((c.clone(), v.clone()));
            }
        }
        let mut cat_sent = false;
        if let Some((c, k, p)) = cat {
            let v = p.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",");
            if fresh((target.clone(), c.to_string(), k.to_string()), v) {
                up.insert(format!("servers/{}/nanban/{}/cats/{}", fkey(&target), fkey(c), fkey(k)), json!({ "pts": p, "at": t, "by": by }));
                cat_sent = true;
            }
        }
        if up.is_empty() {
            return None;
        }
        cl.put(up);
        Some((target, sent, cat_sent))
    }

    /// 교역소 대사 (0.10.0, 감지기 store.rs `report_news` 와 같은 규칙): 도시마다 최근 2건, 6시간 지나면 뺀다.
    /// 대사는 정식 문장으로 맞춰져 오므로 글자가 같으면 같은 소식 → 시각만 새로 하고 맨 위로.
    /// 감지기는 자기 PC 에 목록을 들고 있지만 모듈은 처음 보는 도시면 공유 서버의 목록을 한 번 읽어 와서 합친다 (남이 올린 대사를 지우지 않게).
    /// 올린 대사 수
    pub fn news(&mut self, server: &str, city: &str, texts: &[String]) -> usize {
        let Some(cl) = self.cloud.clone() else { return 0 };
        if is_guild_city(city) {
            return 0;
        }
        let target = TEST_SERVER.unwrap_or(server).to_string();
        let k = (target.clone(), city.to_string());
        if !self.news.contains_key(&k) {
            let path = format!("servers/{}/cities/{}/news", fkey(&target), fkey(city));
            let old: Vec<Value> = match cl.get(&path) {
                Ok(Value::Array(a)) => a.into_iter().filter(|v| v.is_object()).collect(),
                Ok(Value::Object(o)) => o.into_iter().map(|x| x.1).filter(|v| v.is_object()).collect(),
                _ => vec![],
            };
            self.news.insert(k.clone(), old);
        }
        let t = now();
        let list = self.news.get_mut(&k).unwrap();
        let text_of = |n: &Value| n["text"].as_str().unwrap_or("").to_string();
        let mut added = 0;
        for s in texts {
            if let Some(o) = list.iter().find(|n| text_of(n) == *s)
                && t - o["at"].as_i64().unwrap_or(0) < 600
                && list.first().is_some_and(|f| text_of(f) == *s)
            {
                continue; // 이미 맨 위에 있는 최근 소식
            }
            list.retain(|n| text_of(n) != *s);
            list.insert(0, json!({ "text": s, "at": t }));
            added += 1;
        }
        list.retain(|n| t - n["at"].as_i64().unwrap_or(0) < NEWS_TTL);
        list.sort_by_key(|n| -n["at"].as_i64().unwrap_or(0));
        list.truncate(NEWS_KEEP);
        if added > 0 {
            let mut up = Map::new();
            up.insert("news".into(), Value::Array(list.clone()));
            cl.push(&target, city, up, Map::new());
        }
        added
    }

    /// 시세 창 '인근 도시 시세'를 올린다 (0.9.0). 도시마다 따로 올리고 `near: true`, 가격 없음, seen 은 건드리지 않는다
    /// (감지기 store.rs `report_trade(.., near = true, ..)` 와 같은 모양). 올린 서버 경로 이름을 돌려준다
    pub fn near(&mut self, server: &str, items: &[NearItem], by: &str) -> Option<String> {
        let cl = self.cloud.clone()?;
        let target = TEST_SERVER.unwrap_or(server).to_string();
        let t = now();
        for n in items {
            let mut rec = Map::new();
            rec.insert("pct".into(), json!(n.pct));
            let k = (target.clone(), n.city.clone(), n.good.clone());
            if let Some((p, price, at)) = self.last_price.get(&k)
                && *p == n.pct
                && t - at < 3600
            {
                rec.insert("price".into(), json!(price));
            }
            if let Some((tr, col)) = &n.arrow {
                rec.insert("tr".into(), json!(tr));
                rec.insert("col".into(), json!(col));
            }
            rec.insert("near".into(), json!(true));
            let mut sell = rec.clone();
            sell.insert("at".into(), json!(t));
            sell.insert("by".into(), json!(by));
            let mut up = Map::new();
            up.insert(format!("prices/{}/sell", fkey(&n.good)), Value::Object(sell));
            let mut log = Map::new();
            let same = self.last_log.get(&k).is_some_and(|(v, at)| *v == rec && t - at <= LOG_SAME_SEC);
            if !same {
                let mut l = rec.clone();
                l.insert("at".into(), json!(t));
                l.insert("by".into(), json!(by));
                l.insert("city".into(), json!(n.city));
                l.insert("good".into(), json!(n.good));
                log.insert(format!("log/{}/{}", fkey(&target), log_key()), Value::Object(l));
                self.last_log.insert(k, (rec, t));
            }
            cl.push(&target, &n.city, up, log);
        }
        Some(target)
    }
}
