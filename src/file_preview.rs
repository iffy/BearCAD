//! File preview thumbnails for `.bearcad` documents (#1223, #1290).
//!
//! On save, render a zoom-to-fit view of the **Home** camera orientation as a PNG, embed it
//! in the SQLite `blobs` table, and publish it to the OS so Finder (and free-desktop file
//! managers) can show the model in the icon/preview slot. A black silhouette outline keeps
//! the model readable on both light and dark backgrounds.
//!
//! The macOS QuickLook Preview Extension shows the same PNG (#1790 wanted the colorful
//! screenshot back, not a gray mesh). Legacy `preview_stl` blobs are deleted on save.

use crate::camera::Camera;
use crate::extrude::{body_solid_mesh, document_world_bounds, SolidMesh};
use crate::gpu_viewport::body_material_fill;
use crate::model::Document;
use egui::{Pos2, Rect};
use glam::{Mat4, Vec3};

/// Pixel size of the embedded / OS thumbnail (square).
pub const PREVIEW_SIZE: u32 = 1024;
/// `blobs.kind` for the Home-orientation PNG thumbnail.
pub const PREVIEW_PNG_BLOB_KIND: &str = "preview_png";
/// `blobs.kind` of the retired QuickLook STL snapshot: legacy rows are deleted on save
/// so files saved by older builds shrink back down (#1790).
pub const PREVIEW_STL_BLOB_KIND: &str = "preview_stl";
/// Silhouette outline radius in pixels — enough to read on light and dark backgrounds.
const OUTLINE_RADIUS: i32 = 3;
/// Extra margin beyond zoom-to-fit so the black outline is not clipped at the image edge.
const OUTLINE_FRAME_PAD: f32 = 1.08;

/// RGBA preview image.
#[derive(Clone, Debug)]
pub struct PreviewRgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl PreviewRgba {
    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        {
            let encoder = image::codecs::png::PngEncoder::new(&mut buf);
            use image::ImageEncoder;
            encoder
                .write_image(
                    &self.pixels,
                    self.width,
                    self.height,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| format!("encode preview png: {e}"))?;
        }
        Ok(buf)
    }
}

/// Render a Home-orientation, zoom-to-fit preview of `doc`. `None` when there is nothing to draw.
pub fn render_document_preview(doc: &Document) -> Option<PreviewRgba> {
    render_document_preview_sized(doc, PREVIEW_SIZE)
}

/// Like [`render_document_preview`] with an explicit square size (tests use smaller images).
pub fn render_document_preview_sized(doc: &Document, size: u32) -> Option<PreviewRgba> {
    let size = size.max(32);
    let (min, max) = document_world_bounds(doc)?;
    let meshes = collect_preview_meshes(doc);
    if meshes.is_empty() {
        // Sketch-only: still frame, but paint construction geometry as lines.
        return render_sketch_lines_preview(doc, size, min, max);
    }
    Some(rasterize_meshes(&meshes, min, max, size))
}

/// Encode a document preview as PNG bytes, or `None` when empty.
pub fn document_preview_png(doc: &Document) -> Option<Vec<u8>> {
    let preview = render_document_preview(doc)?;
    preview.encode_png().ok()
}

/// Write a preview PNG of `doc` to `path` (scriptable via `bearcad.export_preview`).
pub fn export_preview_png(doc: &Document, path: &str) -> Result<(), String> {
    let png = document_preview_png(doc).ok_or_else(|| {
        "export_preview: document has no geometry to preview".to_string()
    })?;
    std::fs::write(path, png).map_err(|e| format!("export_preview: write {path}: {e}"))
}

/// After a successful SQLite save: embed the preview in `meta` and publish it to the OS.
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_preview_after_save(path: &str, doc: &Document) {
    let Some(png) = document_preview_png(doc) else {
        // Empty model: clear any stale custom icon so Finder doesn't show an old thumbnail.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            clear_os_preview(path);
        }));
        clear_embedded_preview(path);
        return;
    };
    if let Err(e) = embed_preview_png(path, &png) {
        crate::diag::warn(format!("preview embed failed for {path}: {e}"));
    }
    // #1790: the STL QuickLook snapshot is retired — purge legacy blobs so old files
    // slim down on their next save.
    let _ = clear_embedded_preview_stl(path);
    // Icon attach is best-effort: a panic here must not unwind through save (#1339).
    // SIGBUS from ImageIO cannot be caught — `apply_os_preview` must not call it.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply_os_preview(path, &png))) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => crate::diag::warn(format!("preview OS publish failed for {path}: {e}")),
        Err(_) => crate::diag::warn(format!("preview OS publish panicked for {path}")),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn attach_preview_after_save(_path: &str, _doc: &Document) {}

/// Read the embedded preview PNG from a `.bearcad` SQLite file (tests / tooling).
#[cfg(all(not(target_arch = "wasm32"), test))]
pub fn load_embedded_preview_png(path: &str) -> Option<Vec<u8>> {
    crate::storage::load_preview_blob(path, PREVIEW_PNG_BLOB_KIND)
}

/// Read the embedded preview STL from a `.bearcad` SQLite file (tests / tooling / QL).
#[cfg(all(not(target_arch = "wasm32"), test))]
pub fn load_embedded_preview_stl(path: &str) -> Option<Vec<u8>> {
    crate::storage::load_preview_blob(path, PREVIEW_STL_BLOB_KIND)
}

// ── mesh collection ──────────────────────────────────────────────────────────

struct ColoredMesh {
    mesh: SolidMesh,
    color: [u8; 3],
}

