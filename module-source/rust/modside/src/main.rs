//! 시세 모듈의 상주 sidecar (DHO Navigator 2.2.1+, 표준 입출력 한 줄 = JSON 하나, `ndjson-v1`).
//!
//! 하는 일 (4단계: 읽기 + 게임 화면 HUD + 시험 서버에 올리기)
//! - Navigator 가 `target` 으로 알려 준 게임 창(hwnd)을 Windows Graphics Capture 로 복사한다
//!   (교역 창이 보이면 0.1초, 아니면 2.5초 간격 — 감지기와 같음). 대상이 없으면 복사를 멈춘다
//! - 교역 창 4줄·도시를 읽는다 (reader.rs)
//! - `target.pid` 의 연결 목록으로 서버를 정한다 (netaddr, 감지기와 같은 주소표). 모르는 주소면 서버 없음
//! - 바뀐 것만 `emit` 하고, 데이터 폴더의 side-log.txt 에 모두 적는다
//! - 감지기 exe 가 켜져 있으면(같은 mutex 이름) 화면을 읽지 않고 기다린다 — 같은 화면을 두 번 읽지 않게
//!
//! - 매각·시세 창 시세를 공유 서버에 올린다 (upload.rs, 지금은 시험 서버만). 등록자는 읽은 캐릭터 이름
//!
//! 게임에는 아무것도 하지 않는다: 화면에 그려진 결과와 Windows 연결 목록만 읽는다.

// 표준 출력은 Navigator 와 주고받는 줄(JSON)만 쓴다. 감지기 cloud.rs 의 println! 은 기록 파일로 보낸다
// (이 파일 안에서 println! 을 쓰는 점검 명령도 같이 기록·표준 오류로 간다)
macro_rules! println {
    ($($t:tt)*) => { $crate::say_out(&format!($($t)*)) };
}

#[path = "../../crates/gamji/src/config.rs"]
#[allow(dead_code)]
mod config;
#[path = "../../crates/gamji/src/cloud.rs"]
#[allow(dead_code)]
mod cloud;
/// 1.0.5: 웹 지도 네비 (감지기와 같은 /api/nav 서버)
#[path = "../../crates/gamji/src/nav.rs"]
#[allow(dead_code)]
mod nav;
mod ask;
mod hud;
mod nb;
mod reader;
mod upload;

/// cloud.rs 가 쓰는 `crate::collector::now()`
mod collector {
    pub fn now() -> f64 {
        crate::now()
    }
}

static HUD: OnceLock<Hud> = OnceLock::new();

/// 모듈 설정 '게임 화면 알림 끄기' (1.0.1) = 패널 'HUD 끄기' (1.0.4). 켜면 위 줄·알림을 숨긴다 (읽기·올리기는 그대로).
/// 1.0.4: 남만 그래프 틱 HUD 는 켜 두어도 보인다
static HUD_OFF: AtomicBool = AtomicBool::new(false);

/// 1.0.8: 새 모듈 버전 (service update.js 가 storage ui.update 를 보고 보냄). Navigator 알림은 켤 때 한 번이라 놓치기 쉬워,
/// 도시에서 게임 화면 HUD 가 보일 때 한 번 알린다
static UPDATE_V: Mutex<Option<String>> = Mutex::new(None);

/// 1.0.6: HUD 위치 {x, y} (0~1). 숫자가 아니면 기본 자리
fn pos_of(v: &Value) -> Option<(f64, f64)> {
    Some((v["x"].as_f64()?, v["y"].as_f64()?)).filter(|(x, y)| x.is_finite() && y.is_finite())
}

fn set_hud_off(v: &Value) {
    let off = v.as_bool().unwrap_or(false);
    if HUD_OFF.swap(off, Ordering::Relaxed) != off {
        log(if off { "[HUD] HUD 끔 (남만 틱은 보임)" } else { "[HUD] HUD 켬" });
    }
}

/// 모듈 설정 '캐릭터 이름' (0.5.1). 비어 있으면 화면에서 읽은 이름을 쓴다
static SET_NAME: Mutex<String> = Mutex::new(String::new());

fn set_name(v: &Value) {
    let n = v.as_str().unwrap_or("").trim().to_string();
    let ok = n.is_empty() || upload::by_ok(&n);
    let mut g = SET_NAME.lock().unwrap();
    if *g != n || !ok {
        log(&match (n.is_empty(), ok) {
            (true, _) => "[등록자] 설정 비어 있음 · 화면에서 읽은 캐릭터 이름을 씁니다".to_string(),
            (false, true) => format!("[등록자] 설정 이름 {n}"),
            (false, false) => format!("[등록자] 설정 이름 '{n}'은 쓸 수 없는 글자가 있어 무시합니다"),
        });
    }
    *g = if ok { n } else { String::new() };
}

/// 데이터 폴더의 캐릭터 이름 파일 (0.5.2). Navigator 2.2.1-beta.1 모듈 카드에 설정 칸이 안 보여서 둔 예비 길.
/// 모듈 카드 '폴더 열기'로 열어 메모장으로 적는다. '#' 으로 시작하는 줄과 빈 줄은 건너뛴다
const NAME_FILE: &str = "캐릭터이름.txt";


fn name_file_init(data: &Option<PathBuf>) {
    let Some(d) = data else { return };
    let p = d.join(NAME_FILE);
    if !p.exists() {
        let _ = std::fs::write(&p, "# 캐릭터 이름을 한 줄에 하나씩 적습니다 (여러 캐릭터 가능). 시세를 올릴 때 등록자로 보입니다.
# 게임 화면의 이름과 맞는 줄을 골라 씁니다. 20자까지, . $ # [ ] / 는 쓸 수 없습니다.
");
    }
}

/// 입력 창에서 적은 이름을 파일에 더한다 (0.6.4, 2026-11-22 사용자 "한 번 입력했으면 알아서 저장, 접속할 때마다 묻는 것 치명적").
/// 0.5.4~0.6.3 은 이름 하나만 두고 덮어써서 캐릭터를 바꿀 때마다 다시 물었다. 이제 캐릭터마다 한 줄씩 쌓아 두고,
/// 화면 이름 칸이 그중 하나와 맞으면 그 이름으로 올린다
fn name_file_add(data: &Option<PathBuf>, name: &str) -> bool {
    let Some(d) = data else { return false };
    let mut names = name_file_read(data);
    if !names.iter().any(|n| n == name) {
        names.push(name.to_string());
    }
    let txt = format!("# 캐릭터 이름을 한 줄에 하나씩 적습니다 (여러 캐릭터 가능). 시세를 올릴 때 등록자로 보입니다.
# 게임 화면의 이름과 맞는 줄을 골라 씁니다. 20자까지, . $ # [ ] / 는 쓸 수 없습니다.
{}
", names.join("\n"));
    std::fs::write(d.join(NAME_FILE), txt).is_ok()
}

/// 파일의 이름들 ('#' 줄·빈 줄·규칙에 안 맞는 줄은 건너뜀, 같은 이름은 한 번)
fn name_file_read(data: &Option<PathBuf>) -> Vec<String> {
    let Some(txt) = data.as_ref().and_then(|d| std::fs::read_to_string(d.join(NAME_FILE)).ok()) else { return vec![] };
    let mut out: Vec<String> = vec![];
    for l in txt.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if !l.is_empty() && !l.starts_with('#') && upload::by_ok(l) && !out.iter().any(|n| n == l) {
            out.push(l.to_string());
        }
    }
    out
}

/// println! 대신: 기록에 남기고, 올리기 실패는 HUD 에도 알린다
fn say_out(msg: &str) {
    log(msg);
    if msg.contains("올리기 실패") {
        if let Some(h) = HUD.get() {
            h.say("upload", vec![p("공유 서버에 올리지 못함 · 잠시 뒤 다시 보냅니다", WARN)], 5000);
        }
    }
}

use hud::{Hud, MUTED, WARN, p, pb, server_color};
use reader::{Event, Reader};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const FAST_MS: u64 = 100;
const SLOW_MS: u64 = 2500;
/// 남만무역정보 창이 열려 있을 때 (분류를 방향키로 넘기면 0.03초마다 바뀜, 감지기와 같음)
const NB_MS: u64 = 15;
// ---------- 1.0.5 웹 지도 네비 (2026-11-25 사용자: "모듈을 켜도 우리 웹 앱에서도 네비게이션 기능이 작동하도록") ----------
// 감지기(gamji collector.rs update_nav)와 같은 방법: 바다에서 측량 좌표가 보이면 0.25초마다 읽어 127.0.0.1:38765 /api/nav 로 보낸다.
// 공유 서버에는 올리지 않는다. 웹사이트 열기 링크에 ?nav=포트 를 붙여, 웹이 이 포트를 기억한다
const NAV_MS: u64 = 250;
const NAV_HD_STEP: f64 = 0.5;
const NAV_HD_JUMP: f64 = 20.0;
const NAV_LOST_SEC: f64 = 5.0;
static NAV_PORT: OnceLock<Option<u16>> = OnceLock::new();

/// 두 방향(도)의 차이 (-180~180)
fn ang_diff(a: f64, b: f64) -> f64 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}

