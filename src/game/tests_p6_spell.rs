//! Phase 3, round 6, against Thistle Tea's `posture_test.exs`,
//! `facing_test.exs`, `area_test.exs`, `mount_test.exs` and
//! `casting_immunity_test.exs`: what a cast is checked for again as it
//! ends, before it costs anything, and the facing and area rules.

use std::f32::consts::PI;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, casting, health, pass, power};
use crate::game::data::GameData;
use crate::game::data::scripts::SpellArea;
use crate::game::data::spells::{
    ATTR_ALLOW_WHILE_SITTING, ATTR_EX2_AUTO_REPEAT, ATTR_RANGED, CUSTOM_FACE_TARGET, SpellEffect,
    SpellTemplate,
};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_MOUNTED, INTERRUPT_NOT_SEATED};
use crate::game::map::spells::{checks, cooldowns, fail, mods::Mods};

/// Two seconds' heal of the caster, for 30 mana, on a 5 s cooldown; food
/// eaten seated; a bolt at an enemy cast facing it; a spell of one area; a
/// mount.
const HEAL: u32 = 990_900;
const FOOD: u32 = 990_901;
const BOLT: u32 = 990_902;
const PLACE: u32 = 990_903;
const MOUNT: u32 = 990_904;
/// A melee-range strike, a ranged one, and an auto-repeat shot.
const STRIKE: u32 = 990_905;
const SHOT: u32 = 990_906;
const AUTO_SHOT: u32 = 990_907;
const AREA: u32 = 4_242;

fn spells(d: &mut GameData) {
    let timed = |id: u32, school: u32, e: SpellEffect| {
        let mut s = bare(id, school);
        s.casting_time_index = CAST_2S;
        s.mana_cost = 30;
        s.recovery_time = 5_000;
        effect(&mut s, 0, e);
        s
    };
    let heal = SpellEffect {
        effect: 10,
        base_points: 9,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut food = timed(FOOD, 0, heal);
    food.aura_interrupt_flags = INTERRUPT_NOT_SEATED;
    food.attributes[0] |= ATTR_ALLOW_WHILE_SITTING;
    let mut bolt = timed(BOLT, 2, damage(10));
    bolt.custom_flags = CUSTOM_FACE_TARGET;
    let mount = timed(MOUNT, 0, aura(AURA_MOUNTED, 6, 0, 1));
    let mut strike = bare(STRIKE, 0);
    strike.range_index = 2;
    effect(&mut strike, 0, damage(10));
    let mut shot = bare(SHOT, 0);
    effect(&mut shot, 0, damage(10));
    let mut auto = bare(AUTO_SHOT, 0);
    auto.attributes[0] |= ATTR_RANGED;
    auto.attributes[2] |= ATTR_EX2_AUTO_REPEAT;
    effect(&mut auto, 0, damage(10));
    let spells: Vec<SpellTemplate> = vec![
        timed(HEAL, 2, heal),
        food,
        bolt,
        timed(PLACE, 2, heal),
        mount,
        strike,
        shot,
        auto,
    ];
    insert(d, spells);
    d.scripts.spell_areas.push(SpellArea {
        spell: PLACE,
        area: AREA,
        gender: 2,
        ..Default::default()
    });
}

/// `spell` begun by `a` at `t`, `mid` done a second in, and the cast's
/// end passed: whether it is still under way, the mana it took, whether
/// its cooldown started, and the health `a` gained.
fn finish(
    m: &mut Map,
    a: Guid,
    spell: u32,
    t: crate::game::map::spells::Targets,
    mid: impl FnOnce(&mut Map),
) -> (Option<u32>, u32, bool, u32) {
    kit::set_health(m, a, 100);
    let mana = power(m, a, 0);
    kit::fresh(m, a);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let why = checks::check(m, a, &s, &t, None, false, &Mods::default());
    assert!(cast_fresh(m, a, spell, t), "{spell} begins: {why:?}");
    pass(m, 1_000);
    mid(m);
    pass(m, 1_100);
    (
        casting(m, a),
        mana - power(m, a, 0),
        m.caster(a)
            .is_some_and(|c| c.cooldowns.contains_key(&spell)),
        health(m, a) - 100,
    )
}

/// What `c` was told of each cast that failed as it ended, in order.
async fn failures(c: &mut crate::game::harness::Client, n: usize) -> Vec<(u32, u8)> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(
            c.wait_for("SMSG_SPELL_FAILURE", |m| match m {
                ServerOpcodeMessage::SMSG_SPELL_FAILURE(f) => Some((f.spell, f.result.as_int())),
                _ => None,
            })
            .await,
        );
    }
    out
}

