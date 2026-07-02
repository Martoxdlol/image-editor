# Image Editor — Final Design Specification (v1.1)

A non-destructive, tree-based image editor combining the flexibility of Figma, the pixel tools of paint.net, and a parametric, non-baking document model. Standalone, offline-first, collaboration-*proof* by design (sync deferred).

**Stack:** React + Vite SPA (UI shell only) · Rust core (wasm64 for web, native for desktop/headless) · wgpu/WGSL rendering · Tauri desktop.

---

## 1. Core Philosophy

1. **The document tree is the single source of truth.** Nothing is baked. Every visible pixel is the result of evaluating the tree.
2. **Everything is an object with parameters.** Tools create/edit objects; modifiers transform them.
3. **Non-destructive by default, destructive on demand.** Any subtree can be explicitly rasterized ("bake") — itself a recorded, undoable conversion.
4. **Data layer is collaboration-ready:** immutable operation log, CRDT-shaped structures, per-actor undo — even though only one local actor exists today.
5. **The UI never owns state.** React renders from events and queries; all document, tool, and render state lives in Rust.

**Scope exclusions (deferred, schema-compatible):** real-time collaboration server/presence/branches/comments; animation/timeline/transitions; plugin marketplace.

---

## 2. Document Model

### 2.1 Structure

```text
Project (file)
└─ Document
   ├─ globals: variables, palettes, styles, components, assets
   ├─ Artboard[]  (own size ≤16384×16384, DPI, background, color space)
   │  └─ Node tree
   └─ SharedSpace (pasteboard nodes outside artboards)
```

- **Multiple artboards** positioned on an infinite pasteboard; nodes draggable between them; an artboard can be referenced as a live object inside another (collage/mockup use).
- **Artboard params:** name, size (px), DPI, background (transparent/color/checker), export presets, pixel-preview default.

### 2.2 Node tree

```text
Node (recursive)
├─ GroupNode   – children + shared modifiers
├─ LayerNode   – group with blend mode, opacity, visibility, lock
└─ LeafNode    – an actual object
```

**Leaf object types:**

| Type | Description |
|---|---|
| `Bitmap` | Raster data (tile-based, sparse) |
| `VectorPath` | Bézier paths, boolean-combinable |
| `Shape` | Parametric primitives (rect, ellipse, polygon, star, arrow, line) |
| `TextBlock` | Rich text (§7) |
| `StrokeSet` | Freehand strokes stored as stroke data — re-editable |
| `GradientFill` | Parametric gradient region |
| `Reference` | Instance of a component/symbol with overridable params |

**Modifiers** (attach to any node; ordered stack; apply to node + subtree):

- Transform (translate/rotate/scale/skew, arbitrary matrix, perspective; movable origin/anchor point)
- Opacity / blend mode
- Mask (raster paintable, vector, or node-as-mask by alpha/luma; invert, feather)
- SelectionScope (persisted selection region limiting following modifiers)
- Filter / Adjustment (§6)
- Clip (crop to shape)
- Warp/liquify (stored displacement field)

### 2.3 Selections as first-class objects

- Region with geometry (rect/ellipse/lasso/polygon/wand result), feather, anti-alias, boolean combine mode (replace/add/subtract/intersect/xor).
- Transient (classic editing) or saved as a node for reuse as mask/scope.
- Refinement ops: grow/shrink, feather, border, smooth, invert, transform selection.

### 2.4 Conversions (explicit, recorded in history — never data loss)

Shape→Path · Text→Path · Path→Bitmap (rasterize at chosen resolution) · StrokeSet→Bitmap · Bitmap→Path (trace: threshold/smoothing params) · Subtree→Bitmap (flatten).

### 2.5 Bitmaps and painting

- Painting on a `Bitmap` is destructive *within that node*; tool option: **"paint direct"** vs **"paint as strokes"** (records a `StrokeSet` on top instead).
- Bitmap edits stored as tile-based deltas — cheap undo, collab-ready.

---

## 3. Data Layer

### 3.1 Storage primitives (CRDT-shaped from day one)

| Primitive | Strategy |
|---|---|
| Tree structure | Movable-tree; sibling order via fractional index strings |
| Scalar params | LWW registers stamped `(actorId, lamport)` |
| Rich text | Sequence CRDT (RGA/Fugue-style) with inline mark ranges |
| Bitmap tiles | Per-tile op log; strokes replayed deterministically |
| Sets (tags, export marks) | Add-wins OR-Set |
| Deletion | Tombstones with GC on save |

