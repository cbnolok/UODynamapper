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

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};
    use std::io::Write;

    #[test]
    fn test_dictionary_resolve_and_count() {
        let mut dict = Dictionary::new();
        assert_eq!(dict.count(), 0);
        assert_eq!(dict.resolve(12345), None);

        dict.hash_to_name.insert(12345, "test_name".to_string());
        assert_eq!(dict.count(), 1);
        assert_eq!(dict.resolve(12345), Some(&"test_name".to_string()));
    }

    #[test]
    fn test_dictionary_load_bin() {
        let path = std::path::Path::new("test_dict_temp.bin");

        let mut bytes = Vec::new();
        // Write count (2)
        bytes.write_u32::<LittleEndian>(2).unwrap();

        // Entry 1
        bytes.write_u64::<LittleEndian>(1001).unwrap();
        let name1 = b"first_entry";
        bytes.write_u32::<LittleEndian>(name1.len() as u32).unwrap();
        bytes.write_all(name1).unwrap();

        // Entry 2
        bytes.write_u64::<LittleEndian>(2002).unwrap();
        let name2 = b"second_entry";
        bytes.write_u32::<LittleEndian>(name2.len() as u32).unwrap();
        bytes.write_all(name2).unwrap();

        std::fs::write(path, &bytes).unwrap();

        let mut dict = Dictionary::new();
        let res = dict.load_bin(path);
        
        // Clean up immediately
        let _ = std::fs::remove_file(path);

        assert!(res.is_ok());
        assert_eq!(dict.count(), 2);
        assert_eq!(dict.resolve(1001), Some(&"first_entry".to_string()));
        assert_eq!(dict.resolve(2002), Some(&"second_entry".to_string()));
        assert_eq!(dict.resolve(3003), None);
    }
}
