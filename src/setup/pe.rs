//! An executable's file version, from its version resource (`VS_FIXEDFILEINFO`):
//! enough of the PE format to find `WoW.exe`'s build.

/// The four parts of a file version: 1.12.1.5875 for the client the server
/// speaks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u16, pub u16, pub u16, pub u16);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{} ({})", self.0, self.1, self.2, self.3)
    }
}

const RT_VERSION: u32 = 16;
const FIXED_FILE_INFO_SIGNATURE: u32 = 0xFEEF_04BD;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The file version of the PE image `exe`, or `None` if it isn't one or has
/// no version resource.
pub fn file_version(exe: &[u8]) -> Option<Version> {
    if exe.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(exe, 0x3C)? as usize;
    if exe.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let sections = u16_at(exe, coff + 2)? as usize;
    let optional_size = u16_at(exe, coff + 16)? as usize;
    let optional = coff + 20;
    // The data directories follow the optional header's fixed fields, which
    // PE32+ (0x20B) has 16 bytes more of.
    let directories = match u16_at(exe, optional)? {
        0x10B => optional + 96,
        0x20B => optional + 112,
        _ => return None,
    };
    let resources_rva = u32_at(exe, directories + 2 * 8)?;
    if resources_rva == 0 {
        return None;
    }
    let section_table = optional + optional_size;
    let to_offset = |rva: u32| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = section_table + i * 40;
            let virtual_size = u32_at(exe, s + 8)?;
            let virtual_address = u32_at(exe, s + 12)?;
            let raw_size = u32_at(exe, s + 16)?;
            let raw = u32_at(exe, s + 20)?;
            let span = virtual_size.max(raw_size);
            (rva >= virtual_address && rva - virtual_address < span)
                .then(|| (raw + (rva - virtual_address)) as usize)
        })
    };
    let root = to_offset(resources_rva)?;
    // Type, then name, then language: the first entry at each level below
    // the version type.
    let entry = |dir: usize, id: Option<u32>| -> Option<u32> {
        let named = u16_at(exe, dir + 12)? as usize;
        let ids = u16_at(exe, dir + 14)? as usize;
        (0..named + ids).find_map(|i| {
            let e = dir + 16 + i * 8;
            let name = u32_at(exe, e)?;
            (id.is_none() || id == Some(name)).then(|| u32_at(exe, e + 4))?
        })
    };
    let subdir = |to: u32| (to & 0x8000_0000 != 0).then(|| root + (to & 0x7FFF_FFFF) as usize);
    let names = subdir(entry(root, Some(RT_VERSION))?)?;
    let languages = subdir(entry(names, None)?)?;
    let data = entry(languages, None)?;
    if data & 0x8000_0000 != 0 {
        return None;
    }
    let data = root + data as usize;
    let info = to_offset(u32_at(exe, data)?)?;
    let len = u32_at(exe, data + 4)? as usize;
    fixed_file_info(exe.get(info..info + len)?)
}

/// `VS_VERSIONINFO`'s fixed part: after its header (length, value length,
/// type) and the key `VS_VERSION_INFO` in UTF-16, aligned to 32 bits.
fn fixed_file_info(info: &[u8]) -> Option<Version> {
    let key: Vec<u8> = "VS_VERSION_INFO\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    if info.get(6..6 + key.len())? != key.as_slice() {
        return None;
    }
    let fixed = (6 + key.len()).next_multiple_of(4);
    if u32_at(info, fixed)? != FIXED_FILE_INFO_SIGNATURE {
        return None;
    }
    let ms = u32_at(info, fixed + 8)?;
    let ls = u32_at(info, fixed + 12)?;
    Some(Version(
        (ms >> 16) as u16,
        ms as u16,
        (ls >> 16) as u16,
        ls as u16,
    ))
}

