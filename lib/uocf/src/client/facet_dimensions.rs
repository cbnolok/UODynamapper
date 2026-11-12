//! The facet size tables of the client executables and the hard-coded values that go with them: locate and report
//! them in any 32-bit build. Patching a copy: [`super::patch`].
//!
//! Traced in `scripts/dev-helpers/ec-research/ghidra-facets/results/uosa_facets.md` ("Other dimension sites",
//! "Constraints", "Builds and the cross-build patch recipe"), `uokr_facets.md` and
//! `scripts/dev-helpers/ec-research/ghidra-classic-facets/results/classic_facets.md`. Everything is located by
//! structure; the SHA-256 of a traced build ([`KNOWN_BUILDS`]) is only a label. The family ([`ClientFamily`]) is told
//! by the table found:
//!
//! - Enhanced Client and Kingdom Reborn (`UOFacet::g_FacetDimensions`): a run of 12-byte records
//!   `{char* name, u32 width, u32 height}` in a data section. The name pointer resolves to a short NUL-terminated
//!   ASCII name in a readable section, and the sizes are legal facet sizes. Exactly one run of
//!   [`MIN_FACET_RECORDS`]..=[`MAX_FACET_RECORDS`] records is required (the shipped builds have 7: Felucca, Trammel,
//!   Ilshenar, Malas, Tokuno, Gargoyle, Test). The client sizes its sector grid and chunk bounds from it. The
//!   Enhanced Client carries the RTTI name of its FacetDefinition class ([`ENHANCED_CLIENT_MARKER`]); Kingdom Reborn
//!   does not.
//! - `UOFacet::IsSameFeluccaTrammelHalf` (UOSA 0x005df670) returns false, for facets 0 and 1, when two x lie on
//!   different sides of 5120; `BuildChunkFromSector` then does not build the chunk. The 5120 is three imm32 in one
//!   `.text` sequence, found by decoding its instructions ([`SPLIT_SIGNATURE`]); 0x8000, above any legal width,
//!   disables the split. Kingdom Reborn has no split.
//! - Classic clients (`client.exe`, UO Gold to the current client): 48-byte records with wrap rectangle, file ids
//!   and a flag, in one copy or two ([`super::classic_facets`]), plus the startup copy of facet 0 and the x = 5120
//!   wrap seam, which the patcher treats as the Classic split.
//! - The PE checksum is recomputed (the client does not verify it: `UOSA_eclo.exe` ships a wrong one and runs).
//!   A signed (Authenticode) build is refused, since any patch breaks its signature; no shipped build is signed.
//!
//! The executable alone is not enough: the facet data must have the new size too ([`super::facet_data`]).

crate::eyre_imports!();

use crate::client::classic_facets::{
    classic_copies_agree, is_classic_stock, locate_classic_facet_tables, locate_classic_file_names,
    locate_classic_seam, locate_classic_startup_facet, ClassicLayout, ClassicRecord, MAX_CLASSIC_TABLE_COPIES,
    UNLOCATED_FILE_ID_LIMIT,
};
use crate::client::classic_difs::{inspect_classic_difs, ClassicDifUsage};
use crate::client::encryption::{locate_encryption_sites, EncryptionSites, EncryptionState};
use crate::client::login::{locate_login_sites, LoginSites, LoginValues};
use crate::client::version::{read_exe_version, ExeVersion};
use crate::client::pe::{pe_checksum, read_u32, PeImage, PeSection, MACHINE_I386};
use crate::client::sha256::sha256_hex;
use crate::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS, SECTOR_SIDE};

/// Records of the shipped tables (Enhanced Client and Kingdom Reborn); every reader there rejects a facet >= 7.
pub const STOCK_FACET_COUNT: usize = CLIENT_FACET_DIMENSIONS.len();
/// The fewest records of a table: Felucca, Trammel, Ilshenar and Malas exist in every client since Age of Shadows.
pub const MIN_FACET_RECORDS: usize = 4;
/// The most records of a table: the facet nibble of the client's chunk key holds 16 facets.
pub const MAX_FACET_RECORDS: usize = 16;
/// The name pointer of a record (32-bit images only).
pub(crate) const NAME_POINTER_SIZE: usize = size_of::<u32>();
/// Bytes of a table record: `char* name, u32 width, u32 height`.
pub const FACET_RECORD_SIZE: usize = NAME_POINTER_SIZE + 2 * size_of::<u32>();
/// The width and height follow the name pointer, in both record layouts.
pub const FACET_RECORD_WIDTH_OFFSET: usize = NAME_POINTER_SIZE;
pub const FACET_RECORD_HEIGHT_OFFSET: usize = FACET_RECORD_WIDTH_OFFSET + size_of::<u32>();
/// The tables are pointer aligned.
pub(crate) const TABLE_SCAN_STEP: usize = NAME_POINTER_SIZE;
/// Facet names are short ASCII words ("Felucca", "Ter Mur", "Tokuno Islands"): a letter, then letters, digits or
/// [`FACET_NAME_PUNCTUATION`], NUL terminated.
const MIN_FACET_NAME_LEN: usize = 2;
const MAX_FACET_NAME_LEN: usize = 31;
const FACET_NAME_PUNCTUATION: &[u8] = b" _'-";
pub(crate) const CODE_SECTION: &str = ".text";
/// The RTTI type name of the class that reads the FacetDefinition of `facets.uop`: only the Enhanced Client has it.
pub const ENHANCED_CLIENT_MARKER: &[u8] = b".?AVUOFacetDefinition@@";

/// The largest width or height: the land accessors reject x or y > 0x7fe0, and the side is a multiple of 64.
pub const MAX_FACET_SIDE: u32 = 32704;
/// Widths and heights must be multiples of this (Tokuno, 1448, is not a multiple of 64; DXT1 needs 4; the Classic
/// map block is 8 x 8).
pub const FACET_SIDE_ALIGNMENT: u32 = 8;
/// Sector ids are u16.
pub const MAX_SECTOR_COUNT: u32 = u16::MAX as u32 + 1;

