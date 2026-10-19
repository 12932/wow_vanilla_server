//! Phase 3's fourth round of the guardians, ported from Thistle Tea's
//! `test/game/world/entity/guardian_owner_test.exs` (docs/parity/matrix.csv):
//! how a player's guardians come, add up and go, a creature's, and a
//! player's wild summons.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellDuration, SpellEffect};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_combat::{BRUTE, HUMANS, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::spell;
use crate::game::tests_p4_world::on;

const DRAGONLING: u32 = 98_200;
const SQUIRE: u32 = 98_201;
/// Two dragonlings, at once.
const TWO_DRAGONLINGS: u32 = 98_210;
const ONE_SQUIRE: u32 = 98_211;
/// A dragonling for a time, of a category: a recast replaces it.
const KEPT_DRAGONLING: u32 = 98_212;
/// A dragonling for a fifth of a second.
const BRIEF_DRAGONLING: u32 = 98_213;

fn summoning(id: u32, entry: u32, count: i32) -> crate::game::data::spells::SpellTemplate {
    let mut s = spell(
        id,
        &[SpellEffect {
            effect: crate::game::map::pets::EFFECT_SUMMON_GUARDIAN,
            misc_value: entry as i32,
            base_points: count - 1,
            base_dice: 1,
            target_a: 32,
            ..Default::default()
        }],
    );
    s.duration_index = 4;
    s
}

fn world() -> GameData {
    let mut d = data();
    d.spells.durations.insert(
        4,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    d.spells.durations.insert(
        5,
        SpellDuration {
            base: 200,
            per_level: 0,
            max: 200,
        },
    );
    for entry in [DRAGONLING, SQUIRE] {
        let mut t = template(entry, 9);
        t.faction = HUMANS;
        d.creature_templates.insert(entry, t);
    }
    d.spells
        .spells
        .insert(TWO_DRAGONLINGS, summoning(TWO_DRAGONLINGS, DRAGONLING, 2));
    d.spells
        .spells
        .insert(ONE_SQUIRE, summoning(ONE_SQUIRE, SQUIRE, 1));
    let mut kept = summoning(KEPT_DRAGONLING, DRAGONLING, 1);
    kept.category = 7;
    d.spells.spells.insert(KEPT_DRAGONLING, kept);
    let mut brief = summoning(BRIEF_DRAGONLING, DRAGONLING, 1);
    brief.duration_index = 5;
    d.spells.spells.insert(BRIEF_DRAGONLING, brief);
    d
}

/// `owner`'s guardians, by entry, in the map.
fn guardians(m: &Map, owner: Guid) -> Vec<(Guid, u32)> {
    let mut v: Vec<(Guid, u32)> = m
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.alive()
                && (c.owner == Some(owner)
                    || c.scripted.as_deref().and_then(|s| s.summoner) == Some(owner))
        })
        .map(|(_, c)| (c.guid, c.entry))
        .collect();
    v.sort();
    v
}

fn cast(m: &mut Map, g: Guid, spell: u32) {
    crate::game::map::spells::cast(
        m,
        g,
        spell,
        crate::game::map::spells::Targets::default(),
        None,
    );
}

/// Thistle Tea's "multiple guardians coexist and direct casts toggle only
/// their entry" and "triggered summons accumulate and categorized casts
/// replace", as mangos-classic's `EffectSummonGuardian`: a cast of two
/// makes two, each at its own place behind; another entry's adds to them;
/// a second cast of the first sends its two away and makes none; one a
/// spell triggers adds to those there; and a cast of a spell with a time
/// and a category replaces them.
#[tokio::test]
async fn a_second_cast_sends_a_guardian_away_and_a_triggered_one_adds_to_it() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let mut seen = Vec::new();
        let entries = |m: &Map| guardians(m, g).iter().map(|x| x.1).collect::<Vec<u32>>();
        cast(m, g, TWO_DRAGONLINGS);
        let first = guardians(m, g);
        let angles: Vec<f32> = first
            .iter()
            .map(|(x, _)| m.creature(*x).unwrap().pet.as_deref().unwrap().angle)
            .collect();
        seen.push(entries(m));
        cast(m, g, ONE_SQUIRE);
        seen.push(entries(m));
        cast(m, g, TWO_DRAGONLINGS);
        seen.push(entries(m));
        let first_gone = first.iter().all(|(x, _)| m.creature(*x).is_none());
        crate::game::map::spells::cast_triggered(m, g, TWO_DRAGONLINGS, None);
        crate::game::map::spells::cast_triggered(m, g, TWO_DRAGONLINGS, None);
        seen.push(entries(m));
        cast(m, g, KEPT_DRAGONLING);
        seen.push(entries(m));
        (
            seen,
            angles,
            first_gone,
            m.players[&g].owned.guardians.len(),
        )
    })
    .await;
    let (seen, angles, first_gone, owned) = out;
    let (d, s) = (DRAGONLING, SQUIRE);
    assert_eq!(seen[0], [d, d], "two at once");
    assert_ne!(angles[0], angles[1], "each its own place");
    assert_eq!(
        seen[1].iter().filter(|e| **e == s).count(),
        1,
        "a squire added"
    );
    assert_eq!(seen[1].len(), 3);
    assert_eq!(seen[2], [s], "the second cast sends the dragonlings away");
    assert!(first_gone);
    assert_eq!(seen[3].len(), 5, "the triggered add up: {:?}", seen[3]);
    assert_eq!(
        seen[4].iter().filter(|e| **e == d).count(),
        1,
        "a timed, categorized cast replaces them"
    );
    assert_eq!(owned, 2, "the squire and the new dragonling");
}

