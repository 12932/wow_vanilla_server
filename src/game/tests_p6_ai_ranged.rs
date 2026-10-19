//! Round 6, against Thistle Tea's `bt/ranged_test.exs` and `wand_test.exs`
//! (docs/parity/matrix.csv): what each repeating shot meets again before
//! it goes (its shooter's life and control, its range with its modifiers
//! and dead zone, its target in sight), and a wand's hit and damage.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, AURA_ADD_FLAT_MODIFIER};
use crate::game::map::spells::{Targets, mods};
use crate::game::map::{Map, MapHandle, combat, ranged};
use crate::game::tests_p3_combat::{BRUTE, aura_of, equip, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_player2_ammo::{
    self as ammo, ARROW, AUTO_SHOT, BOW, choose, count, fire, repeating, shots,
};

/// A hunter's spell family, and Hawk Eye's six yards more for its shots.
const HUNTER: u32 = 9;
const HAWK_EYE: u32 = 96_400;
/// Eight to thirty-five yards.
const DEAD_ZONED: u32 = 960;

fn world() -> GameData {
    let mut d = ammo::world();
    d.spells.ranges.insert(DEAD_ZONED, (8.0, 35.0));
    let shot = d.spells.spells.get_mut(&AUTO_SHOT).unwrap();
    shot.range_index = DEAD_ZONED;
    shot.family_name = HUNTER;
    shot.family_flags = 0x1;
    // Pacify's kind (the client's own Auto Shot's).
    shot.prevention_type = crate::game::map::spells::checks::PREVENTION_PACIFY;
    let mut hawk = SpellTemplate {
        id: HAWK_EYE,
        name: "Hawk Eye".into(),
        family_name: HUNTER,
        equipped_item_class: -1,
        ..Default::default()
    };
    hawk.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_ADD_FLAT_MODIFIER,
        misc_value: mods::OP_RANGE as i32,
        base_points: 5,
        base_dice: 1,
        item_type: 0x1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(HAWK_EYE, hawk);
    d
}

/// Alice with a bow and plenty of arrows, sure to hit, Auto Shot started
/// at a brute `yards` off (beyond both their reaches).
async fn shooting(
    yards: f32,
) -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
) {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let brute = on(&map, move |m| {
        equip(m, ga, BOW, 17);
        crate::game::map::inventory::add_item(m, ga, ARROW, 50, false).unwrap();
        let brute = spawn(m, ga, BRUTE, 20, 10.0);
        let c = m.creature_mut(brute).unwrap();
        c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 0.0);
        c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 0.0);
        place(m, ga, brute, yards);
        brute
    })
    .await;
    choose(&mut a, ARROW).await;
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().derived.ranged_hit = 100.0;
        ranged::start(
            m,
            ga,
            AUTO_SHOT,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
        );
    })
    .await;
    (realm, a, ga, map, brute)
}

/// `g` put `yards` beyond both reaches in front of `from`, facing it.
fn place(m: &mut Map, from: Guid, g: Guid, yards: f32) {
    let me = m.unit_view(from).unwrap();
    let reaches = me.combat_reach + m.unit_view(g).unwrap().combat_reach;
    let d = yards + reaches;
    let at = Vector3d {
        x: me.position.x + d * me.orientation.cos(),
        y: me.position.y + d * me.orientation.sin(),
        z: me.position.z,
    };
    if let Some(c) = m.creature_mut(g) {
        c.position = at;
        c.orientation = me.orientation + std::f32::consts::PI;
    }
}

/// A spell of `kind` (a test's own) on `target` from `caster`.
fn aura_from(m: &mut Map, caster: Guid, target: Guid, id: u32, kind: u32, amount: i32) {
    let mut s = SpellTemplate {
        id,
        name: format!("Test Aura {id}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    let _ = auras::apply_spell(m, caster, target, &s, &[(0, amount)], Default::default());
}

/// The repeat's next shot made due and tried: whether it still repeats,
/// and the arrows left.
fn try_shot(m: &mut Map, g: Guid) -> (bool, u32) {
    fire(m, g);
    (repeating(m, g), count(m, g, ARROW))
}

async fn cancelled(c: &mut Client) {
    c.wait_for("SMSG_CANCEL_AUTO_REPEAT", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CANCEL_AUTO_REPEAT).then_some(())
    })
    .await;
}

