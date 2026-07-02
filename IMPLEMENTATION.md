# Implementation Status

What of the [README.md](README.md) spec is built, and how faithfully. The
architecture follows the spec's locked decisions; scoped-down areas use the
fallback paths the spec itself defines.

## Running it

```bash
# web app (builds the wasm core on first run)
cd app && pnpm install && pnpm dev

# tests
cargo test --workspace          # 51 tests: ops, undo, expressions, engine, io, clipboard
cd app && pnpm test             # vitest (TS mirrors)

# headless CLI (spec §12.5)
cargo run -p ed-cli -- validate project.myed
cargo run -p ed-cli -- render project.myed --scale 2 -o out.png
cargo run -p ed-cli -- export project.myed -o out/
```

## Architecture (spec §12) — as specced

- **Rust workspace** `crates/`: `ed-core` (ids/geometry/color/params),
  `ed-document` (tree, ops, undo, expressions, serialization — zero render
  deps), `ed-engine` (compositor, filters, raster kernels, hit testing),
  `ed-tools` (tool state machines + `Session` multi-document registry),
  `ed-io` (codecs, .myed container), `ed-wasm` (bindgen boundary),
  `ed-cli` (headless render/export/validate).
- **Golden rule holds**: React renders chrome only. The document lives in a
  core web worker; commands in (JSON), read-model mirrors out (outline,
  props, history, tabs). Pixels render to an `OffscreenCanvas` owned by the
  worker; they never enter React.
- **wasm32 build** — the spec's own fallback tier (§12.3). wasm64 needs
  nightly + memory64; the TS bridge is unchanged if that lands later.

## Data layer (spec §3) — as specced

- Op log with `OpId {actor, lamport}`, causal parent, txn grouping. Ops:
  NodeCreate/Delete(tombstone)/Restore/Move, ParamSet (stores prev),
  ModifierAttach/Detach/Reorder, PaintTilePatch (blob-addressed tile
  deltas), StrokesSet, BitmapReplace, VariableSet, PaletteSet.
- **Per-actor undo appends inverse ops** — history is never rewritten;
  the History panel shows Undo/Redo rows as new transactions.
- Movable tree with **fractional-index sibling order**; content-addressed
  **BlobStore shared across the session** (cross-document paste dedupes).
- Params are a typed map (`Value`: f64/bool/str/color/point/matrix/ref/
  expr/blob); panels auto-render from them.

## Implemented feature areas

| Area | Status |
|---|---|
| Artboards on pasteboard, multiple, 16384² validation | ✅ |
| Node kinds: Group/Layer, Shape (rect/ellipse/polygon/star/line/arrow), Path, Bitmap (sparse 256px tiles), StrokeSet (re-editable, deterministic replay), GradientFill, Text, Reference | ✅ |
| Modifiers: transform (anchor/rotate/scale/skew), clip, mask (node-as-mask by alpha), filters/adjustments | ✅ |
| Filters: gaussian blur, pixelate, noise (seeded); adjustments: brightness/contrast, HSL, levels, invert, grayscale, posterize, threshold | ✅ |
| Tools: select/move/resize/marquee, all shapes, pen (polyline, close-to-fill), text, brush/pencil/eraser (pressure, soft brush, paint-as-strokes mode), fill (bitmap flood + shape recolor), eyedropper (composite), gradient, rect/ellipse/lasso selects, magic wand (composite sample), pan, zoom | ✅ |
| Pixel selections: boolean combine (replace/add/subtract/intersect), feather, invert, marching ants; constrain painting & fill | ✅ |
| **Area cut/copy/move** (two cut semantics): with a pixel selection active, ⌘X/⌘C/Delete act on the AREA — merged region → clipboard as bitmap fragment; bitmaps lose pixels via tile deltas; vector objects get a non-destructive inverted `sel-mask` modifier (toggle/remove per object in Properties); dragging inside a selection lifts it into a floating bitmap and moves it (paint-style); "Affects" scope option (all/selected/bitmaps) | ✅ |
| Selections as saved nodes | ❌ deferred |
| Expressions (§8): full grammar, whitelist funcs, `$var`/`$palette.x` refs, dependency tracking; `=expr` accepted in property fields | ✅ |
| Palettes (named colors, live refs), document variables | ✅ (`.gpl/.ase` import deferred) |
| Conversions (§2.4): Shape→Path, Subtree→Bitmap (rasterize) | ✅ (Text→Path, Bitmap→Path trace deferred) |
| Clipboard (§10): internal full-fidelity fragment w/ dependency closure (palette/variable resolution incl. rename-on-conflict), fresh ids, one-txn paste, paste/paste-in-place, duplicate, cut; system PNG flavor out; OS image paste in | ✅ scoped to §16.6 "clipboard v1" + §10.5 dep resolution |
| Multi-document tabs, shared blob store + clipboard across tabs | ✅ (split view/OS windows deferred) |
| .myed zip container: manifest + document.json + content-addressed blobs | ✅ (oplog/ segments + OPFS autosave deferred) |
| Import PNG/JPEG/WebP/GIF/BMP/TIFF; export PNG/JPEG/WebP at 0.5–4×; drag-drop import | ✅ (SVG/PSD/PDF/EXR/RAW deferred) |
| Pixel-preview mode (artboard-res raster, nearest-neighbor, pixel grid ≥800%) vs vector mode | ✅ |
| UI (§14): menu bar, document tabs, tool options bar, tool rail, layers tree (visibility/lock/badges/context menu), properties (params + modifier stack reorder/toggle + expressions), history, color/palette/variables panels, status bar (cursor/zoom/mode/perf) | ✅ (rulers/guides/snapping/command palette deferred) |
| Headless CLI: render/export/thumbnails/validate — same engine as the app | ✅ |
| Undo/redo everywhere incl. paint tile patches | ✅ |

## Scope reductions (with spec-sanctioned fallbacks)

- **Renderer**: tiny-skia CPU compositor — the spec's deterministic CPU
  path (§4.1). The wgpu/WGSL GPU tier, tile-parallel render graph, mips and
  residency/spilling are deferred. Blending is sRGB-space (the spec's §5.1
  compatibility toggle); tiles are RGBA8, not f16 — the linear-light f32
  pipeline and HDR view transforms (§5) are deferred.
- **Text (§7)**: single bundled Noto Sans face with wrapping/alignment,
  edited via the properties panel. The rustybuzz/BiDi/icu4x pipeline and
  in-canvas IME editing sit behind the same `render_text` call site.
- **Tauri desktop, plugins (§13), OPFS working dir, COOP/COEP threading**:
  deferred (headers are already served; the worker runs single-threaded).
- Bitmap hit-testing is alpha-based so transparent paint layers don't
  swallow clicks.

## Verification

- 51 Rust unit tests + 3 vitest tests, all green.
- Browser end-to-end (Playwright): draw shapes, paint, text, move/resize,
  marching-ants selections, undo/redo (append-only history verified),
  filters via menu, expression params, pixel preview, PNG export bytes,
  .myed save→reopen fidelity (blur modifier, strokes, variables survive),
  duplicate, group, cross-document paste, image import — zero console
  errors.
- CLI renders the browser-saved .myed identically (shared engine).