/// Thistle Tea's "duration expiry clears its matching owner identity" and
/// "world transitions remove every guardian": a guardian goes at the end
/// of its time, out of its owner's hands; and every one goes when its
/// owner leaves the map.
#[tokio::test]
async fn a_guardian_goes_at_its_time_and_all_go_with_their_owner() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let brief = on(&map, move |m| {
        cast(m, g, BRIEF_DRAGONLING);
        guardians(m, g)[0].0
    })
    .await;
    let mut gone = false;
    for _ in 0..100 {
        gone = on(&map, move |m| {
            m.creature(brief).is_none() && !m.players[&g].owned.guardians.contains(&brief)
        })
        .await;
        if gone {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "gone at its time, and out of its owner's hands");
    let two = on(&map, move |m| {
        cast(m, g, TWO_DRAGONLINGS);
        guardians(m, g)
    })
    .await;
    assert_eq!(two.len(), 2);
    on(&map, move |m| {
        m.teleport(
            g,
            MapKey::world(1),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    })
    .await;
    let mut left = two.len();
    for _ in 0..100 {
        let two = two.clone();
        left = on(&map, move |m| {
            two.iter().filter(|(x, _)| m.creature(*x).is_some()).count()
        })
        .await;
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(left, 0, "none left behind");
}

/// Thistle Tea's "scripts remove selected child owners and their world
/// projections": script command 56 takes a creature's guardians of an
/// entry, and with none named all of them.
#[tokio::test]
async fn a_script_takes_a_creatures_guardians_of_an_entry_or_all() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let owner = spawn(m, g, BRUTE, 20, 6.0);
        for spell in [TWO_DRAGONLINGS, ONE_SQUIRE] {
            let s = m.realm.data.spells.get(spell).cloned().unwrap();
            crate::game::map::pets::effect(
                m,
                owner,
                owner,
                &s,
                0,
                s.effects[0].base_points + 1,
                false,
                &Default::default(),
            );
        }
        let entries = |m: &Map| {
            guardians(m, owner)
                .iter()
                .map(|x| x.1)
                .collect::<Vec<u32>>()
        };
        let before = entries(m);
        let step = |entry: u32| crate::game::tests_p3_entity::step(56, [entry, 0, 0, 0]);
        crate::game::map::scripts::execute(m, &step(DRAGONLING), owner, Guid::zero());
        let some = entries(m);
        crate::game::map::scripts::execute(m, &step(0), owner, Guid::zero());
        (before, some, entries(m))
    })
    .await;
    let (mut before, some, none) = out;
    before.sort();
    assert_eq!(before, [DRAGONLING, DRAGONLING, SQUIRE]);
    assert_eq!(some, [SQUIRE], "the dragonlings taken");
    assert!(none.is_empty(), "and then all");
}

/// Thistle Tea's "creature level offsets honor template overrides and
/// valid level bounds", as mangos-classic's `EffectSummonGuardian`: a
/// creature's guardian is at its summoner's level less the effect's
/// offset of none or below, while that is a creature's level; else (an
/// offset above none, or a level below one) at its template's.
#[tokio::test]
async fn a_creatures_guardian_is_at_its_level_less_the_offset() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let levels = on(&map, move |m| {
        let owner = spawn(m, g, BRUTE, 60, 6.0);
        let mut levels = Vec::new();
        for offset in [0.0, -5.0, 1.0, -60.0] {
            let mut s = summoning(TWO_DRAGONLINGS, DRAGONLING, 1);
            s.effects[0].multiple_value = offset;
            crate::game::map::pets::effect(m, owner, owner, &s, 0, 1, false, &Default::default());
            let made = guardians(m, owner);
            let newest = made
                .iter()
                .map(|x| x.0)
                .find(|x| !levels.iter().any(|(y, _)| y == x))
                .unwrap();
            let level = m
                .creature(newest)
                .unwrap()
                .fields
                .get_u32(crate::game::fields::UNIT_FIELD_LEVEL);
            levels.push((newest, level));
        }
        levels.into_iter().map(|x| x.1).collect::<Vec<u32>>()
    })
    .await;
    assert_eq!(levels, [60, 55, 20, 20]);
}