/// Thistle Tea's "does not remain active after death" and "pacify cancels
/// ranged attacks without firing or consuming ammunition": a shooter
/// pacified (or pacified and silenced) stops at its next shot with none
/// loosed and no arrow spent, as one dead does; a silence, which Auto Shot
/// isn't stopped by, lets it shoot (mangos's `_UpdateAutoRepeatSpell`,
/// each repeat's cast checked again and interrupted on its failure).
#[tokio::test]
async fn a_shooter_dead_or_pacified_stops_without_a_shot_and_a_silence_doesnt_stop_it() {
    use crate::game::map::auras::{AURA_MOD_PACIFY, AURA_MOD_PACIFY_SILENCE, AURA_MOD_SILENCE};
    let (_realm, mut a, ga, map, brute) = shooting(20.0).await;
    let silenced = on(&map, move |m| {
        aura_of(m, ga, 96_401, AURA_MOD_SILENCE, 0, 1);
        let r = try_shot(m, ga);
        auras::remove_spell(m, ga, 96_401, None);
        r
    })
    .await;
    assert_eq!(silenced, (true, 49), "silenced, it shoots");
    assert_eq!(shots(&mut a, AUTO_SHOT).await, 1);
    for (id, kind) in [(96_402, AURA_MOD_PACIFY), (96_403, AURA_MOD_PACIFY_SILENCE)] {
        let r = on(&map, move |m| {
            aura_of(m, ga, id, kind, 0, 1);
            let r = try_shot(m, ga);
            auras::remove_spell(m, ga, id, None);
            ranged::start(
                m,
                ga,
                AUTO_SHOT,
                Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
            );
            r
        })
        .await;
        assert_eq!(r, (false, 49), "aura {kind}: stopped, no arrow spent");
        cancelled(&mut a).await;
        assert_eq!(shots(&mut a, AUTO_SHOT).await, 0, "aura {kind}: no shot");
    }
    let dead = on(&map, move |m| {
        combat::kill(m, brute, ga);
        try_shot(m, ga)
    })
    .await;
    assert_eq!(dead, (false, 49), "dead, it stops");
    assert_eq!(shots(&mut a, AUTO_SHOT).await, 0, "the dead shoot nothing");
}

/// Thistle Tea's "Hawk Eye extends repeat shots and removal restores their
/// range" and "measures the dead zone from the edges of both combat
/// reaches": each shot is held to the range its cast would be, Hawk Eye's
/// six yards with it, and its end brings the range back, the repeat
/// stopping at a target beyond it; a target inside the eight yards of the
/// dead zone beyond both reaches (though further than eight from the
/// centre) stops it too (mangos's `GetMinMaxRange` through each repeat's
/// `CheckRange`).
#[tokio::test]
async fn hawk_eye_reaches_each_shot_further_and_a_target_beyond_or_in_the_dead_zone_stops_it() {
    let (_realm, mut a, ga, map, brute) = shooting(38.0).await;
    let far = on(&map, move |m| {
        crate::game::tests_p4_core_wand::put(m, ga, HAWK_EYE);
        let with = try_shot(m, ga);
        auras::remove_spell(m, ga, HAWK_EYE, None);
        let without = try_shot(m, ga);
        (with, without)
    })
    .await;
    assert_eq!(far.0, (true, 49), "Hawk Eye: in range at 38 yards");
    assert_eq!(far.1, (false, 49), "without it: stopped");
    assert_eq!(shots(&mut a, AUTO_SHOT).await, 1);
    cancelled(&mut a).await;
    let close = on(&map, move |m| {
        place(m, ga, brute, 7.0);
        ranged::start(
            m,
            ga,
            AUTO_SHOT,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
        );
        let centre = combat::dist(
            m.players[&ga].position(),
            m.creature(brute).unwrap().position,
        );
        (centre > 8.0, try_shot(m, ga))
    })
    .await;
    assert_eq!(close, (true, (false, 49)), "in the dead zone: stopped");
    cancelled(&mut a).await;
    assert_eq!(shots(&mut a, AUTO_SHOT).await, 0);
}

