fn main() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bsp/board-a");
    println!("cargo:rustc-link-search={}", directory.display());
    println!(
        "cargo:rerun-if-changed={}",
        directory.join("memory.x").display()
    );
}
