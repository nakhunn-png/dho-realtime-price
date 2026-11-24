//! PNG 읽기·쓰기 (그림 틀, 배운 아이콘)
use crate::cv::{Gray, Rgb};

/// 8비트로 펼친 화소와 채널 수
fn decode(bytes: &[u8]) -> Option<(usize, usize, usize, Vec<u8>)> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let ch = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    Some((info.width as usize, info.height as usize, ch, buf))
}

/// RGBA 로 (회색은 R=G=B, 알파 없으면 255)
pub fn load_rgba(bytes: &[u8]) -> Option<(usize, usize, Vec<[u8; 4]>)> {
    let (w, h, ch, b) = decode(bytes)?;
    let px = b
        .chunks_exact(ch)
        .map(|p| match ch {
            1 => [p[0], p[0], p[0], 255],
            2 => [p[0], p[0], p[0], p[1]],
            3 => [p[0], p[1], p[2], 255],
            _ => [p[0], p[1], p[2], p[3]],
        })
        .collect();
    Some((w, h, px))
}

pub fn load_rgb(bytes: &[u8]) -> Option<Rgb> {
    let (w, h, px) = load_rgba(bytes)?;
    Some(Rgb::new(w, h, px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()))
}

/// PIL convert("L") 과 같은 회색 (이미 회색이면 그대로)
pub fn load_l(bytes: &[u8]) -> Option<Gray> {
    let (w, h, ch, b) = decode(bytes)?;
    let data = b
        .chunks_exact(ch)
        .map(|p| {
            if ch <= 2 {
                p[0] as f32
            } else {
                ((p[0] as u32 * 19595 + p[1] as u32 * 38470 + p[2] as u32 * 7471 + 0x8000) >> 16) as f32
            }
        })
        .collect();
    Some(Gray { w, h, data })
}

pub fn save_rgb(path: &std::path::Path, im: &Rgb) -> std::io::Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut e = png::Encoder::new(f, im.w as u32, im.h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    let mut w = e.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(&im.data).map_err(std::io::Error::other)?;
    Ok(())
}

/// 빨리 저장 (압축을 덜 함: 큰 게임 화면을 자주 저장할 때)
pub fn save_rgb_fast(path: &std::path::Path, im: &Rgb) -> std::io::Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut e = png::Encoder::new(f, im.w as u32, im.h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.set_compression(png::Compression::Fast);
    let mut w = e.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(&im.data).map_err(std::io::Error::other)?;
    Ok(())
}
