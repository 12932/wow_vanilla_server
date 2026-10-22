use std::process::Command;

/// `git` with `args` in the package, its output trimmed, if it ran.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The commit built, as `WVS_GIT_HASH` (git's short form, 7 characters):
/// the environment's if a build without the repository sets it (a
/// container's), else git's, else "unknown". Rebuilt as the commit moves:
/// HEAD, and the branch it names: its own ref file, or, only if git has
/// packed it away, the packed refs (which a fetch or git's upkeep rewrites
/// with no commit of ours moving: watched always, they rebuilt the crate
/// minutes after a build with nothing changed). Only files that exist are
/// watched: cargo takes a missing one as changed, every build.
fn git_hash() {
    println!("cargo:rerun-if-env-changed=WVS_GIT_HASH");
    let hash = std::env::var("WVS_GIT_HASH")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| git(&["rev-parse", "--short=7", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=WVS_GIT_HASH={hash}");
    let path_of = |name: &str| {
        git(&["rev-parse", "--git-path", name])
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
    };
    // HEAD, and the local branches' folder (a commit on a packed branch
    // writes its file there anew; cargo scans a folder it watches): a
    // fetch writes only `refs/remotes`, so it rebuilds nothing.
    let mut watch: Vec<std::path::PathBuf> = ["HEAD", "refs/heads"]
        .iter()
        .filter_map(|n| path_of(n))
        .collect();
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"])
        && path_of(&branch).is_none()
        && let Some(p) = path_of("packed-refs")
    {
        // The branch lives in the packed refs alone.
        watch.push(p);
    }
    for p in watch {
        println!("cargo:rerun-if-changed={}", p.display());
    }
}

fn main() {
    // It reads only the target's cfg and the commit: without this, cargo
    // reruns it (and rebuilds the crate) after any change in the package, a
    // doc's too.
    println!("cargo:rerun-if-changed=build.rs");
    git_hash();
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