/// The stock split x of facets 0 and 1 (Enhanced Client), and the stock wrap seam (Classic).
pub const STOCK_SPLIT_X: u32 = 5120;
/// A split x above any legal width: no chunk is split.
pub const DISABLED_SPLIT_X: u32 = 0x8000;
/// The facets the split applies to, and the Classic facets with a flat strip past the wrap.
pub const SPLIT_FACETS: [u8; 2] = [0, 1];
/// The three imm32 of the split.
pub const SPLIT_IMMEDIATE_COUNT: usize = 3;
/// The split sequence the matcher decodes (`r`, `m`: any register or memory operand; `X`: the same operand in
/// both places; `jcc`: a short or near jump; the three imm32 must be equal):
///
/// ```text
/// CMP r/m,1      ; JG|JA   T       facet > 1: same half
/// CMP r/m,imm32  ; JGE|JAE B       x1 >= split
/// CMP X,imm32    ; JL|JB   T       x1 < split, x2 < split: same half
/// F: XOR AL,AL   ...               different halves
/// B: CMP X,imm32 ; JL|JB   F       x1 >= split, x2 < split: different halves
/// T: POP r ...   ; MOV AL,1        same half
/// ```
///
/// UOSA: `83 FF 01 7F 24 81 7C 24 08 [imm32] 7D 12 81 FB [imm32] 7C 12 32 C0 .. 81 FB [imm32] 7C EE 5E B0 01`.
pub const SPLIT_SIGNATURE: &str = "CMP r/m,1; JG T; CMP r/m,S; JGE B; CMP X,S; JL T; F: XOR AL,AL; .. \
                                   B: CMP X,S; JL F; .. T: POP*; MOV AL,1";

/// x86 (32-bit) encodings the matchers decode.
const OPCODE_CMP_RM32_IMM32: u8 = 0x81;
const OPCODE_CMP_RM32_IMM8: u8 = 0x83;
const OPCODE_CMP_EAX_IMM32: u8 = 0x3d;
/// The ModRM reg field of CMP in the 0x81 / 0x83 opcode groups.
const MODRM_REG_CMP: u8 = 7;
const MODRM_MOD_SHIFT: u8 = 6;
const MODRM_REG_SHIFT: u8 = 3;
const MODRM_FIELD_MASK: u8 = 0b111;
const MODRM_REG_FIELD: u8 = MODRM_FIELD_MASK << MODRM_REG_SHIFT;
const MOD_INDIRECT: u8 = 0;
const MOD_DISP8: u8 = 1;
const MOD_DISP32: u8 = 2;
pub(crate) const MOD_REGISTER: u8 = 3;
/// rm = 100: a SIB byte follows; rm = 101 with mod 00 (or SIB base 101 with mod 00): a disp32 without base.
const RM_SIB: u8 = 0b100;
const RM_DISP32: u8 = 0b101;
const REGISTER_EAX: u8 = 0;
const OPCODE_JCC_SHORT: u8 = 0x70;
pub(crate) const OPCODE_TWO_BYTE: u8 = 0x0f;
const OPCODE_JCC_NEAR: u8 = 0x80;
const JCC_CONDITION_MASK: u8 = 0x0f;
const JCC_SHORT_SIZE: usize = 2;
const JCC_NEAR_SIZE: usize = 6;
/// Jcc conditions: below, above or equal, above, less, greater or equal, greater.
const CONDITION_B: u8 = 0x2;
const CONDITION_AE: u8 = 0x3;
const CONDITION_A: u8 = 0x7;
const CONDITION_L: u8 = 0xc;
const CONDITION_GE: u8 = 0xd;
const CONDITION_G: u8 = 0xf;
/// The facet compare: `facet > 1` is outside [`SPLIT_FACETS`].
const SPLIT_FACET_LIMIT: u8 = 1;
/// `XOR AL,AL` (both encodings) or `XOR EAX,EAX`: the false result.
const CLEAR_RESULT: [[u8; 2]; 3] = [[0x32, 0xc0], [0x30, 0xc0], [0x33, 0xc0]];
/// `MOV AL,1`: the true result, after at most [`MAX_POPS_BEFORE_RESULT`] one-byte `POP r32`.
const SET_RESULT: [u8; 2] = [0xb0, 0x01];
const POP_REGISTER_OPCODES: std::ops::RangeInclusive<u8> = 0x58..=0x5f;
const MAX_POPS_BEFORE_RESULT: usize = 4;

/// Which client an executable is, told by its structure ([`ClientExeInspection::inspect`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientFamily {
    /// `UOSA.exe` builds: the 12-byte table, the FacetDefinition class and the x = 5120 chunk split.
    Enhanced,
    /// `UOKR.exe`: the 12-byte table, without the FacetDefinition class or a split.
    KingdomReborn,
    /// `client.exe`, UO Gold to the current Classic client: the 48-byte table, in one or two copies.
    Classic,
}

impl ClientFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::Enhanced => "Enhanced Client",
            Self::KingdomReborn => "Kingdom Reborn",
            Self::Classic => "Classic",
        }
    }
}

/// A client build whose layout was traced. Only a label for reports and a reference for the tests: everything is
/// always located by structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownBuild {
    pub name: &'static str,
    pub sha256: &'static str,
    pub family: ClientFamily,
    /// File offsets of the tables (two copies in Classic 7.0 and later).
    pub table_offsets: &'static [usize],
    /// File offsets of the split imm32 (Enhanced Client) or of the wrap seam imm32 (Classic); empty in Kingdom
    /// Reborn.
    pub split_offsets: &'static [usize],
    /// File offset of the Classic startup copy of facet 0; `None` in the other families.
    pub startup_offset: Option<usize>,
    /// File offset of the PE checksum.
    pub checksum_offset: usize,
}

/// PE checksum offset of the Enhanced Client builds and of `UOKR.exe`.
const EC_CHECKSUM_OFFSET: usize = 0x158;
const KR_CHECKSUM_OFFSET: usize = 0x168;

