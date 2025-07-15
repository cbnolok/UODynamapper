use std::num::IntErrorKind;

pub fn f32_as_u32(from: f32) -> Result<u32, IntErrorKind> {
    if from < 0.0 { Err(IntErrorKind::NegOverflow) } else { Ok(from as u32) }
}
pub fn f32_as_u32_expect(from: f32) -> u32 {
    f32_as_u32(from).expect("Attempting to convert a negative float to an unsigned integer?")
}
