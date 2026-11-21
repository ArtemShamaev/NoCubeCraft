//! Assets baked into the executable by `build.rs`, served as the default asset source.

use std::path::Path;

use bevy::asset::io::{
    AssetSourceBuilder,
    memory::{Dir, MemoryAssetReader},
};

include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

pub fn source() -> AssetSourceBuilder {
    let root = Dir::default();
    for (path, bytes) in FILES {
        root.insert_asset(Path::new(path), *bytes);
    }
    AssetSourceBuilder::new(move || Box::new(MemoryAssetReader { root: root.clone() }))
}
