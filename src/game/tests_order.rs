//! The order of what a client is sent (DECISIONS 11): an object's create
//! reaches it before anything that names the object, whether the object
//! was just introduced to it (its stealth or invisibility ended) or has
//! just come into the map near it. Each test does both in one event, as a
//! packet's handling or one update's step would.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::{MapHandle, auras, combat, scripts, spells};
use crate::game::tests_p3_combat::*;

/// Hides a creature from the players (its invisibility, of a kind no one
/// detects).
const HIDE: u32 = 95_300;
/// A creature's damage over time on a player, ticking once a minute.
const DOT: u32 = 95_301;

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut dot = SpellTemplate {
        id: DOT,
        name: "Rot".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    dot.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_PERIODIC_DAMAGE,
        amplitude: 60_000,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(DOT, dot);
    d
}

/// A brute 40 yards from Alice, invisible to her, and some updates run
/// since (so she would have been told of it).
async fn hidden() -> (Client, Guid, Guid, MapHandle) {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let g = map
        .query(move |m| {
            let g = spawn(m, ga, BRUTE, 20, 40.0);
            aura_of(m, g, HIDE, auras::AURA_MOD_INVISIBILITY, 1, 1);
            g
        })
        .await
        .unwrap();
    updates(&map, 3).await;
    assert!(
        !a.sees_within(g, Duration::from_millis(50)).await,
        "the brute is hidden"
    );
    (a, ga, g, map)
}

/// Wait for `n` of the map's updates.
async fn updates(map: &MapHandle, n: u64) {
    let from = map.query(|m| m.stats.updates).await.unwrap();
    for _ in 0..1000 {
        if map.query(|m| m.stats.updates).await.unwrap() >= from + n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the map ran no {n} updates");
}

/// `g` comes out of hiding.
fn unhide(m: &mut crate::game::map::Map, g: Guid) {
    auras::remove_spell(m, g, HIDE, None);
}

/// `a` gets the first packet `names` takes, and is told of `g` before it.
async fn told_first(
    a: &mut Client,
    g: Guid,
    what: &str,
    names: impl Fn(&ServerOpcodeMessage) -> bool,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let sent: Vec<ServerOpcodeMessage> = a
            .captured()
            .iter()
            .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
            .collect();
        let created = sent.iter().position(|m| harness::creates(m).contains(&g));
        let named = sent.iter().position(&names);
        if let (Some(created), Some(named)) = (created, named) {
            assert!(
                created < named,
                "{what} (packet {named}) reached the client before {g}'s create (packet {created})"
            );
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no {what} and create of {g}: {created:?}, {named:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn a_creature_that_moves_as_it_is_seen_is_created_first() {
    let (mut a, ga, g, map) = hidden().await;
    map.query(move |m| {
        unhide(m, g);
        let at = m.players[&ga].position();
        scripts::move_to(m, g, at, false);
    })
    .await
    .unwrap();
    told_first(
        &mut a,
        g,
        "SMSG_MONSTER_MOVE",
        |m| matches!(m, ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv) if mv.guid == g),
    )
    .await;
}

#[tokio::test]
async fn a_creature_that_casts_as_it_is_seen_is_created_first() {
    let (mut a, ga, g, map) = hidden().await;
    map.query(move |m| {
        unhide(m, g);
        spells::cast_triggered(m, g, THREAT_UP, Some(ga));
    })
    .await
    .unwrap();
    told_first(
        &mut a,
        g,
        "SMSG_SPELL_GO",
        |m| matches!(m, ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.caster == g),
    )
    .await;
}

#[tokio::test]
async fn a_creature_that_emotes_as_it_is_seen_is_created_first() {
    let (mut a, _ga, g, map) = hidden().await;
    map.query(move |m| {
        unhide(m, g);
        scripts::emote(m, g, 1);
    })
    .await
    .unwrap();
    told_first(
        &mut a,
        g,
        "SMSG_EMOTE",
        |m| matches!(m, ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == g),
    )
    .await;
}

#[tokio::test]
async fn a_creature_that_attacks_as_it_is_seen_is_created_first() {
    let (mut a, ga, g, map) = hidden().await;
    map.query(move |m| {
        unhide(m, g);
        combat::start_attack(m, g, ga);
    })
    .await
    .unwrap();
    told_first(
        &mut a,
        g,
        "SMSG_ATTACKSTART",
        |m| matches!(m, ServerOpcodeMessage::SMSG_ATTACKSTART(s) if s.attacker == g),
    )
    .await;
}

#[tokio::test]
async fn an_aura_ticking_from_a_creature_as_it_is_seen_comes_after_its_create() {
    let (mut a, ga, g, map) = hidden().await;
    map.query(move |m| {
        let s = m.realm.data.spells.get(DOT).unwrap().clone();
        auras::apply_spell(m, g, ga, &s, &[(0, 10)], auras::Context::default()).unwrap();
    })
    .await
    .unwrap();
    map.query(move |m| {
        unhide(m, g);
        let now = m.now;
        for h in &mut auras::auras_mut(m, ga).unwrap().holders {
            for x in &mut h.effects {
                x.next_tick = Some(now);
            }
        }
        auras::tick_due(m, ga, DOT, g);
    })
    .await
    .unwrap();
    told_first(
        &mut a,
        g,
        "SMSG_PERIODICAURALOG",
        |m| matches!(m, ServerOpcodeMessage::SMSG_PERIODICAURALOG(l) if l.caster == g),
    )
    .await;
}

#[tokio::test]
async fn a_creature_that_speaks_as_it_spawns_is_created_first() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let g = map
        .query(move |m| {
            let g = spawn(m, ga, BRUTE, 20, 10.0);
            scripts::say(m, g, "Hello", scripts::CHAT_SAY, 0, Guid::zero());
            g
        })
        .await
        .unwrap();
    told_first(&mut a, g, "the brute's say", |m| {
        matches!(
            m,
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. } if sender1 == g)
        )
    })
    .await;
}

#[tokio::test]
async fn a_summoned_pets_bar_comes_after_its_create() {
    use crate::game::tests_pets::{SUMMON_IMP, cast, enter, learn};
    let realm = harness::realm_with(crate::game::tests_pets::data());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    updates(&map, 1).await;
    cast(&w, SUMMON_IMP, None).await;
    let imp = crate::game::tests_pets::pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    told_first(
        &mut w,
        imp,
        "SMSG_PET_SPELLS",
        |m| matches!(m, ServerOpcodeMessage::SMSG_PET_SPELLS(s) if s.pet == imp),
    )
    .await;
}

#[tokio::test]
async fn a_duels_flag_is_created_before_the_challenge_names_it() {
    let (realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 3.0, START.1, START.2).await;
    a.sees(gb).await;
    map.query(move |m| crate::game::map::duel::challenge(m, ga, gb))
        .await
        .unwrap();
    for c in [&mut a, &mut b] {
        let flag = c
            .wait_for("SMSG_DUEL_REQUESTED", |m| match m {
                ServerOpcodeMessage::SMSG_DUEL_REQUESTED(d) => Some(d.initiator),
                _ => None,
            })
            .await;
        told_first(
            c,
            flag,
            "SMSG_DUEL_REQUESTED",
            |m| matches!(m, ServerOpcodeMessage::SMSG_DUEL_REQUESTED(d) if d.initiator == flag),
        )
        .await;
    }
}
