//! ed-wasm — the wasm-bindgen boundary (spec §12.3). The core worker owns
//! a `Session`; JS sends JSON commands and blits the returned frame to an
//! OffscreenCanvas. Pixels stay in wasm memory except the final blit copy.

use ed_tools::Session;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct EditorSession {
    session: Session,
    frame: Vec<u8>,
}

#[wasm_bindgen]
impl EditorSession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> EditorSession {
        console_error_panic_hook();
        EditorSession { session: Session::new(), frame: Vec::new() }
    }

    /// Dispatch a JSON command; returns `{"ok":true,...}` or `{"error":..}`.
    pub fn command(&mut self, json: &str) -> String {
        match self.session.command(json) {
            Ok(v) => serde_json::json!({ "ok": true, "result": v }).to_string(),
            Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
        }
    }

    /// Full UI state mirror (spec §12.1 read models).
    pub fn state(&self) -> String {
        self.session.state_json().to_string()
    }

    /// True when the document/view changed since the last rendered frame.
    pub fn needs_frame(&self) -> bool {
        self.session.needs_frame()
    }

    /// Render the viewport; frame bytes stay in wasm memory.
    pub fn render(&mut self, width: u32, height: u32, ants_phase: f32) {
        self.frame = self.session.render_frame(width, height, ants_phase);
    }

    pub fn frame_ptr(&self) -> *const u8 {
        self.frame.as_ptr()
    }

    pub fn frame_len(&self) -> usize {
        self.frame.len()
    }

    // ---------------------------------------------------------- binary io

    pub fn import_image(&mut self, bytes: &[u8], name: &str) -> String {
        match self.session.import_image(bytes, name) {
            Ok(()) => "{\"ok\":true}".into(),
            Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
        }
    }

    /// Export an artboard; empty vec on failure (use `last_error`).
    pub fn export_artboard(&mut self, artboard: usize, scale: f64, format: &str) -> Vec<u8> {
        self.session.export(artboard, scale, format).unwrap_or_default()
    }

    /// PNG flavor for system-clipboard copy (spec §10.7).
    pub fn copy_as_png(&mut self) -> Vec<u8> {
        self.session.copy_as_png().unwrap_or_default()
    }

    pub fn save_myed(&mut self) -> Vec<u8> {
        self.session.save_myed().unwrap_or_default()
    }

    pub fn open_myed(&mut self, bytes: &[u8], name: &str) -> String {
        match self.session.open_myed(bytes, name) {
            Ok(()) => "{\"ok\":true}".into(),
            Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
        }
    }
}

impl Default for EditorSession {
    fn default() -> Self {
        Self::new()
    }
}

fn console_error_panic_hook() {
    use std::sync::Once;
    static SET_HOOK: Once = Once::new();
    SET_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            web_log(&format!("wasm panic: {info}"));
        }));
    });
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn web_log(s: &str);
}
