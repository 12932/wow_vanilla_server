//! wvs's own `.tele` places, over vmangos's `game_tele`: places a raid
//! tester wants that the table hasn't. They are added after the table's
//! rows, and a row of the same name (in any case) keeps its place.

use super::Tele;

/// A place of wvs's own: its name, map, position and facing.
struct Extra {
    name: &'static str,
    map: u32,
    at: (f32, f32, f32),
    orientation: f32,
}

/// wvs's places (not vmangos's).
const EXTRA_TELE: &[Extra] = &[
    // Inside Frostwolf Keep, just outside Drek'Thar's room, between its two
    // ways in, facing him (his spawn, -1370.26, -219.70, 98.51): the
    // user's spot, where peon's Alterac Valley raid starts. On the navmesh,
    // 41 yd from him by path; none of his room sees it.
    Extra {
        name: "AlteracValleyFrostwolfKeep",
        map: 30,
        at: (-1361.41, -245.86, 99.36),
        orientation: 1.90,
    },
    // Inside Dun Baldar, in the hall on the way in from the keep's
    // entrance, just before the turn into Vanndar Stormpike's room, facing
    // him (his spawn, 722.43, -10.99, 50.70): 36 yd from him by path, and
    // none of his room (his marshals, his guardsmen) or the hall sees it.
    Extra {
        name: "AlteracValleyDunBaldar",
        map: 30,
        at: (690.4, -8.0, 50.62),
        orientation: 6.19,
    },
];

/// `tele` with wvs's places after it, each one the table hasn't by name.
pub fn add_extras(tele: &mut Vec<Tele>) {
    for e in EXTRA_TELE {
        if tele.iter().any(|t| t.name.eq_ignore_ascii_case(e.name)) {
            continue;
        }
        tele.push(Tele {
            name: e.name.to_owned(),
            map: e.map,
            x: e.at.0,
            y: e.at.1,
            z: e.at.2,
            orientation: e.orientation,
        });
    }
}
