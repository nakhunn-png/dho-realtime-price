//! 게임 창 찾기 · 창 위치 · 디스플레이 배율 · 관리자 권한 (Windows API)
use std::ffi::c_void;
use windows::Win32::Foundation::{BOOL, CloseHandle, HANDLE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{ClientToScreen, MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::IsUserAnAdmin;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetForegroundWindow, GetWindowThreadProcessId, GetWindowRect, GetWindowTextW, IsIconic, IsWindow, IsWindowVisible,
    SetProcessDPIAware,
};

pub const GAME_TITLE: &str = "대항해시대 온라인";

/// 창 번호 (Python 판과 같이 정수로 다룬다)
pub type Hwnd = isize;

pub fn hwnd(h: Hwnd) -> HWND {
    HWND(h as *mut c_void)
}

/// 화면 좌표를 실제 픽셀로 받도록 (디스플레이 배율 PC). 프로그램 시작 때 한 번
pub fn set_dpi_aware() {
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
            let _ = SetProcessDPIAware();
        }
    }
}

pub fn window_title(h: Hwnd) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd(h), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// 보이는 최상위 창 전부
fn visible_windows() -> Vec<Hwnd> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let v = unsafe { &mut *(l.0 as *mut Vec<Hwnd>) };
        if unsafe { IsWindowVisible(h) }.as_bool() {
            v.push(h.0 as Hwnd);
        }
        BOOL(1)
    }
    let mut v: Vec<Hwnd> = vec![];
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// 제목이 정확히 `title` 인 보이는 창들
pub fn find_windows(title: &str) -> Vec<Hwnd> {
    visible_windows().into_iter().filter(|&h| window_title(h) == title).collect()
}

pub fn find_game_windows() -> Vec<Hwnd> {
    find_windows(GAME_TITLE)
}

/// 게임 창을 못 찾을 때 로그에 남길, 제목이 비슷한 창들 (제목이 바뀌었는지 보려고)
pub fn similar_windows(limit: usize) -> Vec<String> {
    visible_windows()
        .into_iter()
        .map(window_title)
        .filter(|t| !t.is_empty() && (t.contains("대항") || t.to_uppercase().contains("GV") || t.contains("온라인")))
        .take(limit)
        .collect()
}

/// 창 바깥 테두리(그림자 제외)의 화면 좌표 (left, top, right, bottom)
pub fn frame_rect(h: Hwnd) -> (i32, i32, i32, i32) {
    let mut r = RECT::default();
    unsafe {
        if DwmGetWindowAttribute(hwnd(h), DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() {
            let _ = GetWindowRect(hwnd(h), &mut r);
        }
    }
    (r.left, r.top, r.right, r.bottom)
}

/// 게임 창 안쪽(제목 줄·테두리 제외)의 화면 좌표 (실제 픽셀)
pub fn client_rect_screen(h: Hwnd) -> (i32, i32, i32, i32) {
    let mut r = RECT::default();
    let (mut p, mut q) = (POINT::default(), POINT::default());
    unsafe {
        let _ = GetClientRect(hwnd(h), &mut r);
        q.x = r.right;
        q.y = r.bottom;
        let _ = ClientToScreen(hwnd(h), &mut p);
        let _ = ClientToScreen(hwnd(h), &mut q);
    }
    (p.x, p.y, q.x.max(p.x + r.right), q.y.max(p.y + r.bottom))
}

/// Windows 디스플레이 배율 때문에 게임 창이 늘어나 보이는 배율 (100%면 1.0, 150%면 1.5) = 모니터 DPI / 게임 창 DPI
pub fn dpi_scale(h: Hwnd) -> f64 {
    unsafe {
        let wdpi = GetDpiForWindow(hwnd(h));
        let mon = MonitorFromWindow(hwnd(h), MONITOR_DEFAULTTONEAREST);
        let (mut x, mut y) = (0u32, 0u32);
        if wdpi == 0 || GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_err() {
            return 1.0;
        }
        let s = x as f64 / wdpi as f64;
        if s > 1.05 { s } else { 1.0 }
    }
}

pub fn is_minimized(h: Hwnd) -> bool {
    unsafe { IsIconic(hwnd(h)).as_bool() }
}

pub fn is_window(h: Hwnd) -> bool {
    unsafe { IsWindow(hwnd(h)).as_bool() }
}

/// 창을 만든 프로세스 번호 (모르면 0)
pub fn window_pid(h: Hwnd) -> u32 {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd(h), Some(&mut pid));
    }
    pid
}

/// 창 안쪽 크기 (너비, 높이)
pub fn client_size(h: Hwnd) -> (i64, i64) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd(h), &mut r);
    }
    ((r.right - r.left) as i64, (r.bottom - r.top) as i64)
}

pub fn foreground() -> Hwnd {
    unsafe { GetForegroundWindow().0 as Hwnd }
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

/// 그 프로세스가 관리자 권한으로 켜졌는지. 알 수 없으면 None (게임 보안 프로그램이 막는 경우 등)
pub fn process_elevated(pid: u32) -> Option<bool> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut tok = HANDLE::default();
        let r = if OpenProcessToken(h, TOKEN_QUERY, &mut tok).is_ok() {
            let mut e = TOKEN_ELEVATION::default();
            let mut n = 0u32;
            let ok = GetTokenInformation(tok, TokenElevation, Some(&mut e as *mut _ as *mut c_void), size_of::<TOKEN_ELEVATION>() as u32, &mut n);
            let _ = CloseHandle(tok);
            ok.is_ok().then_some(e.TokenIsElevated != 0)
        } else {
            None
        };
        let _ = CloseHandle(h);
        r
    }
}
