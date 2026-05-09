use uocf::udd::uddp::*;
use uocf::udd::Codec;

#[test]
fn test_meta32_packing() {
    let data_type = 42; // arbitrary
    let codec = Codec::ZstdNoDict;
    let delta = 0xAA000000; // Only high byte is stored

    let packed = pack_meta32(data_type, codec, delta);

    assert_eq!(unpack_type(packed), data_type);
    assert_eq!(unpack_codec(packed), codec);
    
    // Size delta check (high 8 bits only in meta32)
    assert_eq!(unpack_delta_hi8(packed), 0xAA);
}
