//! Round 6's tests of a ghost's run back to a body in a dungeon, ported
//! from Thistle Tea's `test/game/world/entity/player/corpse_travel_test.exs`
//! (docs/parity/matrix.csv): where the body is shown, which portals take
//! the ghost, and what brings them back to life.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::MapTemplate;
use crate::game::data::{AreaTrigger, GameData, MapInfo, Teleport};
use crate::game::harness::{self, Client};
use crate::game::map::death::Death;
use crate::game::map::{MapHandle, MapKey};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::{NONE_WITHIN, on};

/// The Deadmines; Blackrock Depths and Molten Core inside it.
const DEADMINES: u32 = 36;
const DEPTHS: u32 = 230;
const CORE: u32 = 409;
/// The portals, all where players start: the Deadmines', the Depths', the
/// Core's (from inside the Depths, but for the test here), and one into the
/// Deadmines for the level 60 alone.
const DEADMINES_PORTAL: u32 = 9_101;
const DEPTHS_PORTAL: u32 = 9_102;
const CORE_PORTAL: u32 = 9_103;
const HIGH_PORTAL: u32 = 9_104;
/// Where the portals land: the Deadmines' inside, the Core's.
const INSIDE: Vector3d = Vector3d {
    x: -16.4,
    y: -383.07,
    z: 61.78,
};
const IN_CORE: Vector3d = Vector3d {
    x: 1_096.0,
    y: -467.0,
    z: -104.6,
};

