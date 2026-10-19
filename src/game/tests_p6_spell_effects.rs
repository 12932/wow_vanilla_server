//! Phase 3, round 6, against Thistle Tea's `mechanical_healing_test.exs`
//! and `spell_teaching_test.exs`: repairing a mechanical friend, and
//! spells that teach spells and skill steps.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, health, set_health};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{CreatureTemplate, FactionTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::auras::AURA_MOD_HEALING_PCT;
use crate::game::map::spells::{self, checks, fail, mods::Mods};
use crate::game::map::{Map, skills};

/// A mechanical friend, a beast friend, and a mechanical foe.
const GOLEM: u32 = 9_001;
const PET: u32 = 9_002;
const FOE: u32 = 9_003;
/// Repair of 700 at a friendly mechanical unit; a halving of healing taken.
const REPAIR: u32 = 990_930;
const HALVED: u32 = 990_931;

fn creature(entry: u32, creature_type: u32, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Made-up {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 60,
        level_max: 60,
        faction,
        creature_type,
        ..Default::default()
    }
}

fn faction(id: u32, faction: u32, ours: u32, friends: u32, enemies: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction,
        ours,
        friend_mask: friends,
        enemy_mask: enemies,
        ..Default::default()
    }
}

fn repairs(d: &mut GameData) {
    // A human's template, Stormwind's, a monster's.
    d.faction_templates.insert(1, faction(1, 1, 0x3, 0x2, 0x8));
    d.faction_templates
        .insert(11, faction(11, 72, 0x2, 0x2, 0xC));
    d.faction_templates.insert(14, faction(14, 14, 0x8, 0, 0x7));
    for t in [
        creature(GOLEM, 9, 11),
        creature(PET, 1, 11),
        creature(FOE, 9, 14),
    ] {
        d.creature_templates.insert(t.entry, t);
    }
    let mut repair = bare(REPAIR, 0);
    repair.target_creature_type = 1 << 8;
    effect(
        &mut repair,
        0,
        SpellEffect {
            effect: spells::EFFECT_HEAL_MECHANICAL,
            base_points: 699,
            base_dice: 1,
            target_a: 21,
            ..Default::default()
        },
    );
    let mut halved = bare(HALVED, 0);
    halved.duration_index = LASTS_LONG;
    effect(&mut halved, 0, aura(AURA_MOD_HEALING_PCT, 0, -50, 1));
    let spells: Vec<SpellTemplate> = vec![repair, halved];
    insert(d, spells);
}

fn why(
    m: &mut Map,
    caster: wow_world_messages::Guid,
    target: wow_world_messages::Guid,
) -> Option<u8> {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(REPAIR).cloned().unwrap();
    checks::check(m, caster, &s, &at(target), None, false, &Mods::default())
}

/// A repair mends a friendly mechanical unit by its amount, no more than
/// its health's maximum, and less for a cut to the healing it takes; it
/// is cast only at a living, friendly, mechanical unit (Thistle Tea's
/// mechanical healing; mangos's `EffectHealMechanical`, a heal of a
/// `TargetCreatureType` of mechanicals).
#[tokio::test]
async fn a_repair_mends_a_living_friendly_machine() {
    let realm = fast_realm(repairs);
    let (_c, a, map) = human(&realm, "P6EA", "Tinker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[REPAIR]);
            let golem = kit::spawn(m, a, GOLEM, 60, 5.0);
            let beast = kit::spawn(m, a, PET, 60, 5.0);
            let foe = kit::spawn(m, a, FOE, 60, 5.0);
            let mut mended = Vec::new();
            for (from, halved) in [(100, false), (999_900, false), (100, true)] {
                set_health(m, golem, from);
                if halved {
                    kit::apply(m, golem, golem, HALVED);
                }
                let w = why(m, a, golem);
                assert!(cast_fresh(m, a, REPAIR, at(golem)), "{w:?}");
                mended.push(health(m, golem) - from);
            }
            let checks = (why(m, a, golem), why(m, a, beast), why(m, a, foe));
            set_health(m, golem, 0);
            let later = m.now + std::time::Duration::from_secs(600);
            if let Some(c) = m.creature_mut(golem) {
                c.life = crate::game::map::creature::Life::Dead {
                    corpse_until: later,
                    respawn_at: later,
                };
            }
            (mended, checks, why(m, a, golem))
        })
        .await
        .unwrap();
    let (mended, checks, dead) = out;
    assert_eq!(mended, [700, 100, 350], "its amount, capped, halved");
    assert_eq!(
        checks,
        (None, Some(fail::BAD_TARGETS), Some(fail::TARGET_ENEMY)),
        "a machine, a beast, a foe"
    );
    assert_eq!(dead, Some(fail::TARGETS_DEAD));
}

