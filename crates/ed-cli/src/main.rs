//! ed — headless render/export/validate (spec §12.5). Identical engine to
//! the app: output matches the editor exactly (deterministic CPU path).
//!
//! ```text
//! ed render project.myed --artboard "Cover" --scale 2 -o cover.png
//! ed export project.myed -o out/            # all artboards
//! ed validate project.myed
//! ed thumbnails project.myed -o thumbs/
//! ```

use ed_core::ActorId;
use ed_document::{doc::BlobStore, Document};
use ed_engine::Engine;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn load(path: &str) -> Result<(Document, BlobStore), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let mut blobs = BlobStore::default();
    let doc = ed_io::load_myed(&bytes, ActorId(1), &mut blobs)?;
    Ok((doc, blobs))
}

fn artboard_by_name(doc: &Document, name: Option<&str>) -> Result<ed_core::NodeId, String> {
    let abs = doc.artboards();
    match name {
        None => abs.first().copied().ok_or("document has no artboards".into()),
        Some(n) => abs
            .into_iter()
            .find(|&id| doc.node(id).map(|node| node.name() == n).unwrap_or(false))
            .ok_or(format!("no artboard named {n:?}")),
    }
}

fn render_to_file(
    doc: &Document,
    engine: &mut Engine,
    ab: ed_core::NodeId,
    scale: f64,
    out: &str,
) -> Result<(), String> {
    let pm = engine.render_artboard(doc, ab, scale, true).ok_or("render failed")?;
    let rgba = demultiply(&pm);
    let bytes = if out.ends_with(".jpg") || out.ends_with(".jpeg") {
        ed_io::encode_jpeg(pm.width(), pm.height(), &rgba, 90)?
    } else if out.ends_with(".webp") {
        ed_io::encode_webp(pm.width(), pm.height(), &rgba)?
    } else {
        ed_io::encode_png(pm.width(), pm.height(), &rgba)?
    };
    std::fs::write(out, bytes).map_err(|e| e.to_string())?;
    println!("{out} ({}×{})", pm.width(), pm.height());
    Ok(())
}

fn demultiply(pm: &ed_engine::Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for px in pm.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

fn run(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    match cmd {
        "render" => {
            let path = args.get(1).ok_or("usage: ed render <file.myed> [--artboard NAME] [--scale N] -o out.png")?;
            let (doc, _blobs) = load(path)?;
            let ab = artboard_by_name(&doc, flag(args, "--artboard").as_deref())?;
            let scale: f64 = flag(args, "--scale").map(|s| s.parse().unwrap_or(1.0)).unwrap_or(1.0);
            let out = flag(args, "-o").unwrap_or_else(|| "out.png".into());
            render_to_file(&doc, &mut Engine::new(), ab, scale, &out)
        }
        "export" => {
            let path = args.get(1).ok_or("usage: ed export <file.myed> -o outdir/")?;
            let (doc, _blobs) = load(path)?;
            let outdir = flag(args, "-o").unwrap_or_else(|| "out".into());
            std::fs::create_dir_all(&outdir).map_err(|e| e.to_string())?;
            let mut engine = Engine::new();
            for ab in doc.artboards() {
                let name = doc.node(ab).map(|n| n.name().to_string()).unwrap_or_default();
                let safe: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
                render_to_file(&doc, &mut engine, ab, 1.0, &format!("{outdir}/{safe}.png"))?;
            }
            Ok(())
        }
        "thumbnails" => {
            let path = args.get(1).ok_or("usage: ed thumbnails <file.myed> -o dir/")?;
            let (doc, _blobs) = load(path)?;
            let outdir = flag(args, "-o").unwrap_or_else(|| "thumbs".into());
            std::fs::create_dir_all(&outdir).map_err(|e| e.to_string())?;
            let mut engine = Engine::new();
            for ab in doc.artboards() {
                let rect = doc.artboard_rect(ab).ok_or("bad artboard")?;
                let scale = (256.0 / rect.w.max(rect.h)).min(1.0);
                let name = doc.node(ab).map(|n| n.name().to_string()).unwrap_or_default();
                let safe: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
                render_to_file(&doc, &mut engine, ab, scale, &format!("{outdir}/{safe}.png"))?;
            }
            Ok(())
        }
        "validate" => {
            let path = args.get(1).ok_or("usage: ed validate <file.myed>")?;
            let (doc, _blobs) = load(path)?;
            let mut nodes = 0usize;
            for ab in doc.children_of(None) {
                let mut v = Vec::new();
                doc.walk(*ab, &mut v);
                nodes += v.len();
            }
            // artboard size limits (spec §2.1)
            for ab in doc.artboards() {
                let r = doc.artboard_rect(ab).ok_or("artboard without rect")?;
                if r.w > ed_document::MAX_ARTBOARD_DIM || r.h > ed_document::MAX_ARTBOARD_DIM {
                    return Err(format!("artboard exceeds 16384px limit: {}×{}", r.w, r.h));
                }
            }
            println!(
                "ok: {} artboards, {nodes} nodes, {} palette entries, {} variables, {} history txns",
                doc.artboards().len(),
                doc.palette.len(),
                doc.variables.len(),
                doc.history.len(),
            );
            Ok(())
        }
        _ => {
            println!(
                "ed — headless renderer\n\nUsage:\n  ed render <file.myed> [--artboard NAME] [--scale N] [-o out.png]\n  ed export <file.myed> [-o outdir/]\n  ed thumbnails <file.myed> [-o dir/]\n  ed validate <file.myed>"
            );
            Ok(())
        }
    }
}
