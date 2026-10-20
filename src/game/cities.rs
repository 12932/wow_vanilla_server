//! The six capitals' defence (the user's, 2026-10-19 and 2026-10-20), as
//! data: each city's Elite Guard, made in code ([`crate::game::map::elites`]),
//! and its champion, who comes as its alarm reaches the top
//! ([`crate::game::champions`]): Lady Jaina Proudmoore for Stormwind,
//! Ironforge and Darnassus, Rexxar for Orgrimmar, the Undercity and Thunder
//! Bluff. Which of them are defended is `[guards] jaina_cities` and
//! `rexxar_cities`; a city in neither has its guards, its elite and its
//! alarm (`map/guards.rs`), but no Elite Guards and no champion.

use wow_world_messages::vanilla::Vector3d;

use crate::game::champions::Champion;

/// A city's Elite Guard: the Tower Point Warmaster's fight on the city's
/// guard (`looks`: its model, side and look), with weapons of its own.
#[derive(Debug)]
pub struct Elite {
    /// Its entry: past vmangos's last (18199), and named by none of its
    /// tables.
    pub entry: u32,
    pub name: &'static str,
    /// Whose looks and side it has.
    pub looks: u32,
    /// Its main hand, off hand and ranged slot: its own equipment's.
    pub weapons: [u32; 3],
}

/// Where Jaina's Teleport sends its victim in a city, in place of her
/// tower's top in Theramore.
#[derive(Debug)]
pub enum Prison {
    /// Where an area trigger's teleport puts a player out (the data's),
    /// else this spot.
    Trigger(u32, Vector3d, f32),
    /// This spot.
    Spot(Vector3d, f32),
}

/// A capital.
#[derive(Debug)]
pub struct City {
    /// Its area, the town `map/guards.rs` knows it by.
    pub area: u32,
    /// Its name in its champion's words.
    pub name: &'static str,
    pub map: u32,
    pub champion: Champion,
    pub elite: Elite,
    /// Jaina's cities: her Teleport's place there.
    pub prison: Option<Prison>,
}

/// The Stockade's door in Stormwind: area trigger 503 (the Stockade's
/// exit), as vmangos has it.
pub const STOCKADE_EXIT: u32 = 503;
pub const STOCKADE_DOOR: (Vector3d, f32) = (
    Vector3d {
        x: -8766.11,
        y: 845.499,
        z: 87.9952,
    },
    3.83972,
);
/// The Mystic Ward in Ironforge, the mages' and priests' quarter (the
/// user's "Mystic Cavern"): on its upper floor's east side, seven yards
/// past Nittlebur Sparkfizzle, out of the way of the trainers below,
/// facing the ward.
pub const MYSTIC_WARD: (Vector3d, f32) = (
    Vector3d {
        x: -4598.0,
        y: -925.0,
        z: 524.7,
    },
    2.45,
);
/// The Temple of the Moon in Darnassus: its lower hall, by its west wall,
/// past the priest trainers.
pub const TEMPLE_OF_THE_MOON: (Vector3d, f32) = (
    Vector3d {
        x: 9625.0,
        y: 2525.0,
        z: 1331.6,
    },
    0.0,
);

pub const STORMWIND: u32 = 1519;
pub const IRONFORGE: u32 = 1537;
pub const DARNASSUS: u32 = 1657;
pub const ORGRIMMAR: u32 = 1637;
pub const UNDERCITY: u32 = 1497;
pub const THUNDER_BLUFF: u32 = 1638;

