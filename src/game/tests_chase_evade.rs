//! A creature's chase in melee, as mangos-classic's `ChaseMovementGenerator`
//! has it: it holds still in reach, paths again only when its victim has
//! gone out of reach of where it is headed, and sends no move to turn (the
//! client turns a creature to its target); and a creature on its way home,
//! whose blows and spells taken evade (mangos's `VICTIMSTATE_EVADES` and
//! `SPELL_MISS_EVADE`), not "Invalid target".

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit as kit;
use super::tests_p6_wspell::{BEAST, face_east, put, sides, unit_at};
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::ai;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapHandle};
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;

const SMSG_MONSTER_MOVE: u16 = 0x0DD;

fn data(d: &mut GameData) {
    sides(d);
    if let Some(t) = d.creature_templates.get_mut(&BEAST) {
        t.base_attack_time = 2000;
    }
}

/// The moves of `g` a client has had since the last look: (paths, stops,
/// turns).
fn moves_of(c: &mut Client, g: Guid) -> (usize, usize, usize) {
    let mut out = (0, 0, 0);
    for b in c.take_frames(SMSG_MONSTER_MOVE) {
        let (of, n) = packed(&b);
        if of != g {
            continue;
        }
        let f = |i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let start = (f(n), f(n + 4), f(n + 8));
        let kind = b[n + 16];
        if kind == 1 {
            out.1 += 1;
            continue;
        }
        let at = n + 17 + [0, 0, 12, 8, 4][usize::from(kind).min(4)];
        let count = u32::from_le_bytes(b[at + 8..at + 12].try_into().unwrap());
        let end = (f(at + 12), f(at + 16), f(at + 20));
        let still = (end.0 - start.0).abs() < 0.01 && (end.1 - start.1).abs() < 0.01;
        if count == 1 && still {
            out.2 += 1;
        } else {
            out.0 += 1;
        }
    }
    out
}

fn packed(b: &[u8]) -> (Guid, usize) {
    let mask = b[0];
    let mut v = 0u64;
    let mut n = 1;
    for i in 0..8 {
        if mask & (1 << i) != 0 {
            v |= u64::from(b[n]) << (i * 8);
            n += 1;
        }
    }
    (Guid::new(v), n)
}

/// `secs` of the map's creatures at 20 Hz, the player moved by `step` each
/// update.
fn fight_for(m: &mut Map, p: Guid, secs: f32, step: impl Fn(&mut Map, Guid, usize)) {
    let ticks = (secs * 20.0) as usize;
    for i in 0..ticks {
        m.now += Duration::from_millis(50);
        step(m, p, i);
        ai::update_creatures(m);
    }
}

/// A beast fighting the player, come at them from `yards` east, seen.
async fn fighting(account: &str, name: &str, yards: f32) -> (Client, Guid, Guid, MapHandle) {
    let realm = fast_realm(data);
    let (mut c, p, map) = human(&realm, account, name, Class::Warrior).await;
    let beast = map
        .query(move |m| {
            kit::ready(m, p, 60, &[]);
            face_east(m, p);
            unit_at(m, p, BEAST, yards, 0.0)
        })
        .await
        .unwrap();
    c.sees(beast).await;
    map.query(move |m| {
        ai::attacked(m, beast, p);
        fight_for(m, p, 3.0, |_, _, _| {});
    })
    .await
    .unwrap();
    (c, p, beast, map)
}

async fn settle(c: &mut Client, map: &MapHandle) {
    crate::game::harness::an_update(map).await;
    crate::game::harness::an_update(map).await;
    c.clear();
}

/// Come in from afar, and standing in reach of a victim who stands, or who
/// walks about a yard each way, a creature sends no move between its
/// bites: no path, no stop, no turn. (Aimed half a yard inside its reach,
/// it sent a path as soon as its victim stepped back off the reach's edge,
/// and a turn at a bite they had moved for: one of each here.)
#[tokio::test]
async fn in_reach_a_creature_sends_no_move_between_bites() {
    let (mut c, p, beast, map) = fighting("CHASEA", "Chasea", 15.0).await;
    settle(&mut c, &map).await;
    map.query(move |m| fight_for(m, p, 5.0, |_, _, _| {}))
        .await
        .unwrap();
    crate::game::harness::an_update(&map).await;
    let standing = moves_of(&mut c, beast);
    // The victim walks about: a yard away and back, side to side, slowly.
    map.query(move |m| {
        let o = m.position_of(p).unwrap();
        fight_for(m, p, 6.0, move |m, p, i| {
            let t = i as f32 / 20.0;
            let at = Vector3d {
                x: o.x - 0.8 * (t * 0.8).sin().abs(),
                y: o.y + 1.0 * (t * 0.6).sin(),
                z: o.z,
            };
            put(m, p, at);
            m.players.get_mut(&p).unwrap().movement.flags = MovementInfo_MovementFlags::empty()
                .set_forward()
                .set_walk_mode();
        });
        m.players.get_mut(&p).unwrap().movement.flags = MovementInfo_MovementFlags::empty();
    })
    .await
    .unwrap();
    crate::game::harness::an_update(&map).await;
    let walking = moves_of(&mut c, beast);
    eprintln!("standing {standing:?} walking {walking:?}");
    assert_eq!(standing, (0, 0, 0), "a victim standing: no move");
    assert_eq!(
        walking,
        (0, 0, 0),
        "a victim walking about in its reach: no move"
    );
}

