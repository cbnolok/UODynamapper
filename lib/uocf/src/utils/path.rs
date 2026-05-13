pub fn extract_texture_id_from_path(path: &str) -> Option<u32> {
    let file_name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file_name.split('.').next().unwrap_or(file_name);

    stem.split('_')
        .next()
        .and_then(extract_first_digit_run)
        .or_else(|| extract_first_digit_run(stem))
}

pub fn normalize_dictionary_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

pub fn extract_first_digit_run(segment: &str) -> Option<u32> {
    let start = segment.find(|c: char| c.is_ascii_digit())?;
    let digits = segment[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)?.parse().ok()
}


