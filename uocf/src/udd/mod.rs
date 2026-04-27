pub mod codec;
pub mod uddf;
pub mod uddp;

#[cfg(test)]
mod tests;

pub use uddp::{
    AddFileRequest,
    BuildProgress,
    BuildProgressPhase,
    Codec,
    CompressionFlag,
    DataType,
    LookupMode,
    PatchError,
    ResolvedFile,
    UddpBuilder,
    UddpReader,
    UddpiApplier,
    UddpiBuilder,
    MAX_PACKAGE_SIZE,
    MAX_RAW_FILE_SIZE,
    MAX_TYPES,
    PMAN_MAGIC,
    canonical_package_hash64,
    xxh64_virtual_path,
    UDDP_MAGIC,
    UDPI_MAGIC,
};
pub use codec::{
    CodecBits,
    UDDP_DEFAULT_ALIGNMENT,
    UddpCompression,
    UddpContentId,
    align_up,
    decode_payload,
    encode_payload,
};
pub use uddf::UddfFile;
