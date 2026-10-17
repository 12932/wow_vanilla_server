//! Event quests (`SpecialFlags` 0x2) against the real data when it is
//! here: each done by what Thistle Tea does for it, its creature, area
//! trigger, gossip, object or escort script.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::{Map, MapHandle, MapKey, quests};

const DB: &str = "<runs>/tt-data/db";

fn have_data() -> bool {
    Path::new(DB).join("vmangos.sqlite").exists()
}

fn realm() -> Arc<Realm> {
    harness::realm_with(crate::game::data::load::load(Path::new(DB)))
}

/// A player of `race` and `class` at `level`, in the world.
async fn hero(
    realm: &Arc<Realm>,
    account: &str,
    race: Race,
    class: Class,
    level: u8,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let name = format!("{}{}", &account[..1], account[1..].to_lowercase());
    let g = c.enter(&name, race, class).await;
    let map = realm.registry.online(g).expect("online").map();
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, level);
    })
    .await;
    (c, g, map)
}

/// The first creature of `entry` alive here, and where it stands.
fn npc(m: &Map, entry: u32) -> Option<(Guid, Vector3d)> {
    let mut all: Vec<_> = m
        .creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| (c.guid, c.position))
        .collect();
    all.sort_by_key(|(g, _)| g.guid());
    all.into_iter().next()
}

/// The nearest object of `entry` here.
fn object(m: &Map, entry: u32) -> Option<(Guid, Vector3d)> {
    let mut all: Vec<_> = m
        .gameobjects
        .iter()
        .filter(|(_, o)| o.entry == entry)
        .map(|(_, o)| (o.guid, o.position))
        .collect();
    all.sort_by_key(|(g, _)| g.guid());
    all.into_iter().next()
}

/// `c` at `at`, at once.
async fn put(map: &MapHandle, c: &mut Client, at: Vector3d) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, at, 0.0)).await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    assert!(
        until(map, move |m| m
            .players
            .get(&g)
            .is_some_and(|p| p.teleport.is_none()))
        .await,
        "teleported"
    );
}

/// Two yards off `at`.
fn beside(at: Vector3d) -> Vector3d {
    Vector3d {
        x: at.x + 2.0,
        y: at.y,
        z: at.z,
    }
}

