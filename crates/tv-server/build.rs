fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let dist = std::path::Path::new(&manifest).join("../../web/dist");
    if !dist.exists() && std::fs::create_dir_all(&dist).is_ok() {
        let _ = std::fs::write(
            dist.join("index.html"),
            "<!DOCTYPE html><html><body>Run: cd web && bun run build</body></html>",
        );
    }
    println!("cargo:rerun-if-changed=../../web/dist");
}
