#[cfg(test)]
mod tests {
    use crate::uop::codec::{CodecBits, UddpCompression, UDDP_DEFAULT_ALIGNMENT};
    use crate::uop::file::CompressionFlag;
    use crate::uop::{UddfFile, UddpPackage};
    use std::io::Cursor;

    #[test]
    fn codec_bits_roundtrip_for_zstd() {
        let codec = CodecBits::new(42, UddpCompression::Zstd).expect("codec bits");
        assert_eq!(codec.content_id(), 42);
        assert_eq!(codec.compression().expect("compression"), UddpCompression::Zstd);
        assert_eq!(CompressionFlag::from_raw_i16(128).expect("raw 128"), CompressionFlag::Zstd);
        assert_eq!(CompressionFlag::from_raw_i16(32765).expect("raw 32765"), CompressionFlag::Zstd);
    }

    #[test]
    fn uddp_roundtrip_preserves_entries_and_alignment() {
        let mut package = UddpPackage::new();
        package
            .add_dictionary(7, b"dictionary payload")
            .expect("add dictionary");
        package
            .add_entry_from_memory(
                b"tiny zstd payload for art cluster",
                "build/art/00000001.bin",
                7,
                UddpCompression::Zstd,
            )
            .expect("add zstd entry");
        package
            .add_entry_from_memory(
                b"raw map chunk",
                "build/map/00000000.bin",
                9,
                UddpCompression::None,
            )
            .expect("add raw entry");

        let mut writer = Cursor::new(Vec::new());
        package.save_to_writer(&mut writer).expect("save uddp");

        let bytes = writer.into_inner();
        let mut reader = Cursor::new(bytes);
        let loaded = UddpPackage::load_from_reader(&mut reader).expect("load uddp");

        let zstd_entry = loaded
            .get_entry_by_path("build/art/00000001.bin")
            .expect("zstd entry");
        assert_eq!(
            zstd_entry.unpack().expect("unpack zstd"),
            b"tiny zstd payload for art cluster"
        );
        assert_eq!(zstd_entry.data_offset() % UDDP_DEFAULT_ALIGNMENT, 0);

        let raw_entry = loaded
            .get_entry_by_path("build/map/00000000.bin")
            .expect("raw entry");
        assert_eq!(raw_entry.unpack().expect("unpack raw"), b"raw map chunk");
        assert_eq!(raw_entry.data_offset() % UDDP_DEFAULT_ALIGNMENT, 0);

        let dictionary = loaded.get_dictionary(7).expect("dictionary");
        assert_eq!(dictionary.data(), b"dictionary payload");
    }

    #[test]
    fn uddf_roundtrip_preserves_flags_and_payload() {
        let payload = b"single wrapped payload";
        let uddf = UddfFile::wrap_bytes(
            payload,
            0x5544_5431,
            12,
            UddpCompression::Zstd,
            0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00u128,
        )
        .expect("wrap uddf");

        let mut writer = Cursor::new(Vec::new());
        uddf.save_to_writer(&mut writer).expect("save uddf");

        let bytes = writer.into_inner();
        let mut reader = Cursor::new(bytes);
        let loaded = UddfFile::load_from_reader(&mut reader).expect("load uddf");

        assert_eq!(loaded.unpack().expect("unpack uddf"), payload);
        assert_eq!(loaded.flags(), 0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00u128);
        assert_eq!(loaded.payload_offset() % UDDP_DEFAULT_ALIGNMENT, 0);
    }
}
