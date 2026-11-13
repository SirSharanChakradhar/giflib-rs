fn main() {
    // Same soname as upstream giflib 5.x/6.x, so the dynamic loader treats
    // this library as a drop-in replacement for libgif.so.7.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" || os == "android" || os.ends_with("bsd") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libgif.so.7");
        // Export exactly upstream's symbols and nothing else.
        let map = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("libgif.map");
        println!("cargo:rustc-cdylib-link-arg=-Wl,--version-script={}", map.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=libgif.map");
}