fn collect_preview_meshes(doc: &Document) -> Vec<ColoredMesh> {
    let mut out = Vec::new();
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        let Some(mesh) = body_solid_mesh(doc, bi) else {
            continue;
        };
        if mesh.is_empty() {
            continue;
        }
        let c = body_material_fill(doc, body);
        out.push(ColoredMesh {
            mesh,
            color: [c.r(), c.g(), c.b()],
        });
    }
    out
}

// ── camera ───────────────────────────────────────────────────────────────────

/// Home orientation, zoom-to-fit, with a little extra pad so the outline has room.
fn preview_camera(min: Vec3, max: Vec3, size: u32) -> (Camera, Rect) {
    let mut cam = Camera::default(); // default = Home isometric orientation
    let aspect = 1.0;
    cam.frame_bounds_instant(min, max, aspect);
    // frame_bounds_instant already multiplies by ZOOM_FIT_MARGIN; pad a bit more for outline.
    cam.distance *= OUTLINE_FRAME_PAD;
    let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(size as f32, size as f32));
    (cam, viewport)
}

// ── software rasterizer ──────────────────────────────────────────────────────

fn rasterize_meshes(meshes: &[ColoredMesh], min: Vec3, max: Vec3, size: u32) -> PreviewRgba {
    let (cam, viewport) = preview_camera(min, max, size);
    let vp = cam.view_proj(viewport);
    let eye = cam.eye();
    let light = Vec3::new(0.45, 0.35, 0.82).normalize();

    let n = (size as usize) * (size as usize);
    let mut zbuf = vec![f32::INFINITY; n];
    let mut pixels = vec![0u8; n * 4];

    for cm in meshes {
        for tri in &cm.mesh.triangles {
            let nrm = {
                let e1 = tri[1] - tri[0];
                let e2 = tri[2] - tri[0];
                e1.cross(e2).normalize_or_zero()
            };
            // Back-face cull in view space (toward eye).
            let mid = (tri[0] + tri[1] + tri[2]) / 3.0;
            if nrm.dot((eye - mid).normalize_or_zero()) <= 0.0 {
                continue;
            }
            let shade = (0.32 + 0.68 * nrm.dot(light).max(0.0)).clamp(0.0, 1.0);
            let r = (cm.color[0] as f32 * shade) as u8;
            let g = (cm.color[1] as f32 * shade) as u8;
            let b = (cm.color[2] as f32 * shade) as u8;

            let Some(p0) = project_depth(tri[0], &vp, size) else {
                continue;
            };
            let Some(p1) = project_depth(tri[1], &vp, size) else {
                continue;
            };
            let Some(p2) = project_depth(tri[2], &vp, size) else {
                continue;
            };
            fill_triangle(
                &mut pixels,
                &mut zbuf,
                size as i32,
                p0,
                p1,
                p2,
                [r, g, b, 255],
            );
        }
    }

    apply_black_outline(&mut pixels, size as i32, OUTLINE_RADIUS);

    PreviewRgba {
        width: size,
        height: size,
        pixels,
    }
}

fn project_depth(world: Vec3, vp: &Mat4, size: u32) -> Option<(f32, f32, f32)> {
    let clip = *vp * world.extend(1.0);
    if clip.w <= 1e-5 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    // Keep a little slack past the edges so partially-off triangles still contribute.
    if ndc.x < -1.2 || ndc.x > 1.2 || ndc.y < -1.2 || ndc.y > 1.2 {
        return None;
    }
    let x = (ndc.x * 0.5 + 0.5) * size as f32;
    let y = (1.0 - (ndc.y * 0.5 + 0.5)) * size as f32;
    let z = ndc.z;
    Some((x, y, z))
}

fn fill_triangle(
    pixels: &mut [u8],
    zbuf: &mut [f32],
    size: i32,
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    c: (f32, f32, f32),
    color: [u8; 4],
) {
    let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as i32;
    let max_x = a.0.max(b.0).max(c.0).ceil().min(size as f32 - 1.0) as i32;
    let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as i32;
    let max_y = a.1.max(b.1).max(c.1).ceil().min(size as f32 - 1.0) as i32;
    if min_x > max_x || min_y > max_y {
        return;
    }

    let area = edge(a.0, a.1, b.0, b.1, c.0, c.1);
    if area.abs() < 1e-6 {
        return;
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = edge(b.0, b.1, c.0, c.1, px, py) / area;
            let w1 = edge(c.0, c.1, a.0, a.1, px, py) / area;
            let w2 = edge(a.0, a.1, b.0, b.1, px, py) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
            let idx = (y * size + x) as usize;
            if z >= zbuf[idx] {
                continue;
            }
            zbuf[idx] = z;
            let o = idx * 4;
            pixels[o] = color[0];
            pixels[o + 1] = color[1];
            pixels[o + 2] = color[2];
            pixels[o + 3] = color[3];
        }
    }
}

fn edge(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

/// Paint a black ring around the opaque silhouette so the model reads on light *and* dark UIs.
fn apply_black_outline(pixels: &mut [u8], size: i32, radius: i32) {
    let n = (size * size) as usize;
    let mut mask = vec![false; n];
    for i in 0..n {
        mask[i] = pixels[i * 4 + 3] > 0;
    }
    let mut outline = vec![false; n];
    let r2 = radius * radius;
    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) as usize;
            if mask[i] {
                continue;
            }
            'search: for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy > r2 {
                        continue;
                    }
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx < 0 || ny < 0 || nx >= size || ny >= size {
                        continue;
                    }
                    if mask[(ny * size + nx) as usize] {
                        outline[i] = true;
                        break 'search;
                    }
                }
            }
        }
    }
    for i in 0..n {
        if outline[i] {
            let o = i * 4;
            pixels[o] = 0;
            pixels[o + 1] = 0;
            pixels[o + 2] = 0;
            pixels[o + 3] = 255;
        }
    }
}

