pub mod block;
pub mod file;
pub mod package;
pub mod hash;
pub mod hash_bruteforce;
pub mod tests;
pub mod compression; // Add this line

pub use compression::mythic_decompress; // Re-export mythic_decompress
pub use compression::zlib_bwt_codec; // Re-export zlib_bwt_codec
