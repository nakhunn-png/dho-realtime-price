//! 캐릭터 이름 입력 창 (0.5.4, 2026-11-22 사용자 결정 "입력 창 넣기").
//!
//! 적힌 이름(모듈 설정·캐릭터이름.txt)이 없거나 화면 이름과 다를 때 한 번 띄운다.
//! 화면에서 읽은 이름을 미리 채워 두어 맞으면 Enter 만 누르면 된다. 고른 이름은 main.rs 가 캐릭터이름.txt 에 적는다.
//! 적은 이름도 지금처럼 화면 글자 모양과 맞춰 본 뒤에만 등록자로 쓴다 (미리 채운 이름이 잘못 읽힌 것이어도 그대로 올라가지 않게).
//!
//! - 확인(Enter): 이름 저장 · 나중에(Esc, 닫기): 이번 실행 동안 다시 묻지 않는다
//! - 게임 창 가운데에 맨 위로 뜬다. 게임에는 아무것도 하지 않는다
//! - 모양은 Navigator 창·HUD 와 같게 (0.5.6, 2026-11-22 사용자 "이쁘게, 이질감 없게"): 남색 바탕, 따뜻한 흰 글씨, 금색 테두리
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::ApplicationInstallationAndServicing::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Name(String),
    Later,
}

/// 창이 닫히면 Some
pub type Handle = Arc<Mutex<Option<Answer>>>;

const ID_EDIT: i32 = 100;
const ID_ERR: i32 = 101;
const ID_LABEL: i32 = 102;
const ID_TITLE: i32 = 103;
// windows 0.58 에 없는 상수 (Win32 헤더 값 그대로)
const EM_SETSEL: u32 = 0x00B1;
const EM_LIMITTEXT: u32 = 0x00C5;
const ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID: u32 = 0x4;
const ACTCTX_FLAG_RESOURCE_NAME_VALID: u32 = 0x8;
const BS_OWNERDRAW_: u32 = 0xB;
const EN_SETFOCUS_: usize = 0x100;
const EN_KILLFOCUS_: usize = 0x200;
// DWM 창 틀 색 (Windows 11). 안 되는 Windows 에서는 그냥 무시된다
const DWMWA_DARK: i32 = 20;
const DWMWA_BORDER: i32 = 34;
const DWMWA_CAPTION: i32 = 35;
const DWMWA_TEXT: i32 = 36;

// 색은 Navigator 창과 HUD(hud.rs)에 맞춘다
const C_BG: u32 = 0x10202d;
const C_FIELD: u32 = 0x0b1823;
const C_LINE: u32 = 0x34485a;
const C_GOLD: u32 = 0xc9973a;
const C_GOLD_TEXT: u32 = 0xe8b54a;
const C_LABEL: u32 = 0xb89a5e;
const C_FG: u32 = 0xfff4dc;
const C_MUTED: u32 = 0x8fa1b2;
const C_BTN: u32 = 0xc3ccd5;
const C_RED: u32 = 0xff6b5e;

/// 창 하나만 뜨므로 창 함수와는 전역으로 주고받는다
static CUR: Mutex<Option<Handle>> = Mutex::new(None);
/// 안내 줄을 빨갛게 (잘못 적었을 때)
static ERR: AtomicBool = AtomicBool::new(false);
/// 입력 칸에 커서가 있으면 테두리를 금색으로
static FOCUS: AtomicBool = AtomicBool::new(false);
/// 창이 사는 동안 쓰는 붓. 스레드를 넘기려고 isize 로 둔다
static BRUSH_BG: AtomicIsize = AtomicIsize::new(0);
static BRUSH_FIELD: AtomicIsize = AtomicIsize::new(0);
/// 입력 칸 둘레 상자 (창 안 좌표)
static FRAME: Mutex<RECT> = Mutex::new(RECT { left: 0, top: 0, right: 0, bottom: 0 });

