//! The maps the setup builds, and what each takes on disk: the one table of
//! estimates, measured on a bake of the 5875 client (rustigator's navmesh,
//! mangos-zero's terrain tiles).

/// A map of the 1.12 client.
#[derive(Debug, PartialEq, Eq)]
pub struct MapInfo {
    /// Its directory in the client (`World\Maps\<name>`), as rustigator and
    /// the bake name it.
    pub name: &'static str,
    /// Its id in `Map.dbc`.
    pub id: u32,
    /// What a player calls it.
    pub title: &'static str,
    /// Its navmesh on disk; 0 for a map with terrain but no navmesh.
    pub nav_bytes: u64,
    /// Its terrain tiles (one an ADT), and their bytes on disk.
    pub terrain_tiles: u32,
    pub terrain_bytes: u64,
}

const fn map(
    name: &'static str,
    id: u32,
    title: &'static str,
    nav_bytes: u64,
    terrain_tiles: u32,
    terrain_bytes: u64,
) -> MapInfo {
    MapInfo {
        name,
        id,
        title,
        nav_bytes,
        terrain_tiles,
        terrain_bytes,
    }
}

/// Every map the setup knows, the continents first, then by id. The 36 with
/// a navmesh are what a full build makes; the two without have terrain only.
pub const MAPS: &[MapInfo] = &[
    map(
        "Azeroth",
        0,
        "Eastern Kingdoms",
        2_958_394_365,
        687,
        72_196_000,
    ),
    map("Kalimdor", 1, "Kalimdor", 4_291_763_411, 1018, 108_263_692),
    map(
        "PVPZone01",
        30,
        "Alterac Valley",
        140_401_648,
        35,
        3_602_824,
    ),
    map(
        "Shadowfang",
        33,
        "Shadowfang Keep",
        98_955_681,
        25,
        3_194_452,
    ),
    map("StormwindJail", 34, "Stormwind Stockade", 2_892_033, 0, 0),
    map(
        "DeadminesInstance",
        36,
        "Deadmines",
        30_189_372,
        36,
        3_673_008,
    ),
    map("PVPZone02", 37, "Azshara Crater", 0, 30, 4_034_468),
    map("WailingCaverns", 43, "Wailing Caverns", 11_143_237, 0, 0),
    map(
        "RazorfenKraulInstance",
        47,
        "Razorfen Kraul",
        44_968_644,
        6,
        796_080,
    ),
    map("Blackfathom", 48, "Blackfathom Deeps", 10_870_447, 0, 0),
    map("Uldaman", 70, "Uldaman", 7_369_985, 0, 0),
    map("GnomeragonInstance", 90, "Gnomeregan", 9_356_353, 0, 0),
    map("SunkenTemple", 109, "Sunken Temple", 5_713_790, 0, 0),
    map(
        "RazorfenDowns",
        129,
        "Razorfen Downs",
        12_934_124,
        24,
        13_920,
    ),
    map("EmeraldDream", 169, "Emerald Dream", 0, 256, 34_093_844),
    map(
        "MonasteryInstances",
        189,
        "Scarlet Monastery",
        15_508_453,
        36,
        20_880,
    ),
    map(
        "TanarisInstance",
        209,
        "Zul'Farrak",
        112_412_014,
        21,
        2_787_064,
    ),
    map("BlackRockSpire", 229, "Blackrock Spire", 12_423_688, 0, 0),
    map("BlackrockDepths", 230, "Blackrock Depths", 23_374_763, 0, 0),
    map("OnyxiaLairInstance", 249, "Onyxia's Lair", 3_194_636, 0, 0),
    map(
        "CavernsOfTime",
        269,
        "Caverns of Time",
        271_851_036,
        39,
        5_426_764,
    ),
    map(
        "SchoolofNecromancy",
        289,
        "Scholomance",
        111_473_603,
        16,
        2_126_016,
    ),
    map("Zul'gurub", 309, "Zul'Gurub", 235_578_812, 25, 3_397_988),
    map("Stratholme", 329, "Stratholme", 131_999_157, 20, 2_653_600),
    map("Mauradon", 349, "Maraudon", 17_867_767, 0, 0),
    map("DeeprunTram", 369, "Deeprun Tram", 15_115_206, 0, 0),
    map("OrgrimmarInstance", 389, "Ragefire Chasm", 5_640_281, 0, 0),
    map("MoltenCore", 409, "Molten Core", 11_881_583, 0, 0),
    map("DireMaul", 429, "Dire Maul", 28_967_975, 0, 0),
    map(
        "AlliancePVPBarracks",
        449,
        "Alliance PvP Barracks",
        897_996,
        0,
        0,
    ),
    map(
        "HordePVPBarracks",
        450,
        "Horde PvP Barracks",
        3_805_299,
        0,
        0,
    ),
    map(
        "development",
        451,
        "Development Land",
        62_280_318,
        18,
        2_388_752,
    ),
    map(
        "BlackwingLair",
        469,
        "Blackwing Lair",
        148_238_760,
        16,
        2_122_880,
    ),
    map("PVPZone03", 489, "Warsong Gulch", 42_515_936, 16, 1_597_552),
    map(
        "AhnQiraj",
        509,
        "Ruins of Ahn'Qiraj",
        250_371_751,
        40,
        3_863_108,
    ),
    map("PVPZone04", 529, "Arathi Basin", 57_692_060, 16, 2_204_224),
    map(
        "AhnQirajTemple",
        531,
        "Ahn'Qiraj Temple",
        52_592_879,
        25,
        3_317_000,
    ),
    map(
        "Stratholme Raid",
        533,
        "Naxxramas",
        228_869_958,
        24,
        3_184_320,
    ),
];

