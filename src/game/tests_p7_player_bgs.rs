//! Round 7's player tests of battlegrounds, ported from Thistle Tea's
//! `test/game/world/entity/player/battlegrounds_test.exs`
//! (docs/parity/matrix.csv): a carrier's flag kept on a near teleport and
//! dropped at a logout, the way back out of a match, and who may join.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::map::battleground::{Flag, SPELL_DESERTER};
use crate::game::tests_bg5::{alice_takes_the_horde_flag, gulch, horde_flag, put};
use crate::game::tests_p3_entity::on;
use crate::game::tests_pvp::{both_into, marker};

/// Wait until the Horde's flag is as `want` has it.
async fn flag_until(
    map: &crate::game::map::MapHandle,
    want: impl Fn(Option<Flag>) -> bool,
) -> Option<Flag> {
    let mut f = None;
    for _ in 0..200 {
        f = map.query(|m| horde_flag(m)).await.flatten();
        if want(f) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    f
}

/// Thistle Tea's `State.leave_world` "logout removes carrier auras before
/// retaining the character" and `prepare_worldport` "world transfer
/// removes flags while local travel retains them": a carrier moved within
/// the match (a near teleport) still has the flag; logging out with it
/// drops it, and it is not on them when they come back.
#[tokio::test]
async fn a_carrier_keeps_the_flag_on_a_near_teleport_and_drops_it_at_a_logout() {
    let (mut a, _b, map, alice) = alice_takes_the_horde_flag().await;
    let carried = flag_until(&map, |f| matches!(f, Some(Flag::Carried(_)))).await;
    assert_eq!(carried, Some(Flag::Carried(alice)));
    let (x, y, z, _) = crate::game::map::battleground::wsg::FLAG_AT[1];
    put(&map, &mut a, x + 20.0, y, z).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let still = on(&map, move |m| {
        (
            horde_flag(m),
            m.players[&alice]
                .auras
                .has_spell(crate::game::map::battleground::wsg::SPELL_FLAG_CARRIED[1]),
        )
    })
    .await;
    assert_eq!(still, (Some(Flag::Carried(alice)), true), "kept");
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let dropped = flag_until(&map, |f| matches!(f, Some(Flag::Dropped(_)))).await;
    assert!(matches!(dropped, Some(Flag::Dropped(_))), "{dropped:?}");
    a.clear();
    a.login(alice).await;
    a.sees(alice).await;
    let realm = map.query(|m| m.realm.clone()).await.unwrap();
    let here = realm.registry.online(alice).unwrap().map();
    let has = on(&here, move |m| {
        m.players[&alice]
            .auras
            .has_spell(crate::game::map::battleground::wsg::SPELL_FLAG_CARRIED[1])
    })
    .await;
    assert!(!has, "no flag on her after the logout");
}

/// Thistle Tea's `Battlegrounds.port` "returns a player who accepts far
/// from the portal to where they stood": one who accepted the invitation
/// in Elwynn goes back there, facing as they were, when they leave the
/// match.
#[tokio::test]
async fn one_leaving_a_match_goes_back_where_they_accepted() {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let open = realm.maps.get(&realm, MapKey::world(0));
    let stood = on(&open, move |m| {
        let p = m.players.get_mut(&alice).unwrap();
        p.movement.orientation = 2.0;
        p.position()
    })
    .await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Grom", Race::Orc, Class::Warrior).await;
    both_into(&realm, &mut a, &mut c, Map::WarsongGulch).await;
    a.clear();
    a.send(CMSG_LEAVE_BATTLEFIELD {
        map: Map::WarsongGulch,
    })
    .await;
    let (to, at, facing) = a
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some((w.map, w.position, w.orientation)),
            _ => None,
        })
        .await;
    assert_eq!(to, Map::EasternKingdoms);
    assert!(
        (at.x - stood.x).abs() < 0.01 && (at.y - stood.y).abs() < 0.01,
        "{at:?} {stood:?}"
    );
    assert!((facing - 2.0).abs() < 0.01, "{facing}");
}

const HELM: u32 = 64_520;