/// 한 게임 창의 네비 상태 (감지기 St 의 nav_* 칸)
#[derive(Default)]
struct NavSt {
    t_nav: f64,
    t_sent: f64,
    xy: Option<(u32, u32)>,
    hd: Option<f64>,
    bw: Option<f64>,
    hd_raw: Option<f64>,
    town: Option<(String, String)>,
}

impl NavSt {
    /// 측량 좌표 하나 → 지도. 좌표를 읽고 있는 중이면 true
    #[allow(clippy::too_many_arguments)]
    fn step(&mut self, hub: &nav::NavHub, h: isize, chr: &str, coords: Option<(u32, u32)>, t: f64, heading: Option<f64>, bow: Option<f64>) -> bool {
        let raw0 = std::mem::replace(&mut self.hd_raw, heading);
        let mut heading = heading;
        if let Some(r0) = raw0 {
            if heading.is_none_or(|hd| ang_diff(hd, r0).abs() > NAV_HD_JUMP) {
                heading = self.hd;
            }
        }
        if let Some(c) = coords {
            if self.t_nav == 0.0 {
                log(&format!("[네비] 측량 좌표를 읽기 시작함 {},{} · 웹 지도에 보냅니다", c.0, c.1));
            }
            self.t_nav = t;
            let moved = |a: Option<f64>, b: Option<f64>| match (a, b) {
                (Some(a), Some(b)) => ang_diff(a, b).abs() >= NAV_HD_STEP,
                (None, None) => false,
                _ => true,
            };
            let turned = moved(heading, self.hd) || moved(bow, self.bw);
            if Some(c) != self.xy || turned || t - self.t_sent > 5.0 {
                (self.xy, self.hd, self.bw, self.t_sent) = (Some(c), heading, bow, t);
                hub.update(h, chr, c.0, c.1, t, heading, bow);
            }
            true
        } else if self.t_nav > 0.0 && t - self.t_nav > NAV_LOST_SEC {
            log("[네비] 측량 좌표가 안 보임");
            (self.t_nav, self.xy, self.hd, self.bw) = (0.0, None, None, None);
            hub.drop_ship(h);
            false
        } else {
            self.t_nav > 0.0
        }
    }
}
/// 남만 도시·분류 이름 강조 색 (감지기 HL_COLOR)
const HL: u32 = 0xff9a2e;
const NET_EVERY: f64 = 2.0;
const EXE_MUTEX: &str = "Local\\dho-price-collector-A";

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

// ---------- 기록 ----------

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

fn local_hms() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!("{:02}-{:02} {:02}:{:02}:{:02}", t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// 데이터 폴더의 side-log.txt 에 한 줄 (2 MB 가 넘으면 side-log.old.txt 로 옮기고 새로 쓴다). 점검 명령은 표준 오류로
// 0.13.0 (2026-11-24 사용자: 모듈 안에 웹사이트 링크): 패널 '웹사이트 열기' → service → 여기서 기본 브라우저로 연다.
// Navigator SDK에는 외부 주소를 여는 API가 없어서 sidecar가 연다. 주소는 고정 (메시지로 받은 주소는 쓰지 않음), 2초에 한 번만
// 0.14.0: page "install" 이면 모듈 설치 페이지 (업데이트 띠의 버튼)
const SITE_URL: &str = "https://real-time-dho.web.app/";
const INSTALL_URL: &str = "https://real-time-dho.web.app/module.html";
fn open_site(install: bool) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    {
        let mut l = LAST.lock().unwrap();
        if l.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) { return; }
        *l = Some(std::time::Instant::now());
    }
    // 1.0.5: 웹 지도가 이 PC 의 네비 포트를 기억하게 ?nav=포트 (웹 app.js NAV_PORT)
    let url = match (install, NAV_PORT.get().copied().flatten()) {
        (true, _) => INSTALL_URL.to_string(),
        (false, Some(p)) => format!("{SITE_URL}?nav={p}"),
        (false, None) => SITE_URL.to_string(),
    };
    let r = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL) };
    log(&format!("[웹사이트] 열기 {}", if r.0 as isize > 32 { "됨" } else { "실패" }));
}

fn log(msg: &str) {
    match LOG_PATH.get().and_then(|p| p.as_ref()) {
        Some(p) => {
            if std::fs::metadata(p).map(|m| m.len() > 2_000_000).unwrap_or(false) {
                let _ = std::fs::rename(p, p.with_file_name("side-log.old.txt"));
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "{} {msg}", local_hms());
            }
        }
        None => eprintln!("{msg}"),
    }
}

// ---------- Navigator 와 주고받기 ----------

static OUT: Mutex<()> = Mutex::new(());
static GONE: AtomicBool = AtomicBool::new(false);

fn write_line(v: &Value) {
    let _g = OUT.lock().unwrap();
    let mut o = io::stdout().lock();
    let ok = serde_json::to_writer(&mut o, v).is_ok() && o.write_all(b"\n").is_ok() && o.flush().is_ok();
    if !ok {
        GONE.store(true, Ordering::SeqCst); // Host 가 세션을 끝냄
    }
}

/// target: 화면에서 읽은 결과면 Some((역할, 세대)), 대상과 무관하면 None
fn emit(target: Option<(&str, u64)>, data: Value) {
    log(&format!("알림 {data}"));
    let t = target.map_or(Value::Null, |(role, g)| json!({ "role": role, "generation": g }));
    write_line(&json!({ "t": "emit", "target": t, "data": data }));
}

