//! Windows 연결 목록 · 프로세스 목록 읽기
use crate::log;
use std::collections::{BTreeSet, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM};
use windows::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_CONNECTIONS};
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
    Thread32First, Thread32Next,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetWindowThreadProcessId};

pub const GAME_EXE: &str = "gvonline.bin";
/// 게임이 같이 열어 두는 웹 연결(https 443 등)은 게임 서버가 아니다: 빼고 본다
const WEB_PORTS: [u16; 2] = [80, 443];
const MIB_TCP_STATE_ESTAB: u32 = 5;

fn fail(why: &str) {
    static FAILED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    if FAILED.lock().unwrap().get_or_insert_with(HashSet::new).insert(why.into()) {
        log(&format!("[접속 주소] 연결 목록을 읽지 못함: {why}"));
    }
}

fn debug() -> bool {
    std::env::var_os("DHO_DEBUG").is_some()
}

/// 창에 직접 물은 프로세스 번호 (게임 보안 프로그램이 숨기면 0)
pub fn window_pid_direct(hwnd: isize) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(HWND(hwnd as _), Some(&mut pid)) };
    pid
}

/// 게임 창의 프로세스 번호. 창에 직접 물어서 못 얻으면 게임 프로세스의 창 목록에서 찾는다
pub fn window_pid(hwnd: isize) -> u32 {
    match window_pid_direct(hwnd) {
        0 => thread_window_pid(hwnd),
        p => p,
    }
}

/// 게임 프로세스들의 스레드가 가진 창 목록에서 이 창을 찾아 그 프로세스 번호를 돌려준다 (못 찾으면 0)
pub fn thread_window_pid(hwnd: isize) -> u32 {
    let pids = game_pids(Duration::from_secs(10));
    if pids.is_empty() {
        return 0;
    }
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let st = unsafe { &mut *(l.0 as *mut (isize, bool)) };
        if h.0 as isize == st.0 {
            st.1 = true;
            return BOOL(0);
        }
        BOOL(1)
    }
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return 0;
        };
        let mut te = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
        let mut ok = Thread32First(snap, &mut te).is_ok();
        let mut found = 0;
        while ok && found == 0 {
            if pids.contains(&te.th32OwnerProcessID) {
                let mut st = (hwnd, false);
                let _ = EnumThreadWindows(te.th32ThreadID, Some(cb), LPARAM(&mut st as *mut _ as isize));
                if st.1 {
                    found = te.th32OwnerProcessID;
                }
            }
            ok = Thread32Next(snap, &mut te).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

/// 켜져 있는 게임 프로세스 번호들. `max_age` 동안은 찾은 것을 다시 쓴다 (0이면 바로 다시 찾음)
pub fn game_pids(max_age: Duration) -> BTreeSet<u32> {
    static CACHE: Mutex<Option<(Instant, BTreeSet<u32>)>> = Mutex::new(None);
    let mut g = CACHE.lock().unwrap();
    if let Some((t, p)) = g.as_ref() {
        if t.elapsed() < max_age {
            return p.clone();
        }
    }
    let mut out = BTreeSet::new();
    unsafe {
        match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(snap) => {
                let mut pe = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
                let mut ok = Process32FirstW(snap, &mut pe).is_ok();
                while ok {
                    let n = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
                    if String::from_utf16_lossy(&pe.szExeFile[..n]).to_lowercase() == GAME_EXE {
                        out.insert(pe.th32ProcessID);
                    }
                    ok = Process32NextW(snap, &mut pe).is_ok();
                }
                let _ = CloseHandle(snap);
            }
            Err(e) => fail(&format!("프로세스 목록 오류 {}", e.message())),
        }
    }
    *g = Some((Instant::now(), out.clone()));
    out
}

/// 이 PC의 IPv4 TCP 연결 중 연결된 것 [(상대 주소 "1.2.3.4:9067", 프로세스 번호)]. 못 읽으면 None
fn tcp_rows() -> Option<Vec<(String, u32)>> {
    let mut size = 0u32;
    unsafe {
        let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_CONNECTIONS, 0);
    }
    for _ in 0..3 {
        let mut buf = vec![0u8; size as usize + 4096];
        size = buf.len() as u32;
        let r = unsafe {
            GetExtendedTcpTable(Some(buf.as_mut_ptr() as _), &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_CONNECTIONS, 0)
        };
        if r == 0 {
            let n = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
            let rows = unsafe { std::slice::from_raw_parts(buf.as_ptr().add(4) as *const MIB_TCPROW_OWNER_PID, n) };
            return Some(
                rows.iter()
                    .filter(|r| r.dwState == MIB_TCP_STATE_ESTAB)
                    .map(|r| {
                        let ip = r.dwRemoteAddr.to_le_bytes();
                        let port = u16::from_be((r.dwRemotePort & 0xFFFF) as u16);
                        (format!("{}.{}.{}.{}:{port}", ip[0], ip[1], ip[2], ip[3]), r.dwOwningPid)
                    })
                    .collect(),
            );
        }
    }
    fail("목록 크기가 계속 바뀜");
    None
}

/// 그 프로세스가 지금 연결해 있는 상대 주소들 (이 PC 안쪽 연결·웹 연결은 뺌)
pub fn remote_addrs(pid: u32) -> BTreeSet<String> {
    if pid == 0 {
        if debug() {
            log("[접속 주소 점검] 게임 창의 프로세스 번호를 얻지 못함");
        }
        return BTreeSet::new();
    }
    let Some(rows) = tcp_rows() else { return BTreeSet::new() };
    let n = rows.len();
    let out: BTreeSet<String> = rows
        .into_iter()
        .filter(|(a, p)| {
            let (ip, port) = a.rsplit_once(':').unwrap();
            *p == pid && !ip.starts_with("127.") && ip != "0.0.0.0" && !WEB_PORTS.contains(&port.parse().unwrap_or(0))
        })
        .map(|(a, _)| a)
        .collect();
    if debug() {
        log(&format!("[접속 주소 점검] pid={pid} 연결 {n}개, 이 프로세스 {out:?}"));
    }
    out
}

/// 이 PC의 모든 연결 (점검용)
pub fn all_rows() -> Vec<(String, u32)> {
    tcp_rows().unwrap_or_default()
}