/// Two spells to learn and a cooking step: the spells for teaching.
const TAUGHT_A: u32 = 990_932;
const TAUGHT_B: u32 = 990_933;
const TEACH: u32 = 990_934;
const TEACH_AT: u32 = 990_935;
const COOKING: u32 = 185;
const BAD_STEP: u32 = 990_936;

fn teaching(d: &mut GameData) {
    let learn = |spell: u32, target: u32| SpellEffect {
        effect: spells::EFFECT_LEARN_SPELL,
        trigger_spell: spell,
        target_a: target,
        ..Default::default()
    };
    let step = |points: i32, target: u32| SpellEffect {
        effect: spells::EFFECT_SKILL_STEP,
        misc_value: COOKING as i32,
        base_points: points - 1,
        base_dice: 1,
        target_a: target,
        ..Default::default()
    };
    let teach = |id: u32, target: u32| {
        let mut s = bare(id, 0);
        effect(&mut s, 0, learn(TAUGHT_A, target));
        effect(&mut s, 1, learn(TAUGHT_B, target));
        effect(&mut s, 2, step(3, target));
        s
    };
    let mut bad = bare(BAD_STEP, 0);
    effect(&mut bad, 0, step(-2, 1));
    let spells: Vec<SpellTemplate> = vec![
        bare(TAUGHT_A, 0),
        bare(TAUGHT_B, 0),
        teach(TEACH, 1),
        teach(TEACH_AT, 25),
        bad,
    ];
    insert(d, spells);
}

/// A spell of two spells to learn and a skill step teaches both and raises
/// the skill's step and cap, keeping its value and slot; an NPC's teaching
/// cast reaches its player and teaches a creature nothing; a negative step
/// teaches nothing (Thistle Tea's `SpellTeaching`; mangos's
/// `EffectLearnSpell` and `EffectLearnSkill`).
#[tokio::test]
async fn a_teaching_spell_teaches_its_spells_and_steps_its_skill() {
    let realm = fast_realm(teaching);
    let (_c, a, map) = human(&realm, "P6EB", "Pupil", Class::Warrior).await;
    let (_d, b, _) = human(&realm, "P6EC", "Other", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[TEACH]);
            let p = m.players.get_mut(&a).unwrap();
            skills::learn_skill(p, COOKING, 4);
            let slot = (0..128u16)
                .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == COOKING);
            let base = PLAYER_SKILL_INFO_1_1 + slot.unwrap() * 3;
            p.fields.set_u32(base + 1, 125 | (300 << 16));
            cast_fresh(m, a, TEACH, Default::default());
            let p = &m.players[&a];
            let known = (p.spells.contains(&TAUGHT_A), p.spells.contains(&TAUGHT_B));
            let skill = (
                p.fields.get_u32(base) >> 16,
                p.fields.get_u32(base + 1) & 0xFFFF,
                p.fields.get_u32(base + 1) >> 16,
            );
            // A trainer's cast at another player, and at a creature.
            let trainer = kit::dummy(m, b, 60, 3.0);
            spells::cast_triggered(m, trainer, TEACH_AT, Some(b));
            let other = &m.players[&b];
            let taught = (
                other.spells.contains(&TAUGHT_A),
                skills::max_value(other, COOKING),
            );
            let kobold = kit::dummy(m, b, 60, 4.0);
            spells::cast_triggered(m, trainer, TEACH_AT, Some(kobold));
            spells::cast_triggered(m, b, BAD_STEP, None);
            let after_bad = skills::max_value(&m.players[&b], COOKING);
            (
                known,
                skill,
                slot == (0..128u16).find(|i| {
                    m.players[&a].fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == COOKING
                }),
                taught,
                after_bad,
            )
        })
        .await
        .unwrap();
    let (known, skill, same_slot, taught, after_bad) = out;
    assert_eq!(known, (true, true), "both spells");
    assert_eq!(skill, (3, 125, 225), "step 3, its value kept, cap 225");
    assert!(same_slot);
    assert_eq!(taught, (true, Some(225)), "the trainer's pupil");
    assert_eq!(after_bad, Some(225), "a negative step teaches nothing");
}
