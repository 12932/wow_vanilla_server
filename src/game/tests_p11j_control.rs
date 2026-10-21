//! Round 11j's control tests: forced movement's counters, the active mover,
//! a possessed unit's launch, and a charmed player's casts (Thistle Tea's
//! `movement_control_test`, `mover_test`, `knockback_test`,
//! `possession_owner_test` and `spellcasting_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::map::auras::{self, AURA_HOVER, AURA_MOD_ROOT, AURA_WATER_WALK};
use crate::game::tests_p3_entity::{aura_effect, on, spell};

const ROOT: u32 = 97_710;
const WATER_WALK: u32 = 97_711;
const HOVER: u32 = 97_712;
const KNOCK: u32 = 97_713;
const MIND_CONTROL: u32 = crate::game::tests_pets2::MIND_CONTROL;

/// Thistle Tea's "also advances counters for movement flags without
/// implemented acknowledgements": a hover takes the client's next order
/// counter as a root, unroot and water walk do, one more each
/// (`Outbound::order`; wvs settles hover's acknowledgement too).
#[tokio::test]
async fn a_hover_takes_the_next_order_counter_as_every_forced_movement_does() {
    let mut d = crate::game::tests_p3_combat::data();
    for (id, aura) in [
        (ROOT, AURA_MOD_ROOT),
        (WATER_WALK, AURA_WATER_WALK),
        (HOVER, AURA_HOVER),
    ] {
        d.spells
            .spells
            .insert(id, spell(id, &[aura_effect(aura, 1, 0, 1)]));
    }
    let (_realm, mut a, g, map) = crate::game::tests_p3_combat::setup_with(d, Class::Warrior).await;
    a.clear();
    on(&map, move |m| {
        let s = |m: &crate::game::map::Map, id| m.realm.data.spells.get(id).cloned().unwrap();
        let (root, walk, hover) = (s(m, ROOT), s(m, WATER_WALK), s(m, HOVER));
        auras::apply(m, g, g, &root, 0, 0);
        auras::remove_spell(m, g, ROOT, None);
        auras::apply(m, g, g, &walk, 0, 0);
        auras::apply(m, g, g, &hover, 0, 0);
    })
    .await;
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(
            a.wait_for("a forced movement", |m| match m {
                ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) if r.guid == g => {
                    Some(("root", r.counter))
                }
                ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(r) if r.guid == g => {
                    Some(("unroot", r.counter))
                }
                ServerOpcodeMessage::SMSG_MOVE_WATER_WALK(r) if r.guid == g => {
                    Some(("water walk", r.counter))
                }
                ServerOpcodeMessage::SMSG_MOVE_SET_HOVER(r) if r.guid == g => {
                    Some(("hover", r.counter))
                }
                _ => None,
            })
            .await,
        );
    }
    let first = seen[0].1;
    assert_eq!(
        seen,
        [
            ("root", first),
            ("unroot", first + 1),
            ("water walk", first + 2),
            ("hover", first + 3)
        ]
    );
    // Its acknowledgement settles it.
    a.send(CMSG_MOVE_HOVER_ACK {
        guid: g,
        counter: first + 3,
        info: MovementInfo::default(),
        is_applied: 1,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let left = on(&map, move |m| {
        m.players[&g]
            .outbound
            .pending_orders()
            .iter()
            .any(|(c, _)| *c == first + 3)
    })
    .await;
    assert!(!left, "the hover is settled");
}

fn mind_control_world() -> crate::game::data::GameData {
    let mut d = crate::game::tests_pets2::more();
    d.spells.spells.insert(
        KNOCK,
        spell(
            KNOCK,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_KNOCK_BACK,
                base_points: 69,
                base_dice: 1,
                misc_value: 120,
                target_a: 1,
                ..Default::default()
            }],
        ),
    );
    d
}

async fn mover_of(map: &MapHandle, g: Guid) -> Option<Guid> {
    on(map, move |m| m.players[&g].owned.mover).await
}

/// The launch a client is asked to acknowledge of `unit`: its counter and
/// impulse.
async fn launch_of(map: &MapHandle, to: Guid, unit: Guid) -> (u32, [f32; 4]) {
    use crate::game::conn::Order;
    on(map, move |m| {
        crate::game::map::spells::cast_triggered(m, unit, KNOCK, Some(unit));
        m.players[&to]
            .outbound
            .pending_orders()
            .into_iter()
            .find_map(|(c, o)| match o {
                Order::KnockBack { guid, impulse } if guid == unit => Some((c, impulse)),
                _ => None,
            })
            .expect("the launch waits on the controller's client")
    })
    .await
}

