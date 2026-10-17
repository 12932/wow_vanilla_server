//! The player's WoW client, where they say it is: a folder whose `Data`
//! holds 1.12's archives, named by that folder, its `WoW.exe` (or any `.exe`
//! beside `Data`), the `Data` folder, or a file in it.

use std::path::{Path, PathBuf};

use super::pe::{self, Version};

/// The build the server speaks to.
pub const BUILD: Version = Version(1, 12, 1, 5875);

/// The archives a 1.12 client's `Data` has, the ones the builds read.
pub const ARCHIVES: [&str; 6] = [
    "dbc.mpq",
    "terrain.mpq",
    "model.mpq",
    "texture.mpq",
    "wmo.mpq",
    "patch.mpq",
];

/// Archives that only later clients have, with what they are.
const LATER: [(&str, &str); 6] = [
    ("lichking.mpq", "Wrath of the Lich King (3.x)"),
    ("common-2.mpq", "Wrath of the Lich King (3.x)"),
    ("expansion.mpq", "The Burning Crusade (2.x)"),
    ("common.mpq", "The Burning Crusade (2.x)"),
    ("world.mpq", "Cataclysm or later (4.x)"),
    ("art.mpq", "Cataclysm or later (4.x)"),
];

/// How a client's build is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Build {
    /// `WoW.exe` says 1.12.1 (5875).
    Exact,
    /// `WoW.exe` says another vanilla build.
    OtherVanilla(Version),
    /// By its archives alone: there is no `WoW.exe` (a copy of `Data`), or
    /// its version doesn't read.
    ByArchives { exe: bool },
}

/// A client the builds can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// The folder holding `Data`.
    pub root: PathBuf,
    pub data: PathBuf,
    pub build: Build,
    /// Its archives, by lower-cased name, with their sizes: what tells one
    /// client from another.
    pub archives: Vec<(String, u64)>,
}

impl Client {
    /// "World of Warcraft 1.12.1 (5875) at C:\Games\WoW (5.0 GB)".
    pub fn describe(&self) -> String {
        format!(
            "World of Warcraft {} at {} ({})",
            self.describe_build(),
            self.root.display(),
            super::maps::size(self.bytes())
        )
    }

    /// "1.12.1 (5875)", or "1.12" where only the archives tell.
    pub fn describe_build(&self) -> String {
        match &self.build {
            Build::Exact => BUILD.to_string(),
            Build::OtherVanilla(v) => v.to_string(),
            Build::ByArchives { .. } => "1.12".to_owned(),
        }
    }

    pub fn bytes(&self) -> u64 {
        self.archives.iter().map(|a| a.1).sum()
    }

    /// What to know before using it, if anything.
    pub fn note(&self) -> Option<String> {
        match &self.build {
            Build::Exact => None,
            Build::OtherVanilla(v) => Some(format!(
                "WoW.exe is {v}, not {BUILD}: the server speaks to {BUILD}, and another build's maps \
                 may differ from its world"
            )),
            Build::ByArchives { exe: false } => {
                Some("no WoW.exe beside Data: taken as 1.12 by its archives".to_owned())
            }
            Build::ByArchives { exe: true } => {
                Some("WoW.exe's version doesn't read: taken as 1.12 by its archives".to_owned())
            }
        }
    }
}

/// What a folder turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Client(Client),
    /// A WoW client, but not one the server can use, and why.
    Rejected {
        root: PathBuf,
        reason: String,
    },
}

/// The entries of `dir`, by lower-cased name.
fn entries(dir: &Path) -> Vec<(String, std::fs::DirEntry)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.filter_map(Result::ok)
        .map(|e| (e.file_name().to_string_lossy().to_ascii_lowercase(), e))
        .collect()
}

/// A folder, or a link to one: a client's `Data` may be either.
fn leads_to_dir(e: &std::fs::DirEntry) -> bool {
    e.path().is_dir()
}

/// What `path` is: a client folder (holding `Data`), its `Data` itself, a
/// file in `Data` (an archive), or an `.exe` beside `Data` (its `WoW.exe`,
/// or a renamed one, whose version is read). `None` if it isn't a WoW
/// folder at all.
pub fn inspect(path: &Path) -> Option<Verdict> {
    inspect_with(path, None)
}

