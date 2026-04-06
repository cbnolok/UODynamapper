#[cfg(test)]
mod tests {
    use crate::uop::file::{CompressionFlag, UopFile};
    use std::io::Cursor;

    #[test]
    fn test_zstd_compression_decompression() {
        let content = b"This is some test content that should be compressed with Zstd and then decompressed back to its original state. Zstd is a great modern compression algorithm.";
        let filename_hash = 123456789;

        // Create a UopFile with Zstd compression
        let uop_file = UopFile::new().create_file(
            &mut Cursor::new(content),
            filename_hash,
            CompressionFlag::Zstd
        ).expect("Failed to create Zstd compressed UopFile");

        assert_eq!(uop_file.compression(), CompressionFlag::Zstd);
        assert_eq!(uop_file.decompressed_size(), content.len() as u32);
        
        // Verify it actually compressed something
        assert!(uop_file.compressed_size() > 0);

        // Decompress and verify
        let unpacked = uop_file.unpack().expect("Failed to unpack Zstd data");
        assert_eq!(unpacked, content);
    }
}
