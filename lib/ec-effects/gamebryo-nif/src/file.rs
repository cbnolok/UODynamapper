//! The NIF file container: header, block list and footer.
//!
//! The header is hand-written for the generated version (see [`crate::NIF_VERSION`]); in
//! 20.0.0.4 it has no block size table and no string table, and strings are stored inline in the
//! blocks. The block type table is rebuilt on write from the blocks, in first-use order, which is
//! the order Gamebryo writes.

use crate::error::{ErrorKind, NifError, Result};
use crate::generated::{Block, BlockType, NiObject};
use crate::io::{NifIo, Reader, Writer};
use crate::reflect::{Field, Value};
use crate::types::{Link, LinkKind, NiObjectType, NifString, Ref};

/// `Endian Type` value for little-endian files.
pub const ENDIAN_LITTLE: u8 = 1;

/// Mask of the block type index; the top bit flags PhysX block types in later versions.
const BLOCK_TYPE_INDEX_FLAG: u16 = 0x8000;

/// The NIF header fields that are not derived from the blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// The first line without its newline, e.g. `Gamebryo File Format, Version 20.0.0.4`.
    pub header_string: NifString,
    pub version: u32,
    pub endian_type: u8,
    pub user_version: u32,
    pub groups: Vec<u32>,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            header_string: NifString::from(
                format!("Gamebryo File Format, Version {}", crate::version_string(crate::NIF_VERSION))
                    .as_str(),
            ),
            version: crate::NIF_VERSION,
            endian_type: ENDIAN_LITTLE,
            user_version: crate::NIF_USER_VERSION,
            groups: Vec::new(),
        }
    }
}

/// A parsed NIF file. Blocks refer to each other by index into `blocks`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NifFile {
    pub header: Header,
    pub blocks: Vec<Block>,
    /// The footer's root objects.
    pub roots: Vec<Ref<NiObject>>,
}

/// A link that points at a missing block or a block of the wrong type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkIssue {
    /// The block holding the link, or `None` for the footer's roots.
    pub block: Option<usize>,
    /// Field path inside the block, e.g. `Children[2]`.
    pub field: String,
    pub raw: i32,
    pub kind: LinkKind,
    /// The type the link must point at.
    pub expected: &'static str,
    /// The type it actually points at, or `None` if the index is out of range.
    pub found: Option<&'static str>,
}

/// Reads a NIF file. Same as [`NifFile::read`].
pub fn read(data: &[u8]) -> Result<NifFile> {
    NifFile::read(data)
}

/// Reads a NIF file and checks its links ([`NifFile::link_issues`]): the check a NIF passes before it replaces a
/// shipped one. The error names the first wrong link and how many there are.
pub fn read_checked(data: &[u8]) -> std::result::Result<NifFile, String> {
    let nif = NifFile::read(data).map_err(|error| error.to_string())?;
    let issues = nif.link_issues();
    if let Some(first) = issues.first() {
        let holder = first.block.map_or_else(|| "the footer".to_owned(), |block| format!("block {block}"));
        let found = first.found.unwrap_or("no block (out of range)");
        return Err(format!(
            "{} wrong link(s); the first: {holder} {} = {} must point at a {}, points at {found}",
            issues.len(),
            first.field,
            first.raw,
            first.expected
        ));
    }
    Ok(nif)
}

/// Writes a NIF file. Same as [`NifFile::write`].
pub fn write(nif: &NifFile) -> Result<Vec<u8>> {
    nif.write()
}

impl NifFile {
    /// Parses a whole file; fails on unknown block types and trailing bytes.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let header_string = NifString(r.line()?.to_vec());
        if !header_string.0.starts_with(b"Gamebryo File Format, Version ") {
            return Err(NifError::new(ErrorKind::BadHeaderString, Some(0)));
        }
        let field = |name| move |e: NifError| e.in_field(name);
        let version = u32::read(&mut r, 0).map_err(field("nif_file_version"))?;
        if version != crate::NIF_VERSION {
            return Err(r.error(ErrorKind::UnsupportedVersion { version }));
        }
        let endian_type = u8::read(&mut r, 0).map_err(field("source_is_little_endian"))?;
        if endian_type != ENDIAN_LITTLE {
            return Err(r.error(ErrorKind::UnsupportedEndian { endian: endian_type }));
        }
        let user_version = u32::read(&mut r, 0).map_err(field("nif_file_user_defined_version"))?;
        if user_version != crate::NIF_USER_VERSION {
            return Err(r.error(ErrorKind::UnsupportedUserVersion { user_version }));
        }
        let num_blocks = u32::read(&mut r, 0).map_err(field("objects"))?;
        let num_block_types = u16::read(&mut r, 0).map_err(field("rtti_count"))?;
        let mut block_types = Vec::with_capacity(usize::from(num_block_types));
        for index in 0..usize::from(num_block_types) {
            let name = NifString::read(&mut r, 0)
                .map_err(|e| e.at_index(index).in_field("rtti"))?;
            match BlockType::from_name(name.as_bytes()) {
                Some(ty) => block_types.push(ty),
                None => {
                    return Err(r
                        .error(ErrorKind::UnknownBlockType { name: name.to_string_lossy() })
                        .at_index(index)
                        .in_field("rtti"));
                }
            }
        }
        let type_indices: Vec<u16> =
            crate::io::rd_vec(&mut r, "rtti_index", i64::from(num_blocks), 0)?;
        let num_groups = u32::read(&mut r, 0).map_err(field("groups"))?;
        let groups = crate::io::rd_vec(&mut r, "Groups", i64::from(num_groups), 0)?;

