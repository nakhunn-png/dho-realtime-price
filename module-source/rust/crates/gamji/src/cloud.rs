//! Firebase Realtime Database로 시세를 올린다 (Python `cloud.py` 의 Rust 판). 인터넷 연결은 Windows 내장 WinHTTP.
//!
//! 로그인: 익명 로그인(Identity Toolkit REST). 토큰은 data/fb_auth.json 에 저장해 다시 쓴다.
//! DB 구조 (웹과 같음):
//!   /servers/{서버}/cities/{도시}/seen          = 초
//!   /servers/{서버}/cities/{도시}/prices/{품목}/sell = {price, pct, at, by(등록한 캐릭터)}
//!   /log/{서버}/{시각키}                          = {at, by, city, good, pct|tier, price}  (관리자만 읽음)
//!   /servers/{서버}/cities/{도시}/news           = [{text, at}, ...]
//!   /addr/{접속 주소}/{PC 번호}                   = {server, at}
//!   /me/{PC 번호}                                 = 이 PC의 지금 캐릭터
//! 키에 쓸 수 없는 글자(. $ # [ ] / %)는 %XX 로 바꾼다.
use serde_json::{Map, Value, json};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const UA: &str = "DHO-realtime-price-A/0.2";

pub fn fkey(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if ".$#[]/%".contains(c) {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// 등록 기록 키: 시각(ms) 순으로 정렬되고 겹치지 않게
pub fn log_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = (SEQ.fetch_add(1, Ordering::SeqCst) + 1) % 1000;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(ms);
    format!("{ms:013}{seq:03}{:03}", h.finish() % 1000)
}

// ------------------------------------------------------------------ HTTPS (WinHTTP)
#[derive(Debug)]
pub struct HttpError {
    /// HTTP 상태 번호 (연결 실패는 0)
    pub code: u16,
    pub msg: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.code > 0 {
            write!(f, "HTTP Error {}: {}", self.code, self.msg)
        } else {
            write!(f, "{}", self.msg)
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// https 요청 하나. body: (Content-Type, 내용)
pub fn http(method: &str, url: &str, body: Option<(&str, Vec<u8>)>) -> Result<Vec<u8>, HttpError> {
    use windows::Win32::Networking::WinHttp::*;
    use windows::core::PCWSTR;
    let err = |msg: String| HttpError { code: 0, msg };
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| err(format!("https 주소가 아님: {url}")))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    struct H(*mut c_void);
    impl Drop for H {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let _ = WinHttpCloseHandle(self.0);
                }
            }
        }
    }
    unsafe {
        let ua = wide(UA);
        let ses = H(WinHttpOpen(
            PCWSTR(ua.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if ses.0.is_null() {
            return Err(err(format!(
                "인터넷 연결을 열지 못함: {}",
                windows::core::Error::from_win32()
            )));
        }
        let _ = WinHttpSetTimeouts(ses.0, 15000, 15000, 15000, 15000);
        let hw = wide(host);
        let con = H(WinHttpConnect(
            ses.0,
            PCWSTR(hw.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ));
        if con.0.is_null() {
            return Err(err(format!(
                "{host} 에 연결하지 못함: {}",
                windows::core::Error::from_win32()
            )));
        }
        let (mw, pw) = (wide(method), wide(path));
        let req = H(WinHttpOpenRequest(
            con.0,
            PCWSTR(mw.as_ptr()),
            PCWSTR(pw.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err(err(format!(
                "요청을 만들지 못함: {}",
                windows::core::Error::from_win32()
            )));
        }
        let (hdr, data) = match &body {
            Some((ct, b)) => (format!("Content-Type: {ct}\r\n"), b.as_slice()),
            None => (String::new(), &[][..]),
        };
        let hdr_w: Vec<u16> = hdr.encode_utf16().collect();
        WinHttpSendRequest(
            req.0,
            if hdr_w.is_empty() { None } else { Some(&hdr_w) },
            if data.is_empty() {
                None
            } else {
                Some(data.as_ptr() as *const c_void)
            },
            data.len() as u32,
            data.len() as u32,
            0,
        )
        .map_err(|e| err(format!("보내지 못함: {e}")))?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut())
            .map_err(|e| err(format!("응답이 없음: {e}")))?;
        let mut code: u32 = 0;
        let mut len = 4u32;
        let _ = WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut code as *mut u32 as *mut c_void),
            &mut len,
            std::ptr::null_mut(),
        );
        let mut out = Vec::new();
        loop {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(req.0, &mut avail).is_err() || avail == 0 {
                break;
            }
            let mut buf = vec![0u8; avail as usize];
            let mut got = 0u32;
            if WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, avail, &mut got).is_err()
                || got == 0
            {
                break;
            }
            out.extend_from_slice(&buf[..got as usize]);
        }
        if !(200..300).contains(&code) {
            let reason = String::from_utf8_lossy(&out)
                .chars()
                .take(200)
                .collect::<String>();
            return Err(HttpError {
                code: code as u16,
                msg: reason,
            });
        }
        Ok(out)
    }
}

fn req_json(method: &str, url: &str, data: Option<&Value>) -> Result<Value, HttpError> {
    let body = data.map(|d| {
        (
            "application/json; charset=utf-8",
            serde_json::to_vec(d).unwrap_or_default(),
        )
    });
    let b = http(method, url, body)?;
    Ok(serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn req_form(url: &str, form: &[(&str, &str)]) -> Result<Value, HttpError> {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    let body = form
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    let b = http(
        "POST",
        url,
        Some(("application/x-www-form-urlencoded", body.into_bytes())),
    )?;
    Ok(serde_json::from_slice(&b).unwrap_or(Value::Null))
}

// ------------------------------------------------------------------ 올리기
struct Auth {
    token: Option<String>,
    exp: f64,
    refresh: Option<String>,
}

/// 올리기가 실패로 바뀐 횟수 (감지기 3.46: 늘 때마다 HUD '잠시 뒤 다시 보냅니다'. 모듈은 로그 글자로 따로 띄움)
pub static FAILS: AtomicU32 = AtomicU32::new(0);

pub struct Cloud {
    key: String,
    db: String,
    auth_path: PathBuf,
    auth: Mutex<Auth>,
    tx: Mutex<Sender<Map<String, Value>>>,
}

fn now() -> f64 {
    crate::collector::now()
}

impl Cloud {
    /// 올릴 내용을 모아 백그라운드에서 PATCH 한다. 실패하면 모아 두었다가 다시 보낸다.
    pub fn start(api_key: &str, db_url: &str, auth_path: PathBuf) -> Arc<Cloud> {
        let refresh = std::fs::read(&auth_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| {
                v.get("refreshToken")
                    .and_then(|r| r.as_str())
                    .map(String::from)
            });
        let (tx, rx) = channel();
        let c = Arc::new(Cloud {
            key: api_key.to_string(),
            db: db_url.trim_end_matches('/').to_string(),
            auth_path,
            auth: Mutex::new(Auth {
                token: None,
                exp: 0.0,
                refresh,
            }),
            tx: Mutex::new(tx),
        });
        let c2 = c.clone();
        std::thread::spawn(move || c2.run(rx));
        c
    }

    fn drop_token(&self) {
        self.auth.lock().unwrap().token = None;
    }

    fn token(&self) -> Result<String, HttpError> {
        let mut a = self.auth.lock().unwrap();
        if let Some(t) = &a.token {
            if now() < a.exp - 120.0 {
                return Ok(t.clone());
            }
        }
        if let Some(rf) = a.refresh.clone() {
            match req_form(
                &format!(
                    "https://securetoken.googleapis.com/v1/token?key={}",
                    self.key
                ),
                &[("grant_type", "refresh_token"), ("refresh_token", &rf)],
            ) {
                Ok(r) if r["id_token"].is_string() => {
                    a.token = r["id_token"].as_str().map(String::from);
                    a.refresh = r["refresh_token"].as_str().map(String::from).or(Some(rf));
                    a.exp = now()
                        + r["expires_in"]
                            .as_str()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(3600.0);
                    self.save(&a);
                    return Ok(a.token.clone().unwrap());
                }
                Ok(r) => println!("[클라우드] 토큰 갱신 실패, 새로 로그인: {r}"),
                Err(e) => println!("[클라우드] 토큰 갱신 실패, 새로 로그인: {e}"),
            }
        }
        let r = req_json(
            "POST",
            &format!(
                "https://identitytoolkit.googleapis.com/v1/accounts:signUp?key={}",
                self.key
            ),
            Some(&json!({"returnSecureToken": true})),
        )?;
        let tok = r["idToken"].as_str().ok_or_else(|| HttpError {
            code: 0,
            msg: format!("로그인 응답이 이상함: {r}"),
        })?;
        a.token = Some(tok.to_string());
        a.refresh = r["refreshToken"].as_str().map(String::from);
        a.exp = now()
            + r["expiresIn"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3600.0);
        self.save(&a);
        Ok(tok.to_string())
    }

    fn save(&self, a: &Auth) {
        if let Some(d) = self.auth_path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(
            &self.auth_path,
            json!({"refreshToken": a.refresh}).to_string(),
        );
    }

    /// 공유 서버에서 읽는다 (DB 맨 위 기준 경로)
    pub fn get(&self, path: &str) -> Result<Value, HttpError> {
        let tok = self.token()?;
        req_json("GET", &format!("{}/{path}.json?auth={tok}", self.db), None)
    }

    /// DB 맨 위 기준 경로 → 값 묶음을 올릴 줄에 넣는다
    pub fn put(&self, item: Map<String, Value>) {
        let _ = self.tx.lock().unwrap().send(item);
    }

    /// updates: {"seen": 1, "prices/후추/sell": {...}, "news": [...]} (도시 기준 상대 경로), extra: DB 맨 위 기준 경로 (등록 기록 log/...)
    pub fn push(
        &self,
        server: &str,
        city: &str,
        updates: Map<String, Value>,
        extra: Map<String, Value>,
    ) {
        let base = format!("servers/{}/cities/{}", fkey(server), fkey(city));
        let mut item: Map<String, Value> = updates
            .into_iter()
            .map(|(k, v)| (format!("{base}/{k}"), v))
            .collect();
        item.extend(extra);
        self.put(item);
    }

    fn patch(&self, tok: &str, body: &Map<String, Value>) -> Result<Value, HttpError> {
        req_json(
            "PATCH",
            &format!("{}/.json?auth={tok}", self.db),
            Some(&Value::Object(body.clone())),
        )
    }

    fn run(&self, rx: Receiver<Map<String, Value>>) {
        let mut pending: Map<String, Value> = Map::new();
        let mut ok: Option<bool> = None;
        loop {
            let got = if pending.is_empty() {
                rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
            } else {
                rx.recv_timeout(Duration::from_secs(5))
            };
            match got {
                Ok(item) => {
                    pending.extend(item);
                    while let Ok(more) = rx.try_recv() {
                        pending.extend(more); // 한꺼번에 묶어서 한 번에 보냄
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if pending.is_empty() {
                continue;
            }
            let res = self
                .token()
                .and_then(|tok| match self.patch(&tok, &pending) {
                    Err(e) if matches!(e.code, 400 | 401 | 403) => {
                        // 규칙에 막힌 항목이 하나 있으면 묶음 전체가 거절된다 → 토큰을 새로 받아 하나씩 보내고, 막힌 것만 버린다
                        self.drop_token();
                        let tok = self.token()?;
                        for (k, v) in &pending {
                            let mut one = Map::new();
                            one.insert(k.clone(), v.clone());
                            match self.patch(&tok, &one) {
                                Err(e2) if matches!(e2.code, 400 | 401 | 403) => {
                                    println!("[클라우드] 공유 서버 규칙에 막혀 건너뜀: {k}")
                                }
                                Err(e2) => return Err(e2),
                                Ok(_) => {}
                            }
                        }
                        Ok(Value::Null)
                    }
                    r => r,
                });
            match res {
                Ok(_) => {
                    if ok != Some(true) {
                        println!("[클라우드] 연결됨 — 시세를 공유 서버에 올립니다");
                    }
                    ok = Some(true);
                    pending.clear();
                }
                Err(e) => {
                    if ok != Some(false) {
                        println!("[클라우드] 올리기 실패 (나중에 다시 시도): {e}");
                        FAILS.fetch_add(1, Ordering::Relaxed);
                    }
                    ok = Some(false);
                    if e.code == 401 {
                        self.drop_token();
                    }
                    std::thread::sleep(Duration::from_secs(10));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(fkey("a.b/c%d"), "a%2Eb%2Fc%25d");
        assert_eq!(fkey("후추"), "후추");
        let k = log_key();
        assert_eq!(k.len(), 19);
        assert!(k.chars().all(|c| c.is_ascii_digit()));
    }
}

#[cfg(test)]
mod net_tests {
    /// 인터넷 확인: 공개 지도의 config.js 를 읽기만 한다 (cargo test -- --ignored)
    #[test]
    #[ignore]
    fn fetch_public() {
        let b = super::http("GET", "https://real-time-dho.web.app/config.js", None).unwrap();
        assert!(String::from_utf8_lossy(&b).contains("databaseURL"));
        let e = super::http(
            "GET",
            "https://real-time-dho.web.app/no-such-file-xyz.js",
            None,
        )
        .unwrap_err();
        assert_eq!(e.code, 404);
    }
}
