fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // A dependency's Windows build references wsprintfA from user32;
        // `rustc-link-arg-bins` passes it to every binary target too.
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-arg-bins=user32.lib");
    }
}