/// The builds of the research (`uosa_facets.md`, `classic_facets.md`); the `fp` folder's `UOSA_TC.exe` and
/// `UOSA_eclo.exe` are the same files as the main folder's, and the Classic folder's `client.exe` is 7.0.103.0.
pub const KNOWN_BUILDS: [KnownBuild; 12] = [
    KnownBuild {
        name: "Enhanced Client UOSA.exe",
        sha256: "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50",
        family: ClientFamily::Enhanced,
        table_offsets: &[0x9a72a8],
        split_offsets: &[0x1df6db, 0x1df6e3, 0x1df6f5],
        startup_offset: None,
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Enhanced Client UOSA_TC.exe",
        sha256: "4e38c1cb834083c18050ca9fc85f1ebf77cf4d8bcb1a39a1f37878ea314553d0",
        family: ClientFamily::Enhanced,
        table_offsets: &[0x98f870],
        split_offsets: &[0x1cfceb, 0x1cfcf3, 0x1cfd05],
        startup_offset: None,
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Enhanced Client UOSA_eclo.exe",
        sha256: "5aaa654c517adc3aab88467f8bc869e9b048c78c0c195caf085723fecbb8fdf0",
        family: ClientFamily::Enhanced,
        table_offsets: &[0x9a72a8],
        split_offsets: &[0x1df6db, 0x1df6e3, 0x1df6f5],
        startup_offset: None,
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Enhanced Client fp UOSA.exe",
        sha256: "9692f6ae95dd55338985caabf247f3a2490ba2ef4d0cfe2d75694b56ba65e3dd",
        family: ClientFamily::Enhanced,
        table_offsets: &[0x9ad2e0],
        split_offsets: &[0x1e1a0b, 0x1e1a13, 0x1e1a25],
        startup_offset: None,
        checksum_offset: EC_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Kingdom Reborn UOKR.exe",
        sha256: "1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43",
        family: ClientFamily::KingdomReborn,
        table_offsets: &[0x7fb688],
        split_offsets: &[],
        startup_offset: None,
        checksum_offset: KR_CHECKSUM_OFFSET,
    },
    KnownBuild {
        name: "Classic UO Gold client.exe",
        sha256: CLASSIC_GOLD_SHA256,
        family: ClientFamily::Classic,
        table_offsets: &[0x16f918],
        split_offsets: &[0xa80ab, 0xef339],
        startup_offset: Some(0x1cd66c),
        checksum_offset: 0x190,
    },
    KnownBuild {
        name: "Classic Mondain's Legacy client.exe",
        sha256: "17ab35e10bf38077fa101938a4a0b85d1b4caa505245fc6bc83d3b131a0c2918",
        family: ClientFamily::Classic,
        table_offsets: &[0x17d260],
        split_offsets: &[0xaf3f7, 0xfb3b9],
        startup_offset: Some(0x1dbe68),
        checksum_offset: 0x180,
    },
    KnownBuild {
        name: "Classic 5.0.9.1 client.exe",
        sha256: "6250963022ec03855dedf89176c54dc7ad603e230b415edaa4b08d1509f568e9",
        family: ClientFamily::Classic,
        table_offsets: &[0x190610],
        split_offsets: &[0xb4617, 0x1061b9],
        startup_offset: Some(0x1f15d8),
        checksum_offset: 0x180,
    },
    KnownBuild {
        name: "Classic 6.0.14.3 client.exe",
        sha256: "9104cea0ea14bd8ee81ee5aa111f1e3f39eca50ac33f818813202fadfdd3718d",
        family: ClientFamily::Classic,
        table_offsets: &[0x1a6db0],
        split_offsets: &[0xb30bb, 0x100ea0],
        startup_offset: Some(0x2124c8),
        checksum_offset: 0x178,
    },
    KnownBuild {
        name: "Classic 7.0.20.0 client.exe",
        sha256: "27eef2c0a4679f204b923f29dd1506bb2c7e649410054aefad680264bb504c9e",
        family: ClientFamily::Classic,
        table_offsets: &[0x285088, 0x2851d8],
        split_offsets: &[0x12309b, 0x18ded9],
        startup_offset: Some(0x308cc8),
        checksum_offset: 0x188,
    },
    KnownBuild {
        name: "Classic 7.0.50.0 client.exe",
        sha256: "8b3af41390d34930ac1dc914b865edb7961c5448e0592b43a912790814987940",
        family: ClientFamily::Classic,
        table_offsets: &[0x2a2338, 0x2a2488],
        split_offsets: &[0x12329b, 0x190749],
        startup_offset: Some(0x329848),
        checksum_offset: 0x178,
    },
    KnownBuild {
        name: "Classic 7.0.103.0 client.exe",
        sha256: "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a",
        family: ClientFamily::Classic,
        table_offsets: &[0x2b7478, 0x2b75c8],
        split_offsets: &[0x12b5fb, 0x19c680],
        startup_offset: Some(0x34229c),
        checksum_offset: 0x180,
    },
];

/// The UO Gold `client.exe` of the research: its Felucca and Trammel are 6144 wide.
pub const CLASSIC_GOLD_SHA256: &str = "aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2";

/// The known build with SHA-256 `sha256`.
pub fn known_build(sha256: &str) -> Option<&'static KnownBuild> {
    KNOWN_BUILDS.iter().find(|build| build.sha256 == sha256)
}

/// One record of a located table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetTableRecord {
    /// The name the record points to.
    pub name: String,
    pub geometry: FacetGeometry,
    /// The fields of a Classic record; `None` in a 12-byte record.
    pub classic: Option<ClassicRecord>,
}

/// A run of facet records in a data section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetTable {
    pub section: String,
    /// File offset and virtual address of the first record.
    pub offset: usize,
    pub va: u32,
    /// Bytes of a record: [`FACET_RECORD_SIZE`] or [`super::classic_facets::CLASSIC_FACET_RECORD_SIZE`].
    pub record_size: usize,
    pub records: Vec<FacetTableRecord>,
}

impl FacetTable {
    /// File offset of record `facet`.
    pub fn record_offset(&self, facet: usize) -> usize {
        self.offset + self.record_size * facet
    }
}

/// One match of [`SPLIT_SIGNATURE`]: the file offsets of the facet compare and of the three imm32, and their values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitCandidate {
    pub signature_offset: usize,
    pub offsets: [usize; SPLIT_IMMEDIATE_COUNT],
    pub values: [u32; SPLIT_IMMEDIATE_COUNT],
}

/// The split of a build (Enhanced Client) or its wrap seam (Classic): the file offsets of its imm32 and their (equal)
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetSplit {
    /// File offsets of the matched sequences: the facet compare that starts [`SPLIT_SIGNATURE`], or each seam compare
    /// ([`super::classic_facets::CLASSIC_SEAM_SIGNATURE`]).
    pub signature_offsets: Vec<usize>,
    pub offsets: Vec<usize>,
    pub x: u32,
}

