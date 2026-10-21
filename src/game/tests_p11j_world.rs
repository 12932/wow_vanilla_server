//! Round 11j's world tests: a corpse's insignia, an outdoor PvP zone's
//! buffs, a resurrection offered at logout, a script's player across a
//! worldport, a trap disarmed, and a trainer's profession (Thistle Tea's
//! `insignia_test`, `outdoor_pvp_test`, `resurrection_test`,
//! `state_test`, `trap_disarming_test` and `spells_dbc_test`).

use std::time::Duration;

use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::battleground as bg;
use crate::game::tests_p3_entity::on;

/// Thistle Tea's "only one competing claim creates bones, and money can
/// be taken once": Alice's claim on a fallen Horde player's corpse from
/// out of reach (more than ten yards) is refused, the corpse kept; two
/// claims from beside it, one after the other in the map's turn, make one
/// set of bones and open its loot once; its gold is taken once.
#[tokio::test]
async fn one_claim_of_a_corpses_insignia_makes_bones_and_its_gold_is_taken_once() {
    let mut d = crate::game::tests_pvp2::valley();
    let mut s = crate::game::data::spells::SpellTemplate {
        id: 22027,
        name: "Remove Insignia".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = bg::insignia::EFFECT_SKIN_PLAYER_CORPSE;
    d.spells.spells.insert(22027, s);
    let realm = crate::game::harness::realm_with(d);
    let (mut a, alice, _b, grom, map) = crate::game::tests_pvp3::pair(&realm).await;
    let (corpse, at) = on(&map, move |m| {
        bg::died(m, grom, alice);
        crate::game::map::death::kill(m, grom);
        crate::game::map::death::release(m, grom);
        m.corpses
            .values()
            .find(|c| c.owner == grom)
            .map(|c| (c.guid, c.position))
            .expect("his corpse")
    })
    .await;
    let bones = move |m: &crate::game::map::Map| {
        m.corpses
            .values()
            .filter(|c| c.fields.get_u32(CORPSE_FIELD_FLAGS) & 0x01 != 0)
            .map(|c| c.guid)
            .collect::<Vec<_>>()
    };
    // Twelve yards off: refused.
    crate::game::tests_pvp3::put(&map, &mut a, at.x + 12.0, at.y, at.z).await;
    a.clear();
    let far = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, alice, 22027, Some(corpse));
        (m.corpses.contains_key(&corpse), bones(m).len())
    })
    .await;
    assert_eq!(far, (true, 0), "out of reach");
    crate::game::tests_pvp3::put(&map, &mut a, at.x + 1.0, at.y, at.z).await;
    let seen = a.captured().len();
    let made = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, alice, 22027, Some(corpse));
        let first = bones(m);
        // The second claim, on what is now bones.
        if let Some(b) = first.first().copied() {
            crate::game::map::spells::cast_triggered(m, alice, 22027, Some(b));
        }
        crate::game::map::spells::cast_triggered(m, alice, 22027, Some(corpse));
        (first.len(), bones(m).len(), m.corpses.contains_key(&corpse))
    })
    .await;
    assert_eq!(made, (1, 1, false), "one set of bones");
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let windows = a.captured()[seen..]
        .iter()
        .filter(|f| u16::from_le_bytes([f[2], f[3]]) == 0x160)
        .count();
    assert_eq!(windows, 1, "its loot opened once");
    let before = on(&map, move |m| m.players[&alice].money()).await;
    a.send(CMSG_LOOT_MONEY {}).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let once = on(&map, move |m| m.players[&alice].money()).await;
    assert!(once - before >= bg::insignia::gold(1, 50), "{before} {once}");
    a.send(CMSG_LOOT_MONEY {}).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert_eq!(
        on(&map, move |m| m.players[&alice].money()).await,
        once,
        "taken once"
    );
}