/// Thistle Tea's "stops shooting concealed targets and respects detection
/// and caster-specific marks" and the wand's "lost sight ... clear the
/// repeat": a player's side gone into stealth stops the repeat, unless
/// the shooter detects it or marked it (mangos's `CheckCast`, of a target
/// `UNIT_FLAG_PLAYER_CONTROLLED` invisible to the caster); a wall between
/// stops Auto Shot and a wand alike.
#[tokio::test]
async fn a_repeat_stops_at_a_player_side_target_gone_into_stealth_or_out_of_sight() {
    use crate::game::map::auras::{AURA_MOD_STALKED, AURA_MOD_STEALTH, AURA_MOD_STEALTH_DETECT};
    let (_realm, mut a, ga, map, brute) = shooting(15.0).await;
    let restart = move |m: &mut Map| {
        ranged::start(
            m,
            ga,
            AUTO_SHOT,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
        );
    };
    let r = on(&map, move |m| {
        // Another player's (a pet), stealthed past the shooter's eyes.
        m.creature_mut(brute).unwrap().owner = Some(Guid::new(0x7777));
        aura_of(m, brute, 96_410, AURA_MOD_STEALTH, 0, 1_000);
        let hidden = try_shot(m, ga);
        // Detected.
        restart(m);
        aura_of(m, ga, 96_411, AURA_MOD_STEALTH_DETECT, 0, 1_000);
        let detected = try_shot(m, ga);
        auras::remove_spell(m, ga, 96_411, None);
        // Marked by the shooter.
        aura_from(m, ga, brute, 96_412, AURA_MOD_STALKED, 1);
        let marked = try_shot(m, ga);
        auras::remove_spell(m, brute, 96_412, None);
        auras::remove_spell(m, brute, 96_410, None);
        // A wall between.
        let (p, b) = (
            m.players[&ga].position(),
            m.creature(brute).unwrap().position,
        );
        let mid = Vector3d {
            x: (p.x + b.x) / 2.0,
            y: (p.y + b.y) / 2.0,
            z: p.z,
        };
        let side = |d: f32| Vector3d {
            x: mid.x - (b.y - p.y) / 15.0 * d,
            y: mid.y + (b.x - p.x) / 15.0 * d,
            z: mid.z,
        };
        m.spells.walls.push((side(-5.0), side(5.0)));
        let walled = try_shot(m, ga);
        (hidden, detected, marked, walled)
    })
    .await;
    assert_eq!(r.0, (false, 50), "unseen: stopped");
    assert_eq!(r.1, (true, 49), "detected: shot");
    assert_eq!(r.2, (true, 48), "marked: shot");
    assert_eq!(r.3, (false, 48), "out of sight: stopped");
    assert_eq!(shots(&mut a, AUTO_SHOT).await, 2);
    // A wand, out of sight, the same.
    let (_realm, _w, gw, wmap, wbrute) = crate::game::tests_p4_core_wand::alice().await;
    let wand = on(&wmap, move |m| {
        let (p, b) = (
            m.players[&gw].position(),
            m.creature(wbrute).unwrap().position,
        );
        let mid = Vector3d {
            x: (p.x + b.x) / 2.0,
            y: (p.y + b.y) / 2.0,
            z: p.z,
        };
        m.spells.walls.push((
            Vector3d {
                y: mid.y - 5.0,
                ..mid
            },
            Vector3d {
                y: mid.y + 5.0,
                ..mid
            },
        ));
        let dealt = crate::game::tests_p4_core_wand::shot(m, gw, wbrute);
        (dealt, m.combat(gw).unwrap().auto_repeat.is_some())
    })
    .await;
    assert_eq!(wand, (0, false), "a wand out of sight: stopped unshot");
}

/// Thistle Tea's "uses ranged hit modifiers and excludes Hunter's Mark
/// attack power": a wand's shot misses by the ranged hit table (its
/// shooter's hit chance taken off, it misses now and then), and a target's
/// Hunter's Mark adds nothing to it, a wand having no attack power.
#[tokio::test]
async fn a_wand_misses_by_ranged_hit_and_hunters_mark_adds_nothing() {
    use crate::game::map::auras::{AURA_MOD_HIT_CHANCE, AURA_RANGED_ATTACK_POWER_ATTACKER_BONUS};
    let (_realm, _a, g, map, brute) = crate::game::tests_p4_core_wand::alice().await;
    let r = on(&map, move |m| {
        let fresh = |m: &mut Map| {
            ranged::cancel(m, g, false);
            m.combat_mut(g).unwrap().ranged_ready_at = None;
        };
        let plain = crate::game::tests_p4_core_wand::shot(m, g, brute);
        fresh(m);
        aura_from(
            m,
            g,
            brute,
            96_420,
            AURA_RANGED_ATTACK_POWER_ATTACKER_BONUS,
            1_400,
        );
        let marked = crate::game::tests_p4_core_wand::shot(m, g, brute);
        fresh(m);
        // Two hundred off its sure hundred: it misses, now and then.
        aura_of(m, g, 96_421, AURA_MOD_HIT_CHANCE, 0, -200);
        crate::game::map::derive::unit(m, g);
        let mut missed = 0;
        for _ in 0..30 {
            if crate::game::tests_p4_core_wand::shot(m, g, brute) == 0 {
                missed += 1;
            }
            fresh(m);
        }
        (plain, marked, missed, m.players[&g].derived.ranged_hit)
    })
    .await;
    assert_eq!((r.0, r.1), (100, 100), "the mark adds nothing to a wand");
    assert!(r.2 > 0, "missed {} of 30 (ranged hit {})", r.2, r.3);
    let _ = NONE_WITHIN;
}