/// The facet sizes, wraps and split of a build, as stored in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExeValues {
    pub facets: Vec<FacetValues>,
    /// The split x of facets 0 and 1 (Enhanced Client) or the wrap seam (Classic); `None` for a build without
    /// (Kingdom Reborn).
    pub split_x: Option<u32>,
    /// The login values a login edit writes ([`LoginSites::values`]: the Enhanced Client's public hosts and ports,
    /// Kingdom Reborn's fixed address); `None` for Classic, in builds where they were not located, and in manifests
    /// written before they were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<LoginValues>,
    /// Whether the client encrypts its login and game traffic ([`EncryptionSites::values`]); `None` for Classic, in
    /// builds where the ciphers were not all located, and in manifests written before it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionState>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FacetValues {
    pub facet: u8,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// The wrap rectangle (Classic only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_height: Option<u32>,
}

/// What a client executable stores about the facet sizes and the login server.
#[derive(Debug, Clone)]
pub struct ClientExeInspection {
    pub sha256: String,
    pub file_size: usize,
    pub family: ClientFamily,
    /// The traced build of the same SHA-256, as a label.
    pub known: Option<&'static KnownBuild>,
    pub pe: PeImage,
    /// The checksum the file should store.
    pub computed_checksum: u32,
    /// The facet tables: one, or two equal-sized copies (Classic 7.0 and later; the second is read when the server
    /// sets 0xA9 flag 0x8000). Every size edit writes each.
    pub tables: Vec<FacetTable>,
    /// The Enhanced Client split, or the Classic wrap seam.
    pub split: Option<FacetSplit>,
    /// The Classic startup copy of facet 0 and file name table; `None` in the other families.
    pub classic: Option<ClassicLayout>,
    /// The login address code (Enhanced Client and Kingdom Reborn), when located ([`locate_login_sites`]).
    pub login: Option<LoginSites>,
    /// The network ciphers (Enhanced Client and Kingdom Reborn; [`locate_encryption_sites`]); `None` for Classic.
    pub encryption: Option<EncryptionSites>,
    /// The file and product versions of the version resource, when it has one (information only).
    pub version: Option<ExeVersion>,
    /// Classic: whether the code applies the mapdif / stadif patches ([`inspect_classic_difs`]); `None` in the other
    /// families.
    pub difs: Option<ClassicDifUsage>,
}

/// The single element of `items`, or `None`.
pub(crate) fn single<T>(mut items: Vec<T>) -> Option<T> {
    if items.len() == 1 { items.pop() } else { None }
}

impl ClientExeInspection {
    /// Locates the tables, the split or seam and the Classic startup values of the client executable `bytes` (PE32
    /// i386) by their structure, and tells the family. Fails without exactly one 12-byte table or one or two agreeing
    /// Classic tables (not both), with more than one split, or with split or seam values that differ. The SHA-256 only
    /// selects the [`KnownBuild`] label.
    pub fn inspect(bytes: &[u8]) -> eyre::Result<Self> {
        let pe = PeImage::parse(bytes)?;
        eyre::ensure!(pe.machine == MACHINE_I386, "COFF machine {:#x}: not an i386 image", pe.machine);
        let ec_tables = locate_facet_tables(bytes, &pe);
        let file_names = single(locate_classic_file_names(bytes, &pe));
        let id_limit = file_names.as_ref().map_or(UNLOCATED_FILE_ID_LIMIT, |names| names.names.len() as u32);
        let classic_tables = locate_classic_facet_tables(bytes, &pe, id_limit);
        let (family, tables, split, classic) = match (ec_tables.len(), classic_tables.len()) {
            (1, 0) => {
                let marked = bytes.windows(ENHANCED_CLIENT_MARKER.len()).any(|window| window == ENHANCED_CLIENT_MARKER);
                let family = if marked { ClientFamily::Enhanced } else { ClientFamily::KingdomReborn };
                (family, ec_tables, locate_split(bytes, &pe)?, None)
            }
            (0, 1..=MAX_CLASSIC_TABLE_COPIES) => {
                classic_copies_agree(&classic_tables)?;
                let startup = locate_classic_startup_facet(bytes, &pe, &classic_tables[0])?;
                let split = match &startup {
                    Some(startup) => locate_classic_seam(bytes, &pe, startup)?,
                    None => None,
                };
                (ClientFamily::Classic, classic_tables, split, Some(ClassicLayout { startup, file_names }))
            }
            (ec, classic) => eyre::bail!(
                "{ec} Enhanced Client / Kingdom Reborn facet tables (runs of {MIN_FACET_RECORDS}..={MAX_FACET_RECORDS} \
                 {FACET_RECORD_SIZE}-byte {{name pointer, width, height}} records) and {classic} Classic facet tables \
                 found: expected exactly one of the first, or one or {MAX_CLASSIC_TABLE_COPIES} agreeing copies of the \
                 second"
            ),
        };
        let sha256 = sha256_hex(bytes);
        let known = known_build(&sha256);
        let computed_checksum = pe_checksum(bytes, pe.checksum_offset);
        let login = locate_login_sites(bytes, &pe, family)?;
        let encryption = locate_encryption_sites(bytes, &pe, family)?;
        let version = read_exe_version(bytes, &pe);
        let difs = (family == ClientFamily::Classic).then(|| inspect_classic_difs(bytes, &pe));
        Ok(Self {
            sha256,
            file_size: bytes.len(),
            family,
            known,
            pe,
            computed_checksum,
            tables,
            split,
            classic,
            login,
            encryption,
            version,
            difs,
        })
    }

    /// The first table: the only one, or the copy the Classic client reads without the 0xA9 flag.
    pub fn table(&self) -> &FacetTable {
        &self.tables[0]
    }

    /// Records of the table.
    pub fn facet_count(&self) -> usize {
        self.table().records.len()
    }

    pub fn dimensions(&self) -> Vec<FacetGeometry> {
        self.table().records.iter().map(|record| record.geometry).collect()
    }

    /// True when the stored PE checksum is the computed one.
    pub fn checksum_valid(&self) -> bool {
        self.pe.stored_checksum == self.computed_checksum
    }

