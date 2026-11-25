//! 시세 sidecar 가 직접 띄우는 게임 화면 HUD (0.4.0, 2026-11-22 사용자 결정 "직접 띄워").
//!
//! Navigator 모듈 HUD 는 측량이 없으면(도시) 숨고, service ↔ HUD 메시지도 오가지 않아서
//! 감지기 HUD(gamji/src/ui.rs)처럼 sidecar 가 클릭이 통과하는 창을 게임 창 위쪽 가운데에 직접 그린다.
//!
//! - 위 줄(상태): 늘 보인다. `헬레네 · 리스본` / `서버 확인 중` / `서버 모름`
//! - 아래 줄(알림): 정한 시간만 보인다. 4초 이상 알림이 떠 있으면 다음 것은 기다리고, 같은 key 는 바로 바꾼다(대기 최대 3개)
//! - 게임 창이 맨 앞이고 최소화되지 않았을 때만 보인다 (다른 프로그램 위를 가리지 않게)
//! - 게임에는 아무것도 하지 않는다: 우리 창을 게임 창 위에 겹쳐 그릴 뿐이다
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

// 색은 Navigator 창에 맞춘다 (2026-11-22 사용자 "네비랑 비슷하게"): 짙은 남색 바탕, 따뜻한 흰 글씨, 금색 테두리
pub const FG: u32 = 0xfff4dc;
pub const MUTED: u32 = 0x8fa1b2;
pub const WARN: u32 = 0xe8b54a;
const BG: u32 = 0x10202d;
/// 위 줄(상태) 테두리: 흐린 금색, 아래 줄(알림) 테두리: Navigator 버튼 금색
const EDGE: u32 = 0x5c4a2a;
const EDGE_HI: u32 = 0xc9973a;
/// 띠 바깥(투명하게 뺄 색). 글자·바탕에 쓰지 않는 색
const KEY: u32 = 0xff00ff;
/// 감지기는 55% 인데 이쪽은 서버가 잘 보이게 85% (2026-11-17 결정)
const ALPHA: f64 = 0.85;
const TOP: i32 = 36;
const PADX: i32 = 12;
const PADY: i32 = 4;
const GAP: i32 = 4;