// ── sketch-only fallback ─────────────────────────────────────────────────────

fn render_sketch_lines_preview(
    doc: &Document,
    size: u32,
    min: Vec3,
    max: Vec3,
) -> Option<PreviewRgba> {
    use crate::face::{local_to_world, sketch_geometry_frame};

    let (cam, viewport) = preview_camera(min, max, size);
    let vp = cam.view_proj(viewport);
    let n = (size as usize) * (size as usize);
    let mut pixels = vec![0u8; n * 4];
    let mut painted = false;

    let stroke = |pixels: &mut [u8], a: Vec3, b: Vec3, color: [u8; 4]| {
        let Some((x0, y0, _)) = project_depth(a, &vp, size) else {
            return false;
        };
        let Some((x1, y1, _)) = project_depth(b, &vp, size) else {
            return false;
        };
        draw_line(pixels, size as i32, x0, y0, x1, y1, color);
        true
    };

    for line in doc.lines.values().filter(|l| !l.construction) {
        let Some(frame) = sketch_geometry_frame(doc, line.sketch) else {
            continue;
        };
        let samples = line.sample_local(crate::model::BEZIER_SEGMENTS);
        for w in samples.windows(2) {
            let a = local_to_world(&frame, w[0].0, w[0].1);
            let b = local_to_world(&frame, w[1].0, w[1].1);
            if stroke(&mut pixels, a, b, [40, 40, 40, 255]) {
                painted = true;
            }
        }
    }
    for circle in doc.circles.values().filter(|c| !c.construction) {
        let Some(frame) = sketch_geometry_frame(doc, circle.sketch) else {
            continue;
        };
        const SEGS: usize = 48;
        for i in 0..SEGS {
            let t0 = (i as f32 / SEGS as f32) * std::f32::consts::TAU;
            let t1 = ((i + 1) as f32 / SEGS as f32) * std::f32::consts::TAU;
            let a = local_to_world(
                &frame,
                circle.cx + circle.r * t0.cos(),
                circle.cy + circle.r * t0.sin(),
            );
            let b = local_to_world(
                &frame,
                circle.cx + circle.r * t1.cos(),
                circle.cy + circle.r * t1.sin(),
            );
            if stroke(&mut pixels, a, b, [40, 40, 40, 255]) {
                painted = true;
            }
        }
    }

    if !painted {
        return None;
    }
    apply_black_outline(&mut pixels, size as i32, OUTLINE_RADIUS);
    Some(PreviewRgba {
        width: size,
        height: size,
        pixels,
    })
}

fn draw_line(pixels: &mut [u8], size: i32, x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 4]) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs()).ceil() as i32).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (x0 + (x1 - x0) * t).round() as i32;
        let y = (y0 + (y1 - y0) * t).round() as i32;
        // 2px thick stroke for visibility at small sizes.
        for dy in -1..=1 {
            for dx in -1..=1 {
                let px = x + dx;
                let py = y + dy;
                if px < 0 || py < 0 || px >= size || py >= size {
                    continue;
                }
                let o = ((py * size + px) * 4) as usize;
                pixels[o] = color[0];
                pixels[o + 1] = color[1];
                pixels[o + 2] = color[2];
                pixels[o + 3] = color[3];
            }
        }
    }
}

// ── embed in SQLite ──────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn embed_preview_png(path: &str, png: &[u8]) -> Result<(), String> {
    crate::storage::upsert_preview_blob(path, PREVIEW_PNG_BLOB_KIND, png)
}

#[cfg(not(target_arch = "wasm32"))]
fn clear_embedded_preview(path: &str) {
    let _ = crate::storage::delete_preview_blob(path, PREVIEW_PNG_BLOB_KIND);
}

#[cfg(not(target_arch = "wasm32"))]
fn clear_embedded_preview_stl(path: &str) {
    let _ = crate::storage::delete_preview_blob(path, PREVIEW_STL_BLOB_KIND);
}

// ── OS-side thumbnail / custom icon ──────────────────────────────────────────

/// Publish `png` as the file's OS thumbnail/icon so Finder / free-desktop show it.
pub fn apply_os_preview(path: &str, png: &[u8]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return apply_macos_icon(path, png);
    }
    #[cfg(target_os = "linux")]
    {
        return apply_linux_thumbnail(path, png);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (path, png);
        // Windows needs a shell extension for custom document thumbnails — not uniform/easy.
        Ok(())
    }
}

fn clear_os_preview(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = apply_macos_icon_clear(path);
    }
    #[cfg(target_os = "linux")]
    {
        let _ = remove_linux_thumbnail(path);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
    }
}

/// Finder custom-icon resource ID (`kCustomIconResource`).
#[cfg(any(test, target_os = "macos"))]
const CUSTOM_ICON_RESOURCE_ID: i16 = -16455;
/// `FileInfo.finderFlags` bit: this file has a custom icon.
#[cfg(any(test, target_os = "macos"))]
const K_HAS_CUSTOM_ICON: u16 = 0x0400;
#[cfg(any(test, target_os = "macos"))]
const RESOURCE_DATA_OFFSET: u32 = 256;
/// Modern ICNS PNG slots Finder scales from (1024 / 512 / 256 / 128).
#[cfg(any(test, target_os = "macos"))]
const ICNS_PNG_SIZES: [(&[u8; 4], u32); 4] = [(b"ic14", 1024), (b"ic09", 512), (b"ic08", 256), (b"ic07", 128)];

