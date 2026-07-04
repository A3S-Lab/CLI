//! Image preview helpers: half-block terminal rendering + clipboard capture.

use a3s_tui::style::{Color, Style};

/// True if `path` looks like a previewable raster image.
pub(crate) fn is_image_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

/// Render an image as Unicode half-block lines — each line is one text row with
/// fg = the upper pixel and bg = the lower pixel, so it drops cleanly into the
/// line-based renderer and works in any truecolor terminal.
pub(crate) fn render_image_blocks(
    img: &image::DynamicImage,
    max_cols: usize,
    max_rows: usize,
) -> Vec<String> {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || max_cols == 0 || max_rows == 0 {
        return Vec::new();
    }
    // Two vertical pixels per text row.
    let scale = (max_cols as f32 / w as f32)
        .min((max_rows * 2) as f32 / h as f32)
        .clamp(0.001, 1.0);
    let nw = ((w as f32 * scale) as u32).max(1);
    let nh = ((h as f32 * scale) as u32).max(2);
    let rgba = img
        .resize_exact(nw, nh, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let mut lines = Vec::new();
    let mut y = 0;
    while y < nh {
        let mut line = String::new();
        for x in 0..nw {
            let t = *rgba.get_pixel(x, y);
            let b = if y + 1 < nh {
                *rgba.get_pixel(x, y + 1)
            } else {
                t
            };
            line.push_str(
                &Style::new()
                    .fg(Color::Rgb(t[0], t[1], t[2]))
                    .bg(Color::Rgb(b[0], b[1], b[2]))
                    .render("▀"),
            );
        }
        lines.push(line);
        y += 2;
    }
    lines
}

/// Half-block preview of an image file, or `None` if it can't be decoded.
pub(crate) fn render_image_file(
    path: &std::path::Path,
    max_cols: usize,
    max_rows: usize,
) -> Option<Vec<String>> {
    let img = image::open(path).ok()?;
    Some(render_image_blocks(&img, max_cols, max_rows))
}

/// Write the macOS clipboard image to `dest` as PNG. Returns false (and cleans
/// up) if the clipboard holds no image. ponytail: macOS-only via osascript.
pub(crate) fn clipboard_image_to(dest: &std::path::Path) -> bool {
    let path = dest.to_string_lossy();
    let ok = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!("set f to open for access POSIX file \"{path}\" with write permission"),
            "-e",
            "write (the clipboard as «class PNGf») to f",
            "-e",
            "close access f",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let nonempty = std::fs::metadata(dest)
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if ok && nonempty {
        true
    } else {
        let _ = std::fs::remove_file(dest);
        false
    }
}
