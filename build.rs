fn main() {
    // It reads only the target's cfg: without this, cargo reruns it (and
    // rebuilds the crate) after any change in the package, a doc's too.
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // A dependency's Windows build references wsprintfA from user32;
        // `rustc-link-arg-bins` passes it to every binary target too, in
        // the linker's own words: MSVC's link.exe takes a file name, MinGW's
        // ld a `-l`.
        println!("cargo:rustc-link-lib=user32");
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-bins=user32.lib");
        } else {
            println!("cargo:rustc-link-arg-bins=-luser32");
        }
    }
}