/// Navigator 가 알려 준 게임 창 (preferred 역할)
#[derive(Clone, Debug, PartialEq)]
struct Target {
    role: String,
    generation: u64,
    pid: u32,
    hwnd: isize,
    width: i64,
    height: i64,
}

fn parse_target(line: &Value) -> Option<Target> {
    let role = line["preferred"].as_str()?.to_string();
    let view = &line[role.as_str()];
    let hwnd = isize::from_str_radix(view["hwnd"].as_str()?.trim_start_matches("0x"), 16).ok()?;
    Some(Target {
        generation: line[format!("{role}Generation")].as_u64().unwrap_or(0),
        pid: view["pid"].as_u64().unwrap_or(0) as u32,
        hwnd,
        width: view["clientWidth"].as_i64().unwrap_or(0),
        height: view["clientHeight"].as_i64().unwrap_or(0),
        role,
    })
}

// ---------- 서버 (접속 주소) ----------

struct Net {
    book: netaddr::AddrBook,
    t: f64,
    server: Option<String>,
    miss: u32,
    addrs: BTreeSet<String>,
}

/// 에이레네 접속 주소 (모듈 전용. config.js 에 넣으면 헬폴 감지기 exe 동작도 바뀌므로 여기에만 둔다).
/// 2026-11-22 사용자가 에이레네 캐릭터(태아마트)로 접속해 side-log 에서 본 주소. 다른 주소가 보이면 더한다
const EIRENE_RANGES: [(&str, &str); 1] = [("203.242.208.207", "에이레네")];

impl Net {
    fn new(data: Option<&PathBuf>) -> Self {
        let cfg = config::read_config();
        // 모듈은 한 프로그램이 세 서버를 모두 맡는다 → 감지기와 달리 단독 서버(에이레네)도 주소로 판별한다
        let servers: Vec<String> = cfg.as_ref().map(|c| c.servers.clone()).unwrap_or_default();
        let mut ranges = cfg.as_ref().map(|c| c.ranges.clone()).unwrap_or_default();
        ranges.extend(EIRENE_RANGES.iter().map(|(a, s)| (a.to_string(), s.to_string())));
        let book = netaddr::AddrBook::new(
            &cfg.as_ref().map(|c| c.addrs.clone()).unwrap_or_default(),
            &servers,
            data.map(|d| d.join("addrs.json")),
            &ranges,
        );
        Net { book, t: 0.0, server: None, miss: 0, addrs: BTreeSet::new() }
    }

    fn reset(&mut self) {
        self.t = 0.0;
        self.server = None;
        self.miss = 0;
        self.addrs.clear();
    }

    /// 2초마다 게임의 연결 목록 → 서버. 바뀌었으면 Some(새 서버). 아는 주소가 한 번 안 보인 것은 넘긴다 (감지기와 같음)
    fn update(&mut self, pid: u32, t: f64) -> Option<Option<String>> {
        if t - self.t < NET_EVERY {
            return None;
        }
        self.t = t;
        let addrs = if pid != 0 { netaddr::remote_addrs(pid) } else { BTreeSet::new() };
        let net = self.book.server_of(&addrs);
        if addrs != self.addrs {
            let known: Vec<String> = addrs.iter().map(|a| format!("{a}={}", self.book.lookup(a).unwrap_or_else(|| "?".into()))).collect();
            log(&format!("[서버] pid {pid} 연결 {}", if known.is_empty() { "(없음)".into() } else { known.join(", ") }));
            self.addrs = addrs;
        }
        if net.is_some() {
            self.miss = 0;
        } else if self.server.is_some() {
            self.miss += 1;
            if self.miss < 2 {
                return None;
            }
        }
        if net != self.server {
            self.server = net.clone();
            return Some(net);
        }
        None
    }
}

// ---------- 감지기 exe 가 켜져 있나 ----------

