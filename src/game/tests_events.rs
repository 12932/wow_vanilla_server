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

const DB: &str = "/local/home/wallenx/runs/tt-data/db";

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
    // The intro's camera done with: they see from where they stand.
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
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

/// [`until`], free to change the map as it looks.
async fn until_mut(
    map: &MapHandle,
    what: impl Fn(&mut Map) -> bool + Send + Clone + 'static,
) -> bool {
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
    // In sight: a creature nobody sees doesn't run its timers.
    let g = c.guid;
    if guid::is_unit(who) {
        assert!(
            until(&map, move |m| m
                .creature(who)
                .is_some_and(|c| c.observers.contains(&g)))
            .await,
            "{entry} in sight"
        );
    }
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
        if std::env::var("EVENTS_DEBUG").is_ok() {
            let data = m.realm.data.clone();
            let p = &m.players[&g];
            let q = data.npc.quests.get(&quest).unwrap();
            eprintln!(
                "take {quest}: can_take {}, condition {}, level {}, skill {:?}",
                quests::can_take(p, q, &data),
                quests::condition_ok(m, g, who, quest),
                p.level(),
                (q.required_skill != 0)
                    .then(|| crate::game::map::skills::value_with_bonuses(p, q.required_skill)),
            );
        }
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

fn health(m: &Map, g: Guid) -> (u32, u32) {
    m.unit_fields(g).map_or((0, 0), |f| {
        (
            f.get_u32(crate::game::fields::UNIT_FIELD_HEALTH),
            f.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH),
        )
    })
}

