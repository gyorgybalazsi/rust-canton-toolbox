use std::path::Path;

fn main() {
    // Rust types generated from the example DARs by the standalone codegen tool
    // (https://github.com/gyorgybalazsi/daml-rust-codegen), checked in under test/generated/.
    let generated_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("generated");
    let out_dir = std::env::var("OUT_DIR").unwrap();

    for entry in std::fs::read_dir(&generated_dir).expect("Failed to read test/generated/") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let dest = Path::new(&out_dir).join(path.file_name().unwrap());
            std::fs::copy(&path, &dest).expect("Failed to copy generated file");
        }
    }

    println!("cargo::rerun-if-changed={}", generated_dir.display());
}
