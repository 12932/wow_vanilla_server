//! New weapon skills start at 1 (DECISIONS #39), as mangos-classic's
//! `LearnDefaultSkills` and `UpdateSpellTrainedSkills` have them, and as
//! Thistle Tea's `:level` skills: a new character's, and one a trainer
//! teaches at any level. Skills are saved now, so what one rose to stays;
//! a character saved before keeps its weapon skills at their caps.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, skills, stats};
use crate::game::store::{CharacterState, Store};

/// One-Handed Swords, Defense and Arms (a class skill), as a new warrior's
/// spells teach them; Two-Handed Swords, a trainer's.
const SWORDS: u32 = 96_100;
const DEFENSE: u32 = 96_101;
const ARMS: u32 = 96_102;
const TWO_HANDED: u32 = 96_103;

fn skill_spell(id: u32, skill: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Skill {skill}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: crate::game::map::spells::EFFECT_SKILL,
        misc_value: skill,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for (id, skill) in [(SWORDS, 43), (DEFENSE, 95), (ARMS, 26), (TWO_HANDED, 55)] {
        d.spells.spells.insert(id, skill_spell(id, skill));
    }
    let mut start = crate::game::data::start_info(1, 1).expect("a human warrior's start");
    start.spells = vec![SWORDS, DEFENSE, ARMS];
    d.start.insert((1, 1), start);
    d
}

/// A realm as a live one runs: no skill put at its cap for the tests.
fn live_realm() -> Arc<Realm> {
    let (store, startup) = Store::open(":memory:").expect("an in-memory store");
    let mut config = harness::test_config();
    config.players.always_max_skill_for_level = false;
    Realm::new(config, store, startup, data(), None)
}

async fn alice(realm: &Arc<Realm>) -> (Client, Guid) {
    let mut a = Client::connect(realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    (a, g)
}

/// A skill's value and cap.
fn of(m: &Map, g: Guid, skill: u32) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    Some((skills::value(p, skill)?, skills::max_value(p, skill)?))
}

fn level(m: &mut Map, g: Guid, to: u8) {
    let data = m.realm.data.clone();
    stats::set_level(m.players.get_mut(&g).unwrap(), &data, to);
}

async fn relog(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    let g = c.guid;
    c.login(g).await;
}

#[tokio::test]
async fn a_new_characters_weapon_skills_and_defense_start_at_one_and_a_class_skill_at_its_cap() {
    let realm = live_realm();
    let (_a, g) = alice(&realm).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            let new = [43, 95, 26].map(|s| of(m, g, s));
            // Levels raise their caps, not their values.
            level(m, g, 10);
            (new, [43, 95, 26].map(|s| of(m, g, s)))
        })
        .await
        .unwrap();
    assert_eq!(r.0, [Some((1, 5)), Some((1, 5)), Some((5, 5))]);
    assert_eq!(r.1, [Some((1, 50)), Some((1, 50)), Some((5, 50))]);
}

#[tokio::test]
async fn a_weapon_skill_a_trainer_teaches_at_sixty_starts_at_one_of_three_hundred() {
    let realm = live_realm();
    let (_a, g) = alice(&realm).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            level(m, g, 60);
            crate::game::map::spells::learn(m, g, TWO_HANDED);
            let learned = of(m, g, 55);
            // Learning it again keeps what it rose to.
            skills::set_value(m.players.get_mut(&g).unwrap(), 55, 120);
            crate::game::map::spells::learn(m, g, TWO_HANDED);
            (learned, of(m, g, 55))
        })
        .await
        .unwrap();
    assert_eq!(r, (Some((1, 300)), Some((120, 300))));
}

#[tokio::test]
async fn what_a_skill_rose_to_is_kept_across_a_relog() {
    let realm = live_realm();
    let (mut a, g) = alice(&realm).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    map.query(move |m| {
        level(m, g, 20);
        let p = m.players.get_mut(&g).unwrap();
        skills::set_value(p, 43, 37);
        skills::set_value(p, 95, 64);
    })
    .await
    .unwrap();
    relog(&mut a).await;
    let r = realm
        .maps
        .get(&realm, MapKey::world(0))
        .query(move |m| [43, 95, 26].map(|s| of(m, g, s)))
        .await
        .unwrap();
    assert_eq!(r, [Some((37, 100)), Some((64, 100)), Some((5, 100))]);
}

#[tokio::test]
async fn a_character_saved_before_skills_were_keeps_its_weapon_skills_at_their_caps() {
    // Its state has no skills: none were saved.
    let old: CharacterState = serde_json::from_str("{\"health\": 10}").unwrap();
    assert_eq!(old.skills, None);
    let realm = live_realm();
    let (_a, g) = alice(&realm).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            level(m, g, 40);
            let data = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            for s in skills::known(p) {
                skills::forget_skill(p, s);
            }
            // Loaded as such a character is: from its spells, at its caps.
            p.row.state.skills = None;
            skills::init(p, &data);
            let legacy = [43, 95, 26].map(|s| of(m, g, s));
            // Loaded with its values saved: as they were.
            let p = m.players.get_mut(&g).unwrap();
            for s in skills::known(p) {
                skills::forget_skill(p, s);
            }
            p.row.state.skills = Some(vec![(43, 150), (95, 9)]);
            skills::init(p, &data);
            (legacy, [43, 95].map(|s| of(m, g, s)))
        })
        .await
        .unwrap();
    assert_eq!(r.0, [Some((200, 200)), Some((200, 200)), Some((200, 200))]);
    assert_eq!(r.1, [Some((150, 200)), Some((9, 200))]);
}

#[tokio::test]
async fn the_tests_realm_puts_weapon_skills_at_their_caps_at_login_and_each_level() {
    let realm = harness::realm_with(data());
    let (_a, g) = alice(&realm).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            let new = of(m, g, 43);
            level(m, g, 60);
            let sixty = [43, 95].map(|s| of(m, g, s));
            // A skill learned is at 1 until the next level, as mangos has it.
            crate::game::map::spells::learn(m, g, TWO_HANDED);
            (new, sixty, of(m, g, 55))
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some((5, 5)));
    assert_eq!(r.1, [Some((300, 300)), Some((300, 300))]);
    assert_eq!(r.2, Some((1, 300)));
}