/// A victim walking slowly away is followed with a path each time they
/// leave the reach of where it is headed: over six seconds at a walk, five
/// paths (aimed at its reach's edge, it sent 21, one for every 0.7 yards).
#[tokio::test]
async fn a_creature_paths_again_only_when_its_victim_leaves_its_reach() {
    let (mut c, p, beast, map) = fighting("CHASEB", "Chaseb", 3.0).await;
    settle(&mut c, &map).await;
    map.query(move |m| fight_for(m, p, 2.0, |_, _, _| {}))
        .await
        .unwrap();
    crate::game::harness::an_update(&map).await;
    c.clear();
    let out = map
        .query(move |m| {
            let o = m.position_of(p).unwrap();
            fight_for(m, p, 6.0, move |m, p, i| {
                let t = (i + 1) as f32 / 20.0;
                put(
                    m,
                    p,
                    Vector3d {
                        x: o.x - 2.5 * t,
                        ..o
                    },
                );
                m.players.get_mut(&p).unwrap().movement.flags = MovementInfo_MovementFlags::empty()
                    .set_backward()
                    .set_walk_mode();
            });
            let (b, v) = (m.unit_view(beast).unwrap(), m.unit_view(p).unwrap());
            crate::game::map::combat::in_melee_range(&b, &v)
        })
        .await
        .unwrap();
    crate::game::harness::an_update(&map).await;
    let (paths, stops, turns) = moves_of(&mut c, beast);
    eprintln!("away: paths {paths} stops {stops} turns {turns}");
    assert!(out, "it keeps up");
    assert!((1..=6).contains(&paths), "a path at each leaving: {paths}");
    assert_eq!(turns, 0, "no turns");
}

/// Fireball (rank 1), as made up here: instant, fire, at an enemy.
const FIREBALL: u32 = 133;

fn evade_data(d: &mut GameData) {
    data(d);
    let mut s = bare(FIREBALL, 2);
    s.name = "Fireball".into();
    s.range_index = super::tests_p6_wspell::RANGE_30;
    s.speed = 24.0;
    effect(&mut s, 0, damage(14));
    insert(d, [s]);
}

/// A mage, and a beast that fought them on its slow way home from three
/// yards east of them, seen.
async fn evading(account: &str, name: &str) -> (Client, Guid, Guid, MapHandle) {
    let realm = fast_realm(evade_data);
    let (mut c, p, map) = human(&realm, account, name, Class::Mage).await;
    let beast = map
        .query(move |m| {
            kit::ready(m, p, 60, &[FIREBALL]);
            face_east(m, p);
            unit_at(m, p, BEAST, 3.0, 0.0)
        })
        .await
        .unwrap();
    c.sees(beast).await;
    let evading = map
        .query(move |m| {
            ai::attacked(m, beast, p);
            let c = m.creature_mut(beast).unwrap();
            c.home = Vector3d {
                x: c.position.x + 30.0,
                ..c.position
            };
            c.speeds.run = 0.2;
            c.speeds.walk = 0.2;
            ai::enter_evade(m, beast);
            m.creature(beast).unwrap().evading
        })
        .await
        .unwrap();
    assert!(evading, "on its way home");
    crate::game::harness::an_update(&map).await;
    c.clear();
    (c, p, beast, map)
}

/// Whether `beast` is still on its way home, unhurt, and in no fight.
async fn untouched(map: &MapHandle, beast: Guid) -> (bool, bool, bool) {
    map.query(move |m| {
        let c = m.creature(beast).unwrap();
        (
            c.evading,
            c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            c.threat.is_empty() && !c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
        )
    })
    .await
    .unwrap()
}

