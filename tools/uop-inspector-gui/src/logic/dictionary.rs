use color_eyre::eyre;
use std::collections::HashMap;

pub struct Dictionary {
    hash_to_name: HashMap<u64, String>,
}

impl Dictionary {
    pub fn new() -> Self {
        Self {
            hash_to_name: HashMap::new(),
        }
    }

    pub fn resolve(&self, hash: u64) -> Option<&String> {
        self.hash_to_name.get(&hash)
    }

    pub fn load_bin(&mut self, path: &std::path::Path) -> eyre::Result<()> {
        use byteorder::{LittleEndian, ReadBytesExt};
        use std::io::{Cursor, Read};

        let data = std::fs::read(path)?;
        let mut reader = Cursor::new(&data);

        let count = reader.read_u32::<LittleEndian>().unwrap_or(0);
        for _ in 0..count {
            let hash = reader.read_u64::<LittleEndian>()?;
            let len = reader.read_u32::<LittleEndian>()?;
            if len > 100 {
                eyre::bail!("Found entry with large length: {}", len);
            }
            let mut buf = vec![0u8; len as usize];
            reader.read_exact(&mut buf)?;
            if let Ok(s) = String::from_utf8(buf) {
                self.hash_to_name.insert(hash, s);
            }
        }
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.hash_to_name.len()
    }
}
