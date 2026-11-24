//! 그림 자료형과 계산 (Python 판의 numpy · OpenCV 부분을 손으로 옮김).
//!
//! OpenCV와 계산 방식을 맞춰 둔 것: 선형 보간 크기 바꾸기, 템플릿 매칭(TM_CCOEFF_NORMED·마스크 TM_SQDIFF),
//! OTSU 문턱, 3×3 팽창, RGB→회색.

/// RGB 그림 (한 화소 3바이트, 왼쪽 위부터 가로줄 순서)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

/// 실수 한 채널 그림
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

/// 참/거짓 그림 (글자 자리 등)
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub data: Vec<bool>,
}

/// numpy 자르기 `a[y0:y1, x0:x1]`처럼 범위를 그림 안으로 줄인다 (끝이 시작보다 작으면 빈 범위)
fn clamp_range(s: i64, e: i64, n: usize) -> (usize, usize) {
    let n = n as i64;
    let s = s.clamp(0, n);
    let e = e.clamp(0, n);
    (s as usize, e.max(s) as usize)
}

impl Rgb {
    pub fn new(w: usize, h: usize, data: Vec<u8>) -> Self {
        assert_eq!(data.len(), w * h * 3);
        Self { w, h, data }
    }
    #[inline]
    pub fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.w + x) * 3;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    /// `rgb[y0:y1, x0:x1]`
    pub fn crop(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> Rgb {
        let (x0, x1) = clamp_range(x0, x1, self.w);
        let (y0, y1) = clamp_range(y0, y1, self.h);
        let (w, h) = (x1 - x0, y1 - y0);
        let mut data = Vec::with_capacity(w * h * 3);
        for y in y0..y1 {
            let i = (y * self.w + x0) * 3;
            data.extend_from_slice(&self.data[i..i + w * 3]);
        }
        Rgb { w, h, data }
    }
    /// 한 채널 (0=R, 1=G, 2=B)
    pub fn channel(&self, c: usize) -> Gray {
        Gray { w: self.w, h: self.h, data: self.data.chunks_exact(3).map(|p| p[c] as f32).collect() }
    }
    /// 세 채널 중 가장 어두운 값
    pub fn min_channel(&self) -> Gray {
        Gray { w: self.w, h: self.h, data: self.data.chunks_exact(3).map(|p| p[0].min(p[1]).min(p[2]) as f32).collect() }
    }
    /// OpenCV COLOR_RGB2GRAY (8비트 고정소수점과 같은 결과)
    pub fn to_gray_cv(&self) -> Gray {
        let data = self
            .data
            .chunks_exact(3)
            .map(|p| ((p[0] as u32 * 4899 + p[1] as u32 * 9617 + p[2] as u32 * 1868 + (1 << 13)) >> 14) as f32)
            .collect();
        Gray { w: self.w, h: self.h, data }
    }
}

impl Gray {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, data: vec![0.0; w * h] }
    }
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    pub fn crop(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> Gray {
        let (x0, x1) = clamp_range(x0, x1, self.w);
        let (y0, y1) = clamp_range(y0, y1, self.h);
        let (w, h) = (x1 - x0, y1 - y0);
        let mut data = Vec::with_capacity(w * h);
        for y in y0..y1 {
            let i = y * self.w + x0;
            data.extend_from_slice(&self.data[i..i + w]);
        }
        Gray { w, h, data }
    }
    /// 가로·세로 한 칸씩 건너뛰어 줄인 그림 (`a[::2, ::2]`)
    pub fn half(&self) -> Gray {
        let (w, h) = (self.w.div_ceil(2), self.h.div_ceil(2));
        let mut data = Vec::with_capacity(w * h);
        for y in (0..self.h).step_by(2) {
            for x in (0..self.w).step_by(2) {
                data.push(self.at(x, y));
            }
        }
        Gray { w, h, data }
    }
    pub fn gt(&self, th: f32) -> Mask {
        Mask { w: self.w, h: self.h, data: self.data.iter().map(|&v| v > th).collect() }
    }
    pub fn lt(&self, th: f32) -> Mask {
        Mask { w: self.w, h: self.h, data: self.data.iter().map(|&v| v < th).collect() }
    }
    pub fn mean(&self) -> f64 {
        if self.data.is_empty() {
            return f64::NAN;
        }
        self.data.iter().map(|&v| v as f64).sum::<f64>() / self.data.len() as f64
    }
    /// 둘레에 `p`칸씩 0을 붙인다 (`np.pad(a, p)`)
    pub fn pad(&self, p: usize) -> Gray {
        let (w, h) = (self.w + 2 * p, self.h + 2 * p);
        let mut out = Gray::new(w, h);
        for y in 0..self.h {
            let d = (y + p) * w + p;
            out.data[d..d + self.w].copy_from_slice(&self.data[y * self.w..(y + 1) * self.w]);
        }
        out
    }
}