/// Thistle Tea's "replaces ranks and clears rewards at death, exit, and a
/// new login": Alice in the Plaguelands with one tower her side's has its
/// first rank (11413); a second tower taken, the second rank (11414) in
/// its place; dead, none; alive again, the second; out of the zone, none;
/// back, and logged out and in there, the second alone again
/// (`outdoor_pvp::reconcile_buffs`, each second; mangos's
/// `OutdoorPvP::HandlePlayerEnterZone` and `HandlePlayerLeaveZone`).
#[tokio::test]
async fn a_towers_buff_is_its_rank_now_and_off_the_dead_and_those_gone() {
    use crate::game::tests_p3_world_pvp::{AT, in_the_plaguelands, stand, tick};
    const RANKS: [u32; 4] = [11413, 11414, 11415, 1386];
    let (_realm, mut clients, map) = in_the_plaguelands(&[("Alice", Race::Human)]).await;
    let alice = clients[0].guid;
    let ranks = move |m: &mut crate::game::map::Map| {
        RANKS
            .into_iter()
            .filter(|r| m.players[&alice].auras.has_spell(*r))
            .collect::<Vec<_>>()
    };
    on(&map, move |m| {
        stand(m, alice, AT[2]);
        m.outdoor.as_mut().unwrap().towers[2].meter.progress = 240_000;
    })
    .await;
    tick(&map).await;
    assert_eq!(on(&map, ranks).await, vec![11413], "one tower");
    on(&map, move |m| {
        m.outdoor.as_mut().unwrap().towers[3].meter.progress = 240_000;
    })
    .await;
    tick(&map).await;
    assert_eq!(on(&map, ranks).await, vec![11414], "two: the next rank alone");
    on(&map, move |m| crate::game::map::death::kill(m, alice)).await;
    tick(&map).await;
    assert_eq!(on(&map, ranks).await, Vec::<u32>::new(), "dead");
    on(&map, move |m| {
        crate::game::map::death::resurrect(m, alice, 1.0);
    })
    .await;
    tick(&map).await;
    assert_eq!(on(&map, ranks).await, vec![11414], "alive again");
    on(&map, move |m| m.players.get_mut(&alice).unwrap().zone = 12).await;
    tick(&map).await;
    assert_eq!(on(&map, ranks).await, Vec::<u32>::new(), "out of the zone");
    on(&map, move |m| stand(m, alice, AT[2])).await;
    tick(&map).await;
    let c = &mut clients[0];
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE
        )
        .then_some(())
    })
    .await;
    c.clear();
    c.login(alice).await;
    on(&map, move |m| {
        m.players.get_mut(&alice).unwrap().pvp.desired = true;
    })
    .await;
    tick(&map).await;
    assert_eq!(on(&map, ranks).await, vec![11414], "a new login: the rank now");
}

const RES: u32 = 900_390;

/// Thistle Tea's "logout clears an offer before retaining the character":
/// a resurrection offered to a fallen warrior who logs out is not kept
/// (mangos's `m_resurrectGuid` isn't saved): logged in again, a ghost, her
/// acceptance of it does nothing.
#[tokio::test]
async fn a_resurrection_offered_is_gone_after_a_logout() {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let mut d = crate::game::data::GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut s = SpellTemplate {
        id: RES,
        name: "Resurrection".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 18,
        target_a: 25,
        base_points: 34,
        ..Default::default()
    };
    d.spells.spells.insert(RES, s);
    let realm = crate::game::harness::realm_with(d);
    let (_a, priest, map) =
        crate::game::tests_pets::enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, fallen, _) =
        crate::game::tests_pets::enter(&realm, "B", "Fallen", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, priest, fallen, 1_000_000);
        crate::game::map::combat::leave_combat(m, fallen);
        crate::game::map::combat::leave_combat(m, priest);
        // Released, so that she is a ghost at her login.
        crate::game::map::death::release(m, fallen);
        crate::game::map::spells::cast_triggered(m, priest, RES, Some(fallen));
    })
    .await;
    b.wait_frame("the offer", 0x15B, |_| true).await;
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    b.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE
        )
        .then_some(())
    })
    .await;
    b.clear();
    b.login(fallen).await;
    let map = realm.registry.online(fallen).expect("online").map();
    let offer = on(&map, move |m| m.players[&fallen].resurrect_offer).await;
    assert_eq!(offer, None, "no offer after the logout");
    let state = on(&map, move |m| format!("{:?}", m.players[&fallen].death)).await;
    assert!(state.starts_with("Ghost"), "a ghost at login: {state}");
    b.send(CMSG_RESURRECT_RESPONSE {
        guid: priest,
        status: 1,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut b).await;
    let alive = on(&map, move |m| {
        m.players[&fallen].death == crate::game::map::death::Death::Alive
    })
    .await;
    assert!(!alive, "the old offer raises no one");
}

