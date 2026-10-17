//! What the setup builds, what of it is already there, and building the
//! rest: the terrain, the game object models, then each map's navmesh.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustigator_mapbuild::BuildError;
use rustigator_mapbuild::progress::{NoProgress, Progress, Stage};

use super::builder::Builder;
use super::discover::Client;
use super::maps::{MAPS, MODELS_BYTES, MODELS_WORK, MapInfo, Selection, TERRAIN_TILE_WORK};

/// Where the setup writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub nav: PathBuf,
    pub terrain: PathBuf,
}

impl Dirs {
    /// The configured directories, or `data/nav` and `data/terrain`.
    pub fn of(world: &crate::config::WorldConfig) -> Self {
        Self {
            nav: world
                .nav_dir
                .clone()
                .unwrap_or_else(|| crate::config::DEFAULT_NAV_DIR.into()),
            terrain: world
                .terrain_dir
                .clone()
                .unwrap_or_else(|| crate::config::DEFAULT_TERRAIN_DIR.into()),
        }
    }
}

/// The setup's note in each directory it builds of the client it builds
/// from: rustigator's builds take a map that is there as current, whichever
/// client made it.
const RECORD: &str = "setup.json";

/// What built a directory, against the client at hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Nothing is there yet.
    Empty,
    /// This client.
    Same,
    /// Another client, as the setup noted it.
    Other(String),
    /// Something the setup didn't build (a bake from elsewhere).
    Unknown,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Record {
    client: String,
    build: String,
    archives: Vec<(String, u64)>,
}

pub fn origin(dir: &Path, client: &Client) -> Origin {
    let Ok(mut entries) = std::fs::read_dir(dir) else {
        return Origin::Empty;
    };
    if entries.next().is_none() {
        return Origin::Empty;
    }
    let record = std::fs::read(dir.join(RECORD))
        .ok()
        .and_then(|b| serde_json::from_slice::<Record>(&b).ok());
    match record {
        None => Origin::Unknown,
        Some(r) if r.archives == client.archives => Origin::Same,
        Some(r) => Origin::Other(format!("World of Warcraft {} at {}", r.build, r.client)),
    }
}

/// The client the setup last built `dirs` from, where it still is: what to
/// offer when it asks again.
pub fn last_client(dirs: &Dirs) -> Option<PathBuf> {
    [&dirs.nav, &dirs.terrain]
        .into_iter()
        .filter_map(|d| std::fs::read(d.join(RECORD)).ok())
        .filter_map(|b| serde_json::from_slice::<Record>(&b).ok())
        .map(|r| PathBuf::from(r.client))
        .find(|p| p.exists())
}

/// Note in `dir` that `client` builds it.
pub fn mark(dir: &Path, client: &Client) -> std::io::Result<()> {
    let record = Record {
        client: client.root.display().to_string(),
        build: client.describe_build(),
        archives: client.archives.clone(),
    };
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        dir.join(RECORD),
        serde_json::to_vec_pretty(&record).map_err(std::io::Error::other)?,
    )
}

/// Move `dir` aside, to `<dir>.old` (or `.old2`, ...), for a fresh build;
/// where it went.
pub fn set_aside(dir: &Path) -> std::io::Result<PathBuf> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| std::io::Error::other("no folder name"))?;
    let to = (1..)
        .map(|n| match n {
            1 => dir.with_file_name(format!("{name}.old")),
            n => dir.with_file_name(format!("{name}.old{n}")),
        })
        .find(|p| !p.exists())
        .expect("a free name");
    std::fs::rename(dir, &to)?;
    Ok(to)
}

/// Whether `dir` holds all of `map`'s terrain tiles (`MMMMRRCC.map`, the
/// map's id first).
pub fn terrain_current(dir: &Path, map: &MapInfo) -> bool {
    let prefix = format!("{:04}", map.id);
    let Ok(read) = std::fs::read_dir(dir) else {
        return false;
    };
    let tiles = read
        .filter_map(Result::ok)
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.len() == 12 && n.starts_with(&prefix) && n.ends_with(".map")
        })
        .count();
    tiles >= map.terrain_tiles as usize
}