fn exe_running() -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let w: Vec<u16> = EXE_MUTEX.encode_utf16().chain(Some(0)).collect();
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, windows::core::PCWSTR(w.as_ptr())) } {
        Ok(h) => {
            unsafe {
                let _ = CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
}

// ---------- 일꾼: 화면 복사 → 읽기 → 알림 ----------

fn recognizer(data: Option<&PathBuf>) -> recog::Recognizer {
    let (goods, cities) = config::load_names();
    recog::Recognizer::new(&goods, &cities, data.map(|d| d.join("icons")))
}

fn event_json(ev: &Event, server: &Option<String>) -> Value {
    match ev {
        Event::City(c) => json!({ "kind": "city", "city": c }),
        Event::Note(m) => json!({ "kind": "note", "text": m }),
        Event::News { city, texts } => json!({ "kind": "news", "server": server, "city": city, "texts": texts, "ready": server.is_some() }),
        Event::Nanban { culture, city, kind, pts, states, seen, .. } => json!({
            "kind": "nanban", "server": server, "ready": server.is_some(), "culture": culture, "city": city, "cat": kind,
            "pts": pts, "states": states.iter().map(|(c, v)| json!([c, v])).collect::<Vec<_>>(), "seen": seen,
        }),
        // 0.9.0: 인근 도시 시세는 지금 도시를 몰라도 올린다 (서버만 알면 됨, 감지기와 같음)
        Event::Near { good, items } => json!({
            "kind": "near", "server": server, "good": good, "ready": server.is_some(),
            "items": items.iter().map(|n| n.json()).collect::<Vec<_>>(),
        }),
        Event::Trade { kind, city, rows } => {
            // 올릴 수 있는 상태인가 (3단계까지는 올리지 않는다. 판단만 같이 알린다)
            let why = if server.is_none() {
                Some("서버를 모름")
            } else if city.is_none() {
                Some("도시를 모름")
            } else {
                None
            };
            json!({
                "kind": "trade", "type": kind, "server": server, "city": city,
                "ready": why.is_none(), "why": why,
                "rows": rows.iter().map(|r| r.json()).collect::<Vec<_>>(),
            })
        }
    }
}

// ---------- 게임 화면 HUD (0.4.0: sidecar 가 직접 그린다) ----------

/// 위 줄: 헬레네 · 리스본 / 서버 확인 중 / 서버 모름
fn status_parts(server: &Option<String>, checked: bool, city: &Option<String>) -> Vec<hud::Part> {
    let mut v = match server {
        Some(s) => vec![pb(s.clone(), server_color(s))],
        None if checked => vec![p("서버 모름", WARN)],
        None => vec![p("시세 모듈 · 서버 확인 중", MUTED)],
    };
    if let Some(c) = city {
        v.push(p(format!(" · {c}"), hud::FG));
    }
    v
}

/// 아래 줄: 읽은 것을 한 줄 알림으로 (3단계 service 문구 그대로)
fn hud_say(h: &Hud, data: &Value) {
    let fg = hud::FG;
    match data["kind"].as_str() {
        Some("server") => match data["server"].as_str() {
            Some(s) => h.say("server", vec![pb(s, server_color(s)), p(" 서버 확인", fg)], 2500),
            None => h.say("server", vec![p("서버를 모름 · 시세를 올리지 않습니다", WARN)], 4000),
        },
        Some("city") => {
            if let Some(c) = data["city"].as_str() {
                h.say("city", vec![p(format!("{c} 도착 · "), fg), p("교역소 창을 열면 시세를 읽습니다", MUTED)], 3000);
            }
        }
        Some("note") => h.say("note", vec![p(data["text"].as_str().unwrap_or(""), fg)], 2500),
        // 0.10.0 교역소 대사: 올렸을 때만 짧게
        Some("news") => {
            let n = data["uploaded"].as_u64().unwrap_or(0);
            if n > 0 {
                let texts = data["texts"].as_array().cloned().unwrap_or_default();
                let last = texts.last().and_then(|v| v.as_str()).unwrap_or("");
                h.say("news", vec![p(format!("{} 교역소 대사 · ", data["city"].as_str().unwrap_or("")), fg), p(last, MUTED)], 3500);
            }
        }
        // 0.10.0 남만무역: 도시·분류를 새로 읽을 때마다 '나가사키의 식료품 재고 인식' (감지기와 같은 글, 도시·분류는 주황)
        Some("nanban") => {
            if data["ready"] != true {
                if data["seen"] == true {
                    h.say("nanban", vec![p("서버를 몰라 남만무역 정보를 올리지 않습니다", WARN)], 4000);
                }
                return;
            }
            if data["seen"] != true {
                return;
            }
            let (Some(c), Some(k)) = (data["city"].as_str(), data["cat"].as_str()) else { return };
            h.say("nanban", vec![pb(c, HL), p("의 ", fg), pb(k, HL), p(" 재고 인식", fg)], 2500);
        }
        Some("near") => {
            if data["ready"] != true {
                h.say("trade", vec![p("서버를 몰라 인근 시세를 올리지 않습니다", WARN)], 4000);
                return;
            }
            let items = data["items"].as_array().cloned().unwrap_or_default();
            let first = items.first().map_or(String::new(), |r| {
                format!("{} {}%{}", r["city"].as_str().unwrap_or(""), r["pct"], r["tr"].as_str().unwrap_or(""))
            });
            let more = if items.len() > 1 { format!(" 외 {}곳", items.len() - 1) } else { String::new() };
            let srv = data["server"].as_str().unwrap_or("");
            let tail = match data["uploaded"].as_str() {
                Some(to) if Some(to) == upload::TEST_SERVER => {
                    p(format!("· {} 캐릭터로 시험 서버에 올림", data["by"].as_str().unwrap_or("미확인")), MUTED)
                }
                Some(_) => p(format!("· {} 캐릭터로 올림", data["by"].as_str().unwrap_or("미확인")), fg),
                None => p("· 올리지 못함 (공유 서버 설정 없음)", WARN),
            };
            h.say(
                "trade",
                vec![pb(srv, server_color(srv)), p(format!(" 인근 시세 {} · {first}{more} ", data["good"].as_str().unwrap_or("")), fg), tail],
                6000,
            );
        }
        Some("trade") => {
            if data["ready"] != true {
                let why = match data["why"].as_str() {
                    Some("서버를 모름") => "서버를 몰라",
                    Some("도시를 모름") => "도시를 몰라",
                    Some(w) => w,
                    None => "",
                };
                h.say("trade", vec![p(format!("{why} 시세를 올리지 않습니다"), WARN)], 4000);
                return;
            }
            let rows = data["rows"].as_array().cloned().unwrap_or_default();
            let first = rows.first().map_or(String::new(), |r| {
                let pct = match &r["pct"] {
                    Value::Number(n) => format!("{n}%"),
                    v => v.as_str().unwrap_or("").to_string(),
                };
                format!("{} {} {}{}", r["name"].as_str().unwrap_or(""), r["price"], pct, r["tr"].as_str().unwrap_or(""))
            });
            let more = if rows.len() > 1 { format!(" 외 {}종", rows.len() - 1) } else { String::new() };
            let srv = data["server"].as_str().unwrap_or("");
            let kind = if data["type"] == "market" { "시세 창" } else { "매각" };
            // 4단계: 올린 곳과 등록자. 시험 서버면 그렇다고 흐리게 붙인다
            let tail = match data["uploaded"].as_str() {
                Some(to) if Some(to) == upload::TEST_SERVER => {
                    p(format!("· {} 캐릭터로 시험 서버에 올림", data["by"].as_str().unwrap_or("미확인")), MUTED)
                }
                Some(_) => p(format!("· {} 캐릭터로 올림", data["by"].as_str().unwrap_or("미확인")), fg),
                None => p("· 올리지 못함 (공유 서버 설정 없음)", WARN),
            };
            h.say(
                "trade",
                vec![pb(srv, server_color(srv)), p(format!(" {} {kind} {first}{more} ", data["city"].as_str().unwrap_or("")), fg), tail],
                6000,
            );
        }
        _ => {}
    }
}

/// 읽힌 글자가 캐릭터 이름 같은가 (감지기 collector.rs `plausible` 과 같음)
fn plausible(name: &str) -> bool {
    let n = name.chars().count();
    let hangul = name.chars().any(|c| ('가'..='힣').contains(&c));
    let latin = name.chars().any(|c| c.is_ascii_alphanumeric());
    (2..=12).contains(&n) && !(hangul && latin)
}

fn worker(target: Arc<Mutex<Option<Target>>>, data: Option<PathBuf>, stop: Arc<AtomicBool>) {
    let hud = HUD.get_or_init(|| Hud::start(false)).clone();
    let mut up = upload::Uploader::new(data.as_ref());
    log(&format!("[올리기] {}", match (up.ready(), upload::TEST_SERVER) {
        (false, _) => "공유 서버 설정이 없어 올리지 않습니다".to_string(),
        (true, Some(s)) => format!("시험 서버 '{s}' 경로에만 올립니다"),
        (true, None) => "읽은 서버에 올립니다".to_string(),
    }));
    let (mut checked, mut city): (bool, Option<String>) = (false, None);
    // 등록자: 왼쪽 위 함대 목록의 자기 이름 (교역 창이 없을 때 5초마다 읽는다)
    let (mut chr, mut t_chr): (Option<String>, f64) = (None, 0.0);
    name_file_init(&data);
    let (mut file_names, mut t_file): (Vec<String>, f64) = (vec![], -9.0);
    // 적힌 이름들(설정 + 파일)과 화면 확인 결과 (None = 아직 못 봄), 맞은 이름
    let (mut declared, mut verified, mut miss): (Vec<String>, Option<bool>, u32) = (vec![], None, 0);
    let mut matched: Option<String> = None;
    // 이름 입력 창 (0.5.4): 이름이 없을 때 한 번, 파일 이름이 화면과 다를 때 한 번. '나중에'면 이번 실행 동안 다시 안 묻는다
    let (mut ask, mut asked_missing, mut asked_mismatch): (Option<ask::Handle>, bool, bool) = (None, false, false);
    // 입력 창에서 '나중에'를 눌렀으면 이번 실행 동안 다시 묻지 않는다 (캐릭터를 바꿔도)
    let mut said_later = false;
    // 1.0.4: 게임 창(pid)마다 HUD 로 알린 캐릭터 이름. 다시 붙어도(해역 이동·창 바꾸기) 지우지 않는다 → 같은 이름이면 다시 안 알림
    let mut announced: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut t_game0 = 0.0;
    // 화면 이름 칸에서 마지막으로 읽힌 글자 (입력 창에 미리 채운다)
    let mut seen_name: Option<String> = None;
    let t0 = std::time::Instant::now();
    let mut rd = Reader::with_icons(recognizer(data.as_ref()), data.as_ref().map(|d| d.join("nanban_icons")));
    log(&format!("[준비] 이름 목록·글자 그림 {:?}", t0.elapsed()));
    let mut net = Net::new(data.as_ref());
    let mut cur: Option<Target> = None;
    let mut cap: Option<capture::WindowCapture> = None;
    let mut seen_at = 0.0;
    let mut paused: Option<bool> = None;
    let mut t_exe = 0.0;
    // 1.0.5: 웹 지도 네비. 서버는 처음 한 번 (감지기 exe 가 먼저 38765 를 쓰고 있으면 다음 번호)
    let nav_hub = Arc::new(nav::NavHub::default());
    let port = nav::start_server(nav_hub.clone());
    log(&match port {
        Some(p) => format!("[네비] 웹 지도용 127.0.0.1:{p} 열림"),
        None => "[네비] 포트를 열지 못함 · 웹 지도 네비 꺼짐".into(),
    });
    let _ = NAV_PORT.set(port);
    let mut navs: HashMap<isize, NavSt> = HashMap::new();
    let mut update_told: Option<String> = None;
    while !stop.load(Ordering::SeqCst) && !GONE.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(20));
        let t = now();
        // 감지기 exe 와 같이 켜져 있으면 쉰다 (5초마다 확인)
        if t - t_exe >= 5.0 {
            t_exe = t;
            let run = exe_running();
            if paused != Some(run) {
                paused = Some(run);
                emit(None, json!({ "kind": "exe", "running": run }));
                if run {
                    log("[감지기] 감지기 exe 가 켜져 있음 · 모듈은 화면을 읽지 않고 기다립니다");
                }
            }
        }
        let want = if paused == Some(true) { None } else { target.lock().unwrap().clone() };
        // HUD 는 감지기 exe 가 켜져 있으면 숨는다 (감지기 HUD 와 같은 자리라 겹치지 않게)
        // 1.0.4: 'HUD 끄기'는 위 줄·알림만 숨긴다 (남만 틱 HUD 는 보이게 게임 창은 그대로 준다)
        hud.set_game(want.as_ref().map(|w| w.hwnd));
        hud.set_off(HUD_OFF.load(Ordering::Relaxed));
        // 대상 창이 바뀜: 복사를 새로 시작하고 읽던 것을 버린다 (같은 창의 크기·활성 변화는 그대로)
        let same = match (&cur, &want) {
            (Some(a), Some(b)) => a.hwnd == b.hwnd && a.pid == b.pid && a.generation == b.generation,
            (None, None) => true,
            _ => false,
        };
        // 1.0.8: 해역 이동·창 바꾸기 때 Navigator 는 같은 창(같은 hwnd·pid)을 세대만 바꿔 다시 준다 (기록에 하루 수백 번).
        // 그때마다 복사·도시·이름 확인을 처음부터 하면 잠깐 도시를 모르고 등록자가 '미확인'이 되므로, 같은 창이면 이어서 읽는다
        let same_window = cap.is_some() && matches!((&cur, &want), (Some(a), Some(b)) if a.hwnd == b.hwnd && a.pid == b.pid);
        if !same && same_window {
            cur = want.clone();
        } else if !same {
            if let Some(c) = cap.take() {
                c.stop();
            }
            rd.reset();
            net.reset();
            seen_at = 0.0;
            (checked, city) = (false, None);
            (chr, t_chr) = (None, 0.0);
            (verified, miss, matched) = (None, 0, None);
            (t_game0, seen_name) = (now(), None);
            hud.set_status(status_parts(&None, false, &None));
            match &want {
                Some(w) => {
                    log(&format!("[게임 창] 연결 · {} 세대 {} · {}×{}", w.role, w.generation, w.width, w.height));
                    cap = Some(capture::WindowCapture::new(w.hwnd, SLOW_MS));
                }
                None => log("[게임 창] 없음 · 화면 복사를 멈춥니다"),
            }
            cur = want;
        }
        let (Some(tg), Some(c)) = (&cur, &cap) else { continue };
        let role = Some((tg.role.as_str(), tg.generation));
        let before = net.server.clone();
        if let Some(srv) = net.update(tg.pid, t) {
            // 접속이 바뀌면(캐릭터 바꾸기·서버 이동) 이름 확인을 처음부터 (0.6.3, 2026-11-22: 에이레네 티아마트 → 헬레네 캐릭터로 바꾼 뒤
            // 첫 시세가 이름 칸을 다시 보기 전에 '티아마트'로 올라갔음). 다시 맞춰 볼 때까지 등록자는 '미확인'
            if verified.is_some() || chr.is_some() {
                log("[등록자] 접속이 바뀌어 캐릭터 이름을 다시 확인합니다");
            }
            (verified, miss, t_chr, matched) = (None, 0, 0.0, None);
            chr = None;
            // 다른 캐릭터면 이름 창을 다시 띄울 수 있게 ('나중에'를 눌렀으면 그대로)
            asked_mismatch = said_later;
            let d = json!({ "kind": "server", "server": srv });
            emit(role, d.clone());
            checked = true;
            hud.set_status(status_parts(&net.server, checked, &city));
            // 1.0.1: 알던 서버에서 접속이 끊기거나 바뀜 = 캐릭터를 바꿈 (감지기 '캐릭터가 바뀜 · 서버 확인 중'과 같은 뜻)
            if before.is_some() && srv.is_none() {
                hud.say("server", vec![p("캐릭터가 바뀜. 서버 재인식", hud::FG)], 3000);
            } else {
                hud_say(&hud, &d);
            }
        }
        let Some(frame) = c.latest() else { continue };
        if frame.at == seen_at || t - frame.at > 5.0 {
            continue;
        }
        seen_at = frame.at;
        rd.news_on = net.server.is_some();
        let (open, mut events) = rd.frame(&frame.rgb, c.client_offset(), t);
        for l in rd.take_log() {
            log(&l);
        }
        hud.set_tick(rd.tick);
        // 1.0.2: 바다에서는 게임 화면 HUD 를 숨긴다 (도착하면 City 이벤트보다 먼저 풀려 '도착' 알림은 보인다)
        // 1.0.5: 바다 지명을 읽었을 때가 아니라 '도시를 읽은 상태가 아닐 때' 숨긴다. 바다에서 모듈을 켜거나 게임 창에 다시 붙으면
        // 바다 지명을 못 읽는 동안 sea 가 false 로 남아 '헬레네' 같은 HUD 가 바다에서 떴다. 도시 HUD 는 도시를 읽은 뒤에만
        // (이 프레임에서 도시를 새로 읽었으면 rd.city 가 이미 채워져 있어 '도착' 알림은 보인다)
        // 1.0.5: 웹 지도 네비. 교역 창이 열려 있지 않으면 측량 좌표를 읽는다 (감지기와 같음)
        let coords = if open { None } else { recog::nav::read_coords(&frame.rgb, c.client_offset()) };
        // 1.0.8: 측량 좌표는 바다에서만 보인다 → 지명을 못 읽어도 앞 도시를 지우고 HUD 를 숨긴다
        if coords.is_some() {
            if let Some(ev) = rd.at_sea() {
                log("[도시] 측량 좌표가 보여 바다로 봅니다 · 앞 도시를 지웁니다");
                events.push(ev);
            }
        }
        hud.set_sea(rd.hud_hidden());
        // 1.0.8: 새 버전 알림 (도시에서 HUD 가 보일 때, 버전마다 이 실행에서 한 번)
        if !rd.hud_hidden() && !HUD_OFF.load(Ordering::Relaxed) {
            let v = UPDATE_V.lock().unwrap().clone();
            if v.is_some() && v != update_told {
                update_told = v.clone();
                let v = v.unwrap();
                log(&format!("[업데이트] 새 버전 {v} 알림"));
                hud.say("update", vec![p(format!("시세 모듈 새 버전 {v} · "), hud::FG), p("Navigator 패널 맨 위 '업데이트' 버튼으로 설치", MUTED)], 8000);
            }
        }
        let (hd, bw) = match coords {
            Some(_) => (recog::nav::read_heading(&frame.rgb, c.client_offset()), recog::nav::read_bow(&frame.rgb, c.client_offset())),
            None => (None, None),
        };
        let who = matched.clone().or_else(|| chr.clone()).unwrap_or_default();
        let sailing = navs.entry(tg.hwnd).or_default().step(&nav_hub, tg.hwnd, &who, coords, t, hd, bw);
        // 지금 안 보는 창(다른 클라이언트로 바뀜)의 배는 좌표가 끊긴 것처럼 지운다
        navs.retain(|h, s| {
            let gone = *h != tg.hwnd && t - s.t_nav > NAV_LOST_SEC;
            if gone && s.t_nav > 0.0 {
                nav_hub.drop_ship(*h);
                (s.t_nav, s.xy, s.hd, s.bw) = (0.0, None, None, None);
            }
            !gone || s.town.is_some()
        });
        // 복사 간격: 남만 창 0.015초, 교역 창이 보이면 0.1초, 닫힌 지 1.5초가 지나면 2.5초
        if rd.nb_corner.is_some() {
            c.set_interval(NB_MS);
        } else if open {
            c.set_interval(FAST_MS);
        } else if sailing {
            c.set_interval(NAV_MS);
        } else if t - rd.t_trade > 1.5 {
            c.set_interval(SLOW_MS);
        }
        if t - t_file >= 5.0 {
            t_file = t;
            let n = name_file_read(&data);
            if n != file_names {
                log(&format!("[등록자] {NAME_FILE} 이름 {}", if n.is_empty() { "(없음)".into() } else { n.join(", ") }));
                file_names = n;
            }
        }
        // 설정 칸 이름이 있으면 맨 앞, 그다음 파일의 이름들
        let mut setting: Vec<String> = vec![];
        let set = SET_NAME.lock().unwrap().clone();
        if !set.is_empty() {
            setting.push(set);
        }
        for n in &file_names {
            if !setting.contains(n) {
                setting.push(n.clone());
            }
        }
        // 적힌 이름이 바뀌면 그 이름들을 게임 글꼴 그림으로 만들어 두고 화면과 다시 맞춰 본다 (0.5.3, 감지기와 같은 방법)
        if setting != declared {
            // 이름이 더해지기만 했고 지금 맞은 이름이 그대로 있으면 확인 결과는 그대로
            let keep = matched.as_ref().is_some_and(|m| setting.contains(m));
            declared = setting;
            rd.r.chars.set_names(&declared);
            if !keep {
                (verified, miss, t_chr, matched) = (None, 0, 0.0, None);
            } else {
                t_chr = 0.0;
            }
            if !declared.is_empty() {
                log(&format!("[등록자] 적힌 이름 {} · 화면 이름과 맞춰 봅니다", declared.join(", ")));
            }
        }
        if !open && t - t_chr >= 5.0 {
            t_chr = t;
            let (raw, sure) = rd.r.read_character(&frame.rgb, c.client_offset());
            for l in rd.take_log() {
                log(&l);
            }
            // 입력 창에 미리 채울 글자: 이름답지 않아도(숫자 섞임 등) 읽힌 대로. 한두 글자만 고치면 되게
            if let Some(n) = raw.clone().map(|n| n.trim().to_string()).filter(|n| upload::by_ok(n)) {
                seen_name = Some(n);
            }
            if !declared.is_empty() {
                match &raw {
                    // 글자 그림이 화면 이름 칸과 90% 넘게 같다 = 적힌 이름이 맞다
                    Some(n) if sure && declared.contains(n) => {
                        if verified != Some(true) || matched.as_ref() != Some(n) {
                            log(&format!("[등록자] {n} · 화면 이름과 맞음"));
                            if announced.get(&tg.pid) != Some(n) {
                                announced.insert(tg.pid, n.clone());
                                hud.say("name", vec![p(format!("{n} 캐릭터 확인 · 이 이름으로 올립니다"), hud::FG)], 3000);
                            }
                        }
                        (verified, miss, matched) = (Some(true), 0, Some(n.clone()));
                    }
                    // 이름 칸에 글자는 있는데 모양이 다르다: 두 번 연속이면 다른 캐릭터로 본다
                    Some(n) => {
                        miss += 1;
                        if miss >= 2 && verified != Some(false) {
                            (verified, matched) = (Some(false), None);
                            log(&format!("[등록자] 화면 이름이 적힌 이름({})과 다름 (읽힌 글자 {n}) · 등록자는 '미확인'", declared.join(", ")));
                            hud.say("name", vec![p("화면의 캐릭터 이름이 저장된 이름과 달라 등록자를 '미확인'으로 올립니다", WARN)], 6000);
                        }
                    }
                    // 이름 칸이 안 보임 (바다·메뉴 등): 마지막 결과 그대로
                    None => {}
                }
            } else {
                let n = raw.clone().filter(|n| plausible(n) && upload::by_ok(n));
                if n.is_none() && chr.is_none() {
                    capture::say_once(&format!("[캐릭터] 이름을 못 읽음 (읽힌 글자 {raw:?}) · 등록자는 '미확인'"));
                }
                if let Some(n) = n {
                    if chr.as_ref() != Some(&n) {
                        log(&format!("[캐릭터] {n}"));
                        if chr.is_some() {
                            hud.say("name", vec![p("캐릭터가 바뀜. 서버 재인식", hud::FG)], 3000);
                        }
                        chr = Some(n);
                    }
                }
            }
        }
        // 이름 입력 창: 교역 창이 닫혀 있고 게임 창이 맨 앞일 때만 (교역 중에 가리지 않게)
        let from_file = SET_NAME.lock().unwrap().is_empty();
        if ask.is_none() && !open && capture::foreground() == tg.hwnd {
            let why = if declared.is_empty() && !asked_missing && (seen_name.is_some() || t - t_game0 > 30.0) {
                asked_missing = true;
                Some(match &seen_name {
                    Some(_) => "화면에서 읽은 이름을 채워 두었습니다. 맞으면 Enter, 틀린 글자가 있으면 고쳐 주세요.".to_string(),
                    None => "화면에서 이름을 읽지 못했습니다. 직접 적어 주세요.".to_string(),
                })
            } else if verified == Some(false) && from_file && !asked_mismatch {
                asked_mismatch = true;
                Some("처음 보는 캐릭터입니다. 이 캐릭터 이름을 적어 주세요. 한 번 적으면 기억합니다.".to_string())
            } else {
                None
            };
            if let Some(why) = why {
                let pre = seen_name.clone().filter(|n| !declared.contains(n)).unwrap_or_default();
                log(&format!("[등록자] 이름 입력 창 · 미리 채운 이름 {}", if pre.is_empty() { "(없음)" } else { &pre }));
                ask = Some(ask::open(tg.hwnd, pre, why));
                hud.say("name", vec![p("캐릭터 이름 입력 창이 떴습니다 · 창을 눌러 확인해 주세요", WARN)], 8000);
            }
        }
        if let Some(a) = ask.as_ref().and_then(|h| h.lock().unwrap().clone()) {
            ask = None;
            match a {
                ask::Answer::Name(n) => {
                    let ok = name_file_add(&data, &n);
                    log(&format!("[등록자] 입력 창에서 {n} · {NAME_FILE}에 {}", if ok { "더함" } else { "저장 못 함" }));
                    if ok {
                        t_file = -9.0; // 바로 다시 읽는다
                        // 이미 있던 이름을 다시 적었으면 확인을 처음부터
                        if declared.contains(&n) {
                            (verified, miss, t_chr, matched) = (None, 0, 0.0, None);
                        }
                    }
                    hud.say("name", vec![p(format!("캐릭터 이름 {n} 저장 · 화면 이름과 맞춰 봅니다"), hud::FG)], 4000);
                }
                ask::Answer::Later => {
                    said_later = true;
                    log("[등록자] 입력 창 · 나중에");
                    hud.say("name", vec![p(format!("캐릭터 이름은 나중에 · 모듈 폴더의 {NAME_FILE}에도 적을 수 있습니다"), MUTED)], 5000);
                }
            }
        }
        for ev in &events {
            let mut d = event_json(ev, &net.server);
            // 적힌 이름은 화면과 맞을 때만 쓴다. 적힌 이름이 없으면 화면에서 읽은 이름 (확인 안 된 이름)
            let by = if !declared.is_empty() {
                matched.clone().filter(|_| verified == Some(true)).unwrap_or_else(|| "미확인".into())
            } else {
                chr.clone().unwrap_or_else(|| "미확인".into())
            };
            if let (Event::Near { good, items }, Some(srv), true) = (ev, &net.server, d["ready"] == true) {
                let to = up.near(srv, items, &by);
                let list: Vec<String> = items.iter().map(|n| format!("{} {}%{}", n.city, n.pct, n.tr)).collect();
                log(&format!("[올리기] {srv} 인근 {good}: {} · 등록자 {by} → {}", list.join(", "), to.as_deref().unwrap_or("못 올림")));
                d["by"] = json!(by);
                d["uploaded"] = json!(to);
            }
            if let (Event::News { city: nc, texts }, Some(srv)) = (ev, &net.server) {
                let n = up.news(srv, nc, texts);
                log(&format!("[소식] {srv} {nc}: {} · {}", texts.join(" / "), if n > 0 { format!("{n}건 올림") } else { "이미 올린 대사".into() }));
                d["uploaded"] = json!(n);
            }
            if let (Event::Nanban { culture, city: nc, kind, pts, raw, states, .. }, Some(srv)) = (ev, &net.server) {
                // 반올림 전 틱 (교환비 연구용): "소수 11.3 10.9 …"
                let raw_txt = raw.map(|r| format!(" · 소수 {}", r.iter().map(|x| format!("{x:.1}")).collect::<Vec<_>>().join(" "))).unwrap_or_default();
                // 1.0.10: 공유 서버에는 소수 틱 (못 읽었으면 정수)
                let vals: Option<Vec<f32>> = pts.map(|p| match raw {
                    Some(r) => r.iter().map(|x| x.clamp(0.0, 100.0)).collect(),
                    None => p.iter().map(|x| *x as f32).collect(),
                });
                let cat = match (nc, kind, &vals) {
                    (Some(c), Some(k), Some(v)) => Some((c.as_str(), k.as_str(), &v[..])),
                    _ => None,
                };
                if let Some((to, sent, cat_sent)) = up.nanban(srv, states, cat, &by) {
                    let st_txt: Vec<String> = sent.iter().map(|(c, v)| format!("{c} {v}")).collect();
                    let st_txt = if st_txt.is_empty() { String::new() } else { format!(" · 상태 {}", st_txt.join(", ")) };
                    match (cat_sent, cat) {
                        (true, Some((c, k, _))) => {
                            let p = pts.unwrap_or_default();
                            let pts: Vec<String> = p.iter().map(|x| x.to_string()).collect();
                            log(&format!("[남만] {to} {culture} {c} {k} {}틱 · {}{raw_txt}{st_txt} · 등록자 {by}", p[p.len() - 1], pts.join(" ")));
                        }
                        _ => log(&format!("[남만] {to} {culture}{st_txt} · 등록자 {by}")),
                    }
                    d["uploaded"] = json!(to);
                    d["by"] = json!(by);
                }
            }
            if let (Event::Trade { city: Some(tc), rows, .. }, Some(srv), true) = (ev, &net.server, d["ready"] == true) {
                let to = up.trade(srv, tc, rows, &by);
                log(&format!("[올리기] {} {tc} {}종 · 등록자 {by} → {}", srv, rows.len(), to.as_deref().unwrap_or("못 올림")));
                d["by"] = json!(by);
                d["uploaded"] = json!(to);
            }
            // 남만 화면은 방향키로 분류를 넘기면 1초에 30번도 바뀌어서 Navigator 에는 보내지 않는다 (emit 은 초당 평균 20개까지, 모듈 JS 는 안 씀)
            if d["kind"] != "nanban" {
                emit(role, d.clone());
            }
            if let Event::City(c) = ev {
                city = c.clone();
                // 1.0.5: 웹 지도 항해일지가 입항을 바로 마감하도록 (감지기 towns)
                if let Some(cn) = c {
                    let who = matched.clone().or_else(|| chr.clone()).unwrap_or_default();
                    let k = (cn.clone(), who);
                    let st = navs.entry(tg.hwnd).or_default();
                    if st.town.as_ref() != Some(&k) {
                        nav_hub.set_town(tg.hwnd, &k.1, &k.0, t);
                        st.town = Some(k);
                    }
                }
                hud.set_status(status_parts(&net.server, checked, &city));
            }
            hud_say(&hud, &d);
        }
    }
    if let Some(c) = cap {
        c.stop();
    }
}

