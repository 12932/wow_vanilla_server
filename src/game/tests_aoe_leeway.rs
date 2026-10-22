//! An area's reach past its radius: a creature's size for a player's area
//! (mangos-classic's `SpellNotifierCreatureAndPlayer`), and `[spells]`'s
//! leeways when the caster or the unit runs (mangos-classic's
//! `AOE_LEEWAY`, its caster's alone) or strafes. Frost Nova and Arcane
//! Explosion on the real world.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at};
use super::tests_p6_wspell::{BEAST, RANGE_30, face_east, go, off, put, sides, unit_at};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::creature::{Motion, Walk};
use crate::game::map::spells::{Targets, mods, targets};

const R10: u32 = 998;
/// Damage about the caster, ten yards.
const BURST: u32 = 991_700;
/// Damage at a place, ten yards.
const GROUND: u32 = 991_701;
/// A heal of friends about the caster, ten yards.
const FRIENDS: u32 = 991_702;

fn data(d: &mut GameData) {
    sides(d);
    d.spells.radii.insert(R10, 10.0);
    let spell = |id: u32, e: SpellEffect| {
        let mut s = bare(id, 0);
        s.range_index = RANGE_30;
        effect(&mut s, 0, e);
        s
    };
    let heal = SpellEffect {
        effect: 10,
        base_points: 9,
        base_dice: 1,
        target_a: 22,
        target_b: 30,
        radius_index: R10,
        ..Default::default()
    };
    let area = |a: u32, b: u32| SpellEffect {
        target_a: a,
        target_b: b,
        radius_index: R10,
        ..damage(10)
    };
    let spells: Vec<SpellTemplate> = vec![
        spell(BURST, area(22, 15)),
        spell(GROUND, area(16, 0)),
        spell(FRIENDS, heal),
    ];
    insert(d, spells);
}

/// Whether effect 0 of `spell` from `caster` at `t` lands on `g`.
fn hits(m: &Map, caster: Guid, spell: u32, t: &Targets, g: Guid) -> bool {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    targets::effect_targets(m, caster, &s, 0, t, &mods).contains(&g)
}

/// A beast `x` yards east of `from`, of combat reach `reach`.
fn beast(m: &mut Map, from: Guid, x: f32, reach: f32) -> Guid {
    let g = unit_at(m, from, BEAST, x, 0.0);
    m.unit_fields_mut(g)
        .unwrap()
        .set_f32(UNIT_FIELD_COMBATREACH, reach);
    g
}

/// `g` on a path east, at a run or a walk.
fn walks(m: &mut Map, g: Guid, run: bool) {
    let from = m.position_of(g).unwrap();
    let to = Vector3d {
        x: from.x + 30.0,
        ..from
    };
    let now = m.now;
    m.creature_mut(g).unwrap().motion = Motion::Moving(Walk::new(vec![from, to], 7.0, now, 1, run));
}

/// The player's movement flags.
fn moves(m: &mut Map, g: Guid, flags: MovementInfo_MovementFlags) {
    m.players.get_mut(&g).unwrap().movement.flags = flags;
}

fn still() -> MovementInfo_MovementFlags {
    MovementInfo_MovementFlags::empty()
}

/// The leeways set to `moving` and `strafing`.
fn leeways(m: &mut Map, moving: f32, strafing: f32) {
    let mut cfg = (*m.cfg).clone();
    cfg.spells.area_leeway_moving_yards = moving;
    cfg.spells.area_leeway_strafing_yards = strafing;
    m.cfg = Arc::new(cfg);
}

