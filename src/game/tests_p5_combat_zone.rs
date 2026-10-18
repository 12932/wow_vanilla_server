//! Phase 3's fifth round of combat tests, a dungeon boss's fight with the
//! whole dungeon (Thistle Tea's `zone_combat_test.exs`): who a pulse draws
//! in, what a later pulse leaves alone, what starts it and what ends it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey, ai, combat, creature_combat};
use crate::game::tests_p3_combat::{self as base, *};

const BOSS: u32 = 3740;
const BOSS_SPAWN: u32 = 79_100;

fn data() -> GameData {
    let mut d = base::data();
    let mut boss = template(BOSS, 7);
    boss.rank = 3;
    // It fights its whole dungeon.
    boss.static_flags2 = 0x2;
    d.creature_templates.insert(BOSS, boss);
    d.creature_spawns.insert(
        36,
        vec![CreatureSpawn {
            guid: BOSS_SPAWN,
            entries: vec![BOSS],
            map: 36,
            x: 3.0,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        }],
    );
    d.maps.insert(
        36,
        MapInfo {
            id: 36,
            internal_name: "DeadminesInstance".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d
}

/// Alice and Bob, grouped, in the dungeon; its map and its boss.
async fn in_the_dungeon() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Client,
    MapHandle,
    Guid,
) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    for c in [&mut a, &mut b] {
        let g = c.guid;
        realm.registry.online(g).unwrap().map().call(move |m| {
            m.teleport(
                g,
                MapKey::world(36),
                Vector3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                0.0,
            )
        });
        c.wait_for("SMSG_NEW_WORLD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        c.sees(g).await;
    }
    let dungeon = realm.registry.online(a.guid).unwrap().map();
    let boss = guid::unit(BOSS, BOSS_SPAWN);
    a.sees(boss).await;
    (realm, a, b, dungeon, boss)
}

fn threat(m: &Map, boss: Guid, g: Guid) -> Option<f32> {
    m.creature(boss)?
        .threat
        .iter()
        .find(|(t, _)| *t == g)
        .map(|(_, v)| *v)
}

/// Thistle Tea's "excludes dead ... participants", "later pulses skip
/// players already in combat and preserve the current victim and threat"
/// and "NPC contact does not activate the flag": a boss pulled by a
/// creature draws no one in; pulled by Alice, it leaves Bob, dead, out; a
/// later pulse draws Bob in, alive again, and leaves Alice's threat and
/// its victim as they were.
#[tokio::test]
async fn a_bosss_pulse_draws_in_the_living_and_leaves_the_fight_as_it_was() {
    let (_realm, a, b, dungeon, boss) = in_the_dungeon().await;
    let (ga, gb) = (a.guid, b.guid);
    let r = dungeon
        .query(move |m| {
            // A creature's pull: no one drawn in.
            let npc = spawn(m, ga, HELPER, 20, 2.0);
            ai::attacked(m, boss, npc);
            let by_npc = (threat(m, boss, ga), threat(m, boss, gb));
            ai::enter_evade(m, boss);
            m.creature_mut(boss).unwrap().evading = false;
            // Bob dead; Alice pulls.
            combat::deal_damage(m, boss, gb, 1_000_000);
            ai::attacked(m, boss, ga);
            let pulled = (threat(m, boss, ga).is_some(), threat(m, boss, gb));
            ai::add_threat(m, boss, ga, 25.0);
            // Bob up again; the next pulse.
            crate::game::map::death::resurrect(m, gb, 1.0);
            let now = m.now;
            m.creature_mut(boss).unwrap().engagement.zone_pulse_at = Some(now);
            creature_combat::update(m);
            let c = m.creature(boss).unwrap();
            let later = (
                threat(m, boss, ga),
                threat(m, boss, gb).is_some(),
                c.combat.victim == ga,
            );
            (by_npc, pulled, later)
        })
        .await
        .unwrap();
    let (by_npc, pulled, later) = r;
    assert_eq!(by_npc, (None, None), "a creature's pull draws no one in");
    assert_eq!(pulled, (true, None), "the dead left out");
    assert_eq!(
        later,
        (Some(25.0), true, true),
        "Bob in, the rest as it was"
    );
}

/// Thistle Tea's "combat exit clears the pulse": an evade ends the boss's
/// pulses.
#[tokio::test]
async fn a_bosss_pulses_end_with_its_fight() {
    let (_realm, a, _b, dungeon, boss) = in_the_dungeon().await;
    let ga = a.guid;
    let r = dungeon
        .query(move |m| {
            ai::attacked(m, boss, ga);
            let pulsing = m.creature(boss).unwrap().engagement.zone_pulse_at.is_some();
            ai::enter_evade(m, boss);
            let after = m.creature(boss).unwrap().engagement.zone_pulse_at.is_some();
            (pulsing, after)
        })
        .await
        .unwrap();
    assert_eq!(r, (true, false));
}
