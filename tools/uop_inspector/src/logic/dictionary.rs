use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use uocf::uop::hash;
use rayon::prelude::*;
use color_eyre::eyre;

pub struct Dictionary {
    hash_to_name: HashMap<u64, String>,
}

impl Dictionary {
    pub fn new() -> Self {
        Self {
            hash_to_name: HashMap::new(),
        }
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> eyre::Result<()> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        
        let lines: Vec<String> = reader.lines()
            .filter_map(|l| l.ok())
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .collect();

        // Use rayon for fast hashing of large dictionaries
        let mappings: HashMap<u64, String> = lines.into_par_iter()
            .map(|name| {
                let hash = hash::hash_file_name_single(&name);
                (hash, name)
            })
            .collect();

        self.hash_to_name.extend(mappings);
        Ok(())
    }

    pub fn resolve(&self, hash: u64) -> Option<&String> {
        self.hash_to_name.get(&hash)
    }

    pub fn load_bin(&mut self, path: &std::path::Path) -> eyre::Result<()> {
        use std::io::{Read, Cursor};
        use byteorder::{LittleEndian, ReadBytesExt};
        
        let data = std::fs::read(path)?;
        let mut reader = Cursor::new(&data);
        
        let count = reader.read_u32::<LittleEndian>().unwrap_or(0);
        for _ in 0..count {
            let hash = reader.read_u64::<LittleEndian>()?;
            let len = reader.read_u32::<LittleEndian>()?;
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
