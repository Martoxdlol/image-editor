//! Pixel selections as first-class objects (spec §2.3): geometry +
//! feather + boolean combine mode; rasterizable to a coverage mask.

use ed_core::{Rect, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CombineMode {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
    Xor,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SelGeom {
    Rect { rect: Rect },
    Ellipse { rect: Rect },
    Polygon { points: Vec<Vec2> },
    /// Magic wand / color-range result: a document-space coverage bitmap.
    Mask { x: i64, y: i64, w: u32, h: u32, #[serde(skip)] data: Vec<u8> },
}

impl SelGeom {
    pub fn bounds(&self) -> Rect {
        match self {
            SelGeom::Rect { rect } | SelGeom::Ellipse { rect } => *rect,
            SelGeom::Polygon { points } => {
                let mut min = Vec2::new(f64::MAX, f64::MAX);
                let mut max = Vec2::new(f64::MIN, f64::MIN);
                for p in points {
                    min.x = min.x.min(p.x);
                    min.y = min.y.min(p.y);
                    max.x = max.x.max(p.x);
                    max.y = max.y.max(p.y);
                }
                Rect::new(min.x, min.y, (max.x - min.x).max(0.0), (max.y - min.y).max(0.0))
            }
            SelGeom::Mask { x, y, w, h, .. } => Rect::new(*x as f64, *y as f64, *w as f64, *h as f64),
        }
    }

    /// Coverage at a document-space point, 0..=1 (before feather).
    pub fn coverage(&self, p: Vec2) -> f64 {
        match self {
            SelGeom::Rect { rect } => {
                if rect.contains(p) {
                    1.0
                } else {
                    0.0
                }
            }
            SelGeom::Ellipse { rect } => {
                if rect.w <= 0.0 || rect.h <= 0.0 {
                    return 0.0;
                }
                let c = rect.center();
                let dx = (p.x - c.x) / (rect.w / 2.0);
                let dy = (p.y - c.y) / (rect.h / 2.0);
                if dx * dx + dy * dy <= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            SelGeom::Polygon { points } => {
                if points.len() < 3 {
                    return 0.0;
                }
                // even-odd rule
                let mut inside = false;
                let mut j = points.len() - 1;
                for i in 0..points.len() {
                    let (a, b) = (points[i], points[j]);
                    if (a.y > p.y) != (b.y > p.y)
                        && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
                    {
                        inside = !inside;
                    }
                    j = i;
                }
                if inside {
                    1.0
                } else {
                    0.0
                }
            }
            SelGeom::Mask { x, y, w, h, data } => {
                let ix = p.x.floor() as i64 - x;
                let iy = p.y.floor() as i64 - y;
                if ix < 0 || iy < 0 || ix >= *w as i64 || iy >= *h as i64 {
                    return 0.0;
                }
                data.get((iy as usize) * (*w as usize) + ix as usize)
                    .map(|&v| v as f64 / 255.0)
                    .unwrap_or(0.0)
            }
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct SelShape {
    pub geom: SelGeom,
    pub mode: CombineMode,
}

/// The active (transient) pixel selection: an ordered boolean combination
/// of shapes, plus feather (spec §2.3).
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
pub struct PixelSelection {
    pub shapes: Vec<SelShape>,
    pub feather: f64,
    pub inverted: bool,
}

impl PixelSelection {
    pub fn single(geom: SelGeom) -> Self {
        PixelSelection {
            shapes: vec![SelShape { geom, mode: CombineMode::Replace }],
            feather: 0.0,
            inverted: false,
        }
    }

    pub fn combine(&mut self, geom: SelGeom, mode: CombineMode) {
        if mode == CombineMode::Replace {
            self.shapes.clear();
            self.inverted = false;
        }
        self.shapes.push(SelShape { geom, mode });
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    pub fn bounds(&self) -> Rect {
        let mut r = Rect::default();
        for s in &self.shapes {
            if s.mode != CombineMode::Subtract {
                r = r.union(&s.geom.bounds());
            }
        }
        r.inflate(self.feather.max(0.0))
    }

    /// Combined coverage at a point (0..=1), before feather blur.
    pub fn coverage(&self, p: Vec2) -> f64 {
        let mut acc: f64 = 0.0;
        for s in &self.shapes {
            let c = s.geom.coverage(p);
            acc = match s.mode {
                CombineMode::Replace => c,
                CombineMode::Add => acc.max(c),
                CombineMode::Subtract => (acc - c).max(0.0),
                CombineMode::Intersect => acc.min(c),
                CombineMode::Xor => (acc - c).abs(),
            };
        }
        if self.inverted {
            1.0 - acc
        } else {
            acc
        }
    }

    /// Rasterize the selection to an 8-bit coverage mask over an integer
    /// pixel region. Applies feather as a separable box blur.
    pub fn rasterize(&self, x0: i64, y0: i64, w: u32, h: u32) -> Vec<u8> {
        let mut mask = vec![0u8; (w as usize) * (h as usize)];
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let p = Vec2::new((x0 + x) as f64 + 0.5, (y0 + y) as f64 + 0.5);
                mask[(y as usize) * (w as usize) + (x as usize)] =
                    (self.coverage(p) * 255.0).round() as u8;
            }
        }
        let r = self.feather.round() as usize;
        if r > 0 {
            mask = box_blur_mask(&mask, w as usize, h as usize, r);
        }
        mask
    }
}

fn box_blur_mask(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let mut tmp = vec![0u8; src.len()];
    let mut out = vec![0u8; src.len()];
    let win = 2 * r + 1;
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let mut sum: u32 = 0;
        for x in 0..w.min(win) {
            sum += row[x] as u32;
        }
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r + 1).min(w);
            if x > 0 {
                let prev_lo = (x - 1).saturating_sub(r);
                let prev_hi = (x + r).min(w);
                if prev_lo < lo {
                    sum -= row[prev_lo] as u32;
                }
                if prev_hi < hi {
                    sum += row[prev_hi] as u32;
                }
            }
            tmp[y * w + x] = (sum / (hi - lo) as u32) as u8;
        }
    }
    for x in 0..w {
        for y in 0..h {
            let lo = y.saturating_sub(r);
            let hi = (y + r + 1).min(h);
            let mut sum: u32 = 0;
            for yy in lo..hi {
                sum += tmp[yy * w + x] as u32;
            }
            out[y * w + x] = (sum / (hi - lo) as u32) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_coverage_and_combine() {
        let mut sel = PixelSelection::single(SelGeom::Rect { rect: Rect::new(0.0, 0.0, 10.0, 10.0) });
        assert_eq!(sel.coverage(Vec2::new(5.0, 5.0)), 1.0);
        assert_eq!(sel.coverage(Vec2::new(15.0, 5.0)), 0.0);

        sel.combine(SelGeom::Rect { rect: Rect::new(20.0, 0.0, 10.0, 10.0) }, CombineMode::Add);
        assert_eq!(sel.coverage(Vec2::new(25.0, 5.0)), 1.0);

        sel.combine(SelGeom::Rect { rect: Rect::new(0.0, 0.0, 5.0, 10.0) }, CombineMode::Subtract);
        assert_eq!(sel.coverage(Vec2::new(2.0, 5.0)), 0.0);
        assert_eq!(sel.coverage(Vec2::new(7.0, 5.0)), 1.0);
    }

    #[test]
    fn ellipse_and_polygon() {
        let e = SelGeom::Ellipse { rect: Rect::new(0.0, 0.0, 10.0, 10.0) };
        assert_eq!(e.coverage(Vec2::new(5.0, 5.0)), 1.0);
        assert_eq!(e.coverage(Vec2::new(0.5, 0.5)), 0.0); // corner outside circle

        let p = SelGeom::Polygon {
            points: vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(5.0, 10.0)],
        };
        assert_eq!(p.coverage(Vec2::new(5.0, 3.0)), 1.0);
        assert_eq!(p.coverage(Vec2::new(0.0, 9.0)), 0.0);
    }

    #[test]
    fn invert() {
        let mut sel = PixelSelection::single(SelGeom::Rect { rect: Rect::new(0.0, 0.0, 10.0, 10.0) });
        sel.inverted = true;
        assert_eq!(sel.coverage(Vec2::new(5.0, 5.0)), 0.0);
        assert_eq!(sel.coverage(Vec2::new(50.0, 5.0)), 1.0);
    }
}
