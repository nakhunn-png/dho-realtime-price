//! 네비: 측량 좌표(내 배 위치)를 이 PC에서만 지도에 보낸다 (collector/server.py 의 NavHub · /api/nav 부분).
//!
//! GET /api/nav → 실시간 갱신 (Server-Sent Events). 공개 지도(real-time-dho.web.app)가 `&nav=포트` 로 이 주소에 붙는다.
//! 공유 서버에는 올리지 않는다.
use capture::Hwnd;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// /api/nav 를 읽어도 되는 웹 주소 (공개 지도)
const NAV_ORIGINS: [&str; 2] = [
    "https://real-time-dho.web.app",
    "https://real-time-dho.firebaseapp.com",
];
pub const NAV_PORT: u16 = 38765;

/// 게임 창별 배 위치와 마지막으로 읽은 도시. 바뀌면 /api/nav 를 보고 있는 지도에 바로 보낸다.
/// 보내는 값: {"ships": [...], "towns": [{id, char, city, at}]}. towns 는 배가 없을 때(입항)도 보낸다:
/// 지도 항해일지가 좌표 규칙(도시 근처에서 10초 끊김)을 기다리지 않고 그 도시로 바로 마감하고, 다음 출항의 출발 도시로 쓴다
#[derive(Default)]
pub struct NavHub {
    m: Mutex<(u64, BTreeMap<Hwnd, Value>)>,
    towns: Mutex<BTreeMap<Hwnd, Value>>,
    cv: Condvar,
}

impl NavHub {
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        wid: Hwnd,
        chr: &str,
        x: u32,
        y: u32,
        at: f64,
        hd: Option<f64>,
        bw: Option<f64>,
    ) {
        let mut o = Map::new();
        o.insert("id".into(), json!(wid));
        o.insert("char".into(), json!(chr));
        o.insert("x".into(), json!(x));
        o.insert("y".into(), json!(y));
        o.insert("at".into(), json!(at));
        o.insert("hd".into(), json!(hd));
        o.insert("bw".into(), json!(bw));
        let mut g = self.m.lock().unwrap();
        g.1.insert(wid, Value::Object(o));
        g.0 += 1;
        self.cv.notify_all();
    }

    pub fn drop_ship(&self, wid: Hwnd) {
        let mut g = self.m.lock().unwrap();
        if g.1.remove(&wid).is_some() {
            g.0 += 1;
            self.cv.notify_all();
        }
    }

    /// 이 창에서 도시에 들어옴 (도시 이름을 새로 읽었을 때만 부른다. 같은 도시로 돌아온 것도 새로 부른다)
    pub fn set_town(&self, wid: Hwnd, chr: &str, city: &str, at: f64) {
        let mut g = self.m.lock().unwrap();
        self.towns
            .lock()
            .unwrap()
            .insert(wid, json!({"id": wid, "char": chr, "city": city, "at": at}));
        g.0 += 1;
        self.cv.notify_all();
    }

    /// 게임 창이 없어짐
    pub fn drop_town(&self, wid: Hwnd) {
        let mut g = self.m.lock().unwrap();
        if self.towns.lock().unwrap().remove(&wid).is_some() {
            g.0 += 1;
            self.cv.notify_all();
        }
    }

    /// 판(ver)이 바뀔 때까지 최대 timeout 기다린다 → (지금 판, 배 목록, 도시 목록)
    fn wait(&self, ver: u64, timeout: Duration) -> (u64, Vec<Value>, Vec<Value>) {
        let g = self.m.lock().unwrap();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, timeout, |g| g.0 == ver)
            .unwrap();
        let towns = self.towns.lock().unwrap().values().cloned().collect();
        (g.0, g.1.values().cloned().collect(), towns)
    }
}

/// 127.0.0.1 의 38765(사용 중이면 다음 번호, 38784까지)에 서버를 연다 → 포트
pub fn start_server(hub: Arc<NavHub>) -> Option<u16> {
    let l = (NAV_PORT..NAV_PORT + 20).find_map(|p| TcpListener::bind(("127.0.0.1", p)).ok())?;
    let port = l.local_addr().ok()?.port();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let hub = hub.clone();
            std::thread::spawn(move || {
                let _ = handle(s, &hub);
            });
        }
    });
    Some(port)
}

fn handle(mut s: TcpStream, hub: &NavHub) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut first = String::new();
    r.read_line(&mut first)?;
    let mut origin = String::new();
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("origin") {
                origin = v.trim().to_string();
            }
        }
    }
    let mut it = first.split_whitespace();
    let (method, path) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
    let path = path.split('?').next().unwrap_or("");
    // 공개 지도가 이 PC의 /api/nav 를 읽게 허락 (크롬의 로컬 네트워크 접근: 처음 한 번 허용 창이 뜰 수 있다)
    let cors = if NAV_ORIGINS.contains(&origin.as_str()) {
        format!(
            "Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Private-Network: true\r\nVary: Origin\r\n"
        )
    } else {
        String::new()
    };
    if method == "OPTIONS" {
        let h = format!(
            "HTTP/1.1 204 No Content\r\n{cors}Access-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: *\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return s.write_all(h.as_bytes());
    }
    if method != "GET" || path != "/api/nav" {
        return s.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    s.write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\n{cors}Connection: close\r\n\r\n")
            .as_bytes(),
    )?;
    s.set_nodelay(true)?;
    let mut ver = u64::MAX;
    loop {
        let (v2, ships, towns) = hub.wait(ver, Duration::from_secs(15));
        if v2 == ver {
            s.write_all(b": keep-alive\n\n")?; // 15초마다 연결 유지
        } else {
            ver = v2;
            s.write_all(
                format!("data: {}\n\n", json!({ "ships": ships, "towns": towns })).as_bytes(),
            )?;
        }
        s.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn towns_ride_along_with_ships() {
        let hub = NavHub::default();
        hub.set_town(7, "아이", "리스본", 100.0);
        let (v1, ships, towns) = hub.wait(u64::MAX, Duration::from_millis(1));
        assert!(ships.is_empty());
        assert_eq!(
            towns,
            vec![json!({"id": 7, "char": "아이", "city": "리스본", "at": 100.0})]
        );
        // 같은 도시로 다시 들어와도 새 판 (시각이 바뀐다)
        hub.set_town(7, "아이", "리스본", 200.0);
        let (v2, _, towns) = hub.wait(v1, Duration::from_millis(1));
        assert!(v2 != v1 && towns[0]["at"] == json!(200.0));
        hub.drop_town(7);
        let (v3, _, towns) = hub.wait(v2, Duration::from_millis(1));
        assert!(v3 != v2 && towns.is_empty());
    }
}
