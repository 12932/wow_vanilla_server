//! The town guard posts civilians call on, ported from vmangos's
//! `GuardMgr` as Thistle Tea's `GuardPost` has it: each listed area names
//! the guard it sends for each side. A post holds ten charges, spends one a
//! call, refuses calls for ten seconds after each, and gains a charge back
//! each minute.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use ahash::AHashMap;

pub const MAX_CHARGES: u32 = 10;
pub const COOLDOWN: Duration = Duration::from_secs(10);
pub const RECHARGE: Duration = Duration::from_secs(60);
/// How long a summoned guard stays.
pub const GUARD_LIFETIME: Duration = Duration::from_secs(120);
/// Razor Hill's civilians call the grunts.
const RAZOR_HILL: u32 = 362;
const ORC_GRUNTS_TEXT: u32 = 4558;

/// Each posted area's guard for the Alliance and for the Horde.
const GUARDS: &[(u32, u32, u32)] = &[
    (1519, 68, 0),
    (415, 6087, 0),
    (442, 6086, 0),
    (380, 0, 3501),
    (362, 0, 5953),
    (513, 4979, 0),
    (1116, 7939, 0),
    (1638, 0, 3084),
    (221, 0, 7975),
    (222, 0, 7975),
    (215, 0, 7975),
    (1099, 0, 8147),
    (484, 0, 9525),
    (367, 0, 8017),
    (108, 8096, 0),
    (144, 8055, 0),
    (130, 0, 7489),
    (1497, 0, 7980),
    (131, 727, 0),
    (1537, 5595, 0),
    (1657, 4262, 0),
    (1637, 0, 3296),
    (11, 1475, 0),
    (15, 4979, 10036),
    (496, 0, 10036),
    (45, 10696, 2621),
    (320, 10696, 0),
    (159, 0, 7980),
    (85, 0, 7980),
    (154, 0, 7980),
    (267, 2386, 2405),
    (271, 2386, 0),
    (272, 0, 2405),
    (406, 0, 7730),
    (117, 0, 1064),
    (87, 68, 0),
    (75, 0, 866),
    (340, 0, 8155),
    (69, 10037, 0),
    (42, 10038, 0),
    (186, 3571, 0),
    (141, 3571, 0),
    (702, 4262, 0),
    (2361, 11822, 11822),
    (3317, 0, 14730),
    (2268, 16378, 16378),
    (188, 12160, 0),
    (9, 1642, 0),
    (132, 853, 0),
    (363, 0, 5952),
    (35, 4624, 4624),
    (2255, 11190, 11190),
    (976, 9460, 9460),
    (392, 3502, 3502),
    (228, 0, 15138),
    (150, 15137, 0),
    (321, 0, 15136),
];

/// The lines a civilian of a faction shouts: the human's, orc's, dwarf's,
/// gnome's, undead's, night elf's, tauren's and troll's.
const FACTION_TEXTS: &[(u32, &[u32])] = &[
    (
        4403,
        &[
            11, 12, 123, 1078, 1575, 53, 56, 84, 210, 534, 1315, 149, 150, 151, 894, 1075, 1077,
            1096, 1577, 371, 1576,
        ],
    ),
    (
        4561,
        &[
            29, 65, 85, 125, 1074, 1174, 1595, 1612, 1619, 83, 106, 714, 1034, 1314, 1215, 1515,
        ],
    ),
    (4583, &[55, 57, 122, 1611, 1618, 694, 1054, 1055, 1217]),
    (8546, &[23, 64, 875]),
    (4484, &[68, 71, 98, 118, 1134, 1154, 412]),
    (4564, &[79, 80, 124, 1076, 1097, 1594, 1600, 1514]),
    (4560, &[104, 105, 995]),
    (4559, &[126, 876, 877]),
];

/// Whether an area has a post.
pub fn posted(area: u32) -> bool {
    GUARDS.iter().any(|(a, _, _)| *a == area)
}

/// The guard a post sends against an enemy of the other side: the
/// Alliance's guard against the Horde, and the Horde's against the
/// Alliance.
pub fn guard(area: u32, alliance: bool) -> Option<u32> {
    GUARDS
        .iter()
        .find(|(a, _, _)| *a == area)
        .map(|(_, a, h)| if alliance { *a } else { *h })
        .filter(|e| *e != 0)
}

/// The line a civilian of `faction` shouts in `area`.
pub fn text(area: u32, faction: u32) -> Option<u32> {
    if area == RAZOR_HILL {
        return Some(ORC_GRUNTS_TEXT);
    }
    FACTION_TEXTS
        .iter()
        .find(|(_, factions)| factions.contains(&faction))
        .map(|(t, _)| *t)
}

/// A post's charges.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Post {
    pub(crate) charges: u32,
    pub(crate) cooldown_until: Option<Instant>,
    pub(crate) recharged_at: Instant,
}

/// Guard posts by map, copy and area.
type Posts = Mutex<AHashMap<(u32, u32, u32), Post>>;

/// The posts, by map, copy and area, shared by every civilian of a town.
fn posts() -> &'static Posts {
    static POSTS: OnceLock<Posts> = OnceLock::new();
    POSTS.get_or_init(|| Mutex::new(AHashMap::new()))
}

/// A call on the post: whether it answers (a charge spent).
pub fn take(key: (u32, u32, u32), now: Instant) -> bool {
    let Ok(mut posts) = posts().lock() else {
        return false;
    };
    let post = posts.entry(key).or_insert(Post {
        charges: MAX_CHARGES,
        cooldown_until: None,
        recharged_at: now,
    });
    take_from(post, now)
}

pub(crate) fn take_from(post: &mut Post, now: Instant) -> bool {
    // Charges come back by the minute, counted from the request.
    let gained =
        (now.saturating_duration_since(post.recharged_at).as_secs() / RECHARGE.as_secs()) as u32;
    if gained > 0 || post.charges >= MAX_CHARGES {
        post.charges = (post.charges + gained).min(MAX_CHARGES);
        post.recharged_at = if post.charges == MAX_CHARGES {
            now
        } else {
            post.recharged_at + RECHARGE * gained
        };
    }
    if post.charges == 0 || post.cooldown_until.is_some_and(|t| now < t) {
        return false;
    }
    post.charges -= 1;
    post.cooldown_until = Some(now + COOLDOWN);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_post_spends_charges_waits_between_calls_and_recharges() {
        let t0 = Instant::now();
        assert!(take((999, 0, 1519), t0));
        assert!(
            !take((999, 0, 1519), t0 + Duration::from_secs(5)),
            "cooling down"
        );
        // A spent post answers again once a minute has given a charge back.
        let mut post = Post {
            charges: 0,
            cooldown_until: None,
            recharged_at: t0,
        };
        assert!(!take_from(&mut post, t0 + Duration::from_secs(30)));
        assert!(take_from(&mut post, t0 + Duration::from_secs(61)));
        assert!(
            !take_from(&mut post, t0 + Duration::from_secs(75)),
            "spent again"
        );
        assert_eq!(guard(1519, true), Some(68));
        assert_eq!(guard(1519, false), None);
        assert_eq!(text(362, 11), Some(4558));
        assert_eq!(text(1, 11), Some(4403));
    }
}