/// A cast is checked again as it ends: sat down meanwhile, it fails
/// NOT_STANDING, costing nothing and starting no cooldown, but stood up
/// again it goes; food walked off with fails MOVING; a school locked out
/// meanwhile fails SILENCED; a spell of one area, left, REQUIRES_AREA; a
/// mount, shifted into a cat meanwhile, NOT_SHAPESHIFT; a bolt turned away
/// from, UNIT_NOT_INFRONT, its target unharmed (Thistle Tea's `Posture`
/// and `Casting.complete`, its launch's area, mount and facing
/// requirements and school lockout; mangos's `Spell::cast` runs
/// `CheckCast(false)`).
#[tokio::test]
async fn a_cast_is_checked_again_as_it_ends() {
    use wow_world_messages::vanilla::MovementInfo_MovementFlags as Flags;
    let realm = fast_realm(spells);
    let (mut c, a, map) = human(&realm, "P6SA", "Rechecked", Class::Paladin).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[HEAL, FOOD, BOLT, PLACE, MOUNT]);
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000);
            }
            let me = at(a);
            let sat = finish(m, a, HEAL, me, |m| {
                m.players.get_mut(&a).unwrap().set_stand_state(1);
            });
            m.players.get_mut(&a).unwrap().set_stand_state(0);
            let stood = finish(m, a, HEAL, me, |m| {
                m.players.get_mut(&a).unwrap().set_stand_state(1);
                m.players.get_mut(&a).unwrap().set_stand_state(0);
            });
            let walked = finish(m, a, FOOD, me, |m| {
                m.players.get_mut(&a).unwrap().movement.flags = Flags::empty().set_forward();
            });
            m.players.get_mut(&a).unwrap().movement.flags = Flags::empty();
            let still = finish(m, a, FOOD, me, |m| {
                m.players.get_mut(&a).unwrap().set_stand_state(1);
            });
            m.players.get_mut(&a).unwrap().set_stand_state(0);
            let locked = finish(m, a, HEAL, me, |m| {
                cooldowns::lock_school(m, a, HEAL, 5_000)
            });
            m.players.get_mut(&a).unwrap().caster.school_lockout.clear();
            m.players.get_mut(&a).unwrap().area = AREA;
            let left = finish(m, a, PLACE, me, |m| m.players.get_mut(&a).unwrap().area = 1);
            let mounted = finish(m, a, MOUNT, me, |m| {
                if let Some(f) = m.unit_fields_mut(a) {
                    f.set_byte(UNIT_FIELD_BYTES_1, 2, 1);
                }
            });
            if let Some(f) = m.unit_fields_mut(a) {
                f.set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            }
            let kobold = kit::dummy(m, a, 60, 5.0);
            let turned = finish(m, a, BOLT, at(kobold), |m| {
                m.players.get_mut(&a).unwrap().movement.orientation += PI;
            });
            (
                sat,
                stood,
                walked,
                still,
                locked,
                left,
                mounted,
                turned,
                health(m, kobold),
            )
        })
        .await
        .unwrap();
    let (sat, stood, walked, still, locked, left, mounted, turned, kobold) = out;
    let failed = (None, 0, false, 0);
    assert_eq!(sat, failed, "sat down: nothing paid");
    assert_eq!(stood, (None, 30, true, 10), "stood up again: it goes");
    assert_eq!(walked, failed, "food walked off with");
    assert_eq!(still, (None, 30, true, 10), "food eaten sitting");
    assert_eq!(locked, failed, "its school locked out");
    assert_eq!(left, failed, "its area left");
    assert_eq!(mounted, failed, "a cat can't ride");
    assert_eq!(turned.0, None);
    assert_eq!((turned.1, turned.2), (0, false), "turned away");
    assert_eq!(kobold, 1_000_000, "unharmed");
    let told = failures(&mut c, 6).await;
    assert_eq!(
        told,
        [
            (HEAL, fail::NOT_STANDING),
            (FOOD, fail::MOVING),
            (HEAL, fail::SILENCED),
            (PLACE, fail::REQUIRES_AREA),
            (MOUNT, 0x3d),
            (BOLT, fail::UNIT_NOT_INFRONT),
        ]
    );
}

fn why(m: &mut Map, caster: Guid, spell: u32, target: Guid, triggered: bool) -> Option<u8> {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    checks::check(
        m,
        caster,
        &s,
        &at(target),
        None,
        triggered,
        &Mods::default(),
    )
}

