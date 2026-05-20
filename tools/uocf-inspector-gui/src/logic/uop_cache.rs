use std::path::PathBuf;
use std::sync::Arc;
use uocf::classic::art::ArtMap;
use uocf::classic::tiledata::TileData;
// use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::uop_container::package::UopPackage;

pub struct LoadedUop {
    pub path: PathBuf,
    pub package: UopPackage,
}

pub struct UopCache {
    pub loaded_uops: Vec<Arc<LoadedUop>>,
}

impl UopCache {
    pub fn new() -> Self {
        Self {
            loaded_uops: Vec::new(),
        }
    }

    pub fn add(&mut self, path: PathBuf, package: UopPackage) {
        self.loaded_uops.push(Arc::new(LoadedUop { path, package }));
    }
}

pub struct ClientData {
    pub path: std::path::PathBuf,
    pub art: Arc<ArtMap>,
    pub tiledata: Arc<TileData>,
    pub multis: Option<Arc<uocf::classic::multi::MultiMap>>,
    pub _ec_multis: Option<Arc<uocf::enhanced::multis::MultiCollection>>,
    pub hues: Option<Arc<Vec<uocf::classic::hues::HueEntry>>>,
    pub anim_defs: Option<Arc<uocf::classic::anim::AnimationDefinition>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uop_cache_creation_and_addition() {
        let mut cache = UopCache::new();
        assert_eq!(cache.loaded_uops.len(), 0);

        let dummy_package = UopPackage::new_default();
        cache.add(PathBuf::from("dummy_path.uop"), dummy_package);

        assert_eq!(cache.loaded_uops.len(), 1);
        assert_eq!(cache.loaded_uops[0].path, PathBuf::from("dummy_path.uop"));
    }
}
