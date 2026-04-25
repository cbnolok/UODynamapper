#[cfg(test)]
mod tests {
    use crate::uop::file::{CompressionFlag, UopFile};
    use std::io::Cursor;

    #[test]
    fn zlib_roundtrip_preserves_payload() {
        let content = b"legacy zlib payload";
        let file = UopFile::new()
            .create_file(&mut Cursor::new(content), 123, CompressionFlag::Zlib)
            .expect("create zlib-compressed file");

        assert_eq!(file.compression(), CompressionFlag::Zlib);
        assert_eq!(file.unpack().expect("unpack zlib"), content);
    }

    #[test]
    fn mythic_roundtrip_preserves_payload() {
        let content = b"legacy mythic payload";
        let file = UopFile::new()
            .create_file(&mut Cursor::new(content), 124, CompressionFlag::Mythic)
            .expect("create mythic-compressed file");

        assert_eq!(file.compression(), CompressionFlag::Mythic);
        assert_eq!(file.unpack().expect("unpack mythic"), content);
    }

    #[test]
    fn zlib_bwt_roundtrip_preserves_payload() {
        let content = b"legacy zlib+bwt payload";
        let file = UopFile::new()
            .create_file(&mut Cursor::new(content), 125, CompressionFlag::ZlibBwt)
            .expect("create zlib+bwt-compressed file");

        assert_eq!(file.compression(), CompressionFlag::ZlibBwt);
        assert_eq!(file.unpack().expect("unpack zlib+bwt"), content);
    }

    #[test]
    fn none_roundtrip_preserves_payload() {
        let content = b"legacy raw payload";
        let file = UopFile::new()
            .create_file(&mut Cursor::new(content), 456, CompressionFlag::None)
            .expect("create raw file");

        assert_eq!(file.compression(), CompressionFlag::None);
        assert_eq!(file.unpack().expect("unpack raw"), content);
    }

    #[test]
    fn rejects_non_legacy_uop_compression_flags() {
        assert!(CompressionFlag::from_raw_i16(128).is_err());
        assert!(CompressionFlag::from_raw_i16(32765).is_err());
    }
}