fn inspect_with(dir: &Path, exe: Option<PathBuf>) -> Option<Verdict> {
    let named = |p: &Path, n: &str| {
        p.file_name()
            .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n))
    };
    if dir.is_file() {
        let parent = dir.parent()?;
        let is_exe = dir
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
        if is_exe {
            return inspect_with(parent, Some(dir.to_path_buf()));
        }
        // An archive, or anything else, in Data.
        return named(parent, "data")
            .then(|| inspect_with(parent, exe))
            .flatten();
    }
    let (root, data) = match entries(dir)
        .into_iter()
        .find(|(n, e)| n == "data" && leads_to_dir(e))
    {
        Some((_, e)) => (dir.to_path_buf(), e.path()),
        None if named(dir, "data") => (dir.parent()?.to_path_buf(), dir.to_path_buf()),
        None => return None,
    };
    let files = entries(&data);
    let has = |n: &str| files.iter().any(|(f, _)| f == n);
    let rejected = |reason: String| {
        Some(Verdict::Rejected {
            root: root.clone(),
            reason,
        })
    };
    if root.join(".build.info").exists() {
        return rejected(
            "a modern client (its data is CASC, not MPQs); the server needs 1.12.1".to_owned(),
        );
    }
    if let Some((file, what)) = LATER.iter().find(|(f, _)| has(f)) {
        return rejected(format!(
            "it has {}: {what}; the server needs 1.12.1",
            file.replace(".mpq", ".MPQ")
        ));
    }
    let missing: Vec<&str> = ARCHIVES.iter().copied().filter(|a| !has(a)).collect();
    if missing.len() == ARCHIVES.len() {
        return None;
    }
    if !missing.is_empty() {
        let names: Vec<String> = missing.iter().map(|m| m.replace(".mpq", ".MPQ")).collect();
        return rejected(format!(
            "Data has no {}: an incomplete client, or one not patched to 1.12.1",
            names.join(", ")
        ));
    }
    let exe = exe.or_else(|| {
        entries(&root)
            .into_iter()
            .find(|(n, e)| n == "wow.exe" && !leads_to_dir(e))
            .map(|(_, e)| e.path())
    });
    let build = match exe {
        None => Build::ByArchives { exe: false },
        Some(exe) => match std::fs::read(&exe).ok().and_then(|b| pe::file_version(&b)) {
            None => Build::ByArchives { exe: true },
            Some(v) if v == BUILD => Build::Exact,
            Some(v) if v.0 == 1 => Build::OtherVanilla(v),
            Some(v) => {
                let name = exe.file_name().unwrap_or_default().to_string_lossy();
                return rejected(format!("{name} is {v}; the server needs {BUILD}"));
            }
        },
    };
    let mut archives: Vec<(String, u64)> = files
        .iter()
        .filter(|(n, _)| n.ends_with(".mpq"))
        .filter_map(|(n, e)| Some((n.clone(), e.metadata().ok()?.len())))
        .collect();
    archives.sort();
    Some(Verdict::Client(Client {
        root,
        data,
        build,
        archives,
    }))
}

/// A path as the player typed or pasted it: without the quotes (and the
/// `& '...'` PowerShell puts round a file dragged onto it), the spaces
/// round it and a trailing separator, with `~` as their home folder.
pub fn clean(input: &str) -> PathBuf {
    let mut s = input.trim();
    if let Some(rest) = s.strip_prefix("& ") {
        s = rest.trim();
    }
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            s = &s[1..s.len() - 1];
        }
    }
    let s = s.trim();
    let home = || {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
    };
    let path = match s.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => match home() {
            Some(h) => h.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(s),
        },
        _ => PathBuf::from(s),
    };
    let trimmed = path.to_string_lossy();
    let trimmed = trimmed.trim_end_matches(['/', '\\']);
    // Not a root's own separator (`/`, `C:\`).
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path
    } else {
        PathBuf::from(trimmed)
    }
}

