// A reproducible source fingerprint for evidence recorded by experimental Co-Play.
// Only repository sources are read; local settings and saves are never included.
fn visit(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).expect("source directory").flatten() {
            visit(&entry.path(), files);
        }
    } else {
        files.push(path.to_owned());
    }
}
fn main() {
    let mut files = Vec::new();
    for path in [
        "src",
        "Cargo.toml",
        "world_api/schema.yaml",
        "world_api/world_api_compact.md",
    ] {
        println!("cargo:rerun-if-changed={path}");
        visit(std::path::Path::new(path), &mut files);
    }
    files.sort();
    let mut hash = 0xcbf29ce484222325u64;
    for path in files {
        for byte in path
            .to_string_lossy()
            .bytes()
            .chain(std::fs::read(&path).expect("source file"))
        {
            hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
        }
    }
    println!("cargo:rustc-env=VOXEL_BUILD_FINGERPRINT={hash:016x}");
}
