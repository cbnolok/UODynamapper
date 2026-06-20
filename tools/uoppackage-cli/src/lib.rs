/// Parses a hexadecimal string into a u64.
pub fn parse_hex_u64(s: &str) -> Result<u64, std::num::ParseIntError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16)
}