/// Thistle Tea's "discards script continuations while retaining the
/// receipt sequence": a script a player is the source of, with a step a
/// second on (sitting), is dropped when she takes a worldport before it:
/// the map she left runs no more of it, and she doesn't sit in the new
/// one (`scripts::advance` drops a run whose source has left its map). TT's
/// receipt sequence is its own bookkeeping.
#[tokio::test]
async fn a_players_script_is_dropped_when_she_takes_a_worldport() {
    use crate::game::data::scripts::{ScriptStep, ScriptTable};
    let mut d = crate::game::tests_p3_combat::data();
    d.scripts.generic.insert(
        9_590,
        vec![ScriptStep {
            command: 28,
            delay_ms: 1_000,
            datalong: [1, 0, 0, 0],
            ..Default::default()
        }],
    );
    let (realm, mut a, g, from) = crate::game::tests_p3_combat::setup_with(d, Class::Warrior).await;
    from.call(move |m| {
        crate::game::map::scripts::start(m, ScriptTable::Generic, 9_590, g, g);
        m.teleport(
            g,
            crate::game::map::MapKey::world(1),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        );
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_NEW_WORLD(_)
        )
        .then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let to = realm.registry.online(g).unwrap().map();
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let (stand, key) = on(&to, move |m| {
        (
            m.players[&g].fields.get_byte(UNIT_FIELD_BYTES_1, 0),
            m.key.map,
        )
    })
    .await;
    assert_eq!((stand, key), (0, 1), "she doesn't sit in Kalimdor");
    let left = on(&from, move |m| {
        crate::game::map::scripts::update(m);
        m.scripts
            .runs
            .iter()
            .any(|r| r.source == g && r.next != usize::MAX)
    })
    .await;
    assert!(!left, "her run is gone from the map she left");
}

const TRAP: u32 = 97_950;
const TRAP_LOCK: u32 = 97_951;
const DISARM: u32 = 97_952;
const PICK: u32 = 97_953;

/// Thistle Tea's "rechecks detection, range, world, and lock type before
/// removing the target": a Disarm Trap cast at a trap, its caster gone
/// thirty yards off as it ends, fails and the trap stays; a lock of
/// another kind (a lockpick's) is refused it; cast from beside it to its
/// end, the trap goes (`professions::check` again as the cast ends,
/// mangos's `CheckCast(false)`). Detection isn't checked again, as
/// mangos doesn't.
#[tokio::test]
async fn a_disarm_trap_ended_out_of_reach_or_of_another_lock_leaves_the_trap() {
    use crate::game::data::crafts::LockKey;
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let mut d = crate::game::tests_p3_combat::data();
    let mut t = crate::game::data::GameObjectTemplate {
        entry: TRAP,
        kind: crate::game::map::objects::GO_TRAP,
        display_id: 1,
        name: "Trap".into(),
        size: 1.0,
        ..Default::default()
    };
    t.data[0] = TRAP_LOCK;
    d.gameobject_templates.insert(TRAP, t);
    d.crafts.locks.insert(
        TRAP_LOCK,
        vec![LockKey {
            kind: 2,
            index: 4,
            skill: 0,
        }],
    );
    d.spells.cast_times.insert(9, 2000);
    for (id, lock_type) in [(DISARM, 4), (PICK, 1)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Open {id}"),
            equipped_item_class: -1,
            range_index: 1,
            casting_time_index: 9,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 33,
            misc_value: lock_type,
            target_a: 23,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    let (_realm, _a, g, map) = crate::game::tests_p3_combat::setup_with(d, Class::Rogue).await;
    let (trap, at) = on(&map, move |m| {
        let p = m.players[&g].position();
        let spot = Vector3d { x: p.x + 2.0, ..p };
        let trap =
            crate::game::map::objects::summon(m, None, TRAP, spot, 0.0, 0, 0, 0, None).unwrap();
        (trap, p)
    })
    .await;
    let targets = move || crate::game::map::spells::Targets {
        object: Some(trap),
        ..Default::default()
    };
    // Begun beside it, ended thirty yards off.
    let far = on(&map, move |m| {
        crate::game::map::spells::cast(m, g, DISARM, targets(), None);
        let begun = m.caster(g).is_some_and(|c| c.casting.is_some());
        m.players.get_mut(&g).unwrap().movement.position = Vector3d {
            x: at.x + 30.0,
            ..at
        };
        let now = m.now;
        if let Some(c) = m.caster_mut(g).and_then(|c| c.casting.as_mut()) {
            c.ends = now;
        }
        begun
    })
    .await;
    assert!(far, "begun");
    crate::game::harness::an_update(&map).await;
    crate::game::harness::an_update(&map).await;
    let left = on(&map, move |m| {
        (
            m.gameobject(trap).is_some(),
            m.caster(g).is_some_and(|c| c.casting.is_some()),
        )
    })
    .await;
    assert_eq!(left, (true, false), "out of reach: the trap stays");
    // A lockpick's lock: refused.
    let picked = on(&map, move |m| {
        m.players.get_mut(&g).unwrap().movement.position = at;
        m.now = std::time::Instant::now();
        if let Some(p) = m.players.get_mut(&g) {
            p.caster.gcd_until = None;
        }
        crate::game::map::spells::cast(m, g, PICK, targets(), None);
        m.caster(g).is_some_and(|c| c.casting.is_some())
    })
    .await;
    assert!(!picked, "another kind of lock");
    // Beside it to the end: disarmed.
    on(&map, move |m| {
        crate::game::map::spells::cast(m, g, DISARM, targets(), None);
        let now = m.now;
        if let Some(c) = m.caster_mut(g).and_then(|c| c.casting.as_mut()) {
            c.ends = now;
        }
    })
    .await;
    let mut gone = false;
    for _ in 0..100 {
        crate::game::harness::an_update(&map).await;
        gone = on(&map, move |m| m.gameobject(trap).is_none()).await;
        if gone {
            break;
        }
    }
    assert!(gone, "disarmed");
}
