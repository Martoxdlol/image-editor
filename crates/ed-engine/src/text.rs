//! Basic text rendering (scoped §7: single bundled Noto face, line
//! wrapping, alignment; the full rustybuzz/BiDi/itemizer stack is the
//! documented deferred upgrade path behind this same call site).


use ab_glyph::{Font, FontRef, Glyph, ScaleFont};
use ed_core::{Color, Mat3, Vec2};
use ed_document::{Document, Node};
use tiny_skia::{Mask, Pixmap, PixmapPaint, Transform};

static NOTO: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");

pub fn font() -> FontRef<'static> {
    FontRef::try_from_slice(NOTO).expect("embedded font is valid")
}

pub struct Line {
    pub text: String,
    pub width: f64,
}

/// Greedy word wrap at `max_width` (None = no wrap / auto-size).
pub fn layout_lines(text: &str, size: f64, max_width: Option<f64>) -> Vec<Line> {
    let f = font();
    let sf = f.as_scaled(size as f32);
    let measure = |s: &str| -> f64 {
        let mut w = 0.0f32;
        let mut prev: Option<ab_glyph::GlyphId> = None;
        for c in s.chars() {
            let id = f.glyph_id(c);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w as f64
    };
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        match max_width {
            None => out.push(Line { text: raw_line.to_string(), width: measure(raw_line) }),
            Some(mw) => {
                let mut cur = String::new();
                for word in raw_line.split(' ') {
                    let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                    if !cur.is_empty() && measure(&cand) > mw {
                        out.push(Line { text: cur.clone(), width: measure(&cur) });
                        cur = word.to_string();
                    } else {
                        cur = cand;
                    }
                }
                out.push(Line { text: cur.clone(), width: measure(&cur) });
            }
        }
    }
    out
}

/// Measure the natural size of a text block (auto-size boxes).
pub fn measure_text(text: &str, size: f64, line_height: f64) -> Vec2 {
    let lines = layout_lines(text, size, None);
    let w = lines.iter().map(|l| l.width).fold(0.0, f64::max);
    Vec2::new(w, lines.len() as f64 * size * line_height)
}

/// Render a Text node. Handles translation+uniform scale from `m`;
/// rotation/skew fall back to axis-aligned placement (documented v1 limit).
pub fn render_text(doc: &Document, node: &Node, canvas: &mut Pixmap, m: &Mat3, mask: Option<&Mask>) {
    let text = doc.param_str(node, "text", "");
    if text.is_empty() {
        return;
    }
    let size = doc.param_f64(node, "font-size", 24.0).max(1.0);
    let line_height = doc.param_f64(node, "line-height", 1.3);
    let align = doc.param_str(node, "align", "left");
    let color = doc.param_color(node, "fill-color", Color::BLACK);
    let x = doc.param_f64(node, "x", 0.0);
    let y = doc.param_f64(node, "y", 0.0);
    let auto = doc.param_bool(node, "auto-size", true);
    let box_w = doc.param_f64(node, "w", 200.0);

    // effective uniform scale of the view/export transform
    let scale = (m.a * m.a + m.b * m.b).sqrt().max(0.01);
    let px_size = (size * scale) as f32;
    let f = font();
    let sf = f.as_scaled(px_size);

    let lines = layout_lines(&text, size, if auto { None } else { Some(box_w) });
    let [r, g, b, _] = color.to_srgb8();
    let alpha = color.rgba[3].clamp(0.0, 1.0);

    for (li, line) in lines.iter().enumerate() {
        let line_y = y + (li as f64 + 0.8) * size * line_height; // baseline
        let offset_x = match align.as_str() {
            "center" => {
                let w = if auto { 0.0 } else { box_w };
                (w - line.width).max(0.0) / 2.0
            }
            "right" => {
                let w = if auto { 0.0 } else { box_w };
                (w - line.width).max(0.0)
            }
            _ => 0.0,
        };
        let origin = m.apply(Vec2::new(x + offset_x, line_y));
        let mut pen_x = origin.x as f32;
        let pen_y = origin.y as f32;
        let mut prev: Option<ab_glyph::GlyphId> = None;
        for c in line.text.chars() {
            let id = f.glyph_id(c);
            if let Some(p) = prev {
                pen_x += sf.kern(p, id);
            }
            let glyph: Glyph = id.with_scale_and_position(px_size, ab_glyph::point(pen_x, pen_y));
            pen_x += sf.h_advance(id);
            prev = Some(id);
            if let Some(og) = f.outline_glyph(glyph) {
                let bounds = og.px_bounds();
                let gw = bounds.width().ceil() as u32;
                let gh = bounds.height().ceil() as u32;
                if gw == 0 || gh == 0 {
                    continue;
                }
                let Some(mut gpm) = Pixmap::new(gw, gh) else { continue };
                {
                    let data = gpm.data_mut();
                    og.draw(|gx, gy, cov| {
                        let i = ((gy * gw + gx) * 4) as usize;
                        if i + 3 < data.len() {
                            let a = (cov * alpha * 255.0) as u8;
                            // premultiplied
                            data[i] = ((r as u32 * a as u32) / 255) as u8;
                            data[i + 1] = ((g as u32 * a as u32) / 255) as u8;
                            data[i + 2] = ((b as u32 * a as u32) / 255) as u8;
                            data[i + 3] = a;
                        }
                    });
                }
                canvas.draw_pixmap(
                    bounds.min.x as i32,
                    bounds.min.y as i32,
                    gpm.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    mask,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_loads_and_measures() {
        let sz = measure_text("Hello", 24.0, 1.3);
        assert!(sz.x > 20.0 && sz.x < 200.0, "width {}", sz.x);
        assert!(sz.y > 20.0);
        let wide = measure_text("Hello Hello", 24.0, 1.3);
        assert!(wide.x > sz.x);
    }

    #[test]
    fn wrapping_produces_lines() {
        let one = layout_lines("aaa bbb ccc", 20.0, None);
        assert_eq!(one.len(), 1);
        let wrapped = layout_lines("aaa bbb ccc", 20.0, Some(40.0));
        assert!(wrapped.len() >= 2, "got {} lines", wrapped.len());
        let newlines = layout_lines("a\nb\nc", 20.0, None);
        assert_eq!(newlines.len(), 3);
    }
}