/// The capitals. Their Elite Guards (the Stormwind one first, as it was):
///
/// - Stormwind's: a Stormwind Royal Guard's look, a one-handed sword in
///   each hand (the user's: the Alliance's PvP blade, the Grand Marshal's
///   Swiftblade).
/// - Ironforge's: an Ironforge Guard's (dwarves, the city's guard), with
///   its own one-handed double axe and a dwarven hammer (the Stormhammer's
///   look) in its off hand.
/// - Darnassus's: a Darnassus Sentinel's, with the Sentinel's three-bladed
///   glaive in each hand.
/// - Orgrimmar's: a Kor'kron Elite's (the Kor'kron-style grunt the user
///   asked for), with the Kor'kron's two-handed axe.
/// - The Undercity's: a Deathguard Elite's (the user's Deathguard; the
///   Undercity Guardian has no weapons), with the Deathguard's double axe
///   in each hand.
/// - Thunder Bluff's: a Bluffwatcher's, with the Bluffwatcher's two-handed
///   Mulgore Protector axe.
pub const CITIES: &[City] = &[
    City {
        area: STORMWIND,
        name: "Stormwind",
        map: 0,
        champion: Champion::Jaina,
        elite: Elite {
            entry: 101_756,
            name: "Stormwind Elite Guard",
            looks: 1756,
            weapons: [21573, 23456, 0],
        },
        prison: Some(Prison::Trigger(
            STOCKADE_EXIT,
            STOCKADE_DOOR.0,
            STOCKADE_DOOR.1,
        )),
    },
    City {
        area: IRONFORGE,
        name: "Ironforge",
        map: 0,
        champion: Champion::Jaina,
        elite: Elite {
            entry: 101_757,
            name: "Ironforge Elite Guard",
            looks: 5595,
            weapons: [5286, 10591, 0],
        },
        prison: Some(Prison::Spot(MYSTIC_WARD.0, MYSTIC_WARD.1)),
    },
    City {
        area: DARNASSUS,
        name: "Darnassus",
        map: 1,
        champion: Champion::Jaina,
        elite: Elite {
            entry: 101_758,
            name: "Darnassus Elite Sentinel",
            looks: 4262,
            weapons: [5598, 5598, 0],
        },
        prison: Some(Prison::Spot(TEMPLE_OF_THE_MOON.0, TEMPLE_OF_THE_MOON.1)),
    },
    City {
        area: ORGRIMMAR,
        name: "Orgrimmar",
        map: 1,
        champion: Champion::Rexxar,
        elite: Elite {
            entry: 101_759,
            name: "Orgrimmar Elite Grunt",
            looks: 14304,
            weapons: [5289, 0, 0],
        },
        prison: None,
    },
    City {
        area: UNDERCITY,
        name: "Undercity",
        map: 0,
        champion: Champion::Rexxar,
        elite: Elite {
            entry: 101_760,
            name: "Undercity Elite Deathguard",
            looks: 7980,
            weapons: [5286, 5286, 0],
        },
        prison: None,
    },
    City {
        area: THUNDER_BLUFF,
        name: "Thunder Bluff",
        map: 1,
        champion: Champion::Rexxar,
        elite: Elite {
            entry: 101_761,
            name: "Thunder Bluff Elite Bluffwatcher",
            looks: 3084,
            weapons: [12754, 0, 0],
        },
        prison: None,
    },
];

/// Cities the navmesh's areas don't know, by the box they are in: map,
/// area, and x, y and z from and to. Ironforge is inside its mountain, a
/// world model whose areas the bake hasn't: it reads as Dun Morogh's
/// there (and its gate as the Gates of Ironforge, 809, outside it).
const INDOORS: &[(u32, u32, [f32; 6])] = &[(
    0,
    IRONFORGE,
    [-5050.0, -4560.0, -1500.0, -760.0, 480.0, 540.0],
)];

/// The zone and area at `at` on `map`: the navmesh's (or the terrain's),
/// but inside a city it doesn't know ([`INDOORS`]), the city's.
pub fn zone_and_area(map: &mut crate::game::map::Map, at: Vector3d) -> Option<(u32, u32)> {
    let indoors = INDOORS.iter().find(|(m, _, b)| {
        *m == map.key.map
            && (b[0]..=b[1]).contains(&at.x)
            && (b[2]..=b[3]).contains(&at.y)
            && (b[4]..=b[5]).contains(&at.z)
    });
    match indoors {
        Some((_, area, _)) => Some((*area, *area)),
        None => crate::game::map::world::zone_and_area(map, at),
    }
}

/// The capital of `area`.
pub fn city(area: u32) -> Option<&'static City> {
    CITIES.iter().find(|c| c.area == area)
}

/// The capital whose Elite Guard `entry` is.
pub fn of_elite(entry: u32) -> Option<&'static City> {
    CITIES.iter().find(|c| c.elite.entry == entry)
}

/// Whether `entry` is an Elite Guard of a capital's.
pub fn is_elite(entry: u32) -> bool {
    of_elite(entry).is_some()
}

/// The capital of `area`, if its champion's list in `cfg` has it: a
/// defended city, whose alarm brings its Elite Guards and its champion.
pub fn defended(cfg: &crate::config::GuardsConfig, area: u32) -> Option<&'static City> {
    city(area).filter(|c| c.champion.settings(cfg).cities.contains(&area))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_capitals_each_once_three_a_side() {
        let mut areas: Vec<u32> = CITIES.iter().map(|c| c.area).collect();
        areas.sort_unstable();
        areas.dedup();
        assert_eq!(areas.len(), 6);
        let mut entries: Vec<u32> = CITIES.iter().map(|c| c.elite.entry).collect();
        entries.sort_unstable();
        entries.dedup();
        assert_eq!(entries.len(), 6);
        for c in CITIES {
            assert_eq!(
                c.prison.is_some(),
                c.champion == Champion::Jaina,
                "{}: Jaina's cities have her Teleport's place",
                c.name
            );
            assert!(
                crate::game::map::guards::guards_of(c.area).is_some(),
                "{}: its guards come per victim",
                c.name
            );
        }
        let cfg = crate::config::GuardsConfig::default();
        assert!(CITIES.iter().all(|c| defended(&cfg, c.area).is_some()));
        assert!(defended(&cfg, 87).is_none(), "Goldshire isn't a capital");
    }
}