impl Mask {
    pub fn count(&self) -> usize {
        self.data.iter().filter(|&&v| v).count()
    }
    pub fn to_gray(&self) -> Gray {
        Gray { w: self.w, h: self.h, data: self.data.iter().map(|&v| v as u8 as f32).collect() }
    }
    pub fn crop(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> Mask {
        let (x0, x1) = clamp_range(x0, x1, self.w);
        let (y0, y1) = clamp_range(y0, y1, self.h);
        let (w, h) = (x1 - x0, y1 - y0);
        let mut data = Vec::with_capacity(w * h);
        for y in y0..y1 {
            let i = y * self.w + x0;
            data.extend_from_slice(&self.data[i..i + w]);
        }
        Mask { w, h, data }
    }
    pub fn and(&self, o: &Mask) -> Mask {
        Mask { w: self.w, h: self.h, data: self.data.iter().zip(&o.data).map(|(&a, &b)| a && b).collect() }
    }
    /// 참인 칸을 감싸는 가장 작은 상자 (x0, y0, x1, y1). 하나도 없으면 None
    pub fn bbox(&self) -> Option<(usize, usize, usize, usize)> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.data[y * self.w + x] {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        (x0 != usize::MAX).then_some((x0, y0, x1, y1))
    }
    /// numpy packbits 와 같은 비트 묶음 (모양 비교 열쇠로 씀)
    pub fn packbits(&self) -> Vec<u8> {
        self.data.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (i, &v)| b | ((v as u8) << (7 - i)))).collect()
    }
    /// 3×3 팽창 (cv2.dilate, 바깥은 영향 없음)
    pub fn dilate3(&self) -> Mask {
        let mut out = Mask { w: self.w, h: self.h, data: vec![false; self.data.len()] };
        for y in 0..self.h {
            for x in 0..self.w {
                if !self.data[y * self.w + x] {
                    continue;
                }
                for yy in y.saturating_sub(1)..(y + 2).min(self.h) {
                    for xx in x.saturating_sub(1)..(x + 2).min(self.w) {
                        out.data[yy * self.w + xx] = true;
                    }
                }
            }
        }
        out
    }
}

/// `trim(a, m)`: 마스크가 참인 범위만 잘라낸다
pub fn trim(a: &Gray, m: &Mask) -> Option<Gray> {
    let (x0, y0, x1, y1) = m.bbox()?;
    Some(a.crop(x0 as i64, y0 as i64, x1 as i64, y1 as i64))
}

pub fn trim_mask(m: &Mask) -> Option<Mask> {
    let (x0, y0, x1, y1) = m.bbox()?;
    Some(m.crop(x0 as i64, y0 as i64, x1 as i64, y1 as i64))
}

/// 흰 글씨와 파란 글씨(희귀품)를 모두 살리는 밝기 채널. 시세 창 선택 행의 청록 바탕은 0
pub fn bright(rgb: &Rgb) -> Gray {
    let data = rgb
        .data
        .chunks_exact(3)
        .map(|p| {
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            let teal = g > r + 70.0 && b > r + 70.0 && (g - b).abs() < 40.0;
            if teal { 0.0 } else { r.min(g).min(b).max(r.max(g).max(b) * 0.75) }
        })
        .collect();
    Gray { w: rgb.w, h: rgb.h, data }
}