fn knock_ack(
    unit: Guid,
    counter: u32,
    impulse: [f32; 4],
    at: Vector3d,
) -> CMSG_MOVE_KNOCK_BACK_ACK {
    CMSG_MOVE_KNOCK_BACK_ACK {
        guid: unit,
        counter,
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
                cos_angle: impulse[0],
                sin_angle: impulse[1],
                xy_speed: impulse[2],
                z_speed: impulse[3],
            }),
            position: at,
            ..Default::default()
        },
    }
}

/// Thistle Tea's "acknowledgements cannot select a unit outside current
/// control" and "routes a possessed mover and discards its reply after
/// control ends": a priest's client can't make a player it doesn't
/// control its mover, nor itself while it controls her (its own guid is
/// no mover: it moves itself); the possessed player's own client can't
/// either. A launch of the possessed player is the priest's client's to
/// acknowledge, and moves her; one acknowledged after Mind Control ends
/// is settled, and moves nothing (mangos's `HandleMoveKnockBackAck` of
/// `GetClientControlling`'s mover).
#[tokio::test]
async fn only_a_unit_under_control_is_a_mover_and_its_launch_is_its_controllers() {
    use crate::game::tests_pets::{cast, enter, learn};
    use crate::game::tests_pets2::{bring, control_update, flag_pvp};
    let realm = crate::game::harness::realm_with(mind_control_world());
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    learn(&map, ga, &[MIND_CONTROL]);
    bring(&map, gb, ga, 8.0).await;
    a.sees(gb).await;
    flag_pvp(&map, vec![ga, gb]);
    // Not under control: no mover.
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(mover_of(&map, ga).await, None, "she isn't his");
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(control_update(&mut a, gb).await, "the priest takes hold");
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(mover_of(&map, ga).await, Some(gb));
    a.send(CMSG_SET_ACTIVE_MOVER { guid: ga }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(mover_of(&map, ga).await, None, "his own: he moves himself");
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(mover_of(&map, ga).await, Some(gb));
    // Her own client selects nothing.
    b.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut b).await;
    assert_eq!(mover_of(&map, gb).await, None);
    // A launch of hers is his to acknowledge, and moves her.
    let here = on(&map, move |m| m.players[&gb].position()).await;
    let (counter, impulse) = launch_of(&map, ga, gb).await;
    let away = Vector3d {
        x: here.x + 2.0,
        ..here
    };
    a.send(knock_ack(gb, counter, impulse, away)).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let (moved, pending) = on(&map, move |m| {
        (
            m.players[&gb].position(),
            m.players[&ga].outbound.pending_orders().len(),
        )
    })
    .await;
    assert!((moved.x - away.x).abs() < 0.01, "{moved:?}");
    assert_eq!(pending, 0);
    // Another, acknowledged once control has ended: settled, nowhere.
    let (counter, impulse) = launch_of(&map, ga, gb).await;
    on(&map, move |m| {
        auras::remove_spell(m, gb, MIND_CONTROL, None)
    })
    .await;
    assert!(control_update(&mut a, ga).await, "the priest has his own");
    let further = Vector3d {
        x: here.x + 6.0,
        ..here
    };
    a.send(knock_ack(gb, counter, impulse, further)).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let (stayed, pending) = on(&map, move |m| {
        (
            m.players[&gb].position(),
            m.players[&ga]
                .outbound
                .pending_orders()
                .iter()
                .any(|(c, _)| *c == counter),
        )
    })
    .await;
    assert!((stayed.x - further.x).abs() > 1.0, "{stayed:?}");
    assert!(!pending, "settled");
}

/// The charmed mage's bolt, and the wolf that charms her.
const CHARM: u32 = 9060;
const BOLT: u32 = 990_890;
const BLIZZARD: u32 = 990_891;

fn charm_world(ground: bool) -> crate::game::data::GameData {
    use crate::game::tests_pets;
    let mut d = crate::game::tests_pets2::more();
    let none = SpellEffect::default();
    let mut s = if ground {
        // At a place among the foes (`TARGET_AREAEFFECT_INSTANT` at the
        // destination), as a Blizzard is.
        let mut e = tests_pets::effect(2, 9, 16);
        e.target_b = 0;
        let mut s = tests_pets::spell(BLIZZARD, "Blizzard", [e, none, none]);
        // TARGET_FLAG_DEST_LOCATION.
        s.targets = 0x40;
        s
    } else {
        tests_pets::spell(BOLT, "Fireball", [tests_pets::effect(2, 9, 6), none, none])
    };
    s.family_name = 3;
    s.spell_level = 1;
    s.mana_cost = 10;
    s.school = 2;
    d.spells.spells.insert(s.id, s);
    d
}