### 3.2 Operation schema

```rust
struct OpId { actor: ActorId, lamport: u64 }

struct Op {
    id: OpId,
    parents: Vec<OpId>,     // causal deps
    txn: TxnId,             // undo grouping (one drag = one txn)
    kind: OpKind,
}

enum OpKind {
    NodeCreate { node_id, kind, initial_params },
    NodeDelete { node_id },                       // tombstone
    NodeMove   { node_id, new_parent, frac_index },
    ParamSet   { node_id, path, value },          // stores previous value
    ModifierAttach { node_id, modifier_id, index },
    ModifierDetach { .. },
    ModifierReorder { .. },
    PaintStroke { bitmap_id, brush_params, points }, // intent, not pixels
    PaintTilePatch { bitmap_id, tile, blob_hash, blend }, // checkpoint
    TextEdit   { .. },                            // sequence CRDT ops
    VariableSet { .. },
    ComponentPublish { .. },
    AssetAdd   { blob_hash },
}
```

- **Blobs are content-addressed** (hash → binary store); ops reference hashes.
- **`PaintStroke` carries intent** (brush params + points); deterministic replay produces identical tiles anywhere. Expensive strokes may be compacted into materialized `PaintTilePatch` checkpoints transparently.

### 3.3 Undo/redo & history

- **Per-actor undo:** undo appends inverse ops — history is never rewritten.
- Grouped transactions via `txnId`; history panel shows txn-level entries.
- Named snapshots = refs to version vectors; history playback scrubber possible from the op log.
- Autosave = append ops to the working directory log (crash-safe).

### 3.4 Values & params

```rust
enum Value {
    F64(f64), Bool(bool), Str(SmolStr),
    Color(Color),          // { space, [f32;4] } — HDR-safe, unclamped
    Point(Vec2), Matrix(Mat3),
    Ref(RefValue),         // palette entry / variable / node
    Expr(ExprAst),         // §8
    Blob(BlobHash),
}
```

Every tool and modifier exposes a **typed param schema** (number/enum/color/curve/bool/point) from which options bars and property panels auto-render. **Tool presets** save/load named param sets.

---

## 4. Rendering Engine

### 4.1 Pipeline

```text
Tree → Compile (resolve exprs/vars, expand components, flatten modifier stacks)
     → RenderGraph (DAG of tile-parallel ops)
     → GPU execution (wgpu/WGSL) with CPU fallback
     → View transform → Display
```

- **Tiled evaluation:** 256×256 tiles; only tiles intersecting dirty region ∩ viewport re-render.
- **Per-node per-tile caches** keyed by content hash of inputs+params; hash change = invalidation; dirty flags propagate downstream only.
- **Mip pyramids** per surface for zoomed-out views and thumbnails.
- **One WGSL shader codebase** across web (WebGPU) and native (Vulkan/Metal/DX12). CPU fallback path (Rust + SIMD) for correctness and **deterministic export** (reproducible across machines).

### 4.2 Render modes

- **Vector mode:** resolution matches viewport zoom; crisp at any zoom.
- **Pixel-preview mode:** raster at artboard resolution, nearest-neighbor magnified, pixel grid overlay at ≥800% zoom — exactly what export produces; enables true pixel editing.

### 4.3 Interactive performance

- Brush strokes: immediate GPU preview in viewport; committed to tiles asynchronously on stroke end.
- Filter param drags: half-resolution progressive preview, full-res on release.

### 4.4 Tile store & memory (16k×16k ceiling)

16384² @ f16 RGBA = 2 GiB per full-res surface → cannot keep everything resident.

```rust
struct TileKey { surface: SurfaceId, level: u8, x: u16, y: u16 }

enum TileData {
    Uniform(Color),        // solid/empty tiles cost 16 bytes
    Gpu(TextureHandle),
    Cpu(Box<[f16]>),
    Compressed(BlobRef),   // lz4 in RAM
    Disk(BlobRef),         // spilled to OPFS/disk
    Absent,                // recomputable from the DAG
}
```

- Sparse allocation; three-level residency (GPU ↔ CPU/compressed ↔ disk) with LRU + viewport pinning.
- Configurable memory budget (e.g., 4–8 GiB wasm64, more native); graceful degradation via spill + recompute.
- 16k×16k limit enforced in artboard validation.

---