/// OpenCV THRESH_OTSU 문턱 (8비트)
pub fn otsu(v: &[u8]) -> f64 {
    let mut hist = [0f64; 256];
    for &x in v {
        hist[x as usize] += 1.0;
    }
    let scale = 1.0 / v.len() as f64;
    let mu: f64 = (0..256).map(|i| i as f64 * hist[i]).sum::<f64>() * scale;
    let (mut mu1, mut q1, mut max_sigma, mut max_val) = (0f64, 0f64, 0f64, 0f64);
    let eps = f32::EPSILON as f64;
    for (i, &h) in hist.iter().enumerate() {
        let p_i = h * scale;
        mu1 *= q1;
        q1 += p_i;
        let q2 = 1.0 - q1;
        if q1.min(q2) < eps || q1.max(q2) > 1.0 - eps {
            continue;
        }
        mu1 = (mu1 + i as f64 * p_i) / q1;
        let mu2 = (mu - q1 * mu1) / q2;
        let sigma = q1 * q2 * (mu1 - mu2) * (mu1 - mu2);
        if sigma > max_sigma {
            max_sigma = sigma;
            max_val = i as f64;
        }
    }
    max_val
}

/// 글자 자리: OTSU 문턱(최소 120)보다 밝은 곳
pub fn text_mask(gray: &Gray) -> Mask {
    let g: Vec<u8> = gray.data.iter().map(|&v| v.clamp(0.0, 255.0) as u8).collect();
    let th = otsu(&g).max(120.0) as f32;
    gray.gt(th)
}

/// cv2.resize(..., INTER_LINEAR) — 실수 그림
pub fn resize_linear(src: &Gray, dw: usize, dh: usize) -> Gray {
    if src.w == dw && src.h == dh {
        return src.clone();
    }
    let tab = |sn: usize, dn: usize| -> Vec<(usize, f32)> {
        let scale = sn as f64 / dn as f64;
        (0..dn)
            .map(|d| {
                let f = ((d as f64 + 0.5) * scale - 0.5) as f32;
                let mut s = f.floor() as i64;
                let mut fr = f - s as f32;
                if s < 0 {
                    s = 0;
                    fr = 0.0;
                }
                if s >= sn as i64 - 1 {
                    s = sn as i64 - 1;
                    fr = 0.0;
                }
                (s as usize, fr)
            })
            .collect()
    };
    let xt = tab(src.w, dw);
    let yt = tab(src.h, dh);
    // 가로 먼저 (OpenCV와 같은 순서)
    let row = |y: usize| -> Vec<f32> {
        xt.iter()
            .map(|&(sx, fx)| {
                let a = src.at(sx, y);
                if fx == 0.0 { a } else { a * (1.0 - fx) + src.at(sx + 1, y) * fx }
            })
            .collect()
    };
    let mut out = Gray::new(dw, dh);
    for (dy, &(sy, fy)) in yt.iter().enumerate() {
        let r0 = row(sy);
        if fy == 0.0 {
            out.data[dy * dw..(dy + 1) * dw].copy_from_slice(&r0);
        } else {
            let r1 = row(sy + 1);
            for x in 0..dw {
                out.data[dy * dw + x] = r0[x] * (1.0 - fy) + r1[x] * fy;
            }
        }
    }
    out
}

/// 크기를 맞춘 두 그림의 정규화 상관 (Python `ncc`)
pub fn ncc(a: &Gray, b: &Gray) -> f64 {
    if a.is_empty() || b.is_empty() {
        return -1.0;
    }
    let bb = resize_linear(b, a.w, a.h);
    let (ma, mb) = (a.mean(), bb.mean());
    let (mut sab, mut saa, mut sbb) = (0f64, 0f64, 0f64);
    for (&x, &y) in a.data.iter().zip(&bb.data) {
        let (x, y) = (x as f64 - ma, y as f64 - mb);
        sab += x * y;
        saa += x * x;
        sbb += y * y;
    }
    let d = (saa * sbb).sqrt();
    if d != 0.0 { sab / d } else { 0.0 }
}

/// cv2.matchTemplate(img, tpl, TM_CCOEFF_NORMED). 결과 크기 (W-w+1)×(H-h+1)
pub fn match_ccoeff_normed(img: &Gray, tpl: &Gray) -> Gray {
    match_ccoeff_normed_ch(&[img], &[tpl])
}