/// `g` hits `victim` until `stop` holds (or it is dead): a tenth of its
/// health at a time.
async fn beat(
    map: &MapHandle,
    g: Guid,
    victim: Guid,
    stop: impl Fn(&Map) -> bool + Send + Clone + 'static,
) {
    for _ in 0..40 {
        let s = stop.clone();
        let over = map
            .query(move |m| {
                if s(m) || health(m, victim).0 == 0 {
                    return true;
                }
                let blow = (health(m, victim).1 / 10).max(1);
                crate::game::map::combat::deal_damage(m, g, victim, blow);
                false
            })
            .await
            .unwrap();
        if over {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn hostile(m: &Map, a: Guid, b: Guid) -> bool {
    crate::game::map::combat::can_attack(m, a, b)
}

/// Beat Bartleby: taking the quest sets him on the warrior; beaten to his
/// last fifteen percent, he yields.
async fn beat_bartleby(realm: &Arc<Realm>) {
    const QUEST: u32 = 1640;
    const BARTLEBY: u32 = 6090;
    let (mut c, g, _) = hero(realm, "BRAWLER", Race::Human, Class::Warrior, 10).await;
    let map = take(realm, &mut c, BARTLEBY, QUEST, &[1639]).await;
    let bartleby = map.query(|m| npc(m, BARTLEBY)).await.flatten().unwrap().0;
    assert!(
        until(&map, move |m| hostile(m, bartleby, g)).await,
        "Bartleby turns on the warrior"
    );
    beat(&map, g, bartleby, move |m| done(m, g, QUEST)).await;
    let after = map
        .query(move |m| (done(m, g, QUEST), health(m, bartleby).0 > 0))
        .await
        .unwrap();
    assert_eq!(after, (true, true), "he yields, alive");
    assert!(
        until(&map, move |m| !hostile(m, bartleby, g)).await,
        "and is a friend again"
    );
    turn_in(realm, &mut c, BARTLEBY, QUEST).await;
}

/// The cast under way ends now.
fn finish_cast(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m
        .players
        .get_mut(&g)
        .and_then(|p| p.caster.casting.as_mut())
    {
        c.ends = now;
    }
    crate::game::map::spells::update(m);
}

/// `g` knows profession or secondary skill `skill` at `value`.
fn skilled(m: &mut Map, g: Guid, skill: u32, value: u32) {
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::skills::learn_skill(p, skill, 4);
    crate::game::map::skills::set_value(p, skill, value);
}

/// Every timer of creature `g`'s script due now.
fn hurry(m: &mut Map, g: Guid) {
    if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
        for t in s.timers.iter_mut() {
            *t = None;
        }
    }
}

/// Triage: the doctor lays the wounded in his bunks; fifteen bandaged
/// before six die do it.
async fn triage(realm: &Arc<Realm>) {
    const QUEST: u32 = 6624;
    const DOCTOR: u32 = 12939;
    const BANDAGE: u32 = 16991;
    const WOUNDED: [u32; 3] = [12938, 12936, 12937];
    let (mut c, g, map) = hero(realm, "MEDIC", Race::Human, Class::Priest, 45).await;
    map.query(move |m| skilled(m, g, 129, 225)).await;
    let map = take(realm, &mut c, DOCTOR, QUEST, &[]).await;
    let doctor = map.query(|m| npc(m, DOCTOR)).await.flatten().unwrap().0;
    let mut bandaged: Vec<Guid> = Vec::new();
    for _ in 0..200 {
        if map.query(move |m| done(m, g, QUEST)).await.unwrap() {
            break;
        }
        let seen = bandaged.clone();
        let next = map
            .query(move |m| {
                hurry(m, doctor);
                m.creatures
                    .iter()
                    .filter(|(_, c)| WOUNDED.contains(&c.entry) && c.alive())
                    .map(|(_, c)| c.guid)
                    .find(|w| !seen.contains(w))
            })
            .await
            .flatten();
        let Some(wounded) = next else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        use_on(&mut c, &map, BANDAGE, wounded).await;
        until(&map, move |m| {
            m.players[&g]
                .caster
                .casting
                .as_ref()
                .is_some_and(|c| c.targets.unit == Some(wounded))
        })
        .await;
        map.query(move |m| finish_cast(m, g)).await;
        bandaged.push(wounded);
    }
    assert!(
        map.query(move |m| done(m, g, QUEST)).await.unwrap(),
        "{QUEST} done, {} bandaged",
        bandaged.len()
    );
    turn_in(realm, &mut c, DOCTOR, QUEST).await;
}

/// An escort under way, hurried: the escortee walks fast, sets out at
/// once, and its player keeps up, until `quest` is done and the escort
/// over (leaving it sooner fails the quest).
async fn hurry_escort(map: &MapHandle, g: Guid, escortee: u32, quest: u32) -> bool {
    // A minute at most: a loaded machine can take a while over the walk.
    for _ in 0..2000 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let over = map
            .query(move |m| {
                let who = m
                    .creatures
                    .iter()
                    .filter(|(_, c)| c.entry == escortee && c.alive())
                    .map(|(_, c)| c.guid)
                    .find(|c| m.escorts.iter().any(|e| e.creature == *c));
                if let Some(c) = who.and_then(|w| m.creature_mut(w)) {
                    c.speeds.walk = 60.0;
                    c.speeds.run = 60.0;
                    let at = c.position;
                    if let Some(p) = m.players.get_mut(&g) {
                        p.movement.position = at;
                    }
                }
                for e in m.escorts.iter_mut() {
                    e.skip_start();
                }
                done(m, g, quest) && !m.escorts.iter().any(|e| e.quest == quest)
            })
            .await
            .unwrap_or(false);
        if over {
            return true;
        }
    }
    false
}

/// Stinky's Escape: "Stinky" Ignatz led out of the swamp.
async fn stinkys_escape(realm: &Arc<Realm>) {
    const STINKY: u32 = 4880;
    for (account, race, quest, ender) in [
        ("SWAMPER", Race::Orc, 1270, 3446),
        ("BOGGER", Race::Human, 1222, 4794),
    ] {
        let (mut c, g, _) = hero(realm, account, race, Class::Warrior, 37).await;
        let map = take(realm, &mut c, STINKY, quest, &[]).await;
        assert!(
            hurry_escort(&map, g, STINKY, quest).await,
            "{quest} done at the credit point"
        );
        turn_in(realm, &mut c, ender, quest).await;
        // The escort's end despawns him on his spawn's timer (three
        // minutes), as Thistle Tea's; the next walk wants him back now.
        let _ = map
            .query(|m| {
                let dead: Vec<Guid> = m
                    .creatures
                    .iter()
                    .filter(|(_, c)| c.entry == STINKY && !c.alive())
                    .map(|(_, c)| c.guid)
                    .collect();
                for g in dead {
                    crate::game::map::ai::respawn_now(m, g);
                }
            })
            .await;
    }
}

/// `quest` in `g`'s log, as though taken.
fn logs(m: &mut Map, g: Guid, quest: u32) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.journal.log.push(crate::game::store::Logged {
        quest,
        ..Default::default()
    });
    quests::refresh_log(p, &data);
}

