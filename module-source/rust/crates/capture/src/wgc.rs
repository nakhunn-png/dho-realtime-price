//! 창 하나를 복사하고 가장 최근 화면만 들고 있는다 (Python `WindowCapture`).
//!
//! - 빠른 간격(교역소 창이 열렸을 때, 1초 미만): 복사를 계속 연결해 두고 화면을 받는다.
//! - 느린 간격(평소·해상): 복사를 붙여 두지 않는다. 간격마다 한 장만 찍고 바로 끊는다
//!   (복사 연결이 게임 창에 계속 붙어 있으면 해상에서도 잔렉이 생길 수 있어서).
use crate::win::{self, Hwnd, is_admin, is_minimized, is_window};
use crate::{log, say_once};
use recog::Rgb;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Foundation::TimeSpan;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind, GraphicsCaptureItem,
    GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{Interface, Result};

/// 이보다 긴 간격은 한 장씩 찍고 끊는다
pub const SNAPSHOT_MIN_MS: u64 = 1000;

/// 받은 화면 (BGRA → RGB, 배율로 늘어난 창은 원래 크기로 줄인 것)
pub struct Frame {
    pub rgb: Arc<Rgb>,
    /// 받은 시각 (1970년부터 초)
    pub at: f64,
}

#[derive(Default)]
struct Shared {
    interval_ms: AtomicU64,
    stop: AtomicBool,
    closed: AtomicBool,
    frame: Mutex<Option<(Arc<Rgb>, f64)>>,
    scale_bits: AtomicU64,
}

pub struct WindowCapture {
    pub hwnd: Hwnd,
    shared: Arc<Shared>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl WindowCapture {
    pub fn new(hwnd: Hwnd, interval_ms: u64) -> Self {
        let shared = Arc::new(Shared::default());
        shared.interval_ms.store(interval_ms, Ordering::SeqCst);
        shared.scale_bits.store(1f64.to_bits(), Ordering::SeqCst);
        let s = shared.clone();
        std::thread::spawn(move || Worker::new(hwnd, s).run());
        WindowCapture { hwnd, shared }
    }

    pub fn interval_ms(&self) -> u64 {
        self.shared.interval_ms.load(Ordering::SeqCst)
    }

    pub fn set_interval(&self, ms: u64) {
        self.shared.interval_ms.store(ms, Ordering::SeqCst);
    }

    pub fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }

    pub fn scale(&self) -> f64 {
        f64::from_bits(self.shared.scale_bits.load(Ordering::SeqCst))
    }

    /// 가장 최근 화면
    pub fn latest(&self) -> Option<Frame> {
        self.shared.frame.lock().unwrap().as_ref().map(|(rgb, at)| Frame { rgb: rgb.clone(), at: *at })
    }

    /// 복사한 그림에서 게임 화면(제목 줄·테두리 제외)이 시작하는 곳. 줄인 그림이면 줄인 좌표로
    pub fn client_offset(&self) -> (i64, i64) {
        let (fx, fy, _, _) = win::frame_rect(self.hwnd);
        let (cx, cy, _, _) = win::client_rect_screen(self.hwnd);
        let s = self.scale();
        (((cx - fx) as f64 / s).round().max(0.0) as i64, ((cy - fy) as f64 / s).round().max(0.0) as i64)
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.closed.store(true, Ordering::SeqCst);
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------- Direct3D 장치
struct D3d {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    winrt: IDirect3DDevice,
    staging: Option<(u32, u32, ID3D11Texture2D)>,
}

impl D3d {
    fn new() -> Result<Self> {
        let (mut device, mut ctx) = (None, None);
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )?;
        }
        let device = device.unwrap();
        let dxgi: IDXGIDevice = device.cast()?;
        let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        Ok(D3d { device, ctx: ctx.unwrap(), winrt, staging: None })
    }

