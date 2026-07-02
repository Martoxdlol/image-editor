//! Raster kernels: brush stamping into tile-based bitmaps (spec §2.5,
//! §6.1), stroke replay for StrokeSet nodes, flood fill and magic wand.
//! Deterministic: strokes replayed anywhere produce identical tiles (§3.2).

use ed_core::Vec2;
use ed_document::{BitmapData, Stroke};
use tiny_skia::Pixmap;

/// Brush parameters shared by brush/pencil/eraser (spec §6.1).
#[derive(Clone, Copy, Debug)]
pub struct BrushParams {
    pub size: f64,
    pub hardness: f64,
    pub opacity: f64,
    pub flow: f64,
    pub erase: bool,
    /// Pencil mode: hard 1px-accurate stamp, no anti-alias.
    pub pixel_perfect: bool,
}

/// Coverage of a soft round brush at distance `d` from center (radius r).
fn brush_coverage(d: f64, r: f64, hardness: f64) -> f64 {
    if d >= r {
        return 0.0;
    }
    let hard_r = r * hardness.clamp(0.0, 1.0);
    if d <= hard_r {
        1.0
    } else {
        let t = (d - hard_r) / (r - hard_r).max(1e-6);
        // smoothstep falloff
        1.0 - t * t * (3.0 - 2.0 * t)
    }
}

/// Stamp one dab. `color` is straight sRGB8; bitmap stores straight RGBA8.
fn stamp(bm: &mut BitmapData, center: Vec2, params: &BrushParams, color: [u8; 4], mask: Option<&SelMask>) {
    let r = (params.size / 2.0).max(0.1);
    let x0 = ((center.x - r).floor().max(0.0)) as u32;
    let y0 = ((center.y - r).floor().max(0.0)) as u32;
    let x1 = ((center.x + r).ceil().min(bm.width as f64)) as u32;
    let y1 = ((center.y + r).ceil().min(bm.height as f64)) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let d = Vec2::new(x as f64 + 0.5, y as f64 + 0.5).distance(center);
            let mut cov = if params.pixel_perfect {
                if d <= r {
                    1.0
                } else {
                    0.0
                }
            } else {
                brush_coverage(d, r, params.hardness)
            };
            if cov <= 0.0 {
                continue;
            }
            if let Some(m) = mask {
                cov *= m.at(x, y);
                if cov <= 0.0 {
                    continue;
                }
            }
            let a = (cov * params.flow * params.opacity).clamp(0.0, 1.0);
            let dst = bm.get_pixel(x, y);
            let out = if params.erase {
                let keep = 1.0 - a;
                [
                    dst[0],
                    dst[1],
                    dst[2],
                    (dst[3] as f64 * keep) as u8,
                ]
            } else {
                blend_straight(dst, color, a)
            };
            bm.set_pixel(x, y, out);
        }
    }
}

/// Source-over blend on straight-alpha u8.
fn blend_straight(dst: [u8; 4], src: [u8; 4], src_a: f64) -> [u8; 4] {
    let sa = src_a * (src[3] as f64 / 255.0);
    let da = dst[3] as f64 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return [0, 0, 0, 0];
    }
    let f = |s: u8, d: u8| {
        let sv = s as f64 / 255.0;
        let dv = d as f64 / 255.0;
        (((sv * sa + dv * da * (1.0 - sa)) / out_a) * 255.0 + 0.5) as u8
    };
    [f(src[0], dst[0]), f(src[1], dst[1]), f(src[2], dst[2]), (out_a * 255.0 + 0.5) as u8]
}

