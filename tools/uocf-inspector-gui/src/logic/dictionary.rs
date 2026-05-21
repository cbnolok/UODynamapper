use color_eyre::eyre;
use uocf::uop_container::hash_dictionary::HashDictionary;

pub struct Dictionary {
    hash_dictionary: HashDictionary,
}

impl Dictionary {
    pub fn new() -> Self {
        Self {
            hash_dictionary: HashDictionary::new(),
        }
    }

    pub fn resolve(&self, hash: u64) -> Option<&str> {
        self.hash_dictionary.resolve(hash)
    }

    pub fn load_dic(&mut self, path: &std::path::Path) -> eyre::Result<()> {
        let incoming = HashDictionary::load(path)?;
        self.hash_dictionary.merge(incoming);
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.hash_dictionary.named_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_resolve_and_count() {
        let mut dict = Dictionary::new();
        assert_eq!(dict.count(), 0);
        assert_eq!(dict.resolve(12345), None);

        dict.hash_dictionary.set(12345, "test_name");
        assert_eq!(dict.count(), 1);
        assert_eq!(dict.resolve(12345), Some("test_name"));
    }

    #[test]
    fn test_dictionary_load_dic() {
        let path = std::env::temp_dir().join("uocf_inspector_test_dict.dic");
        let mut source = HashDictionary::new();
        source.set(1001, "first_entry");
        source.set(2002, "second_entry");
        source.save(&path).unwrap();

        let mut dict = Dictionary::new();
        let res = dict.load_dic(&path);
        
        let _ = std::fs::remove_file(path);

        assert!(res.is_ok());
        assert_eq!(dict.count(), 2);
        assert_eq!(dict.resolve(1001), Some("first_entry"));
        assert_eq!(dict.resolve(2002), Some("second_entry"));
        assert_eq!(dict.resolve(3003), None);
    }
}
