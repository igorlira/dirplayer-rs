fn main() {
    // 0x200000 bytes = 2MiB — only valid for wasm targets
    if std::env::var("TARGET").unwrap_or_default().contains("wasm32") {
        println!("cargo:rustc-link-arg=-zstack-size=0x800000");
    }
}