async fn until(map: &MapHandle, what: impl Fn(&Map) -> bool + Send + Clone + 'static) -> bool {
    for _ in 0..500 {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

fn logged(m: &Map, g: Guid, quest: u32) -> bool {
    m.players[&g].journal.has(quest)
}

fn done(m: &Map, g: Guid, quest: u32) -> bool {
    let p = &m.players[&g];
    let q = m.realm.data.npc.quests.get(&quest).expect("quest");
    p.journal
        .slot_of(quest)
        .is_some_and(|s| quests::complete(p, q, &p.journal.log[s]))
}

fn rewarded(m: &Map, g: Guid, quest: u32) -> bool {
    m.players[&g].journal.rewarded.contains(&quest)
}

/// Take `quest` from the creature (or object) `giver`, beside it, its
/// earlier quests done.

/// Where `entry` (a creature, else an object) is spawned: its map and
/// place.
fn spawn_of(realm: &Realm, entry: u32) -> Option<(u32, Vector3d)> {
    let data = &realm.data;
    let mut creatures: Vec<_> = data
        .creature_spawns
        .values()
        .flatten()
        .filter(|s| s.entries.contains(&entry))
        .map(|s| (s.guid, s.map, s.x, s.y, s.z))
        .collect();
    creatures.sort_by_key(|s| s.0);
    let mut objects: Vec<_> = data
        .gameobject_spawns
        .values()
        .flatten()
        .filter(|s| s.entry == entry)
        .map(|s| (s.guid, s.map, s.x, s.y, s.z))
        .collect();
    objects.sort_by_key(|s| s.0);
    creatures.first().or(objects.first()).map(|s| {
        (
            s.1,
            Vector3d {
                x: s.2,
                y: s.3,
                z: s.4,
            },
        )
    })
}

/// `c` at `at` in map `id`, by a teleport across maps if need be: the
/// map they are in then.
async fn go(realm: &Arc<Realm>, c: &mut Client, id: u32, at: Vector3d) -> MapHandle {
    let g = c.guid;
    let from = realm.registry.online(g).expect("online").map();
    let here = from.query(|m| m.key.map).await.unwrap();
    if here == id {
        put(&from, c, at).await;
        return from;
    }
    from.call(move |m| m.teleport(g, MapKey::world(id), at, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    realm.registry.online(g).expect("online").map()
}

/// `c` beside the spawn of `entry`: the map, the creature or object, and
/// where it stands.
async fn visit(realm: &Arc<Realm>, c: &mut Client, entry: u32) -> (MapHandle, Guid, Vector3d) {
    let (id, at) = spawn_of(realm, entry).expect("spawned");
    let map = go(realm, c, id, beside(at)).await;
    let (who, at) = map
        .query(move |m| npc(m, entry).or_else(|| object(m, entry)))
        .await
        .flatten()
        .expect("here");
    put(&map, c, beside(at)).await;
    (map, who, at)
}

/// Take `quest` from the creature (or object) `giver`, beside it, its
/// earlier quests done: the map it is in.
async fn take(
    realm: &Arc<Realm>,
    c: &mut Client,
    giver: u32,
    quest: u32,
    before: &'static [u32],
) -> MapHandle {
    let g = c.guid;
    let (map, who, _) = visit(realm, c, giver).await;
    map.query(move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .journal
            .rewarded
            .extend(before.iter().copied());
        quests::accept(m, g, who, quest);
    })
    .await;
    assert!(
        until(&map, move |m| logged(m, g, quest)).await,
        "{quest} taken"
    );
    map
}

/// Turn `quest` in at `ender`, beside it.
async fn turn_in(realm: &Arc<Realm>, c: &mut Client, ender: u32, quest: u32) {
    let g = c.guid;
    let (map, who, _) = visit(realm, c, ender).await;
    map.query(move |m| quests::choose_reward(m, g, who, quest, 0))
        .await;
    assert!(
        until(&map, move |m| rewarded(m, g, quest)).await,
        "{quest} rewarded"
    );
}

/// A creature of `entry`, `yards` before `g`.
fn spawn(m: &mut Map, g: Guid, entry: u32, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let t = data.creature_template(entry).cloned().expect("template");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &t,
        t.level_min.max(1),
        at,
        o + std::f32::consts::PI,
    )
    .with_base();
    m.add_creature(c)
}

fn item_at(m: &Map, g: Guid, entry: u32) -> Option<(u8, u8)> {
    m.players[&g]
        .items
        .iter()
        .find(|i| i.entry == entry)
        .map(|i| (i.bag, i.slot))
}

async fn use_on(c: &mut Client, map: &MapHandle, item: u32, target: Guid) {
    let g = c.guid;
    let (bag, slot) = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.caster.gcd_until = None;
            p.caster.cooldowns.clear();
            p.caster.category_cooldowns.clear();
            item_at(m, g, item)
        })
        .await
        .flatten()
        .expect("the item");
    c.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: crate::game::tests_pets::at(target),
    })
    .await;
}