/// The game object models (`<nav>/models/`), built with any navmesh.
pub const MODELS_BYTES: u64 = 15_746_765;

impl MapInfo {
    pub fn has_navmesh(&self) -> bool {
        self.nav_bytes > 0
    }

    pub fn by_name(name: &str) -> Option<&'static MapInfo> {
        MAPS.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

/// What to build: some maps' navmesh and terrain. A map without a navmesh
/// brings its terrain only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection(pub Vec<&'static MapInfo>);

impl Selection {
    /// Every map: the 36 navmeshes and all the terrain.
    pub fn everything() -> Self {
        Self(MAPS.iter().collect())
    }

    /// The Eastern Kingdoms (map 0): Gurubashi Arena, Stormwind, the
    /// Undercity.
    pub fn eastern_kingdoms() -> Self {
        Self(vec![&MAPS[0]])
    }

    /// `all`, `azeroth`, or map names separated by commas (as in the client,
    /// in any case).
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" | "everything" => return Ok(Self::everything()),
            "azeroth" | "eastern kingdoms" => return Ok(Self::eastern_kingdoms()),
            _ => {}
        }
        let mut maps = Vec::new();
        for name in s.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let m = MapInfo::by_name(name).ok_or_else(|| {
                format!(
                    "no map called {name:?}; the maps are: {}",
                    MAPS.iter().map(|m| m.name).collect::<Vec<_>>().join(", ")
                )
            })?;
            if !maps.contains(&m) {
                maps.push(m);
            }
        }
        if maps.is_empty() {
            return Err("no maps given".into());
        }
        Ok(Self(maps))
    }

    pub fn navmeshes(&self) -> impl Iterator<Item = &'static MapInfo> + '_ {
        self.0.iter().copied().filter(|m| m.has_navmesh())
    }

    pub fn terrain(&self) -> impl Iterator<Item = &'static MapInfo> + '_ {
        self.0.iter().copied().filter(|m| m.terrain_tiles > 0)
    }

    /// Bytes on disk: the navmesh (with the models) and the terrain.
    pub fn estimate(&self) -> (u64, u64) {
        let nav: u64 = self.navmeshes().map(|m| m.nav_bytes).sum();
        let models = if nav > 0 { MODELS_BYTES } else { 0 };
        (nav + models, self.terrain().map(|m| m.terrain_bytes).sum())
    }
}

/// `bytes` roughly, for an estimate: "about 9 GB", "about 260 MB".
pub fn about(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    const MB: f64 = (1u64 << 20) as f64;
    let b = bytes as f64;
    if b >= GB {
        format!("about {:.0} GB", b / GB)
    } else if b >= 100.0 * MB {
        format!("about {:.0} MB", (b / MB / 10.0).round() * 10.0)
    } else {
        format!("about {:.0} MB", (b / MB).ceil())
    }
}

/// `bytes` as measured: "2.8 GB", "258 MB".
pub fn size(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    const MB: f64 = (1u64 << 20) as f64;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", (b / 1024.0).ceil())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_build_is_the_36_navmeshes_and_2429_terrain_tiles() {
        let all = Selection::everything();
        assert_eq!(all.navmeshes().count(), 36);
        assert_eq!(all.terrain().map(|m| m.terrain_tiles).sum::<u32>(), 2429);
        let (nav, terrain) = all.estimate();
        assert_eq!(about(nav + terrain), "about 9 GB");
        assert_eq!(about(terrain), "about 250 MB");
        let (nav, terrain) = Selection::eastern_kingdoms().estimate();
        assert_eq!(about(nav + terrain), "about 3 GB");
    }

    #[test]
    fn maps_are_named_as_the_client_names_them() {
        let s = Selection::parse("azeroth").unwrap();
        assert_eq!(s, Selection::eastern_kingdoms());
        let s = Selection::parse("kalimdor, PVPZone03,Stratholme Raid,kalimdor").unwrap();
        let ids: Vec<u32> = s.0.iter().map(|m| m.id).collect();
        assert_eq!(ids, [1, 489, 533]);
        assert_eq!(Selection::parse("ALL").unwrap(), Selection::everything());
        assert!(
            Selection::parse("Outland")
                .unwrap_err()
                .contains("no map called")
        );
        assert!(Selection::parse(" , ").is_err());
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(size(2_958_394_365), "2.8 GB");
        assert_eq!(size(270_000_000), "257 MB");
        assert_eq!(about(MODELS_BYTES), "about 16 MB");
    }
}