/// A Fireball at a creature on its way home isn't refused ("Invalid
/// target"): it goes off, and misses as an evade (mangos-classic's
/// `SPELL_MISS_EVADE`, 6, in the go), and the creature takes no harm and
/// no fight from it.
#[tokio::test]
async fn a_fireball_at_an_evading_creature_evades() {
    let (mut c, _p, beast, map) = evading("EVADEA", "Evadea").await;
    c.send(CMSG_CAST_SPELL {
        spell: FIREBALL,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty()
                .set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Unit { unit_target: beast }),
        },
    })
    .await;
    let refused = c
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_CAST_RESULT(r)
                if r.spell == FIREBALL
                    && !matches!(r.result, SMSG_CAST_RESULT_SimpleSpellCastResult::Success) =>
            {
                Some(format!("{r:?}"))
            }
            _ => None,
        })
        .await;
    assert_eq!(refused, None, "not refused");
    assert_eq!(c.go_miss(FIREBALL).await, (beast, 6), "an evade, in the go");
    crate::game::harness::an_update(&map).await;
    assert_eq!(
        untouched(&map, beast).await,
        (true, true, true),
        "on its way home still, unhurt, in no fight"
    );
}

/// A swing at a creature on its way home starts (not "You cannot attack
/// that target"), and each blow evades: `SMSG_ATTACKERSTATEUPDATE` a miss
/// of no sound, no damage, the victim state 6 (mangos-classic's
/// `MELEE_HIT_EVADE`, `VICTIMSTATE_EVADES`); the creature takes no harm
/// and no fight from it.
#[tokio::test]
async fn a_swing_at_an_evading_creature_evades() {
    let (mut c, p, beast, map) = evading("EVADEB", "Evadeb").await;
    c.send(CMSG_ATTACKSWING { guid: beast }).await;
    let blow = c
        .wait_frame(
            "the blow",
            crate::game::map::combat::SMSG_ATTACKERSTATEUPDATE,
            |b| {
                let (attacker, n) = packed(&b[4..]);
                attacker == p && packed(&b[4 + n..]).0 == beast
            },
        )
        .await;
    let hit_info = u32::from_le_bytes(blow[..4].try_into().unwrap());
    let (_, a) = packed(&blow[4..]);
    let (_, t) = packed(&blow[4 + a..]);
    let at = 4 + a + t;
    let total = u32::from_le_bytes(blow[at..at + 4].try_into().unwrap());
    let lines = usize::from(blow[at + 4]);
    let state_at = at + 5 + lines * 20;
    let state = u32::from_le_bytes(blow[state_at..state_at + 4].try_into().unwrap());
    assert_eq!(state, 6, "the victim evades");
    assert_eq!(total, 0, "no damage");
    assert_eq!(hit_info, 0x2 | 0x10 | 0x8_0000, "a miss of no sound");
    let started = c
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::SMSG_ATTACKSWING_CANT_ATTACK => Some(()),
            _ => None,
        })
        .await;
    assert_eq!(started, None, "no refusal");
    crate::game::harness::an_update(&map).await;
    assert_eq!(
        untouched(&map, beast).await,
        (true, true, true),
        "on its way home still, unhurt, in no fight"
    );
}

/// The real Fireball (rank 1), its cast and its flight, at a Kobold Vermin
/// on its way home: begun and gone off, not refused, and an evade.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_fireball_at_an_evading_kobold_evades() {
    let realm = kit::realm();
    let (mut c, p, map) = kit::player(&realm, "EVADEC", "Evadec", Race::Human, Class::Mage).await;
    // Her intro's camera over, she sees from where she stands.
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let kobold = map
        .query(move |m| {
            kit::ready(m, p, 60, &[FIREBALL]);
            face_east(m, p);
            let k = kit::dummy(m, p, 10, 1.0);
            put(m, k, super::tests_p6_wspell::off(m, p, 12.0, 0.0));
            k
        })
        .await
        .unwrap();
    c.sees_on(&map, kobold).await;
    map.query(move |m| {
        ai::attacked(m, kobold, p);
        let c = m.creature_mut(kobold).unwrap();
        c.home = Vector3d {
            x: c.position.x + 30.0,
            ..c.position
        };
        c.speeds.run = 0.2;
        c.speeds.walk = 0.2;
        ai::enter_evade(m, kobold);
    })
    .await
    .unwrap();
    crate::game::harness::an_update(&map).await;
    c.clear();
    c.send(CMSG_CAST_SPELL {
        spell: FIREBALL,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty().set_unit(
                SpellCastTargets_SpellCastTargetFlags_Unit::Unit {
                    unit_target: kobold,
                },
            ),
        },
    })
    .await;
    c.wait_for("its start", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_START(s) if s.spell == FIREBALL => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(
        c.go_miss(FIREBALL).await,
        (kobold, 6),
        "an evade, in the go"
    );
    assert_eq!(
        untouched(&map, kobold).await,
        (true, true, true),
        "on its way home still, unhurt, in no fight"
    );
}