    pub fn values(&self) -> ExeValues {
        ExeValues {
            facets: self
                .table()
                .records
                .iter()
                .enumerate()
                .map(|(facet, record)| FacetValues {
                    facet: facet as u8,
                    name: record.name.clone(),
                    width: record.geometry.width,
                    height: record.geometry.height,
                    wrap_width: record.classic.map(|classic| classic.wrap.width),
                    wrap_height: record.classic.map(|classic| classic.wrap.height),
                })
                .collect(),
            split_x: self.split.as_ref().map(|split| split.x),
            login: match self.family {
                ClientFamily::Enhanced | ClientFamily::KingdomReborn => self.login.as_ref().map(LoginSites::values),
                ClientFamily::Classic => None,
            },
            encryption: self.encryption.as_ref().and_then(EncryptionSites::values),
        }
    }

    /// True when every record has the shipped size of its facet and the split (if any) is at 5120. Enhanced Client
    /// and Kingdom Reborn: [`CLIENT_FACET_DIMENSIONS`]; Classic: [`super::classic_facets::is_classic_stock`].
    pub fn is_stock(&self) -> bool {
        let split_stock = self.split.as_ref().is_none_or(|split| split.x == STOCK_SPLIT_X);
        let records_stock = match &self.classic {
            Some(classic) => is_classic_stock(&self.tables, classic),
            None => {
                self.facet_count() <= STOCK_FACET_COUNT
                    && self.table().records.iter().zip(CLIENT_FACET_DIMENSIONS).all(|(record, stock)| record.geometry == stock)
            }
        };
        split_stock && records_stock
    }
}

/// A legal facet width or height.
pub(crate) fn plausible_side(side: u32) -> bool {
    (1..=MAX_FACET_SIDE).contains(&side) && side % FACET_SIDE_ALIGNMENT == 0
}

/// The facet name at virtual address `va`: NUL-terminated ASCII in a readable section (see [`MIN_FACET_NAME_LEN`]).
pub(crate) fn facet_name_at(bytes: &[u8], pe: &PeImage, va: u32) -> Option<String> {
    let section = pe.section_of_va(va).filter(|section| section.is_readable())?;
    let start = section.raw_offset as usize + (va - section.virtual_address) as usize;
    let end = section.raw_range().end.min(bytes.len()).min(start + MAX_FACET_NAME_LEN + 1);
    let text = bytes.get(start..end)?;
    let len = text.iter().position(|byte| *byte == 0)?;
    let name = &text[..len];
    let valid = len >= MIN_FACET_NAME_LEN
        && name[0].is_ascii_alphabetic()
        && name.iter().all(|byte| byte.is_ascii_alphanumeric() || FACET_NAME_PUNCTUATION.contains(byte));
    valid.then(|| String::from_utf8_lossy(name).into_owned())
}

/// The 12-byte record at file offset `offset` of `section`, when it is one.
fn facet_record_at(bytes: &[u8], pe: &PeImage, section: &PeSection, offset: usize) -> Option<FacetTableRecord> {
    if offset + FACET_RECORD_SIZE > section.raw_range().end.min(bytes.len()) {
        return None;
    }
    let width = read_u32(bytes, offset + FACET_RECORD_WIDTH_OFFSET).ok()?;
    let height = read_u32(bytes, offset + FACET_RECORD_HEIGHT_OFFSET).ok()?;
    if !(plausible_side(width) && plausible_side(height)) {
        return None;
    }
    let name = facet_name_at(bytes, pe, read_u32(bytes, offset).ok()?)?;
    Some(FacetTableRecord { name, geometry: FacetGeometry::new(width, height), classic: None })
}

/// The maximal runs of [`MIN_FACET_RECORDS`]..=[`MAX_FACET_RECORDS`] consecutive `record_size`-byte records, at
/// pointer steps, in the data sections; `record_at` reads one record.
pub(crate) fn locate_record_runs(
    bytes: &[u8],
    pe: &PeImage,
    record_size: usize,
    record_at: impl Fn(&PeSection, usize) -> Option<FacetTableRecord>,
) -> Vec<FacetTable> {
    let mut tables = Vec::new();
    for section in pe.sections.iter().filter(|section| section.is_data()) {
        let range = section.raw_range();
        for start in (range.start..range.end.min(bytes.len())).step_by(TABLE_SCAN_STEP) {
            let continues_a_run = start >= range.start + record_size && record_at(section, start - record_size).is_some();
            if continues_a_run {
                continue;
            }
            let records: Vec<FacetTableRecord> = (0..)
                .map_while(|index| record_at(section, start + record_size * index))
                .take(MAX_FACET_RECORDS + 1)
                .collect();
            if (MIN_FACET_RECORDS..=MAX_FACET_RECORDS).contains(&records.len()) {
                let va = pe.offset_to_va(start).expect("the table lies in its section");
                tables.push(FacetTable { section: section.name.clone(), offset: start, va, record_size, records });
            }
        }
    }
    tables
}

/// Every candidate 12-byte table (Enhanced Client, Kingdom Reborn).
pub fn locate_facet_tables(bytes: &[u8], pe: &PeImage) -> Vec<FacetTable> {
    locate_record_runs(bytes, pe, FACET_RECORD_SIZE, |section, offset| facet_record_at(bytes, pe, section, offset))
}

/// The `.text` bytes of `pe` and their file offset.
pub(crate) fn code_section<'a>(bytes: &'a [u8], pe: &PeImage) -> eyre::Result<(&'a [u8], usize)> {
    let text = pe.section(CODE_SECTION).ok_or_else(|| eyre!("no {CODE_SECTION} section"))?;
    let range = text.raw_range();
    Ok((bytes.get(range.start..range.end.min(bytes.len())).unwrap_or_default(), range.start))
}

/// The operand of a decoded compare: a register, or the ModRM (reg field cleared), SIB and displacement bytes of a
/// memory operand. Equal operands are equal locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operand<'a> {
    Register(u8),
    Memory(u8, &'a [u8]),
}

/// The r/m operand of the ModRM byte at `at`: its ModRM reg field, the operand and the bytes ModRM, SIB and
/// displacement take.
pub(crate) fn decode_modrm(code: &[u8], at: usize) -> Option<(u8, Operand<'_>, usize)> {
    let modrm = *code.get(at)?;
    let (mode, reg, rm) =
        (modrm >> MODRM_MOD_SHIFT, (modrm >> MODRM_REG_SHIFT) & MODRM_FIELD_MASK, modrm & MODRM_FIELD_MASK);
    if mode == MOD_REGISTER {
        return Some((reg, Operand::Register(rm), 1));
    }
    let mut len = 1;
    let mut absolute = mode == MOD_INDIRECT && rm == RM_DISP32;
    if rm == RM_SIB {
        let base = *code.get(at + len)? & MODRM_FIELD_MASK;
        absolute = mode == MOD_INDIRECT && base == RM_DISP32;
        len += 1;
    }
    len += match mode {
        MOD_DISP8 => size_of::<u8>(),
        MOD_DISP32 => size_of::<u32>(),
        _ if absolute => size_of::<u32>(),
        _ => 0,
    };
    let rest = code.get(at + 1..at + len)?;
    Some((reg, Operand::Memory(modrm & !MODRM_REG_FIELD, rest), len))
}

