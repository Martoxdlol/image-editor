//! The compositor (spec §4.1): tree → per-node evaluation → composited
//! frame. Renders artboards on the infinite pasteboard, honors modifier
//! stacks (transform, clip, filters), blend modes and opacity, and both
//! render modes (vector / pixel-preview, spec §4.2).

use crate::filters;
use crate::overlay::Overlay;
use crate::paint::{fill_paint, stroke_paint, to_sk_blend, to_sk_color};
use crate::raster::{bitmap_to_pixmap, replay_stroke};
use crate::shapes::{parse_path_data, shape_path};
use ed_core::{BlobHash, Color, Mat3, NodeId, Rect, Vec2};
use ed_document::{BitmapData, Document, Node, NodeKind, SelGeom};
use std::collections::HashMap;
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

/// Viewport state (owned by the core session, mirrored to the UI).
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub zoom: f64,
    /// Document coordinate at the viewport's top-left corner.
    pub pan: Vec2,
    /// Pixel-preview render mode (spec §4.2).
    pub pixel_preview: bool,
    /// Marching-ants animation phase, in device px.
    pub ants_phase: f32,
}

impl Default for View {
    fn default() -> Self {
        View { zoom: 1.0, pan: Vec2::new(-40.0, -40.0), pixel_preview: false, ants_phase: 0.0 }
    }
}

impl View {
    pub fn doc_to_screen(&self) -> Mat3 {
        Mat3::scale(self.zoom, self.zoom).mul(&Mat3::translate(Vec2::new(-self.pan.x, -self.pan.y)))
    }

    pub fn screen_to_doc(&self, p: Vec2) -> Vec2 {
        Vec2::new(p.x / self.zoom + self.pan.x, p.y / self.zoom + self.pan.y)
    }
}

pub fn to_transform(m: &Mat3) -> Transform {
    Transform::from_row(m.a as f32, m.b as f32, m.c as f32, m.d as f32, m.e as f32, m.f as f32)
}

