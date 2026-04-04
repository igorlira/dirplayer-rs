fn main() {
    // 0x200000 bytes = 2MiB — only for the WASM linker
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("wasm32") {
        println!("cargo:rustc-link-arg=-zstack-size=0x800000");
    }
}