/// Pack a PNG (any size) into a modern ICNS. Pure Rust — no ImageIO (#1339).
#[cfg(any(test, target_os = "macos"))]
fn encode_icns_from_png(png: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(png).map_err(|e| format!("decode preview PNG: {e}"))?;
    let rgba = img.to_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return Err("preview icon has zero size".into());
    }

    let mut entries: Vec<(&[u8; 4], Vec<u8>)> = Vec::with_capacity(ICNS_PNG_SIZES.len());
    for &(tag, dim) in &ICNS_PNG_SIZES {
        let bytes = if rgba.width() == dim && rgba.height() == dim {
            png.to_vec()
        } else {
            let resized = image::imageops::resize(
                &rgba,
                dim,
                dim,
                image::imageops::FilterType::Triangle,
            );
            encode_rgba_png(&resized)?
        };
        entries.push((tag, bytes));
    }
    pack_icns(&entries)
}

#[cfg(any(test, target_os = "macos"))]
fn encode_rgba_png(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        use image::ImageEncoder;
        encoder
            .write_image(
                img.as_raw(),
                img.width(),
                img.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| format!("encode icon png: {e}"))?;
    }
    Ok(buf)
}

#[cfg(any(test, target_os = "macos"))]
fn pack_icns(entries: &[(&[u8; 4], Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut total = 8u32;
    for (_, data) in entries {
        total = total
            .checked_add(8)
            .and_then(|t| t.checked_add(data.len() as u32))
            .ok_or_else(|| "icns too large".to_string())?;
    }
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&total.to_be_bytes());
    for (tag, data) in entries {
        let elen = 8u32 + data.len() as u32;
        out.extend_from_slice(*tag);
        out.extend_from_slice(&elen.to_be_bytes());
        out.extend_from_slice(data);
    }
    debug_assert_eq!(out.len(), total as usize);
    Ok(out)
}

/// Resource fork containing a single `icns` resource with ID -16455.
#[cfg(any(test, target_os = "macos"))]
fn macos_custom_icon_resource_fork(icns: &[u8]) -> Vec<u8> {
    let data_len = 4u32 + icns.len() as u32;
    let map_offset = RESOURCE_DATA_OFFSET + data_len;
    // 16 reserved + 4 next + 2 refNum + 2 attrs + 2 typeList + 2 nameList
    // + 2 (type count-1) + 8 (one type) + 12 (one ref) = 50
    const MAP_LEN: u32 = 50;
    const TYPE_LIST_OFF: u16 = 28;
    const NAME_LIST_OFF: u16 = 50;
    const REF_LIST_OFF: u16 = 10;

    let mut out = vec![0u8; RESOURCE_DATA_OFFSET as usize];
    out[0..4].copy_from_slice(&RESOURCE_DATA_OFFSET.to_be_bytes());
    out[4..8].copy_from_slice(&map_offset.to_be_bytes());
    out[8..12].copy_from_slice(&data_len.to_be_bytes());
    out[12..16].copy_from_slice(&MAP_LEN.to_be_bytes());

    out.extend_from_slice(&(icns.len() as u32).to_be_bytes());
    out.extend_from_slice(icns);

    // Resource map. First 16 bytes stay zero (in-memory header copy).
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&TYPE_LIST_OFF.to_be_bytes());
    out.extend_from_slice(&NAME_LIST_OFF.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // number of types − 1
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&0u16.to_be_bytes()); // resources of this type − 1
    out.extend_from_slice(&REF_LIST_OFF.to_be_bytes());
    out.extend_from_slice(&CUSTOM_ICON_RESOURCE_ID.to_be_bytes());
    out.extend_from_slice(&0xFFFFu16.to_be_bytes());
    out.push(0);
    out.extend_from_slice(&[0, 0, 0]); // data offset inside the data section
    out.extend_from_slice(&0u32.to_be_bytes());
    debug_assert_eq!(out.len() as u32, map_offset + MAP_LEN);
    out
}

/// 32-byte `com.apple.FinderInfo` with `kHasCustomIcon` set.
#[cfg(any(test, target_os = "macos"))]
fn macos_finder_info_with_custom_icon(existing: Option<&[u8]>) -> [u8; 32] {
    macos_finder_info_set_custom_icon(existing, true)
}

#[cfg(any(test, target_os = "macos"))]
fn macos_finder_info_set_custom_icon(existing: Option<&[u8]>, on: bool) -> [u8; 32] {
    let mut info = [0u8; 32];
    if let Some(e) = existing {
        let n = e.len().min(32);
        info[..n].copy_from_slice(&e[..n]);
    }
    let mut flags = u16::from_be_bytes([info[8], info[9]]);
    if on {
        flags |= K_HAS_CUSTOM_ICON;
    } else {
        flags &= !K_HAS_CUSTOM_ICON;
    }
    info[8..10].copy_from_slice(&flags.to_be_bytes());
    info
}

