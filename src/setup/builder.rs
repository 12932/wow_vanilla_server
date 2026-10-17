//! The builds the setup runs, behind a trait: rustigator's, and a fake that
//! goes through the motions (synthetic tiles, small files) for the tests.

use std::path::Path;
use std::time::Duration;

use rustigator_mapbuild::BuildError;
use rustigator_mapbuild::preflight::State;
use rustigator_mapbuild::progress::{Progress, Stage};

use super::maps::MapInfo;

/// What a preflight of the client found that stops a build: a DBC, or a
/// map's WDT or ADTs, missing or unreadable, with the archive it's in.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Checked {
    pub problems: Vec<String>,
}

pub trait Builder: Sync {
    /// Check that what the builds of `maps` read is in the client.
    fn preflight(&self, data: &Path, maps: &[&str], threads: u32) -> Result<Checked, BuildError>;

    /// The game object models, into `<nav>/models/`; how many.
    fn models(
        &self,
        data: &Path,
        nav: &Path,
        threads: u32,
        progress: &dyn Progress,
    ) -> Result<u32, BuildError>;

    /// A map's navmesh, into `nav`.
    fn navmesh(
        &self,
        data: &Path,
        nav: &Path,
        map: &str,
        threads: u32,
        progress: &dyn Progress,
    ) -> Result<(), BuildError>;

    /// The terrain tiles of `maps` (Map.dbc ids), into `out`; how many.
    fn terrain(
        &self,
        data: &Path,
        out: &Path,
        maps: &[u32],
        threads: u32,
        progress: &dyn Progress,
    ) -> Result<u32, BuildError>;

    /// Whether `nav` holds current models, or a current navmesh of `map`.
    fn models_current(&self, nav: &Path) -> bool;
    fn navmesh_current(&self, nav: &Path, map: &str) -> bool;
}

/// rustigator-mapbuild's builds.
pub struct Rustigator;

impl Builder for Rustigator {
    fn preflight(&self, data: &Path, maps: &[&str], threads: u32) -> Result<Checked, BuildError> {
        let report = rustigator_mapbuild::preflight::preflight(data, maps, threads)?;
        let problem = |r: &rustigator_mapbuild::preflight::Resource| {
            let state = match r.state {
                State::Ok => return None,
                State::Missing => "is missing".to_owned(),
                State::Unreadable => format!(
                    "in {} doesn't read ({})",
                    r.archive.as_deref().unwrap_or("its archive"),
                    r.detail.as_deref().unwrap_or("unknown")
                ),
            };
            Some(format!("{} {state}", r.path))
        };
        let mut problems: Vec<String> = report.dbcs.iter().filter_map(problem).collect();
        for m in &report.maps {
            problems.extend(problem(&m.wdt));
            problems.extend(m.adts_unreadable.iter().filter_map(problem));
            if !m.adts_missing.is_empty() {
                problems.push(format!(
                    "{}: {} of its {} ADTs are missing",
                    m.name,
                    m.adts_missing.len(),
                    m.adts_listed
                ));
            }
        }
        Ok(Checked { problems })
    }

    fn models(
        &self,
        data: &Path,
        nav: &Path,
        threads: u32,
        progress: &dyn Progress,
    ) -> Result<u32, BuildError> {
        rustigator_mapbuild::build_bvh_with(data, nav, threads, progress)
    }

    fn navmesh(
        &self,
        data: &Path,
        nav: &Path,
        map: &str,
        threads: u32,
        progress: &dyn Progress,
    ) -> Result<(), BuildError> {
        rustigator_mapbuild::build_map_with(data, nav, map, Path::new(""), threads, progress)
    }

    fn terrain(
        &self,
        data: &Path,
        out: &Path,
        maps: &[u32],
        threads: u32,
        progress: &dyn Progress,
    ) -> Result<u32, BuildError> {
        rustigator_mapbuild::terrain::extract(data, out, Some(maps), threads, progress)
    }

    fn models_current(&self, nav: &Path) -> bool {
        rustigator_mapbuild::bvh_files_exist(nav).unwrap_or(false)
    }