async fn charmed(
    ground: bool,
) -> (
    std::sync::Arc<crate::game::Realm>,
    (Client, Client),
    Guid,
    Guid,
    Guid,
    MapHandle,
) {
    use crate::game::tests_pets::{self, enter, place};
    use crate::game::tests_pets2::bring;
    let realm = crate::game::harness::realm_with(charm_world(ground));
    let (a, ga, map) = enter(&realm, "A", "Mage", Race::Human, Class::Mage).await;
    let (b, gb, _) = enter(&realm, "B", "Foe", Race::Human, Class::Warrior).await;
    bring(&map, gb, ga, 15.0).await;
    let wolf = place(&map, ga, tests_pets::WOLF, 0x70_0090, 6.0).await;
    let spell = if ground { BLIZZARD } else { BOLT };
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().spells.push(spell);
        crate::game::map::ai::attacked(m, wolf, gb);
        crate::game::map::spells::cast_triggered(m, wolf, CHARM, Some(ga));
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
        p.fields.set_u32(UNIT_FIELD_POWER1, 1000);
    })
    .await;
    (realm, (a, b), ga, gb, wolf, map)
}

/// A spell start of `caster`'s within `within`: its spell.
async fn spell_start(a: &mut Client, caster: Guid, within: Duration) -> Option<u32> {
    a.gets(within, |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_START(s) if s.caster == caster => Some(s.spell),
        _ => None,
    })
    .await
}

/// Thistle Tea's "charm casts route to the explicit owner and stale
/// control cannot cast": a mage a wolf charms casts her own bolt at its
/// foe; the charm ended (its fight over), she casts nothing more of
/// hers for it (`possess::fight` only while `possessed_by` is set).
#[tokio::test]
async fn a_charmed_mage_casts_for_her_charmer_and_not_once_released() {
    let (_realm, (mut a, _b), ga, gb, wolf, map) = charmed(false).await;
    assert!(
        !crate::game::tests_pets2::control_update(&mut a, ga).await,
        "charmed: no control"
    );
    assert_eq!(
        spell_start(&mut a, ga, crate::game::harness::WAIT).await,
        Some(BOLT)
    );
    let target = on(&map, move |m| {
        m.unit_fields(ga).map(|f| f.get_guid(UNIT_FIELD_TARGET))
    })
    .await;
    assert_eq!(target, Some(gb), "at the wolf's foe");
    on(&map, move |m| {
        crate::game::map::combat::leave_combat(m, wolf);
        crate::game::map::ai::forget(m, gb);
        crate::game::map::spells::interrupt(m, ga);
    })
    .await;
    assert!(
        crate::game::tests_pets2::control_update(&mut a, ga).await,
        "hers again"
    );
    a.clear();
    assert_eq!(
        spell_start(&mut a, ga, Duration::from_millis(2000)).await,
        None,
        "released: nothing cast for it"
    );
}