/// A player's area about them reaches a creature's combat reach past its
/// radius (ten yards and its 1.5); a creature of no size, the radius
/// alone; none past them standing. Running, the caster or the creature,
/// two yards more; walking, none; strafing, 8/3; none past those. At 0
/// the leeways are gone, the size not.
#[tokio::test]
async fn an_area_reaches_a_creatures_size_and_further_when_either_moves() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "AOEA", "Arear", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            face_east(m, a);
            let none = Targets::default();
            let at = |m: &mut Map, x: f32, reach: f32, how: &dyn Fn(&mut Map, Guid)| {
                let g = beast(m, a, x, reach);
                how(m, g);
                let hit = hits(m, a, BURST, &none, g);
                m.remove_creature(g);
                hit
            };
            let standing = |_: &mut Map, _: Guid| {};
            let running = |m: &mut Map, g: Guid| walks(m, g, true);
            let walking = |m: &mut Map, g: Guid| walks(m, g, false);
            let size = [
                at(m, 9.9, 0.0, &standing),
                at(m, 10.4, 0.0, &standing),
                at(m, 11.4, 1.5, &standing),
                at(m, 11.6, 1.5, &standing),
            ];
            let target_moves = [
                at(m, 13.4, 1.5, &running),
                at(m, 13.4, 1.5, &walking),
                at(m, 13.6, 1.5, &running),
            ];
            moves(m, a, still().set_forward());
            let caster_runs = [
                at(m, 13.4, 1.5, &standing),
                at(m, 13.6, 1.5, &standing),
                at(m, 14.0, 1.5, &standing),
            ];
            moves(m, a, still().set_backward());
            let backward = at(m, 13.4, 1.5, &standing);
            moves(m, a, still().set_forward().set_walk_mode());
            let caster_walks = at(m, 11.6, 1.5, &standing);
            moves(m, a, still().set_strafe_left());
            let strafes = [at(m, 14.1, 1.5, &standing), at(m, 14.3, 1.5, &standing)];
            moves(m, a, still().set_forward().set_strafe_right());
            let diagonal = at(m, 14.1, 1.5, &running);
            leeways(m, 0.0, 0.0);
            let off_ = [at(m, 11.4, 1.5, &standing), at(m, 11.6, 1.5, &running)];
            (
                size,
                target_moves,
                caster_runs,
                backward,
                caster_walks,
                strafes,
                diagonal,
                off_,
            )
        })
        .await
        .unwrap();
    let (size, target_moves, caster_runs, backward, caster_walks, strafes, diagonal, off_) = out;
    assert_eq!(
        size,
        [true, false, true, false],
        "in the radius; a creature of no size past it; one of 1.5 within it, past it"
    );
    assert_eq!(
        target_moves,
        [true, false, false],
        "the creature running, within two yards more; walking, not; past them, not"
    );
    assert_eq!(
        caster_runs,
        [true, false, false],
        "the caster running: two yards more, not past them"
    );
    assert!(backward, "running back is running");
    assert!(!caster_walks, "walking, no leeway");
    assert_eq!(strafes, [true, false], "strafing: 8/3, not past it");
    assert!(diagonal, "a strafe forward is a strafe");
    assert_eq!(off_, [true, false], "no leeways, the size still");
}

/// A place's area reaches a creature's size and the leeway as one about the
/// caster does; a player in a player's area gets no size (mangos-classic
/// counts it for the player-controlled at the not), but its strafe's
/// leeway; a creature's area reaches no player's size.
#[tokio::test]
async fn a_places_area_and_a_players_reach_alike_but_a_players_size() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "AOEB", "Placer", Class::Mage).await;
    let (_d, b, _) = human(&realm, "AOEC", "Friend", Class::Priest).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            face_east(m, a);
            let place = off(m, a, 20.0, 0.0);
            let there = Targets {
                dest: Some(place),
                ..Default::default()
            };
            let g = beast(m, a, 31.4, 1.5);
            let sized = hits(m, a, GROUND, &there, g);
            walks(m, g, true);
            put(m, g, off(m, a, 33.4, 0.0));
            let running = hits(m, a, GROUND, &there, g);
            put(m, g, off(m, a, 33.6, 0.0));
            let past = hits(m, a, GROUND, &there, g);
            m.remove_creature(g);
            let none = Targets::default();
            let friend = |m: &mut Map, x: f32| {
                put(m, b, off(m, a, x, 0.0));
                hits(m, a, FRIENDS, &none, b)
            };
            let player = [friend(m, 10.0), friend(m, 10.4)];
            moves(m, b, still().set_strafe_left());
            let strafing = [friend(m, 12.6), friend(m, 12.8)];
            moves(m, b, still());
            // A beast's area at a player of 1.5: the radius alone.
            let source = beast(m, a, 0.0, 1.5);
            put(m, source, off(m, b, -10.4, 0.0));
            let foe = hits(m, source, BURST, &none, b);
            (sized, running, past, player, strafing, foe)
        })
        .await
        .unwrap();
    let (sized, running, past, player, strafing, foe) = out;
    assert!(sized, "a place's area, the creature's size");
    assert!(running, "and its running");
    assert!(!past, "not past them");
    assert_eq!(player, [true, false], "a player, no size");
    assert_eq!(strafing, [true, false], "a strafing player, the leeway");
    assert!(!foe, "a creature's area, no player's size");
}

