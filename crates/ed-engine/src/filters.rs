//! CPU filter & adjustment kernels (spec §6.5/§6.6). Operate in place on
//! premultiplied RGBA8 pixmaps; adjustments demultiply, transform,
//! re-premultiply. Deterministic — this is the export ground truth (§4.1).

use ed_document::{Document, Modifier};
use tiny_skia::Pixmap;

/// Apply one modifier's filter/adjustment to a rendered layer.
/// `scale` converts doc-space radii to device pixels.
pub fn apply(doc: &Document, m: &Modifier, pm: &mut Pixmap, scale: f32) {
    let f = |key: &str, d: f64| m.params.get(key).map(|v| doc.resolve(v)).and_then(|v| v.as_f64()).unwrap_or(d);
    match m.kind.as_str() {
        "filter.gaussian-blur" => gaussian_blur(pm, (f("radius", 4.0) as f32 * scale).max(0.0)),
        "filter.pixelate" => pixelate(pm, (f("size", 8.0) as f32 * scale).max(1.0) as u32),
        "filter.noise" => noise(pm, f("amount", 0.2) as f32, 0x9e3779b9),
        "adjust.brightness-contrast" => {
            brightness_contrast(pm, f("brightness", 0.0) as f32, f("contrast", 0.0) as f32)
        }
        "adjust.hsl" => hsl(pm, f("hue", 0.0) as f32, f("saturation", 0.0) as f32, f("lightness", 0.0) as f32),
        "adjust.invert" => invert(pm),
        "adjust.grayscale" => grayscale(pm),
        "adjust.posterize" => posterize(pm, f("levels", 4.0).clamp(2.0, 32.0) as u32),
        "adjust.threshold" => threshold(pm, (f("level", 0.5) as f32).clamp(0.0, 1.0)),
        "adjust.levels" => levels(
            pm,
            f("in-black", 0.0) as f32,
            f("in-white", 1.0) as f32,
            f("gamma", 1.0) as f32,
            f("out-black", 0.0) as f32,
            f("out-white", 1.0) as f32,
        ),
        _ => {}
    }
}

pub fn is_filter_kind(kind: &str) -> bool {
    kind.starts_with("filter.") || kind.starts_with("adjust.")
}

/// Per-pixel map on straight (demultiplied) RGB, alpha untouched.
fn map_rgb(pm: &mut Pixmap, f: impl Fn(f32, f32, f32) -> (f32, f32, f32)) {
    for px in pm.pixels_mut() {
        let a = px.alpha();
        if a == 0 {
            continue;
        }
        let af = a as f32 / 255.0;
        let (r, g, b) = (
            px.red() as f32 / 255.0 / af,
            px.green() as f32 / 255.0 / af,
            px.blue() as f32 / 255.0 / af,
        );
        let (r, g, b) = f(r.min(1.0), g.min(1.0), b.min(1.0));
        let e = |v: f32| ((v.clamp(0.0, 1.0) * af) * 255.0 + 0.5) as u8;
        *px = tiny_skia::PremultipliedColorU8::from_rgba(e(r), e(g), e(b), a).unwrap();
    }
}

pub fn brightness_contrast(pm: &mut Pixmap, brightness: f32, contrast: f32) {
    let c = (1.0 + contrast).max(0.0);
    map_rgb(pm, |r, g, b| {
        let f = |v: f32| (v - 0.5) * c + 0.5 + brightness;
        (f(r), f(g), f(b))
    });
}

pub fn invert(pm: &mut Pixmap) {
    map_rgb(pm, |r, g, b| (1.0 - r, 1.0 - g, 1.0 - b));
}

pub fn grayscale(pm: &mut Pixmap) {
    map_rgb(pm, |r, g, b| {
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        (l, l, l)
    });
}

pub fn posterize(pm: &mut Pixmap, levels: u32) {
    let n = (levels - 1).max(1) as f32;
    map_rgb(pm, |r, g, b| {
        let f = |v: f32| (v * n).round() / n;
        (f(r), f(g), f(b))
    });
}

