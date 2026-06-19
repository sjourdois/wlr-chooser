//! Resolve an application icon from its app-id and rasterize it to RGBA8
//! (unmultiplied alpha), for display next to window names. PNG/JPEG via `image`,
//! SVG via `resvg`.

use std::path::{Path, PathBuf};

fn app_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/usr/share/applications")];
    if let Ok(home) = std::env::var("HOME") {
        v.push(PathBuf::from(home).join(".local/share/applications"));
    }
    v
}

fn icon_dirs() -> Vec<PathBuf> {
    let sizes = [
        "256x256", "128x128", "64x64", "48x48", "32x32", "512x512", "scalable",
    ];
    let mut v: Vec<PathBuf> = sizes
        .iter()
        .map(|s| PathBuf::from(format!("/usr/share/icons/hicolor/{s}/apps")))
        .collect();
    if let Ok(home) = std::env::var("HOME") {
        for s in sizes {
            v.push(PathBuf::from(format!(
                "{home}/.local/share/icons/hicolor/{s}/apps"
            )));
        }
    }
    v.push(PathBuf::from("/usr/share/pixmaps"));
    v
}

/// Find the icon name from a matching `.desktop` file (handles app-id ≠ icon name,
/// e.g. `footclient` → `foot`), falling back to the app-id itself.
fn icon_name(app_id: &str) -> String {
    let needle = app_id.to_lowercase();
    for base in app_dirs() {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for e in entries.flatten() {
            let fname = e.file_name().to_string_lossy().to_lowercase();
            if fname.ends_with(".desktop") && fname.contains(&needle) {
                if let Ok(content) = std::fs::read_to_string(e.path()) {
                    for line in content.lines() {
                        if let Some(v) = line.strip_prefix("Icon=") {
                            return v.trim().to_string();
                        }
                    }
                }
            }
        }
    }
    app_id.to_string()
}

/// Resolve an app-id to an icon file on disk.
pub fn resolve(app_id: &str) -> Option<PathBuf> {
    let name = icon_name(app_id);
    if Path::new(&name).is_file() {
        return Some(PathBuf::from(name));
    }
    for dir in icon_dirs() {
        for ext in ["png", "svg", "svgz"] {
            let p = dir.join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Rasterize an icon to at most `size`×`size`, RGBA8 unmultiplied.
pub fn load(path: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("svg") | Some("svgz") => load_svg(path, size),
        _ => load_raster(path, size),
    }
}

fn load_raster(path: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::open(path).ok()?.to_rgba8();
    let small = image::imageops::thumbnail(&img, size, size);
    Some((small.width(), small.height(), small.into_raw()))
}

fn load_svg(path: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    use resvg::{tiny_skia, usvg};
    let data = std::fs::read(path).ok()?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
    let ts = tree.size();
    let scale = (size as f32 / ts.width()).min(size as f32 / ts.height());
    let w = ((ts.width() * scale).round() as u32).max(1);
    let h = ((ts.height() * scale).round() as u32).max(1);
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia is premultiplied; un-premultiply for egui's unmultiplied textures.
    let mut rgba = pixmap.take();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        if a > 0 {
            let af = a as f32;
            for c in &mut px[..3] {
                *c = (*c as f32 * 255.0 / af).round().min(255.0) as u8;
            }
        }
    }
    Some((w, h, rgba))
}