pub struct Engine {
    bitmap_cache: HashMap<NodeId, (u64, Pixmap)>,
    strokes_cache: HashMap<NodeId, (BlobHash, Vec2, Pixmap)>,
    checker: Pixmap,
    /// Perf HUD counters (spec §14).
    pub last_nodes_rendered: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        let mut checker = Pixmap::new(16, 16).unwrap();
        let light = tiny_skia::Color::from_rgba8(255, 255, 255, 255);
        let dark = tiny_skia::Color::from_rgba8(203, 203, 203, 255);
        for y in 0..16 {
            for x in 0..16 {
                let c = if (x < 8) == (y < 8) { light } else { dark };
                let px = tiny_skia::PremultipliedColorU8::from_rgba(
                    (c.red() * 255.0) as u8,
                    (c.green() * 255.0) as u8,
                    (c.blue() * 255.0) as u8,
                    255,
                )
                .unwrap();
                checker.pixels_mut()[y * 16 + x] = px;
            }
        }
        Engine {
            bitmap_cache: HashMap::new(),
            strokes_cache: HashMap::new(),
            checker,
            last_nodes_rendered: 0,
        }
    }

    pub fn drop_caches(&mut self) {
        self.bitmap_cache.clear();
        self.strokes_cache.clear();
    }

    // ------------------------------------------------------------ frame

    /// Render the full viewport: pasteboard, artboards, shared-space nodes,
    /// selection ants and tool overlays.
    pub fn render(
        &mut self,
        doc: &Document,
        view: &View,
        width: u32,
        height: u32,
        overlays: &[Overlay],
        selected: &[NodeId],
    ) -> Pixmap {
        let mut canvas = Pixmap::new(width.max(1), height.max(1)).unwrap();
        canvas.fill(tiny_skia::Color::from_rgba8(34, 34, 38, 255)); // pasteboard
        self.last_nodes_rendered = 0;

        let m = view.doc_to_screen();
        let viewport_doc = Rect::new(
            view.pan.x,
            view.pan.y,
            width as f64 / view.zoom,
            height as f64 / view.zoom,
        );

        for &root in doc.children_of(None) {
            let Some(node) = doc.node(root) else { continue };
            if node.kind == NodeKind::Artboard {
                self.render_artboard_in_view(doc, root, &mut canvas, view, &viewport_doc);
            } else {
                // shared-space pasteboard node
                self.render_node(doc, root, &mut canvas, &m, None, view.zoom, false);
            }
        }

        // selected node outlines
        for &id in selected {
            if let Some(b) = self.node_bounds(doc, id) {
                draw_screen_rect(&mut canvas, &b.transform(&m), false, [88, 166, 255], 0.0);
            }
        }

        // pixel selection marching ants
        if let Some(sel) = &doc.pixel_selection {
            if !sel.is_empty() {
                self.draw_ants(&mut canvas, doc, view, &m);
            }
        }

        for ov in overlays {
            self.draw_overlay(&mut canvas, ov, &m, view);
        }

        canvas
    }

    fn render_artboard_in_view(
        &mut self,
        doc: &Document,
        id: NodeId,
        canvas: &mut Pixmap,
        view: &View,
        viewport_doc: &Rect,
    ) {
        let Some(rect) = doc.artboard_rect(id) else { return };
        if !rect.intersects(viewport_doc) {
            return;
        }
        let m = view.doc_to_screen();
        let screen_rect = rect.transform(&m);
        let node = doc.node(id).unwrap();

        // shadow + background
        let Some(sk_rect) = tiny_skia::Rect::from_xywh(
            screen_rect.x as f32,
            screen_rect.y as f32,
            screen_rect.w.max(1.0) as f32,
            screen_rect.h.max(1.0) as f32,
        ) else {
            return;
        };
        let mut shadow = Paint::default();
        shadow.set_color(tiny_skia::Color::from_rgba8(0, 0, 0, 90));
        let sr = tiny_skia::Rect::from_xywh(
            sk_rect.x() + 3.0,
            sk_rect.y() + 3.0,
            sk_rect.width(),
            sk_rect.height(),
        );
        if let Some(sr) = sr {
            canvas.fill_rect(sr, &shadow, Transform::identity(), None);
        }
        self.fill_background(canvas, doc, node, sk_rect);

        // clip mask for artboard content
        let mut clip_pb = PathBuilder::new();
        clip_pb.push_rect(sk_rect);
        let clip_path = clip_pb.finish().unwrap();
        let mut mask = Mask::new(canvas.width(), canvas.height()).unwrap();
        mask.fill_path(&clip_path, FillRule::Winding, true, Transform::identity());

        if view.pixel_preview {
            // raster at artboard resolution, blit nearest-neighbor (spec §4.2)
            let ab_w = rect.w.round().max(1.0) as u32;
            let ab_h = rect.h.round().max(1.0) as u32;
            if ab_w <= 8192 && ab_h <= 8192 {
                let mut ab_pm = Pixmap::new(ab_w, ab_h).unwrap();
                let ab_m = Mat3::translate(Vec2::new(-rect.x, -rect.y));
                for &c in doc.children_of(Some(id)) {
                    self.render_node(doc, c, &mut ab_pm, &ab_m, None, 1.0, true);
                }
                let paint = PixmapPaint {
                    quality: tiny_skia::FilterQuality::Nearest,
                    ..Default::default()
                };
                let t = Transform::from_row(
                    view.zoom as f32,
                    0.0,
                    0.0,
                    view.zoom as f32,
                    screen_rect.x as f32,
                    screen_rect.y as f32,
                );
                canvas.draw_pixmap(0, 0, ab_pm.as_ref(), &paint, t, Some(&mask));
                if view.zoom >= 8.0 {
                    self.draw_pixel_grid(canvas, &rect, view);
                }
                return;
            }
        }

        let m = view.doc_to_screen();
        for &c in doc.children_of(Some(id)) {
            self.render_node(doc, c, canvas, &m, Some(&mask), view.zoom, false);
        }
    }

    fn fill_background(&self, canvas: &mut Pixmap, doc: &Document, node: &Node, rect: tiny_skia::Rect) {
        let bg = doc.param_str(node, "background", "color");
        match bg.as_str() {
            "transparent" | "checker" => {
                let mut p = Paint::default();
                p.shader = tiny_skia::Pattern::new(
                    self.checker.as_ref(),
                    tiny_skia::SpreadMode::Repeat,
                    tiny_skia::FilterQuality::Nearest,
                    1.0,
                    Transform::from_translate(rect.x(), rect.y()),
                );
                canvas.fill_rect(rect, &p, Transform::identity(), None);
            }
            _ => {
                let c = doc.param_color(node, "bg-color", Color::WHITE);
                let mut p = Paint::default();
                p.set_color(to_sk_color(c));
                canvas.fill_rect(rect, &p, Transform::identity(), None);
            }
        }
    }

    /// Render one artboard at a chosen scale with no view/overlays — the
    /// deterministic export path (spec §9) and thumbnail source.
    pub fn render_artboard(
        &mut self,
        doc: &Document,
        id: NodeId,
        scale: f64,
        include_background: bool,
    ) -> Option<Pixmap> {
        let rect = doc.artboard_rect(id)?;
        let w = (rect.w * scale).round().max(1.0) as u32;
        let h = (rect.h * scale).round().max(1.0) as u32;
        let mut pm = Pixmap::new(w, h)?;
        let node = doc.node(id)?;
        if include_background {
            let bg = doc.param_str(node, "background", "color");
            if bg == "color" {
                pm.fill(to_sk_color(doc.param_color(node, "bg-color", Color::WHITE)));
            }
        }
        let m = Mat3::scale(scale, scale).mul(&Mat3::translate(Vec2::new(-rect.x, -rect.y)));
        for &c in doc.children_of(Some(id)) {
            self.render_node(doc, c, &mut pm, &m, None, scale, true);
        }
        Some(pm)
    }

    // ------------------------------------------------------------ nodes

    #[allow(clippy::too_many_arguments)]
    fn render_node(
        &mut self,
        doc: &Document,
        id: NodeId,
        canvas: &mut Pixmap,
        m: &Mat3,
        mask: Option<&Mask>,
        zoom: f64,
        export: bool,
    ) {
        let Some(node) = doc.node(id) else { return };
        if !node.visible() {
            return;
        }
        self.last_nodes_rendered += 1;

        // compose transform modifiers (spec §2.2)
        let local = modifier_matrix(doc, node);
        let m2 = m.mul(&local);

        let has_filters = node
            .modifiers
            .iter()
            .any(|md| md.enabled && filters::is_filter_kind(&md.kind));
        let has_clip = node.modifiers.iter().any(|md| md.enabled && md.kind == "clip");
        let has_mask_mod = node.modifiers.iter().any(|md| md.enabled && md.kind == "mask");
        let opacity = node.opacity();
        let blend = node.blend();
        let needs_layer = opacity < 0.999
            || blend != ed_core::BlendMode::Normal
            || has_filters
            || has_clip
            || has_mask_mod;

        if needs_layer {
            let mut layer = Pixmap::new(canvas.width(), canvas.height()).unwrap();
            self.render_content(doc, node, &mut layer, &m2, None, zoom, export);
            // clip modifier: keep only pixels inside the clip shape
            for md in node.modifiers.iter().filter(|md| md.enabled) {
                match md.kind.as_str() {
                    "clip" => {
                        let g = |k: &str, d: f64| {
                            md.params.get(k).map(|v| doc.resolve(v)).and_then(|v| v.as_f64()).unwrap_or(d)
                        };
                        let r = Rect::new(g("x", 0.0), g("y", 0.0), g("w", 100.0), g("h", 100.0))
                            .transform(&m2);
                        apply_rect_clip(&mut layer, &r);
                    }
                    "mask" => {
                        // node-as-mask by alpha (spec §2.2): render the mask
                        // node's subtree and multiply alpha.
                        if let Some(Value_node) = md.params.get("node") {
                            if let ed_core::Value::Ref(ed_core::value::RefValue::Node { id: mid }) =
                                Value_node
                            {
                                let mut mpm =
                                    Pixmap::new(canvas.width(), canvas.height()).unwrap();
                                self.render_node(doc, *mid, &mut mpm, m, None, zoom, export);
                                let invert = md
                                    .params
                                    .get("invert")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                apply_alpha_mask(&mut layer, &mpm, invert);
                            }
                        }
                    }
                    k if filters::is_filter_kind(k) => {
                        filters::apply(doc, md, &mut layer, zoom as f32);
                    }
                    _ => {}
                }
            }
            let paint = PixmapPaint {
                opacity: opacity as f32,
                blend_mode: to_sk_blend(blend),
                quality: tiny_skia::FilterQuality::Nearest,
            };
            canvas.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::identity(), mask);
        } else {
            self.render_content(doc, node, canvas, &m2, mask, zoom, export);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_content(
        &mut self,
        doc: &Document,
        node: &Node,
        canvas: &mut Pixmap,
        m: &Mat3,
        mask: Option<&Mask>,
        zoom: f64,
        export: bool,
    ) {
        let t = to_transform(m);
        match node.kind {
            NodeKind::Group | NodeKind::Layer | NodeKind::Artboard => {
                for &c in doc.children_of(Some(node.id)) {
                    // children get the parent-composed matrix
                    self.render_node(doc, c, canvas, m, mask, zoom, export);
                }
            }
            NodeKind::Shape => {
                if let Some(path) = shape_path(doc, node) {
                    let is_line = matches!(doc.param_str(node, "shape", "rect").as_str(), "line" | "arrow");
                    if !is_line {
                        if let Some(paint) = fill_paint(doc, node, t) {
                            if let Some(p) = path.clone().transform(t) {
                                canvas.fill_path(&p, &paint, FillRule::EvenOdd, Transform::identity(), mask);
                            }
                        }
                    }
                    if let Some((paint, stroke)) = stroke_paint(doc, node, zoom as f32) {
                        if let Some(p) = path.transform(t) {
                            canvas.stroke_path(&p, &paint, &stroke, Transform::identity(), mask);
                        }
                    } else if is_line {
                        // lines always need a stroke; default 1px black
                        let mut paint = Paint::default();
                        paint.anti_alias = true;
                        paint.set_color(to_sk_color(doc.param_color(node, "stroke-color", Color::BLACK)));
                        let stroke = tiny_skia::Stroke { width: zoom as f32, ..Default::default() };
                        if let Some(p) = path.transform(t) {
                            canvas.stroke_path(&p, &paint, &stroke, Transform::identity(), mask);
                        }
                    }
                }
            }
            NodeKind::Path => {
                let d = doc.param_str(node, "d", "");
                if let Some(path) = parse_path_data(&d) {
                    if let Some(paint) = fill_paint(doc, node, t) {
                        if let Some(p) = path.clone().transform(t) {
                            canvas.fill_path(&p, &paint, FillRule::EvenOdd, Transform::identity(), mask);
                        }
                    }
                    if let Some((paint, stroke)) = stroke_paint(doc, node, zoom as f32) {
                        if let Some(p) = path.transform(t) {
                            canvas.stroke_path(&p, &paint, &stroke, Transform::identity(), mask);
                        }
                    }
                }
            }
            NodeKind::Bitmap => {
                let Some(bm) = &node.bitmap else { return };
                let rev = bm.rev;
                let cached = self.bitmap_cache.get(&node.id);
                if cached.map(|(r, _)| *r != rev).unwrap_or(true) {
                    if let Some(pm) = bitmap_to_pixmap(bm) {
                        self.bitmap_cache.insert(node.id, (rev, pm));
                    } else {
                        return;
                    }
                }
                let (_, pm) = &self.bitmap_cache[&node.id];
                let x = doc.param_f64(node, "x", 0.0);
                let y = doc.param_f64(node, "y", 0.0);
                let quality = if export || zoom >= 3.0 {
                    tiny_skia::FilterQuality::Nearest
                } else {
                    tiny_skia::FilterQuality::Bilinear
                };
                let paint = PixmapPaint { quality, ..Default::default() };
                let t2 = to_transform(&m.mul(&Mat3::translate(Vec2::new(x, y))));
                canvas.draw_pixmap(0, 0, pm.as_ref(), &paint, t2, mask);
            }
            NodeKind::StrokeSet => {
                if node.strokes.is_empty() {
                    return;
                }
                // strokes live in doc coords; rasterize into bbox-sized
                // buffer then draw like a bitmap (cached by content hash)
                let hash = strokes_hash(node);
                let needs = self
                    .strokes_cache
                    .get(&node.id)
                    .map(|(h, _, _)| *h != hash)
                    .unwrap_or(true);
                if needs {
                    let Some(bbox) = strokes_bounds(node) else { return };
                    let origin = Vec2::new(bbox.x.floor(), bbox.y.floor());
                    let w = (bbox.w.ceil() as u32 + 2).min(8192);
                    let h = (bbox.h.ceil() as u32 + 2).min(8192);
                    let mut bm = BitmapData::new(w, h);
                    for s in &node.strokes {
                        replay_stroke(&mut bm, s, origin);
                    }
                    if let Some(pm) = bitmap_to_pixmap(&bm) {
                        self.strokes_cache.insert(node.id, (hash, origin, pm));
                    }
                }
                if let Some((_, origin, pm)) = self.strokes_cache.get(&node.id) {
                    let paint = PixmapPaint {
                        quality: if zoom >= 3.0 {
                            tiny_skia::FilterQuality::Nearest
                        } else {
                            tiny_skia::FilterQuality::Bilinear
                        },
                        ..Default::default()
                    };
                    let t2 = to_transform(&m.mul(&Mat3::translate(*origin)));
                    canvas.draw_pixmap(0, 0, pm.as_ref(), &paint, t2, mask);
                }
            }
            NodeKind::GradientFill => {
                let x = doc.param_f64(node, "x", 0.0);
                let y = doc.param_f64(node, "y", 0.0);
                let w = doc.param_f64(node, "w", 100.0);
                let h = doc.param_f64(node, "h", 100.0);
                if let Some(shader) = crate::paint::gradient_shader(doc, node, "", t) {
                    let mut paint = Paint::default();
                    paint.anti_alias = true;
                    paint.shader = shader;
                    let r = Rect::new(x, y, w, h).transform(m);
                    if let Some(sk) = tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.w.max(0.01) as f32, r.h.max(0.01) as f32) {
                        canvas.fill_rect(sk, &paint, Transform::identity(), mask);
                    }
                }
            }
            NodeKind::Text => {
                crate::text::render_text(doc, node, canvas, m, mask);
            }
            NodeKind::Reference => {
                // live instance: render the referenced subtree at our offset
                if let Some(ed_core::Value::Ref(ed_core::value::RefValue::Node { id: comp })) =
                    node.get_param("component")
                {
                    if comp != node.id && doc.node(comp).is_some() && !doc.is_ancestor(comp, node.id)
                    {
                        let dx = doc.param_f64(node, "x", 0.0);
                        let dy = doc.param_f64(node, "y", 0.0);
                        let src = doc.node_position(comp);
                        let m2 = m.mul(&Mat3::translate(Vec2::new(dx - src.x, dy - src.y)));
                        self.render_node(doc, comp, canvas, &m2, mask, zoom, export);
                    }
                }
            }
        }
    }

    /// Render one node subtree standalone (rasterize/copy-as-PNG paths).
    pub fn render_node_standalone(
        &mut self,
        doc: &Document,
        id: NodeId,
        pm: &mut Pixmap,
        m: &Mat3,
    ) {
        self.render_node(doc, id, pm, m, None, 1.0, true);
    }

    // ------------------------------------------------------------ bounds & hit

    /// Doc-space axis-aligned bounds of a node, including transform mods.
    pub fn node_bounds(&self, doc: &Document, id: NodeId) -> Option<Rect> {
        let node = doc.node(id)?;
        let raw = self.raw_bounds(doc, node)?;
        let m = modifier_matrix(doc, node);
        Some(raw.transform(&m))
    }

    fn raw_bounds(&self, doc: &Document, node: &Node) -> Option<Rect> {
        match node.kind {
            NodeKind::Artboard => doc.artboard_rect(node.id),
            NodeKind::Group | NodeKind::Layer => {
                let mut acc: Option<Rect> = None;
                for &c in doc.children_of(Some(node.id)) {
                    if let Some(b) = self.node_bounds(doc, c) {
                        acc = Some(acc.map(|a| a.union(&b)).unwrap_or(b));
                    }
                }
                acc
            }
            NodeKind::Shape => {
                let p = shape_path(doc, node)?;
                let b = p.bounds();
                let sw = doc.param_f64(node, "stroke-width", 0.0);
                Some(Rect::new(b.left() as f64, b.top() as f64, b.width() as f64, b.height() as f64).inflate(sw / 2.0))
            }
            NodeKind::Path => {
                let p = parse_path_data(&doc.param_str(node, "d", ""))?;
                let b = p.bounds();
                let sw = doc.param_f64(node, "stroke-width", 0.0);
                Some(Rect::new(b.left() as f64, b.top() as f64, b.width() as f64, b.height() as f64).inflate(sw / 2.0))
            }
            NodeKind::Bitmap => {
                let bm = node.bitmap.as_ref()?;
                Some(Rect::new(
                    doc.param_f64(node, "x", 0.0),
                    doc.param_f64(node, "y", 0.0),
                    bm.width as f64,
                    bm.height as f64,
                ))
            }
            NodeKind::StrokeSet => strokes_bounds(node),
            NodeKind::GradientFill | NodeKind::Text => Some(Rect::new(
                doc.param_f64(node, "x", 0.0),
                doc.param_f64(node, "y", 0.0),
                doc.param_f64(node, "w", 100.0),
                doc.param_f64(node, "h", 40.0),
            )),
            NodeKind::Reference => {
                if let Some(ed_core::Value::Ref(ed_core::value::RefValue::Node { id: comp })) =
                    node.get_param("component")
                {
                    let b = self.node_bounds(doc, comp)?;
                    let dx = doc.param_f64(node, "x", 0.0);
                    let dy = doc.param_f64(node, "y", 0.0);
                    let src = doc.node_position(comp);
                    Some(Rect::new(b.x + dx - src.x, b.y + dy - src.y, b.w, b.h))
                } else {
                    None
                }
            }
        }
    }

    // ------------------------------------------------------------ overlays

    fn draw_ants(&self, canvas: &mut Pixmap, doc: &Document, view: &View, m: &Mat3) {
        let Some(sel) = &doc.pixel_selection else { return };
        for shape in &sel.shapes {
            match &shape.geom {
                SelGeom::Rect { rect } => {
                    draw_marching_rect(canvas, &rect.transform(m), view.ants_phase)
                }
                SelGeom::Ellipse { rect } => {
                    let r = rect.transform(m);
                    if let Some(sk) = tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.w.max(0.01) as f32, r.h.max(0.01) as f32) {
                        let mut pb = PathBuilder::new();
                        pb.push_oval(sk);
                        if let Some(p) = pb.finish() {
                            stroke_ants_path(canvas, &p, view.ants_phase);
                        }
                    }
                }
                SelGeom::Polygon { points } => {
                    let mut pb = PathBuilder::new();
                    for (i, pt) in points.iter().enumerate() {
                        let s = m.apply(*pt);
                        if i == 0 {
                            pb.move_to(s.x as f32, s.y as f32);
                        } else {
                            pb.line_to(s.x as f32, s.y as f32);
                        }
                    }
                    pb.close();
                    if let Some(p) = pb.finish() {
                        stroke_ants_path(canvas, &p, view.ants_phase);
                    }
                }
                SelGeom::Mask { .. } => {
                    draw_marching_rect(canvas, &shape.geom.bounds().transform(m), view.ants_phase)
                }
            }
        }
    }

    fn draw_overlay(&self, canvas: &mut Pixmap, ov: &Overlay, m: &Mat3, view: &View) {
        match ov {
            Overlay::RectOutline { rect, dashed } => {
                draw_screen_rect(canvas, &rect.transform(m), *dashed, [88, 166, 255], view.ants_phase)
            }
            Overlay::EllipseOutline { rect, dashed } => {
                let r = rect.transform(m);
                if let Some(sk) = tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.w.max(0.01) as f32, r.h.max(0.01) as f32) {
                    let mut pb = PathBuilder::new();
                    pb.push_oval(sk);
                    if let Some(p) = pb.finish() {
                        stroke_overlay_path(canvas, &p, *dashed, [88, 166, 255]);
                    }
                }
            }
            Overlay::PolyOutline { points, close, dashed } => {
                if points.len() < 2 {
                    return;
                }
                let mut pb = PathBuilder::new();
                for (i, pt) in points.iter().enumerate() {
                    let s = m.apply(*pt);
                    if i == 0 {
                        pb.move_to(s.x as f32, s.y as f32);
                    } else {
                        pb.line_to(s.x as f32, s.y as f32);
                    }
                }
                if *close {
                    pb.close();
                }
                if let Some(p) = pb.finish() {
                    stroke_overlay_path(canvas, &p, *dashed, [88, 166, 255]);
                }
            }
            Overlay::Handles { rect, .. } => {
                let r = rect.transform(m);
                draw_screen_rect(canvas, &r, false, [88, 166, 255], 0.0);
                for (hx, hy) in handle_points(&r) {
                    draw_handle(canvas, hx, hy);
                }
            }
            Overlay::BrushCursor { pos, size } => {
                let s = m.apply(*pos);
                let r = (size / 2.0 * view.zoom).max(1.0);
                let mut pb = PathBuilder::new();
                pb.push_circle(s.x as f32, s.y as f32, r as f32);
                if let Some(p) = pb.finish() {
                    stroke_overlay_path(canvas, &p, false, [255, 255, 255]);
                    // dark inner ring for visibility on light bg
                    let mut pb2 = PathBuilder::new();
                    pb2.push_circle(s.x as f32, s.y as f32, (r + 1.0) as f32);
                    if let Some(p2) = pb2.finish() {
                        stroke_overlay_path(canvas, &p2, false, [0, 0, 0]);
                    }
                }
            }
            Overlay::Line { from, to, dashed } => {
                let a = m.apply(*from);
                let b = m.apply(*to);
                let mut pb = PathBuilder::new();
                pb.move_to(a.x as f32, a.y as f32);
                pb.line_to(b.x as f32, b.y as f32);
                if let Some(p) = pb.finish() {
                    stroke_overlay_path(canvas, &p, *dashed, [88, 166, 255]);
                }
            }
            Overlay::PathPreview { d } => {
                if let Some(p) = parse_path_data(d) {
                    if let Some(p) = p.transform(to_transform(m)) {
                        stroke_overlay_path(canvas, &p, false, [88, 166, 255]);
                    }
                }
            }
            Overlay::Anchors { points, active } => {
                for (i, pt) in points.iter().enumerate() {
                    let s = m.apply(*pt);
                    draw_anchor(canvas, s.x as f32, s.y as f32, Some(i) == *active);
                }
            }
            Overlay::Crosshair { pos } => {
                let s = m.apply(*pos);
                let mut pb = PathBuilder::new();
                pb.move_to(s.x as f32 - 8.0, s.y as f32);
                pb.line_to(s.x as f32 + 8.0, s.y as f32);
                pb.move_to(s.x as f32, s.y as f32 - 8.0);
                pb.line_to(s.x as f32, s.y as f32 + 8.0);
                if let Some(p) = pb.finish() {
                    stroke_overlay_path(canvas, &p, false, [255, 255, 255]);
                }
            }
        }
    }

    fn draw_pixel_grid(&self, canvas: &mut Pixmap, ab_rect: &Rect, view: &View) {
        let m = view.doc_to_screen();
        let mut pb = PathBuilder::new();
        let x0 = ab_rect.x.floor() as i64;
        let x1 = (ab_rect.x + ab_rect.w).ceil() as i64;
        let y0 = ab_rect.y.floor() as i64;
        let y1 = (ab_rect.y + ab_rect.h).ceil() as i64;
        for x in x0..=x1 {
            let a = m.apply(Vec2::new(x as f64, y0 as f64));
            let b = m.apply(Vec2::new(x as f64, y1 as f64));
            pb.move_to(a.x as f32, a.y as f32);
            pb.line_to(b.x as f32, b.y as f32);
        }
        for y in y0..=y1 {
            let a = m.apply(Vec2::new(x0 as f64, y as f64));
            let b = m.apply(Vec2::new(x1 as f64, y as f64));
            pb.move_to(a.x as f32, a.y as f32);
            pb.line_to(b.x as f32, b.y as f32);
        }
        if let Some(p) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(tiny_skia::Color::from_rgba8(128, 128, 128, 60));
            let stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
            canvas.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
        }
    }
}

