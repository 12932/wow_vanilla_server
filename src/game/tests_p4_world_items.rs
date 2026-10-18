//! Phase 3's fourth round of the gadgets' outcomes, ported from Thistle
//! Tea's `test/game/world/entity/effect_resolver` (`item_spell_dbc_test`,
//! `control_device_dbc_test`, `conditional_triggers_dbc_test`;
//! docs/parity/matrix.csv): what the Six Demon Bag, the Universal Remote,
//! the Gnomish Mind Control Cap and the Scorpid Surprise do, and to whom.
//! Each outcome is a marker aura here, so that who it lands on shows.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::{Map, auras, item_scripts};
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::spell;
use crate::game::tests_p4_world::on;

const SIX_DEMON_BAG: u32 = 14537;
/// Frostbolt, Fireball, Chain Lightning, Polymorph (on the target, or its
/// user), Enveloping Winds, the Felhound (its user's).
const BAG_OUTCOMES: [u32; 6] = [11921, 13322, 21179, 13323, 25189, 14642];
const UNIVERSAL_REMOTE: u32 = 8344;
/// The control, the root, the enrage.
const REMOTE_OUTCOMES: [u32; 3] = [8345, 8346, 8599];
const MIND_CONTROL_CAP: u32 = 13180;
const MIND_CONTROL: u32 = 13181;
const SCORPID_SURPRISE: u32 = 6410;

fn dummy(id: u32) -> SpellTemplate {
    spell(
        id,
        &[SpellEffect {
            effect: 3,
            target_a: 6,
            ..Default::default()
        }],
    )
}

/// A marker: a dummy aura on whom it is cast at.
fn marker(id: u32) -> SpellTemplate {
    spell(
        id,
        &[SpellEffect {
            effect: 6,
            aura: 4,
            target_a: 6,
            ..Default::default()
        }],
    )
}

fn world() -> GameData {
    let mut d = data();
    for id in [SIX_DEMON_BAG, UNIVERSAL_REMOTE, MIND_CONTROL_CAP] {
        d.spells.spells.insert(id, dummy(id));
    }
    for id in BAG_OUTCOMES
        .iter()
        .chain(&REMOTE_OUTCOMES)
        .chain(&[MIND_CONTROL])
    {
        d.spells.spells.insert(*id, marker(*id));
    }
    // The food, and the poison now and then.
    let mut surprise = marker(SCORPID_SURPRISE);
    surprise.effects[1] = SpellEffect {
        effect: 64,
        trigger_spell: 6411,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(SCORPID_SURPRISE, surprise);
    d
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// What of `outcomes` `user` and `target` hold, then none of it.
fn took(m: &mut Map, user: Guid, target: Guid, outcomes: &[u32]) -> Vec<(u32, bool)> {
    let mut got = Vec::new();
    for o in outcomes {
        for (g, on_user) in [(user, true), (target, false)] {
            if has(m, g, *o) {
                got.push((*o, on_user));
                auras::remove_spell(m, g, *o, None);
            }
        }
    }
    got
}

fn uses(
    m: &mut Map,
    user: Guid,
    target: Guid,
    id: u32,
    outcomes: &[u32],
    times: usize,
) -> ahash::AHashMap<(u32, bool), u32> {
    let s = m.realm.data.spells.get(id).cloned().unwrap();
    let mut counts = ahash::AHashMap::new();
    for _ in 0..times {
        assert!(item_scripts::effect(m, user, target, &s, 0, None));
        let got = took(m, user, target, outcomes);
        assert!(got.len() <= 1, "one outcome a use: {got:?}");
        let key = got.first().copied().unwrap_or((0, false));
        *counts.entry(key).or_default() += 1;
    }
    counts
}

/// Thistle Tea's "bag damage uses real child spells", "bag polymorph can
/// backfire on its caster" and "the bag's felhound belongs to the
/// caster": each use of the Six Demon Bag looses one of its tricks, by
/// their weights; its Polymorph now and then on its user, its Felhound
/// always theirs, the rest at the target.
#[tokio::test]
async fn the_six_demon_bag_looses_one_trick_a_use() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let counts = on(&map, move |m| {
        let target = spawn(m, g, BRUTE, 20, 5.0);
        uses(m, g, target, SIX_DEMON_BAG, &BAG_OUTCOMES, 2_000)
    })
    .await;
    let n = |o: u32, on_user: bool| counts.get(&(o, on_user)).copied().unwrap_or(0);
    assert_eq!(n(0, false), 0, "a trick every use: {counts:?}");
    // Out of 2,000: 25%, 25%, 20%, 7% and 3%, 15%, 5%.
    for (o, on_user, share) in [
        (11921, false, 500),
        (13322, false, 500),
        (21179, false, 400),
        (13323, false, 140),
        (13323, true, 60),
        (25189, false, 300),
        (14642, true, 100),
    ] {
        let got = n(o, on_user);
        assert!(
            got > share / 2 && got < share * 3 / 2,
            "{o} on {}: {got} of about {share}",
            if on_user { "its user" } else { "the target" }
        );
    }
    assert_eq!(n(11921, true) + n(25189, true) + n(14642, false), 0);
}

/// Thistle Tea's "the remote hands possession to the caster's channel
/// owner", "remote malfunctions root or enrage the machine" and "cap
/// backfires reverse the control owner": the Universal Remote's control
/// and root are its user's at the machine, and its enrage the machine's
/// own; the Mind Control Cap's control is its wearer's at the target four
/// times in six, the target's at its wearer once.
#[tokio::test]
async fn the_remote_and_the_cap_go_right_or_turn_on_their_user() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let (remote, cap) = on(&map, move |m| {
        let target = spawn(m, g, BRUTE, 20, 5.0);
        let remote = uses(m, g, target, UNIVERSAL_REMOTE, &REMOTE_OUTCOMES, 600);
        let cap = uses(m, g, target, MIND_CONTROL_CAP, &[MIND_CONTROL], 600);
        (remote, cap)
    })
    .await;
    let n = |c: &ahash::AHashMap<(u32, bool), u32>, k: (u32, bool)| c.get(&k).copied().unwrap_or(0);
    for o in REMOTE_OUTCOMES {
        let got = n(&remote, (o, false));
        assert!(
            (120..280).contains(&got),
            "{o}, a third, on the machine: {got}"
        );
    }
    assert_eq!(n(&remote, (0, false)), 0, "{remote:?}");
    let (right, backfired, nothing) = (
        n(&cap, (MIND_CONTROL, false)),
        n(&cap, (MIND_CONTROL, true)),
        n(&cap, (0, false)),
    );
    assert!((320..480).contains(&right), "four in six: {right}");
    assert!(
        (50..150).contains(&backfired),
        "one in six on its wearer: {backfired}"
    );
    assert!(
        (50..150).contains(&nothing),
        "one in six nothing: {nothing}"
    );
}