pub fn threshold(pm: &mut Pixmap, level: f32) {
    map_rgb(pm, |r, g, b| {
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let v = if l >= level { 1.0 } else { 0.0 };
        (v, v, v)
    });
}

pub fn levels(pm: &mut Pixmap, in_black: f32, in_white: f32, gamma: f32, out_black: f32, out_white: f32) {
    let span = (in_white - in_black).max(1e-4);
    let g = gamma.max(0.01);
    map_rgb(pm, |r, gg, b| {
        let f = |v: f32| {
            let t = ((v - in_black) / span).clamp(0.0, 1.0).powf(1.0 / g);
            out_black + t * (out_white - out_black)
        };
        (f(r), f(gg), f(b))
    });
}

pub fn hsl(pm: &mut Pixmap, hue_deg: f32, sat: f32, light: f32) {
    map_rgb(pm, |r, g, b| {
        let (mut h, mut s, mut l) = rgb_to_hsl(r, g, b);
        h = (h + hue_deg / 360.0).rem_euclid(1.0);
        s = (s * (1.0 + sat)).clamp(0.0, 1.0);
        l = (l + light * 0.5).clamp(0.0, 1.0);
        hsl_to_rgb(h, s, l)
    });
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s < 1e-6 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

/// Gaussian blur approximated by three box blurs (all four channels,
/// operating on premultiplied data — correct for compositing).
pub fn gaussian_blur(pm: &mut Pixmap, radius: f32) {
    if radius < 0.3 {
        return;
    }
    let w = pm.width() as usize;
    let h = pm.height() as usize;
    // three box passes approximating a gaussian of sigma ≈ radius/2
    let sigma = radius / 2.0;
    let boxes = boxes_for_gauss(sigma, 3);
    let data = pm.data_mut();
    let mut buf = data.to_vec();
    for r in boxes {
        box_blur_h(&buf, data, w, h, r);
        box_blur_v(data, &mut buf, w, h, r);
    }
    data.copy_from_slice(&buf);
}

fn boxes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    let w_ideal = ((12.0 * sigma * sigma / n as f32) + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - (n as f32) * (wl as f32) * (wl as f32)
        - 4.0 * (n as f32) * (wl as f32)
        - 3.0 * (n as f32))
        / (-4.0 * (wl as f32) - 4.0);
    let m = m_ideal.round() as usize;
    (0..n).map(|i| ((if i < m { wl } else { wu } - 1) / 2).max(0) as usize).collect()
}

fn box_blur_h(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    for y in 0..h {
        let row = y * w * 4;
        let mut sums = [0u32; 4];
        for x in 0..=r.min(w - 1) {
            for c in 0..4 {
                sums[c] += src[row + x * 4 + c] as u32;
            }
        }
        let mut count = r.min(w - 1) + 1;
        for x in 0..w {
            for c in 0..4 {
                dst[row + x * 4 + c] = (sums[c] / count as u32) as u8;
            }
            // slide window
            let add = x + r + 1;
            if add < w {
                for c in 0..4 {
                    sums[c] += src[row + add * 4 + c] as u32;
                }
                count += 1;
            }
            if x >= r {
                let rem = x - r;
                for c in 0..4 {
                    sums[c] -= src[row + rem * 4 + c] as u32;
                }
                count -= 1;
            }
        }
    }
}

fn box_blur_v(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    for x in 0..w {
        let mut sums = [0u32; 4];
        for y in 0..=r.min(h - 1) {
            for c in 0..4 {
                sums[c] += src[(y * w + x) * 4 + c] as u32;
            }
        }
        let mut count = r.min(h - 1) + 1;
        for y in 0..h {
            for c in 0..4 {
                dst[(y * w + x) * 4 + c] = (sums[c] / count as u32) as u8;
            }
            let add = y + r + 1;
            if add < h {
                for c in 0..4 {
                    sums[c] += src[(add * w + x) * 4 + c] as u32;
                }
                count += 1;
            }
            if y >= r {
                let rem = y - r;
                for c in 0..4 {
                    sums[c] -= src[(rem * w + x) * 4 + c] as u32;
                }
                count -= 1;
            }
        }
    }
}

