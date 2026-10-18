//! Thistle Tea's Stratholme instance data as Aurius's quests and gossip
//! and the Baron's fight read and write it, on the real world: each copy's
//! own, and read again each time a player asks.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p4_scripts_dungeons::*;
use crate::game::harness::Client;
use crate::game::map::instance_script::DONE;

const AURIUS: u32 = 10917;

/// What `c` is shown at Aurius's hello: the greeting's text and the
/// quests offered.
async fn asks(c: &mut Client, aurius: Guid) -> (u32, Vec<u32>) {
    c.clear();
    hello(c, aurius).await;
    c.wait_for("Aurius's menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == aurius => Some((
            g.title_text_id,
            g.quests.iter().map(|q| q.quest_id).collect(),
        )),
        _ => None,
    })
    .await
}

/// Aurius by Stratholme's field seven: at 0 he offers the Medallion of
/// Faith (5122) and greets with 3755; at 1 neither quest and 3756; at 2
/// his Reckoning (5125) and 3757; another copy's Aurius as its own data
/// has it; and one who would take 5125 off a menu gone stale is told it
/// isn't to be had. The real conditions and quests are as Thistle Tea
/// pins them.
#[tokio::test]
#[ignore = "extended"]
async fn aurius_reads_his_copys_field_seven() {
    let Some(realm) = realm() else {
        return;
    };
    let d = &realm.data;
    for (id, field, value) in [(3755, 7, 0), (3756, 7, 1), (3757, 7, 2), (3758, 5, 3)] {
        let c = &d.scripts.conditions[&id];
        assert_eq!(
            (c.kind, c.values[0], c.values[1]),
            (34, field, value),
            "{id}"
        );
    }
    assert_eq!(d.scripts.quest_condition.get(&5122), Some(&3755));
    assert_eq!(d.scripts.quest_condition.get(&5125), Some(&3757));
    let (mut c, _, map) = inside(&realm, "Aurius", 329, AURIUS).await;
    let (mut o, _, other) = inside(&realm, "Otherone", 329, AURIUS).await;
    let aurius = alive(&map, AURIUS).await[0];
    let theirs = alive(&other, AURIUS).await[0];
    c.sees(aurius).await;
    o.sees(theirs).await;
    assert_eq!(asks(&mut c, aurius).await, (3755, vec![5122]), "at 0");
    map.query(|m| set(m, 7, 1)).await;
    assert_eq!(asks(&mut c, aurius).await, (3756, vec![]), "at 1");
    assert_eq!(
        asks(&mut o, theirs).await,
        (3755, vec![5122]),
        "the other copy's"
    );
    map.query(|m| set(m, 7, 2)).await;
    assert_eq!(asks(&mut c, aurius).await, (3757, vec![5125]), "at 2");
    // The menu goes stale.
    map.query(|m| set(m, 7, 1)).await;
    c.clear();
    c.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: aurius,
        quest_id: 5125,
    })
    .await;
    c.wait_for("SMSG_QUESTGIVER_QUEST_INVALID", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(_)).then_some(())
    })
    .await;
    let me = c.guid;
    assert_eq!(
        map.query(move |m| m.players.get(&me).is_some_and(|p| p.journal.has(5125)))
            .await,
        Some(false),
        "not taken"
    );
}

/// The Baron's fight writes field five, as his real script has it: begun
/// as he is pulled, three as he dies (condition 3758 met then), in his
/// copy alone; the real ziggurat bosses, crystals and abominations are
/// where Thistle Tea pins them.
#[tokio::test]
#[ignore = "extended"]
async fn the_barons_life_writes_field_five() {
    let Some(realm) = realm() else {
        return;
    };
    let d = &realm.data;
    let spawns_of = |entries: &[u32]| -> Vec<u32> {
        let mut v: Vec<u32> = d.creature_spawns[&329]
            .iter()
            .filter(|s| s.entries.iter().any(|e| entries.contains(e)))
            .map(|s| s.guid)
            .collect();
        v.sort_unstable();
        v
    };
    assert_eq!(
        spawns_of(&[10436, 10437, 10438, 10440]),
        [54238, 54239, 54240, 54241]
    );
    assert_eq!(spawns_of(&[10415]), [53955, 53963, 53968]);
    assert_eq!(spawns_of(&[10399]).len(), 15);
    assert_eq!(spawns_of(&[10416, 10417]).len(), 13);
    let (_c, me, map) = inside(&realm, "Baron", 329, 10440).await;
    let (_o, _, other) = inside(&realm, "Bystander", 329, 10440).await;
    map.query(move |m| {
        let baron = first(m, 10440).unwrap();
        crate::game::map::ai::attacked(m, baron, me);
    })
    .await;
    assert!(
        until(&map, 3000, |m| data(m, 5) == 1).await,
        "begun as he is pulled"
    );
    map.query(move |m| kill_all(m, me, 10440)).await;
    assert!(
        until(&map, 3000, |m| data(m, 5) == DONE).await,
        "three as he dies"
    );
    let met = map
        .query(move |m| crate::game::map::conditions::met(m, 3758, None, Some(me)))
        .await
        .unwrap();
    assert!(met, "condition 3758 met");
    sleep_ms(100).await;
    assert_eq!(other.query(|m| data(m, 5)).await, Some(0), "his copy alone");
}
