//! What the setup asks of the operating system: the free disk space.

use std::path::Path;

/// Bytes free to this user on the volume holding `path`, or the nearest
/// folder above it that exists.
pub fn free_space(path: &Path) -> Option<u64> {
    let path = std::path::absolute(path).ok()?;
    let existing = path.ancestors().find(|p| p.exists())?;
    free_space_at(existing)
}

#[cfg(unix)]
fn free_space_at(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c` is a NUL-terminated path, and `s` is written in full when
    // the call succeeds.
    if unsafe { libc::statvfs(c.as_ptr(), s.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: as above.
    let s = unsafe { s.assume_init() };
    // The fields' types differ between platforms.
    #[allow(clippy::unnecessary_cast)]
    let free = s.f_bavail as u64 * s.f_frsize as u64;
    Some(free)
}

#[cfg(windows)]
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain([0]).collect()
}

#[cfg(windows)]
fn free_space_at(path: &Path) -> Option<u64> {
    let w = wide(path.as_os_str());
    let mut free = 0u64;
    // SAFETY: `w` is NUL-terminated; the two totals may be null.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            w.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(not(any(unix, windows)))]
fn free_space_at(_: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_not_made_yet_has_its_parents_free_space() {
        let dir = std::env::temp_dir();
        let here = free_space(&dir).expect("the temp dir's free space");
        let below = free_space(&dir.join("wvs-setup-not-there/nav")).unwrap();
        // Another process may write in between: the same volume, roughly.
        assert!(here.abs_diff(below) < 1 << 30, "{here} {below}");
    }
}