pub fn pixelate(pm: &mut Pixmap, size: u32) {
    if size <= 1 {
        return;
    }
    let w = pm.width();
    let h = pm.height();
    let size = size as usize;
    let data = pm.data_mut();
    for by in (0..h as usize).step_by(size) {
        for bx in (0..w as usize).step_by(size) {
            let mut sums = [0u32; 4];
            let mut n = 0u32;
            for y in by..(by + size).min(h as usize) {
                for x in bx..(bx + size).min(w as usize) {
                    let i = (y * w as usize + x) * 4;
                    for c in 0..4 {
                        sums[c] += data[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let avg = [
                (sums[0] / n) as u8,
                (sums[1] / n) as u8,
                (sums[2] / n) as u8,
                (sums[3] / n) as u8,
            ];
            for y in by..(by + size).min(h as usize) {
                for x in bx..(bx + size).min(w as usize) {
                    let i = (y * w as usize + x) * 4;
                    data[i..i + 4].copy_from_slice(&avg);
                }
            }
        }
    }
}

/// Deterministic value noise (seeded — spec §13 determinism rule).
pub fn noise(pm: &mut Pixmap, amount: f32, seed: u32) {
    let w = pm.width() as usize;
    let mut i = 0usize;
    for px in pm.pixels_mut() {
        let x = (i % w) as u32;
        let y = (i / w) as u32;
        i += 1;
        let a = px.alpha();
        if a == 0 {
            continue;
        }
        let mut n = x.wrapping_mul(374761393).wrapping_add(y.wrapping_mul(668265263)).wrapping_add(seed);
        n = (n ^ (n >> 13)).wrapping_mul(1274126177);
        let v = ((n ^ (n >> 16)) & 0xff) as f32 / 255.0 - 0.5;
        let d = (v * amount * 255.0) as i32;
        let f = |ch: u8| (ch as i32 + d).clamp(0, a as i32) as u8;
        *px = tiny_skia::PremultipliedColorU8::from_rgba(f(px.red()), f(px.green()), f(px.blue()), a)
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> Pixmap {
        let mut pm = Pixmap::new(w, h).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
        pm
    }

    #[test]
    fn invert_roundtrip() {
        let mut pm = solid(4, 4, 200, 100, 50, 255);
        invert(&mut pm);
        let p = pm.pixel(0, 0).unwrap();
        assert_eq!((p.red(), p.green(), p.blue()), (55, 155, 205));
        invert(&mut pm);
        let p = pm.pixel(0, 0).unwrap();
        assert_eq!((p.red(), p.green(), p.blue()), (200, 100, 50));
    }

    #[test]
    fn threshold_splits() {
        let mut dark = solid(2, 2, 10, 10, 10, 255);
        threshold(&mut dark, 0.5);
        assert_eq!(dark.pixel(0, 0).unwrap().red(), 0);
        let mut light = solid(2, 2, 250, 250, 250, 255);
        threshold(&mut light, 0.5);
        assert_eq!(light.pixel(0, 0).unwrap().red(), 255);
    }

    #[test]
    fn blur_spreads_energy() {
        let mut pm = Pixmap::new(9, 9).unwrap();
        // single bright pixel in the middle
        let px = tiny_skia::PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap();
        pm.pixels_mut()[4 * 9 + 4] = px;
        gaussian_blur(&mut pm, 3.0);
        let center = pm.pixel(4, 4).unwrap().red();
        let edge = pm.pixel(0, 4).unwrap().red();
        assert!(center < 255, "center spread out");
        assert!(center > edge, "energy concentrated near center");
    }

    #[test]
    fn grayscale_flattens_channels() {
        let mut pm = solid(2, 2, 255, 0, 0, 255);
        grayscale(&mut pm);
        let p = pm.pixel(0, 0).unwrap();
        assert_eq!(p.red(), p.green());
        assert_eq!(p.green(), p.blue());
    }
}