/// A decoded `CMP r/m32, imm32` (or `CMP EAX, imm32`).
pub(crate) struct CompareImm32<'a> {
    pub operand: Operand<'a>,
    pub imm_offset: usize,
    pub imm: u32,
    pub end: usize,
}

pub(crate) fn decode_cmp_imm32(code: &[u8], at: usize) -> Option<CompareImm32<'_>> {
    let (operand, imm_offset) = match *code.get(at)? {
        OPCODE_CMP_EAX_IMM32 => (Operand::Register(REGISTER_EAX), at + 1),
        OPCODE_CMP_RM32_IMM32 => {
            let (reg, operand, len) = decode_modrm(code, at + 1)?;
            (reg == MODRM_REG_CMP).then_some((operand, at + 1 + len))?
        }
        _ => return None,
    };
    let imm = u32::from_le_bytes(code.get(imm_offset..imm_offset + size_of::<u32>())?.try_into().ok()?);
    Some(CompareImm32 { operand, imm_offset, imm, end: imm_offset + size_of::<u32>() })
}

/// The end of a `CMP r/m32, imm8` with immediate `expected` at `at`.
fn decode_cmp_imm8(code: &[u8], at: usize, expected: u8) -> Option<usize> {
    (*code.get(at)? == OPCODE_CMP_RM32_IMM8).then_some(())?;
    let (reg, _, len) = decode_modrm(code, at + 1)?;
    let imm_at = at + 1 + len;
    (reg == MODRM_REG_CMP && *code.get(imm_at)? == expected).then_some(imm_at + size_of::<u8>())
}

/// A short or near conditional jump at `at`: its condition, target and end.
fn decode_jcc(code: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let opcode = *code.get(at)?;
    let (condition, displacement, end) = if opcode & !JCC_CONDITION_MASK == OPCODE_JCC_SHORT {
        (opcode & JCC_CONDITION_MASK, i64::from(*code.get(at + 1)? as i8), at + JCC_SHORT_SIZE)
    } else if opcode == OPCODE_TWO_BYTE && *code.get(at + 1)? & !JCC_CONDITION_MASK == OPCODE_JCC_NEAR {
        let rel = code.get(at + 2..at + JCC_NEAR_SIZE)?;
        let condition = code[at + 1] & JCC_CONDITION_MASK;
        (condition, i64::from(i32::from_le_bytes(rel.try_into().ok()?)), at + JCC_NEAR_SIZE)
    } else {
        return None;
    };
    let target = usize::try_from(end as i64 + displacement).ok().filter(|target| *target < code.len())?;
    Some((condition, target, end))
}

/// A jump at `at` with one of `conditions`: its target and end.
fn jump_if(code: &[u8], at: usize, conditions: [u8; 2]) -> Option<(usize, usize)> {
    let (condition, target, end) = decode_jcc(code, at)?;
    conditions.contains(&condition).then_some((target, end))
}

/// [`SPLIT_SIGNATURE`] at `at` of `code`: the three compares.
fn match_split(code: &[u8], at: usize) -> Option<[CompareImm32<'_>; SPLIT_IMMEDIATE_COUNT]> {
    let after_facet = decode_cmp_imm8(code, at, SPLIT_FACET_LIMIT)?;
    let (same_half, after) = jump_if(code, after_facet, [CONDITION_G, CONDITION_A])?;
    let first = decode_cmp_imm32(code, after)?;
    let (first_above, after) = jump_if(code, first.end, [CONDITION_GE, CONDITION_AE])?;
    let second = decode_cmp_imm32(code, after)?;
    let (target, different_halves) = jump_if(code, second.end, [CONDITION_L, CONDITION_B])?;
    (target == same_half).then_some(())?;
    let clears = code.get(different_halves..different_halves + CLEAR_RESULT[0].len())?;
    CLEAR_RESULT.iter().any(|clear| clear == clears).then_some(())?;
    let third = decode_cmp_imm32(code, first_above)?;
    (third.operand == second.operand).then_some(())?;
    let (target, _) = jump_if(code, third.end, [CONDITION_L, CONDITION_B])?;
    (target == different_halves).then_some(())?;
    let pops = code.get(same_half..)?.iter().take(MAX_POPS_BEFORE_RESULT);
    let result = same_half + pops.take_while(|opcode| POP_REGISTER_OPCODES.contains(opcode)).count();
    (code.get(result..result + SET_RESULT.len())? == SET_RESULT).then_some([first, second, third])
}

/// Every match of [`SPLIT_SIGNATURE`] in `.text`, whatever its registers, displacements, jump forms and values.
pub fn locate_split_candidates(bytes: &[u8], pe: &PeImage) -> eyre::Result<Vec<SplitCandidate>> {
    let (code, start) = code_section(bytes, pe)?;
    Ok((0..code.len())
        .filter(|&at| code[at] == OPCODE_CMP_RM32_IMM8)
        .filter_map(|at| {
            let compares = match_split(code, at)?;
            Some(SplitCandidate {
                signature_offset: start + at,
                offsets: compares.each_ref().map(|compare| start + compare.imm_offset),
                values: compares.each_ref().map(|compare| compare.imm),
            })
        })
        .collect())
}

/// The split of an Enhanced Client build: at most one match of [`SPLIT_SIGNATURE`], with three equal values.
fn locate_split(bytes: &[u8], pe: &PeImage) -> eyre::Result<Option<FacetSplit>> {
    match locate_split_candidates(bytes, pe)?[..] {
        [] => Ok(None),
        [candidate] => {
            let [first, ..] = candidate.values;
            eyre::ensure!(
                candidate.values.iter().all(|value| *value == first),
                "the three split values at {:#x} differ: {:?}",
                candidate.signature_offset,
                candidate.values
            );
            Ok(Some(FacetSplit {
                signature_offsets: vec![candidate.signature_offset],
                offsets: candidate.offsets.to_vec(),
                x: first,
            }))
        }
        ref candidates => eyre::bail!("{} x = 5120 split sequences found, expected at most 1", candidates.len()),
    }
}

