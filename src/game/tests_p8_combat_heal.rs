//! Round 8's combat tests, healing taken (Thistle Tea's
//! `healing_received_test.exs`): Blessing of Light on the heal it is for,
//! no heal raising the dead, and a stack of a healing cut dispelled.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_DUMMY, AURA_MOD_HEALING_PCT};
use crate::game::map::spells::outcome;
use crate::game::map::{Map, combat, spells};
use crate::game::tests_p3_combat::{self as p3, HELPER};

/// Blessing of Light (its family, flag and visual): 400 to a Holy Light,
/// 114 to a Flash of Light.
const BLESSING: u32 = 97_100;
/// Holy Light and Flash of Light as 1.12 has them (a script effect that
/// casts the heal), the heals they cast, a priest's heal and a paladin's
/// heal of another kind, each of 200.
const HOLY_LIGHT: u32 = 97_101;
const FLASH: u32 = 97_102;
const HOLY_LIGHT_HEAL: u32 = 19_968;
const FLASH_HEAL: u32 = 19_993;
const PRIEST_HEAL: u32 = 97_103;
const OTHER_HEAL: u32 = 97_104;
/// A healing cut of a tenth a stack, a disease's, and what cures one.
const BLIGHT: u32 = 97_105;
const CURE: u32 = 97_106;
const LONG: u32 = 971;

fn heal(id: u32, family: u32, flags: u64, effect: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Heal {id}"),
        school: 1,
        dmg_class: 1,
        range_index: 1,
        equipped_item_class: -1,
        family_name: family,
        family_flags: flags,
        spell_level: 60,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect,
        base_points: 199,
        base_dice: 1,
        target_a: 21,
        bonus_coefficient: 0.5,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = p3::data();
    d.spells.durations.insert(
        LONG,
        crate::game::data::spells::SpellDuration {
            base: 600_000,
            per_level: 0,
            max: 600_000,
        },
    );
    let mut blessing = SpellTemplate {
        id: BLESSING,
        name: "Blessing of Light".into(),
        equipped_item_class: -1,
        family_name: 10,
        family_flags: 0x1000_0000,
        spell_visual: 300,
        duration_index: LONG,
        ..Default::default()
    };
    for (i, amount) in [(0, 400), (1, 114)] {
        blessing.effects[i] = SpellEffect {
            effect: 6,
            aura: AURA_DUMMY,
            base_points: amount - 1,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
    }
    let mut blight = SpellTemplate {
        id: BLIGHT,
        name: "Blight".into(),
        equipped_item_class: -1,
        dispel: 3,
        stack_amount: 5,
        duration_index: LONG,
        ..Default::default()
    };
    blight.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_HEALING_PCT,
        base_points: -11,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut cure = SpellTemplate {
        id: CURE,
        name: "Cure".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    cure.effects[0] = SpellEffect {
        effect: 38,
        misc_value: 3,
        base_points: 0,
        base_dice: 1,
        target_a: 21,
        ..Default::default()
    };
    for s in [
        blessing,
        heal(HOLY_LIGHT, 10, 0x8000_0000, 77),
        heal(FLASH, 10, 0x4000_0000, 77),
        heal(HOLY_LIGHT_HEAL, 10, 0x4000, 10),
        heal(FLASH_HEAL, 10, 0x2000, 10),
        heal(PRIEST_HEAL, 6, 0x8000_0000, 10),
        heal(OTHER_HEAL, 10, 0x1, 10),
        blight,
        cure,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn set_health(m: &mut Map, g: Guid, h: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_HEALTH, h);
    }
}

/// Thistle Tea's "Blessing of Light selects the matching Paladin heal
/// effect", as mangos's `BlessingOfLight` script adds each of its effects
/// to its own heal: a Holy Light on one so blessed heals 200 and the
/// blessing's 400 by the heal's half coefficient, a Flash of Light 200 and
/// half of 114; a priest's heal, or a paladin's of another kind, 200 alone.
#[tokio::test]
async fn blessing_of_light_adds_its_own_to_holy_light_and_to_flash_of_light_alone() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let friend = p3::spawn(m, ga, HELPER, 60, 3.0);
            crate::game::tests_p3_spells_kit::apply(m, friend, friend, BLESSING);
            let mut out = Vec::new();
            for spell in [HOLY_LIGHT, FLASH, PRIEST_HEAL, OTHER_HEAL] {
                set_health(m, friend, 1_000);
                spells::cast_triggered(m, ga, spell, Some(friend));
                out.push(health(m, friend) - 1_000);
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, [400, 257, 200, 200]);
}

/// Thistle Tea's "a delayed transfer cannot resurrect either owner": a
/// heal that comes to the dead (a drain's or a funnel's, arriving as its
/// caster or its target died) leaves them dead, player or creature.
#[tokio::test]
async fn a_heal_arriving_at_the_dead_raises_no_one() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let friend = p3::spawn(m, ga, HELPER, 60, 3.0);
            let mut out = Vec::new();
            for g in [friend, ga] {
                combat::kill(m, g, g);
                spells::leech_heal(m, g, HOLY_LIGHT_HEAL, 200, false);
                out.push((health(m, g), m.unit_view(g).is_some_and(|u| u.alive)));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, [(0, false), (0, false)], "(the creature, Alice)");
}

