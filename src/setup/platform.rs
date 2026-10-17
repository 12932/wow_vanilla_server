//! What the setup asks of the operating system: free disk space, and on
//! Windows the fixed drives and the client's registry entry.

use std::path::{Path, PathBuf};

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

/// The roots of the fixed drives (`C:\`, `D:\`), for the full search.
#[cfg(windows)]
pub fn fixed_drives() -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_FIXED: u32 = 3;
    // SAFETY: no arguments.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|root| {
            let w = wide(std::ffi::OsStr::new(root));
            // SAFETY: `w` is NUL-terminated.
            unsafe { GetDriveTypeW(w.as_ptr()) == DRIVE_FIXED }
        })
        .map(PathBuf::from)
        .collect()
}

/// Every drive's root: on Windows the fixed drives, elsewhere `/`.
#[cfg(not(windows))]
pub fn fixed_drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

/// Where Blizzard's installer said it put the client: `InstallPath` under
/// `Blizzard Entertainment\World of Warcraft`, for the machine or the user.
#[cfg(windows)]
pub fn registry_install_paths() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };
    let mut found = Vec::new();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for key in [
            r"SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft",
            r"SOFTWARE\Blizzard Entertainment\World of Warcraft",
        ] {
            let key = wide(std::ffi::OsStr::new(key));
            let value = wide(std::ffi::OsStr::new("InstallPath"));
            let mut buf = vec![0u16; 1024];
            let mut bytes = (buf.len() * 2) as u32;
            // SAFETY: the strings are NUL-terminated and `bytes` is `buf`'s
            // size in bytes.
            let status = unsafe {
                RegGetValueW(
                    root,
                    key.as_ptr(),
                    value.as_ptr(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            if status != 0 {
                continue;
            }
            let len = (bytes as usize / 2).min(buf.len());
            let s = &buf[..len];
            let s = s.split(|&c| c == 0).next().unwrap_or(s);
            let path = PathBuf::from(std::ffi::OsString::from_wide(s));
            if !path.as_os_str().is_empty() && !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(not(windows))]
pub fn registry_install_paths() -> Vec<PathBuf> {
    Vec::new()
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
        assert!(!fixed_drives().is_empty());
    }
}
