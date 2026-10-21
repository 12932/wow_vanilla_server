//! Round 12i: what every battleground shares, as mangos-classic runs it,
//! through the protocol: a side's raid and its /bg chat, three queues at
//! once, a match ended early with a side gone, AFK in a match, and a place
//! kept five minutes for one gone offline, Deserter after.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{battleground, into, queue};
use crate::game::battlegrounds::SMSG_BATTLEFIELD_STATUS;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::battleground::{Phase, SPELL_DESERTER};
use crate::game::tests_p3_entity::NONE_WITHIN;

const WSG: u32 = 489;

/// The three battlegrounds, and Deserter, an aura that stays.
fn all_three() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, WSG, 2, "PVPZone03", [770, 771]);
    battleground(&mut d, 529, 3, "PVPzone04", [890, 889]);
    battleground(&mut d, 30, 1, "PVPZone01", [611, 610]);
    let mut s = crate::game::data::spells::SpellTemplate {
        id: SPELL_DESERTER,
        name: "Deserter".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0].effect = 6;
    s.effects[0].aura = 4;
    s.effects[0].target_a = 1;
    d.spells.spells.insert(s.id, s);
    d
}

fn map_of(realm: &crate::game::Realm, g: Guid) -> MapHandle {
    realm.registry.online(g).unwrap().map()
}

fn in_match(realm: &crate::game::Realm, g: Guid) -> bool {
    realm
        .social
        .battlegrounds
        .lock()
        .matches
        .values()
        .any(|m| m.members.contains_key(&g))
}