/// The client at `path`, as the player named it (`--client`, `WOW_DIR` or
/// the question), or what is wrong with it, said so they can put it right.
pub fn given(path: &Path) -> Result<Client, String> {
    if !path.exists() {
        return Err(format!("there is nothing at {}", path.display()));
    }
    match inspect(path) {
        Some(Verdict::Client(c)) => Ok(c),
        Some(Verdict::Rejected { root, reason }) => Err(format!("{}: {reason}", root.display())),
        None => Err(format!(
            "{} isn't a WoW client: there is no Data folder in it with {}. Give the folder that \
             holds WoW.exe and Data, WoW.exe itself, or the Data folder",
            path.display(),
            ARCHIVES.map(|a| a.replace(".mpq", ".MPQ")).join(", ")
        )),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A temporary folder, removed when dropped.
    pub struct Temp(pub PathBuf);

    impl Temp {
        pub fn new(name: &str) -> Self {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("wvs-setup-{name}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A client folder at `dir` with these archives in `Data` (empty files,
    /// but for their names), and a `WoW.exe` of `exe`'s version.
    pub fn fake_client(dir: &Path, archives: &[&str], exe: Option<Version>) {
        let data = dir.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        for a in archives {
            std::fs::write(data.join(a), b"MPQ\x1a").unwrap();
        }
        if let Some(v) = exe {
            std::fs::write(dir.join("WoW.exe"), pe::fixture(v)).unwrap();
        }
    }

    const VANILLA: [&str; 14] = [
        "backup.MPQ",
        "base.MPQ",
        "dbc.MPQ",
        "fonts.MPQ",
        "interface.MPQ",
        "misc.MPQ",
        "model.MPQ",
        "patch.MPQ",
        "patch-2.MPQ",
        "sound.MPQ",
        "speech.MPQ",
        "terrain.MPQ",
        "texture.MPQ",
        "wmo.MPQ",
    ];

    fn client(v: Option<Verdict>) -> Client {
        match v {
            Some(Verdict::Client(c)) => c,
            other => panic!("not a client: {other:?}"),
        }
    }

    fn reason(v: Option<Verdict>) -> String {
        match v {
            Some(Verdict::Rejected { reason, .. }) => reason,
            other => panic!("not turned away: {other:?}"),
        }
    }

    #[test]
    fn a_1_12_1_client_is_taken_exactly() {
        let t = Temp::new("exact");
        fake_client(&t.0, &VANILLA, Some(BUILD));
        let c = client(inspect(&t.0));
        assert_eq!(c.build, Build::Exact);
        assert_eq!(c.root, t.0);
        assert_eq!(c.data, t.0.join("Data"));
        assert_eq!(c.bytes(), 4 * VANILLA.len() as u64);
        assert_eq!(c.archives[0], ("backup.mpq".to_owned(), 4));
        assert_eq!(c.note(), None);
        assert!(
            c.describe()
                .starts_with("World of Warcraft 1.12.1 (5875) at ")
        );
        // Given its Data, or its WoW.exe, it is the same client.
        assert_eq!(client(inspect(&t.0.join("Data"))), c);
        assert_eq!(client(inspect(&t.0.join("WoW.exe"))), c);
        // A Data that is a link to the archives elsewhere.
        #[cfg(unix)]
        {
            let linked = Temp::new("exact-linked");
            std::os::unix::fs::symlink(t.0.join("Data"), linked.0.join("Data")).unwrap();
            assert_eq!(client(inspect(&linked.0)).archives, c.archives);
        }
    }

    #[test]
    fn a_copy_of_data_alone_is_taken_by_its_archives_with_a_note() {
        let t = Temp::new("data-only");
        // As a copy on Linux might have it: other cases.
        let lower: Vec<String> = VANILLA.iter().map(|a| a.to_lowercase()).collect();
        let lower: Vec<&str> = lower.iter().map(String::as_str).collect();
        fake_client(&t.0, &lower, None);
        let c = client(inspect(&t.0));
        assert_eq!(c.build, Build::ByArchives { exe: false });
        assert!(c.note().unwrap().contains("no WoW.exe"));
        // A WoW.exe that isn't a PE image.
        std::fs::write(t.0.join("WoW.exe"), b"not an exe").unwrap();
        assert_eq!(client(inspect(&t.0)).build, Build::ByArchives { exe: true });
    }

    #[test]
    fn another_vanilla_build_is_offered_with_a_warning() {
        let t = Temp::new("other");
        fake_client(&t.0, &VANILLA, Some(Version(1, 11, 2, 5464)));
        let c = client(inspect(&t.0));
        assert_eq!(c.build, Build::OtherVanilla(Version(1, 11, 2, 5464)));
        assert!(
            c.note()
                .unwrap()
                .contains("1.11.2 (5464), not 1.12.1 (5875)")
        );
    }

    #[test]
    fn later_clients_are_turned_away_with_the_reason() {
        let t = Temp::new("tbc");
        let mut tbc = VANILLA.to_vec();
        tbc.extend(["common.MPQ", "expansion.MPQ"]);
        fake_client(&t.0, &tbc, Some(Version(2, 4, 3, 8606)));
        assert!(reason(inspect(&t.0)).contains("The Burning Crusade"));
        let t = Temp::new("wotlk");
        fake_client(&t.0, &["common.MPQ", "common-2.MPQ", "lichking.MPQ"], None);
        assert!(reason(inspect(&t.0)).contains("Wrath of the Lich King"));
        // A TBC WoW.exe over a vanilla Data.
        let t = Temp::new("tbc-exe");
        fake_client(&t.0, &VANILLA, Some(Version(2, 4, 3, 8606)));
        assert!(reason(inspect(&t.0)).contains("WoW.exe is 2.4.3 (8606)"));
        let t = Temp::new("casc");
        fake_client(&t.0, &[], None);
        std::fs::write(t.0.join(".build.info"), b"").unwrap();
        assert!(reason(inspect(&t.0)).contains("CASC"));
    }

    #[test]
    fn an_unpatched_client_is_turned_away_and_other_folders_are_nothing() {
        let t = Temp::new("unpatched");
        let unpatched: Vec<&str> = VANILLA
            .iter()
            .copied()
            .filter(|a| !a.starts_with("patch"))
            .collect();
        fake_client(&t.0, &unpatched, None);
        assert!(reason(inspect(&t.0)).contains("Data has no patch.MPQ"));
        let t = Temp::new("project");
        std::fs::create_dir_all(t.0.join("data")).unwrap();
        std::fs::write(t.0.join("data/thing.json"), b"{}").unwrap();
        assert_eq!(inspect(&t.0), None);
        assert_eq!(inspect(&t.0.join("nowhere")), None);
        assert!(given(&t.0).unwrap_err().contains("isn't a WoW client"));
        assert!(
            given(&t.0.join("nowhere"))
                .unwrap_err()
                .contains("nothing at")
        );
    }

    #[test]
    fn the_client_is_named_by_its_folder_any_exe_its_data_or_a_file_in_it() {
        let t = Temp::new("named");
        let root = t.0.join("World of Warcraft 1.12");
        fake_client(&root, &VANILLA, Some(BUILD));
        let c = given(&root).unwrap();
        for named in [
            root.join("WoW.exe"),
            root.join("Data"),
            root.join("Data").join("dbc.MPQ"),
        ] {
            assert_eq!(given(&named).unwrap(), c, "{}", named.display());
        }
        // A renamed WoW.exe is read for its version.
        std::fs::rename(root.join("WoW.exe"), root.join("WoW-tweaked.exe")).unwrap();
        let renamed = given(&root.join("WoW-tweaked.exe")).unwrap();
        assert_eq!(renamed.build, Build::Exact);
        assert_eq!(renamed.root, root);
        // Another file beside Data is not the client.
        std::fs::write(root.join("readme.txt"), b"").unwrap();
        assert!(
            given(&root.join("readme.txt"))
                .unwrap_err()
                .contains("isn't a WoW client")
        );
        assert!(
            given(&t.0.join("nowhere"))
                .unwrap_err()
                .contains("nothing at")
        );
    }

    #[test]
    fn a_typed_or_pasted_path_is_cleaned() {
        let p = |s: &str| clean(s).to_string_lossy().into_owned();
        assert_eq!(p("  C:\\Games\\WoW  "), "C:\\Games\\WoW");
        assert_eq!(
            p("\"C:\\Program Files (x86)\\WoW\\\""),
            "C:\\Program Files (x86)\\WoW"
        );
        assert_eq!(
            p("& 'C:\\Games\\World of Warcraft\\WoW.exe'"),
            "C:\\Games\\World of Warcraft\\WoW.exe"
        );
        assert_eq!(p("'/home/me/wow/'"), "/home/me/wow");
        assert_eq!(p("C:\\"), "C:\\");
        assert_eq!(p("/"), "/");
        if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            assert_eq!(clean("~/Games/WoW"), PathBuf::from(home).join("Games/WoW"));
        }
    }

    /// A real client, in `WOW_DIR` where it is set.
    #[test]
    fn a_real_client_is_a_client() {
        let Some(dir) = std::env::var_os("WOW_DIR") else {
            return;
        };
        let c = given(Path::new(&dir)).unwrap();
        assert!(c.bytes() > 4_000_000_000, "{}", c.describe());
    }
}