// ---------------------------------------------------------------- helpers

/// Compose all enabled transform modifiers of a node (spec §2.2).
pub fn modifier_matrix(doc: &Document, node: &Node) -> Mat3 {
    let mut m = Mat3::IDENTITY;
    for md in node.modifiers.iter().filter(|md| md.enabled && md.kind == "transform") {
        let g = |k: &str, d: f64| {
            md.params.get(k).map(|v| doc.resolve(v)).and_then(|v| v.as_f64()).unwrap_or(d)
        };
        let anchor = Vec2::new(g("ax", 0.0), g("ay", 0.0));
        let local = Mat3::translate(Vec2::new(g("tx", 0.0), g("ty", 0.0)))
            .mul(&Mat3::translate(anchor))
            .mul(&Mat3::rotate(g("rotate", 0.0).to_radians()))
            .mul(&Mat3::scale(g("sx", 1.0), g("sy", 1.0)))
            .mul(&Mat3::skew(g("skew-x", 0.0).to_radians(), g("skew-y", 0.0).to_radians()))
            .mul(&Mat3::translate(Vec2::new(-anchor.x, -anchor.y)));
        m = local.mul(&m);
    }
    m
}

fn strokes_hash(node: &Node) -> BlobHash {
    let mut acc: Vec<u8> = Vec::new();
    for s in &node.strokes {
        acc.extend_from_slice(&(s.points.len() as u32).to_le_bytes());
        acc.extend_from_slice(&s.size.to_le_bytes());
        acc.extend_from_slice(&s.opacity.to_le_bytes());
        for c in s.color.rgba {
            acc.extend_from_slice(&c.to_le_bytes());
        }
        if let (Some(f), Some(l)) = (s.points.first(), s.points.last()) {
            acc.extend_from_slice(&f.pos.x.to_le_bytes());
            acc.extend_from_slice(&f.pos.y.to_le_bytes());
            acc.extend_from_slice(&l.pos.x.to_le_bytes());
            acc.extend_from_slice(&l.pos.y.to_le_bytes());
        }
    }
    BlobHash::of(&acc)
}

