//! Round 7's corpse loot tests, ported from Thistle Tea's
//! `test/game/world/entity/mob/corpse_test.exs` (docs/parity/matrix.csv):
//! the rules of the group a creature was tapped for, a kill that leaves
//! nothing turning no one's round robin, and a corpse with nothing on it
//! going once.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{MapKey, ai, combat, loot};
use crate::game::tests_p3_combat::{BRUTE, START, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

/// A warrior at the start, hardy enough for a brute's blows meanwhile.
async fn player(realm: &std::sync::Arc<crate::game::Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    on(&map, move |m| {
        let f = &mut m.players.get_mut(&g).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
        f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
    })
    .await;
    c
}

/// `by` invites `c`, who accepts.
async fn joins(by: &mut Client, c: &mut Client, name: &str) {
    by.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    c.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    c.send(CMSG_GROUP_ACCEPT {}).await;
    c.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's group loot preparation: a creature Alice tapped in her
/// round-robin party is looted by that party's rules though she has left
/// it for Dee's: its turn goes to one of the party, she may not open it,
/// and Dee's group's turn doesn't move ("uses the original group's loot
/// policy after its tagger leaves"; mangos's `GetGroupLootRecipient`). A
/// kill the players did too little of leaves nothing, and turns neither
/// group's round robin ("an NPC kill below the cutoff does not rotate
/// either group's looter").
#[tokio::test]
async fn a_corpse_is_looted_by_the_rules_of_the_group_it_was_tapped_for() {
    let realm = harness::realm_with(crate::game::tests_p3_combat::data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Cid").await;
    let mut d = player(&realm, "Dee").await;
    let (ga, gb, gc, gd) = (a.guid, b.guid, c.guid, d.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    joins(&mut a, &mut b, "Bob").await;
    joins(&mut a, &mut c, "Cid").await;
    a.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::RoundRobin,
        loot_master: Guid::zero(),
        loot_threshold: ItemQuality::Uncommon,
    })
    .await;
    handled(&mut a).await;
    let party = realm.social.groups.lock().group_of(ga).map(|g| g.id);
    for _ in 0..500 {
        let method = realm
            .social
            .groups
            .lock()
            .group_of(ga)
            .map(|g| g.loot_method);
        if method == Some(GroupLootSetting::RoundRobin) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        realm
            .social
            .groups
            .lock()
            .group_of(ga)
            .map(|g| g.loot_method),
        Some(GroupLootSetting::RoundRobin)
    );
    let (first, second) = on(&map, move |m| {
        let first = spawn(m, ga, BRUTE, 20, 3.0);
        let second = spawn(m, ga, BRUTE, 20, 3.0);
        for k in [first, second] {
            ai::attacked(m, k, ga);
        }
        (first, second)
    })
    .await;
    a.send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut a).await;
    a.clear();
    joins(&mut d, &mut a, "Alice").await;
    for _ in 0..500 {
        if realm.social.groups.lock().group_of(gd).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let turns = |realm: &crate::game::Realm| {
        let groups = realm.social.groups.lock();
        (
            party.and_then(|p| groups.get(p)).map(|g| g.round_robin),
            groups.group_of(gd).map(|g| g.round_robin),
        )
    };
    assert_eq!(turns(&realm), (Some(Guid::zero()), Some(Guid::zero())));
    let looted = on(&map, move |m| {
        combat::kill(m, ga, first);
        let l = loot::loot_ref(m, first).unwrap();
        (l.method, l.assigned, l.may_loot(ga))
    })
    .await;
    let (method, assigned, alice) = looted;
    assert_eq!(method, Some(GroupLootSetting::RoundRobin), "the party's");
    assert!(
        assigned.is_some_and(|g| g == gb || g == gc),
        "the party's turn: {assigned:?}"
    );
    assert!(!alice, "not hers");
    let (party_turn, dees) = turns(&realm);
    assert_eq!(party_turn, assigned, "the party's turn moved");
    assert_eq!(dees, Some(Guid::zero()), "Dee's group's didn't");
    // The other, mostly another's kill: nothing, and no turn moves.
    let nothing = on(&map, move |m| {
        let k = m.creature_mut(second).unwrap();
        k.engagement.damage_by_players = 35;
        k.engagement.damage_by_npcs = 65;
        combat::kill(m, ga, second);
        let l = loot::loot_ref(m, second).unwrap();
        (l.method, l.assigned, l.is_empty())
    })
    .await;
    assert_eq!(nothing, (None, None, true));
    assert_eq!(turns(&realm), (assigned, Some(Guid::zero())));
}

/// Thistle Tea's `Corpse.remove/2` "removes an unlootable corpse without
/// a loot component": a corpse with nothing on it goes from view when it
/// decays, once; its going again is nothing.
#[tokio::test]
async fn a_corpse_with_nothing_on_it_goes_once() {
    let realm = harness::realm_with(crate::game::tests_p3_combat::data());
    let mut a = player(&realm, "Alice").await;
    let ga = a.guid;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let k = on(&map, move |m| spawn(m, ga, BRUTE, 20, 3.0)).await;
    a.sees(k).await;
    let lootable = on(&map, move |m| {
        let c = m.creature_mut(k).unwrap();
        c.engagement.damage_by_players = 0;
        c.engagement.damage_by_npcs = 100;
        combat::kill(m, ga, k);
        let c = m.creature(k).unwrap();
        (
            c.loot.as_ref().is_some_and(|l| l.is_empty()),
            c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & loot::DYNFLAG_LOOTABLE,
        )
    })
    .await;
    assert_eq!(lootable, (true, 0), "nothing on it, and not lootable");
    // Its decay due: it goes from view.
    on(&map, move |m| {
        let now = m.now;
        if let Some(c) = m.creature_mut(k)
            && let crate::game::map::creature::Life::Dead { corpse_until, .. } = &mut c.life
        {
            *corpse_until = now;
        }
        ai::update_dead(m);
    })
    .await;
    a.forgets(k).await;
    a.clear();
    // Again: nothing more.
    on(&map, move |m| {
        ai::update_dead(m);
        ai::update_dead(m);
    })
    .await;
    harness::an_update(&map).await;
    assert!(
        a.gets(Duration::from_millis(200), |m| {
            harness::forgotten(m).contains(&k).then_some(())
        })
        .await
        .is_none(),
        "gone once"
    );
}