/// Fails on an Enhanced Client or Kingdom Reborn size the client cannot hold; returns the warnings of an allowed but
/// unusual one.
pub fn validate_facet_size(width: u32, height: u32) -> eyre::Result<Vec<String>> {
    for (what, side) in [("width", width), ("height", height)] {
        eyre::ensure!((1..=MAX_FACET_SIDE).contains(&side), "{what} {side} is outside 1..={MAX_FACET_SIDE}");
        eyre::ensure!(side % FACET_SIDE_ALIGNMENT == 0, "{what} {side} is not a multiple of {FACET_SIDE_ALIGNMENT}");
    }
    let sectors = FacetGeometry::new(width, height).sector_count();
    eyre::ensure!(sectors <= MAX_SECTOR_COUNT, "{width}x{height} needs {sectors} sectors, at most {MAX_SECTOR_COUNT}");
    Ok([("width", width), ("height", height)]
        .into_iter()
        .filter(|(_, side)| side % SECTOR_SIDE != 0)
        .map(|(what, side)| format!("{what} {side} is not a multiple of {SECTOR_SIDE}: the last sectors are padded"))
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::pe::tests::{synthetic_pe, TEST_IMAGE_BASE, TEST_SECTION_ALIGNMENT};

    /// Offset of the table inside the synthetic `.data`.
    pub const TABLE_DATA_OFFSET: usize = 16;
    pub const NEW_SPLIT: u32 = 6144;
    /// The shipped facet names.
    const STOCK_NAMES: [&str; STOCK_FACET_COUNT] =
        ["Felucca", "Trammel", "Ilshenar", "Malas", "Tokuno", "Gargoyle", "Test"];
    /// `UOFacet::IsSameFeluccaTrammelHalf` from the facet compare to its end (UOSA.exe file 0x1df6d2), with the
    /// split imm32 at [`SPLIT_IMMEDIATES`].
    const UOSA_SPLIT_CODE: [u8; 51] = [
        0x83, 0xff, 0x01, 0x7f, 0x24, 0x81, 0x7c, 0x24, 0x08, 0x00, 0x14, 0x00, 0x00, 0x7d, 0x12, 0x81, 0xfb, 0x00,
        0x14, 0x00, 0x00, 0x7c, 0x12, 0x32, 0xc0, 0x5e, 0x5b, 0x83, 0xc4, 0x0c, 0xc2, 0x04, 0x00, 0x81, 0xfb, 0x00,
        0x14, 0x00, 0x00, 0x7c, 0xee, 0x5e, 0xb0, 0x01, 0x5b, 0x83, 0xc4, 0x0c, 0xc2, 0x04, 0x00,
    ];
    const SPLIT_IMMEDIATES: [usize; SPLIT_IMMEDIATE_COUNT] = [9, 17, 35];
    /// Code before the split sequence.
    const SPLIT_CODE_START: usize = 8;
    /// Register and displacement variants of [`UOSA_SPLIT_CODE`]: (offset, byte). `CMP ESI,1`, `CMP [ESP+0x10]`,
    /// `CMP ECX` twice.
    const VARIANT_BYTES: [(usize, u8); 4] = [(1, 0xfe), (8, 0x10), (16, 0xf9), (34, 0xf9)];
    /// The second `CMP X` with another register than the first: not the split.
    const MISMATCHED_OPERAND: (usize, u8) = (34, 0xfa);

    fn split_code(x: u32, edits: &[(usize, u8)]) -> Vec<u8> {
        let mut code = vec![0x90; SPLIT_CODE_START];
        code.extend(UOSA_SPLIT_CODE);
        for offset in SPLIT_IMMEDIATES {
            code[SPLIT_CODE_START + offset..][..size_of::<u32>()].copy_from_slice(&x.to_le_bytes());
        }
        for (offset, byte) in edits {
            code[SPLIT_CODE_START + offset] = *byte;
        }
        code.extend([0xcc; SPLIT_CODE_START]);
        code
    }

    /// Writes the computed PE checksum into `image`.
    pub fn fix_checksum(image: &mut [u8]) {
        let pe = PeImage::parse(image).unwrap();
        let checksum = pe_checksum(image, pe.checksum_offset);
        image[pe.checksum_offset..pe.checksum_offset + 4].copy_from_slice(&checksum.to_le_bytes());
    }

    /// A 12-byte-table client: `.text` (`text`, or a nop sled), `.rdata` with the names (and the Enhanced Client
    /// marker when `enhanced`), `.data` with `records` stock records.
    fn synthetic_image(text: Option<Vec<u8>>, records: usize, enhanced: bool, signed: bool) -> Vec<u8> {
        synthetic_image_parts(text, Vec::new(), records, enhanced, signed)
    }

    /// The virtual address of `.rdata` in a synthetic image whose `.text` has `text_len` bytes.
    pub fn synthetic_rdata_va(text_len: usize) -> u32 {
        // The sections are laid out at aligned offsets equal to their RVAs: .text, then .rdata, then .data.
        TEST_IMAGE_BASE + TEST_SECTION_ALIGNMENT * (1 + (text_len as u32).div_ceil(TEST_SECTION_ALIGNMENT))
    }

    /// A stock 7-record client with `text` as `.text` and `rdata_prefix` at the start of `.rdata` (at
    /// [`synthetic_rdata_va`]), the facet names after it.
    pub fn synthetic_image_with(text: Vec<u8>, rdata_prefix: Vec<u8>, enhanced: bool) -> Vec<u8> {
        synthetic_image_parts(Some(text), rdata_prefix, STOCK_FACET_COUNT, enhanced, false)
    }

    fn synthetic_image_parts(text: Option<Vec<u8>>, rdata_prefix: Vec<u8>, records: usize, enhanced: bool, signed: bool) -> Vec<u8> {
        let text = text.unwrap_or_else(|| vec![0x90; 64]);
        let rdata_va = synthetic_rdata_va(text.len());
        let mut rdata = rdata_prefix;
        let mut pointers = Vec::new();
        for name in &STOCK_NAMES[..records] {
            pointers.push(rdata_va + rdata.len() as u32);
            rdata.extend_from_slice(name.as_bytes());
            rdata.push(0);
        }
        if enhanced {
            rdata.extend_from_slice(ENHANCED_CLIENT_MARKER);
            rdata.push(0);
        }
        let mut data = vec![0u8; TABLE_DATA_OFFSET];
        for (pointer, geometry) in pointers.iter().zip(CLIENT_FACET_DIMENSIONS) {
            for value in [*pointer, geometry.width, geometry.height] {
                data.extend_from_slice(&value.to_le_bytes());
            }
        }
        // Room for a second table, so a test can copy one there.
        data.extend(vec![0; FACET_RECORD_SIZE * STOCK_FACET_COUNT + TABLE_DATA_OFFSET]);
        let mut image = synthetic_pe(&[(".text", text), (".rdata", rdata), (".data", data)], signed);
        fix_checksum(&mut image);
        image
    }

    /// An Enhanced Client-like image with the 7 stock records and, when `split` is set, the UOSA split sequence.
    pub fn synthetic_client(split: Option<u32>, signed: bool) -> Vec<u8> {
        synthetic_image(split.map(|x| split_code(x, &[])), STOCK_FACET_COUNT, true, signed)
    }

    /// A Kingdom Reborn-like image: the 7 stock records, no marker, no split.
    pub fn synthetic_kr_client() -> Vec<u8> {
        synthetic_image(None, STOCK_FACET_COUNT, false, false)
    }

    #[test]
    fn table_and_split_are_located() {
        let image = synthetic_client(Some(STOCK_SPLIT_X), false);
        let facets = ClientExeInspection::inspect(&image).unwrap();
        assert!(facets.known.is_none());
        assert_eq!(facets.family, ClientFamily::Enhanced);
        assert_eq!(facets.dimensions(), CLIENT_FACET_DIMENSIONS);
        assert_eq!(facets.table().records.iter().map(|record| record.name.as_str()).collect::<Vec<_>>(), STOCK_NAMES);
        assert_eq!(facets.split.as_ref().unwrap().x, STOCK_SPLIT_X);
        assert!(facets.is_stock() && facets.checksum_valid());
        let data = facets.pe.section(".data").unwrap();
        assert_eq!(facets.table().offset, data.raw_offset as usize + TABLE_DATA_OFFSET);
        let text = facets.pe.section(CODE_SECTION).unwrap().raw_offset as usize + SPLIT_CODE_START;
        let split = facets.split.clone().unwrap();
        assert_eq!(split.signature_offsets, [text]);
        assert_eq!(split.offsets, SPLIT_IMMEDIATES.map(|offset| text + offset));

        let kr = ClientExeInspection::inspect(&synthetic_kr_client()).unwrap();
        assert!(kr.split.is_none() && kr.is_stock());
        assert_eq!(kr.family, ClientFamily::KingdomReborn);

        let mut unequal = image.clone();
        let offset = split.offsets[2];
        unequal[offset] ^= 1;
        assert!(ClientExeInspection::inspect(&unequal).is_err(), "differing split values");
        let mut two_tables = image.clone();
        let (table, table_size) = (facets.table().offset, FACET_RECORD_SIZE * STOCK_FACET_COUNT);
        two_tables.copy_within(table..table + table_size, table + table_size + TABLE_DATA_OFFSET);
        assert!(ClientExeInspection::inspect(&two_tables).is_err(), "two tables");
    }

    #[test]
    fn shorter_tables_are_located_and_too_short_runs_are_not() {
        for records in MIN_FACET_RECORDS..=STOCK_FACET_COUNT {
            let facets = ClientExeInspection::inspect(&synthetic_image(None, records, true, false)).unwrap();
            assert_eq!(facets.facet_count(), records);
            assert!(facets.is_stock());
        }
        assert!(ClientExeInspection::inspect(&synthetic_image(None, MIN_FACET_RECORDS - 1, true, false)).is_err());

        // A record with an illegal size ends a run: record 3 leaves two runs of 3 records, both too short.
        let image = synthetic_client(None, false);
        let facets = ClientExeInspection::inspect(&image).unwrap();
        let mut broken = image.clone();
        let width = facets.table().record_offset(3) + FACET_RECORD_WIDTH_OFFSET;
        broken[width..width + 4].copy_from_slice(&(MAX_FACET_SIDE + FACET_SIDE_ALIGNMENT).to_le_bytes());
        assert!(locate_facet_tables(&broken, &facets.pe).is_empty());
        // A name that is not a word does not count either.
        let mut unnamed = image.clone();
        let name = facets.pe.va_to_offset(read_u32(&image, facets.table().offset).unwrap()).unwrap();
        unnamed[name] = b'#';
        let tables = locate_facet_tables(&unnamed, &facets.pe);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].records.len(), STOCK_FACET_COUNT - 1, "the run starts at Trammel");
    }

    #[test]
    fn the_split_matcher_takes_any_register_and_needs_the_same_x_operand() {
        let stock = ClientExeInspection::inspect(&synthetic_client(Some(STOCK_SPLIT_X), false)).unwrap();
        let variant = synthetic_image(Some(split_code(NEW_SPLIT, &VARIANT_BYTES)), STOCK_FACET_COUNT, true, false);
        let split = ClientExeInspection::inspect(&variant).unwrap().split.unwrap();
        assert_eq!(split.x, NEW_SPLIT);
        assert_eq!(split.offsets, stock.split.unwrap().offsets);

        let code = split_code(STOCK_SPLIT_X, &[MISMATCHED_OPERAND]);
        let mismatched = synthetic_image(Some(code), STOCK_FACET_COUNT, true, false);
        assert!(locate_split_candidates(&mismatched, &PeImage::parse(&mismatched).unwrap()).unwrap().is_empty());
        assert!(ClientExeInspection::inspect(&mismatched).unwrap().split.is_none());

        let mut twice = split_code(STOCK_SPLIT_X, &[]);
        twice.extend(split_code(STOCK_SPLIT_X, &VARIANT_BYTES));
        let image = synthetic_image(Some(twice), STOCK_FACET_COUNT, true, false);
        assert_eq!(locate_split_candidates(&image, &PeImage::parse(&image).unwrap()).unwrap().len(), 2);
        assert!(ClientExeInspection::inspect(&image).is_err(), "two split sequences");
    }
}