pub fn strokes_bounds(node: &Node) -> Option<Rect> {
    let mut acc: Option<Rect> = None;
    for s in &node.strokes {
        for p in &s.points {
            let r = Rect::new(p.pos.x, p.pos.y, 0.0, 0.0).inflate(s.size / 2.0 + 1.0);
            acc = Some(acc.map(|a| a.union(&r)).unwrap_or(r));
        }
    }
    acc
}

fn apply_rect_clip(layer: &mut Pixmap, keep: &Rect) {
    let w = layer.width() as i64;
    let h = layer.height() as i64;
    let x0 = keep.x.floor().max(0.0) as i64;
    let y0 = keep.y.floor().max(0.0) as i64;
    let x1 = (keep.x + keep.w).ceil().min(w as f64) as i64;
    let y1 = (keep.y + keep.h).ceil().min(h as f64) as i64;
    let data = layer.data_mut();
    for y in 0..h {
        for x in 0..w {
            if x < x0 || x >= x1 || y < y0 || y >= y1 {
                let i = ((y * w + x) * 4) as usize;
                data[i..i + 4].fill(0);
            }
        }
    }
}

fn apply_alpha_mask(layer: &mut Pixmap, mask_pm: &Pixmap, invert: bool) {
    let mask_px = mask_pm.pixels();
    for (i, px) in layer.pixels_mut().iter_mut().enumerate() {
        let ma = mask_px.get(i).map(|p| p.alpha()).unwrap_or(0);
        let f = if invert { 255 - ma } else { ma } as u32;
        let r = (px.red() as u32 * f / 255) as u8;
        let g = (px.green() as u32 * f / 255) as u8;
        let b = (px.blue() as u32 * f / 255) as u8;
        let a = (px.alpha() as u32 * f / 255) as u8;
        *px = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, a).unwrap();
    }
}

