fn main() {
    // 0x200000 bytes = 2MiB
    println!("cargo:rustc-link-arg=-zstack-size=0x800000")
}
