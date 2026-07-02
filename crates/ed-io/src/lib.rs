//! ed-io — codecs and the .myed container (spec §9, §11).

use ed_core::ActorId;
use ed_document::{doc::BlobStore, serialize::Snapshot, Document};
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

pub const MANIFEST_VERSION: u32 = 1;

/// Decode any supported raster format to straight RGBA8 (spec §9 import).
pub fn decode_image(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// Encode straight RGBA8 to PNG (spec §9 export; deterministic).
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or("bad buffer size")?;
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Encode to JPEG with quality (alpha composited over white).
pub fn encode_jpeg(w: u32, h: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for px in rgba.chunks_exact(4) {
        let a = px[3] as u32;
        for c in 0..3 {
            rgb.push(((px[c] as u32 * a + 255 * (255 - a)) / 255) as u8);
        }
    }
    let img = image::RgbImage::from_raw(w, h, rgb).ok_or("bad buffer size")?;
    let mut out = Cursor::new(Vec::new());
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    img.write_with_encoder(enc).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Encode to WebP (lossless).
pub fn encode_webp(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or("bad buffer size")?;
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::WebP).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Pack a document into the .myed zip container (spec §11):
/// `manifest.json` + `document.json` (deflate) + `blobs/<hash>` (stored).
pub fn save_myed(doc: &Document, blobs: &mut BlobStore) -> Result<Vec<u8>, String> {
    let snapshot = doc.to_snapshot(blobs);
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    let deflate = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let store = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let manifest = serde_json::json!({
        "format": MANIFEST_VERSION,
        "generator": "ed 0.1",
        "name": doc.name,
    });
    zw.start_file("manifest.json", deflate).map_err(|e| e.to_string())?;
    zw.write_all(manifest.to_string().as_bytes()).map_err(|e| e.to_string())?;

    zw.start_file("document.json", deflate).map_err(|e| e.to_string())?;
    let doc_json = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    zw.write_all(&doc_json).map_err(|e| e.to_string())?;

    // referenced blobs only (content-addressed, stored uncompressed)
    let mut written = std::collections::HashSet::new();
    let tile_refs = snapshot.tiles.values().flatten().map(|r| r.blob);
    for hash in tile_refs.chain(snapshot.param_blobs.iter().copied()) {
        if written.insert(hash) {
            if let Some(data) = blobs.get(hash) {
                zw.start_file(format!("blobs/{hash}"), store).map_err(|e| e.to_string())?;
                zw.write_all(data).map_err(|e| e.to_string())?;
            }
        }
    }

    let cursor = zw.finish().map_err(|e| e.to_string())?;
    Ok(cursor.into_inner())
}

/// Load a .myed container.
pub fn load_myed(bytes: &[u8], actor: ActorId, blobs: &mut BlobStore) -> Result<Document, String> {
    let mut za = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;

    // blobs first so snapshot tiles resolve
    for i in 0..za.len() {
        let mut f = za.by_index(i).map_err(|e| e.to_string())?;
        if f.name().starts_with("blobs/") {
            let mut data = Vec::new();
            f.read_to_end(&mut data).map_err(|e| e.to_string())?;
            blobs.put(data);
        }
    }

    let mut doc_json = String::new();
    za.by_name("document.json")
        .map_err(|_| "missing document.json".to_string())?
        .read_to_string(&mut doc_json)
        .map_err(|e| e.to_string())?;
    let snapshot: Snapshot = serde_json::from_str(&doc_json).map_err(|e| e.to_string())?;
    Document::from_snapshot(snapshot, actor, blobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed_core::Value;
    use ed_document::NodeKind;
    use std::collections::BTreeMap;

    #[test]
    fn png_roundtrip() {
        let rgba: Vec<u8> = (0..16 * 16 * 4).map(|i| (i % 251) as u8).collect();
        let png = encode_png(16, 16, &rgba).unwrap();
        let (w, h, back) = decode_image(&png).unwrap();
        assert_eq!((w, h), (16, 16));
        assert_eq!(back, rgba, "png is lossless");
    }

    #[test]
    fn jpeg_and_webp_encode() {
        let rgba = vec![128u8; 8 * 8 * 4];
        assert!(!encode_jpeg(8, 8, &rgba, 90).unwrap().is_empty());
        assert!(!encode_webp(8, 8, &rgba).unwrap().is_empty());
    }

    #[test]
    fn myed_roundtrip_with_bitmap() {
        let mut blobs = BlobStore::default();
        let actor = ActorId(1);
        let mut doc = Document::with_artboard(actor, "proj", 640.0, 480.0, &blobs);
        let ab = doc.artboards()[0];
        doc.begin_txn("build");
        let mut params = BTreeMap::new();
        params.insert("x".into(), Value::F64(12.0));
        let shape = doc.create_node(NodeKind::Shape, Some(ab), params, &blobs).unwrap();
        let bmp = doc.create_node(NodeKind::Bitmap, Some(ab), BTreeMap::new(), &blobs).unwrap();
        doc.commit_txn();
        {
            let bm = doc.bitmap_mut(bmp).unwrap();
            bm.width = 300;
            bm.height = 200;
            bm.set_pixel(299, 199, [1, 2, 3, 255]);
        }

        let zip = save_myed(&doc, &mut blobs).unwrap();
        assert_eq!(&zip[..2], b"PK");

        let mut blobs2 = BlobStore::default();
        let doc2 = load_myed(&zip, actor, &mut blobs2).unwrap();
        assert_eq!(doc2.artboards().len(), 1);
        assert_eq!(doc2.param_f64(doc2.node(shape).unwrap(), "x", 0.0), 12.0);
        let bm = doc2.node(bmp).unwrap().bitmap.as_ref().unwrap();
        assert_eq!((bm.width, bm.height), (300, 200));
        assert_eq!(bm.get_pixel(299, 199), [1, 2, 3, 255]);
    }
}