fn rgb(c: u32) -> COLORREF {
    COLORREF(((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff))
}

fn brush(a: &AtomicIsize) -> HBRUSH {
    HBRUSH(a.load(SeqCst) as *mut _)
}

/// 창을 띄우고 바로 돌아온다. `why` = 둘째 줄 안내
pub fn open(game: isize, prefill: String, why: String) -> Handle {
    let h: Handle = Arc::new(Mutex::new(None));
    let h2 = h.clone();
    std::thread::spawn(move || {
        run(game, &prefill, &why, h2.clone());
        // 어떤 길로 닫혀도 답은 남긴다
        h2.lock().unwrap().get_or_insert(Answer::Later);
    });
    h
}

fn answer(a: Answer) {
    if let Some(h) = CUR.lock().unwrap().as_ref() {
        h.lock().unwrap().get_or_insert(a);
    }
}

/// 요즘 Windows 모양의 단추·입력 칸 (exe 에 manifest 가 없어서 shell32 의 것을 빌린다). 안 되면 옛 모양 그대로
fn visual_styles() {
    unsafe {
        let mut dir = [0u16; 260];
        let n = windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut dir)) as usize;
        let dir = String::from_utf16_lossy(&dir[..n]);
        let src = HSTRING::from(format!("{dir}\\shell32.dll"));
        let d = HSTRING::from(dir);
        let ctx = ACTCTXW {
            cbSize: size_of::<ACTCTXW>() as u32,
            dwFlags: ACTCTX_FLAG_RESOURCE_NAME_VALID | ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID,
            lpSource: PCWSTR(src.as_ptr()),
            lpAssemblyDirectory: PCWSTR(d.as_ptr()),
            lpResourceName: PCWSTR(124 as *const u16),
            ..Default::default()
        };
        if let Ok(a) = CreateActCtxW(&ctx) {
            let mut cookie = 0usize;
            let _ = ActivateActCtx(a, &mut cookie);
        }
    }
}

