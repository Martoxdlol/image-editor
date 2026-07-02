//! Paint construction from node params (spec §15: Paint = Solid | Gradient
//! | Pattern | None) plus blend-mode mapping.

use ed_core::{BlendMode, Color};
use ed_document::{Document, Node};
use tiny_skia::{
    GradientStop, LinearGradient, Paint, Point, RadialGradient, Shader, SpreadMode, Transform,
};

pub fn to_sk_color(c: Color) -> tiny_skia::Color {
    let [r, g, b, a] = c.to_srgb8();
    tiny_skia::Color::from_rgba8(r, g, b, a)
}

pub fn to_sk_blend(b: BlendMode) -> tiny_skia::BlendMode {
    use tiny_skia::BlendMode as SB;
    match b {
        BlendMode::Normal => SB::SourceOver,
        BlendMode::Multiply => SB::Multiply,
        BlendMode::Screen => SB::Screen,
        BlendMode::Overlay => SB::Overlay,
        BlendMode::Darken => SB::Darken,
        BlendMode::Lighten => SB::Lighten,
        BlendMode::ColorDodge => SB::ColorDodge,
        BlendMode::ColorBurn => SB::ColorBurn,
        BlendMode::HardLight => SB::HardLight,
        BlendMode::SoftLight => SB::SoftLight,
        BlendMode::Difference => SB::Difference,
        BlendMode::Exclusion => SB::Exclusion,
        BlendMode::Hue => SB::Hue,
        BlendMode::Saturation => SB::Saturation,
        BlendMode::Color => SB::Color,
        BlendMode::Luminosity => SB::Luminosity,
        BlendMode::Add => SB::Plus,
    }
}

/// Gradient stops parsed from the `stops` param (JSON array of
/// `{pos, color}` hex strings) or the from/to color pair.
fn gradient_stops(doc: &Document, node: &Node, prefix: &str) -> Vec<GradientStop> {
    let stops_json = doc.param_str(node, &format!("{prefix}stops"), "");
    if !stops_json.is_empty() {
        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&stops_json) {
            let mut parsed: Vec<(f32, Color)> = list
                .iter()
                .filter_map(|s| {
                    let pos = s.get("pos")?.as_f64()? as f32;
                    let color = Color::from_hex(s.get("color")?.as_str()?)?;
                    Some((pos.clamp(0.0, 1.0), color))
                })
                .collect();
            if parsed.len() >= 2 {
                parsed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                return parsed
                    .into_iter()
                    .map(|(pos, c)| GradientStop::new(pos, to_sk_color(c)))
                    .collect();
            }
        }
    }
    let from = doc.param_color(node, &format!("{prefix}from-color"), Color::BLACK);
    let to = doc.param_color(node, &format!("{prefix}to-color"), Color::WHITE);
    vec![GradientStop::new(0.0, to_sk_color(from)), GradientStop::new(1.0, to_sk_color(to))]
}

/// Build a shader for a gradient described by node params under `prefix`
/// (`from`/`to` points in doc coords, `gradient` kind).
pub fn gradient_shader(
    doc: &Document,
    node: &Node,
    prefix: &str,
    doc_to_screen: Transform,
) -> Option<Shader<'static>> {
    let from = node
        .get_param(&format!("{prefix}from"))
        .map(|v| doc.resolve(&v))
        .and_then(|v| v.as_point())
        .unwrap_or(ed_core::Vec2::new(0.0, 0.0));
    let to = node
        .get_param(&format!("{prefix}to"))
        .map(|v| doc.resolve(&v))
        .and_then(|v| v.as_point())
        .unwrap_or(ed_core::Vec2::new(100.0, 0.0));
    let stops = gradient_stops(doc, node, prefix);
    let kind = doc.param_str(node, &format!("{prefix}gradient"), "linear");
    let p0 = Point::from_xy(from.x as f32, from.y as f32);
    let p1 = Point::from_xy(to.x as f32, to.y as f32);
    match kind.as_str() {
        "radial" => RadialGradient::new(
            p0,
            p0,
            ((to.x - from.x).hypot(to.y - from.y) as f32).max(0.01),
            stops,
            SpreadMode::Pad,
            doc_to_screen,
        ),
        "reflected" => LinearGradient::new(p0, p1, stops, SpreadMode::Reflect, doc_to_screen),
        _ => LinearGradient::new(p0, p1, stops, SpreadMode::Pad, doc_to_screen),
    }
}

/// Fill paint for a node, honoring `fill` = solid|gradient|none.
pub fn fill_paint<'a>(
    doc: &Document,
    node: &Node,
    doc_to_screen: Transform,
) -> Option<Paint<'a>> {
    let kind = doc.param_str(node, "fill", "solid");
    let mut paint = Paint::default();
    paint.anti_alias = true;
    match kind.as_str() {
        "none" => return None,
        "gradient" => {
            paint.shader = gradient_shader(doc, node, "fill-", doc_to_screen)?;
        }
        _ => {
            let c = doc.param_color(node, "fill-color", Color::from_hex("#cccccc").unwrap());
            paint.set_color(to_sk_color(c));
        }
    }
    Some(paint)
}

/// Stroke paint + stroke geometry params (spec §6.2).
pub fn stroke_paint<'a>(
    doc: &Document,
    node: &Node,
    zoom: f32,
) -> Option<(Paint<'a>, tiny_skia::Stroke)> {
    let width = doc.param_f64(node, "stroke-width", 0.0);
    if width <= 0.0 {
        return None;
    }
    let kind = doc.param_str(node, "stroke", "solid");
    if kind == "none" {
        return None;
    }
    let mut paint = Paint::default();
    paint.anti_alias = true;
    let c = doc.param_color(node, "stroke-color", Color::BLACK);
    paint.set_color(to_sk_color(c));
    let mut stroke = tiny_skia::Stroke {
        width: (width as f32) * zoom,
        ..Default::default()
    };
    stroke.line_cap = match doc.param_str(node, "stroke-cap", "butt").as_str() {
        "round" => tiny_skia::LineCap::Round,
        "square" => tiny_skia::LineCap::Square,
        _ => tiny_skia::LineCap::Butt,
    };
    stroke.line_join = match doc.param_str(node, "stroke-join", "miter").as_str() {
        "round" => tiny_skia::LineJoin::Round,
        "bevel" => tiny_skia::LineJoin::Bevel,
        _ => tiny_skia::LineJoin::Miter,
    };
    let dash = doc.param_f64(node, "stroke-dash", 0.0);
    if dash > 0.0 {
        let gap = doc.param_f64(node, "stroke-gap", dash);
        stroke.dash = tiny_skia::StrokeDash::new(vec![dash as f32 * zoom, gap as f32 * zoom], 0.0);
    }
    Some((paint, stroke))
}