/// Rasterized selection mask limiting paint (spec §2.3 as scope).
pub struct SelMask {
    pub x0: i64,
    pub y0: i64,
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl SelMask {
    fn at(&self, x: u32, y: u32) -> f64 {
        let ix = x as i64 - self.x0;
        let iy = y as i64 - self.y0;
        if ix < 0 || iy < 0 || ix >= self.w as i64 || iy >= self.h as i64 {
            return 0.0;
        }
        self.data[(iy as usize) * (self.w as usize) + ix as usize] as f64 / 255.0
    }
}

/// Stamp a segment of a stroke with spacing proportional to size.
/// `last` is the distance already covered past the previous stamp; returns
/// the carry-over so consecutive segments space evenly.
pub fn stroke_segment(
    bm: &mut BitmapData,
    from: Vec2,
    to: Vec2,
    params: &BrushParams,
    color: [u8; 4],
    carry: f64,
    mask: Option<&SelMask>,
) -> f64 {
    let spacing = (params.size * 0.25).max(if params.pixel_perfect { 1.0 } else { 0.5 });
    let dist = from.distance(to);
    if dist <= 0.0 {
        stamp(bm, to, params, color, mask);
        return carry;
    }
    let mut t = carry;
    while t <= dist {
        stamp(bm, from.lerp(to, t / dist), params, color, mask);
        t += spacing;
    }
    t - dist
}

/// Replay a full stroke (StrokeSet render + deterministic replay §3.2).
pub fn replay_stroke(bm: &mut BitmapData, stroke: &Stroke, offset: Vec2) {
    let params = BrushParams {
        size: stroke.size,
        hardness: stroke.hardness,
        opacity: stroke.opacity,
        flow: 1.0,
        erase: stroke.erase,
        pixel_perfect: false,
    };
    let color = stroke.color.to_srgb8();
    let mut carry = 0.0;
    let mut prev: Option<Vec2> = None;
    for p in &stroke.points {
        let mut params = params;
        params.size = stroke.size * p.pressure.max(0.05);
        let pos = p.pos - offset;
        match prev {
            None => {
                stamp(bm, pos, &params, color, None);
            }
            Some(fr) => {
                carry = stroke_segment(bm, fr, pos, &params, color, carry, None);
            }
        }
        prev = Some(pos);
    }
}

/// Convert a bitmap (straight RGBA8) into a premultiplied Pixmap for
/// compositing.
pub fn bitmap_to_pixmap(bm: &BitmapData) -> Option<Pixmap> {
    if bm.width == 0 || bm.height == 0 {
        return None;
    }
    let straight = bm.to_rgba();
    let mut pm = Pixmap::new(bm.width, bm.height)?;
    let data = pm.data_mut();
    for (i, chunk) in straight.chunks_exact(4).enumerate() {
        let a = chunk[3] as u32;
        data[i * 4] = ((chunk[0] as u32 * a + 127) / 255) as u8;
        data[i * 4 + 1] = ((chunk[1] as u32 * a + 127) / 255) as u8;
        data[i * 4 + 2] = ((chunk[2] as u32 * a + 127) / 255) as u8;
        data[i * 4 + 3] = a as u8;
    }
    Some(pm)
}

fn color_distance(a: [u8; 4], b: [u8; 4]) -> f64 {
    let dr = a[0] as f64 - b[0] as f64;
    let dg = a[1] as f64 - b[1] as f64;
    let db = a[2] as f64 - b[2] as f64;
    let da = a[3] as f64 - b[3] as f64;
    ((dr * dr + dg * dg + db * db + da * da) / 4.0).sqrt() / 255.0
}

/// Flood fill (bucket, spec §6.1). Returns true if anything changed.
pub fn flood_fill(
    bm: &mut BitmapData,
    seed: (u32, u32),
    color: [u8; 4],
    tolerance: f64,
    contiguous: bool,
) -> bool {
    if seed.0 >= bm.width || seed.1 >= bm.height {
        return false;
    }
    let target = bm.get_pixel(seed.0, seed.1);
    if target == color && tolerance <= 0.0 {
        return false;
    }
    let mut changed = false;
    if contiguous {
        let mut visited = vec![false; (bm.width * bm.height) as usize];
        let mut stack = vec![seed];
        while let Some((x, y)) = stack.pop() {
            let idx = (y * bm.width + x) as usize;
            if visited[idx] {
                continue;
            }
            visited[idx] = true;
            if color_distance(bm.get_pixel(x, y), target) > tolerance {
                continue;
            }
            bm.set_pixel(x, y, color);
            changed = true;
            if x > 0 {
                stack.push((x - 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if x + 1 < bm.width {
                stack.push((x + 1, y));
            }
            if y + 1 < bm.height {
                stack.push((x, y + 1));
            }
        }
    } else {
        for y in 0..bm.height {
            for x in 0..bm.width {
                if color_distance(bm.get_pixel(x, y), target) <= tolerance {
                    bm.set_pixel(x, y, color);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Magic wand (spec §6.3): coverage mask of pixels similar to the seed,
/// over an RGBA8 sample buffer (layer or composite).
pub fn magic_wand(
    rgba: &[u8],
    w: u32,
    h: u32,
    seed: (u32, u32),
    tolerance: f64,
    contiguous: bool,
) -> Vec<u8> {
    let mut mask = vec![0u8; (w * h) as usize];
    if seed.0 >= w || seed.1 >= h {
        return mask;
    }
    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    let target = px(seed.0, seed.1);
    if contiguous {
        let mut stack = vec![seed];
        while let Some((x, y)) = stack.pop() {
            let idx = (y * w + x) as usize;
            if mask[idx] != 0 {
                continue;
            }
            if color_distance(px(x, y), target) > tolerance {
                continue;
            }
            mask[idx] = 255;
            if x > 0 {
                stack.push((x - 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if x + 1 < w {
                stack.push((x + 1, y));
            }
            if y + 1 < h {
                stack.push((x, y + 1));
            }
        }
        // second pass: unmark the "visited but rejected" cells — they were
        // never set, so nothing to do (mask only holds accepted pixels).
    } else {
        for y in 0..h {
            for x in 0..w {
                if color_distance(px(x, y), target) <= tolerance {
                    mask[(y * w + x) as usize] = 255;
                }
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed_core::Color;
    use ed_document::StrokePoint;

    #[test]
    fn stamp_paints_and_erases() {
        let mut bm = BitmapData::new(64, 64);
        let p = BrushParams { size: 10.0, hardness: 1.0, opacity: 1.0, flow: 1.0, erase: false, pixel_perfect: false };
        stamp(&mut bm, Vec2::new(32.0, 32.0), &p, [255, 0, 0, 255], None);
        assert_eq!(bm.get_pixel(32, 32), [255, 0, 0, 255]);
        assert_eq!(bm.get_pixel(0, 0), [0, 0, 0, 0]);

        let e = BrushParams { erase: true, ..p };
        stamp(&mut bm, Vec2::new(32.0, 32.0), &e, [0, 0, 0, 0], None);
        assert_eq!(bm.get_pixel(32, 32)[3], 0, "eraser clears alpha");
    }

    #[test]
    fn soft_brush_feathers() {
        let mut bm = BitmapData::new(64, 64);
        let p = BrushParams { size: 20.0, hardness: 0.0, opacity: 1.0, flow: 1.0, erase: false, pixel_perfect: false };
        stamp(&mut bm, Vec2::new(32.0, 32.0), &p, [0, 0, 0, 255], None);
        let center = bm.get_pixel(32, 32)[3];
        let edge = bm.get_pixel(40, 32)[3];
        assert!(center > 200);
        assert!(edge < center, "soft edge falls off ({edge} < {center})");
    }

    #[test]
    fn stroke_replay_deterministic() {
        let stroke = Stroke {
            color: Color::from_hex("#123456").unwrap(),
            size: 8.0,
            hardness: 0.5,
            opacity: 0.9,
            erase: false,
            points: vec![
                StrokePoint { pos: Vec2::new(5.0, 5.0), pressure: 1.0 },
                StrokePoint { pos: Vec2::new(50.0, 20.0), pressure: 0.7 },
                StrokePoint { pos: Vec2::new(20.0, 55.0), pressure: 1.0 },
            ],
        };
        let mut a = BitmapData::new(64, 64);
        let mut b = BitmapData::new(64, 64);
        replay_stroke(&mut a, &stroke, Vec2::ZERO);
        replay_stroke(&mut b, &stroke, Vec2::ZERO);
        assert_eq!(a.to_rgba(), b.to_rgba(), "replay is deterministic");
        assert!(a.to_rgba().iter().any(|&v| v != 0));
    }

    #[test]
    fn flood_fill_contiguous_respects_boundary() {
        let mut bm = BitmapData::new(16, 16);
        // vertical wall at x=8
        for y in 0..16 {
            bm.set_pixel(8, y, [0, 0, 0, 255]);
        }
        flood_fill(&mut bm, (2, 2), [0, 255, 0, 255], 0.05, true);
        assert_eq!(bm.get_pixel(2, 2), [0, 255, 0, 255]);
        assert_eq!(bm.get_pixel(12, 2), [0, 0, 0, 0], "wall blocked the fill");
        assert_eq!(bm.get_pixel(8, 2), [0, 0, 0, 255], "wall untouched");
    }

    #[test]
    fn wand_selects_similar_region() {
        let mut rgba = vec![0u8; 16 * 16 * 4];
        // left half red, right half blue
        for y in 0..16 {
            for x in 0..16 {
                let i = (y * 16 + x) * 4;
                if x < 8 {
                    rgba[i] = 255;
                } else {
                    rgba[i + 2] = 255;
                }
                rgba[i + 3] = 255;
            }
        }
        let mask = magic_wand(&rgba, 16, 16, (2, 2), 0.1, true);
        assert_eq!(mask[2 * 16 + 2], 255);
        assert_eq!(mask[2 * 16 + 12], 0);
    }
}