fn run(game: isize, prefill: &str, why: &str, h: Handle) {
    *CUR.lock().unwrap() = Some(h);
    ERR.store(false, SeqCst);
    FOCUS.store(false, SeqCst);
    visual_styles();
    unsafe {
        let bg = CreateSolidBrush(rgb(C_BG));
        let field = CreateSolidBrush(rgb(C_FIELD));
        BRUSH_BG.store(bg.0 as isize, SeqCst);
        BRUSH_FIELD.store(field.0 as isize, SeqCst);
        let inst = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc_),
            hInstance: inst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: bg,
            lpszClassName: w!("PriceSideAsk"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let g = capture::hwnd(game);
        let alive = game != 0 && capture::is_window(game);
        let dpi = if alive { GetDpiForWindow(g) } else { GetDpiForSystem() }.max(96) as i32;
        let s = |v: i32| v * dpi / 96;
        let (cw, ch) = (s(420), s(220));
        // 바깥 틀 크기
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let mut rc = RECT { left: 0, top: 0, right: cw, bottom: ch };
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(&mut rc, style, false, WS_EX_TOPMOST | WS_EX_APPWINDOW | WS_EX_DLGMODALFRAME, dpi as u32);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        let (x, y) = if alive {
            let (l, t, r, b) = capture::client_rect_screen(game);
            (l + (r - l - ww) / 2, t + (b - t - wh) / 2)
        } else {
            ((GetSystemMetrics(SM_CXSCREEN) - ww) / 2, (GetSystemMetrics(SM_CYSCREEN) - wh) / 2)
        };
        let Ok(wnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_APPWINDOW | WS_EX_DLGMODALFRAME,
            w!("PriceSideAsk"),
            w!("시세 모듈 · 캐릭터 이름"),
            style,
            x,
            y,
            ww,
            wh,
            None,
            None,
            inst,
            None,
        ) else {
            return;
        };
        // 제목 줄도 Navigator 처럼: 남색 바탕, 따뜻한 흰 글씨, 금색 테두리
        for (attr, v) in [(DWMWA_DARK, 1u32), (DWMWA_CAPTION, rgb(C_BG).0), (DWMWA_TEXT, rgb(C_FG).0), (DWMWA_BORDER, rgb(C_GOLD).0)] {
            let _ = DwmSetWindowAttribute(wnd, DWMWINDOWATTRIBUTE(attr), &v as *const u32 as *const _, 4);
        }
        let mk = |px: i32, weight: i32| CreateFontW(-s(px), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET.0 as u32, 0, 0, CLEARTYPE_QUALITY.0 as u32, 0, w!("맑은 고딕"));
        let (f_label, f_title, f_body, f_edit, f_btn) = (mk(11, 700), mk(15, 700), mk(12, 400), mk(15, 400), mk(13, 700));
        let child = |class: PCWSTR, text: &str, st: u32, x: i32, y: i32, w: i32, h: i32, id: i32, f: HFONT| {
            let c = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                &HSTRING::from(text),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(st),
                s(x),
                s(y),
                s(w),
                s(h),
                wnd,
                HMENU(id as isize as *mut _),
                inst,
                None,
            )
            .unwrap_or_default();
            SendMessageW(c, WM_SETFONT, WPARAM(f.0 as usize), LPARAM(1));
            c
        };
        child(w!("STATIC"), "시세 모듈  ·  등록자 이름", 0, 20, 16, 380, 16, ID_LABEL, f_label);
        child(w!("STATIC"), "시세를 올릴 때 보일 캐릭터 이름을 적어 주세요", 0, 20, 36, 380, 22, ID_TITLE, f_title);
        child(w!("STATIC"), why, 0, 20, 62, 380, 34, -1, f_body);
        // 입력 칸: 테두리 없는 EDIT 를 직접 그린 상자(FRAME) 안에 넣어 글자가 가운데 오게
        *FRAME.lock().unwrap() = RECT { left: s(20), top: s(100), right: s(400), bottom: s(138) };
        let edit = child(w!("EDIT"), prefill, WS_TABSTOP.0 | ES_AUTOHSCROLL as u32, 32, 109, 356, 22, ID_EDIT, f_edit);
        SendMessageW(edit, EM_LIMITTEXT, WPARAM(20), LPARAM(0));
        child(w!("STATIC"), "20자까지 · . $ # [ ] / 는 쓸 수 없습니다", 0, 20, 144, 380, 18, ID_ERR, f_body);
        child(w!("BUTTON"), "확인", WS_TABSTOP.0 | BS_OWNERDRAW_, 222, 176, 86, 30, IDOK.0, f_btn);
        child(w!("BUTTON"), "나중에", WS_TABSTOP.0 | BS_OWNERDRAW_, 314, 176, 86, 30, IDCANCEL.0, f_btn);
        // 게임에서 키를 누르던 중일 수 있어 포커스를 빼앗지 않는다 (Windows 도 뒤에서 뜬 창이 포커스를 가져가지 못하게 막는다).
        // 사용자가 창을 누르면 그때 입력 칸으로 간다 (WM_ACTIVATE). 작업 표시줄 단추는 깜빡여 알린다
        let _ = ShowWindow(wnd, SW_SHOWNA);
        let _ = FlashWindowEx(&FLASHWINFO {
            cbSize: size_of::<FLASHWINFO>() as u32,
            hwnd: wnd,
            dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
            uCount: 0,
            dwTimeout: 0,
        });
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab·Enter(확인)·Esc(나중에)
            if IsDialogMessageW(wnd, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for f in [f_label, f_title, f_body, f_edit, f_btn] {
            let _ = DeleteObject(f);
        }
        let _ = UnregisterClassW(w!("PriceSideAsk"), inst);
        let _ = DeleteObject(bg);
        let _ = DeleteObject(field);
    }
}

/// 입력 칸 상자: 어두운 바탕 + 1px 테두리 (커서가 있으면 금색)
unsafe fn paint(wnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(wnd, &mut ps);
        let r = *FRAME.lock().unwrap();
        FillRect(dc, &r, brush(&BRUSH_FIELD));
        let edge = CreateSolidBrush(rgb(if FOCUS.load(SeqCst) { C_GOLD } else { C_LINE }));
        FrameRect(dc, &r, edge);
        let _ = DeleteObject(edge);
        let _ = EndPaint(wnd, &ps);
    }
}