/// Through the protocol: a caster's strafe told in a heartbeat takes their
/// area to a beast 14 yards off (ten, its 1.5 and 8/3); stopped, the area
/// no longer reaches it.
#[tokio::test]
async fn a_strafe_heartbeat_widens_the_cast_its_stop_narrows_it() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "AOED", "Strafer", Class::Mage).await;
    let (g, here) = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BURST]);
            face_east(m, a);
            (beast(m, a, 14.0, 1.5), m.position_of(a).unwrap())
        })
        .await
        .unwrap();
    let beat = |flags: MovementInfo_MovementFlags| MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags,
            timestamp: 0,
            position: here,
            orientation: 0.0,
            fall_time: 0,
        },
    };
    let cast = CMSG_CAST_SPELL {
        spell: BURST,
        targets: SpellCastTargets::default(),
    };
    for (flags, expect) in [(still().set_strafe_left(), true), (still(), false)] {
        c.send(beat(flags)).await;
        harness_wait_flags(&map, a, flags).await;
        map.query(move |m| kit::fresh(m, a)).await.unwrap();
        c.clear();
        c.send(cast.clone()).await;
        let hit = go(&mut c, BURST).await;
        assert_eq!(hit.contains(&g), expect, "{flags:?}: {hit:?}");
    }
}

/// Until the map has the player's movement flags as `flags`.
async fn harness_wait_flags(
    map: &crate::game::map::MapHandle,
    g: Guid,
    flags: MovementInfo_MovementFlags,
) {
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        let now = map
            .query(move |m| m.players[&g].movement.flags)
            .await
            .unwrap();
        if now == flags {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "never {flags:?}");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// On the real world: a mage's Frost Nova and Arcane Explosion (ten yards)
/// reach a Young Wolf as far as its size past them; a little further, as
/// it runs or the mage does; further again as the mage strafes; not
/// beyond. Each cast is told to the mage's client.
#[tokio::test]
#[ignore = "extended"]
async fn frost_nova_and_arcane_explosion_reach_as_far_as_their_leeways() {
    if !kit::have_data() {
        return;
    }
    const FROST_NOVA: u32 = 122;
    const ARCANE_EXPLOSION: u32 = 1449;
    const YOUNG_WOLF: u32 = 299;
    let realm = kit::realm();
    let (mut c, mage, map) = kit::player(&realm, "AOEE", "Novaa", Race::Human, Class::Mage).await;
    let (radii, reach) = map
        .query(move |m| {
            kit::ready(m, mage, 20, &[FROST_NOVA, ARCANE_EXPLOSION]);
            face_east(m, mage);
            let data = m.realm.data.clone();
            let r = |id: u32| {
                let s = data.spells.get(id).cloned().unwrap();
                data.spells.radius(s.effects[0].radius_index)
            };
            let w = kit::spawn(m, mage, YOUNG_WOLF, 1, 5.0);
            let reach = m.unit_fields(w).unwrap().get_f32(UNIT_FIELD_COMBATREACH);
            m.remove_creature(w);
            ((r(FROST_NOVA), r(ARCANE_EXPLOSION)), reach)
        })
        .await
        .unwrap();
    assert_eq!(radii, (10.0, 10.0), "both ten yards");
    assert!(reach > 0.0, "a wolf has a size");
    let moving = targets::AREA_LEEWAY_MOVING;
    let strafing = targets::AREA_LEEWAY_STRAFING;
    // (the mage's flags, the wolf running, yards past radius and reach, hit)
    let cases = [
        (still(), false, -0.1, true),
        (still(), false, 0.1, false),
        (still(), true, moving - 0.1, true),
        (still().set_forward(), false, moving - 0.1, true),
        (still().set_forward(), false, moving + 0.1, false),
        (still().set_strafe_right(), false, strafing - 0.1, true),
        (still().set_strafe_right(), false, strafing + 0.1, false),
    ];
    for spell in [FROST_NOVA, ARCANE_EXPLOSION] {
        for (i, (flags, runs, past, expect)) in cases.into_iter().enumerate() {
            // The wolf placed, its run and the mage's flags, and the cast,
            // in one turn of the map: no update moves either between.
            c.clear();
            let (wolf, cast) = map
                .query(move |m| {
                    let w = kit::spawn(m, mage, YOUNG_WOLF, 1, 5.0);
                    put(m, w, off(m, mage, 10.0 + reach + past, 0.0));
                    if runs {
                        walks(m, w, true);
                    }
                    moves(m, mage, flags);
                    (w, kit::cast_fresh(m, mage, spell, at(mage)))
                })
                .await
                .unwrap();
            assert!(cast, "{spell} goes off ({i})");
            let hit = go(&mut c, spell).await;
            assert_eq!(hit.contains(&wolf), expect, "{spell}, case {i}: {hit:?}");
            map.query(move |m| {
                m.remove_creature(wolf);
                moves(m, mage, still());
            })
            .await
            .unwrap();
        }
    }
}