/// Attach a Finder custom icon without `NSWorkspace.setIcon`.
///
/// `setIcon:forFile:` builds an ICNS through ImageIO `PNGWritePlugin` and SIGBUS
/// (`0xbad4007`) when a bundled libpng collides with ImageIO (#1339 / #1304 / egui#7155).
/// Write the ICNS ourselves and set the resource fork + FinderInfo flag instead.
#[cfg(target_os = "macos")]
fn apply_macos_icon(path: &str, png: &[u8]) -> Result<(), String> {
    let icns = encode_icns_from_png(png)?;
    let fork = macos_custom_icon_resource_fork(&icns);
    write_macos_resource_fork(path, &fork)?;
    let existing = macos_get_xattr(path, "com.apple.FinderInfo");
    let info = macos_finder_info_with_custom_icon(existing.as_deref());
    macos_set_xattr(path, "com.apple.FinderInfo", &info)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_macos_icon_clear(path: &str) -> Result<(), String> {
    let rsrc = macos_resource_fork_path(path);
    let _ = std::fs::remove_file(&rsrc);
    if let Some(existing) = macos_get_xattr(path, "com.apple.FinderInfo") {
        let info = macos_finder_info_set_custom_icon(Some(&existing), false);
        if info.iter().all(|&b| b == 0) {
            let _ = macos_remove_xattr(path, "com.apple.FinderInfo");
        } else {
            let _ = macos_set_xattr(path, "com.apple.FinderInfo", &info);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_resource_fork_path(path: &str) -> std::path::PathBuf {
    std::path::Path::new(path).join("..namedfork").join("rsrc")
}

#[cfg(target_os = "macos")]
fn write_macos_resource_fork(path: &str, data: &[u8]) -> Result<(), String> {
    std::fs::write(macos_resource_fork_path(path), data)
        .map_err(|e| format!("write resource fork: {e}"))
}

#[cfg(target_os = "macos")]
fn macos_get_xattr(path: &str, name: &str) -> Option<Vec<u8>> {
    use std::os::raw::{c_char, c_int, c_void};
    extern "C" {
        fn getxattr(
            path: *const c_char,
            name: *const c_char,
            value: *mut c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> isize;
    }
    let c_path = std::ffi::CString::new(path).ok()?;
    let c_name = std::ffi::CString::new(name).ok()?;
    let needed = unsafe {
        getxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    if needed <= 0 {
        return None;
    }
    let mut buf = vec![0u8; needed as usize];
    let n = unsafe {
        getxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            buf.len(),
            0,
            0,
        )
    };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(buf)
}

#[cfg(target_os = "macos")]
fn macos_set_xattr(path: &str, name: &str, value: &[u8]) -> Result<(), String> {
    use std::os::raw::{c_char, c_int, c_void};
    extern "C" {
        fn setxattr(
            path: *const c_char,
            name: *const c_char,
            value: *const c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> c_int;
    }
    let c_path = std::ffi::CString::new(path).map_err(|_| "path contains NUL".to_string())?;
    let c_name = std::ffi::CString::new(name).map_err(|_| "xattr name contains NUL".to_string())?;
    let rc = unsafe {
        setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr() as *const c_void,
            value.len(),
            0,
            0,
        )
    };
    if rc != 0 {
        return Err(format!(
            "setxattr {name}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_remove_xattr(path: &str, name: &str) -> Result<(), String> {
    use std::os::raw::{c_char, c_int};
    extern "C" {
        fn removexattr(path: *const c_char, name: *const c_char, options: c_int) -> c_int;
    }
    let c_path = std::ffi::CString::new(path).map_err(|_| "path contains NUL".to_string())?;
    let c_name = std::ffi::CString::new(name).map_err(|_| "xattr name contains NUL".to_string())?;
    let rc = unsafe { removexattr(c_path.as_ptr(), c_name.as_ptr(), 0) };
    if rc != 0 {
        return Err(format!(
            "removexattr {name}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// FreeDesktop thumbnail cache so GNOME/KDE file managers show the preview.
#[cfg(target_os = "linux")]
fn apply_linux_thumbnail(path: &str, png: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::path::PathBuf;

    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let uri = format!("file://{}", abs.display());
    let hash = md5_hex(uri.as_bytes());

    let home = std::env::var_os("HOME").ok_or_else(|| "HOME not set".to_string())?;
    // Prefer large (256) then normal (128) — write both from our 1024 PNG scaled down.
    for (subdir, dim) in [("large", 256u32), ("normal", 128u32)] {
        let dir = PathBuf::from(&home)
            .join(".cache/thumbnails")
            .join(subdir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dest = dir.join(format!("{hash}.png"));
        let img = image::load_from_memory(png).map_err(|e| e.to_string())?;
        let resized = img.resize_exact(dim, dim, image::imageops::FilterType::Lanczos3);
        let mut bytes = Vec::new();
        resized
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        // FreeDesktop wants tEXt chunks for URI/mtime; plain PNG still shows in many managers.
        let mut f = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_linux_thumbnail(path: &str) -> Result<(), String> {
    use std::path::PathBuf;
    let Ok(abs) = std::fs::canonicalize(path) else {
        return Ok(());
    };
    let uri = format!("file://{}", abs.display());
    let hash = md5_hex(uri.as_bytes());
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    for subdir in ["large", "normal"] {
        let dest = PathBuf::from(&home)
            .join(".cache/thumbnails")
            .join(subdir)
            .join(format!("{hash}.png"));
        let _ = std::fs::remove_file(dest);
    }
    Ok(())
}

/// Minimal MD5 hex digest for FreeDesktop thumbnail filenames (RFC 1321).
#[cfg(target_os = "linux")]
fn md5_hex(data: &[u8]) -> String {
    // Tiny public-domain MD5 — only used for the FreeDesktop cache key.
    fn f(x: u32, y: u32, z: u32) -> u32 {
        (x & y) | (!x & z)
    }
    fn g(x: u32, y: u32, z: u32) -> u32 {
        (x & z) | (y & !z)
    }
    fn h(x: u32, y: u32, z: u32) -> u32 {
        x ^ y ^ z
    }
    fn i(x: u32, y: u32, z: u32) -> u32 {
        y ^ (x | !z)
    }
    fn op(a: u32, b: u32, c: u32, d: u32, x: u32, s: u32, ac: u32, fun: fn(u32, u32, u32) -> u32) -> u32 {
        a.wrapping_add(fun(b, c, d))
            .wrapping_add(x)
            .wrapping_add(ac)
            .rotate_left(s)
            .wrapping_add(b)
    }

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    for chunk in msg.chunks_exact(64) {
        let mut x = [0u32; 16];
        for (i, w) in x.iter_mut().enumerate() {
            *w = u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        // Use :literal (not :expr) for k/s/ac — adjacent expr fragments are a hard
        // error on stable (`$k:expr` followed by `$s:expr` is not allowed).
        macro_rules! round {
            ($fun:ident; $($a:ident $b:ident $c:ident $d:ident $k:literal $s:literal $ac:literal),* $(,)?) => {
                $( $a = op($a, $b, $c, $d, x[$k], $s, $ac, $fun); )*
            };
        }
        round!(f;
            a b c d 0 7 0xd76aa478, d a b c 1 12 0xe8c7b756, c d a b 2 17 0x242070db, b c d a 3 22 0xc1bdceee,
            a b c d 4 7 0xf57c0faf, d a b c 5 12 0x4787c62a, c d a b 6 17 0xa8304613, b c d a 7 22 0xfd469501,
            a b c d 8 7 0x698098d8, d a b c 9 12 0x8b44f7af, c d a b 10 17 0xffff5bb1, b c d a 11 22 0x895cd7be,
            a b c d 12 7 0x6b901122, d a b c 13 12 0xfd987193, c d a b 14 17 0xa679438e, b c d a 15 22 0x49b40821,
        );
        round!(g;
            a b c d 1 5 0xf61e2562, d a b c 6 9 0xc040b340, c d a b 11 14 0x265e5a51, b c d a 0 20 0xe9b6c7aa,
            a b c d 5 5 0xd62f105d, d a b c 10 9 0x02441453, c d a b 15 14 0xd8a1e681, b c d a 4 20 0xe7d3fbc8,
            a b c d 9 5 0x21e1cde6, d a b c 14 9 0xc33707d6, c d a b 3 14 0xf4d50d87, b c d a 8 20 0x455a14ed,
            a b c d 13 5 0xa9e3e905, d a b c 2 9 0xfcefa3f8, c d a b 7 14 0x676f02d9, b c d a 12 20 0x8d2a4c8a,
        );
        round!(h;
            a b c d 5 4 0xfffa3942, d a b c 8 11 0x8771f681, c d a b 11 16 0x6d9d6122, b c d a 14 23 0xfde5380c,
            a b c d 1 4 0xa4beea44, d a b c 4 11 0x4bdecfa9, c d a b 7 16 0xf6bb4b60, b c d a 10 23 0xbebfbc70,
            a b c d 13 4 0x289b7ec6, d a b c 0 11 0xeaa127fa, c d a b 3 16 0xd4ef3085, b c d a 6 23 0x04881d05,
            a b c d 9 4 0xd9d4d039, d a b c 12 11 0xe6db99e5, c d a b 15 16 0x1fa27cf8, b c d a 2 23 0xc4ac5665,
        );
        round!(i;
            a b c d 0 6 0xf4292244, d a b c 7 10 0x432aff97, c d a b 14 15 0xab9423a7, b c d a 5 21 0xfc93a039,
            a b c d 12 6 0x655b59c3, d a b c 3 10 0x8f0ccc92, c d a b 10 15 0xffeff47d, b c d a 1 21 0x85845dd1,
            a b c d 8 6 0x6fa87e4f, d a b c 15 10 0xfe2ce6e0, c d a b 6 15 0xa3014314, b c d a 13 21 0x4e0811a1,
            a b c d 4 6 0xf7537e82, d a b c 11 10 0xbd3af235, c d a b 2 15 0x2ad7d2bb, b c d a 9 21 0xeb86d391,
        );
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out.iter().map(|b| format!("{b:02x}")).collect()
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Body, BodySource, Document, ImportedMesh};
    use glam::Vec3;

    /// Minimal cube as triangle soup — no kernel, no extrusion schema surprises.
    fn cube_mesh() -> SolidMesh {
        let p = |x, y, z| Vec3::new(x, y, z);
        let mut triangles = Vec::new();
        let faces = [
            // +Z
            [p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.)],
            [p(0., 0., 1.), p(1., 1., 1.), p(0., 1., 1.)],
            // -Z
            [p(0., 0., 0.), p(1., 1., 0.), p(1., 0., 0.)],
            [p(0., 0., 0.), p(0., 1., 0.), p(1., 1., 0.)],
            // +Y
            [p(0., 1., 0.), p(0., 1., 1.), p(1., 1., 1.)],
            [p(0., 1., 0.), p(1., 1., 1.), p(1., 1., 0.)],
            // -Y
            [p(0., 0., 0.), p(1., 0., 0.), p(1., 0., 1.)],
            [p(0., 0., 0.), p(1., 0., 1.), p(0., 0., 1.)],
            // +X
            [p(1., 0., 0.), p(1., 1., 0.), p(1., 1., 1.)],
            [p(1., 0., 0.), p(1., 1., 1.), p(1., 0., 1.)],
            // -X
            [p(0., 0., 0.), p(0., 1., 1.), p(0., 1., 0.)],
            [p(0., 0., 0.), p(0., 0., 1.), p(0., 1., 1.)],
        ];
        for mut tri in faces {
            for v in tri.iter_mut() {
                *v *= 40.0;
            }
            triangles.push(tri);
        }
        SolidMesh { triangles }
    }

    fn cube_document() -> Document {
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(ImportedMesh {
            triangles: cube_mesh().triangles,
            source_name: "cube".into(),
            step_bytes: None,
        });
        doc.bodies.insert(Body {
            source: BodySource::Imported(mesh),
            material: None,
            name: Some("cube".into()),
            shadow: false,
        });
        doc
    }

    #[test]
    fn empty_document_has_no_preview() {
        let doc = Document::default();
        assert!(render_document_preview(&doc).is_none());
        assert!(document_preview_png(&doc).is_none());
    }

    #[test]
    fn cube_preview_has_content_and_black_outline() {
        let meshes = [ColoredMesh {
            mesh: cube_mesh(),
            color: [150, 168, 196],
        }];
        let bounds = meshes[0].mesh.bounds().unwrap();
        let img = rasterize_meshes(&meshes, bounds.0, bounds.1, 128);

        let opaque: Vec<(usize, usize)> = (0..128)
            .flat_map(|y| (0..128).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let o = (y * 128 + x) * 4;
                img.pixels[o + 3] > 0
            })
            .collect();
        assert!(
            opaque.len() > 200,
            "expected many opaque pixels for a zoom-to-fit cube, got {}",
            opaque.len()
        );

        // Pure-black outline pixels (RGB=0, A=255) next to model-coloured fill.
        let mut black_outline = 0usize;
        for &(x, y) in &opaque {
            let o = (y * 128 + x) * 4;
            let is_black = img.pixels[o] == 0 && img.pixels[o + 1] == 0 && img.pixels[o + 2] == 0;
            if !is_black {
                continue;
            }
            let mut near_fill = false;
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= 128 || ny >= 128 {
                        continue;
                    }
                    let no = ((ny as usize) * 128 + nx as usize) * 4;
                    if img.pixels[no + 3] > 0
                        && (img.pixels[no] > 20 || img.pixels[no + 1] > 20 || img.pixels[no + 2] > 20)
                    {
                        near_fill = true;
                    }
                }
            }
            if near_fill {
                black_outline += 1;
            }
        }
        assert!(
            black_outline > 30,
            "expected a black outline ring around the model, got {black_outline} outline pixels"
        );

        let png = img.encode_png().expect("png encode");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "PNG magic");
        assert!(png.len() > 100);
    }

    #[test]
    fn home_orientation_is_default_isometric() {
        let meshes = [ColoredMesh {
            mesh: cube_mesh(),
            color: [200, 100, 50],
        }];
        let bounds = meshes[0].mesh.bounds().unwrap();
        let img = rasterize_meshes(&meshes, bounds.0, bounds.1, 64);
        // Under isometric Home, more than one face is visible → colour variance.
        let mut colors = std::collections::HashSet::new();
        for y in 0..64 {
            for x in 0..64 {
                let o = (y * 64 + x) * 4;
                if img.pixels[o + 3] == 0 {
                    continue;
                }
                if img.pixels[o] == 0 && img.pixels[o + 1] == 0 && img.pixels[o + 2] == 0 {
                    continue;
                }
                colors.insert((
                    img.pixels[o] / 16,
                    img.pixels[o + 1] / 16,
                    img.pixels[o + 2] / 16,
                ));
            }
        }
        assert!(
            colors.len() >= 2,
            "isometric Home should show multiple faces with different shades, got {colors:?}"
        );
    }

    #[test]
    fn document_preview_png_from_imported_mesh() {
        let doc = cube_document();
        let png = document_preview_png(&doc).expect("imported cube must preview");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(png.len() > 200);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_embeds_preview_png_meta() {
        let doc = cube_document();
        assert!(
            body_solid_mesh(&doc, doc.bodies.keys().next().unwrap()).is_some(),
            "test fixture body must mesh"
        );

        let path = std::env::temp_dir().join("bearcad_preview_embed_test.bearcad");
        let path_s = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        crate::storage::save(&path_s, &doc).expect("save");
        attach_preview_after_save(&path_s, &doc);

        let png = load_embedded_preview_png(&path_s).expect("save should embed preview_png blob");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(png.len() > 200);

        let loaded = crate::storage::open(&path_s).expect("open");
        assert_eq!(loaded.bodies.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    /// #1790: the retired `preview_stl` blob is purged on save, so files saved by older
    /// builds slim down and QuickLook shows only the PNG screenshot.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_purges_legacy_preview_stl_blob() {
        let doc = cube_document();
        let path = std::env::temp_dir().join("bearcad_preview_stl_purge_test.bearcad");
        let path_s = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        crate::storage::save(&path_s, &doc).expect("save");
        // Simulate a file saved by a build that still embedded the STL snapshot.
        let _ = crate::storage::upsert_preview_blob(&path_s, PREVIEW_STL_BLOB_KIND, &[0u8; 84]);
        assert!(load_embedded_preview_stl(&path_s).is_some(), "fixture");

        attach_preview_after_save(&path_s, &doc);

        assert!(
            load_embedded_preview_stl(&path_s).is_none(),
            "legacy preview_stl blob must be deleted on save"
        );
        assert!(load_embedded_preview_png(&path_s).is_some(), "PNG survives");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn export_preview_writes_png_file() {
        let doc = cube_document();
        let path = std::env::temp_dir().join("bearcad_export_preview_test.png");
        let path_s = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);
        export_preview_png(&doc, &path_s).expect("export_preview");
        let read = std::fs::read(&path).expect("read png");
        assert!(read.starts_with(&[0x89, b'P', b'N', b'G']));
        let _ = std::fs::remove_file(&path);
    }

    /// #1304 / #1339: OS preview publish must never go through ImageIO (SIGBUS).
    /// Bad PNG yields `Err`; good PNG writes a resource fork + FinderInfo, never setIcon.
    #[cfg(all(not(target_arch = "wasm32"), any(target_os = "macos", target_os = "linux")))]
    #[test]
    fn apply_os_preview_rejects_garbage_and_accepts_real_png() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_os_preview_icon_test_{}.bearcad",
            std::process::id()
        ));
        let path_s = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);
        // Real file so the icon / thumbnail cache has a target.
        std::fs::write(&path, b"not-a-sqlite-but-exists").expect("touch target");

        let garbage = apply_os_preview(&path_s, b"not a png at all");
        assert!(
            garbage.is_err(),
            "invalid PNG must soft-fail, got {garbage:?}"
        );

        let png = tiny_preview_png();
        apply_os_preview(&path_s, &png).expect("valid preview PNG must publish without crashing");

        #[cfg(target_os = "macos")]
        {
            let fork = std::fs::read(macos_resource_fork_path(&path_s))
                .expect("custom icon lives in the resource fork, not NSWorkspace.setIcon");
            assert!(
                fork.windows(8).any(|w| w == b"\x89PNG\r\n\x1a\n"),
                "resource fork must carry our PNG-in-ICNS payload"
            );
            let info = macos_get_xattr(&path_s, "com.apple.FinderInfo").expect("FinderInfo");
            assert!(info.len() >= 10);
            let flags = u16::from_be_bytes([info[8], info[9]]);
            assert_eq!(flags & K_HAS_CUSTOM_ICON, K_HAS_CUSTOM_ICON);
            // Data fork must be untouched — save already succeeded before icon attach.
            assert_eq!(
                std::fs::read(&path).expect("data fork"),
                b"not-a-sqlite-but-exists"
            );
        }

        let _ = std::fs::remove_file(&path);
    }

    /// #1339: a missing target is a soft error, not a crash — save already wrote the file.
    #[cfg(target_os = "macos")]
    #[test]
    fn apply_os_preview_missing_path_is_err_not_crash() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_missing_icon_{}.bearcad",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let err = apply_os_preview(&path.to_string_lossy(), &tiny_preview_png()).unwrap_err();
        assert!(!err.is_empty(), "{err}");
    }

    fn tiny_preview_png() -> Vec<u8> {
        PreviewRgba {
            width: 4,
            height: 4,
            pixels: [200u8, 40, 40, 255].repeat(16),
        }
        .encode_png()
        .expect("tiny png")
    }

    /// #1339: ICNS is packed in Rust so AppKit/ImageIO never writes PNG for the Finder icon.
    #[test]
    fn icns_from_png_is_well_formed() {
        let png = tiny_preview_png();
        let icns = encode_icns_from_png(&png).expect("icns from png");
        assert_eq!(&icns[0..4], b"icns");
        let declared = u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize;
        assert_eq!(declared, icns.len(), "icns length field must match bytes");
        assert!(
            icns.windows(8).any(|w| w == b"\x89PNG\r\n\x1a\n"),
            "modern ICNS icon types carry a PNG payload"
        );
        assert!(
            icns.windows(4).any(|w| w == b"ic14" || w == b"ic09" || w == b"ic08" || w == b"ic07"),
            "expected a 128/256/512/1024 PNG icon type, got {} bytes",
            icns.len()
        );
    }

    #[test]
    fn icns_from_png_rejects_garbage() {
        let err = encode_icns_from_png(b"not a png at all").unwrap_err();
        assert!(
            err.to_ascii_lowercase().contains("png") || err.to_ascii_lowercase().contains("decode"),
            "{err}"
        );
    }

    #[test]
    fn custom_icon_resource_fork_holds_icns() {
        let icns = b"icns\x00\x00\x00\x10........";
        let fork = macos_custom_icon_resource_fork(icns);
        assert!(fork.len() > 256, "resource header is 256 bytes plus data");
        let data_off = u32::from_be_bytes(fork[0..4].try_into().unwrap());
        let map_off = u32::from_be_bytes(fork[4..8].try_into().unwrap());
        let data_len = u32::from_be_bytes(fork[8..12].try_into().unwrap());
        assert_eq!(data_off, 256);
        assert_eq!(data_len as usize, 4 + icns.len());
        assert_eq!(map_off, 256 + data_len);
        let res_len = u32::from_be_bytes(fork[256..260].try_into().unwrap()) as usize;
        assert_eq!(res_len, icns.len());
        assert_eq!(&fork[260..260 + icns.len()], icns);
        assert!(
            fork[map_off as usize..].windows(4).any(|w| w == b"icns"),
            "resource map type list must include 'icns'"
        );
        // Resource ID -16455 is stored big-endian as 0xBFC9.
        assert!(
            fork[map_off as usize..]
                .windows(2)
                .any(|w| w == CUSTOM_ICON_RESOURCE_ID.to_be_bytes()),
            "custom icon resource id -16455"
        );
    }

    #[test]
    fn finder_info_sets_has_custom_icon() {
        let info = macos_finder_info_with_custom_icon(None);
        assert_eq!(info.len(), 32);
        let flags = u16::from_be_bytes([info[8], info[9]]);
        assert_eq!(flags & K_HAS_CUSTOM_ICON, K_HAS_CUSTOM_ICON);

        let mut existing = [0u8; 32];
        existing[0..4].copy_from_slice(b"TEST");
        existing[8..10].copy_from_slice(&0x0100u16.to_be_bytes());
        let merged = macos_finder_info_with_custom_icon(Some(&existing));
        assert_eq!(&merged[0..4], b"TEST", "preserve existing type code");
        let flags = u16::from_be_bytes([merged[8], merged[9]]);
        assert_eq!(flags & K_HAS_CUSTOM_ICON, K_HAS_CUSTOM_ICON);
        assert_eq!(flags & 0x0100, 0x0100, "preserve existing Finder flags");
    }
}
