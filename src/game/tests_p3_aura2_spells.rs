//! Phase 3's spells in the world, Thistle Tea's `SpellRequirementsTest`
//! and its kin ported: what a cast's end checks of its target before it
//! costs anything (range, with mangos's tolerance; facing; life).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::spells::{self, checks::fail};
use crate::game::tests_p3_aura2::{add_spells, get, health};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};

/// A fire bolt of a second and a half's cast, 30 yards, ten mana and a
/// cooldown of five seconds, cast facing its target.
const SLOW_BOLT: u32 = 96_700;
const CAST_TIME: u32 = 96;

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    d.spells.cast_times.insert(CAST_TIME, 1_500);
    let mut s = SpellTemplate {
        id: SLOW_BOLT,
        name: "Slow Bolt".into(),
        school: 2,
        equipped_item_class: -1,
        range_index: 1,
        casting_time_index: CAST_TIME,
        mana_cost: 10,
        recovery_time: 5_000,
        custom_flags: sd::CUSTOM_FACE_TARGET,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: spells::EFFECT_SCHOOL_DAMAGE,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    add_spells(&mut d, [s]);
    d
}

/// The reaches of `a` and `b` summed.
fn reaches(m: &Map, a: Guid, b: Guid) -> f32 {
    m.unit_view(a).map_or(0.0, |u| u.combat_reach) + m.unit_view(b).map_or(0.0, |u| u.combat_reach)
}

/// Put `g` `yards` straight ahead of the player `from`.
fn put_ahead(m: &mut Map, from: Guid, g: Guid, yards: f32) {
    let p = &m.players[&from];
    let (pos, o) = (p.position(), p.movement.orientation);
    if let Some(c) = m.creature_mut(g) {
        c.position = Vector3d {
            x: pos.x + yards * o.cos(),
            y: pos.y + yards * o.sin(),
            z: pos.z,
        };
    }
}

/// What a cast came to as it ended: whether it began, the target's health
/// and the caster's mana after, and whether its cooldown started.
fn finish(
    m: &mut Map,
    g: Guid,
    target: Guid,
    between: impl FnOnce(&mut Map),
) -> (bool, u32, u32, bool) {
    if let Some(c) = m.caster_mut(g) {
        c.cooldowns.clear();
        c.gcd_until = None;
    }
    crate::game::tests_p3_aura2::set(m, g, UNIT_FIELD_POWER1, 100);
    let began = spells::cast(
        m,
        g,
        SLOW_BOLT,
        spells::Targets {
            unit: Some(target),
            ..Default::default()
        },
        None,
    );
    between(m);
    m.now += Duration::from_millis(1_500);
    spells::update(m);
    let cooling = m
        .caster(g)
        .is_some_and(|c| c.cooldowns.contains_key(&SLOW_BOLT));
    (
        began,
        health(m, target),
        get(m, g, UNIT_FIELD_POWER1),
        cooling,
    )
}

/// The failure `c` was told of `SLOW_BOLT`'s cast.
async fn failure(c: &mut Client) -> u8 {
    let b = c
        .wait_frame("SMSG_CAST_RESULT", 0x130, |b| {
            b[..4] == SLOW_BOLT.to_le_bytes() && b[4] == 2
        })
        .await;
    b[5]
}

/// Thistle Tea's launch checks, and mangos's `CheckCast(false)` as a cast
/// ends: a target gone beyond the range and a player's launch leeway (6.25
/// yards, Thistle Tea's `Range`, round 11c) fails the cast as out of range, costing no mana
/// and starting no cooldown; one just within it is hit, the cast paid
/// for. A caster turned away from a target it must face fails as not in
/// front, and the same cast facing it again goes; a target dead fails as
/// dead.
#[tokio::test]
async fn a_cast_ends_only_on_a_target_still_in_range_in_front_and_alive() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Priest).await;
    a.clear();
    let gone = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 20.0);
            let edge = 30.0 + reaches(m, ga, brute) + 6.25;
            finish(m, ga, brute, |m| put_ahead(m, ga, brute, edge + 0.05))
        })
        .await
        .unwrap();
    assert_eq!(gone, (true, 100_000, 100, false), "nothing dealt or paid");
    assert_eq!(failure(&mut a).await, fail::OUT_OF_RANGE);
    let (within, turned, faced) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 20.0);
            let edge = 30.0 + reaches(m, ga, brute) + 6.25;
            let within = finish(m, ga, brute, |m| put_ahead(m, ga, brute, edge - 0.05));
            put_ahead(m, ga, brute, 20.0);
            let turned = finish(m, ga, brute, |m| {
                if let Some(p) = m.players.get_mut(&ga) {
                    p.movement.orientation += std::f32::consts::PI;
                }
            });
            if let Some(p) = m.players.get_mut(&ga) {
                p.movement.orientation -= std::f32::consts::PI;
            }
            let faced = finish(m, ga, brute, |_| {});
            (within, turned, faced)
        })
        .await
        .unwrap();
    assert_eq!(within, (true, 99_990, 90, true), "hit within the tolerance");
    assert_eq!(turned, (true, 99_990, 100, false), "turned away");
    assert_eq!(faced, (true, 99_980, 90, true), "faced again");
    assert_eq!(failure(&mut a).await, fail::UNIT_NOT_INFRONT);
    let dead = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 1, 10.0);
            finish(m, ga, brute, |m| {
                crate::game::map::combat::deal_damage(m, ga, brute, 1_000_000)
            })
        })
        .await
        .unwrap();
    assert_eq!((dead.2, dead.3), (100, false), "nothing paid for the dead");
    assert_eq!(failure(&mut a).await, fail::TARGETS_DEAD);
}
