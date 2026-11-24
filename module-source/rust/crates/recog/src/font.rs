//! 굴림 14px 글자 틀 그리기 (Windows GDI).
//!
//! 게임은 굴림의 내장 비트맵 글꼴로 글자를 그린다. GDI도 같은 비트맵을 쓰므로
//! Python 판(PIL + FreeType)과 같은 모양이 나온다 (tests/font.rs 에서 4천여 개 틀로 확인).

use crate::cv::{Gray, trim};
use std::ffi::c_void;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::*;
use windows::core::w;

const H: usize = 30;

pub struct Gulim {
    dc: HDC,
    bmp: HBITMAP,
    font: HFONT,
    bits: *mut u32,
    cap: usize, // 그림 폭
}

impl Gulim {
    /// 굴림 `px` 크기 (게임 글자는 14, 숫자 대체 글자는 13)
    pub fn new(px: i32) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let font = CreateFontW(
                -px, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
                HANGUL_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32, NONANTIALIASED_QUALITY.0 as u32,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, w!("Gulim"),
            );
            if font.is_invalid() {
                let _ = DeleteDC(dc);
                return None;
            }
            SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(0x00FF_FFFF));
            let mut g = Gulim { dc, bmp: HBITMAP::default(), font, bits: std::ptr::null_mut(), cap: 0 };
            g.ensure(1024)?;
            Some(g)
        }
    }

    fn ensure(&mut self, w: usize) -> Option<()> {
        if w <= self.cap {
            return Some(());
        }
        let w = w.next_power_of_two();
        unsafe {
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w as i32;
            bi.bmiHeader.biHeight = -(H as i32);
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(self.dc, &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            SelectObject(self.dc, bmp);
            if !self.bmp.is_invalid() {
                let _ = DeleteObject(self.bmp);
            }
            self.bmp = bmp;
            self.bits = bits as *mut u32;
            self.cap = w;
        }
        Some(())
    }

    /// 글자 하나의 나아가는 폭(px)
    pub fn advance(&self, ch: char) -> i32 {
        let mut buf = [0u16; 2];
        let s = ch.encode_utf16(&mut buf);
        let mut sz = SIZE::default();
        unsafe {
            let _ = GetTextExtentPoint32W(self.dc, s, &mut sz);
        }
        sz.cx
    }

    /// Python `NameMatcher._render`: 글자 사이에 `extra`px, 빈칸 뒤에 `space`px 더 띄워 그린 뒤
    /// 글자 부분만 잘라 0/255 로 돌려준다. 아무것도 안 그려지면 None
    pub fn render(&mut self, t: &str, extra: i32, space: i32) -> Option<Gray> {
        let n = t.chars().count();
        let nsp = t.chars().filter(|&c| c == ' ').count();
        let w = (16 + extra.max(0)) as usize * n + 20 + space.max(0) as usize * nsp;
        self.ensure(w)?;
        unsafe {
            std::ptr::write_bytes(self.bits, 0, self.cap * H);
            let mut x = 4i32;
            for ch in t.chars() {
                let mut buf = [0u16; 2];
                let s = ch.encode_utf16(&mut buf);
                let _ = TextOutW(self.dc, x, 4, s);
                x += self.advance(ch) + if ch != ' ' { extra } else { space };
            }
            let _ = GdiFlush();
        }
        let px = unsafe { std::slice::from_raw_parts(self.bits, self.cap * H) };
        let mut g = Gray::new(w, H);
        for y in 0..H {
            for x in 0..w {
                // 흰 글자: 빨강 칸 > 100 (Python `a > 100`)
                g.data[y * w + x] = if (px[y * self.cap + x] >> 16) & 0xFF > 100 { 255.0 } else { 0.0 };
            }
        }
        trim(&g, &g.gt(100.0))
    }
}

impl Drop for Gulim {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.dc);
            let _ = DeleteObject(self.bmp);
            let _ = DeleteObject(self.font);
        }
    }
}