const BOT: u32 = 98_202;
const TWO_BOTS: u32 = 98_214;

/// Thistle Tea's "ordinary summons keep their template faction level and
/// independent lifetime" and "repeated casts create independent actors
/// without replacing companions", as mangos-classic's `EffectSummonWild`:
/// a player's wild summon (a Field Repair Bot) is a creature of its own,
/// of its template's faction and level, made by the spell, no one's pet
/// and following no one; a second cast makes two more beside the two.
#[tokio::test]
async fn a_players_wild_summons_are_their_own_and_add_up() {
    let mut d = world();
    let mut bot = template(BOT, 9);
    bot.faction = 35;
    bot.level_min = 30;
    bot.level_max = 30;
    d.creature_templates.insert(BOT, bot);
    let mut s = summoning(TWO_BOTS, BOT, 2);
    s.effects[0].effect = crate::game::map::pets::EFFECT_SUMMON_WILD;
    d.spells.spells.insert(TWO_BOTS, s);
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        cast(m, g, TWO_BOTS);
        let first = guardians(m, g);
        cast(m, g, TWO_BOTS);
        let all = guardians(m, g);
        let looks: Vec<_> = all
            .iter()
            .map(|(x, _)| {
                let c = m.creature(*x).unwrap();
                (
                    c.fields
                        .get_u32(crate::game::fields::UNIT_FIELD_FACTIONTEMPLATE),
                    c.fields.get_u32(crate::game::fields::UNIT_FIELD_LEVEL),
                    c.fields.get_u32(crate::game::fields::UNIT_CREATED_BY_SPELL),
                    c.pet.is_some() || c.owner.is_some(),
                    c.scripted.as_deref().and_then(|s| s.follow).is_some(),
                )
            })
            .collect();
        let kept = first.iter().all(|(x, _)| m.creature(*x).is_some());
        (
            first.len(),
            all.len(),
            kept,
            looks,
            m.players[&g].owned.guardians.len(),
        )
    })
    .await;
    let (first, all, kept, looks, owned) = out;
    assert_eq!((first, all), (2, 4), "two a cast, adding up");
    assert!(kept, "the first two stay");
    for look in looks {
        assert_eq!(look, (35, 30, TWO_BOTS, false, false));
    }
    assert_eq!(owned, 0, "none of them a guardian");
}

const BURNING_WISH: u32 = 18_789;

/// Thistle Tea's "reviving a guardian removes sacrifice without reviving a
/// different retained pet", as mangos-classic's `EffectSummonPet` has it
/// for a warlock: a demon summoned ends the buff Demonic Sacrifice gave
/// for the last (Burning Wish, of class script 2228).
#[tokio::test]
async fn a_demon_summoned_ends_demonic_sacrifices_buff() {
    use crate::game::map::auras::{self, AURA_OVERRIDE_CLASS_SCRIPTS};
    let mut d = crate::game::tests_pets::data();
    let mut wish = spell(
        BURNING_WISH,
        &[SpellEffect {
            effect: 6,
            aura: AURA_OVERRIDE_CLASS_SCRIPTS,
            misc_value: 2228,
            target_a: 1,
            ..Default::default()
        }],
    );
    wish.duration_index = 4;
    d.spells.durations.insert(
        4,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    d.spells.spells.insert(BURNING_WISH, wish);
    let realm = crate::game::harness::realm_with(d);
    let (_w, gw, map) =
        crate::game::tests_pets::enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let out = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, gw, BURNING_WISH, Some(gw));
        let had = auras::auras(m, gw).is_some_and(|a| a.has_spell(BURNING_WISH));
        crate::game::map::spells::cast_triggered(
            m,
            gw,
            crate::game::tests_pets::SUMMON_IMP,
            Some(gw),
        );
        let imp = m.players[&gw].owned.pet.is_some();
        let kept = auras::auras(m, gw).is_some_and(|a| a.has_spell(BURNING_WISH));
        (had, imp, kept)
    })
    .await;
    assert_eq!(
        out,
        (true, true, false),
        "the buff, the imp, and the buff gone"
    );
}