/// 여러 채널(색 그림) TM_CCOEFF_NORMED: 채널마다 평균을 빼고, 분자·분모는 채널을 모두 합친다 (OpenCV와 같음)
pub fn match_ccoeff_normed_ch(img: &[&Gray], tpl: &[&Gray]) -> Gray {
    let (iw0, ih0, tw, th) = (img[0].w, img[0].h, tpl[0].w, tpl[0].h);
    assert!(tw <= iw0 && th <= ih0 && tw * th > 0);
    let (rw, rh) = (iw0 - tw + 1, ih0 - th + 1);
    let n = (tw * th) as f64;
    let iw = iw0 + 1;
    let mut num = vec![0f32; rw * rh];
    let mut tnorm2 = 0f64;
    // 채널마다 창 안 합(적분 그림)
    let integral = |f: &dyn Fn(usize, usize) -> f64| -> Vec<f64> {
        let mut s = vec![0f64; iw * (ih0 + 1)];
        for y in 0..ih0 {
            let mut r = 0f64;
            for x in 0..iw0 {
                r += f(x, y);
                s[(y + 1) * iw + x + 1] = s[y * iw + x + 1] + r;
            }
        }
        s
    };
    let mut s1s = Vec::new();
    let s2 = integral(&|x, y| img.iter().map(|im| { let v = im.at(x, y) as f64; v * v }).sum());
    for (im, tp) in img.iter().zip(tpl) {
        let tmean = tp.mean();
        let tz: Vec<f32> = tp.data.iter().map(|&v| (v as f64 - tmean) as f32).collect();
        tnorm2 += tz.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>();
        s1s.push(integral(&|x, y| im.at(x, y) as f64));
        // 분자: 그림 × (틀 − 평균). 줄마다 더해 가면 가로 방향이 벡터화된다
        for ty in 0..th {
            for tx in 0..tw {
                let t = tz[ty * tw + tx];
                if t == 0.0 {
                    continue;
                }
                for y in 0..rh {
                    let src = &im.data[(y + ty) * iw0 + tx..(y + ty) * iw0 + tx + rw];
                    let dst = &mut num[y * rw..(y + 1) * rw];
                    for (d, &s) in dst.iter_mut().zip(src) {
                        *d += s * t;
                    }
                }
            }
        }
    }
    let tnorm = tnorm2.sqrt();
    if tnorm < f64::EPSILON {
        return Gray { w: rw, h: rh, data: vec![1.0; rw * rh] };
    }
    let mut out = Gray::new(rw, rh);
    for y in 0..rh {
        for x in 0..rw {
            let a = |s: &[f64]| s[(y + th) * iw + x + tw] - s[y * iw + x + tw] - s[(y + th) * iw + x] + s[y * iw + x];
            let sum2 = a(&s2);
            let mean2: f64 = s1s.iter().map(|s1| { let v = a(s1); v * v / n }).sum();
            let var = (sum2 - mean2).max(0.0);
            let t = var.sqrt() * tnorm;
            let v = num[y * rw + x] as f64;
            out.data[y * rw + x] = if v.abs() < t {
                (v / t) as f32
            } else if v.abs() < t * 1.125 {
                v.signum() as f32
            } else {
                0.0
            };
        }
    }
    out
}

/// 가장 큰 값과 그 자리 (cv2.minMaxLoc 의 max 쪽: 같으면 먼저 나온 자리)
pub fn max_loc(r: &Gray) -> (f32, usize, usize) {
    let mut best = (f32::NEG_INFINITY, 0, 0);
    for y in 0..r.h {
        for x in 0..r.w {
            let v = r.at(x, y);
            if v > best.0 {
                best = (v, x, y);
            }
        }
    }
    best
}

pub fn match_max(img: &Gray, tpl: &Gray) -> f32 {
    max_loc(&match_ccoeff_normed(img, tpl)).0
}

