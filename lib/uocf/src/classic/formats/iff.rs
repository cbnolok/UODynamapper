//! The IFF container (EA IFF 85) of the Classic client's localized UI text (`Intloc*`, `Chat.*`, `Tilehelp.*`
//! and the rest) and of `Langcode.iff`, read and written byte for byte ([`IffNode`]).
//!
//! A node is a 4-byte tag and a big-endian `u32` size, then that many bytes, then a zero byte when the size is
//! odd. A `FORM` node's bytes are a 4-byte kind and its children; its size counts their padding. Every shipped
//! file is one `FORM`.

crate::eyre_imports!();

/// The tag of a node holding children.
pub const FORM_TAG: [u8; 4] = *b"FORM";
/// Bytes of a tag, and of a form's kind.
pub const TAG_BYTES: usize = 4;
/// Bytes of a node header: the tag and the size.
pub const NODE_HEADER_BYTES: usize = TAG_BYTES + size_of::<u32>();
/// The value of the byte after an odd-sized node.
const PAD_BYTE: u8 = 0;

/// One IFF node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IffNode {
    Form { kind: [u8; 4], children: Vec<IffNode> },
    Chunk { tag: [u8; 4], data: Vec<u8> },
}

/// A tag as text, for messages.
pub fn tag_name(tag: &[u8; 4]) -> String {
    tag.iter().map(|&byte| if byte.is_ascii_graphic() { char::from(byte) } else { '?' }).collect()
}

impl IffNode {
    /// Reads a file that is one node, with nothing after it.
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let (node, end) = read_node(bytes, 0, bytes.len())?;
        if end != bytes.len() {
            eyre::bail!("{} bytes after the {} node", bytes.len() - end, node.tag_name());
        }
        Ok(node)
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.write(&mut bytes)?;
        Ok(bytes)
    }

    pub fn tag_name(&self) -> String {
        match self {
            Self::Form { kind, .. } => format!("FORM {}", tag_name(kind)),
            Self::Chunk { tag, .. } => tag_name(tag),
        }
    }

    fn write(&self, out: &mut Vec<u8>) -> eyre::Result<()> {
        let start = out.len();
        match self {
            Self::Form { kind, children } => {
                out.extend_from_slice(&FORM_TAG);
                out.extend_from_slice(&[0; size_of::<u32>()]);
                out.extend_from_slice(kind);
                for child in children {
                    child.write(out)?;
                }
            }
            Self::Chunk { tag, data } => {
                if *tag == FORM_TAG {
                    eyre::bail!("a chunk cannot be tagged FORM");
                }
                out.extend_from_slice(tag);
                out.extend_from_slice(&[0; size_of::<u32>()]);
                out.extend_from_slice(data);
            }
        }
        let size = out.len() - start - NODE_HEADER_BYTES;
        let size_field = u32::try_from(size).map_err(|_| eyre!("the {} node's {size} bytes do not fit 32 bits", self.tag_name()))?;
        out[start + TAG_BYTES..start + NODE_HEADER_BYTES].copy_from_slice(&size_field.to_be_bytes());
        if size % 2 == 1 {
            out.push(PAD_BYTE);
        }
        Ok(())
    }

    /// The children of a form, none for a chunk.
    pub fn children(&self) -> &[IffNode] {
        match self {
            Self::Form { children, .. } => children,
            Self::Chunk { .. } => &[],
        }
    }
}

fn read_node(bytes: &[u8], start: usize, limit: usize) -> eyre::Result<(IffNode, usize)> {
    if start + NODE_HEADER_BYTES > limit {
        eyre::bail!("at {start}: {} bytes, fewer than a node header", limit - start);
    }
    let tag: [u8; 4] = bytes[start..start + TAG_BYTES].try_into().expect("four bytes");
    let size = u32::from_be_bytes(bytes[start + TAG_BYTES..start + NODE_HEADER_BYTES].try_into().expect("four bytes")) as usize;
    let data_start = start + NODE_HEADER_BYTES;
    let data_end = data_start.checked_add(size).filter(|&end| end <= limit).ok_or_else(|| {
        eyre!("at {start}: the {} node's {size} bytes run past its container's end at {limit}", tag_name(&tag))
    })?;
    let mut end = data_end;
    if size % 2 == 1 {
        match bytes.get(data_end).filter(|_| data_end < limit) {
            Some(&PAD_BYTE) => end += 1,
            Some(&other) => eyre::bail!("at {data_end}: the pad byte after the {} node is {other:#04x}, not 0", tag_name(&tag)),
            None => eyre::bail!("at {data_end}: the {} node's size is odd and no pad byte follows", tag_name(&tag)),
        }
    }
    if tag != FORM_TAG {
        return Ok((IffNode::Chunk { tag, data: bytes[data_start..data_end].to_vec() }, end));
    }
    if size < TAG_BYTES {
        eyre::bail!("at {start}: a FORM of {size} bytes has no kind");
    }
    let kind: [u8; 4] = bytes[data_start..data_start + TAG_BYTES].try_into().expect("four bytes");
    let mut children = Vec::new();
    let mut position = data_start + TAG_BYTES;
    while position < data_end {
        let (child, child_end) = read_node(bytes, position, data_end)
            .wrap_err_with(|| format!("in the FORM {} at {start}", tag_name(&kind)))?;
        children.push(child);
        position = child_end;
    }
    Ok((IffNode::Form { kind, children }, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_round_trip_with_their_padding() {
        let file = IffNode::Form {
            kind: *b"DATA",
            children: vec![
                IffNode::Chunk { tag: *b"INFO", data: vec![1, 2, 3] },
                IffNode::Form { kind: *b"LANG", children: vec![IffNode::Chunk { tag: *b"TEXT", data: b"a\0".to_vec() }] },
            ],
        };
        let bytes = file.to_bytes().unwrap();
        assert_eq!(&bytes[TAG_BYTES..NODE_HEADER_BYTES], &((bytes.len() - NODE_HEADER_BYTES) as u32).to_be_bytes());
        assert_eq!(bytes[NODE_HEADER_BYTES + TAG_BYTES + NODE_HEADER_BYTES + 3], PAD_BYTE);
        assert_eq!(IffNode::from_bytes(&bytes).unwrap(), file);

        let mut bad_pad = bytes.clone();
        bad_pad[NODE_HEADER_BYTES + TAG_BYTES + NODE_HEADER_BYTES + 3] = 1;
        assert!(format!("{:#}", IffNode::from_bytes(&bad_pad).unwrap_err()).contains("pad byte"));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(IffNode::from_bytes(&trailing).is_err());
        assert!(IffNode::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    }
}