/// Honoring a Hero: a tribute offered at Uther's tomb or Grom's monument,
/// in the Harvest Festival, which brings its quest givers.
async fn honoring_a_hero(realm: &Arc<Realm>) {
    for (account, race, quest, tribute, statue) in [
        ("DWARFY", Race::Dwarf, 8149, 19850, 180204),
        ("TROLLY", Race::Troll, 8150, 19851, 180205),
    ] {
        let (mut c, g, map) = hero(realm, account, race, Class::Warrior, 30).await;
        map.query(move |m| {
            logs(m, g, quest);
            crate::game::map::inventory::add_item(m, g, tribute, 1, false).expect("room");
        })
        .await;
        let (map, _, _) = visit(realm, &mut c, statue).await;
        use_on(&mut c, &map, tribute, g).await;
        until(&map, move |m| m.players[&g].caster.casting.is_some()).await;
        map.query(move |m| finish_cast(m, g)).await;
        assert!(
            until(&map, move |m| done(m, g, quest)).await,
            "{quest} done"
        );
    }
}

/// `c` steps into area trigger `id`: the map it is in.
async fn step_in(realm: &Arc<Realm>, c: &mut Client, id: u32) -> MapHandle {
    let t = realm
        .data
        .area_triggers
        .get(&id)
        .cloned()
        .expect("the trigger");
    let map = go(
        realm,
        c,
        t.map,
        Vector3d {
            x: t.x,
            y: t.y,
            z: t.z,
        },
    )
    .await;
    c.send(CMSG_AREATRIGGER { trigger_id: id }).await;
    map
}

/// Quests done by reaching a place, as Thistle Tea's area trigger scripts
/// have them: Resupplying the Excavation, at Huldar and Miran's camp, and
/// the Missing Diplomat's Sentry Point, where Tervosh comes to hear it.
async fn reaching_a_place(realm: &Arc<Realm>) {
    for (account, race, level, giver, quest, before, trigger, ender) in [
        (
            "SUPPLIER",
            Race::Dwarf,
            12,
            1105,
            273,
            &[302u32][..],
            171,
            2057,
        ),
        (
            "DIPLOMAT",
            Race::Human,
            30,
            4964,
            1265,
            &[1264][..],
            1667,
            4967,
        ),
    ] {
        let before: &'static [u32] = Box::leak(before.to_vec().into_boxed_slice());
        let (mut c, g, _) = hero(realm, account, race, Class::Warrior, level).await;
        take(realm, &mut c, giver, quest, before).await;
        let map = step_in(realm, &mut c, trigger).await;
        assert!(
            until(&map, move |m| done(m, g, quest)).await,
            "{quest} done"
        );
        turn_in(realm, &mut c, ender, quest).await;
    }
}

/// Children's Week: an orphan taken to the sights it asks for.
async fn childrens_week(realm: &Arc<Realm>) {
    for (account, race, whistle, orphan, quest, trigger) in [
        ("WARDEN", Race::Human, 18598, 14305, 1479, 3546),
        ("MATRON", Race::Orc, 18597, 14444, 1800, 3547),
    ] {
        let before: &'static [u32] = if orphan == 14305 { &[1468] } else { &[172] };
        let (mut c, g, map) = hero(realm, account, race, Class::Warrior, 10).await;
        map.query(move |m| {
            crate::game::map::inventory::add_item(m, g, whistle, 1, false).expect("room");
        })
        .await;
        // At the sight, the orphan called.
        let t = realm.data.area_triggers.get(&trigger).cloned().unwrap();
        let map = go(
            realm,
            &mut c,
            t.map,
            Vector3d {
                x: t.x,
                y: t.y,
                z: t.z,
            },
        )
        .await;
        use_on(&mut c, &map, whistle, g).await;
        assert!(
            until(&map, move |m| m.players[&g].owned.critter.is_some()).await,
            "the orphan comes"
        );
        let child = map
            .query(move |m| m.players[&g].owned.critter)
            .await
            .flatten()
            .unwrap();
        let entry = map
            .query(move |m| m.creature(child).map(|c| c.entry))
            .await
            .flatten();
        assert_eq!(entry, Some(orphan));
        map.query(move |m| {
            m.players
                .get_mut(&g)
                .unwrap()
                .journal
                .rewarded
                .extend(before.iter().copied());
            quests::accept(m, g, child, quest);
        })
        .await;
        assert!(
            until(&map, move |m| logged(m, g, quest)).await,
            "{quest} taken"
        );
        c.send(CMSG_AREATRIGGER {
            trigger_id: trigger,
        })
        .await;
        assert!(
            until(&map, move |m| done(m, g, quest)).await,
            "{quest} done"
        );
        map.query(move |m| quests::choose_reward(m, g, child, quest, 0))
            .await;
        assert!(
            until(&map, move |m| rewarded(m, g, quest)).await,
            "{quest} rewarded"
        );
    }
}