/// What a selection still needs: what isn't built yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Todo {
    pub terrain: Vec<&'static MapInfo>,
    pub models: bool,
    pub navmesh: Vec<&'static MapInfo>,
    /// The selection's maps already built, navmesh and terrain.
    pub built: Vec<&'static MapInfo>,
}

impl Todo {
    pub fn of(selection: &Selection, dirs: &Dirs, builder: &dyn Builder) -> Self {
        let terrain: Vec<_> = selection
            .terrain()
            .filter(|m| !terrain_current(&dirs.terrain, m))
            .collect();
        let navmesh: Vec<_> = selection
            .navmeshes()
            .filter(|m| !builder.navmesh_current(&dirs.nav, m.name))
            .collect();
        let models = selection.navmeshes().next().is_some() && !builder.models_current(&dirs.nav);
        let built = selection
            .0
            .iter()
            .copied()
            .filter(|m| !terrain.contains(m) && !navmesh.contains(m))
            .collect();
        Self {
            terrain,
            models,
            navmesh,
            built,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terrain.is_empty() && !self.models && self.navmesh.is_empty()
    }

    /// The bytes it will write: into the navmesh's directory, and the
    /// terrain's.
    pub fn bytes(&self) -> (u64, u64) {
        let models = if self.models { MODELS_BYTES } else { 0 };
        (
            models + self.navmesh.iter().map(|m| m.nav_bytes).sum::<u64>(),
            self.terrain.iter().map(|m| m.terrain_bytes).sum(),
        )
    }

    /// Its stages, in the order they run, each with its share of the work
    /// (for an overall bar), as the estimate table has it.
    pub fn stages(&self) -> Vec<(Stage<'static>, u64)> {
        let mut stages: Vec<(Stage<'static>, u64)> = self
            .terrain
            .iter()
            .map(|m| {
                let work = m.terrain_tiles as u64 * TERRAIN_TILE_WORK;
                (Stage::Terrain(m.name), work)
            })
            .collect();
        if self.models {
            stages.push((Stage::Models, MODELS_WORK));
        }
        stages.extend(
            self.navmesh
                .iter()
                .map(|m| (Stage::Navmesh(m.name), (m.nav_bytes >> 10).max(1))),
        );
        stages
    }
}

/// The free space the build needs, against what there is: the navmesh's
/// directory needs room for both if they share a drive, which this doesn't
/// try to tell.
pub fn check_space(
    dirs: &Dirs,
    todo: &Todo,
    free: impl Fn(&Path) -> Option<u64>,
) -> Result<(), String> {
    let (nav, terrain) = todo.bytes();
    // A margin for the estimate: another client's maps differ a little.
    let need = |b: u64| b + b / 20;
    for (dir, bytes, what) in [
        (&dirs.nav, need(nav + terrain), "the navmesh and terrain"),
        (&dirs.terrain, need(terrain), "the terrain"),
    ] {
        if bytes == 0 {
            continue;
        }
        if let Some(free) = free(dir)
            && free < bytes
        {
            return Err(format!(
                "not enough disk space for {what}: {} needs {}, and its drive has {} free. Free \
                 some space, pick fewer maps, or set [world] nav_dir and terrain_dir in \
                 config.toml to a bigger drive",
                dir.display(),
                super::maps::about(bytes),
                super::maps::size(free)
            ));
        }
    }
    Ok(())
}

/// The progress of the whole setup: rustigator's for each build, and the
/// setup's own word that a stage is done (its build returned, whether or
/// not it said so).
pub trait Watch: Progress {
    fn done(&self, _stage: Stage<'_>) {}
}

impl Watch for NoProgress {}

/// How the build ended, short of finishing.
#[derive(Debug)]
pub enum Stop {
    /// Ctrl-C, between tiles.
    Cancelled,
    /// A build failed: which, and why.
    Failed(String, BuildError),
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stop::Cancelled => write!(f, "stopped"),
            Stop::Failed(what, e) => {
                write!(f, "{what}: {e}")?;
                if matches!(e, BuildError::Parser(_)) {
                    write!(
                        f,
                        ". The client's archive may be damaged: copy its Data again from the \
                         original, or repair it with its launcher"
                    )?;
                }
                Ok(())
            }
        }
    }
}

/// What was built.
#[derive(Debug, Default)]
pub struct Built {
    pub terrain_tiles: u32,
    pub models: u32,
    /// Each map's navmesh and how long it took.
    pub maps: Vec<(&'static str, Duration)>,
    pub elapsed: Duration,
}

/// Build `todo` from the client's `data` into `dirs`.
pub fn build(
    data: &Path,
    dirs: &Dirs,
    todo: &Todo,
    threads: u32,
    builder: &dyn Builder,
    watch: &dyn Watch,
) -> Result<Built, Stop> {
    let started = Instant::now();
    let mut built = Built::default();
    let failed = |what: &str| {
        let what = what.to_owned();
        move |e: BuildError| match e {
            BuildError::Cancelled => Stop::Cancelled,
            e => Stop::Failed(what, e),
        }
    };
    let make_dir = |dir: &Path| {
        std::fs::create_dir_all(dir).map_err(|e| failed(&dir.display().to_string())(e.into()))
    };
    if !todo.terrain.is_empty() {
        make_dir(&dirs.terrain)?;
        let ids: Vec<u32> = todo.terrain.iter().map(|m| m.id).collect();
        built.terrain_tiles = builder
            .terrain(data, &dirs.terrain, &ids, threads, watch)
            .map_err(failed("the terrain"))?;
        for m in &todo.terrain {
            watch.done(Stage::Terrain(m.name));
        }
    }
    if todo.models {
        make_dir(&dirs.nav)?;
        if watch.cancelled() {
            return Err(Stop::Cancelled);
        }
        built.models = builder
            .models(data, &dirs.nav, threads, watch)
            .map_err(failed("the game object models"))?;
        watch.done(Stage::Models);
    }
    for m in &todo.navmesh {
        make_dir(&dirs.nav)?;
        if watch.cancelled() {
            return Err(Stop::Cancelled);
        }
        let t = Instant::now();
        builder
            .navmesh(data, &dirs.nav, m.name, threads, watch)
            .map_err(failed(&format!("{}'s navmesh", m.title)))?;
        built.maps.push((m.name, t.elapsed()));
        watch.done(Stage::Navmesh(m.name));
    }
    built.elapsed = started.elapsed();
    Ok(built)
}

/// The bytes of the files under `dir`.
pub fn dir_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// Whether the server would find a navmesh in `dir`: one current map.
pub fn has_navmesh(dir: &Path, builder: &dyn Builder) -> bool {
    MAPS.iter()
        .filter(|m| m.has_navmesh())
        .any(|m| builder.navmesh_current(dir, m.name))
}

/// Whether `dir` holds any terrain tiles.
pub fn has_terrain(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|r| {
        r.filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().ends_with(".map"))
    })
}

