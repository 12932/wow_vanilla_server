//! Round 8's creature AI tests of spell lists, ported from Thistle Tea's
//! `test/game/core/ai/bt/mob/spells_test.exs` (docs/parity/matrix.csv): an
//! entry's script as its cast begins, a walk halted and the victim faced
//! for a cast that takes time, and a spell's creature types.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{CreatureSpell, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::creature::Motion;
use crate::game::map::spells::lists;
use crate::game::map::{Map, ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::step;

/// An instant bolt; one that takes three seconds; one of three seconds
/// for beasts alone (creature type 1, mask 0x1).
const BOLT: u32 = 95_840;
const SLOW: u32 = 95_841;
const MASKED: u32 = 95_842;
/// Lists of each, the bolt's with script 21_147 (an emote, 15).
const WITH_SCRIPT: u32 = 9_584;
const SLOW_LIST: u32 = 9_585;
const MASKED_LIST: u32 = 9_586;
const SCRIPT: u32 = 21_147;

fn world() -> GameData {
    let mut d = data();
    // A forty-yard range (index 4; index 2 is 1.12's melee "Combat Range").
    d.spells.ranges.insert(4, (0.0, 40.0));
    d.spells.cast_times.insert(1, 0);
    d.spells.cast_times.insert(99, 3_000);
    for (id, time, mask) in [(BOLT, 1, 0), (SLOW, 99, 0), (MASKED, 99, 0x1)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Bolt {id}"),
            range_index: 4,
            casting_time_index: time,
            target_creature_type: mask,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 2,
            base_points: 0,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    for (list, spell, script) in [
        (WITH_SCRIPT, BOLT, SCRIPT),
        (SLOW_LIST, SLOW, 0),
        (MASKED_LIST, MASKED, 0),
    ] {
        d.spells.creature_spells.insert(
            list,
            vec![CreatureSpell {
                spell,
                probability: 100,
                target: 1,
                delay_repeat: (5_000, 5_000),
                script,
                ..Default::default()
            }],
        );
    }
    d.scripts
        .creature_spells
        .insert(SCRIPT, vec![step(1, [15, 0, 0, 0])]);
    d.emote_states.insert(15);
    d
}

/// A creature `yards` east of Alice fighting her, on `list`.
fn caster(m: &mut Map, ga: Guid, yards: f32, list: u32) -> Guid {
    let k = spawn(m, ga, BRUTE, 20, yards);
    crate::game::map::event_ai::scripted_mut(m, k)
        .unwrap()
        .no_combat_movement = true;
    ai::attacked(m, k, ga);
    lists::set_list(m, k, list);
    k
}

fn casting(m: &Map, g: Guid) -> Option<u32> {
    m.caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| c.spell)
}

fn emote(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_NPC_EMOTESTATE))
}

/// Alice `yards` east of her start, out of the caster's way.
fn alice_at(m: &mut Map, ga: Guid, from: Guid, yards: f32) {
    let c = m.position_of(from).unwrap();
    let p = m.players.get_mut(&ga).unwrap();
    p.movement.position = Vector3d {
        x: c.x - yards,
        ..c
    };
}

/// Thistle Tea's "runs the entry's spell script once the cast starts": a
/// list's spell cast at its victim 30 yards off runs its entry's script
/// (its emote); one out of range, at 100, isn't cast and runs none.
#[tokio::test]
async fn a_list_entrys_script_runs_as_its_cast_begins_and_not_without_one() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            [30.0, 100.0]
                .into_iter()
                .map(|yards| {
                    let k = caster(m, ga, 5.0, WITH_SCRIPT);
                    alice_at(m, ga, k, yards);
                    lists::tick(m, k, ga);
                    let out = emote(m, k);
                    ai::die(m, k, ga);
                    out
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(out, [15, 0]);
}

/// Thistle Tea's "halts an in-flight move and faces the victim before a
/// timed cast": a creature walking when its list casts a spell that takes
/// time stops where it is, faced at its victim, and casts.
#[tokio::test]
async fn a_list_cast_that_takes_time_halts_its_walk_and_faces_its_victim() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = caster(m, ga, 20.0, SLOW_LIST);
            let now = m.now;
            let id = m.next_spline_id;
            m.next_spline_id += 1;
            let c = m.creature_mut(k).unwrap();
            let from = c.position;
            let to = Vector3d {
                y: from.y + 30.0,
                ..from
            };
            let _ = c.walk(vec![from, to], false, now, id);
            c.orientation = std::f32::consts::FRAC_PI_2;
            m.now += Duration::from_millis(200);
            lists::tick(m, k, ga);
            let c = m.creature(k).unwrap();
            let her = m.position_of(ga).unwrap();
            let facing = (her.y - c.position.y).atan2(her.x - c.position.x);
            (
                casting(m, k),
                matches!(c.motion, Motion::Idle { .. }),
                c.position.y > from.y,
                (c.orientation - facing).abs() < 0.01,
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (Some(SLOW), true, true, true));
}

/// Thistle Tea's "rejects targets outside the spell's creature type mask":
/// a spell for beasts alone isn't cast at a player, from a list or a
/// script (a player is a humanoid, whom Thistle Tea's mask 0x40 names, as
/// mangos's `GetCreatureTypeMask` has it).
#[tokio::test]
async fn a_spell_for_some_creature_types_isnt_cast_at_a_player() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let k = caster(m, ga, 10.0, MASKED_LIST);
            lists::tick(m, k, ga);
            let masked = casting(m, k);
            let direct = scripts::cast(m, k, ga, MASKED, 0);
            (masked, direct, casting(m, k))
        })
        .await
        .unwrap();
    assert_eq!(out, (None, false, None));
}