/// Curing the Sick: the salve on a sickly beast cures it, and the cured
/// one counts.
#[allow(clippy::too_many_arguments)]
async fn curing_the_sick(
    realm: &Arc<Realm>,
    account: &str,
    race: Race,
    quest: u32,
    giver: u32,
    before: &'static [u32],
    sickly: u32,
    cured: u32,
) {
    const SALVE: u32 = 15826;
    let (mut c, g, _) = hero(realm, account, race, Class::Druid, 14).await;
    let map = take(realm, &mut c, giver, quest, before).await;
    assert_eq!(
        map.query(move |m| item_at(m, g, SALVE).is_some()).await,
        Some(true)
    );
    for i in 0..10u32 {
        let beast = map.query(move |m| spawn(m, g, sickly, 5.0)).await.unwrap();
        use_on(&mut c, &map, SALVE, beast).await;
        assert!(
            until(&map, move |m| m
                .creature(beast)
                .is_some_and(|c| c.entry == cured))
            .await,
            "{quest}: beast {i} cured"
        );
        assert!(
            until(&map, move |m| {
                let p = &m.players[&g];
                p.journal
                    .slot_of(quest)
                    .is_some_and(|s| p.journal.log[s].kills[0] > i)
            })
            .await,
            "{quest}: beast {i} counted"
        );
    }
    assert_eq!(map.query(move |m| done(m, g, quest)).await, Some(true));
    turn_in(realm, &mut c, 11802, quest).await;
}

/// The id of the option in `npc`'s menu that reads `text`.
async fn option(c: &mut Client, npc: Guid, text: &'static str) -> u32 {
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("the menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => g
            .gossips
            .iter()
            .find(|o| o.message.contains(text))
            .map(|o| o.id),
        _ => None,
    })
    .await
}

async fn choose(c: &Client, npc: Guid, option: u32) {
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: option,
        unknown: None,
    })
    .await;
}

async fn emote_at(c: &Client, emote: TextEmote, target: Guid) {
    c.send(CMSG_TEXT_EMOTE {
        text_emote: emote,
        emote: 0,
        target,
    })
    .await;
}

/// Get the Scoop: beckoned, Plucky Johnson is himself again, and tells
/// the phrase.
async fn get_the_scoop(realm: &Arc<Realm>) {
    const QUEST: u32 = 1950;
    const PLUCKY: u32 = 6626;
    let (mut c, g, _) = hero(realm, "SCOOP", Race::Human, Class::Mage, 30).await;
    take(realm, &mut c, 6548, QUEST, &[1949]).await;
    let (map, plucky, _) = visit(realm, &mut c, PLUCKY).await;
    emote_at(&c, TextEmote::Beckon, plucky).await;
    let id = option(&mut c, plucky, "Phrase").await;
    choose(&c, plucky, id).await;
    assert!(
        until(&map, move |m| done(m, g, QUEST)).await,
        "{QUEST} done"
    );
    turn_in(realm, &mut c, 6548, QUEST).await;
}

/// The Spider God: the Tablet of Theka, read in Zul'Farrak, names him.
async fn the_spider_god(realm: &Arc<Realm>) {
    const QUEST: u32 = 2936;
    const TABLET: u32 = 142715;
    let (mut c, g, _) = hero(realm, "SPIDER", Race::Troll, Class::Shaman, 45).await;
    take(realm, &mut c, 3188, QUEST, &[2935]).await;
    let (map, tablet, _) = visit(realm, &mut c, TABLET).await;
    c.send(CMSG_GAMEOBJ_USE { guid: tablet }).await;
    assert!(
        until(&map, move |m| done(m, g, QUEST)).await,
        "{QUEST} done"
    );
    turn_in(realm, &mut c, 3188, QUEST).await;
}

fn wanted(name: &str) -> bool {
    let yes = std::env::var("EVENTS_ONLY").map_or(true, |v| v.split(',').any(|w| w == name));
    if yes {
        eprintln!("event quests: {name}");
    }
    yes
}

#[tokio::test]
async fn event_quests_are_done_as_thistle_tea_does_them() {
    if !have_data() {
        return;
    }
    let realm = realm();
    if wanted("sick") {
        curing_the_sick(
            &realm,
            "TAUREN",
            Race::Tauren,
            6129,
            3448,
            &[6128],
            12296,
            12297,
        )
        .await;
        curing_the_sick(
            &realm,
            "ELF",
            Race::NightElf,
            6124,
            3702,
            &[6123],
            12298,
            12299,
        )
        .await;
    }
    if wanted("scoop") {
        get_the_scoop(&realm).await;
    }
    if wanted("spider") {
        the_spider_god(&realm).await;
    }
}
