fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // namigator-sys's stormlib (Windows path) references wsprintfA
        // from user32. The library build picks this up via the normal
        // link line, but binaries in the same package that don't
        // depend on the lib crate (e.g. `loadtest`, which has no
        // `use wow_vanilla_server::*`) wouldn't otherwise inherit it.
        // `rustc-link-arg-bins` covers every binary target in the
        // package; `rustc-link-lib` still drives the lib's link.
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-arg-bins=user32.lib");
    }
}