/// The map's scripts and creature timers `by` on: what waits on them
/// comes sooner.
fn skip(m: &mut Map, by: Duration) {
    for r in m.scripts.runs.iter_mut() {
        if let Some(t) = r.started.checked_sub(by) {
            r.started = t;
        }
    }
    let all: Vec<Guid> = m.creatures.iter().map(|(_, c)| c.guid).collect();
    for g in all {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
            for t in s.timers.iter_mut().flatten() {
                if let Some(sooner) = t.checked_sub(by) {
                    *t = sooner;
                }
            }
        }
    }
}

/// `g` holds out with `defended` against what comes for it, killing the
/// `attackers` as they come, until `quest` is done.
async fn hold_out(
    map: &MapHandle,
    g: Guid,
    defended: Guid,
    attackers: &'static [u32],
    quest: u32,
) -> bool {
    for _ in 0..400 {
        let over = map
            .query(move |m| {
                skip(m, Duration::from_secs(5));
                let foes: Vec<Guid> = m
                    .creatures
                    .iter()
                    .filter(|(_, c)| attackers.contains(&c.entry) && c.alive())
                    .map(|(_, c)| c.guid)
                    .collect();
                let n = foes.len();
                for f in foes {
                    let hp = health(m, f).0;
                    crate::game::map::combat::deal_damage(m, g, f, hp);
                }
                if std::env::var("EVENTS_DEBUG").is_ok() {
                    let phase =
                        crate::game::map::event_ai::scripted_mut(m, defended).map(|s| s.phase);
                    let active = crate::game::map::map_events::active(m, quest);
                    let runs = m.scripts.runs.len();
                    eprintln!(
                        "hold_out {quest}: {n} foes, phase {phase:?}, event {active}, {runs} runs"
                    );
                }
                // Those it fights don't wear it down meanwhile.
                if let Some(c) = m.creature_mut(defended) {
                    let max = c.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH);
                    c.fields
                        .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, max);
                }
                done(m, g, quest)
            })
            .await
            .unwrap_or(false);
        if over {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Defenses: Shakes O'Breen against the naga (Death From Below), Piznik
/// against the Windshear goblins (Gerenzo's Orders).
async fn defenses(realm: &Arc<Realm>) {
    for (account, level, giver, quest, before, attackers) in [
        ("SAILOR", 40, 2610, 667, &[670u32][..], &[2595u32, 2596][..]),
        ("MINER", 20, 4276, 1090, &[][..], &[3998, 4001, 4003][..]),
    ] {
        let before: &'static [u32] = Box::leak(before.to_vec().into_boxed_slice());
        let attackers: &'static [u32] = Box::leak(attackers.to_vec().into_boxed_slice());
        let (mut c, g, _) = hero(realm, account, Race::Human, Class::Warrior, level).await;
        let map = take(realm, &mut c, giver, quest, before).await;
        let defended = map.query(move |m| npc(m, giver)).await.flatten().unwrap().0;
        map.query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.fields
                .set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
            p.fields
                .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
        })
        .await;
        assert!(
            hold_out(&map, g, defended, attackers, quest).await,
            "{quest} done"
        );
        turn_in(realm, &mut c, giver, quest).await;
    }
}

/// The Missing Diplomat's brawl: Dashel Stonefist and his thugs, until he
/// gives up at a fifth of his health.
async fn dashels_brawl(realm: &Arc<Realm>) {
    const QUEST: u32 = 1447;
    const DASHEL: u32 = 4961;
    let (mut c, g, _) = hero(realm, "BRUISER", Race::Human, Class::Warrior, 30).await;
    let map = take(realm, &mut c, DASHEL, QUEST, &[1246]).await;
    let dashel = map.query(|m| npc(m, DASHEL)).await.flatten().unwrap().0;
    map.query(move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.fields
            .set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
        p.fields
            .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
    })
    .await;
    assert!(
        until(&map, move |m| hostile(m, dashel, g)).await,
        "Dashel turns on the player"
    );
    beat(&map, g, dashel, move |m| done(m, g, QUEST)).await;
    assert!(
        until_mut(&map, move |m| {
            skip(m, Duration::from_secs(2));
            done(m, g, QUEST)
        })
        .await,
        "{QUEST} done"
    );
    turn_in(realm, &mut c, DASHEL, QUEST).await;
}