/// Thistle Tea's death and combat wear "PvP, controlled pets,
/// battlegrounds, and exempt spells avoid death penalties", and
/// `Instances.refresh` "does not start a timer in open worlds or
/// battlegrounds": dying to a creature in a battleground wears no gear
/// (no `SMSG_DURABILITY_DAMAGE_DEATH`); and one there in no group gets no
/// dungeon's grace to leave (`SMSG_RAID_GROUP_ONLY`).
#[tokio::test]
async fn a_battleground_spares_gear_at_death_and_starts_no_grace() {
    use crate::game::fields::ITEM_FIELD_DURABILITY;
    use crate::game::store::BAG_PLAYER;
    let mut d = gulch();
    d.creature_templates.insert(
        crate::game::tests_p3_combat::BRUTE,
        crate::game::tests_p3_combat::template(crate::game::tests_p3_combat::BRUTE, 7),
    );
    d.items.insert(
        HELM,
        crate::game::data::ItemTemplate {
            entry: HELM,
            name: "Helm".into(),
            class: 4,
            subclass: 1,
            inventory_type: 1,
            stackable: 1,
            max_durability: 10,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut c, Map::WarsongGulch).await;
    a.clear();
    let left = on(&map, move |m| {
        let data = m.realm.data.clone();
        let helm = Guid::new(0x4000_0000_0000_0000 | 964_520);
        let p = m.players.get_mut(&alice).unwrap();
        p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == 0));
        p.items.push(crate::game::map::player::Item::new(
            &data, helm, alice, HELM, BAG_PLAYER, 0, 1,
        ));
        let brute = crate::game::tests_p3_combat::spawn(
            m,
            alice,
            crate::game::tests_p3_combat::BRUTE,
            20,
            3.0,
        );
        crate::game::map::combat::kill(m, brute, alice);
        let p = &m.players[&alice];
        let i = p.item_by_guid(helm).unwrap();
        (p.death, p.items[i].fields.get_u32(ITEM_FIELD_DURABILITY))
    })
    .await;
    assert_ne!(left.0, crate::game::map::death::Death::Alive, "she fell");
    assert_eq!(left.1, 10, "no wear in a battleground");
    let told = a
        .gets(Duration::from_millis(1_500), |m| match m {
            ServerOpcodeMessage::SMSG_DURABILITY_DAMAGE_DEATH => Some("wear"),
            ServerOpcodeMessage::SMSG_RAID_GROUP_ONLY(_) => Some("grace"),
            _ => None,
        })
        .await;
    assert_eq!(told, None, "neither wear nor a grace");
}

/// Thistle Tea's `join` "group admission reads the leader's current aura
/// before its projection" and "departure cannot requeue before the owner
/// has exited its battleground": a party's leader who is a deserter keeps
/// the whole party out (and is told so); one still in a battleground's
/// map, the match left, can't queue again until out of it.
#[tokio::test]
async fn a_deserter_leader_and_one_still_inside_cant_queue() {
    let mut d = gulch();
    marker(&mut d, SPELL_DESERTER);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    let (ao, bo) = (
        realm.registry.online(alice).unwrap(),
        realm.registry.online(bob).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &ao, &bo));
    let open = realm.maps.get(&realm, MapKey::world(0));
    on(&open, move |m| {
        crate::game::map::spells::cast_triggered(m, alice, SPELL_DESERTER, Some(alice));
    })
    .await;
    a.clear();
    a.send(CMSG_BATTLEMASTER_JOIN {
        guid: Guid::zero(),
        map: Map::WarsongGulch,
        instance_id: 0,
        join_as_group: true,
    })
    .await;
    let told = a
        .wait_for("SMSG_GROUP_JOINED_BATTLEGROUND", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_JOINED_BATTLEGROUND(j) => Some(j.id),
            _ => None,
        })
        .await;
    assert_eq!(told, BgTypeId::RemoveFromQueue, "the deserter's error");
    let queued = |g| realm.social.battlegrounds.lock().queued(g);
    assert!(!queued(alice) && !queued(bob), "neither queued");
    // Inside a match, the match left but the map not yet: no queue.
    crate::game::social::groups::leave(&realm, &bo);
    let mut c = Client::connect(&realm, "C");
    c.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut b, &mut c, Map::WarsongGulch).await;
    on(&map, move |m| {
        let realm = m.realm.clone();
        for x in realm.social.battlegrounds.lock().matches.values_mut() {
            x.members.remove(&bob);
        }
    })
    .await;
    b.send(CMSG_BATTLEMASTER_JOIN {
        guid: Guid::zero(),
        map: Map::WarsongGulch,
        instance_id: 0,
        join_as_group: false,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    harness::an_update(&map).await;
    assert!(!queued(bob), "not from inside");
}