/// Thistle Tea's "starts learned casts and retains death and resource
/// validation": a charmed mage out of mana casts nothing of hers (her
/// blade instead: `tests_p7_pets::a_creature_charmed_mage_casts_her_best_at_its_foe_or_strikes_it`);
/// dead, the charm lets her go and nothing is cast; and a spell cast at a
/// place lands where her foe stands (`targets::center` of her target).
#[tokio::test]
async fn a_charmed_mages_place_spell_goes_where_her_foe_stands_and_the_dead_cast_nothing() {
    let (_realm, (mut a, _b), ga, gb, _wolf, map) = charmed(true).await;
    assert_eq!(
        spell_start(&mut a, ga, crate::game::harness::WAIT).await,
        Some(BLIZZARD)
    );
    // It lands where the foe stands: on him.
    let mut hurt = false;
    for _ in 0..200 {
        hurt = on(&map, move |m| {
            m.unit_fields(gb)
                .is_some_and(|f| f.get_u32(UNIT_FIELD_HEALTH) < f.get_u32(UNIT_FIELD_MAXHEALTH))
        })
        .await;
        if hurt {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(hurt, "the foe is struck where he stands");
    on(&map, move |m| {
        crate::game::map::spells::interrupt(m, ga);
        crate::game::map::death::kill(m, ga);
    })
    .await;
    a.clear();
    assert_eq!(
        spell_start(&mut a, ga, Duration::from_millis(2000)).await,
        None,
        "the dead cast nothing"
    );
    let held = on(&map, move |m| m.players[&ga].owned.possessed_by.is_some()).await;
    assert!(!held, "let go");
}

const CONFUSE: u32 = 97_714;

async fn confusion_setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let mut d = crate::game::tests_p3_combat::data();
    let mut s = spell(
        CONFUSE,
        &[aura_effect(
            crate::game::map::auras::AURA_MOD_CONFUSE,
            1,
            0,
            1,
        )],
    );
    // A minute.
    s.duration_index = 0;
    d.spells.spells.insert(CONFUSE, s);
    crate::game::tests_p3_combat::setup_with(d, Class::Warrior).await
}

/// Confuse `g` and start their first walk; the walk's start, duration and
/// the confusion's anchor.
fn confuse(m: &mut crate::game::map::Map, g: Guid) -> Option<(std::time::Instant, Duration)> {
    use crate::game::map::player_motion;
    let s = m.realm.data.spells.get(CONFUSE).cloned().unwrap();
    auras::apply(m, g, g, &s, 0, 0);
    for _ in 0..20 {
        player_motion::update(m);
        if let Some(w) = m.players[&g].driven.as_ref().and_then(|d| d.walk.as_ref()) {
            return Some((w.started, w.duration));
        }
        m.now += Duration::from_millis(1600);
    }
    None
}

fn anchor_of(m: &crate::game::map::Map, g: Guid) -> Option<Vector3d> {
    match crate::game::map::player_motion::driven_by(m, g) {
        Some(crate::game::map::player_motion::Mode::Confusion { anchor }) => Some(anchor),
        _ => None,
    }
}

/// Thistle Tea's "cancels confusion movement and reanchors after
/// relocation": a confused player's walk ended with the confusion stops
/// where it had got to, the stop told, control back; moved a hundred yards
/// and confused again, the new confusion wanders about where they are now
/// (mangos's `ConfusedMovementGenerator` takes its place at its start).
#[tokio::test]
async fn a_confusion_ended_stops_its_walk_where_it_got_and_a_new_one_starts_from_there() {
    use crate::game::map::player_motion;
    let (_realm, mut a, g, map) = confusion_setup().await;
    let (start, anchor, mid, stopped, driven) = on(&map, move |m| {
        let start = m.players[&g].position();
        let (started, duration) = confuse(m, g).expect("a walk");
        let anchor = anchor_of(m, g);
        let mid = started + duration / 2;
        m.now = mid;
        let at = m.players[&g]
            .driven
            .as_ref()
            .and_then(|d| d.walk.as_ref())
            .map(|w| w.position_at(mid).0);
        auras::remove_spell(m, g, CONFUSE, None);
        player_motion::control(m, g);
        (
            start,
            anchor,
            at,
            m.players[&g].position(),
            m.players[&g].driven.is_some(),
        )
    })
    .await;
    assert_eq!(anchor, Some(start), "anchored where it began");
    let mid = mid.unwrap();
    assert!(
        (stopped.x - mid.x).abs() < 0.05 && (stopped.y - mid.y).abs() < 0.05,
        "{stopped:?} {mid:?}"
    );
    assert!(!driven, "control back");
    a.wait_for("the stop", |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == g && matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    // Elsewhere, confused again: about there.
    let (there, anchor) = on(&map, move |m| {
        let p = m.players[&g].position();
        let there = Vector3d {
            x: p.x + 100.0,
            ..p
        };
        crate::game::map::movement::teleport_within(m, g, there, 0.0);
        crate::game::map::movement::teleport_ack(m, g);
        let at = m.players[&g].position();
        confuse(m, g);
        (at, anchor_of(m, g))
    })
    .await;
    assert_eq!(anchor, Some(there));
}

/// Thistle Tea's "publishes player spline progress and clears the
/// projection after arrival": a player the server walks is, halfway, at
/// the walk's halfway point, in that point's cell; at the end, at its end,
/// the walk over (`player_motion::advance`).
#[tokio::test]
async fn a_driven_players_place_follows_the_walk_and_the_walk_ends_at_its_end() {
    use crate::game::map::player_motion;
    let (_realm, _a, g, map) = confusion_setup().await;
    let (halfway, at_mid, cell, end, at_end, walking) = on(&map, move |m| {
        let (started, duration) = confuse(m, g).expect("a walk");
        let (halfway, end) = {
            let w = m.players[&g].driven.as_ref().unwrap().walk.clone().unwrap();
            (w.position_at(started + duration / 2).0, w.destination())
        };
        m.now = started + duration / 2;
        player_motion::update(m);
        let p = &m.players[&g];
        let (at_mid, cell) = (p.position(), p.cell);
        m.now = started + duration;
        player_motion::update(m);
        let p = &m.players[&g];
        let walking = p.driven.as_ref().is_some_and(|d| d.walk.is_some());
        (halfway, at_mid, cell, end, p.position(), walking)
    })
    .await;
    let close = |a: Vector3d, b: Vector3d| (a.x - b.x).abs() < 0.05 && (a.y - b.y).abs() < 0.05;
    assert!(close(at_mid, halfway), "{at_mid:?} {halfway:?}");
    assert_eq!(cell, crate::game::map::grid::cell_of(halfway.x, halfway.y));
    assert!(close(at_end, end), "{at_end:?} {end:?}");
    assert!(!walking, "the walk over");
}