/// 확인(금색 테두리·금색 글씨, Navigator '모듈 설치' 단추)·나중에(회색 테두리, 골랐을 때 금색) 단추
unsafe fn draw_button(d: &DRAWITEMSTRUCT) {
    unsafe {
        let ok = d.CtlID as i32 == IDOK.0;
        let pressed = d.itemState.0 & ODS_SELECTED.0 != 0;
        let focused = d.itemState.0 & ODS_FOCUS.0 != 0;
        let r = d.rcItem;
        let fill = CreateSolidBrush(rgb(if pressed { 0x1c3242 } else if ok { 0x16262f } else { C_BG }));
        FillRect(d.hDC, &r, fill);
        let _ = DeleteObject(fill);
        let edge = CreateSolidBrush(rgb(if ok || focused { C_GOLD } else { C_LINE }));
        FrameRect(d.hDC, &r, edge);
        let _ = DeleteObject(edge);
        let mut buf = [0u16; 16];
        let n = GetWindowTextW(d.hwndItem, &mut buf) as usize;
        SetBkMode(d.hDC, TRANSPARENT);
        SetTextColor(d.hDC, rgb(if ok { C_GOLD_TEXT } else { C_BTN }));
        let mut tr = r;
        DrawTextW(d.hDC, &mut buf[..n], &mut tr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    }
}

/// 확인을 눌렀을 때: 쓸 수 있는 이름이면 답하고 닫는다
unsafe fn submit(wnd: HWND) {
    unsafe {
        let edit = GetDlgItem(wnd, ID_EDIT).unwrap_or_default();
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(edit, &mut buf) as usize;
        let name = String::from_utf16_lossy(&buf[..n]).trim().to_string();
        let bad = if name.is_empty() {
            Some("이름을 적어 주세요")
        } else if !crate::upload::by_ok(&name) {
            Some(". $ # [ ] / 는 쓸 수 없습니다 · 20자까지")
        } else {
            None
        };
        match bad {
            Some(m) => {
                ERR.store(true, SeqCst);
                let _ = SetDlgItemTextW(wnd, ID_ERR, &HSTRING::from(m));
                let _ = SetFocus(edit);
            }
            None => {
                answer(Answer::Name(name));
                let _ = DestroyWindow(wnd);
            }
        }
    }
}

unsafe extern "system" fn proc_(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let code = (wp.0 >> 16) & 0xffff;
                if (wp.0 & 0xffff) as i32 == ID_EDIT && (code == EN_SETFOCUS_ || code == EN_KILLFOCUS_) {
                    FOCUS.store(code == EN_SETFOCUS_, SeqCst);
                    let r = *FRAME.lock().unwrap();
                    let _ = InvalidateRect(h, Some(&r), TRUE);
                    return LRESULT(0);
                }
                match (wp.0 & 0xffff) as i32 {
                    x if x == IDOK.0 => submit(h),
                    x if x == IDCANCEL.0 => {
                        answer(Answer::Later);
                        let _ = DestroyWindow(h);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as *mut _);
                let id = GetDlgCtrlID(HWND(lp.0 as *mut _));
                let c = match id {
                    ID_ERR if ERR.load(SeqCst) => C_RED,
                    ID_LABEL => C_LABEL,
                    ID_TITLE => C_FG,
                    _ => C_MUTED,
                };
                SetTextColor(dc, rgb(c));
                SetBkMode(dc, TRANSPARENT);
                LRESULT(brush(&BRUSH_BG).0 as isize)
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as *mut _);
                SetTextColor(dc, rgb(C_FG));
                SetBkColor(dc, rgb(C_FIELD));
                LRESULT(brush(&BRUSH_FIELD).0 as isize)
            }
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                LRESULT(1)
            }
            WM_PAINT => {
                paint(h);
                LRESULT(0)
            }
            // 눌러서 앞으로 오면 입력 칸에 글자를 모두 고른 채로 (바로 고쳐 쓰거나 Enter)
            WM_ACTIVATE if (wp.0 & 0xffff) != 0 => {
                if let Ok(edit) = GetDlgItem(h, ID_EDIT) {
                    let _ = SetFocus(edit);
                    SendMessageW(edit, EM_SETSEL, WPARAM(0), LPARAM(-1));
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                answer(Answer::Later);
                let _ = DestroyWindow(h);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}