    fn navmesh_current(&self, nav: &Path, map: &str) -> bool {
        rustigator_mapbuild::map_files_exist(nav, map).unwrap_or(false)
    }
}

/// Builds nothing real: each stage reports its tiles (the estimate table's
/// counts) one by one, `delay` apart, and writes a small file a tile, so
/// what is built is found current afterwards. For the tests, and for trying
/// the prompts and bars (`wvs-setup --fake`).
#[derive(Debug, Default)]
pub struct Fake {
    pub delay: Duration,
    /// The preflight's problems.
    pub problems: Vec<String>,
    /// A map whose navmesh fails.
    pub fail: Option<&'static str>,
}

/// The tiles a fake navmesh has: one for 4.3 MB of the real one, as
/// Azeroth's.
pub fn fake_tiles(m: &MapInfo) -> u64 {
    (m.nav_bytes / 4_300_000).max(1)
}

impl Fake {
    fn steps(
        &self,
        stage: Stage<'_>,
        total: u64,
        progress: &dyn Progress,
    ) -> Result<(), BuildError> {
        progress.started(stage, total);
        for _ in 0..total {
            if progress.cancelled() {
                return Err(BuildError::Cancelled);
            }
            std::thread::sleep(self.delay);
            progress.advanced(stage, 1);
        }
        progress.finished(stage);
        Ok(())
    }
}

impl Builder for Fake {
    fn preflight(&self, _: &Path, _: &[&str], _: u32) -> Result<Checked, BuildError> {
        Ok(Checked {
            problems: self.problems.clone(),
        })
    }

    fn models(
        &self,
        _: &Path,
        nav: &Path,
        _: u32,
        progress: &dyn Progress,
    ) -> Result<u32, BuildError> {
        self.steps(Stage::Models, 942, progress)?;
        std::fs::create_dir_all(nav.join("models"))?;
        std::fs::write(nav.join("models/index.bin"), b"fake")?;
        Ok(942)
    }

    fn navmesh(
        &self,
        _: &Path,
        nav: &Path,
        map: &str,
        _: u32,
        progress: &dyn Progress,
    ) -> Result<(), BuildError> {
        if self.fail == Some(map) {
            return Err(BuildError::Recast(format!("{map} failed, as asked")));
        }
        let info = MapInfo::by_name(map).ok_or_else(|| BuildError::MapIdNotFound(map.into()))?;
        self.steps(Stage::Navmesh(map), fake_tiles(info), progress)?;
        std::fs::create_dir_all(nav)?;
        std::fs::write(nav.join(format!("{map}_32_32.nav")), b"fake")?;
        // The header last, as rustigator writes it.
        std::fs::write(nav.join(format!("{map}.map")), b"fake")?;
        Ok(())
    }

    fn terrain(
        &self,
        _: &Path,
        out: &Path,
        maps: &[u32],
        _: u32,
        progress: &dyn Progress,
    ) -> Result<u32, BuildError> {
        std::fs::create_dir_all(out)?;
        let mut written = 0;
        for &id in maps {
            let info = super::maps::MAPS
                .iter()
                .find(|m| m.id == id)
                .ok_or_else(|| BuildError::MapIdNotFound(id.to_string()))?;
            let stage = Stage::Terrain(info.name);
            progress.started(stage, info.terrain_tiles as u64);
            for t in 0..info.terrain_tiles {
                if progress.cancelled() {
                    return Err(BuildError::Cancelled);
                }
                std::thread::sleep(self.delay);
                let name = format!("{id:04}{:02}{:02}.map", t / 64, t % 64);
                std::fs::write(out.join(name), b"fake")?;
                written += 1;
                progress.advanced(stage, 1);
            }
            progress.finished(stage);
        }
        Ok(written)
    }

    fn models_current(&self, nav: &Path) -> bool {
        nav.join("models/index.bin").exists()
    }

    fn navmesh_current(&self, nav: &Path, map: &str) -> bool {
        nav.join(format!("{map}.map")).exists()
    }
}