// ---------- 점검: 그림 파일로 읽어 보기 ----------

/// `price-side --file 그림.png ...` : 그림마다 두 번 읽어(연속 2번 규칙) 결과를 찍는다. 게임 화면 시작점은 (2,39)
fn file_test(paths: &[String]) {
    let mut rd = Reader::new(recognizer(None));
    let mut t = now();
    for p in paths {
        let Some(rgb) = std::fs::read(p).ok().and_then(|b| recog::pngio::load_rgb(&b)) else {
            println!("{p}: 읽기 실패 (PNG만)");
            continue;
        };
        rd.reset();
        for _ in 0..2 {
            t += 2.0;
            let (open, evs) = rd.frame(&rgb, (2, 39), t);
            for l in rd.take_log() {
                println!("  {l}");
            }
            for e in &evs {
                println!("{p}: 교역창 {open} · {}", event_json(e, &Some("시험".into())));
            }
        }
    }
}

/// `price-side --hud-test` : 열린 게임 창 위에 HUD 를 맨 앞 여부와 상관없이 20초 동안 띄워 본다 (문구 차례대로)
fn hud_test() {
    let Some(&g) = capture::find_game_windows().first() else {
        println!("게임 창이 없습니다");
        return;
    };
    let h = Hud::start(true);
    h.set_game(Some(g));
    h.set_status(status_parts(&None, false, &None));
    let steps: Vec<(u64, Value)> = vec![
        (1500, json!({ "kind": "server", "server": "헬레네" })),
        (3000, json!({ "kind": "city", "city": "리스본" })),
        (3500, json!({ "kind": "trade", "type": "sell", "server": "헬레네", "city": "리스본", "ready": true, "why": null,
            "rows": [{"name":"브랜디","price":360,"pct":110,"tr":"↘"},{"name":"수정","price":731,"pct":108,"tr":"↗"}] })),
        (7000, json!({ "kind": "trade", "type": "market", "server": null, "city": "리스본", "ready": false, "why": "서버를 모름", "rows": [] })),
    ];
    let mut city = None;
    for (wait, d) in steps {
        std::thread::sleep(Duration::from_millis(wait));
        if d["kind"] == "city" {
            city = d["city"].as_str().map(String::from);
        }
        let srv = if d["kind"] == "trade" && d["server"].is_null() { None } else { Some("헬레네".to_string()) };
        h.set_status(status_parts(&srv, true, &city));
        hud_say(&h, &d);
        println!("{d}");
    }
    // 0.10.0 남만: '재고 인식' 알림 + 틱 HUD (남만 창이 (300,150)에 있다고 치고 98틱 → 19틱)
    hud_say(&h, &json!({ "kind": "nanban", "ready": true, "seen": true, "city": "안평", "cat": "식료품" }));
    for i in 0..50 {
        let v: u8 = if i < 25 { 98 } else { 19 };
        h.set_tick(Some((300 + recog::nanban::WIN_W + 10, 150 + recog::nanban::tick_y(v), v as f32 + 0.3)));
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(2000));
}

