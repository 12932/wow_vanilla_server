//! Phase 3 against Thistle Tea's honor tests (`test/game/core/honor`): a
//! kill's honor shared by the damage done, by group, among the living
//! near; the week's standings and the rank points a level may hold.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::honor::{self, Candidate};
use crate::game::map::{Map, MapKey, pvp, stats};

/// Northshire, where every one of them is put.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// Level 60 humans by these names, and an undead to kill (`victim`), all
/// in Northshire.
async fn cast(realm: &Arc<Realm>, names: &[&str], victim: &str) -> (Vec<Client>, Vec<Guid>, Guid) {
    let mut clients = Vec::new();
    let mut guids = Vec::new();
    for name in names {
        let mut c = Client::connect(realm, name);
        guids.push(c.enter(name, Race::Human, Class::Warrior).await);
        clients.push(c);
    }
    let mut v = Client::connect(realm, victim);
    let victim = v.enter(victim, Race::Undead, Class::Warrior).await;
    clients.push(v);
    let all: Vec<Guid> = guids.iter().copied().chain([victim]).collect();
    on_map(realm, move |m| {
        let data = m.realm.data.clone();
        for g in all {
            let p = m.players.get_mut(&g).unwrap();
            stats::set_level(p, &data, 60);
            p.movement.position = Vector3d {
                x: START.0,
                y: START.1,
                z: START.2,
            };
        }
    })
    .await;
    (clients, guids, victim)
}

/// The honor each of `who` has today.
fn today(realm: &Realm, who: &[Guid]) -> Vec<u32> {
    let ledger = realm.social.honor.lock();
    who.iter()
        .map(|g| {
            ledger
                .entries
                .get(&guid::counter(*g))
                .and_then(|e| e.days.get(&ledger.day))
                .map_or(0, |d| d.contribution as u32)
        })
        .collect()
}

/// Two level 60s, the victim unranked, by `share`.
fn worth(share: f32) -> u32 {
    (honor::kill_points(60, 60, 0, 0) * share).trunc() as u32
}

#[tokio::test]
async fn a_kills_honor_is_by_the_damage_done_a_creatures_diluting_and_none_to_the_dead_far_or_friendly()
 {
    let realm = harness::realm();
    let (_clients, g, victim) = cast(&realm, &["Alice", "Bob"], "Grim").await;
    let (alice, bob) = (g[0], g[1]);
    // Alice 300, Bob 100, a creature 100: three fifths and a fifth.
    on_map(&realm, move |m| {
        pvp::damaged(m, alice, victim, 300);
        pvp::damaged(m, bob, victim, 100);
        pvp::damaged(m, guid::unit(3500, 1), victim, 100);
        pvp::player_killed(m, alice, victim);
    })
    .await;
    assert_eq!(today(&realm, &[alice, bob]), vec![worth(0.6), worth(0.2)]);
    // A self-inflicted death: those who hurt the victim lately share it.
    let (_clients, g, victim) = cast(&realm, &["Carol", "Dave", "Eve"], "Gnash").await;
    let (carol, dave, eve) = (g[0], g[1], g[2]);
    on_map(&realm, move |m| {
        pvp::damaged(m, carol, victim, 100);
        pvp::damaged(m, victim, victim, 500);
        pvp::player_killed(m, victim, victim);
    })
    .await;
    assert_eq!(today(&realm, &[carol]), vec![worth(1.0)]);
    // Dave dead, Eve far, and a friend of the victim's: their damage is
    // no one's, and Carol has her fifth.
    let mut friend = Client::connect(&realm, "Hilda");
    let hilda = friend.enter("Hilda", Race::Undead, Class::Warrior).await;
    let mut other = Client::connect(&realm, "Gorm");
    let victim = other.enter("Gorm", Race::Undead, Class::Warrior).await;
    let before = today(&realm, &[carol])[0];
    on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&victim).unwrap();
        stats::set_level(p, &data, 60);
        p.movement.position = Vector3d {
            x: START.0,
            y: START.1,
            z: START.2,
        };
        crate::game::map::death::kill(m, dave);
        m.players.get_mut(&eve).unwrap().movement.position.x += 200.0;
        for (by, d) in [(carol, 100), (dave, 100), (eve, 100), (hilda, 100)] {
            pvp::damaged(m, by, victim, d);
        }
        pvp::damaged(m, guid::player(999), victim, 100);
        pvp::player_killed(m, carol, victim);
    })
    .await;
    let after = today(&realm, &[carol, dave, eve]);
    assert_eq!(after, vec![before + worth(0.2), 0, 0]);
}

