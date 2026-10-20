//! The fifth code review's findings in start-up, the data and the store
//! (`docs/review/review5.md`, `rev5-data`), each shown in the fast suite.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::{MapKey, skills};
use crate::game::store::Store;

const SWORDS: u32 = 96_100;
const DEFENSE: u32 = 96_101;

/// A directory of the test's own, emptied.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wvs-r5d-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn db(&self) -> PathBuf {
        self.0.join("characters.sqlite")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

/// A human warrior's start, knowing One-Handed Swords and Defense.
fn data() -> GameData {
    let mut d = GameData::empty();
    for (id, skill) in [(SWORDS, 43), (DEFENSE, 95)] {
        d.spells.spells.insert(id, skill_spell(id, skill));
    }
    let mut start = crate::game::data::start_info(1, 1).expect("a human warrior's start");
    start.spells = vec![SWORDS, DEFENSE];
    d.start.insert((1, 1), start);
    d
}

fn server(db: &Path, data: GameData) -> Arc<Realm> {
    let (store, startup) = Store::open(db).expect("the store");
    let mut config = harness::test_config();
    config.players.always_max_skill_for_level = false;
    Realm::new(config, store, startup, data, None)
}

async fn logout(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
}

async fn stop(realm: Arc<Realm>, client: Client) {
    realm.shutdown().await;
    drop(client);
    drop(realm);
}

async fn skill_of(realm: &Arc<Realm>, g: Guid, skill: u32) -> Option<u32> {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(move |m| skills::value(&m.players[&g], skill))
        .await
        .unwrap()
}

/// #6: a login on a world whose data didn't load (no spells, no
/// `SkillLineAbility`, so no skill slots) kept no skill: the save wrote
/// only the slots. The values loaded are kept for the skills the data
/// gives no slot.
#[tokio::test]
async fn a_login_on_missing_data_keeps_the_saved_skills() {
    let t = Temp::new("skills");
    let realm = server(&t.db(), data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    realm
        .maps
        .get(&realm, MapKey::world(0))
        .query(move |m| {
            let data = m.realm.data.clone();
            crate::game::map::stats::set_level(m.players.get_mut(&g).unwrap(), &data, 20);
            skills::set_value(m.players.get_mut(&g).unwrap(), 43, 37);
        })
        .await
        .unwrap();
    assert_eq!(skill_of(&realm, g, 43).await, Some(37));
    logout(&mut a).await;
    stop(realm, a).await;

    // A world with no data: Alice logs in and out.
    let realm = server(&t.db(), GameData::empty());
    let mut a = Client::connect(&realm, "A");
    a.login(g).await;
    assert_eq!(
        skill_of(&realm, g, 43).await,
        None,
        "no slot without the data"
    );
    logout(&mut a).await;
    stop(realm, a).await;

    // The data back: her swords are as she left them.
    let realm = server(&t.db(), data());
    let mut a = Client::connect(&realm, "A");
    a.login(g).await;
    let swords = skill_of(&realm, g, 43).await;
    stop(realm, a).await;
    assert_eq!(
        swords,
        Some(37),
        "Alice's swords after a login on missing data"
    );
}

/// #6: a skill unlearned isn't kept by the carrying over: it is gone from
/// what was loaded as well as from its slot.
#[tokio::test]
async fn a_skill_forgotten_is_not_carried_over() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let saved = realm
        .maps
        .get(&realm, MapKey::world(0))
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.skills = Some(vec![(43, 5), (95, 5), (164, 120)]);
            skills::forget_skill(p, 43);
            skills::forget_skill(p, 164);
            skills::saved(p)
        })
        .await
        .unwrap();
    assert!(saved.iter().any(|(s, _)| *s == 95), "{saved:?}");
    assert!(
        !saved.iter().any(|(s, _)| *s == 43 || *s == 164),
        "{saved:?}"
    );
}