#[cfg(test)]
mod tests {
    use super::super::builder::{Fake, fake_tiles};
    use super::super::discover::tests::Temp;
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Records what the builds say, and cancels after `cancel_after` steps.
    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<String>>,
        steps: AtomicU64,
        cancel_after: Option<u64>,
    }

    fn name(stage: Stage<'_>) -> String {
        match stage {
            Stage::Models => "models".into(),
            Stage::Navmesh(m) => format!("navmesh {m}"),
            Stage::Terrain(m) => format!("terrain {m}"),
        }
    }

    impl Progress for Recorder {
        fn started(&self, stage: Stage<'_>, total: u64) {
            self.events
                .lock()
                .unwrap()
                .push(format!("start {} {total}", name(stage)));
        }
        fn advanced(&self, _: Stage<'_>, n: u64) {
            self.steps.fetch_add(n, Ordering::Relaxed);
        }
        fn cancelled(&self) -> bool {
            self.cancel_after
                .is_some_and(|n| self.steps.load(Ordering::Relaxed) >= n)
        }
    }

    impl Watch for Recorder {
        fn done(&self, stage: Stage<'_>) {
            self.events
                .lock()
                .unwrap()
                .push(format!("done {}", name(stage)));
        }
    }

    fn dirs_in(t: &Temp) -> Dirs {
        Dirs {
            nav: t.0.join("nav"),
            terrain: t.0.join("terrain"),
        }
    }

    fn selection(names: &str) -> Selection {
        Selection::parse(names).unwrap()
    }

    #[test]
    fn a_build_runs_terrain_models_then_each_map_with_their_totals() {
        let t = Temp::new("plan-build");
        let dirs = dirs_in(&t);
        let fake = Fake::default();
        let todo = Todo::of(&selection("Azeroth,StormwindJail,PVPZone02"), &dirs, &fake);
        // A map with no navmesh brings its terrain; one with no ADTs, none.
        let names = |ms: &[&MapInfo]| ms.iter().map(|m| m.name).collect::<Vec<_>>();
        assert_eq!(names(&todo.terrain), ["Azeroth", "PVPZone02"]);
        assert_eq!(names(&todo.navmesh), ["Azeroth", "StormwindJail"]);
        assert!(todo.models);
        let rec = Recorder::default();
        let built = build(Path::new("client"), &dirs, &todo, 4, &fake, &rec).unwrap();
        assert_eq!(built.terrain_tiles, 687 + 30);
        assert_eq!(built.models, 942);
        let maps: Vec<_> = built.maps.iter().map(|(m, _)| *m).collect();
        assert_eq!(maps, ["Azeroth", "StormwindJail"]);
        let azeroth = fake_tiles(MapInfo::by_name("Azeroth").unwrap());
        assert_eq!(
            *rec.events.lock().unwrap(),
            [
                "start terrain Azeroth 687".to_owned(),
                "start terrain PVPZone02 30".into(),
                "done terrain Azeroth".into(),
                "done terrain PVPZone02".into(),
                "start models 942".into(),
                "done models".into(),
                format!("start navmesh Azeroth {azeroth}"),
                "done navmesh Azeroth".into(),
                "start navmesh StormwindJail 1".into(),
                "done navmesh StormwindJail".into(),
            ]
        );
        assert_eq!(
            rec.steps.load(Ordering::Relaxed),
            687 + 30 + 942 + azeroth + 1
        );
        // A second run has nothing left to do.
        let again = Todo::of(&selection("Azeroth,StormwindJail,PVPZone02"), &dirs, &fake);
        assert!(again.is_empty(), "{again:?}");
        assert_eq!(again.built.len(), 3);
        assert!(has_navmesh(&dirs.nav, &fake));
        assert!(has_terrain(&dirs.terrain));
    }

    #[test]
    fn a_cancelled_build_keeps_what_it_finished_and_carries_on_after() {
        let t = Temp::new("plan-cancel");
        let dirs = dirs_in(&t);
        let fake = Fake::default();
        let sel = selection("Shadowfang,Azeroth");
        let todo = Todo::of(&sel, &dirs, &fake);
        let shadowfang = fake_tiles(MapInfo::by_name("Shadowfang").unwrap());
        // Into Azeroth's navmesh: the terrain, models and Shadowfang done.
        let rec = Recorder {
            cancel_after: Some(25 + 687 + 942 + shadowfang + 10),
            ..Default::default()
        };
        let stop = build(Path::new("client"), &dirs, &todo, 1, &fake, &rec).unwrap_err();
        assert!(matches!(stop, Stop::Cancelled), "{stop}");
        assert!(
            !rec.events
                .lock()
                .unwrap()
                .contains(&"done navmesh Azeroth".to_owned())
        );
        let left = Todo::of(&sel, &dirs, &fake);
        assert!(left.terrain.is_empty() && !left.models);
        assert_eq!(left.navmesh, [MapInfo::by_name("Azeroth").unwrap()]);
        let built = build(Path::new("client"), &dirs, &left, 1, &fake, &NoProgress).unwrap();
        assert_eq!(built.maps.len(), 1);
        // Cancelled in the terrain.
        let t = Temp::new("plan-cancel-terrain");
        let dirs = dirs_in(&t);
        let rec = Recorder {
            cancel_after: Some(100),
            ..Default::default()
        };
        let todo = Todo::of(&selection("Azeroth"), &dirs, &fake);
        assert!(matches!(
            build(Path::new("client"), &dirs, &todo, 1, &fake, &rec),
            Err(Stop::Cancelled)
        ));
        assert!(!terrain_current(&dirs.terrain, &MAPS[0]));
    }

    #[test]
    fn a_failed_map_says_which() {
        let t = Temp::new("plan-fail");
        let dirs = dirs_in(&t);
        let fake = Fake {
            fail: Some("Kalimdor"),
            ..Default::default()
        };
        let todo = Todo::of(&selection("Kalimdor"), &dirs, &fake);
        let stop = build(Path::new("client"), &dirs, &todo, 1, &fake, &NoProgress).unwrap_err();
        assert!(
            stop.to_string().starts_with("Kalimdor's navmesh: "),
            "{stop}"
        );
    }

    #[test]
    fn a_directory_knows_the_client_that_built_it() {
        let t = Temp::new("plan-origin");
        let client = |root: &str, patch: u64| Client {
            root: t.0.join(root),
            data: t.0.join(root).join("Data"),
            build: super::super::discover::Build::Exact,
            archives: vec![("dbc.mpq".into(), 3_875_834), ("patch.mpq".into(), patch)],
        };
        let ours = client("WoW", 2_055_856_542);
        let dir = t.0.join("nav");
        assert_eq!(origin(&dir, &ours), Origin::Empty);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(origin(&dir, &ours), Origin::Empty);
        std::fs::write(dir.join("Azeroth.map"), b"a bake from elsewhere").unwrap();
        assert_eq!(origin(&dir, &ours), Origin::Unknown);
        mark(&dir, &ours).unwrap();
        assert_eq!(origin(&dir, &ours), Origin::Same);
        // The same client moved is the same; another patch is another.
        assert_eq!(origin(&dir, &client("Moved", 2_055_856_542)), Origin::Same);
        let Origin::Other(other) = origin(&dir, &client("Other", 1)) else {
            panic!("another client's");
        };
        assert!(
            other.contains("1.12.1 (5875)") && other.ends_with("WoW"),
            "{other}"
        );
        // Set aside for a fresh build, twice.
        assert_eq!(set_aside(&dir).unwrap(), t.0.join("nav.old"));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(set_aside(&dir).unwrap(), t.0.join("nav.old2"));
        assert!(t.0.join("nav.old/Azeroth.map").exists() && !dir.exists());
    }

    #[test]
    fn the_disk_must_hold_what_is_left_to_build() {
        let t = Temp::new("plan-space");
        let dirs = dirs_in(&t);
        let fake = Fake::default();
        let todo = Todo::of(&Selection::everything(), &dirs, &fake);
        let (nav, terrain) = todo.bytes();
        assert!(check_space(&dirs, &todo, |_| Some(20 << 30)).is_ok());
        let err = check_space(&dirs, &todo, |_| Some(nav)).unwrap_err();
        assert!(err.contains("needs about 10 GB"), "{err}");
        assert!(err.contains("pick fewer maps"), "{err}");
        // Unknown free space doesn't stop it.
        assert!(check_space(&dirs, &todo, |_| None).is_ok());
        // Only the terrain left: only its bytes.
        let terrain_only = Todo {
            terrain: todo.terrain.clone(),
            ..Default::default()
        };
        assert!(check_space(&dirs, &terrain_only, |_| Some(terrain * 2)).is_ok());
        // The stages weigh the continents most.
        let stages = todo.stages();
        let weight = |s: Stage<'_>| stages.iter().find(|(x, _)| *x == s).unwrap().1;
        assert!(weight(Stage::Navmesh("Kalimdor")) > weight(Stage::Navmesh("Azeroth")));
        assert!(weight(Stage::Navmesh("Azeroth")) > weight(Stage::Terrain("Azeroth")));
    }
}