async fn has_deserter(realm: &crate::game::Realm, g: Guid) -> bool {
    for _ in 0..200 {
        let on = map_of(realm, g)
            .query(move |m| crate::game::battlegrounds::deserter(m, g))
            .await
            .unwrap_or(false);
        if on {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

async fn speak(c: &Client, chat_type: CMSG_MESSAGECHAT_ChatType, text: &str) {
    c.send(CMSG_MESSAGECHAT {
        chat_type,
        language: Language::Common,
        message: text.into(),
    })
    .await;
}

/// Alice and Grom, each alone, in Warsong Gulch, its battle begun.
async fn two_in(
    realm: &std::sync::Arc<crate::game::Realm>,
) -> (Client, Guid, Client, Guid, MapHandle) {
    let mut a = Client::connect(realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut g = Client::connect(realm, "G");
    let grom = g.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = super::tests_pvp::both_into(realm, &mut a, &mut g, Map::WarsongGulch).await;
    (a, alice, g, grom, map)
}

#[tokio::test]
async fn going_afk_in_a_match_leaves_it_a_deserter() {
    let realm = harness::realm_with(all_three());
    let (a, alice, _g, _grom, _map) = two_in(&realm).await;
    assert!(in_match(&realm, alice));
    speak(&a, CMSG_MESSAGECHAT_ChatType::Afk, "").await;
    let mut out = false;
    for _ in 0..200 {
        out = !in_match(&realm, alice);
        if out {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(out, "AFK, out of the match");
    assert!(has_deserter(&realm, alice).await, "a deserter");
}

#[tokio::test]
async fn one_offline_keeps_their_place_five_minutes_then_deserts() {
    let realm = harness::realm_with(all_three());
    let (a, alice, _g, _grom, _map) = two_in(&realm).await;
    a.disconnect();
    for _ in 0..200 {
        if realm.registry.online(alice).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Her place, kept five minutes (mangos's MAX_OFFLINE_TIME), not an
    // invitation's two.
    let until = realm
        .social
        .battlegrounds
        .lock()
        .matches
        .values()
        .find_map(|m| {
            m.members
                .contains_key(&alice)
                .then(|| m.offline.get(&alice).copied())
                .flatten()
        });
    let until = until.expect("her place kept, as a member");
    assert!(
        until > Instant::now() + Duration::from_secs(240),
        "five minutes"
    );
    // The five minutes up: out, and a deserter at her next login.
    for m in realm.social.battlegrounds.lock().matches.values_mut() {
        if let Some(t) = m.offline.get_mut(&alice) {
            *t = Instant::now();
        }
    }
    crate::game::battlegrounds::sweep(&realm);
    assert!(!in_match(&realm, alice), "out of the match");
    realm.store.flush().await;
    let mut a2 = Client::connect(&realm, "A");
    a2.login(alice).await;
    a2.sees(alice).await;
    assert_ne!(map_of(&realm, alice).key.map, WSG, "home");
    assert!(has_deserter(&realm, alice).await, "a deserter at login");
}

#[tokio::test]
async fn a_side_gone_ends_the_match_after_its_warning_the_other_side_winning() {
    let realm = harness::realm_with(all_three());
    let (mut a, _alice, g, _grom, map) = two_in(&realm).await;
    a.clear();
    g.send(CMSG_LEAVE_BATTLEFIELD {
        map: Map::WarsongGulch,
    })
    .await;
    // The Horde gone: the warning, five minutes.
    let warned = a
        .wait_for("the warning", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. })
                    && c.message.starts_with("Not enough players") =>
            {
                Some(c.message.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(
        warned,
        "Not enough players. This game will close in 5 mins."
    );
    // Its time up: over, the Alliance the larger side.
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            b.premature = Some(Instant::now());
        }
    })
    .await;
    let mut phase = None;
    for _ in 0..200 {
        phase = map
            .query(|m| m.battleground.as_ref().map(|b| b.phase))
            .await
            .flatten();
        if matches!(phase, Some(Phase::Ended { .. })) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        matches!(phase, Some(Phase::Ended { winner: 0, .. })),
        "{phase:?}"
    );
}

#[tokio::test]
async fn a_match_ended_early_goes_to_the_side_ahead_before_the_larger() {
    let realm = harness::realm_with(all_three());
    let (_a, _alice, g, _grom, map) = two_in(&realm).await;
    // The Horde a capture ahead, then gone.
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            b.score = [0, 1];
        }
    })
    .await;
    g.send(CMSG_LEAVE_BATTLEFIELD {
        map: Map::WarsongGulch,
    })
    .await;
    let mut started = false;
    for _ in 0..200 {
        started = map
            .query(|m| {
                m.battleground
                    .as_ref()
                    .is_some_and(|b| b.premature.is_some())
            })
            .await
            .unwrap_or(false);
        if started {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(started, "the countdown under way");
    map.query(|m| {
        if let Some(b) = m.battleground.as_mut() {
            b.premature = Some(Instant::now());
        }
    })
    .await;
    let mut phase = None;
    for _ in 0..200 {
        phase = map
            .query(|m| m.battleground.as_ref().map(|b| b.phase))
            .await
            .flatten();
        if matches!(phase, Some(Phase::Ended { .. })) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        matches!(phase, Some(Phase::Ended { winner: 1, .. })),
        "the Horde, ahead: {phase:?}"
    );
}

/// The queue slot, map and status of a `SMSG_BATTLEFIELD_STATUS` body.
fn status(f: &[u8]) -> (u32, u32, u32) {
    let u = |i: usize| u32::from_le_bytes(f[i..i + 4].try_into().unwrap());
    if f.len() < 17 {
        return (u(0), 0, 0);
    }
    (u(0), u(4), u(13))
}

async fn join(c: &Client, map: Map) {
    c.send(CMSG_BATTLEMASTER_JOIN {
        guid: Guid::zero(),
        map,
        instance_id: 0,
        join_as_group: false,
    })
    .await;
}

#[tokio::test]
async fn three_queues_at_once_each_in_its_slot() {
    let realm = harness::realm_with(all_three());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    for (map, id, slot) in [
        (Map::WarsongGulch, 489, 0),
        (Map::ArathiBasin, 529, 1),
        (Map::AlteracValley, 30, 2),
    ] {
        join(&a, map).await;
        let f = a
            .wait_frame("queued", SMSG_BATTLEFIELD_STATUS, |f| {
                status(f) == (slot, id, 1)
            })
            .await;
        assert_eq!(status(&f), (slot, id, 1));
    }
    let slots = realm.social.battlegrounds.lock().slots(alice);
    assert_eq!(slots, vec![(0, 2), (1, 3), (2, 1)]);
    // Grom queues for Arathi Basin: Alice is invited, in its slot.
    let mut g = Client::connect(&realm, "G");
    g.enter("Grom", Race::Orc, Class::Warrior).await;
    join(&g, Map::ArathiBasin).await;
    a.wait_frame("the invitation", SMSG_BATTLEFIELD_STATUS, |f| {
        status(f) == (1, 529, 2)
    })
    .await;
    a.send(CMSG_BATTLEFIELD_PORT {
        map: Map::ArathiBasin,
        action: BattlefieldPortAction::EnterBattle,
    })
    .await;
    a.wait_for("SMSG_NEW_WORLD", |m| match m {
        ServerOpcodeMessage::SMSG_NEW_WORLD(w) => (w.map == Map::ArathiBasin).then_some(()),
        _ => None,
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(alice).await;
    // In the match, still queued for the other two.
    a.clear();
    a.send(CMSG_BATTLEFIELD_STATUS {}).await;
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let f = a
            .wait_frame("a status", SMSG_BATTLEFIELD_STATUS, |_| true)
            .await;
        seen.push(status(&f));
    }
    seen.sort();
    assert_eq!(seen, vec![(0, 489, 1), (1, 529, 3), (2, 30, 1)]);
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    leader
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
}

/// The text of a line of `kind` that `c` hears within a while.
async fn hears(c: &mut Client, kind: fn(&SMSG_MESSAGECHAT_ChatType) -> bool, text: &str) -> bool {
    let text = text.to_owned();
    c.gets(NONE_WITHIN, move |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(x) if kind(&x.chat_type) && x.message == text => {
            Some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

#[tokio::test]
async fn each_side_is_a_raid_with_bg_chat_its_members_own_groups_kept_apart() {
    let realm = harness::realm_with(all_three());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Dwarf, Class::Paladin).await;
    let mut c = Client::connect(&realm, "C");
    let carol = c.enter("Carol", Race::Gnome, Class::Mage).await;
    let mut g = Client::connect(&realm, "G");
    g.enter("Grom", Race::Orc, Class::Warrior).await;
    group(&mut a, &mut b, "Bob").await;
    a.send(CMSG_BATTLEMASTER_JOIN {
        guid: Guid::zero(),
        map: Map::WarsongGulch,
        instance_id: 0,
        join_as_group: true,
    })
    .await;
    b.wait_frame("queued", SMSG_BATTLEFIELD_STATUS, |f| status(f).2 == 1)
        .await;
    queue(&c, Map::WarsongGulch).await;
    c.wait_frame("queued", SMSG_BATTLEFIELD_STATUS, |f| status(f).2 == 1)
        .await;
    queue(&g, Map::WarsongGulch).await;
    for cl in [&mut a, &mut b, &mut c, &mut g] {
        into(&realm, cl, Map::WarsongGulch).await;
    }
    // The Alliance's raid: Alice (her party's leader) leads it, with Bob
    // and Carol.
    let raid = a
        .wait_for("the side's raid", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l)
                if l.group_type == GroupType::Raid && l.members.len() == 2 =>
            {
                Some(l.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(raid.leader, alice);
    let mut names: Vec<Guid> = raid.members.iter().map(|m| m.guid).collect();
    names.sort();
    let mut want = vec![bob, carol];
    want.sort();
    assert_eq!(names, want);
    // /bg reaches the side; not the Horde.
    a.clear();
    c.clear();
    g.clear();
    speak(&b, CMSG_MESSAGECHAT_ChatType::Battleground, "Flag mid").await;
    let is_bg =
        |t: &SMSG_MESSAGECHAT_ChatType| matches!(t, SMSG_MESSAGECHAT_ChatType::Battleground { .. });
    assert!(hears(&mut c, is_bg, "Flag mid").await, "Carol hears /bg");
    assert!(hears(&mut a, is_bg, "Flag mid").await, "Alice hears /bg");
    assert!(!hears(&mut g, is_bg, "Flag mid").await, "the Horde doesn't");
    // Party chat goes to Bob's own party: Alice, not Carol.
    speak(&b, CMSG_MESSAGECHAT_ChatType::Party, "just us").await;
    let is_party =
        |t: &SMSG_MESSAGECHAT_ChatType| matches!(t, SMSG_MESSAGECHAT_ChatType::Party { .. });
    assert!(hears(&mut a, is_party, "just us").await, "Alice, his party");
    assert!(!hears(&mut c, is_party, "just us").await, "not Carol");
    // Bob leaves: back in his party with Alice.
    b.clear();
    b.send(CMSG_LEAVE_BATTLEFIELD {
        map: Map::WarsongGulch,
    })
    .await;
    let back = b
        .wait_for("his party again", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if l.group_type == GroupType::Normal => {
                Some(l.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(back.members.len(), 1);
    assert_eq!(back.members[0].guid, alice);
}
