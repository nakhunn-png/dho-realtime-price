//! Windows 내장 한국어 글자 인식 (Windows.Media.Ocr). Python 판의 winocr 와 같은 엔진.
use crate::cv::{Rgb, upscale_lanczos};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::HSTRING;

/// 한 줄 (글자, 첫 낱말의 x, y — 확대 전 좌표)
pub type Line = (String, f64, f64);

thread_local! {
    static ENGINE: std::cell::RefCell<Option<OcrEngine>> = const { std::cell::RefCell::new(None) };
}

fn engine() -> windows::core::Result<OcrEngine> {
    ENGINE.with(|e| {
        if let Some(x) = e.borrow().as_ref() {
            return Ok(x.clone());
        }
        unsafe {
            // 이미 초기화돼 있으면 실패해도 괜찮다
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        let x = OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from("ko"))?)?;
        *e.borrow_mut() = Some(x.clone());
        Ok(x)
    })
}

/// `scale` 배로 키워(Lanczos) 읽는다. 한국어 글자 인식이 없는 PC 등에서는 Err
pub fn ocr_lines(rgb: &Rgb, scale: usize) -> Result<Vec<Line>, String> {
    let im = upscale_lanczos(rgb, scale);
    let mut rgba = Vec::with_capacity(im.w * im.h * 4);
    for p in im.data.chunks_exact(3) {
        rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    let go = || -> windows::core::Result<Vec<Line>> {
        let w = DataWriter::new()?;
        w.WriteBytes(&rgba)?;
        let buf = w.DetachBuffer()?;
        let bmp = SoftwareBitmap::CreateCopyFromBuffer(&buf, BitmapPixelFormat::Rgba8, im.w as i32, im.h as i32)?;
        let res = engine()?.RecognizeAsync(&bmp)?.get()?;
        let mut out = vec![];
        for line in res.Lines()? {
            let words = line.Words()?;
            let (x, y) = if words.Size()? > 0 {
                let r = words.GetAt(0)?.BoundingRect()?;
                (r.X as f64 / scale as f64, r.Y as f64 / scale as f64)
            } else {
                (0.0, 0.0)
            };
            out.push((line.Text()?.to_string(), x, y));
        }
        Ok(out)
    };
    go().map_err(|e| e.message().to_string())
}
