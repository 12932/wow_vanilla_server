//! Phase 7's loader round, spell effects (Thistle Tea's `world/loader/`
//! tests of pulls, knockbacks, drink, critters and wild summons): the real
//! spells, and what they do.

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells;
use crate::game::map::{Map, MapHandle};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// `SMSG_MOVE_KNOCK_BACK`'s horizontal and vertical speeds.
fn launch_of(b: &[u8]) -> (f32, f32) {
    let at = 1 + b[0].count_ones() as usize + 4;
    let f = |i: usize| f32::from_le_bytes(b[at + i * 4..at + i * 4 + 4].try_into().unwrap());
    (f(2), f(3))
}

/// Thistle Tea's pulls and knockbacks (`knockback_dbc_test.exs`), on the
/// real spells: each player pull (28266 to 28621, effect 124) throws its
/// player at the caster at its misc value's speed, a tenth of it a
/// second (30, 30, 40, 20, 30, 40, 50); each knockback (effect 98) at its
/// own two speeds, the misc value's along and the points' up: 10689 10
/// and 10, 11019 12 and 7 to 9, 13009 30 and 4.8 to 5.2, 16716 none along
/// and 50 up.
#[tokio::test]
#[ignore = "extended"]
async fn real_pulls_and_knockbacks_launch_at_their_own_speeds() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, g, map) = player(&realm, "P7EPULL", "Pulla", Race::Human, Class::Warrior).await;
    let foe = on(&map, move |m| {
        ready(m, g, 60, &[]);
        dummy(m, g, 60, 10.0)
    })
    .await;
    let mut pulls = Vec::new();
    for id in [28_266, 28_337, 28_434, 28_618, 28_619, 28_620, 28_621] {
        c.clear();
        on(&map, move |m| spells::cast_triggered(m, foe, id, Some(g))).await;
        let b = c.wait_frame("a pull", 0x0EF, |_| true).await;
        pulls.push((id, launch_of(&b).0));
    }
    assert_eq!(
        pulls,
        vec![
            (28_266, 30.0),
            (28_337, 30.0),
            (28_434, 40.0),
            (28_618, 20.0),
            (28_619, 30.0),
            (28_620, 40.0),
            (28_621, 50.0)
        ]
    );
    for (id, along, up) in [
        (10_689, 10.0, 10.0..=10.0),
        (11_019, 12.0, 7.0..=9.0),
        (13_009, 30.0, 4.8..=5.2),
        (16_716, 0.0, 50.0..=50.0),
    ] {
        c.clear();
        on(&map, move |m| spells::cast_triggered(m, foe, id, Some(g))).await;
        let b = c.wait_frame("a knockback", 0x0EF, |_| true).await;
        let (xy, z) = launch_of(&b);
        assert_eq!(xy, along, "{id}");
        assert!(up.contains(&-z), "{id}: {z}");
    }
}

/// Thistle Tea's drink (`intoxication_dbc_test.exs`): the real drinks'
/// strengths (11007 5, 11008 10, 11009 20, 11629 50, 16712 100), each a
/// 256th of the drunk value; Sober Up (24635, -1000) takes it all.
#[tokio::test]
#[ignore = "extended"]
async fn real_drinks_inebriate_by_their_strength_and_sober_up_sobers() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P7EDRINK", "Drinka", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let mut seen = Vec::new();
        for id in [11_007, 11_008, 11_009, 11_629, 16_712, 24_635] {
            let before = crate::game::map::intoxication::value(m, g);
            spells::cast_triggered(m, g, id, Some(g));
            let after = crate::game::map::intoxication::value(m, g);
            seen.push((id, (i64::from(after) - i64::from(before)) / 256));
        }
        (seen, crate::game::map::intoxication::value(m, g))
    })
    .await;
    assert_eq!(
        out.0,
        vec![
            (11_007, 5),
            (11_008, 10),
            (11_009, 20),
            (11_629, 50),
            (16_712, 100),
            (24_635, -185)
        ]
    );
    assert_eq!(out.1, 0, "sober");
}

/// Thistle Tea's critters and wild summons (`spell_mini_pet_dbc_test.exs`,
/// `spell_wild_dbc_test.exs`): the 100 critter summons (effect 97) each
/// put out their critter by their caster, with no unit chosen; the
/// client's spells have 372 wild summon effects (41), and vmangos's
/// `spell_effect_mod` makes 24 more (the target dummies, Alterac's
/// beacons and the like); a creature's wild summon
/// of a known creature, cast with no target, puts it out by its caster.
#[tokio::test]
#[ignore = "extended"]
async fn critters_and_wild_summons_come_out_by_their_caster() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let mut critters: Vec<(u32, u32)> = d
        .spells
        .spells
        .values()
        .filter(|s| s.effects[0].effect == 97)
        .map(|s| (s.id, s.effects[0].misc_value.max(0) as u32))
        .collect();
    critters.sort_unstable();
    assert_eq!(critters.len(), 100);
    let wild: usize = d
        .spells
        .spells
        .values()
        .map(|s| s.effects.iter().filter(|e| e.effect == 41).count())
        .sum();
    assert_eq!(
        wild,
        372 + 24,
        "the DBC's, and vmangos's spell_effect_mod's"
    );
    let (_c, g, map) = player(&realm, "P7ECRIT", "Critta", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let d = m.realm.data.clone();
        let mut missing = Vec::new();
        for (id, entry) in critters {
            if d.creature_template(entry).is_none() {
                continue;
            }
            // One out already goes first: the same one again would only
            // dismiss it (Murky's two spells).
            m.players.get_mut(&g).unwrap().owned.critter = None;
            spells::cast_triggered(m, g, id, None);
            let out = m.players[&g]
                .owned
                .critter
                .and_then(|c| m.creature(c))
                .map(|c| c.entry);
            if out != Some(entry) {
                missing.push(id);
            }
        }
        // A few of a creature's wild summons (Summon Flesh Eating Worms,
        // Summon Ghost Saber, Summon Vekniss Borer).
        let caster = dummy(m, g, 60, 20.0);
        let mut wild = Vec::new();
        for id in [3428, 5968, 6122] {
            let Some(s) = d.spells.get(id) else {
                continue;
            };
            let Some(e) = s.effects.iter().find(|e| e.effect == 41) else {
                continue;
            };
            let entry = e.misc_value.max(0) as u32;
            let count = |m: &Map| m.creatures.iter().filter(|(_, c)| c.entry == entry).count();
            let before = count(m);
            spells::cast_triggered(m, caster, id, None);
            wild.push((id, count(m) > before));
        }
        (missing, wild)
    })
    .await;
    assert_eq!(out.0, Vec::<u32>::new(), "every critter came out");
    assert!(
        !out.1.is_empty() && out.1.iter().all(|(_, came)| *came),
        "{:?}",
        out.1
    );
}

/// Thistle Tea's insignia (`insignia_spell_test.exs`): the client's Remove
/// Insignia (22027) takes a second, from ten yards, its one effect the
/// insignia's (`SPELL_EFFECT_SKIN_PLAYER_CORPSE`, 116).
#[test]
#[ignore = "extended"]
fn the_real_insignia_spell_is_a_second_from_ten_yards() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let s = d.spells.get(22_027).unwrap();
    assert_eq!(d.spells.cast_times[&s.casting_time_index], 1000);
    assert_eq!(d.spells.ranges[&s.range_index].1, 10.0);
    let live: Vec<u32> = s
        .effects
        .iter()
        .map(|e| e.effect)
        .filter(|e| *e != 0)
        .collect();
    assert_eq!(live, vec![116]);
}
