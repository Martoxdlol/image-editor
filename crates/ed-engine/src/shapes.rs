//! Parametric shape → path building (spec §6.2) and a minimal SVG path
//! data parser (absolute M/L/C/Q/Z — the subset our tools emit).

use ed_core::Vec2;
use ed_document::{Document, Node};
use tiny_skia::{Path, PathBuilder};

/// Build the outline path for a Shape node, in document coordinates.
pub fn shape_path(doc: &Document, node: &Node) -> Option<Path> {
    let x = doc.param_f64(node, "x", 0.0);
    let y = doc.param_f64(node, "y", 0.0);
    let w = doc.param_f64(node, "w", 0.0);
    let h = doc.param_f64(node, "h", 0.0);
    let kind = doc.param_str(node, "shape", "rect");
    let mut pb = PathBuilder::new();
    match kind.as_str() {
        "ellipse" => {
            pb.push_oval(tiny_skia::Rect::from_xywh(x as f32, y as f32, w.max(0.01) as f32, h.max(0.01) as f32)?);
        }
        "polygon" | "star" => {
            let sides = doc.param_f64(node, "sides", if kind == "star" { 5.0 } else { 6.0 }).max(3.0) as usize;
            let cx = x + w / 2.0;
            let cy = y + h / 2.0;
            let rx = w / 2.0;
            let ry = h / 2.0;
            let inner = doc.param_f64(node, "inner-radius", 0.5).clamp(0.05, 1.0);
            let steps = if kind == "star" { sides * 2 } else { sides };
            for i in 0..steps {
                let frac = i as f64 / steps as f64;
                let ang = frac * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
                let (r_x, r_y) = if kind == "star" && i % 2 == 1 {
                    (rx * inner, ry * inner)
                } else {
                    (rx, ry)
                };
                let px = (cx + ang.cos() * r_x) as f32;
                let py = (cy + ang.sin() * r_y) as f32;
                if i == 0 {
                    pb.move_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
            pb.close();
        }
        "line" | "arrow" => {
            let x2 = doc.param_f64(node, "x2", x + w);
            let y2 = doc.param_f64(node, "y2", y + h);
            pb.move_to(x as f32, y as f32);
            pb.line_to(x2 as f32, y2 as f32);
            if kind == "arrow" {
                let dir = Vec2::new(x2 - x, y2 - y);
                let len = dir.length().max(1e-6);
                let u = Vec2::new(dir.x / len, dir.y / len);
                let n = Vec2::new(-u.y, u.x);
                let head = doc.param_f64(node, "head-size", 12.0);
                let tip = Vec2::new(x2, y2);
                let b1 = tip - u * head + n * (head * 0.5);
                let b2 = tip - u * head - n * (head * 0.5);
                pb.move_to(b1.x as f32, b1.y as f32);
                pb.line_to(tip.x as f32, tip.y as f32);
                pb.line_to(b2.x as f32, b2.y as f32);
            }
        }
        // default: rect with optional uniform corner radius
        _ => {
            let r = doc.param_f64(node, "radius", 0.0).clamp(0.0, w.min(h) / 2.0);
            let rect = tiny_skia::Rect::from_xywh(x as f32, y as f32, w.max(0.01) as f32, h.max(0.01) as f32)?;
            if r <= 0.0 {
                pb.push_rect(rect);
            } else {
                let (x, y, w, h, r) = (x as f32, y as f32, w as f32, h as f32, r as f32);
                // rounded rect via kappa arcs
                let k = 0.5523f32 * r;
                pb.move_to(x + r, y);
                pb.line_to(x + w - r, y);
                pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
                pb.line_to(x + w, y + h - r);
                pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
                pb.line_to(x + r, y + h);
                pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
                pb.line_to(x, y + r);
                pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
                pb.close();
            }
        }
    }
    pb.finish()
}

/// Parse SVG path data (absolute M, L, C, Q, Z — what the pen tool writes).
pub fn parse_path_data(d: &str) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut nums: Vec<f32> = Vec::new();
    let mut cmd = ' ';
    let mut it = d.chars().peekable();

    fn flush(pb: &mut PathBuilder, cmd: char, nums: &mut Vec<f32>) {
        let mut i = 0;
        match cmd {
            'M' => {
                while i + 1 < nums.len() + 1 && nums.len() - i >= 2 {
                    if i == 0 {
                        pb.move_to(nums[i], nums[i + 1]);
                    } else {
                        pb.line_to(nums[i], nums[i + 1]);
                    }
                    i += 2;
                }
            }
            'L' => {
                while nums.len() - i >= 2 {
                    pb.line_to(nums[i], nums[i + 1]);
                    i += 2;
                }
            }
            'Q' => {
                while nums.len() - i >= 4 {
                    pb.quad_to(nums[i], nums[i + 1], nums[i + 2], nums[i + 3]);
                    i += 4;
                }
            }
            'C' => {
                while nums.len() - i >= 6 {
                    pb.cubic_to(nums[i], nums[i + 1], nums[i + 2], nums[i + 3], nums[i + 4], nums[i + 5]);
                    i += 6;
                }
            }
            _ => {}
        }
        nums.clear();
    }

    while let Some(&c) = it.peek() {
        match c {
            'M' | 'L' | 'C' | 'Q' | 'Z' | 'z' => {
                flush(&mut pb, cmd, &mut nums);
                if c == 'Z' || c == 'z' {
                    pb.close();
                    cmd = ' ';
                } else {
                    cmd = c;
                }
                it.next();
            }
            c if c.is_ascii_digit() || c == '-' || c == '.' || c == '+' => {
                let mut s = String::new();
                while let Some(&c2) = it.peek() {
                    if c2.is_ascii_digit() || c2 == '.' || ((c2 == '-' || c2 == '+') && s.is_empty()) || c2 == 'e' || c2 == 'E'
                    {
                        s.push(c2);
                        it.next();
                    } else {
                        break;
                    }
                }
                nums.push(s.parse().ok()?);
            }
            _ => {
                it.next();
            }
        }
    }
    flush(&mut pb, cmd, &mut nums);
    pb.finish()
}

/// Serialize points into path data (used by pen/lasso tools).
pub fn polyline_to_path_data(points: &[Vec2], close: bool) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let c = if i == 0 { 'M' } else { 'L' };
        d.push_str(&format!("{c}{:.2} {:.2} ", p.x, p.y));
    }
    if close {
        d.push('Z');
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generated_path_data() {
        let pts = vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        let d = polyline_to_path_data(&pts, true);
        let path = parse_path_data(&d).unwrap();
        let b = path.bounds();
        assert_eq!((b.left(), b.top(), b.right(), b.bottom()), (0.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn parses_curves() {
        let path = parse_path_data("M0 0 C10 0 10 10 20 10 Q30 10 30 0 Z").unwrap();
        assert!(path.bounds().right() >= 30.0);
    }
}
