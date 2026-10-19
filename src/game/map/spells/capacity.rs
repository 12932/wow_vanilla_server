//! Aura capacity, as Thistle Tea's `Capacity` and `Priority` (after
//! vmangos's `SpellAuraHolder`) keep it: a unit shows at most 32 buffs and
//! 16 debuffs, and one more of a kind takes the place of the one of least
//! worth (the oldest of those) rather than going on unseen.

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, Holder};

/// Debuffs' worth by their first rank: the lists vmangos keeps.
const RANKED: [(u8, &[u32]); 5] = [
    (
        0,
        &[
            1604, 2094, 3409, 11201, 3600, 4066, 4067, 4069, 4507, 5246, 5530, 6358, 6788, 9672,
            339, 2908, 8056, 8122, 605, 6770, 18202, 18075, 17511, 17504, 17484, 17483, 17407,
            17196, 16406, 14914, 21162, 13752, 12328, 13526, 13318, 11366, 133, 12721, 24388,
            21159, 21151, 12421, 12562, 118, 13237, 13327, 13747, 13808, 14251, 3355, 14308, 14309,
            15235, 15487, 8034, 16454, 17153, 5484, 18093, 18118, 18223, 710, 18798, 19229, 19410,
            19755, 19784, 19872, 19975, 19974, 19973, 19972, 19971, 19970, 20066, 20586, 24375,
            24394,
        ],
    ),
    (
        1,
        &[
            1833, 5917, 7922, 9658, 10578, 2096, 453, 1714, 11103, 11113, 2974, 3043, 16413, 16415,
            17330, 17506, 17639, 18289, 18425, 18469, 18498, 19821, 20184, 20511, 22639, 23023,
            28272,
        ],
    ),
    (
        2,
        &[
            12579, 67, 26017, 26018, 23605, 702, 11374, 8921, 18265, 980, 13797, 1978, 21992, 2818,
            348, 23577, 17315, 16928, 17877, 23415, 9035, 2944, 24640, 12766, 2943, 19249, 19251,
            19252, 19253, 19254, 8050, 172, 16528, 15258, 13003, 589, 12654,
        ],
    ),
    (
        3,
        &[
            355, 603, 676, 1161, 1543, 2855, 4068, 5116, 5209, 5782, 770, 16857, 120, 10, 2120,
            122, 853, 5760, 8692, 11398, 1120, 5740, 689, 5138, 704, 1490, 12323, 11071, 12543,
            12798, 12809, 13218, 13810, 3034, 1510, 13812, 1130, 15286, 15268, 15407, 16922, 17364,
            16914, 17794, 6789, 17862, 18656, 2637, 19185, 10797, 19503, 19675, 19769, 20006,
            21183, 20188, 20300, 20301, 20302, 20303, 20185, 20344, 20345, 20346, 20186, 20354,
            20355, 20549, 20253, 20614, 20615, 20116, 26573, 21152, 22959, 23454, 23694, 24423,
            5570, 116, 25999,
        ],
    ),
    (
        4,
        &[
            6136, 6795, 17331, 27648, 24323, 24322, 28732, 25181, 25177, 25178, 25180, 25183, 7321,
            16597, 20005,
        ],
    ),
];

/// Debuffs' worth by their family's flags.
const FAMILY: [(u32, u64, u8); 9] = [
    (8, 0x8, 1),
    (8, 0x10_0100, 2),
    (4, 0x20, 2),
    (7, 0x80_1000, 2),
    (4, 0xA00_4082, 3),
    (8, 0x228_0000, 3),
    (7, 0x2000, 3),
    (4, 0x2_0000, 4),
    (7, 0x8, 4),
];

/// Control: taunt, threat, charm, fear, confuse, possess, stun.
const CONTROL: [u32; 7] = [11, 10, 6, 7, 5, 2, 12];
/// What a group fights better for: resistances, language, stalking,
/// disarms, damage taken and done, fleeing, crits against, haste, attack
/// power, healing.
const GROUP: [u32; 13] = [22, 75, 68, 67, 87, 92, 179, 138, 9, 99, 13, 14, 118];
const PERIODIC_DAMAGE: [u32; 3] = [3, 89, 53];

/// What a holder on `target` is worth keeping, from 0 (least) to 4.
pub fn worth(data: &GameData, target: Guid, h: &Holder) -> u8 {
    if h.positive {
        return if h.expires.is_none() {
            3
        } else if h.caster != target {
            2
        } else if h.item.is_some() && !h.equipped {
            1
        } else {
            0
        };
    }
    let Some(s) = data.spells.get(h.spell) else {
        return 0;
    };
    if let Some((_, _, p)) = FAMILY
        .iter()
        .find(|(family, mask, _)| s.family_name == *family && s.family_flags & mask != 0)
    {
        return *p;
    }
    let first = data.spells.first_rank(s.id);
    if let Some((p, _)) = RANKED.iter().find(|(_, ids)| ids.contains(&first)) {
        return *p;
    }
    by_effects(s, h)
}

fn by_effects(s: &SpellTemplate, h: &Holder) -> u8 {
    if h.has(auras::AURA_MOD_CHARM) {
        return adjust(4, s, h);
    }
    if h.has(127) || (h.has(auras::AURA_DUMMY) && (s.spell_visual, s.spell_icon) == (3582, 150)) {
        return 3;
    }
    let p = h
        .effects
        .iter()
        .map(|x| {
            if CONTROL.contains(&x.kind) {
                4
            } else if GROUP.contains(&x.kind) {
                3
            } else if PERIODIC_DAMAGE.contains(&x.kind) {
                1
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0);
    adjust(p, s, h)
}

/// A triggered holder is worth at most 2 (if it was worth more) or
/// nothing; a channelled one at least 1.
fn adjust(p: u8, s: &SpellTemplate, h: &Holder) -> u8 {
    if h.triggered {
        if p > 2 { 2 } else { 0 }
    } else if s.attr(1, sd::ATTR_EX_CHANNELED) {
        p.max(1)
    } else {
        p
    }
}

/// The holder of `target`'s that gives way when one more of its kind
/// (`positive`) would show: the least worth, the oldest of those (a
/// refresh makes one new, as Thistle Tea's "refreshing at capacity ...
/// protects the refreshed aura"; those put on at once in their order); it
/// may be the newcomer at `index` itself.
pub fn gives_way(map: &Map, target: Guid, positive: bool, index: usize) -> Option<usize> {
    let a = auras::auras(map, target)?;
    let data = &map.realm.data;
    a.holders
        .iter()
        .enumerate()
        .filter(|(i, h)| h.positive == positive && (h.slot.is_some() || *i == index))
        .min_by_key(|(i, h)| (worth(data, target, h), h.applied, *i, h.spell))
        .map(|(i, _)| i)
}