fn handle_points(r: &Rect) -> Vec<(f32, f32)> {
    let (x0, y0) = (r.x as f32, r.y as f32);
    let (x1, y1) = ((r.x + r.w) as f32, (r.y + r.h) as f32);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    vec![
        (x0, y0),
        (mx, y0),
        (x1, y0),
        (x1, my),
        (x1, y1),
        (mx, y1),
        (x0, y1),
        (x0, my),
    ]
}

fn draw_handle(canvas: &mut Pixmap, x: f32, y: f32) {
    let s = 3.5;
    if let Some(rect) = tiny_skia::Rect::from_xywh(x - s, y - s, s * 2.0, s * 2.0) {
        let mut fill = Paint::default();
        fill.set_color(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
        canvas.fill_rect(rect, &fill, Transform::identity(), None);
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        if let Some(p) = pb.finish() {
            let mut border = Paint::default();
            border.set_color(tiny_skia::Color::from_rgba8(88, 166, 255, 255));
            let stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
            canvas.stroke_path(&p, &border, &stroke, Transform::identity(), None);
        }
    }
}

fn draw_anchor(canvas: &mut Pixmap, x: f32, y: f32, active: bool) {
    let s = 3.0;
    if let Some(rect) = tiny_skia::Rect::from_xywh(x - s, y - s, s * 2.0, s * 2.0) {
        let mut fill = Paint::default();
        if active {
            fill.set_color(tiny_skia::Color::from_rgba8(88, 166, 255, 255));
        } else {
            fill.set_color(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
        }
        canvas.fill_rect(rect, &fill, Transform::identity(), None);
    }
}

fn draw_screen_rect(canvas: &mut Pixmap, r: &Rect, dashed: bool, color: [u8; 3], phase: f32) {
    let Some(sk) = tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.w.max(0.01) as f32, r.h.max(0.01) as f32) else {
        return;
    };
    let mut pb = PathBuilder::new();
    pb.push_rect(sk);
    if let Some(p) = pb.finish() {
        if dashed && phase != 0.0 {
            stroke_ants_path(canvas, &p, phase);
        } else {
            stroke_overlay_path(canvas, &p, dashed, color);
        }
    }
}

fn draw_marching_rect(canvas: &mut Pixmap, r: &Rect, phase: f32) {
    let Some(sk) = tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.w.max(0.01) as f32, r.h.max(0.01) as f32) else {
        return;
    };
    let mut pb = PathBuilder::new();
    pb.push_rect(sk);
    if let Some(p) = pb.finish() {
        stroke_ants_path(canvas, &p, phase);
    }
}

/// Marching ants: white base + black dashes offset by phase.
fn stroke_ants_path(canvas: &mut Pixmap, path: &tiny_skia::Path, phase: f32) {
    let mut white = Paint::default();
    white.set_color(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    let stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
    canvas.stroke_path(path, &white, &stroke, Transform::identity(), None);

    let mut black = Paint::default();
    black.set_color(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let mut dash_stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
    dash_stroke.dash = tiny_skia::StrokeDash::new(vec![4.0, 4.0], phase);
    canvas.stroke_path(path, &black, &dash_stroke, Transform::identity(), None);
}

fn stroke_overlay_path(canvas: &mut Pixmap, path: &tiny_skia::Path, dashed: bool, color: [u8; 3]) {
    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.set_color(tiny_skia::Color::from_rgba8(color[0], color[1], color[2], 255));
    let mut stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
    if dashed {
        stroke.dash = tiny_skia::StrokeDash::new(vec![4.0, 4.0], 0.0);
    }
    canvas.stroke_path(path, &paint, &stroke, Transform::identity(), None);
}