        let mut blocks = Vec::with_capacity(type_indices.len());
        for (index, &type_index) in type_indices.iter().enumerate() {
            let ty = block_types
                .get(usize::from(type_index))
                .filter(|_| type_index & BLOCK_TYPE_INDEX_FLAG == 0)
                .copied()
                .ok_or_else(|| {
                    r.error(ErrorKind::BadBlockTypeIndex {
                        index: type_index,
                        num_types: block_types.len(),
                    })
                    .at_index(index)
                    .in_field("rtti_index")
                })?;
            let block = Block::read(ty, &mut r).map_err(|e| e.in_block(index, ty.name()))?;
            blocks.push(block);
        }

        let num_roots = u32::read(&mut r, 0).map_err(field("top_objects"))?;
        let roots = crate::io::rd_vec(&mut r, "Roots", i64::from(num_roots), 0)?;
        if r.remaining() != 0 {
            return Err(r.error(ErrorKind::TrailingData { bytes: r.remaining() }));
        }
        Ok(Self {
            header: Header { header_string, version, endian_type, user_version, groups },
            blocks,
            roots,
        })
    }

    /// Serializes the file. Fails if an array's length disagrees with its count field.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        let header = &self.header;
        w.bytes(header.header_string.as_bytes());
        w.bytes(b"\n");
        header.version.write(&mut w, 0)?;
        header.endian_type.write(&mut w, 0)?;
        header.user_version.write(&mut w, 0)?;
        count_u32(&w, self.blocks.len())?.write(&mut w, 0)?;

        let mut types: Vec<BlockType> = Vec::new();
        let mut type_indices = Vec::with_capacity(self.blocks.len());
        for block in &self.blocks {
            let ty = block.block_type();
            let index = match types.iter().position(|&t| t == ty) {
                Some(index) => index,
                None => {
                    types.push(ty);
                    types.len() - 1
                }
            };
            type_indices.push(u16::try_from(index).map_err(|_| {
                w.error(ErrorKind::CountOverflow { count: index }).in_field("rtti_index")
            })?);
        }
        u16::try_from(types.len())
            .map_err(|_| w.error(ErrorKind::CountOverflow { count: types.len() }))?
            .write(&mut w, 0)?;
        for ty in &types {
            NifString(ty.name().as_bytes().to_vec()).write(&mut w, 0)?;
        }
        for index in &type_indices {
            index.write(&mut w, 0)?;
        }
        count_u32(&w, header.groups.len())?.write(&mut w, 0)?;
        for group in &header.groups {
            group.write(&mut w, 0)?;
        }

        for (index, block) in self.blocks.iter().enumerate() {
            block.write(&mut w).map_err(|e| e.in_block(index, block.type_name()))?;
        }

        count_u32(&w, self.roots.len())?.write(&mut w, 0)?;
        for root in &self.roots {
            root.write(&mut w, 0)?;
        }
        Ok(w.into_bytes())
    }

    /// The block a link points at, if it exists and has the link's target type.
    pub fn get<L: Link>(&self, link: L) -> Option<&L::Target> {
        L::Target::from_block(self.blocks.get(link.index()?)?)
    }

    pub fn get_mut<L: Link>(&mut self, link: L) -> Option<&mut L::Target> {
        L::Target::from_block_mut(self.blocks.get_mut(link.index()?)?)
    }

    /// Indices of the blocks of type `T` (including subtypes).
    pub fn blocks_of<T: NiObjectType>(&self) -> impl Iterator<Item = (usize, &T)> {
        self.blocks.iter().enumerate().filter_map(|(i, b)| T::from_block(b).map(|t| (i, t)))
    }

    /// Checks that every non-null link (in blocks and footer) points at an existing block of the
    /// declared type.
    pub fn link_issues(&self) -> Vec<LinkIssue> {
        let mut issues = Vec::new();
        for (index, block) in self.blocks.iter().enumerate() {
            let mut path = String::new();
            self.check_fields(Some(index), &block.fields(), &mut path, &mut issues);
        }
        for (i, root) in self.roots.iter().enumerate() {
            let field = format!("Roots[{i}]");
            self.check_link(None, field, root.raw(), LinkKind::Ref, NiObject::NAME, &mut issues);
        }
        issues
    }

    fn check_fields(
        &self,
        block: Option<usize>,
        fields: &[Field<'_>],
        path: &mut String,
        issues: &mut Vec<LinkIssue>,
    ) {
        for field in fields.iter().filter(|f| f.present) {
            let len = path.len();
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(field.info.name);
            self.check_value(block, field.value, path, issues);
            path.truncate(len);
        }
    }

    fn check_value(
        &self,
        block: Option<usize>,
        value: Value<'_>,
        path: &mut String,
        issues: &mut Vec<LinkIssue>,
    ) {
        match value {
            Value::Link { raw, kind, target } => {
                self.check_link(block, path.clone(), raw, kind, target, issues)
            }
            Value::Struct(s) => self.check_fields(block, &s.fields(), path, issues),
            Value::List(list) => {
                for (i, item) in list.iter().enumerate() {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    self.check_value(block, item, path, issues);
                    path.truncate(len);
                }
            }
            _ => {}
        }
    }

    fn check_link(
        &self,
        block: Option<usize>,
        field: String,
        raw: i32,
        kind: LinkKind,
        expected: &'static str,
        issues: &mut Vec<LinkIssue>,
    ) {
        if raw == crate::types::NULL_LINK {
            return;
        }
        let target = usize::try_from(raw).ok().and_then(|i| self.blocks.get(i));
        if !target.is_some_and(|t| t.block_type().is_a(expected)) {
            let found = target.map(Block::type_name);
            issues.push(LinkIssue { block, field, raw, kind, expected, found });
        }
    }
}

fn count_u32(w: &Writer, count: usize) -> Result<u32> {
    u32::try_from(count).map_err(|_| w.error(ErrorKind::CountOverflow { count }))
}
