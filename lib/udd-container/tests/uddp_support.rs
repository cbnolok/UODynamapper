use udd_container::*;
use udd_container::Codec;

#[test]
fn test_meta32_packing() {
    let data_type = 42; // arbitrary
    let codec = Codec::ZstdNoDict;
    let delta = 0xAA000000; // Only high byte is stored

    let packed = pack_meta32(data_type, codec, delta, false);

    assert_eq!(unpack_type(packed), data_type);
    assert_eq!(unpack_codec(packed), codec);
    assert!(!unpack_independent_frames(packed));
    
    // Size delta check (high 8 bits only in meta32)
    assert_eq!(unpack_delta_hi8(packed), 0xAA);

    // The independent frames bit leaves the other fields alone.
    let framed = pack_meta32(data_type, codec, delta, true);
    assert!(unpack_independent_frames(framed));
    assert_eq!(unpack_type(framed), data_type);
    assert_eq!(unpack_codec(framed), codec);
    assert_eq!(unpack_delta_hi8(framed), 0xAA);
}