/// #6: the start refuses data that was looked for and doesn't load, and
/// the empty world over a store with characters; it takes the empty world
/// only for a store with none and no data looked for.
#[test]
fn the_start_refuses_world_data_that_is_missing_or_broken() {
    use crate::game::world_data;
    let t = Temp::new("data");
    // Nothing there.
    let e = world_data(Some(&t.0), 0).unwrap_err();
    assert!(e.contains("vmangos.sqlite is missing"), "{e}");
    // Git LFS pointers, as a clone without Git LFS leaves them.
    for name in ["vmangos.sqlite", "dbc.sqlite"] {
        std::fs::write(
            t.0.join(name),
            "version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 1\n",
        )
        .unwrap();
    }
    let e = world_data(Some(&t.0), 0).unwrap_err();
    assert!(e.contains("Git LFS pointer"), "{e}");
    // Not a database.
    for name in ["vmangos.sqlite", "dbc.sqlite"] {
        std::fs::write(t.0.join(name), vec![7u8; 4096]).unwrap();
    }
    let e = world_data(Some(&t.0), 0).unwrap_err();
    assert!(e.contains("doesn't read"), "{e}");
    // Databases with none of the tables.
    for name in ["vmangos.sqlite", "dbc.sqlite"] {
        std::fs::remove_file(t.0.join(name)).unwrap();
        rusqlite::Connection::open(t.0.join(name))
            .unwrap()
            .execute_batch("CREATE TABLE other (x INTEGER);")
            .unwrap();
    }
    let e = world_data(Some(&t.0), 0).unwrap_err();
    assert!(
        e.contains("no creature templates, spells, areas, skill lines"),
        "{e}"
    );
    assert!(e.contains("Not starting"), "{e}");
    // No data looked for: the empty world, only for a store without
    // characters.
    assert!(world_data(None, 0).is_ok_and(|d| !d.is_loaded()));
    let e = world_data(None, 3).unwrap_err();
    assert!(e.contains("3 characters"), "{e}");
}

/// #6: the real data passes the start's check.
#[test]
#[ignore = "extended"]
fn the_real_data_passes_the_starts_check() {
    let dir = harness::real_data("db");
    let data = crate::game::world_data(Some(&dir), 1).expect("the real data loads");
    assert!(data.is_loaded());
}

async fn delete(c: &mut Client, g: Guid) {
    c.send(CMSG_CHAR_DELETE { guid: g }).await;
    let r = c
        .wait_for("SMSG_CHAR_DELETE", |m| match m {
            ServerOpcodeMessage::SMSG_CHAR_DELETE(d) => Some(d.result),
            _ => None,
        })
        .await;
    assert_eq!(r, WorldResult::CharDeleteSuccess);
}

/// #11: the newest character deleted and the server restarted, the next
/// character made (on any account) had its GUID, and with it what is
/// still kept under it: its honor here. GUIDs, the items' too, are now
/// kept in the store as they are handed out; the honor entry and the raid
/// binds go with the character, as mangos's `DeleteFromDB` has it.
#[tokio::test]
async fn a_deleted_characters_guid_is_never_given_again() {
    use crate::game::guid::counter;
    let t = Temp::new("guid");
    let realm = server(&t.db(), data());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let n = counter(alice);
    {
        let mut h = realm.social.honor.lock();
        h.register(n, 0, 60);
        let e = h.entries.get_mut(&n).unwrap();
        e.lifetime_honorable_kills = 5_000;
        e.highest_rank = 12;
        h.dirty.insert(n);
    }
    realm.social.instances.lock().raids.insert(
        77,
        crate::game::instances::RaidSave {
            id: 77,
            map: 409,
            reset_at: i64::MAX / 2,
            players: vec![n, 999],
        },
    );
    let items = realm.next_item.load(std::sync::atomic::Ordering::Relaxed);
    assert!(items > 1, "Alice has her starting gear");
    logout(&mut a).await;
    delete(&mut a, alice).await;
    assert!(!realm.social.honor.lock().entries.contains_key(&n));
    assert_eq!(realm.social.instances.lock().raids[&77].players, vec![999]);
    stop(realm, a).await;

    let realm = server(&t.db(), data());
    assert!(
        realm.next_item.load(std::sync::atomic::Ordering::Relaxed) >= items,
        "Alice's items' GUIDs are not handed out again"
    );
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    let inherited = realm
        .social
        .honor
        .lock()
        .entries
        .get(&counter(bob))
        .map(|e| (e.lifetime_honorable_kills, e.highest_rank));
    let raids = realm.social.instances.lock().raids.get(&77).cloned();
    stop(realm, b).await;
    assert_ne!(bob, alice, "Bob, on another account, has Alice's GUID");
    assert!(
        inherited.is_none_or(|h| h == (0, 0)),
        "Bob's honor is Alice's: {inherited:?}"
    );
    assert_eq!(raids.map(|r| r.players), Some(vec![999]));
}