fn world(core_kind: u32) -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for (id, kind, parent, name) in [
        (DEADMINES, 1, 0, "Deadmines"),
        (DEPTHS, 1, 0, "Blackrock Depths"),
        (CORE, core_kind, DEPTHS, "Molten Core"),
    ] {
        d.maps.insert(
            id,
            MapInfo {
                id,
                internal_name: format!("Map{id}"),
                name: name.into(),
                instance_type: kind,
                player_limit: 0,
            },
        );
        d.scripts.map_templates.insert(
            id,
            MapTemplate {
                map: id,
                map_type: kind,
                parent,
                name: name.into(),
                player_limit: 40,
                ghost_entrance: Some((0, -11_207.8, 1_681.15)),
                ..Default::default()
            },
        );
    }
    for (id, to, at, level) in [
        (DEADMINES_PORTAL, DEADMINES, INSIDE, 0),
        (DEPTHS_PORTAL, DEPTHS, INSIDE, 0),
        (CORE_PORTAL, CORE, IN_CORE, 0),
        (HIGH_PORTAL, DEADMINES, INSIDE, 60),
    ] {
        d.area_triggers.insert(
            id,
            AreaTrigger {
                id,
                map: 0,
                x: START.0,
                y: START.1,
                z: START.2,
                radius: 10.0,
                teleport: Some(Teleport {
                    map: to,
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    required_level: level,
                    message: if level > 0 {
                        "Level 60 required".into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
    }
    d
}

/// Players who are no GMs.
fn realm(d: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.instance_unload = std::time::Duration::from_secs(600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

async fn alice(realm: &Arc<Realm>) -> (Client, Guid) {
    let mut a = Client::connect(realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    (a, g)
}

fn map_of(realm: &Realm, g: Guid) -> MapHandle {
    realm.registry.online(g).unwrap().map()
}

/// `g` a ghost where they are, their body in `map` (or nowhere).
async fn ghost(realm: &Arc<Realm>, g: Guid, body: Option<u32>) {
    on(&map_of(realm, g), move |m| {
        crate::game::map::death::kill(m, g);
        let now = m.now;
        let p = m.players.get_mut(&g).unwrap();
        p.row.state.corpse = body.map(|map| crate::game::store::CorpseState {
            map,
            x: INSIDE.x,
            y: INSIDE.y,
            z: INSIDE.z,
            orientation: 0.0,
        });
        p.row.state.released_at = crate::game::map::corpses::unix_ms();
        crate::game::map::death::become_ghost(p, now);
    })
    .await;
}

/// What `a` is told stepping into `trigger`: the map it is sent to, or the
/// refusal's text, or neither.
async fn step(a: &mut Client, trigger: u32) -> Option<Result<Map, String>> {
    a.clear();
    a.send(CMSG_AREATRIGGER {
        trigger_id: trigger,
    })
    .await;
    a.gets(harness::WAIT, |m| match m {
        ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some(Ok(w.map)),
        ServerOpcodeMessage::SMSG_AREA_TRIGGER_MESSAGE(t) => Some(Err(t.message.clone())),
        _ => None,
    })
    .await
}

async fn death_of(realm: &Arc<Realm>, g: Guid) -> Death {
    on(&map_of(realm, g), move |m| m.players[&g].death).await
}

/// Thistle Tea's `Corpses.portal_destination` "routes a parent portal to
/// the inner entrance": a ghost whose body is in Molten Core, stepping into
/// Blackrock Depths' portal, is taken on to the Core's entrance, and comes
/// back to life there.
#[tokio::test]
async fn a_ghost_at_a_parent_dungeons_portal_goes_on_to_the_inner_ones() {
    let realm = realm(world(1));
    let (mut a, ga) = alice(&realm).await;
    ghost(&realm, ga, Some(CORE)).await;
    let to = step(&mut a, DEPTHS_PORTAL).await;
    assert_eq!(to, Some(Ok(Map::MoltenCore)), "on to the Core");
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let (key, at, alive) = on(&map_of(&realm, ga), move |m| {
        let p = &m.players[&ga];
        (m.key.map, p.position(), p.death == Death::Alive)
    })
    .await;
    assert_eq!(key, CORE);
    assert!((at.x - IN_CORE.x).abs() < 0.5, "at its entrance: {at:?}");
    assert!(alive, "alive at the portal");
}

/// Thistle Tea's `AreaTriggers.handle` "revives at the correct portal and
/// returns to the original copy", "rejects unrelated and missing corpses"
/// and "keeps the ghost when portal requirements fail": a ghost whose body
/// is in the Deadmines copy she was in comes back to life at its portal,
/// at half health, into that copy; a ghost with no body is turned away,
/// and one short of a portal's level is told so, still a ghost.
#[tokio::test]
async fn a_ghost_lives_again_at_her_bodys_portal_into_her_copy_and_no_other() {
    let realm = realm(world(1));
    let (mut a, ga) = alice(&realm).await;
    // Her copy of the Deadmines, entered and left.
    let map = map_of(&realm, ga);
    map.call(move |m| m.teleport(ga, MapKey::world(DEADMINES), INSIDE, 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let copy = map_of(&realm, ga).query(|m| m.key).await.unwrap();
    let back = map_of(&realm, ga);
    let start = Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    };
    back.call(move |m| m.teleport(ga, MapKey::world(0), start, 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    // No body: no way in.
    ghost(&realm, ga, None).await;
    assert_eq!(
        step(&mut a, DEADMINES_PORTAL).await,
        Some(Err("You cannot enter Deadmines while in ghost form.".into()))
    );
    assert!(matches!(death_of(&realm, ga).await, Death::Ghost { .. }));
    // Her body in the Deadmines: too low for one portal, let in by its own.
    on(&map_of(&realm, ga), move |m| {
        m.players.get_mut(&ga).unwrap().row.state.corpse = Some(crate::game::store::CorpseState {
            map: DEADMINES,
            x: INSIDE.x,
            y: INSIDE.y,
            z: INSIDE.z,
            orientation: 0.0,
        });
    })
    .await;
    assert_eq!(
        step(&mut a, HIGH_PORTAL).await,
        Some(Err("Level 60 required".into()))
    );
    assert!(
        matches!(death_of(&realm, ga).await, Death::Ghost { .. }),
        "still a ghost"
    );
    assert_eq!(
        step(&mut a, DEADMINES_PORTAL).await,
        Some(Ok(Map::Deadmines))
    );
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let inside = map_of(&realm, ga);
    let (key, alive, health, max, body) = on(&inside, move |m| {
        let p = &m.players[&ga];
        (
            m.key,
            p.death == Death::Alive,
            p.fields.get_u32(crate::game::fields::UNIT_FIELD_HEALTH),
            p.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH),
            p.row.state.corpse.is_some(),
        )
    })
    .await;
    assert_eq!(key, copy, "her copy");
    assert!(alive && !body, "alive, the body gone");
    assert_eq!(health, max / 2, "at half health");
}

/// Thistle Tea's `AreaTriggers.handle` "revives even if copy admission
/// subsequently fails": a ghost whose body is in Molten Core, a raid, at
/// its portal comes back to life there, but alone, in no raid group, is
/// told so and not let in (mangos revives at the portal before the map's
/// `CanEnter`).
#[tokio::test]
async fn a_ghost_lives_again_at_her_portal_though_the_raid_then_turns_her_away() {
    let realm = realm(world(2));
    let (mut a, ga) = alice(&realm).await;
    ghost(&realm, ga, Some(CORE)).await;
    a.clear();
    a.send(CMSG_AREATRIGGER {
        trigger_id: CORE_PORTAL,
    })
    .await;
    a.wait_for("SMSG_RAID_GROUP_ONLY", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(_)).then_some(())
    })
    .await;
    assert_eq!(death_of(&realm, ga).await, Death::Alive, "alive");
    assert!(
        a.gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await
        .is_none(),
        "not let in"
    );
    assert_eq!(
        map_of(&realm, ga).query(|m| m.key).await,
        Some(MapKey::world(0))
    );
}

/// The corpse `a` is shown: the map it's shown on, where, and the map it's
/// in; `None` for none.
async fn shown(a: &mut Client) -> Option<(Map, Vector3d, Map)> {
    a.clear();
    a.send(MSG_CORPSE_QUERY_Client {}).await;
    a.wait_for("MSG_CORPSE_QUERY", |m| match m {
        ServerOpcodeMessage::MSG_CORPSE_QUERY(q) => Some(match &q.result {
            MSG_CORPSE_QUERY_Server_CorpseQueryResult::Found {
                map,
                position,
                corpse_map,
            } => Some((*map, *position, *corpse_map)),
            _ => None,
        }),
        _ => None,
    })
    .await
}

/// Thistle Tea's `Corpses.location` "shows the entrance outside and the
/// body inside": outside, a ghost is shown her dungeon's entrance; inside
/// it, the body itself; with no body, none.
#[tokio::test]
async fn a_ghost_is_shown_the_entrance_outside_and_the_body_inside() {
    let realm = realm(world(1));
    let (mut a, ga) = alice(&realm).await;
    ghost(&realm, ga, Some(DEADMINES)).await;
    let (on_map, at, body) = shown(&mut a).await.expect("shown");
    assert_eq!((on_map, body), (Map::EasternKingdoms, Map::Deadmines));
    assert_eq!((at.x, at.y), (-11_207.8, 1_681.15), "the entrance");
    // Inside the Deadmines, a ghost still.
    let map = map_of(&realm, ga);
    map.call(move |m| m.teleport(ga, MapKey::world(DEADMINES), INSIDE, 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let (on_map, at, body) = shown(&mut a).await.expect("shown");
    assert_eq!((on_map, body), (Map::Deadmines, Map::Deadmines));
    assert_eq!(
        (at.x, at.y, at.z),
        (INSIDE.x, INSIDE.y, INSIDE.z),
        "the body"
    );
    on(&map_of(&realm, ga), move |m| {
        m.players.get_mut(&ga).unwrap().row.state.corpse = None;
    })
    .await;
    assert_eq!(shown(&mut a).await, None, "no body");
}

/// Thistle Tea's `Corpses.restore` "keeps ghosts with bodies and recovers
/// missing bodies": logged back in, a ghost with a body is a ghost still,
/// her body there; one who died and went before releasing (no body) is up
/// again.
#[tokio::test]
async fn a_ghost_back_is_one_with_her_body_and_without_one_lives() {
    let realm = realm(world(1));
    let (mut a, ga) = alice(&realm).await;
    let relog = async |a: &mut Client| {
        a.send(CMSG_LOGOUT_REQUEST {}).await;
        a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
        })
        .await;
        a.clear();
        a.login(ga).await;
        a.sees(ga).await;
    };
    ghost(&realm, ga, Some(0)).await;
    relog(&mut a).await;
    let (death, body) = on(&map_of(&realm, ga), move |m| {
        (
            m.players[&ga].death,
            m.corpses
                .contains_key(&crate::game::guid::corpse(crate::game::guid::counter(ga))),
        )
    })
    .await;
    assert!(matches!(death, Death::Ghost { .. }), "a ghost");
    assert!(body, "her body back where it lay");
    ghost(&realm, ga, None).await;
    on(&map_of(&realm, ga), move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.row.state.dead = true;
    })
    .await;
    relog(&mut a).await;
    assert_eq!(death_of(&realm, ga).await, Death::Alive, "up again");
    let (health, max) = on(&map_of(&realm, ga), move |m| {
        let f = &m.players[&ga].fields;
        (
            f.get_u32(crate::game::fields::UNIT_FIELD_HEALTH),
            f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH),
        )
    })
    .await;
    assert_eq!(health, max / 2, "at half health (mangos's `LoadCorpse`)");
}