/// A minimal PE32 image with a version resource, for the tests: one
/// section, `.rsrc`, holding the resource tree and `VS_VERSIONINFO`.
#[cfg(test)]
pub fn fixture(version: Version) -> Vec<u8> {
    fn put16(b: &mut [u8], at: usize, v: u16) {
        b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn put32(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    const PE: usize = 0x80;
    const OPTIONAL: usize = PE + 24;
    const OPTIONAL_SIZE: usize = 96 + 16 * 8;
    const SECTIONS: usize = OPTIONAL + OPTIONAL_SIZE;
    const RAW: usize = 0x200;
    const RVA: u32 = 0x3000;
    let mut b = vec![0u8; RAW + 0x200];
    b[..2].copy_from_slice(b"MZ");
    put32(&mut b, 0x3C, PE as u32);
    b[PE..PE + 4].copy_from_slice(b"PE\0\0");
    put16(&mut b, PE + 4, 0x14C); // i386
    put16(&mut b, PE + 6, 1);
    put16(&mut b, PE + 20, OPTIONAL_SIZE as u16);
    put16(&mut b, OPTIONAL, 0x10B);
    put32(&mut b, OPTIONAL + 92, 16);
    put32(&mut b, OPTIONAL + 96 + 2 * 8, RVA);
    put32(&mut b, OPTIONAL + 96 + 2 * 8 + 4, 0x200);
    b[SECTIONS..SECTIONS + 5].copy_from_slice(b".rsrc");
    put32(&mut b, SECTIONS + 8, 0x200);
    put32(&mut b, SECTIONS + 12, RVA);
    put32(&mut b, SECTIONS + 16, 0x200);
    put32(&mut b, SECTIONS + 20, RAW as u32);
    // The tree: type 16, name 1, language 0x409, each a directory of one
    // id entry; then the data entry and the data.
    let r = RAW;
    for (dir, id, to) in [
        (0x00, RT_VERSION, 0x8000_0018),
        (0x18, 1, 0x8000_0030),
        (0x30, 0x409, 0x48),
    ] {
        put16(&mut b, r + dir + 14, 1);
        put32(&mut b, r + dir + 16, id);
        put32(&mut b, r + dir + 20, to);
    }
    let info = 0x58;
    put32(&mut b, r + 0x48, RVA + info as u32);
    put32(&mut b, r + 0x4C, 0x5C);
    let v = r + info;
    put16(&mut b, v, 0x5C);
    put16(&mut b, v + 2, 52);
    for (i, unit) in "VS_VERSION_INFO".encode_utf16().enumerate() {
        put16(&mut b, v + 6 + 2 * i, unit);
    }
    put32(&mut b, v + 40, FIXED_FILE_INFO_SIGNATURE);
    put32(&mut b, v + 44, 0x0001_0000);
    put32(&mut b, v + 48, (version.0 as u32) << 16 | version.1 as u32);
    put32(&mut b, v + 52, (version.2 as u32) << 16 | version.3 as u32);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_resource_gives_the_build() {
        let exe = fixture(Version(1, 12, 1, 5875));
        assert_eq!(file_version(&exe), Some(Version(1, 12, 1, 5875)));
        assert_eq!(Version(1, 12, 1, 5875).to_string(), "1.12.1 (5875)");
        assert_eq!(
            file_version(&fixture(Version(2, 4, 3, 8606))),
            Some(Version(2, 4, 3, 8606))
        );
    }

    #[test]
    fn what_isnt_a_pe_image_has_no_version() {
        assert_eq!(file_version(b""), None);
        assert_eq!(file_version(b"MZ"), None);
        assert_eq!(file_version(&[0u8; 4096]), None);
        let mut exe = fixture(Version(1, 12, 1, 5875));
        // No resource directory.
        exe[0x80 + 24 + 96 + 16..0x80 + 24 + 96 + 20].fill(0);
        assert_eq!(file_version(&exe), None);
        let mut cut = fixture(Version(1, 12, 1, 5875));
        cut.truncate(0x260);
        assert_eq!(file_version(&cut), None);
    }
}