/// `price-side --upload-test [폴더]` : 시험 서버 리스본에 브랜디 한 줄을 올리고 다시 읽어 본다 (등록자 '모듈시험')
fn upload_test(dir: Option<PathBuf>) {
    let Some(test) = upload::TEST_SERVER else {
        eprintln!("시험 서버가 꺼져 있어 하지 않습니다");
        return;
    };
    let mut up = upload::Uploader::new(dir.as_ref());
    let row = reader::Row { name: "브랜디".into(), price: 360, pct: recog::Pct::Num(110), tr: "↘", arrow: None };
    let to = up.trade("헬레네", "리스본", &[row], "모듈시험");
    eprintln!("보냄 → {to:?}");
    std::thread::sleep(Duration::from_secs(6));
    let Some(cfg) = config::read_config() else { return };
    let auth = dir.unwrap_or_else(std::env::temp_dir).join("fb_auth.json");
    let cl = cloud::Cloud::start(&cfg.api_key, &cfg.database_url, auth);
    match cl.get(&format!("servers/{}/cities/{}/prices/{}/sell", cloud::fkey(test), cloud::fkey("리스본"), cloud::fkey("브랜디"))) {
        Ok(v) => eprintln!("다시 읽음: {v}"),
        Err(e) => eprintln!("다시 읽기 실패: {e}"),
    }
}