/// Thistle Tea's "Scorpid Surprise always feeds the player and optionally
/// applies real periodic poison": the food's first effect is always had;
/// its second, the poison, one time in eleven.
#[tokio::test]
async fn a_scorpid_surprise_poisons_one_time_in_eleven() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let (fed, poisoned) = on(&map, move |m| {
        let s = m.realm.data.spells.get(SCORPID_SURPRISE).cloned().unwrap();
        let (mut fed, mut poisoned) = (0, 0);
        for _ in 0..2_200 {
            if !item_scripts::effect(m, g, g, &s, 0, None) {
                fed += 1;
            }
            if !item_scripts::effect(m, g, g, &s, 1, None) {
                poisoned += 1;
            }
        }
        (fed, poisoned)
    })
    .await;
    assert_eq!(fed, 2_200, "fed every time");
    assert!((120..300).contains(&poisoned), "one in eleven: {poisoned}");
}

const SNOWBALL: u32 = 21343;
const SNOWBALL_KNOCKDOWN: u32 = 21167;

/// Thistle Tea's "applies its effects only while both players share a
/// group": a snowball knocks down a player of its thrower's group, and
/// no one else.
#[tokio::test]
async fn a_snowball_knocks_down_only_one_of_its_throwers_group() {
    let mut d = world();
    d.spells.spells.insert(
        SNOWBALL,
        spell(
            SNOWBALL,
            &[SpellEffect {
                effect: 77,
                target_a: 25,
                ..Default::default()
            }],
        ),
    );
    let mut knockdown = marker(SNOWBALL_KNOCKDOWN);
    knockdown.effects[0].target_a = 1;
    d.spells.spells.insert(SNOWBALL_KNOCKDOWN, knockdown);
    let (realm, mut a, g, map) = setup_with(d, Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bea", Race::Human, Class::Warrior).await;
    let mut c = crate::game::harness::Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    let throw = |at: Guid| {
        let map = map.clone();
        async move {
            on(&map, move |m| {
                crate::game::map::spells::cast_triggered(m, g, SNOWBALL, Some(at));
                let down = has(m, at, SNOWBALL_KNOCKDOWN);
                auras::remove_spell(m, at, SNOWBALL_KNOCKDOWN, None);
                down
            })
            .await
        }
    };
    assert!(!throw(gb).await, "not grouped: no knockdown");
    a.send(CMSG_GROUP_INVITE { name: "Bea".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GROUP_INVITE(_)
        )
        .then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("the roster", |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GROUP_LIST(l) => {
            (!l.members.is_empty()).then_some(())
        }
        _ => None,
    })
    .await;
    assert!(throw(gb).await, "grouped: down");
    assert!(!throw(gc).await, "one outside the group: not");
}