pub fn server_color(s: &str) -> u32 {
    match s {
        "헬레네" => 0xff9a2e,
        "폴라리스" => 0x4da6ff,
        "에이레네" => 0xd08cff,
        _ => FG,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub text: String,
    pub color: u32,
    pub bold: bool,
}

pub fn p(text: impl Into<String>, color: u32) -> Part {
    Part { text: text.into(), color, bold: false }
}

pub fn pb(text: impl Into<String>, color: u32) -> Part {
    Part { text: text.into(), color, bold: true }
}

#[derive(Clone, Debug)]
struct Line {
    key: &'static str,
    parts: Vec<Part>,
    ms: u64,
}

#[derive(Default)]
struct State {
    game: Option<isize>,
    /// 점검용: 게임 창이 맨 앞이 아니어도 보인다
    force: bool,
    status: Vec<Part>,
    cur: Option<(Line, f64)>,
    queue: VecDeque<Line>,
    /// 남만 틱 HUD (0.10.0): (캡처 x, 캡처 y = 마지막 점 높이, 틱, 마지막으로 읽은 시각)
    tick: Option<(i64, i64, u8, f64)>,
    /// 1.0.2: 바다에 있음 — 위 줄·알림 모두 숨기고, 그동안 온 알림은 버린다 (항해 중 HUD 가 거슬린다는 말이 많음)
    sea: bool,
    /// 1.0.4: 패널 'HUD 끄기' — 위 줄·알림만 숨긴다. 남만 틱 HUD 는 그대로 보인다
    off: bool,
    /// 1.0.4: 게임 창에 다시 붙은 뒤 바다인지 아직 모름 — 알림은 쌓아 두고 그리지 않는다
    /// (해역을 넘을 때 Navigator 가 대상을 다시 줘서, 그 틈에 바다에서 알림이 새던 것)
    hold: bool,
}

// ---------- 남만 틱 HUD (0.10.0, 감지기 ui.rs `tick_hud` 와 같은 모양·자리) ----------
// 남만무역정보 창 오른쪽 바깥, 마지막 점(지금 값) 높이에 '재고 N 틱'. 남색 바탕 + 금색 테두리 2px, 숫자 금색 크게.
// 1초 안에 읽었고 게임 창이 맨 앞일 때만 (창을 닫으면 바로 사라짐)
const TICK_GOLD: u32 = 0xf2c14e;
const TICK_MUTED: u32 = 0x9fb3cc;
const TICK_ALPHA: f64 = 0.95;
/// 감지기와 같은 크기 (2026-11-19 사용자: 처음 크기의 75%)
const TICK_SCALE: f64 = 0.75;
const TICK_FRESH_SEC: f64 = 1.0;
static TICK_TEXT: Mutex<String> = Mutex::new(String::new());

#[derive(Clone)]
pub struct Hud(Arc<Mutex<State>>);

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl Hud {
    /// HUD 창을 따로 도는 스레드에서 만든다
    pub fn start(force: bool) -> Hud {
        let h = Hud(Arc::new(Mutex::new(State { force, ..Default::default() })));
        let st = h.0.clone();
        std::thread::spawn(move || run(st));
        h
    }

    pub fn set_game(&self, game: Option<isize>) {
        self.0.lock().unwrap().game = game;
    }

    pub fn set_status(&self, parts: Vec<Part>) {
        self.0.lock().unwrap().status = parts;
    }

    /// 남만 틱 HUD 자리 (캡처 좌표). 읽을 때마다 부른다 — 1초 넘게 안 부르면 사라진다
    pub fn set_tick(&self, tick: Option<(i64, i64, u8)>) {
        self.0.lock().unwrap().tick = tick.map(|(x, y, v)| (x, y, v, now()));
    }

    /// 바다로 나가면 떠 있던 알림까지 지운다
    pub fn set_sea(&self, sea: bool) {
        let mut s = self.0.lock().unwrap();
        if sea && !s.sea {
            s.cur = None;
            s.queue.clear();
        }
        s.sea = sea;
    }

    /// 1.0.4: 'HUD 끄기'. 켜는 순간 떠 있던 알림도 지운다
    pub fn set_off(&self, off: bool) {
        let mut s = self.0.lock().unwrap();
        if off && !s.off {
            s.cur = None;
            s.queue.clear();
        }
        s.off = off;
    }

    /// 1.0.4: 바다인지 알 때까지 위 줄·알림을 그리지 않는다
    pub fn set_hold(&self, hold: bool) {
        self.0.lock().unwrap().hold = hold;
    }

    pub fn say(&self, key: &'static str, parts: Vec<Part>, ms: u64) {
        let mut s = self.0.lock().unwrap();
        if s.sea || s.off {
            return;
        }
        let line = Line { key, parts, ms };
        let t = now();
        if let Some((c, _)) = &s.cur {
            if c.key == key {
                // 같은 종류는 기다리지 않고 지금 것을 새 내용으로 바꾼다
                s.cur = Some((line, t + ms as f64 / 1000.0));
                return;
            }
        }
        match s.queue.iter().position(|q| q.key == key) {
            Some(i) => s.queue[i] = line,
            None => s.queue.push_back(line),
        }
        while s.queue.len() > 3 {
            s.queue.pop_front();
        }
        // 지금 떠 있는 것이 짧은 알림(4초 미만)이면 바로 넘긴다
        if s.cur.as_ref().is_none_or(|(c, _)| c.ms < 4000) {
            let next = s.queue.pop_front().unwrap();
            let until = t + next.ms as f64 / 1000.0;
            s.cur = Some((next, until));
        }
    }
}

/// 창이 그릴 것 (창 프로시저가 읽는다)
#[derive(Clone, Default, PartialEq)]
struct View {
    rows: Vec<(Vec<Part>, bool)>, // (조각, 큰 글씨인가)
    dpi: u32,
}

static VIEW: Mutex<Option<View>> = Mutex::new(None);

fn rgb(c: u32) -> COLORREF {
    COLORREF(((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff))
}

fn font(px: i32, bold: bool) -> HFONT {
    unsafe {
        CreateFontW(
            -px,
            0,
            0,
            0,
            if bold { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            // 색 키 투명과 같이 쓰므로 가장자리가 분홍으로 번지지 않게 ClearType 대신 일반 안티앨리어싱
            ANTIALIASED_QUALITY.0 as u32,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Malgun Gothic"),
        )
    }
}

fn pt(size: f64, dpi: u32) -> i32 {
    (size * dpi as f64 / 72.0).round() as i32
}

struct Fonts {
    small: (HFONT, HFONT),
    big: (HFONT, HFONT),
}

impl Fonts {
    fn new(dpi: u32) -> Self {
        Fonts { small: (font(pt(8.0, dpi), false), font(pt(8.0, dpi), true)), big: (font(pt(9.0, dpi), false), font(pt(9.0, dpi), true)) }
    }
    fn get(&self, big: bool, bold: bool) -> HFONT {
        let f = if big { self.big } else { self.small };
        if bold { f.1 } else { f.0 }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.small.0, self.small.1, self.big.0, self.big.1] {
                let _ = DeleteObject(f);
            }
        }
    }
}

fn text_size(dc: HDC, f: HFONT, s: &str) -> (i32, i32) {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    unsafe {
        let old = SelectObject(dc, f);
        DrawTextW(dc, &mut v, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
    (r.right - r.left, r.bottom - r.top)
}

/// 줄마다 (너비, 높이) — 여백 포함
fn row_sizes(dc: HDC, fonts: &Fonts, v: &View) -> Vec<(i32, i32)> {
    v.rows
        .iter()
        .map(|(parts, big)| {
            let (mut w, mut h) = (0, 0);
            for pt in parts {
                let (a, b) = text_size(dc, fonts.get(*big, pt.bold), &pt.text);
                w += a;
                h = h.max(b);
            }
            (w + PADX * 2, h + PADY * 2)
        })
        .collect()
}

fn paint(dc: HDC, rc: RECT) {
    let Some(v) = VIEW.lock().unwrap().clone() else { return };
    let fonts = Fonts::new(v.dpi);
    unsafe {
        let key = CreateSolidBrush(rgb(KEY));
        FillRect(dc, &rc, key);
        let _ = DeleteObject(key);
        let bg = CreateSolidBrush(rgb(BG));
        SetBkMode(dc, TRANSPARENT);
        let sizes = row_sizes(dc, &fonts, &v);
        let width = rc.right - rc.left;
        let mut y = 0;
        for ((parts, big), (w, h)) in v.rows.iter().zip(sizes) {
            let x0 = (width - w) / 2;
            let band = RECT { left: x0, top: y, right: x0 + w, bottom: y + h };
            FillRect(dc, &band, bg);
            // Navigator 카드처럼 1px 각진 테두리 (알림 줄은 더 밝은 금색)
            let edge = CreateSolidBrush(rgb(if *big { EDGE_HI } else { EDGE }));
            FrameRect(dc, &band, edge);
            let _ = DeleteObject(edge);
            let mut x = x0 + PADX;
            for pt in parts {
                let f = fonts.get(*big, pt.bold);
                let (tw, _) = text_size(dc, f, &pt.text);
                let old = SelectObject(dc, f);
                SetTextColor(dc, rgb(pt.color));
                let mut s: Vec<u16> = pt.text.encode_utf16().collect();
                let mut r = RECT { left: x, top: y + PADY, right: x + tw + 2, bottom: y + h };
                DrawTextW(dc, &mut s, &mut r, DT_SINGLELINE | DT_NOPREFIX | DT_NOCLIP);
                SelectObject(dc, old);
                x += tw;
            }
            y += h + GAP;
        }
        let _ = DeleteObject(bg);
    }
}

unsafe extern "system" fn proc_(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(h, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(h, &mut rc);
                paint(dc, rc);
                let _ = EndPaint(h, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

/// 틱 HUD 글자 배치: '재고' · 숫자 · '틱' 을 아래 줄을 맞춰 나란히. draw 마다 (글자, 글꼴, 색, x, 아래 y), 창 크기를 돌려준다 (감지기 ui.rs 와 같음)
fn tick_layout(dc: HDC, dpi: u32, num: &str, mut draw: impl FnMut(&str, HFONT, u32, i32, i32)) -> (i32, i32) {
    let parts: [(&str, f64, u32); 3] = [("재고", 10.5 * TICK_SCALE, TICK_MUTED), (num, 24.0 * TICK_SCALE, TICK_GOLD), ("틱", 14.0 * TICK_SCALE, FG)];
    let fonts: Vec<HFONT> = parts.iter().map(|p| font(pt(p.1, dpi), true)).collect();
    let sizes: Vec<(i32, i32)> = parts.iter().zip(&fonts).map(|(p, f)| text_size(dc, *f, p.0)).collect();
    let (padx, pady, gap) = (pt(10.0 * TICK_SCALE, dpi), pt(3.0 * TICK_SCALE, dpi), pt(4.0 * TICK_SCALE, dpi));
    let hmax = sizes.iter().map(|s| s.1).max().unwrap_or(0);
    let mut x = padx;
    for (i, (p, f)) in parts.iter().zip(&fonts).enumerate() {
        // 작은 글자는 큰 숫자와 아래 줄을 맞춘다
        let drop = ((hmax - sizes[i].1) as f64 * 0.8) as i32;
        draw(p.0, *f, p.2, x, pady + drop + sizes[i].1);
        x += sizes[i].0 + if i + 1 < parts.len() { gap } else { 0 };
    }
    for f in fonts {
        unsafe {
            let _ = DeleteObject(f);
        }
    }
    (x + padx, hmax + pady * 2)
}

fn paint_tick(h: HWND, dc: HDC, rc: RECT) {
    unsafe {
        // 금색 테두리 2px + 남색 바탕
        let gold = CreateSolidBrush(rgb(EDGE_HI));
        FillRect(dc, &rc, gold);
        let _ = DeleteObject(gold);
        let dpi = GetDpiForWindow(h).max(96);
        let b = (dpi as i32 * 2 + 95) / 96;
        let inner = RECT { left: rc.left + b, top: rc.top + b, right: rc.right - b, bottom: rc.bottom - b };
        let br = CreateSolidBrush(rgb(BG));
        FillRect(dc, &inner, br);
        let _ = DeleteObject(br);
        // 빈 글자는 그리지 않는다 (감지기: 빈 글자로 DrawTextW 를 부르면 USER32 안에서 죽었음)
        let num = TICK_TEXT.lock().unwrap().clone();
        if num.is_empty() {
            return;
        }
        SetBkMode(dc, TRANSPARENT);
        tick_layout(dc, dpi, &num, |s, f, c, x, bottom| {
            if s.is_empty() {
                return;
            }
            let old = SelectObject(dc, f);
            SetTextColor(dc, rgb(c));
            let mut v: Vec<u16> = s.encode_utf16().collect();
            let mut r = RECT { left: x, top: 0, right: x + 400, bottom };
            DrawTextW(dc, &mut v, &mut r, DT_SINGLELINE | DT_BOTTOM | DT_NOPREFIX | DT_NOCLIP);
            SelectObject(dc, old);
        });
    }
}

unsafe extern "system" fn tick_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(h, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(h, &mut rc);
                paint_tick(h, dc, rc);
                let _ = EndPaint(h, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

/// 틱 HUD: 방금 읽었고 그 게임 창이 맨 앞일 때만, 창이 움직여도 따라간다
fn tick_window(st: &Arc<Mutex<State>>, wnd: HWND, shown: &mut Option<(String, (i32, i32, i32, i32))>) {
    let t = now();
    let (game, force, tick) = {
        let s = st.lock().unwrap();
        (s.game, s.force, s.tick)
    };
    let show = match (game, tick) {
        (Some(g), Some((x, y, v, at)))
            if t - at <= TICK_FRESH_SEC && capture::is_window(g) && !capture::is_minimized(g) && (force || capture::foreground() == g) =>
        {
            Some((g, x, y, v))
        }
        _ => None,
    };
    let Some((g, x, y, v)) = show else {
        if shown.take().is_some() {
            unsafe {
                let _ = ShowWindow(wnd, SW_HIDE);
            }
        }
        return;
    };
    let num = v.to_string();
    let dpi = unsafe { GetDpiForWindow(capture::hwnd(g)) }.max(96);
    let (w, h) = unsafe {
        let dc = GetDC(wnd);
        let sz = tick_layout(dc, dpi, &num, |_, _, _, _, _| {});
        ReleaseDC(wnd, dc);
        sz
    };
    let (fl, ft, _, _) = capture::frame_rect(g); // 캡처 화면 (0,0) = 바깥 테두리 왼쪽 위
    let rect = (fl + x as i32, ft + y as i32 - h / 2, w, h);
    if shown.as_ref().is_some_and(|(n, rc)| *n == num && *rc == rect) {
        unsafe {
            let _ = SetWindowPos(wnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        return;
    }
    *TICK_TEXT.lock().unwrap() = num.clone();
    unsafe {
        let _ = SetWindowPos(wnd, HWND_TOPMOST, rect.0, rect.1, rect.2, rect.3, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(wnd, None, TRUE);
    }
    *shown = Some((num, rect));
}

fn run(st: Arc<Mutex<State>>) {
    let tick_wnd = unsafe {
        let inst = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(tick_proc),
            hInstance: inst,
            lpszClassName: w!("PriceSideTick"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            w!("PriceSideTick"),
            w!(""),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            inst,
            None,
        )
        .ok()
    };
    if let Some(h) = tick_wnd {
        unsafe {
            let _ = SetLayeredWindowAttributes(h, COLORREF(0), (TICK_ALPHA * 255.0).round() as u8, LWA_ALPHA);
        }
    }
    let mut tick_shown: Option<(String, (i32, i32, i32, i32))> = None;
    let wnd = unsafe {
        let inst = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc_),
            hInstance: inst,
            lpszClassName: w!("PriceSideHud"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let h = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            w!("PriceSideHud"),
            w!(""),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            inst,
            None,
        );
        let Ok(h) = h else { return };
        let _ = SetLayeredWindowAttributes(h, rgb(KEY), (ALPHA * 255.0).round() as u8, LWA_ALPHA | LWA_COLORKEY);
        SetTimer(h, 1, 100, None);
        h
    };
    let mut shown: Option<(View, (i32, i32, i32, i32))> = None;
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd == wnd {
                tick(&st, wnd, &mut shown);
                if let Some(tw) = tick_wnd {
                    tick_window(&st, tw, &mut tick_shown);
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 0.1초마다: 알림 차례 넘기기 → 보일지 정하기 → 내용·자리가 바뀌었을 때만 다시 그리기
fn tick(st: &Arc<Mutex<State>>, wnd: HWND, shown: &mut Option<(View, (i32, i32, i32, i32))>) {
    let t = now();
    let (game, force, rows) = {
        let mut s = st.lock().unwrap();
        // 보류 중에는 다음 알림으로 넘기지 않는다
        if !s.hold && s.cur.as_ref().is_some_and(|(_, until)| t >= *until) {
            s.cur = s.queue.pop_front().map(|l| {
                let until = t + l.ms as f64 / 1000.0;
                (l, until)
            });
        }
        let mut rows = vec![];
        if s.sea || s.off || s.hold {
            // 바다·HUD 끄기·바다인지 모름: 아무것도 그리지 않는다 (rows 가 비면 창을 숨김)
        } else {
            if !s.status.is_empty() {
                rows.push((s.status.clone(), false));
            }
            if let Some((l, _)) = &s.cur {
                rows.push((l.parts.clone(), true));
            }
        }
        (s.game, s.force, rows)
    };
    let visible = game.is_some_and(|g| {
        capture::is_window(g) && !capture::is_minimized(g) && (force || capture::foreground() == g) && !rows.is_empty()
    });
    let Some(g) = game.filter(|_| visible) else {
        if shown.take().is_some() {
            unsafe {
                let _ = ShowWindow(wnd, SW_HIDE);
            }
        }
        return;
    };
    let dpi = unsafe { GetDpiForWindow(capture::hwnd(g)) }.max(96);
    let view = View { rows, dpi };
    let (l, top, r, _) = capture::client_rect_screen(g);
    let fonts = Fonts::new(dpi);
    let (w, h) = unsafe {
        let dc = GetDC(wnd);
        let sizes = row_sizes(dc, &fonts, &view);
        ReleaseDC(wnd, dc);
        let w = sizes.iter().map(|s| s.0).max().unwrap_or(0);
        let h: i32 = sizes.iter().map(|s| s.1).sum::<i32>() + GAP * (sizes.len() as i32 - 1).max(0);
        (w, h)
    };
    let off = (TOP as f64 * dpi as f64 / 96.0).round() as i32;
    let rect = (l + (r - l - w) / 2, top + off, w, h);
    if shown.as_ref().is_some_and(|(v, rc)| *v == view && *rc == rect) {
        // 같은 내용이어도 맨 위는 지킨다 (게임 창을 눌렀을 때 밑으로 가지 않게)
        unsafe {
            let _ = SetWindowPos(wnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        return;
    }
    *VIEW.lock().unwrap() = Some(view.clone());
    unsafe {
        let _ = SetWindowPos(wnd, HWND_TOPMOST, rect.0, rect.1, rect.2, rect.3, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(wnd, None, TRUE);
    }
    *shown = Some((view, rect));
}
