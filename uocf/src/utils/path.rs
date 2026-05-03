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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_dictionary_path() {
        assert_eq!(
            normalize_dictionary_path("Data/WorldArt/Ankh.tga"),
            r"data\worldart\ankh.tga"
        );
        assert_eq!(
            normalize_dictionary_path(r"DATA\WORLDART\ANKH.TGA"),
            r"data\worldart\ankh.tga"
        );
    }

    #[test]
    fn test_extract_first_digit_run() {
        assert_eq!(extract_first_digit_run("abc123def"), Some(123));
        assert_eq!(extract_first_digit_run("00001234"), Some(1234));
        assert_eq!(extract_first_digit_run("no digits"), None);
    }

    #[test]
    fn test_extract_texture_id_from_path() {
        assert_eq!(
            extract_texture_id_from_path(r"Data\WorldArt\00000002_ankh.tga"),
            Some(2)
        );
        assert_eq!(
            extract_texture_id_from_path(r"Data\TileArtLegacy\3.tga"),
            Some(3)
        );
        assert_eq!(
            extract_texture_id_from_path("Data/TileArtEnhanced/02000540_Sand_Cliff_EW_A.tga"),
            Some(2000540)
        );
    }
}