/// 색 그림 + 마스크 TM_SQDIFF 의 가장 작은 값 (세 채널 합)
pub fn sqdiff_masked_min(img: &Rgb, tpl: &[[f32; 3]], tw: usize, th: usize, mask: &[bool]) -> f64 {
    let (rw, rh) = (img.w - tw + 1, img.h - th + 1);
    let mut best = f64::INFINITY;
    for y in 0..rh {
        for x in 0..rw {
            let mut s = 0f64;
            for ty in 0..th {
                for tx in 0..tw {
                    if !mask[ty * tw + tx] {
                        continue;
                    }
                    let p = img.px(x + tx, y + ty);
                    let t = tpl[ty * tw + tx];
                    for c in 0..3 {
                        let d = t[c] as f64 - p[c] as f64;
                        s += d * d;
                    }
                }
            }
            best = best.min(s);
        }
    }
    best
}

/// PIL `Image.resize(..., Image.LANCZOS)` 와 화소까지 같은 확대 (글자 인식에 넘기기 전).
/// PIL 처럼 가로 먼저 8비트로 반올림해 두고 세로를 한다. 계수는 22비트 고정소수점.
pub fn upscale_lanczos(src: &Rgb, k: usize) -> Rgb {
    if k == 1 {
        return src.clone();
    }
    resize_lanczos_pil(src, src.w * k, src.h * k)
}

pub fn resize_lanczos_pil(src: &Rgb, dw: usize, dh: usize) -> Rgb {
    const PB: u32 = 22; // PRECISION_BITS = 32 - 8 - 2
    fn sinc(x: f64) -> f64 {
        if x == 0.0 {
            return 1.0;
        }
        let x = x * std::f64::consts::PI;
        x.sin() / x
    }
    fn lanczos(x: f64) -> f64 {
        if -3.0 < x && x < 3.0 { sinc(x) * sinc(x / 3.0) } else { 0.0 }
    }
    // PIL precompute_coeffs + normalize_coeffs_8bpc
    let coeffs = |insize: usize, outsize: usize| -> Vec<(usize, Vec<i64>)> {
        let scale = insize as f64 / outsize as f64;
        let filterscale = scale.max(1.0);
        let support = 3.0 * filterscale;
        let ss = 1.0 / filterscale;
        (0..outsize)
            .map(|xx| {
                let center = (xx as f64 + 0.5) * scale;
                let xmin = ((center - support + 0.5) as i64).max(0) as usize;
                let xmax = ((center + support + 0.5) as i64).min(insize as i64) as usize;
                let mut w: Vec<f64> = (xmin..xmax).map(|x| lanczos((x as f64 - center + 0.5) * ss)).collect();
                let total: f64 = w.iter().sum();
                if total != 0.0 {
                    w.iter_mut().for_each(|v| *v /= total);
                }
                let kk = w
                    .iter()
                    .map(|&v| {
                        let s = v * (1u64 << PB) as f64;
                        if v < 0.0 { (-0.5 + s).trunc() as i64 } else { (0.5 + s).trunc() as i64 }
                    })
                    .collect();
                (xmin, kk)
            })
            .collect()
    };
    let clip8 = |v: i64| -> u8 { (v >> PB).clamp(0, 255) as u8 };
    // 가로
    let xc = coeffs(src.w, dw);
    let mut tmp = vec![0u8; dw * src.h * 3];
    for y in 0..src.h {
        for (x, (xmin, kk)) in xc.iter().enumerate() {
            for c in 0..3 {
                let mut s: i64 = 1 << (PB - 1);
                for (i, &k) in kk.iter().enumerate() {
                    s += src.data[(y * src.w + xmin + i) * 3 + c] as i64 * k;
                }
                tmp[(y * dw + x) * 3 + c] = clip8(s);
            }
        }
    }
    // 세로
    let yc = coeffs(src.h, dh);
    let mut out = vec![0u8; dw * dh * 3];
    for (y, (ymin, kk)) in yc.iter().enumerate() {
        for x in 0..dw {
            for c in 0..3 {
                let mut s: i64 = 1 << (PB - 1);
                for (i, &k) in kk.iter().enumerate() {
                    s += tmp[((ymin + i) * dw + x) * 3 + c] as i64 * k;
                }
                out[(y * dw + x) * 3 + c] = clip8(s);
            }
        }
    }
    Rgb { w: dw, h: dh, data: out }
}
