use std::path::Path;

pub fn export_rgba_png(
    default_name: String,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<Option<String>, String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG Image", &["png"])
        .set_file_name(default_name)
        .save_file()
    else {
        return Ok(None);
    };

    save_rgba_png(&path, width, height, rgba)
        .map_err(|e| e.to_string())
        .map(|()| Some(path.display().to_string()))
}

fn save_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> image::ImageResult<()> {
    image::save_buffer_with_format(
        path,
        rgba,
        width,
        height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
}

pub fn sanitize_file_stem(value: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_ascii_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_file_stem_keeps_portable_names() {
        assert_eq!(sanitize_file_stem("body 400/action:2", "frame"), "body_400action2");
        assert_eq!(sanitize_file_stem("///", "frame"), "frame");
    }
}
