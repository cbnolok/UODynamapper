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



/// The file `file_name` in `dir`, matched ignoring ASCII case: Windows client folders mix cases (UO Gold ships
/// `Anim.idx` beside `anim.mul`), which a case-sensitive file system tells apart. An exact match wins.
pub fn find_file_ignoring_case(dir: &std::path::Path, file_name: &str) -> Option<std::path::PathBuf> {
    let exact = dir.join(file_name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_str().is_some_and(|name| name.eq_ignore_ascii_case(file_name)))
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
}