## 5. Color & HDR

### 5.1 Pipeline

- **Working space:** linear scene-referred; f32 in compute, f16 tile storage.
- All compositing/blending in **linear light**; unbounded values (>1.0) allowed until output transform. Legacy sRGB-space blending as a per-document compatibility toggle.
- Working space options: sRGB-linear, Display P3-linear, Rec.2020-linear, ACEScg.
- Every imported `Bitmap` carries its profile; converted on import.

### 5.2 View transforms & display

- OCIO-style view transform per viewport: SDR sRGB / Display P3 / HDR PQ / HDR HLG; tone mapping for SDR preview of HDR content (ACES / AgX / clip).
- **HDR display is desktop-first** (native wgpu HDR swapchain, per-viewport nits target). Web ships tone-mapped SDR view; revisit when browser HDR canvas matures.

### 5.3 HDR-aware features

- Color picker with exposure slider, nits/stops readout, >1.0 values; UI models: sRGB, OKLCH, HSL, hex (SDR), linear float entry.
- Exposure/tone-map/highlight-recovery adjustments.
- Filters declare their working model: geometric filters (blur etc.) in linear; perceptual adjustments (HSL) in OKLab/OKHSL.
- Scopes panel: per-channel histogram, waveform, log scale, out-of-gamut warning overlay.

---

## 6. Tools, Modifiers, Filters

### 6.1 Raster drawing tools

| Tool | Key params |
|---|---|
| Pencil | size, hardness=1, pixel-perfect mode, dithering |
| Brush | size, hardness, flow, opacity, spacing, smoothing/stabilizer, pressure curves (size/opacity), custom tips, blend mode |
| Eraser | brush params + erase to transparent/background |
| Fill (bucket) | tolerance, contiguous/global, anti-alias, sample layer/composite, fill with color/pattern/gradient |
| Clone stamp | source point, aligned/unaligned, sample layer/composite |
| Smudge / Blur / Sharpen / Dodge / Burn | size, strength |
| Recolor brush | color A→B, tolerance |

### 6.2 Vector & shapes

- Pen tool (Bézier); node editing (add/delete/convert anchors, handles).
- Shapes: rect (per-corner radius), ellipse, polygon (sides), star (points, inner radius), line/arrow (caps, dashes).
- Boolean ops: union, subtract, intersect, exclude.
- Stroke: width, cap, join, miter, dash pattern, align inside/center/outside, pressure profile. Fill: solid/gradient/pattern/none.

### 6.3 Selection tools

- Rectangle, ellipse, lasso, polygonal lasso, magnetic lasso (edge snap).
- **Magic wand:** tolerance, contiguous/global, sample layer/composite, anti-alias, comparison color model (RGB/HSL/luma).
- Select by color range.

### 6.4 Utility tools

- Move/transform (handles + numeric + matrix input), crop (canvas or per-node clip), eyedropper (1px/3×3/5×5; layer/composite; → palette), zoom/pan/viewport rotation, measure (distance/angle).
- Gradient tool: linear/radial/conic/diamond/reflected, multi-stop editor, dither, live on-canvas handles.
- Slice/export-area tool.

### 6.5 Adjustments (parametric, non-destructive)

Brightness/contrast, levels, curves (per-channel), HSL, color balance, white balance, exposure, vibrance, B&W channel mixer, invert, posterize, threshold, gradient map, LUT.

### 6.6 Filters (modifier nodes: reorderable, toggleable, maskable)

Gaussian/motion/zoom/lens blur, sharpen/unsharp mask, noise add/reduce, median, pixelate, artistic set, distort (twist/bulge/wave/displacement), edge detect/outline, drop shadow, inner/outer glow, bevel, stroke effect.

### 6.7 Palettes, styles, variables, components

- **Palettes:** named colors referenced by objects (change propagates); import/export `.gpl`/`.ase`.
- **Styles:** named fill/stroke/text/effect styles, referenceable.
- **Global variables:** document-level named values (number/color/string/point); any param can bind to a variable or expression.
- **Components/symbols:** reusable subtrees with overridable params; edit once, update everywhere.

### 6.8 Tool interaction model (Rust)

- Each tool is a state machine: `handle(InputEvent, &DocView) → ToolEffect` where `ToolEffect` = transactional ops + overlay draw commands + cursor + ephemeral preview.
- Stabilizer/smoothing runs in the tool before op emission; low-latency brush path per §4.3.

---