#[tokio::test]
async fn a_groups_share_goes_to_its_living_members_near_however_little_they_did_and_a_dead_killers_to_no_one()
 {
    let realm = harness::realm();
    let (mut clients, g, victim) =
        cast(&realm, &["Alice", "Bob", "Carol", "Dave", "Eve"], "Grim").await;
    let (alice, dave, eve) = (g[0], g[3], g[4]);
    // Alice leads Bob, Carol and Dave.
    let (lead, rest) = clients.split_at_mut(1);
    for (member, name) in rest.iter_mut().take(3).zip(["Bob", "Carol", "Dave"]) {
        lead[0].send(CMSG_GROUP_INVITE { name: name.into() }).await;
        member
            .wait_for("SMSG_GROUP_INVITE", |m| {
                matches!(
                    m,
                    wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GROUP_INVITE(_)
                )
                .then_some(())
            })
            .await;
        member.send(CMSG_GROUP_ACCEPT {}).await;
    }
    for _ in 0..200 {
        let n = realm
            .social
            .groups
            .lock()
            .group_of(alice)
            .map_or(0, |gr| gr.members.len());
        if n == 4 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // Alice 300 and Dave (dead) 100 for the group, Eve 100 alone: the
    // group's four fifths among its three living, at a third's rate.
    on_map(&realm, move |m| {
        crate::game::map::death::kill(m, dave);
        pvp::damaged(m, alice, victim, 300);
        pvp::damaged(m, dave, victim, 100);
        pvp::damaged(m, eve, victim, 100);
        pvp::player_killed(m, alice, victim);
    })
    .await;
    let each = worth(0.8 * honor::group_rate(3) / 3.0);
    assert_eq!(today(&realm, &g), vec![each, each, each, 0, worth(0.2)]);
    // Ten share at the large group's rate: a twentieth each.
    assert!((honor::group_rate(10) / 10.0 - 0.05).abs() < 1e-6);
    // Killed by a dead Eve's last blow, alone: no one.
    let before = today(&realm, &[eve]);
    on_map(&realm, move |m| {
        crate::game::map::death::kill(m, eve);
        pvp::damaged(m, eve, victim, 100);
        pvp::player_killed(m, eve, victim);
    })
    .await;
    assert_eq!(today(&realm, &[eve]), before);
}

#[test]
fn a_levels_rank_points_are_capped_and_equal_contributions_stand_equal() {
    assert_eq!(
        [29, 30, 35, 36, 39, 40, 43, 44, 52, 53, 60].map(honor::rank_maximum),
        [
            6_500.0, 7_150.0, 12_025.0, 13_325.0, 17_225.0, 18_850.0, 23_725.0, 26_000.0, 44_200.0,
            46_800.0, 65_000.0
        ]
    );
    let candidate = |guid, contribution, level| Candidate {
        guid,
        team: 0,
        level,
        honorable_kills: 15,
        contribution,
        rank_points: 0.0,
    };
    // A hundred of 500 each: the same points, by GUID.
    let equal: Vec<Candidate> = (1..=100).map(|g| candidate(g, 500.0, 60)).collect();
    let mut reversed = equal.clone();
    reversed.reverse();
    let mut a = honor::weekly(&equal);
    let mut b = honor::weekly(&reversed);
    a.sort_by_key(|(g, _)| *g);
    b.sort_by_key(|(g, _)| *g);
    assert_eq!(a, b);
    assert_eq!((a[0].1.position, a[99].1.position), (1, 100));
    assert!(a.iter().all(|(_, s)| s.earning == a[0].1.earning));
    // The best of a thousand at level 29: no more than 6,500.
    let many: Vec<Candidate> = (1..=1000)
        .map(|g| candidate(g, (1001 - g) as f32, 29))
        .collect();
    let best = honor::weekly(&many)
        .into_iter()
        .find(|(g, _)| *g == 1)
        .unwrap()
        .1;
    assert_eq!(best.rank_points, 6_500.0);
}
