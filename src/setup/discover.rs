//! Finding the player's WoW client: a folder whose `Data` holds 1.12's
//! archives. Where it was given, then where installers and launchers put it,
//! then (if asked) every folder of every fixed drive.

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

    fn rank(&self) -> u8 {
        match self.build {
            Build::Exact => 0,
            Build::ByArchives { .. } => 1,
            Build::OtherVanilla(_) => 2,
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

impl Verdict {
    fn root(&self) -> &Path {
        match self {
            Verdict::Client(c) => &c.root,
            Verdict::Rejected { root, .. } => root,
        }
    }
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

/// A folder, not a link to one (a search doesn't follow links).
fn is_dir(e: &std::fs::DirEntry) -> bool {
    e.file_type().is_ok_and(|t| t.is_dir())
}

/// A folder, or a link to one: a client's `Data` may be either.
fn leads_to_dir(e: &std::fs::DirEntry) -> bool {
    e.path().is_dir()
}

/// What `dir` is: a client folder (holding `Data`), its `Data` itself, or
/// its `WoW.exe`. `None` if it isn't a WoW folder at all.
pub fn inspect(dir: &Path) -> Option<Verdict> {
    let named = |p: &Path, n: &str| {
        p.file_name()
            .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n))
    };
    if dir.is_file() && named(dir, "wow.exe") {
        return inspect(dir.parent()?);
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
    let exe = entries(&root)
        .into_iter()
        .find(|(n, e)| n == "wow.exe" && !leads_to_dir(e))
        .map(|(_, e)| e.path());
    let build = match exe {
        None => Build::ByArchives { exe: false },
        Some(exe) => match std::fs::read(&exe).ok().and_then(|b| pe::file_version(&b)) {
            None => Build::ByArchives { exe: true },
            Some(v) if v == BUILD => Build::Exact,
            Some(v) if v.0 == 1 => Build::OtherVanilla(v),
            Some(v) => return rejected(format!("WoW.exe is {v}; the server needs {BUILD}")),
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

/// What a search found: the usable clients, the best first, and the WoW
/// folders turned away.
#[derive(Debug, Default)]
pub struct Found {
    pub clients: Vec<Client>,
    pub rejected: Vec<(PathBuf, String)>,
}

impl Found {
    fn add(&mut self, v: Verdict) {
        let key = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let root = key(v.root());
        if self.clients.iter().any(|c| key(&c.root) == root)
            || self.rejected.iter().any(|(r, _)| key(r) == root)
        {
            return;
        }
        match v {
            Verdict::Client(c) => {
                self.clients.push(c);
                self.clients.sort_by_key(Client::rank);
            }
            Verdict::Rejected { root, reason } => self.rejected.push((root, reason)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty() && self.rejected.is_empty()
    }
}

/// A folder a search skips: the system's and package managers' own, which
/// never hold a game, and are large.
fn skipped(path: &Path, name: &str) -> bool {
    const ANYWHERE: [&str; 9] = [
        "$recycle.bin",
        "system volume information",
        "programdata",
        "node_modules",
        ".git",
        "target",
        ".cache",
        ".cargo",
        ".rustup",
    ];
    // Directly under `/`: the system's, not the user's. (`/run/media` holds
    // other drives, and is searched on its own.)
    const UNIX_ROOT: [&str; 17] = [
        "proc", "sys", "dev", "run", "boot", "usr", "lib", "lib32", "lib64", "bin", "sbin", "etc",
        "var", "nix", "snap", "System", "private",
    ];
    if ANYWHERE.contains(&name.to_ascii_lowercase().as_str()) {
        return true;
    }
    if cfg!(unix) && path.parent() == Some(Path::new("/")) && UNIX_ROOT.contains(&name) {
        return true;
    }
    // Windows's own folder, wherever the drive is mounted.
    name.eq_ignore_ascii_case("windows") && path.join("System32").is_dir()
}

/// Search `root` and the folders below it, `depth` levels down (`None`: all
/// the way), into `found`. `visit` is told of each folder searched (and
/// what is found so far), and stops the search by returning `false`; then
/// this returns `false`.
pub fn search(
    root: &Path,
    depth: Option<usize>,
    found: &mut Found,
    visit: &mut dyn FnMut(&Path, &Found) -> bool,
) -> bool {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, level)) = stack.pop() {
        if !visit(&dir, found) {
            return false;
        }
        let entries = entries(&dir);
        if entries.iter().any(|(n, e)| n == "data" && leads_to_dir(e))
            && let Some(v) = inspect(&dir)
        {
            found.add(v);
            // A client's folder holds no other client.
            continue;
        }
        if depth.is_some_and(|d| level >= d) {
            continue;
        }
        let mut below: Vec<PathBuf> = entries
            .iter()
            // Not into links: they loop, and lead off the drive.
            .filter(|(_, e)| is_dir(e))
            .filter(|(_, e)| !skipped(&e.path(), &e.file_name().to_string_lossy()))
            .map(|(_, e)| e.path())
            .collect();
        // Depth first, in name order.
        below.sort_by(|a, b| b.cmp(a));
        stack.extend(below.into_iter().map(|p| (p, level + 1)));
    }
    true
}

/// Where installers, launchers and players put the client, with how deep
/// to look in each, fastest first.
pub fn usual_places() -> Vec<(PathBuf, usize)> {
    let mut places: Vec<(PathBuf, usize)> = super::platform::registry_install_paths()
        .into_iter()
        .map(|p| (p, 0))
        .collect();
    let home = home();
    if cfg!(windows) {
        for drive in super::platform::fixed_drives() {
            for (dir, depth) in [
                ("Games", 2),
                ("Program Files (x86)", 2),
                ("Program Files", 2),
                ("", 1),
            ] {
                places.push((drive.join(dir), depth));
            }
        }
    }
    if let Some(home) = &home {
        for (dir, depth) in [
            ("Desktop", 2),
            ("Downloads", 2),
            ("Documents", 2),
            ("Games", 4),
            ("", 1),
        ] {
            let place = if dir.is_empty() {
                home.clone()
            } else {
                home.join(dir)
            };
            places.push((place, depth));
        }
        if !cfg!(windows) {
            // Wine, Lutris (whose prefixes are under ~/Games), Bottles,
            // Proton's prefixes, CrossOver.
            for (dir, depth) in [
                (".wine/drive_c", 3),
                (".local/share/bottles/bottles", 5),
                (".var/app/com.usebottles.bottles/data/bottles/bottles", 5),
                (".steam/steam/steamapps/compatdata", 6),
                (".local/share/Steam/steamapps/compatdata", 6),
                (
                    ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/compatdata",
                    6,
                ),
                ("Library/Application Support/CrossOver/Bottles", 5),
                ("Applications", 2),
            ] {
                places.push((home.join(dir), depth));
            }
        }
    }
    if cfg!(target_os = "macos") {
        places.push((PathBuf::from("/Applications"), 2));
    } else if cfg!(unix) {
        places.push((PathBuf::from("/opt"), 2));
    }
    places.retain(|(p, _)| p.is_dir());
    places
}

/// Where the full search starts: every fixed drive.
pub fn drives() -> Vec<PathBuf> {
    let mut roots = super::platform::fixed_drives();
    // Other drives on Linux are mounted under the skipped `/run`.
    if cfg!(target_os = "linux") {
        roots.push(PathBuf::from("/run/media"));
    }
    roots.retain(|p| p.is_dir());
    roots
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Search the usual places.
pub fn usual(visit: &mut dyn FnMut(&Path, &Found) -> bool) -> Found {
    let mut found = Found::default();
    for (place, depth) in usual_places() {
        if !search(&place, Some(depth), &mut found, visit) {
            break;
        }
    }
    found
}

/// The client at `dir`, given by the player (`--client` or `WOW_DIR`), or
/// why it isn't one.
pub fn given(dir: &Path) -> Result<Client, String> {
    if !dir.exists() {
        return Err(format!("{} doesn't exist", dir.display()));
    }
    match inspect(dir) {
        Some(Verdict::Client(c)) => Ok(c),
        Some(Verdict::Rejected { root, reason }) => Err(format!("{}: {reason}", root.display())),
        None => Err(format!(
            "{} isn't a WoW client: it has no Data folder holding {}. Give the folder with WoW.exe \
             and Data in it",
            dir.display(),
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
                .contains("doesn't exist")
        );
    }

    #[test]
    fn a_search_finds_clients_below_and_skips_what_it_should() {
        let t = Temp::new("search");
        fake_client(&t.0.join("Games/WoW 1.12"), &VANILLA, Some(BUILD));
        fake_client(&t.0.join("Games/Old/vanilla copy"), &VANILLA, None);
        fake_client(
            &t.0.join("Games/TBC"),
            &["common.MPQ", "expansion.MPQ"],
            None,
        );
        fake_client(&t.0.join("work/node_modules/wow"), &VANILLA, None);
        let mut seen = 0;
        let mut found = Found::default();
        assert!(search(&t.0, None, &mut found, &mut |_, _| {
            seen += 1;
            true
        }));
        let roots: Vec<&Path> = found.clients.iter().map(|c| c.root.as_path()).collect();
        // The exact build first.
        assert_eq!(
            roots,
            [
                t.0.join("Games/WoW 1.12"),
                t.0.join("Games/Old/vanilla copy")
            ]
        );
        assert_eq!(found.rejected.len(), 1);
        assert!(found.rejected[0].1.contains("Burning Crusade"));
        // Not into node_modules, nor below a client's own folder.
        assert_eq!(seen, 7, "the root, work, Games, Old and the three clients");
        // Too shallow to reach the copy two levels down.
        let mut found = Found::default();
        search(&t.0.join("Games"), Some(1), &mut found, &mut |_, _| true);
        assert_eq!(found.clients.len(), 1);
        // Stopped at once.
        let mut found = Found::default();
        assert!(!search(&t.0, None, &mut found, &mut |_, _| false));
        assert!(found.is_empty());
        // The same client found twice is one.
        let mut found = Found::default();
        search(&t.0.join("Games"), Some(2), &mut found, &mut |_, _| true);
        search(
            &t.0.join("Games/WoW 1.12"),
            Some(0),
            &mut found,
            &mut |_, _| true,
        );
        assert_eq!(found.clients.len(), 2);
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
