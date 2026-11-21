//! Bakes the models and sounds into the executable, so the game ships as a single file.
//! (Terrain textures and the font are already included by the code that uses them.)

use std::{env, fs, path::Path};

const EMBEDDED: [&str; 2] = ["models", "sounds"];

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let mut files = Vec::new();
    for dir in EMBEDDED {
        println!("cargo:rerun-if-changed=assets/{dir}");
        collect(&root, &root.join(dir), &mut files);
    }
    files.sort();
    let mut out = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for (name, path) in files {
        out += &format!("    ({name:?}, include_bytes!({path:?})),\n");
    }
    out += "];\n";
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("embedded_assets.rs"), out).unwrap();
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let name = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            files.push((name, path.to_string_lossy().into_owned()));
        }
    }
}