    /// 받은 화면을 CPU로 옮겨 RGB 로
    fn read(&mut self, frame: &Direct3D11CaptureFrame) -> Result<Rgb> {
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { tex.GetDesc(&mut desc) };
        let cs = frame.ContentSize()?;
        let (w, h) = ((cs.Width.max(1) as u32).min(desc.Width), (cs.Height.max(1) as u32).min(desc.Height));
        if !matches!(&self.staging, Some((sw, sh, _)) if *sw == desc.Width && *sh == desc.Height) {
            let mut sd = desc;
            sd.Usage = D3D11_USAGE_STAGING;
            sd.BindFlags = 0;
            sd.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            sd.MiscFlags = 0;
            sd.MipLevels = 1;
            sd.ArraySize = 1;
            let mut st = None;
            unsafe { self.device.CreateTexture2D(&sd, None, Some(&mut st))? };
            self.staging = Some((desc.Width, desc.Height, st.unwrap()));
        }
        let st = &self.staging.as_ref().unwrap().2;
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.ctx.CopyResource(st, &tex);
            self.ctx.Map(st, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
        }
        let (w, h) = (w as usize, h as usize);
        let mut data = vec![0u8; w * h * 3];
        let pitch = m.RowPitch as usize;
        let src = unsafe { std::slice::from_raw_parts(m.pData as *const u8, pitch * h) };
        for y in 0..h {
            let row = &src[y * pitch..y * pitch + w * 4];
            for (d, s) in data[y * w * 3..(y + 1) * w * 3].chunks_exact_mut(3).zip(row.chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
            }
        }
        unsafe { self.ctx.Unmap(st, 0) };
        Ok(Rgb::new(w, h, data))
    }
}

struct Session {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
}