/// A creature, as a player, casts a melee-range spell only facing its
/// target, unless the spell is triggered; a ranged one it may cast turned
/// away (turning to it is its AI's). A player's auto-repeat shot must face
/// its target (Thistle Tea's `Facing.validate`).
#[tokio::test]
async fn creatures_face_for_melee_spells_and_players_for_auto_repeat() {
    let realm = fast_realm(spells);
    let (_c, a, map) = kit::player(&realm, "P6SB", "Faced", Race::Dwarf, Class::Hunter).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[STRIKE, SHOT, AUTO_SHOT]);
            let kobold = kit::dummy(m, a, 60, 2.0);
            let facing = why(m, kobold, STRIKE, a, false);
            m.creature_mut(kobold).unwrap().orientation += PI;
            let away = (
                why(m, kobold, STRIKE, a, false),
                why(m, kobold, SHOT, a, false),
                why(m, kobold, STRIKE, a, true),
            );
            let shot_facing = why(m, a, AUTO_SHOT, kobold, false);
            m.players.get_mut(&a).unwrap().movement.orientation += PI;
            let shot_away = why(m, a, AUTO_SHOT, kobold, false);
            (facing, away, shot_facing, shot_away)
        })
        .await
        .unwrap();
    let (facing, away, shot_facing, shot_away) = out;
    assert_eq!(facing, None);
    assert_eq!(
        away,
        (Some(fail::UNIT_NOT_INFRONT), None, None),
        "melee, ranged, triggered"
    );
    assert_eq!(shot_facing, None);
    assert_eq!(shot_away, Some(fail::UNIT_NOT_INFRONT));
}

/// A spell's area rules: any one of them holding will do; a rule with a
/// race, gender, quest or aura holds for no creature, while one of a place
/// alone does; a start quest must be rewarded, or, for a rule that says
/// so, only taken (Thistle Tea's `Area.matches?` and `validate/2`,
/// mangos's `SpellArea::IsFitToRequirements`).
#[tokio::test]
async fn area_rules_take_any_alternative_and_want_a_player_for_a_players_rule() {
    use crate::game::map::spell_areas::allowed;
    const TWO: u32 = 990_910;
    const RACE: u32 = 990_911;
    const ROAM: u32 = 990_912;
    const TAKEN: u32 = 990_913;
    const DONE: u32 = 990_914;
    let realm = fast_realm(|d| {
        spells(d);
        let rule = |spell: u32, area: u32| SpellArea {
            spell,
            area,
            gender: 2,
            ..Default::default()
        };
        d.scripts.spell_areas.extend([
            rule(TWO, 148),
            SpellArea {
                gender: 1,
                ..rule(TWO, 139)
            },
            SpellArea {
                race_mask: 1,
                ..rule(RACE, 0)
            },
            rule(ROAM, 0),
            SpellArea {
                quest_start: 10,
                quest_start_active: true,
                ..rule(TAKEN, 0)
            },
            SpellArea {
                quest_start: 10,
                ..rule(DONE, 0)
            },
        ]);
    });
    let (_c, a, map) = human(&realm, "P6SC", "Placed", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let kobold = kit::dummy(m, a, 60, 5.0);
            let p = m.players.get_mut(&a).unwrap();
            p.zone = 139;
            p.area = 139;
            p.row.gender = 0;
            let female_only = allowed(m, a, TWO);
            m.players.get_mut(&a).unwrap().area = 148;
            let other = allowed(m, a, TWO);
            let creature = (allowed(m, kobold, RACE), allowed(m, kobold, ROAM));
            let human = allowed(m, a, RACE);
            let none = (allowed(m, a, TAKEN), allowed(m, a, DONE));
            m.players
                .get_mut(&a)
                .unwrap()
                .journal
                .add(crate::game::map::quests::Logged {
                    quest: 10,
                    ..Default::default()
                });
            let taken = (allowed(m, a, TAKEN), allowed(m, a, DONE));
            let p = m.players.get_mut(&a).unwrap();
            p.journal.clear(0);
            p.journal.rewarded.insert(10);
            let rewarded = (allowed(m, a, TAKEN), allowed(m, a, DONE));
            (female_only, other, creature, human, none, taken, rewarded)
        })
        .await
        .unwrap();
    let (female_only, other, creature, human, none, taken, rewarded) = out;
    assert!(!female_only, "a man where only women may");
    assert!(other, "the other rule's place");
    assert_eq!(creature, (false, true), "a race asks a player");
    assert!(human);
    assert_eq!(none, (false, false), "no quest");
    assert_eq!(taken, (true, false), "taken: only the active rule");
    assert_eq!(rewarded, (true, true));
}
