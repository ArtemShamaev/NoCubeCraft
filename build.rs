//! Bakes the models and sounds into the executable, so the game ships as a single file.
//! (Terrain textures and the font are already included by the code that uses them.)

use std::{env, fs, path::Path, process::Command};

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
    let out_dir = Path::new(&env::var("OUT_DIR").unwrap()).to_owned();
    fs::write(out_dir.join("embedded_assets.rs"), out).unwrap();

    if env::var("TARGET").unwrap() == "x86_64-pc-windows-gnu" {
        static_stdcpp(&out_dir);
    }
}

/// meshopt links libstdc++ by name, which MinGW resolves to libstdc++-6.dll — a DLL a bare
/// Windows doesn't have. A search directory holding only the static library wins over it.
fn static_stdcpp(out_dir: &Path) {
    let found = Command::new("x86_64-w64-mingw32-gcc").arg("-print-file-name=libstdc++.a").output().unwrap();
    let lib = String::from_utf8(found.stdout).unwrap();
    let dir = out_dir.join("static");
    fs::create_dir_all(&dir).unwrap();
    fs::copy(lib.trim(), dir.join("libstdc++.a")).unwrap();
    println!("cargo:rustc-link-search=native={}", dir.display());
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