impl Session {
    fn open(d: &D3d, h: Hwnd, interval_ms: u64) -> Result<Self> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(win::hwnd(h))? };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d.winrt, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size)?;
        let session = pool.CreateCaptureSession(&item)?;
        // 이 PC의 Windows가 못 쓰는 옵션은 빼고 그대로 시작한다 (2026-11-12: 테두리 끄기를 못 하는 PC가 있었다)
        if let Err(e) = session.SetIsCursorCaptureEnabled(false) {
            say_once(&format!("[화면 복사] 이 Windows에서 안 되는 옵션을 빼고 복사합니다 (커서 끄기): {}", e.message()));
        }
        if let Err(e) = session.SetIsBorderRequired(false) {
            say_once(&format!("[화면 복사] 이 Windows에서 안 되는 옵션을 빼고 복사합니다 (테두리 끄기): {}", e.message()));
        }
        if let Err(e) = session.SetMinUpdateInterval(TimeSpan { Duration: interval_ms as i64 * 10_000 }) {
            say_once(&format!("[화면 복사] 이 Windows에서 안 되는 옵션을 빼고 복사합니다 (간격 지정): {}", e.message()));
        }
        session.StartCapture()?;
        Ok(Session { pool, session, size })
    }

    fn close(self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

// ---------------------------------------------------------------- 복사 일꾼 (창마다 스레드 하나)
struct Worker {
    hwnd: Hwnd,
    s: Arc<Shared>,
    got_any: bool,
    misses: u32,
    t_scale: Option<Instant>,
}

impl Worker {
    fn new(hwnd: Hwnd, s: Arc<Shared>) -> Self {
        Worker { hwnd, s, got_any: false, misses: 0, t_scale: None }
    }

    fn alive(&self) -> bool {
        !self.s.stop.load(Ordering::SeqCst)
    }

    fn run(mut self) {
        // 테두리 없이 복사해도 되는지 Windows에 한 번 묻는다 (안 되는 PC는 그냥 넘어감)
        if let Ok(op) = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless) {
            let _ = op.get();
        }
        let mut d = match D3d::new() {
            Ok(d) => d,
            Err(e) => {
                say_once(&format!("[화면 복사] 창 {} 복사를 시작하지 못함: {}", self.hwnd, e.message()));
                self.s.closed.store(true, Ordering::SeqCst);
                return;
            }
        };
        while self.alive() {
            if !is_window(self.hwnd) {
                break;
            }
            let iv = self.s.interval_ms.load(Ordering::SeqCst);
            if iv >= SNAPSHOT_MIN_MS {
                let t0 = Instant::now();
                if !is_minimized(self.hwnd) {
                    self.snapshot_once(&mut d, Duration::from_millis(1500));
                }
                let end = Instant::now() + Duration::from_millis(iv).saturating_sub(t0.elapsed()).max(Duration::from_millis(50));
                while Instant::now() < end && self.alive() && self.s.interval_ms.load(Ordering::SeqCst) == iv {
                    std::thread::sleep(Duration::from_millis(50));
                }
            } else {
                self.stream(&mut d, iv);
            }
        }
        self.s.closed.store(true, Ordering::SeqCst);
    }

    /// 느린 간격: 한 장 찍고 끊는다
    fn snapshot_once(&mut self, d: &mut D3d, timeout: Duration) {
        let ses = match Session::open(d, self.hwnd, SNAPSHOT_MIN_MS) {
            Ok(s) => s,
            Err(e) => {
                say_once(&format!("[화면 복사] 창 {} 복사를 시작하지 못함: {}", self.hwnd, e.message()));
                self.missed();
                return;
            }
        };
        let t0 = Instant::now();
        let mut got = false;
        while t0.elapsed() < timeout && self.alive() {
            if let Ok(f) = ses.pool.TryGetNextFrame() {
                if let Ok(rgb) = d.read(&f) {
                    self.store(rgb);
                    got = true;
                }
                let _ = f.Close();
                if got {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        ses.close();
        if !got {
            self.missed();
        }
    }

    /// 빠른 간격: 연결해 둔 채로 화면을 받는다. 간격이 1초 이상으로 바뀌면 끊고 돌아간다
    fn stream(&mut self, d: &mut D3d, iv: u64) {
        let mut ses = match Session::open(d, self.hwnd, iv) {
            Ok(s) => s,
            Err(e) => {
                say_once(&format!("[화면 복사] 창 {} 복사를 시작하지 못함: {}", self.hwnd, e.message()));
                std::thread::sleep(Duration::from_millis(500));
                return;
            }
        };
        let mut last: Option<Instant> = None;
        let (mut arrived, mut recreated, t0) = (0u32, 0u32, Instant::now());
        while self.alive() && self.s.interval_ms.load(Ordering::SeqCst) == iv && is_window(self.hwnd) {
            if let Ok(f) = ses.pool.TryGetNextFrame() {
                arrived += 1;
                if let Ok(cs) = f.ContentSize() {
                    if cs.Width != ses.size.Width || cs.Height != ses.size.Height {
                        // 창 크기가 바뀜: 받는 그림 크기를 맞춘다
                        recreated += 1;
                        ses.size = cs;
                        let _ = ses.pool.Recreate(&d.winrt, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, cs);
                    }
                }
                if last.is_none_or(|t| t.elapsed().as_secs_f64() >= iv as f64 / 1000.0 * 0.8) {
                    if let Ok(rgb) = d.read(&f) {
                        self.store(rgb);
                        last = Some(Instant::now());
                    }
                }
                let _ = f.Close();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if std::env::var_os("DHO_DEBUG").is_some() {
            log(&format!("[화면 복사 점검] 창 {} 연결 {:?} 동안 화면 {arrived}장 옴 · 크기 다시 맞춤 {recreated}번", self.hwnd, t0.elapsed()));
        }
        ses.close();
    }

    fn store(&mut self, rgb: Rgb) {
        // 창을 다른 배율의 모니터로 옮길 수 있어서 가끔 다시 본다
        if self.t_scale.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.t_scale = Some(Instant::now());
            let s = win::dpi_scale(self.hwnd);
            let old = f64::from_bits(self.s.scale_bits.load(Ordering::SeqCst));
            if s != old || !self.got_any && s != 1.0 {
                log(&if s != 1.0 {
                    format!("[화면] 디스플레이 배율 {}% · 게임 화면을 원래 크기로 줄여서 읽습니다", (s * 100.0).round())
                } else {
                    "[화면] 디스플레이 배율 100%".into()
                });
            }
            self.s.scale_bits.store(s.to_bits(), Ordering::SeqCst);
        }
        let scale = f64::from_bits(self.s.scale_bits.load(Ordering::SeqCst));
        let rgb = if scale != 1.0 {
            let (w, h) = (((rgb.w as f64 / scale).round() as usize).max(1), ((rgb.h as f64 / scale).round() as usize).max(1));
            resize_lanczos4(&rgb, w, h)
        } else {
            rgb
        };
        let (w, h) = (rgb.w, rgb.h);
        *self.s.frame.lock().unwrap() = Some((Arc::new(rgb), now()));
        if !self.got_any {
            self.got_any = true;
            log(&format!(
                "[화면 복사] 게임 화면을 받기 시작함 · 창 {} · {w}×{h}{}",
                self.hwnd,
                if scale != 1.0 { format!(" (배율 {}%를 줄인 크기)", (scale * 100.0).round()) } else { String::new() }
            ));
        } else if self.misses >= 3 {
            log(&format!("[화면 복사] 창 {} 화면을 다시 받음", self.hwnd));
        }
        self.misses = 0;
    }

    /// 화면이 안 왔다: 3번 연달아(약 8초)면 로그에 이유 후보를 남긴다
    fn missed(&mut self) {
        self.misses += 1;
        if self.misses == 3 {
            let why = if is_minimized(self.hwnd) {
                "최소화됨"
            } else if !is_admin() {
                "감지기가 관리자 권한이 아님 → 오른쪽 클릭 → '관리자 권한으로 실행'으로 켜 보세요"
            } else {
                "원인 모름"
            };
            log(&format!(
                "[화면 복사] 창 {} 화면이 오지 않음 ({}) · {why}",
                self.hwnd,
                if self.got_any { "받다가 끊김" } else { "한 번도 못 받음" }
            ));
        }
    }
}

/// cv2.resize(..., INTER_LANCZOS4): 배율로 늘어난 창을 원래 크기로 (글자가 가장 잘 돌아오는 방식, 1.25·1.5배 시험)
pub fn resize_lanczos4(src: &Rgb, dw: usize, dh: usize) -> Rgb {
    fn coeffs(x: f32) -> [f32; 8] {
        let mut c = [0f32; 8];
        if x < f32::EPSILON {
            c[3] = 1.0;
            return c;
        }
        let mut sum = 0f32;
        for (i, v) in c.iter_mut().enumerate() {
            let t = (x as f64 + 3.0 - i as f64) * std::f64::consts::PI;
            *v = ((t.sin() / t) * ((t / 4.0).sin() / (t / 4.0))) as f32;
            sum += *v;
        }
        c.iter_mut().for_each(|v| *v /= sum);
        c
    }
    let tab = |sn: usize, dn: usize| -> Vec<(i64, [f32; 8])> {
        let scale = sn as f64 / dn as f64;
        (0..dn)
            .map(|d| {
                let f = ((d as f64 + 0.5) * scale - 0.5) as f32;
                let s = f.floor();
                (s as i64, coeffs(f - s))
            })
            .collect()
    };
    let (xt, yt) = (tab(src.w, dw), tab(src.h, dh));
    let clampi = |v: i64, n: usize| v.clamp(0, n as i64 - 1) as usize;
    let mut tmp = vec![0f32; dw * src.h * 3];
    for y in 0..src.h {
        for (x, (sx, c)) in xt.iter().enumerate() {
            for ch in 0..3 {
                let mut s = 0f32;
                for (k, &ck) in c.iter().enumerate() {
                    s += ck * src.data[(y * src.w + clampi(sx - 3 + k as i64, src.w)) * 3 + ch] as f32;
                }
                tmp[(y * dw + x) * 3 + ch] = s;
            }
        }
    }
    let mut out = vec![0u8; dw * dh * 3];
    for (y, (sy, c)) in yt.iter().enumerate() {
        for x in 0..dw {
            for ch in 0..3 {
                let mut s = 0f32;
                for (k, &ck) in c.iter().enumerate() {
                    s += ck * tmp[(clampi(sy - 3 + k as i64, src.h) * dw + x) * 3 + ch];
                }
                out[(y * dw + x) * 3 + ch] = s.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgb::new(dw, dh, out)
}
