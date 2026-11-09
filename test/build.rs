use std::path::Path;

fn main() {
    let codegen_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("codegen")
        .join("generated");
    let out_dir = std::env::var("OUT_DIR").unwrap();

    for entry in std::fs::read_dir(&codegen_dir).expect("Failed to read codegen/generated/") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let dest = Path::new(&out_dir).join(path.file_name().unwrap());
            std::fs::copy(&path, &dest).expect("Failed to copy generated file");
        }
    }

    println!("cargo::rerun-if-changed={}", codegen_dir.display());
}