fn main() {
    capture::set_dpi_aware();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--upload-test") {
        upload_test(args.get(2).map(PathBuf::from));
        return;
    }
    // `price-side --ask-test [미리 채울 이름]` : 이름 입력 창만 띄워 보고 답을 적는다 (게임 창이 있으면 그 가운데)
    if args.get(1).map(String::as_str) == Some("--ask-test") {
        let g = capture::find_game_windows().first().copied().unwrap_or(0);
        let h = ask::open(g, args.get(2).cloned().unwrap_or_default(), "화면에서 읽은 이름을 채워 두었습니다. 맞으면 Enter 를 누르세요.".into());
        while h.lock().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("답: {:?}", h.lock().unwrap());
        return;
    }
    if args.get(1).map(String::as_str) == Some("--hud-test") {
        hud_test();
        return;
    }
    if args.get(1).map(String::as_str) == Some("--file") {
        let _ = LOG_PATH.set(None);
        file_test(&args[2..]);
        return;
    }
    let data = std::env::var_os("DHO_SIDECAR_DATA_DIR").map(PathBuf::from);
    let _ = LOG_PATH.set(data.as_ref().map(|d| d.join("side-log.txt")));
    capture::set_log(log);
    netaddr::set_log(log);

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let next = |lines: &mut dyn Iterator<Item = io::Result<String>>| -> Option<Value> {
        for l in lines {
            let l = l.ok()?;
            let l = l.trim_end_matches('\r');
            if !l.is_empty() {
                if let Ok(v) = serde_json::from_str::<Value>(l) {
                    return Some(v);
                }
            }
        }
        None
    };
    let Some(hello) = next(&mut lines) else { return };
    if hello["t"] != "hello" {
        std::process::exit(2);
    }
    log(&format!("[시작] 시세 sidecar {} · Navigator {}", env!("CARGO_PKG_VERSION"), hello["navigatorVersion"].as_str().unwrap_or("?")));
    set_name(&hello["input"]["character"]);
    set_hud_off(&hello["input"]["hudOff"]);
    hud::set_pos(pos_of(&hello["input"]["hudPos"]), false);
    write_line(&json!({ "t": "ready" }));
    emit(None, json!({ "kind": "ready", "version": env!("CARGO_PKG_VERSION") }));

    let target: Arc<Mutex<Option<Target>>> = Arc::new(Mutex::new(None));
    let stop = Arc::new(AtomicBool::new(false));
    let w = {
        let (target, stop, data) = (target.clone(), stop.clone(), data.clone());
        std::thread::spawn(move || worker(target, data, stop))
    };
    while let Some(line) = next(&mut lines) {
        match line["t"].as_str() {
            Some("target") => {
                let tg = parse_target(&line);
                let mut g = target.lock().unwrap();
                if tg.as_ref().map(|t| (t.hwnd, t.pid, t.generation)) != g.as_ref().map(|t| (t.hwnd, t.pid, t.generation)) {
                    let msg = match &tg {
                        Some(t) => format!("[대상] {} 세대 {} · pid {}", t.role, t.generation, if t.pid != 0 { "있음" } else { "없음" }),
                        None => "[대상] 없음".into(),
                    };
                    log(&msg);
                }
                *g = tg;
            }
            Some("message") => {
                if line["data"]["kind"] == "character" {
                    set_name(&line["data"]["character"]);
                } else if line["data"]["kind"] == "hud" {
                    set_hud_off(&line["data"]["off"]);
                } else if line["data"]["kind"] == "hudpos" {
                    hud::set_pos(pos_of(&line["data"]), true);
                } else if line["data"]["kind"] == "update" {
                    *UPDATE_V.lock().unwrap() = line["data"]["v"].as_str().map(String::from);
                } else if line["data"]["kind"] == "open-site" {
                    open_site(line["data"]["page"] == "install");
                }
            }
            Some("stop") => {
                log(&format!("[종료] {}", line["reason"].as_str().unwrap_or("?")));
                break;
            }
            _ => {}
        }
        if GONE.load(Ordering::SeqCst) {
            break;
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = w.join();
}