/// Thistle Tea's "partial dispels reduce suppression without resetting
/// expiry": a healing cut of five stacks halves a heal; a cure takes one
/// stack, the cut down to four tenths and its end where it was.
#[tokio::test]
async fn a_cure_takes_a_stack_of_a_healing_cut_and_leaves_its_end_be() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let friend = p3::spawn(m, ga, HELPER, 60, 3.0);
            let s = m.realm.data.spells.get(BLIGHT).cloned().unwrap();
            for _ in 0..5 {
                let _ = auras::apply_spell(
                    m,
                    friend,
                    friend,
                    &s,
                    &[(0, -10)],
                    auras::Context::default(),
                );
            }
            let blight = |m: &Map| crate::game::tests_p3_spells_kit::holder(m, friend, BLIGHT);
            let (stacks, expires) = blight(m).map(|h| (h.stacks, h.expires)).unwrap();
            let cut = outcome::healing_taken(m, friend, 200.0) as u32;
            spells::cast_triggered(m, ga, CURE, Some(friend));
            let after = blight(m).map(|h| (h.stacks, h.expires));
            (
                stacks,
                cut,
                after,
                expires,
                outcome::healing_taken(m, friend, 200.0) as u32,
            )
        })
        .await
        .unwrap();
    let (stacks, cut, after, expires, cured) = r;
    assert_eq!((stacks, cut), (5, 100), "five stacks: half");
    assert_eq!(after, Some((4, expires)), "a stack gone, its end kept");
    assert_eq!(cured, 120, "four tenths");
}

/// Thistle Tea's "player immunity also blocks helpful spells while NPC
/// immunity does not": a friend immune to players (unit flag 0x100) takes
/// no heal of Alice's, nor of her pet's, though a creature may help it;
/// one immune to creatures alone (0x200) takes hers.
#[tokio::test]
async fn one_immune_to_players_takes_no_help_of_theirs() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let friend = p3::spawn(m, ga, HELPER, 60, 3.0);
            let npc = p3::spawn(m, ga, HELPER, 60, 4.0);
            let pet = p3::spawn(m, ga, HELPER, 60, 5.0);
            m.creature_mut(pet).unwrap().owner = Some(ga);
            let flags = |m: &mut Map, f: u32| {
                m.unit_fields_mut(friend)
                    .unwrap()
                    .set_u32(UNIT_FIELD_FLAGS, f)
            };
            flags(m, 0x300);
            let protected = (
                combat::can_assist(m, ga, friend),
                combat::can_assist(m, pet, friend),
                combat::can_assist(m, npc, friend),
                combat::can_assist(m, friend, ga),
            );
            set_health(m, friend, 1_000);
            crate::game::tests_p3_spells_kit::cast_fresh(
                m,
                ga,
                OTHER_HEAL,
                crate::game::tests_p3_spells_kit::at(friend),
            );
            let healed = health(m, friend) > 1_000;
            flags(m, 0x200);
            (protected, healed, combat::can_assist(m, ga, friend))
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        (false, false, true, true),
        "(Alice, her pet, a creature, it helping her)"
    );
    assert!(!r.1, "her heal refused");
    assert!(r.2, "immune to creatures alone, hers it takes");
}