## 7. Rich Text

### 7.1 Features

- Block: paragraphs, headings H1–H3, bulleted/numbered lists, alignment, line height, paragraph spacing.
- Inline: bold, italic, underline, strikethrough, color, highlight, font family/size, letter spacing, sub/superscript.
- Behaviors: auto-size vs fixed box (overflow clip), text on path, autocorrect + spellcheck (toggleable), smart quotes.
- Always editable; exact convert-to-path (rendering already uses outlines).

### 7.2 Text stack (correctness-first — locked decision)

`ed-text` composes the full pro pipeline:

| Concern | Choice |
|---|---|
| Shaping | `rustybuzz` (full complex-script: Arabic, Indic, …) |
| BiDi | `unicode-bidi` (UAX #9) |
| Line/word/grapheme segmentation | `icu_segmenter` (UAX #14/#29) |
| Itemization | `unicode-script` + custom itemizer (split runs by script/direction/font/style) |
| Font fallback | Per-run chains via `fontdb`: user font → style fallbacks → bundled Noto coverage |
| Outlines/metrics/variable fonts | `skrifa`; glyphs rendered as paths through the engine rasterizer; glyph atlas cache for raster mode; variation axes exposed as params |

Correctness guarantees: cluster-safe caret movement (never split graphemes), BiDi dual carets at direction boundaries, correct mixed-direction selection geometry, ligature-aware hit testing, emoji ZWJ handling. (Evaluate `parley` as the layout assembly layer; adopt only if its attribute model maps cleanly onto our text CRDT runs.)

Fonts on web: bundled Noto subset (lazy per script), user font import **embedded into document blobs** (portable files), Local Font Access API where available with embed-on-save. Editing: `ed-text` owns caret/selection; React positions caret/IME overlays from core-reported geometry; DOM IME composition events forwarded to core.

---

## 8. Expressions (simple — locked)

```text
expr   := term (("+"|"-") term)*
term   := factor (("*"|"/"|"%") factor)*
factor := number | ref | "(" expr ")" | "-" factor | call
ref    := "$" ident ("." ident)*            // $gridSize, $palette.accent
call   := ident "(" expr ("," expr)* ")"    // whitelist: min max clamp round floor ceil abs lerp
```

- No loops, no user functions, no side effects. Types: number, point (`.x`/`.y`), color (via refs + `lerp`).
- Parsed to AST at input time (validated), stored as AST, pretty-printed for editing.
- Dependency graph tracked → changing `$gridSize` dirties exactly the dependent nodes; cycle detection → error badge in properties panel.

---

## 9. Import / Export

### Import
- PNG, JPEG, WebP, GIF, BMP, TIFF, SVG (**as vector tree**), PSD (best-effort layers), PDF (page → group), EXR, HDR/Radiance, AVIF/HEIC (HDR gain maps), 16-bit PNG/TIFF, RAW (basic develop).
- Options: place at size / fit / original pixels; resample (nearest/bilinear/bicubic/Lanczos); new node vs new document; embed vs link (link = URL + hash with relink UX).
- Clipboard paste, drag-and-drop (full clipboard model: §10).

### Export
- PNG (bit depth, indexed), JPEG (quality, chroma), WebP, SVG (vector subtree), TIFF, PDF, GIF, EXR (16/32f), AVIF/JPEG-XL with HDR metadata, PNG + gain map, native format.
- Per-export: scale (0.5×/1×/2×/custom DPI), background, color profile, tone map for SDR targets.
- **Export areas (slices):** named regions or any node/group marked exportable, each with format+scale presets; batch export all.
- Deterministic renderer guarantees identical output on any machine.

---

## 10. Clipboard & Copy/Paste

Copy/paste follows the golden rule like everything else: React only brokers raw system-clipboard bytes; serialization, dependency resolution, and paste placement all happen in the Rust core.

### 10.1 Two channels

| Channel | Content | Use |
|---|---|---|
| Internal clipboard (core session) | Full-fidelity document fragment, zero-copy in the core worker | Copy/paste within a document and between open tabs |
| System clipboard (OS/browser) | Fragment payload + standard fallback flavors (PNG, SVG, HTML, plain text) | Cross-project between app instances; interop with external apps |

Every copy writes both. Paste reads the highest-fidelity flavor available: internal fragment → system fragment → SVG → image → text.

### 10.2 Fragment format

A self-contained snapshot, MIME `application/x-myed-fragment+zip`:

```text
fragment.myedclip (zip)
├─ fragment.json   # subtree snapshot(s) — same schema as document.json
├─ deps.json       # dependency closure: palette entries, variables, styles, components, fonts
└─ blobs/<hash>    # tiles, images, embedded fonts (content-addressed)
```

Metadata in `fragment.json`: format version, source document id, source artboard id, selection bounds, working color space, DPI. Content-addressed blobs mean cross-project paste dedupes automatically against the target document's blob store.

### 10.3 What copy captures (context-sensitive)

| Context | Copy | Cut |
|---|---|---|
| Node selection | Subtree(s) + modifier stacks + dependency closure | + recorded delete txn |
| Pixel selection on a `Bitmap` | Selected raster region as a Bitmap fragment | Clears region (tile deltas) |
| Pixel selection, **Copy Merged** | Flattened composite of the visible tree within the region | — |
| Text caret/range | Rich-text runs (internal) + HTML and plain text (system) | + text delete ops |
| **Copy Style** | Fill/stroke/text/effect params of the selection | — |
| **Copy Modifiers** | The selection's modifier stack | — |

Duplicate (Ctrl+D) = copy + paste-in-place as a single txn, without touching either clipboard.

### 10.4 Paste semantics

- **Paste:** into the active artboard/group at viewport center; if the source location is visible in the same document, paste at source position + (10, 10) offset.
- **Paste in Place:** exact source coordinates (same or different artboard).
- **Paste Into:** as child of the selected group/layer; with an active selection, pasted content gets the selection as a mask.
- **Paste as New Artboard** / **Paste as New Document**.
- **Paste Style / Paste Modifiers** applies onto the current selection.
- Raster fragment pasted while painting on a `Bitmap` → floating pasted selection (movable before commit), committed as tile deltas on deselect; in any other context it becomes a new `Bitmap` node.
- Every paste is **one transaction of ordinary ops** (`NodeCreate`, `ModifierAttach`, `ParamSet`, `AssetAdd`, `PaintTilePatch`) — no new op kinds, fully undoable, collab-safe. All pasted nodes receive fresh ids; fractional indexes are generated at the target insertion point.

### 10.5 Cross-project paste & dependency resolution

Fragments carry their dependency closure; pasting into a different document resolves it:

| Dependency | Resolution |
|---|---|
| Palette entry / style / variable / component | Match by stable global id, then content hash: identical → reuse target's; absent → import into target globals (tagged "Imported"); same name but different content → import under a suffixed name (`accent (2)`) |
| Fonts | Embedded font blobs copied into target document blobs |
| Expressions referencing variables | Stay bound if the variable resolves after the step above; otherwise the variable is imported too — a paste never silently breaks a binding |
| Color space | Fragment records the source working space; bitmap tiles and color values are converted to the target working space on paste (HDR values preserved, unclamped) |
| Unknown/missing plugin node types | Preserved opaquely — same never-data-loss rule as file load |

A per-paste option (with a document-level default) chooses **Keep linked** (import/reuse refs — default) vs **Flatten values** (dereference palette/variable refs to literal values).

### 10.6 Platform transports

- **Desktop (Tauri):** native OS clipboard; custom fragment format and all fallback flavors written together.
- **Web:** async Clipboard API. Chromium: custom web format (`web application/x-myed-fragment`). Safari/Firefox: the fragment is persisted to an OPFS clipboard store keyed by UUID and a stub reference rides in the HTML flavor — tabs in the same browser profile recover full fidelity; external apps get the standard fallbacks. Clipboard I/O runs on the main thread (permission-gated); bytes are forwarded to the core worker.
- **Large payloads:** above a threshold (default 64 MiB) the system clipboard carries only a stub; fragment data spills to OPFS/temp file and streams on paste.

### 10.7 External interop

- **Paste from outside:** any importable image format (§9) → import-on-paste with the standard placement options; SVG clipboard → vector tree; HTML/plain text → `TextBlock`; file list → batch import.
- **Copy to outside:** flavors written simultaneously — PNG (composited at artboard resolution, background per artboard settings), SVG (vector subtrees), HTML + plain text (text nodes). The receiving app picks what it understands.
- Plugins reach the clipboard only through the `clipboard` capability permission (§13).

---

## 11. File Format (locked: zip)

```text
project.myed (zip; Zip64 enabled)
├─ manifest.json      # format version, engine range, plugins used
├─ document.json      # compacted tree snapshot + globals (deflate)
├─ oplog/             # op segments since snapshot
├─ blobs/<hash>       # tiles, images, brush tips, LUTs (store, no recompression)
├─ thumbnails/        # per-artboard previews
└─ versions.json      # named snapshots → version vectors
```

- **Live working format:** unpacked directory in OPFS (web) / temp dir (desktop) — fast op appends, crash-safe autosave. Zip is the interchange format, packed on explicit save.
- Save of large docs: background repack → write temp → fsync → atomic rename.
- Forward compatibility: unknown node/modifier/plugin types preserved opaquely — **never data loss**.

---

## 12. System Architecture (locked)

### 12.1 Overview

```text
┌────────────────────────────────────────────────┐
│ React + Vite SPA — UI shell only, no doc state │
├────────────────────────────────────────────────┤
│ Generated TS bindings: commands, queries, subs │
├────────────────────────────────────────────────┤
│ Rust core (one codebase)                       │
│  ed-core / ed-document / ed-engine / ed-tools  │
│  ed-text / ed-io / ed-plugins                  │
├──────────────────┬─────────────────────────────┤
│ wasm64 (web)     │ native (Tauri desktop, CLI) │
└──────────────────┴─────────────────────────────┘
```

**Golden rule:** React never touches document data or pixels. Commands in, delta events + lightweight read-model mirrors out (tree outline, selected props, history list).

**Multi-document session:** the core worker hosts a `DocumentSession` registry — every open tab (§14) is a `Document` instance with its own op log, undo stack, and render state. Shared across documents: the content-addressed blob store (cross-project paste dedupes to a hash lookup), the font database, the GPU device, and one global tile-memory budget (background tabs demote aggressively: GPU → compressed → disk). Background documents pause rendering but keep autosaving. The internal clipboard (§10) lives in the session, outside any document.

### 12.2 Rust workspace

```text
crates/
├─ ed-core      # ids, geometry, color, params, errors
├─ ed-document  # tree, ops, undo, expressions, serialization (zero render deps)
├─ ed-engine    # render DAG, tile store, compositor, filters (CPU + wgpu/WGSL)
├─ ed-tools     # tool state machines
├─ ed-text      # rich text model, shaping/layout/editing (§7.2)
├─ ed-io        # codecs
├─ ed-plugins   # plugin host
├─ ed-wasm      # wasm-bindgen boundary, workers
├─ ed-desktop   # Tauri wrapper
└─ ed-cli       # headless render/export/validate/batch
```

All crates except `ed-desktop`/`ed-cli` build for wasm. `ed-document` is what a future sync server compiles natively.

### 12.3 Web (wasm64 — locked)

- **Primary build:** `wasm64` (Chrome/Firefox). **Fallback build:** `wasm32` behind the same TS bridge; capability probe at load picks the module. Threads + memory64 + SharedArrayBuffer probed together; degrade gracefully.
- Threading: main thread (React + bridge) · core worker (Rust: document + tools + engine) · rayon worker pool (SAB + wasm threads) · WebGPU device owned by core worker via transferred `OffscreenCanvas` — **pixels never cross into JS**.
- COOP/COEP headers required (service worker injection fallback).
- Input: pointer events (coalesced, pressure/tilt) forwarded to core worker.
- Persistence: OPFS working directory; open/save to disk via File System Access API (download fallback).
- wasm64 memory budget 4–8 GiB configurable; tile spilling retained for wasm32 fallback and worst cases.

### 12.4 Desktop — Tauri (locked: native wgpu + webview overlay)

- Rust core runs **natively** in the Tauri process; wgpu renders to a **native child surface positioned under the webview's canvas region**; React UI composites as transparent chrome over/around it.
- Benefits: full native GPU perf, no wasm ceiling, zero-copy present, **real HDR swapchain output** (desktop is the HDR display target).
- React reports canvas rect → core positions/resizes surface; canvas-region input forwarded via the same event protocol.
- **Linux fallback** (WebKitGTK layering flakiness): shared texture → WebGPU `<canvas>` in the webview (one copy). Both paths behind a `Presenter` trait, chosen at startup.
- Bridge = Tauri IPC; identical generated TS API as web.

### 12.5 Headless CLI

```bash
ed render project.myed --artboard "Cover" --scale 2 --format png -o cover.png
ed export project.myed --all-slices -o out/
ed validate project.myed
ed thumbnails project.myed
```

Identical engine → output matches the app exactly. Also the CI test harness (golden-image regression tests).

---

## 13. Plugin System

- **Runtime:** WASM component model — `wasmtime` on native/CLI; on web, plugin wasm modules load into their own workers (browser is the sandbox), host API bridged over messages.
- **Capability permissions** in manifest, user-approved: `document.read`, `document.write`, `network`, `clipboard`, `ui.panel`, `filesystem.scoped`.

| Plugin type | Contract |
|---|---|
| Filter/Adjustment | `process(tiles, params) → tiles`; param schema (auto-UI); declares HDR behavior (linear/perceptual); CPU wasm impl mandatory, WGSL variant optional |
| Tool | pointer/keyboard events + overlay draw API; emits document ops |
| Node type | custom leaf: `render(params) → raster/vector` + serialization |
| Importer/Exporter | `decode(bytes) → subtree` / `encode(subtree, opts) → bytes` |
| Panel/UI | declarative schema UI or sandboxed webview over RPC |
| Automation/script | one-shot scripts over the Document API |

- **Document API:** typed mirror of the op schema — `doc.createNode()`, `node.setParam()`, `doc.transaction(fn)`, `doc.query(selector)`, change subscriptions. Plugins never touch tree/pixels directly → automatic undo + future-collab compatibility.
- **Determinism:** filter plugins must be pure functions of (input, params, seed); seeded RNG provided.
- Distribution: local install + dev mode (load from folder, hot reload) from day one; documents record plugins used; missing plugin → opaque placeholder nodes, never data loss. Marketplace deferred.

---

## 14. UI Design

```text
┌──────────────────────────────────────────────────────────────┐
│ Menu bar: File Edit View Object Select Filter Window Help    │
├──────────────────────────────────────────────────────────────┤
│ Document tabs: [poster.myed *][logo.myed][+]                 │
├──────────────────────────────────────────────────────────────┤
│ Tool options bar (active tool params, presets dropdown)      │
├───┬──────────────────────────────────────────────┬───────────┤
│ T │  Rulers (units px/mm/in/%, drag-out guides)  │ Panels:   │
│ o │ ┌──────────────────────────────────────────┐ │ • Tree/   │
│ o │ │   Pasteboard with artboards              │ │   Layers  │
│ l │ │   (zoom, pan, pixel grid, guides,        │ │ • Props   │
│ b │ │    snapping, marching ants, handles)     │ │ • History │
│ a │ │                                          │ │ • Colors/ │
│ r │ └──────────────────────────────────────────┘ │   Palette │
│   │                                              │ • Styles  │
│   │                                              │ • Scopes  │
├───┴──────────────────────────────────────────────┴───────────┤
│ Status: cursor pos, selection size, zoom %, doc size,        │
│ render mode (vector/pixel-preview), view transform selector  │
└──────────────────────────────────────────────────────────────┘
```

- **Document tabs:** the app is multi-document — each open project is a tab in the tab strip (name + unsaved-changes dot; close on hover; middle-click closes). Drag to reorder; drag to a viewport edge for **split view** (two tab groups side by side — nodes drag-and-drop across projects, complementing clipboard transfer §10); drag out of the window → new OS window (desktop). Ctrl+Tab / Ctrl+Shift+Tab cycle tabs; Ctrl+W closes (prompt if unsaved); Ctrl+Shift+T reopens a recently closed tab (session-scoped, restored from the working directory). Tab overflow scrolls, with an all-tabs dropdown. "Multiple windows of the same document" surface as linked tabs (`name:2`) sharing one document instance.
- **Tree/Layers panel:** artboards as top-level entries; drag reorder/reparent; per-node visibility/lock/opacity/blend + modifier badges; context menu: group, rasterize, convert, mask, duplicate, create component, "new artboard from selection".
- **Properties panel:** context-sensitive; all params of selection + reorderable/toggleable modifier stack; numeric fields accept expressions and variable bindings; expression error badges.
- **Color panel:** wheel/sliders (sRGB/OKLCH/HSL/hex/linear float), exposure slider, swatches, document palette, recents.
- **History panel:** txn entries, jump-to, named snapshots.
- **Helpers:** rulers, guides, smart guides (object alignment snapping), grid + pixel grid, snap toggles (grid/guides/objects/pixels), per-object anchor points (draggable transform origin), navigator minimap.
- **View:** zoom 1%–6400%, fit/fill/100%, viewport rotation, multiple windows of the same document, fullscreen.
- **Misc:** command palette (Ctrl+K), customizable shortcuts, floating context toolbar near selection (align, boolean ops, crop), plugin manager + plugin panel dock + permission prompts, performance HUD (tiles rendered, cache hit rate, GPU memory).

---

## 15. Type Hierarchy Summary

```text
Node { id, name, visible, locked, blendMode, opacity, modifierStack }
├─ GroupNode { children }
└─ LeafNode
   ├─ Bitmap { tiles, colorProfile }
   ├─ VectorPath { paths, fill: Paint, stroke: Stroke }
   ├─ Shape { kind, params, fill, stroke }      → VectorPath
   ├─ TextBlock { richText, box, fill }         → VectorPath
   ├─ StrokeSet { strokes[], brushParams }      → Bitmap
   ├─ GradientFill { gradient, region }
   └─ Reference { componentId, overrides }

Modifier = Transform | Mask | Filter | Adjustment | Clip | SelectionScope | Warp
Paint    = Solid(colorOrPaletteRef) | Gradient | Pattern | None
ParamVal = literal | paletteRef | variableRef | expression
```

---

## 16. Build Order

1. **Skeleton:** workspace, `ed-wasm` worker bridge, React shell, wgpu clear on OffscreenCanvas; capability probe (wasm64/32, threads).
2. **Document core:** tree + ops + undo + OPFS working dir; tree & properties panels via read-model mirrors.
3. **Engine v1:** tile store (uniform + CPU tiles), compositor, vector rasterization, pixel-preview mode.
4. **Tools v1:** move/transform, shapes, rect/ellipse/lasso selection, fill, eyedropper, zoom/pan.
5. **Raster:** Bitmap nodes, brush/pencil/eraser (low-latency path), magic wand; tile spilling + mips.
6. **Import/export SDR** (PNG/JPEG/WebP, SVG import), artboards, export slices, zip save/load, **clipboard v1** (internal within-project copy/paste, image paste from OS, copy-as-PNG), `ed-cli` + golden-image CI.
7. **Multi-document & clipboard v2:** document tabs (`DocumentSession` registry, shared blob store + global tile budget), fragment format on system-clipboard transports, cross-project paste (dependency resolution completes with step 11's globals).
8. **Modifiers & filters:** masks, adjustments, first filter set (CPU + WGSL).
9. **Rich text:** full itemizer/BiDi/shaping pipeline early; exhaustive snapshot tests (mixed RTL/LTR, Indic clusters, emoji ZWJ).
10. **HDR:** view transforms, EXR/AVIF I/O, filter linear/perceptual audit; desktop HDR swapchain.
11. **Expressions, palettes/variables, components** — including cross-project paste resolution for palette/variable/style/component refs (§10.5).
12. **Tauri desktop:** native presenter + Linux fallback spike, packaging.
13. **Plugin host:** filter + script types first, dev mode with hot reload.

---

## 17. Deferred (schema-compatible by design)

| Feature | Readiness hook already in place |
|---|---|
| Real-time collaboration | Op log with actor/lamport ids, causal parents, tombstones, per-actor undo, deterministic stroke replay |
| Presence, comments, branches | Ephemeral channel slot in protocol; op log = audit trail; version vectors |
| Animation/timeline | `paramPath` addressing lets keyframe tracks attach to any param without schema change |
| Plugin marketplace | Manifest/versioning/permissions model already defined |
| Web HDR display | View-transform architecture ready; swap output transform when browsers ship HDR canvas |

## 18. Risk Register

1. **Linux Tauri presenter** — layered webview over native surface; fallback path designed, needs an early spike (step 12, prototype sooner).
2. **wasm64 on Safari** — wasm32 fallback build covers it; validate threads+SAB+memory64 matrix per browser at startup.
3. **Text subsystem size** — largest single subsystem; scheduled early with snapshot-test coverage.
4. **Large-doc save latency** — background zip repack with atomic replace.
5. **wgpu determinism** — CPU fallback renderer is the export ground truth; GPU/CPU parity tested in CI goldens.
6. **Web custom clipboard formats** — arbitrary clipboard MIME types are Chromium-only; Safari/Firefox rely on the OPFS-stub path (§10.6), which only spans tabs in the same browser profile. Degradation (external paste falls back to PNG/SVG) must be surfaced clearly in UX and covered by cross-browser tests.