/// Quests the world database does: The Fargodeep Mine, explored by its
/// area trigger, and Expert Blacksmith!, whose start script ends in its
/// event.
async fn the_databases_own(realm: &Arc<Realm>) {
    const MINE: u32 = 62;
    let (mut c, g, _) = hero(realm, "DELVER", Race::Human, Class::Warrior, 5).await;
    take(realm, &mut c, 240, MINE, &[]).await;
    let map = step_in(realm, &mut c, 88).await;
    assert!(until(&map, move |m| done(m, g, MINE)).await, "{MINE} done");
    turn_in(realm, &mut c, 240, MINE).await;

    const SMITH: u32 = 2765;
    let (mut c, g, map) = hero(realm, "SMITH", Race::Dwarf, Class::Warrior, 40).await;
    map.query(move |m| skilled(m, g, 164, 225)).await;
    let map = take(realm, &mut c, 7802, SMITH, &[2761, 2762, 2763]).await;
    assert!(
        until_mut(&map, move |m| {
            skip(m, Duration::from_secs(5));
            done(m, g, SMITH)
        })
        .await,
        "{SMITH} done"
    );
    turn_in(realm, &mut c, 7802, SMITH).await;
}

fn wanted(name: &str) -> bool {
    let yes = std::env::var("EVENTS_ONLY").map_or(true, |v| v.split(',').any(|w| w == name));
    if yes {
        eprintln!("event quests: {name}");
    }
    yes
}

/// Creatures, gossip, objects, spells and places.
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
    if wanted("bartleby") {
        beat_bartleby(&realm).await;
    }
    if wanted("tribute") {
        honoring_a_hero(&realm).await;
    }
    if wanted("places") {
        reaching_a_place(&realm).await;
    }
    if wanted("children") {
        childrens_week(&realm).await;
    }
}

/// Map events: a doctor's patients, an escort, defenses and a brawl.
#[tokio::test]
async fn event_quests_with_their_map_events_are_done() {
    if !have_data() {
        return;
    }
    let realm = realm();
    if wanted("triage") {
        triage(&realm).await;
    }
    if wanted("stinky") {
        stinkys_escape(&realm).await;
    }
    if wanted("defenses") {
        defenses(&realm).await;
    }
    if wanted("dashel") {
        dashels_brawl(&realm).await;
    }
    if wanted("database") {
        the_databases_own(&realm).await;
    }
}

/// A quest's details go out as 1.12 has them: its long details whole, its
/// Accept button on, each reward item with its display id, the money it
/// asks for, its emotes up to the last.
#[test]
fn a_quests_details_are_laid_out_as_the_client_reads_them() {
    use crate::game::data::GameData;
    use crate::game::data::items::ItemTemplate;
    use crate::game::data::npc::Quest;
    let mut data = GameData::empty();
    data.items.insert(
        2589,
        ItemTemplate {
            entry: 2589,
            display_id: 7341,
            ..Default::default()
        },
    );
    let mut q = Quest {
        id: 6084,
        title: "Taming the Beast".into(),
        details: "x".repeat(466),
        objectives: "o".repeat(107),
        reward_money: -50,
        reward_spell: 1579,
        ..Default::default()
    };
    q.rewards[0] = (2589, 2);
    q.details_emotes = [(1, 0), (0, 0), (5, 500), (0, 0)];
    let giver = Guid::new(0xF130_0004_CF00_0001);
    let frame = quests::details_frame(&data, giver, &q);
    let body = &frame[4..];
    let strings = 17 + 467 + 108;
    assert_eq!(
        body.len(),
        8 + 4 + strings + 4 + 4 + 4 + 12 + 4 + 4 + 4 + 3 * 8
    );
    let at = 12 + strings;
    let word = |i: usize| u32::from_le_bytes(body[at + i * 4..at + i * 4 + 4].try_into().unwrap());
    // Accept on, no choices, one item: 2589, two of them, its display.
    assert_eq!(
        (word(0), word(1), word(2), word(3), word(4), word(5)),
        (1, 0, 1, 2589, 2, 7341)
    );
    assert_eq!((word(6) as i32, word(7), word(8)), (-50, 1579, 3));
    let Some(ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_DETAILS(d)) =
        harness::quest_details_112(&frame)
    else {
        panic!("read back");
    };
    assert_eq!((d.guid, d.quest_id, d.details.len()), (giver, 6084, 466));
    assert_eq!(d.item_rewards.len(), 1);
    assert_eq!(d.emotes.len(), 3);
}
