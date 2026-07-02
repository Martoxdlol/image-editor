use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn distance(self, other: Vec2) -> f64 {
        (self - other).length()
    }

    pub fn lerp(self, other: Vec2, t: f64) -> Vec2 {
        Vec2::new(self.x + (other.x - self.x) * t, self.y + (other.y - self.y) * t)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl std::ops::Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

/// Row-major 2D affine matrix: [a c e; b d f; 0 0 1].
/// Maps (x, y) → (a·x + c·y + e, b·x + d·y + f).
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug)]
pub struct Mat3 {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Mat3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(t: Vec2) -> Mat3 {
        Mat3 { e: t.x, f: t.y, ..Mat3::IDENTITY }
    }

    pub fn scale(sx: f64, sy: f64) -> Mat3 {
        Mat3 { a: sx, d: sy, ..Mat3::IDENTITY }
    }

    pub fn rotate(radians: f64) -> Mat3 {
        let (s, c) = radians.sin_cos();
        Mat3 { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    pub fn skew(sx: f64, sy: f64) -> Mat3 {
        Mat3 { a: 1.0, b: sy.tan(), c: sx.tan(), d: 1.0, e: 0.0, f: 0.0 }
    }

    /// self ∘ other — applies `other` first, then `self`.
    pub fn mul(&self, o: &Mat3) -> Mat3 {
        Mat3 {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }

    pub fn apply(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }

    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    pub fn invert(&self) -> Option<Mat3> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        Some(Mat3 {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            e: (self.c * self.f - self.d * self.e) * inv,
            f: (self.b * self.e - self.a * self.f) * inv,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_points(a: Vec2, b: Vec2) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        Self { x, y, w: (a.x - b.x).abs(), h: (a.y - b.y).abs() }
    }

    pub fn min(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    pub fn max(&self) -> Vec2 {
        Vec2::new(self.x + self.w, self.y + self.h)
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.y >= self.y && p.x <= self.x + self.w && p.y <= self.y + self.h
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.w <= 0.0 && self.h <= 0.0 {
            return *o;
        }
        if o.w <= 0.0 && o.h <= 0.0 {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let x2 = (self.x + self.w).max(o.x + o.w);
        let y2 = (self.y + self.h).max(o.y + o.h);
        Rect::new(x, y, x2 - x, y2 - y)
    }

    pub fn inflate(&self, d: f64) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }

    /// Axis-aligned bounds of this rect under an affine transform.
    pub fn transform(&self, m: &Mat3) -> Rect {
        let ps = [
            m.apply(self.min()),
            m.apply(Vec2::new(self.x + self.w, self.y)),
            m.apply(Vec2::new(self.x, self.y + self.h)),
            m.apply(self.max()),
        ];
        let mut min = ps[0];
        let mut max = ps[0];
        for p in &ps[1..] {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        Rect::new(min.x, min.y, max.x - min.x, max.y - min.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_compose_and_invert() {
        let m = Mat3::translate(Vec2::new(10.0, 5.0)).mul(&Mat3::scale(2.0, 2.0));
        let p = m.apply(Vec2::new(3.0, 4.0));
        assert_eq!(p, Vec2::new(16.0, 13.0));
        let inv = m.invert().unwrap();
        let back = inv.apply(p);
        assert!((back.x - 3.0).abs() < 1e-9 && (back.y - 4.0).abs() < 1e-9);
    }

    #[test]
    fn rotation_direction() {
        let m = Mat3::rotate(std::f64::consts::FRAC_PI_2);
        let p = m.apply(Vec2::new(1.0, 0.0));
        assert!((p.x).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rect_ops() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Vec2::new(5.0, 5.0)));
        assert!(r.intersects(&Rect::new(9.0, 9.0, 5.0, 5.0)));
        assert!(!r.intersects(&Rect::new(11.0, 0.0, 5.0, 5.0)));
        let u = r.union(&Rect::new(-5.0, 2.0, 3.0, 3.0));
        assert_eq!(u, Rect::new(-5.0, 0.0, 15.0, 10.0));
    }
}
