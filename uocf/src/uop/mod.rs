pub mod block;
pub mod codec;
pub mod uddf;
pub mod uddp;
pub mod file;
pub mod package;
pub mod hash;
pub mod hash_bruteforce;
pub mod tests;
pub mod compression; // Add this line

pub use compression::mythic_decompress; // Re-export mythic_decompress
pub use compression::zlib_bwt_codec; // Re-export zlib_bwt_codec
pub use codec::{CodecBits, UddpCompression, UddpContentId, UDDP_DEFAULT_ALIGNMENT};
pub use uddf::UddfFile;
pub use uddp::{UddpDictionary, UddpEntry, UddpPackage};
