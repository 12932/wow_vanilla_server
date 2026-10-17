//! Game objects through the protocol: the objects spells summon (a mage's
//! portal, a campfire, a hunter's trap, a summoning portal), what objects
//! do when used (pages, quest objects, doors that wait on a spawn), the
//! spell that works an object, and the quests that ask for an object.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::objects::*;
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapKey};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const PORTAL_SPELL: u32 = 10_059;
const PORTAL: u32 = 176_296;
const PORTAL_TELEPORT: u32 = 17_334;
const SHORT_PORTAL_SPELL: u32 = 900_200;
const CAMPFIRE_SPELL: u32 = 818;
const CAMPFIRE: u32 = 29_784;
const CAMPFIRE_FIRE: u32 = 31_442;
const COOKING: u32 = 900_201;
const TRAP_SPELL: u32 = 1499;
const TRAP: u32 = 2561;
const TRAP_EFFECT: u32 = 3355;
const RITUAL_SPELL: u32 = 698;
const SUMMONING_PORTAL: u32 = 36_727;
const SUMMONS: u32 = 7720;
const GOOBER: u32 = 900_300;
const BOOK: u32 = 900_301;
const DOOR: u32 = 900_302;
const BUTTON: u32 = 900_303;
const WORK_BUTTON: u32 = 900_202;
const GOOBER_QUEST: u32 = 9001;
const BUTTON_QUEST: u32 = 9002;
const GUARD: u32 = 900_400;
const MONSTER: u32 = 900_401;
/// Where the portal sends.
const AWAY: (f32, f32, f32) = (-9000.0, -100.0, 84.0);

fn spell(id: u32, effect: u32, target_a: u32, misc: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect,
        target_a,
        misc_value: misc,
        radius_index: 7,
        ..Default::default()
    };
    s
}

fn object(entry: u32, kind: u8, name: &str, data: &[(usize, u32)]) -> GameObjectTemplate {
    let mut t = GameObjectTemplate {
        entry,
        kind,
        display_id: 1,
        name: name.into(),
        size: 1.0,
        ..Default::default()
    };
    for (i, v) in data {
        t.data[*i] = *v;
    }
    t
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.spells.radii.insert(7, 3.0);
    for (index, ms) in [(3, 60_000), (6, 10_000), (8, 2_000), (9, 400)] {
        d.spells.durations.insert(
            index,
            SpellDuration {
                base: ms,
                ..Default::default()
            },
        );
    }
    d.spells.ranges.insert(1, (0.0, 30.0));
    // A mage's portal: two uses, for the mage's group; and one that lasts
    // no time at all.
    let mut portal = spell(PORTAL_SPELL, EFFECT_TRANS_DOOR, 47, PORTAL as i32);
    portal.duration_index = 3;
    let mut short = spell(SHORT_PORTAL_SPELL, EFFECT_TRANS_DOOR, 47, PORTAL as i32);
    short.duration_index = 9;
    d.spells.spells.insert(PORTAL_SPELL, portal);
    d.spells.spells.insert(SHORT_PORTAL_SPELL, short);
    d.spells
        .spells
        .insert(PORTAL_TELEPORT, spell(PORTAL_TELEPORT, 5, 1, 0));
    d.spells
        .target_positions
        .insert(PORTAL_TELEPORT, (0, AWAY.0, AWAY.1, AWAY.2, 0.0));
    d.gameobject_templates.insert(
        PORTAL,
        object(
            PORTAL,
            GO_SPELLCASTER,
            "Portal to Stormwind",
            &[(0, PORTAL_TELEPORT), (1, 2), (2, 1)],
        ),
    );
    // A campfire, a focus for cooking, with its fire.
    let mut campfire = spell(CAMPFIRE_SPELL, EFFECT_TRANS_DOOR, 32, CAMPFIRE as i32);
    campfire.duration_index = 9;
    d.spells.spells.insert(CAMPFIRE_SPELL, campfire);
    d.gameobject_templates.insert(
        CAMPFIRE,
        object(
            CAMPFIRE,
            GO_SPELL_FOCUS,
            "Basic Campfire",
            &[(0, 4), (1, 10), (2, CAMPFIRE_FIRE)],
        ),
    );
    d.gameobject_templates.insert(
        CAMPFIRE_FIRE,
        object(CAMPFIRE_FIRE, GO_TRAP, "Campfire Damage", &[(2, 1)]),
    );
    let mut cooking = spell(COOKING, 0, 0, 0);
    cooking.requires_spell_focus = 4;
    d.spells.spells.insert(COOKING, cooking);
    // A freezing trap: armed a second after it is set, one spring.
    let mut trap = spell(TRAP_SPELL, EFFECT_SUMMON_OBJECT_SLOT1, 47, TRAP as i32);
    trap.duration_index = 3;
    d.spells.spells.insert(TRAP_SPELL, trap);
    let mut freeze = spell(TRAP_EFFECT, 6, 6, 0);
    freeze.effects[0].aura = 12;
    freeze.duration_index = 8;
    d.spells.spells.insert(TRAP_EFFECT, freeze);
    d.gameobject_templates.insert(
        TRAP,
        object(
            TRAP,
            GO_TRAP,
            "Freezing Trap",
            &[(0, 12), (2, 5), (3, TRAP_EFFECT), (4, 1), (7, 1), (9, 1)],
        ),
    );
    // The Ritual of Summoning: its portal wants three, its owner one of
    // them, all of a group.
    let mut ritual = spell(RITUAL_SPELL, EFFECT_TRANS_DOOR, 47, SUMMONING_PORTAL as i32);
    ritual.duration_index = 6;
    ritual.attributes[1] |= 0x4;
    d.spells.spells.insert(RITUAL_SPELL, ritual);
    d.spells
        .spells
        .insert(SUMMONS, spell(SUMMONS, EFFECT_SUMMON_PLAYER, 0, 0));
    d.gameobject_templates.insert(
        SUMMONING_PORTAL,
        object(
            SUMMONING_PORTAL,
            GO_RITUAL,
            "Summoning Portal",
            &[(0, 3), (1, SUMMONS), (2, RITUAL_SPELL), (6, 1)],
        ),
    );
    // A quest object with a page, for those on its quest; a book; a door
    // that opens once its guard is dead, for a second; a button a spell
    // works.
    d.gameobject_templates.insert(
        GOOBER,
        object(
            GOOBER,
            GO_GOOBER,
            "Strange Altar",
            &[(1, GOOBER_QUEST), (3, 2 * 0x10000), (7, 50)],
        ),
    );
    d.gameobject_templates
        .insert(BOOK, object(BOOK, GO_TEXT, "Old Book", &[(0, 51)]));
    d.gameobject_templates
        .insert(DOOR, object(DOOR, GO_DOOR, "Gate", &[(2, 0x10000)]));
    d.gameobject_templates
        .insert(BUTTON, object(BUTTON, GO_BUTTON, "Lever", &[]));
    let mut work = spell(WORK_BUTTON, EFFECT_ACTIVATE_OBJECT, 40, 16);
    work.effects[0].radius_index = 0;
    d.spells.spells.insert(WORK_BUTTON, work);
    d.spells
        .script_targets
        .insert(WORK_BUTTON, vec![(0, BUTTON)]);
    let near = |guid, entry, dx: f32, state| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1 + 2.0,
        z: START.2,
        state,
        respawn_min: 60,
        ..Default::default()
    };
    d.gameobject_spawns.insert(
        0,
        vec![
            near(500, GOOBER, 0.0, 1),
            near(501, BOOK, 1.0, 1),
            near(502, DOOR, -1.0, 1),
            near(503, BUTTON, 2.0, 1),
        ],
    );
    d.object_requirements.insert(502, (0, 600));
    let quest = |id, objective: (i32, u32), cast| Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        flags: 8,
        required_kills: [objective, (0, 0), (0, 0), (0, 0)],
        required_spell_casts: [cast, 0, 0, 0],
        ..Default::default()
    };
    d.npc
        .quests
        .insert(GOOBER_QUEST, quest(GOOBER_QUEST, (-(GOOBER as i32), 1), 0));
    d.npc.quests.insert(
        BUTTON_QUEST,
        quest(BUTTON_QUEST, (-(BUTTON as i32), 1), WORK_BUTTON),
    );
    // Factions: the players' (Human 1, Orc 2), and a monster's.
    for (id, ours, friends, enemies) in [(1, 1, 1, 8 | 2), (2, 2, 2, 8 | 1), (14, 8, 8, 1 | 2)] {
        d.faction_templates.insert(
            id,
            FactionTemplate {
                id,
                faction: id + 100,
                ours,
                friend_mask: friends,
                enemy_mask: enemies,
                ..Default::default()
            },
        );
    }
    let creature = |entry, name: &str| CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction: 14,
        unit_class: 1,
        health_max: 500,
        speed_walk: 0.01,
        speed_run: 0.01,
        detection_range: 0.1,
        ..Default::default()
    };
    d.creature_templates
        .insert(GUARD, creature(GUARD, "Gate Guard"));
    d.creature_templates
        .insert(MONSTER, creature(MONSTER, "Wolf"));
    d.creature_spawns.insert(
        0,
        vec![
            CreatureSpawn {
                guid: 600,
                entries: vec![GUARD],
                map: 0,
                x: START.0 - 20.0,
                y: START.1 - 20.0,
                z: START.2,
                respawn_min: 600,
                respawn_max: 600,
                ..Default::default()
            },
            // Three yards ahead of where the hunter faces (east).
            CreatureSpawn {
                guid: 601,
                entries: vec![MONSTER],
                map: 0,
                x: START.0 + 3.0,
                y: START.1 - 30.0,
                z: START.2,
                respawn_min: 600,
                respawn_max: 600,
                ..Default::default()
            },
        ],
    );
    d
}

fn map0(realm: &Arc<Realm>) -> crate::game::map::MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map0(realm).query(f).await.expect("the map answers")
}

async fn grouped(realm: &Arc<Realm>, names: &[&str]) -> Vec<Client> {
    let mut out = Vec::new();
    for name in names {
        let mut c = Client::connect(realm, name);
        c.enter(name, Race::Human, Class::Warrior).await;
        out.push(c);
    }
    for i in 1..out.len() {
        let name = names[i].to_owned();
        out[0].send(CMSG_GROUP_INVITE { name }).await;
        let who = &mut out[i];
        who.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        who.send(CMSG_GROUP_ACCEPT {}).await;
        who.wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
    }
    out
}

/// The objects of `entry` on the map.
async fn objects_of(realm: &Arc<Realm>, entry: u32) -> Vec<Guid> {
    on_map(realm, move |m| {
        m.gameobjects
            .iter()
            .filter(|(_, o)| o.entry == entry)
            .map(|(_, o)| o.guid)
            .collect()
    })
    .await
}

async fn cast(realm: &Arc<Realm>, g: Guid, spell: u32) {
    on_map(realm, move |m| {
        // Facing east, where the objects go.
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.orientation = 0.0;
        }
        crate::game::map::spells::cast_triggered(m, g, spell, None)
    })
    .await;
}

async fn put(realm: &Arc<Realm>, g: Guid, x: f32, y: f32, z: f32) {
    on_map(realm, move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.position = Vector3d { x, y, z };
        }
    })
    .await;
}

async fn position(realm: &Arc<Realm>, g: Guid) -> Vector3d {
    on_map(realm, move |m| m.position_of(g).unwrap()).await
}

async fn wait_until(what: &str, mut f: impl AsyncFnMut() -> bool) {
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never: {what}");
}

#[tokio::test]
async fn a_mages_portal_is_their_groups_until_its_charges_run_out() {
    let realm = harness::realm_with(data());
    let mut cs = grouped(&realm, &["Alice", "Bob"]).await;
    let mut eve = Client::connect(&realm, "E");
    let ge = eve.enter("Eve", Race::Human, Class::Warrior).await;
    let (ga, gb) = (cs[0].guid, cs[1].guid);
    cast(&realm, ga, PORTAL_SPELL).await;
    let portal = objects_of(&realm, PORTAL).await;
    assert_eq!(portal.len(), 1);
    let portal = portal[0];
    // Alice's, at her level, three yards ahead of her.
    let (by, level, at) = on_map(&realm, move |m| {
        let o = m.gameobject(portal).unwrap();
        (
            o.fields.get_guid(OBJECT_FIELD_CREATED_BY),
            o.fields.get_u32(GAMEOBJECT_LEVEL),
            o.position,
        )
    })
    .await;
    let alice = position(&realm, ga).await;
    assert_eq!((by, level), (ga, 1));
    assert!(
        (at.x - alice.x - 3.0).abs() < 0.01 && (at.y - alice.y).abs() < 0.01,
        "{at:?}"
    );
    for c in cs.iter_mut() {
        c.sees(portal).await;
    }
    eve.sees(portal).await;
    // Not Eve's to take; Bob's and Alice's.
    eve.send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        (position(&realm, ge).await.x - AWAY.0).abs() > 1.0,
        "Eve stays"
    );
    cs[1].send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    wait_until("Bob goes through", async || {
        (position(&realm, gb).await.x - AWAY.0).abs() < 0.1
    })
    .await;
    cs[0].send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    cs[0]
        .wait_for("the portal spent", |m| match m {
            ServerOpcodeMessage::SMSG_GAMEOBJECT_DESPAWN_ANIM(d) if d.guid == portal => Some(()),
            _ => None,
        })
        .await;
    assert!(
        objects_of(&realm, PORTAL).await.is_empty(),
        "two uses and it is gone"
    );
    // One whose time is up goes too.
    put(&realm, ga, START.0, START.1, START.2).await;
    cast(&realm, ga, SHORT_PORTAL_SPELL).await;
    assert_eq!(objects_of(&realm, PORTAL).await.len(), 1);
    wait_until("the short portal goes", async || {
        objects_of(&realm, PORTAL).await.is_empty()
    })
    .await;
}

/// Why `g` can't cook now, if they can't.
async fn focused(realm: &Arc<Realm>, g: Guid) -> Option<u8> {
    on_map(realm, move |m| {
        if let Some(p) = m.players.get_mut(&g)
            && !p.spells.contains(&COOKING)
        {
            p.spells.push(COOKING);
        }
        let s = m.realm.data.spells.get(COOKING).cloned().unwrap();
        let mods = crate::game::map::spells::mods::for_spell(m, g, &s);
        crate::game::map::spells::checks::check(m, g, &s, &Targets::default(), None, false, &mods)
    })
    .await
}

#[tokio::test]
async fn a_campfire_is_a_cooking_focus_with_its_fire_until_it_burns_out() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    assert_eq!(
        focused(&realm, g).await,
        Some(crate::game::map::spells::checks::fail::REQUIRES_SPELL_FOCUS),
        "no fire, no cooking"
    );
    cast(&realm, g, CAMPFIRE_SPELL).await;
    let fire = objects_of(&realm, CAMPFIRE).await;
    let flames = objects_of(&realm, CAMPFIRE_FIRE).await;
    assert_eq!(
        (fire.len(), flames.len()),
        (1, 1),
        "the campfire and its fire"
    );
    assert_eq!(focused(&realm, g).await, None, "cooking by the fire");
    wait_until("the campfire burns out", async || {
        objects_of(&realm, CAMPFIRE).await.is_empty()
            && objects_of(&realm, CAMPFIRE_FIRE).await.is_empty()
    })
    .await;
}

async fn frozen(realm: &Arc<Realm>, g: Guid) -> bool {
    on_map(realm, move |m| {
        crate::game::map::auras::auras(m, g)
            .is_some_and(|a| a.holders.iter().any(|h| h.spell == TRAP_EFFECT))
    })
    .await
}

#[tokio::test]
async fn a_hunters_trap_arms_springs_on_an_enemy_and_gives_way_to_the_next() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let wolf = guid::unit(MONSTER, 601);
    // The hunter, three yards west of the wolf.
    put(&realm, g, START.0, START.1 - 30.0, START.2).await;
    cast(&realm, g, TRAP_SPELL).await;
    cast(&realm, g, TRAP_SPELL).await;
    let traps = objects_of(&realm, TRAP).await;
    assert_eq!(
        traps.len(),
        1,
        "one trap a slot: the second replaces the first"
    );
    let trap = traps[0];
    // Not armed yet.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!frozen(&realm, wolf).await, "it arms after a second");
    wait_until("the wolf frozen", async || frozen(&realm, wolf).await).await;
    assert!(
        objects_of(&realm, TRAP).await.is_empty(),
        "its one charge spent: {trap}"
    );
    // An ally walking over one isn't sprung on; a trap stays for an enemy.
    cast(&realm, g, TRAP_SPELL).await;
    let trap = objects_of(&realm, TRAP).await[0];
    on_map(&realm, move |m| m.remove_creature(wolf)).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let at = on_map(&realm, move |m| m.gameobject(trap).unwrap().position).await;
    put(&realm, gb, at.x, at.y, at.z).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!frozen(&realm, gb).await, "a friend isn't caught");
    assert_eq!(objects_of(&realm, TRAP).await, vec![trap]);
}

#[tokio::test]
async fn a_stealthed_trap_hides_from_its_owners_enemies() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut o = Client::connect(&realm, "O");
    let go = o.enter("Mallory", Race::Human, Class::Warrior).await;
    // Alice and Mallory, free for all, are each other's enemies.
    on_map(&realm, move |m| {
        for x in [g, go] {
            if let Some(p) = m.players.get_mut(&x) {
                p.fields.set_flag(
                    PLAYER_FLAGS,
                    crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
                    true,
                );
            }
        }
    })
    .await;
    cast(&realm, g, TRAP_SPELL).await;
    let trap = objects_of(&realm, TRAP).await[0];
    a.sees(trap).await;
    b.sees(trap).await;
    assert!(
        !o.sees_within(trap, Duration::from_millis(300)).await,
        "Mallory can't see it"
    );
    let seen = on_map(&realm, move |m| {
        (
            seen_by(m, gb, trap),
            seen_by(m, go, trap),
            seen_by(m, g, trap),
        )
    })
    .await;
    assert_eq!(seen, (true, false, true));
}

#[tokio::test]
async fn a_summoning_ritual_takes_its_group_then_summons_its_target() {
    let realm = harness::realm_with(data());
    let mut cs = grouped(&realm, &["Alice", "Bob", "Cid"]).await;
    let mut eve = Client::connect(&realm, "E");
    eve.enter("Eve", Race::Human, Class::Warrior).await;
    let mut dave = Client::connect(&realm, "D");
    let gd = dave.enter("Dave", Race::Human, Class::Warrior).await;
    let (ga, gb, gc) = (cs[0].guid, cs[1].guid, cs[2].guid);
    put(&realm, gd, START.0 + 40.0, START.1 + 40.0, START.2).await;
    // Alice, Dave her target, raises the portal and channels at it.
    on_map(&realm, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_guid(UNIT_FIELD_TARGET, gd);
    })
    .await;
    cast(&realm, ga, RITUAL_SPELL).await;
    let portal = objects_of(&realm, SUMMONING_PORTAL).await;
    assert_eq!(portal.len(), 1);
    let portal = portal[0];
    let channel = on_map(&realm, move |m| {
        m.unit_fields(ga)
            .unwrap()
            .get_guid(UNIT_FIELD_CHANNEL_OBJECT)
    })
    .await;
    assert_eq!(channel, portal, "her channel is on the portal");
    // Eve isn't of the group: she can't help.
    eve.sees(portal).await;
    eve.send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    // Bob helps, channelling.
    cs[1].sees(portal).await;
    cs[1].send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    cs[1]
        .wait_for("Bob's channel", |m| match m {
            ServerOpcodeMessage::MSG_CHANNEL_START(c) if c.spell == RITUAL_SPELL => Some(()),
            _ => None,
        })
        .await;
    let users = on_map(&realm, move |m| m.objects.rituals[&portal].users.clone()).await;
    assert_eq!(users, vec![ga, gb], "Eve wasn't let in");
    // Cid makes three: Dave is summoned, and the portal goes.
    cs[2].sees(portal).await;
    cs[2].send(CMSG_GAMEOBJ_USE { guid: portal }).await;
    let summoner = dave
        .wait_for("SMSG_SUMMON_REQUEST", |m| match m {
            ServerOpcodeMessage::SMSG_SUMMON_REQUEST(r) => Some(r.summoner),
            _ => None,
        })
        .await;
    assert_eq!(summoner, ga);
    wait_until("the portal gone", async || {
        objects_of(&realm, SUMMONING_PORTAL).await.is_empty()
    })
    .await;
    let channels = on_map(&realm, move |m| {
        [ga, gb, gc].map(|g| m.caster(g).is_some_and(|c| c.casting.is_some()))
    })
    .await;
    assert_eq!(channels, [false; 3], "the channels end with it");
    dave.send(CMSG_SUMMON_RESPONSE { summoner: ga }).await;
    let alice = position(&realm, ga).await;
    wait_until("Dave by Alice", async || {
        crate::game::map::combat::dist(position(&realm, gd).await, alice) < 5.0
    })
    .await;
    // A channel broken off takes the portal with it.
    cast(&realm, ga, RITUAL_SPELL).await;
    assert_eq!(objects_of(&realm, SUMMONING_PORTAL).await.len(), 1);
    on_map(&realm, move |m| crate::game::map::spells::interrupt(m, ga)).await;
    assert!(objects_of(&realm, SUMMONING_PORTAL).await.is_empty());
}

fn take_quest(m: &mut Map, g: Guid, quest: u32) {
    let d = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.journal.log.push(crate::game::store::Logged {
        quest,
        ..Default::default()
    });
    crate::game::map::quests::refresh_log(p, &d);
}

fn credited(m: &Map, g: Guid, quest: u32) -> u32 {
    m.players[&g]
        .journal
        .log
        .iter()
        .find(|l| l.quest == quest)
        .map_or(0, |l| l.kills[0])
}

async fn spawned(realm: &Arc<Realm>, entry: u32) -> Guid {
    objects_of(realm, entry).await[0]
}

async fn state_of(realm: &Arc<Realm>, o: Guid) -> (u32, u32) {
    on_map(realm, move |m| {
        let x = m.gameobject(o).unwrap();
        (
            x.fields.get_u32(GAMEOBJECT_STATE),
            x.fields.get_u32(GAMEOBJECT_FLAGS),
        )
    })
    .await
}

#[tokio::test]
async fn a_quest_object_goes_off_for_its_quest_and_credits_the_group() {
    let realm = harness::realm_with(data());
    let mut cs = grouped(&realm, &["Alice", "Bob"]).await;
    let mut cid = Client::connect(&realm, "C");
    let gc = cid.enter("Cid", Race::Human, Class::Warrior).await;
    let (ga, gb) = (cs[0].guid, cs[1].guid);
    let altar = spawned(&realm, GOOBER).await;
    on_map(&realm, move |m| {
        take_quest(m, ga, GOOBER_QUEST);
        take_quest(m, gb, GOOBER_QUEST);
    })
    .await;
    // Cid isn't on the quest: its page, no more.
    cid.send(CMSG_GAMEOBJ_USE { guid: altar }).await;
    cid.wait_for("the altar's page", |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_PAGETEXT(p) if p.guid == altar => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(state_of(&realm, altar).await, (GO_STATE_READY, 0));
    // Alice is: it opens, shows its page, and credits her and Bob.
    cs[0].send(CMSG_GAMEOBJ_USE { guid: altar }).await;
    let credit = cs[0]
        .wait_for("SMSG_QUESTUPDATE_ADD_KILL", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) => Some((k.creature_id, k.guid)),
            _ => None,
        })
        .await;
    assert_eq!(credit, (GOOBER | 0x8000_0000, altar));
    assert_eq!(
        state_of(&realm, altar).await,
        (GO_STATE_ACTIVE, GO_FLAG_IN_USE)
    );
    let counts = on_map(&realm, move |m| {
        (
            credited(m, ga, GOOBER_QUEST),
            credited(m, gb, GOOBER_QUEST),
            m.players[&gc].journal.log.len(),
        )
    })
    .await;
    assert_eq!(counts, (1, 1, 0), "the group near shares the credit");
    // In use, it does nothing more; then it closes.
    cs[1].send(CMSG_GAMEOBJ_USE { guid: altar }).await;
    wait_until("the altar closes", async || {
        state_of(&realm, altar).await == (GO_STATE_READY, 0)
    })
    .await;
    // A book shows its page.
    let book = spawned(&realm, BOOK).await;
    cid.send(CMSG_GAMEOBJ_USE { guid: book }).await;
    cid.wait_for("the book's page", |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_PAGETEXT(p) if p.guid == book => Some(()),
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_door_waits_on_its_guard_and_a_spell_works_a_lever() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let gate = spawned(&realm, DOOR).await;
    let guard = guid::unit(GUARD, 600);
    a.send(CMSG_GAMEOBJ_USE { guid: gate }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        state_of(&realm, gate).await.0,
        GO_STATE_READY,
        "the guard stands"
    );
    on_map(&realm, move |m| crate::game::map::combat::kill(m, g, guard)).await;
    a.send(CMSG_GAMEOBJ_USE { guid: gate }).await;
    wait_until("the gate open", async || {
        state_of(&realm, gate).await == (GO_STATE_ACTIVE, GO_FLAG_IN_USE)
    })
    .await;
    wait_until("the gate shut again", async || {
        state_of(&realm, gate).await == (GO_STATE_READY, 0)
    })
    .await;
    // A spell works the lever: no more to be used, and the quest asking
    // for the spell on it is credited.
    let lever = spawned(&realm, BUTTON).await;
    on_map(&realm, move |m| {
        take_quest(m, g, BUTTON_QUEST);
        // Cast, not for free: a triggered spell credits no quest.
        m.players.get_mut(&g).unwrap().spells.push(WORK_BUTTON);
        crate::game::map::spells::cast(m, g, WORK_BUTTON, Targets::default(), None);
    })
    .await;
    let (flags, credit) = on_map(&realm, move |m| {
        (
            m.gameobject(lever)
                .unwrap()
                .fields
                .get_u32(GAMEOBJECT_FLAGS),
            credited(m, g, BUTTON_QUEST),
        )
    })
    .await;
    assert_eq!(
        (flags & GO_FLAG_NO_INTERACT, credit),
        (GO_FLAG_NO_INTERACT, 1)
    );
    a.send(CMSG_GAMEOBJ_USE { guid: lever }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        state_of(&realm, lever).await.0,
        GO_STATE_READY,
        "not to be used"
    );
}

/// The real objects: the portals, campfire, traps and rituals load as
/// their spells want them.
#[test]
fn the_real_objects_load() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let kind = |e: u32| d.gameobject_templates.get(&e).map(|t| t.kind);
    assert_eq!(kind(176_296), Some(GO_SPELLCASTER));
    assert_eq!(kind(29_784), Some(GO_SPELL_FOCUS));
    assert_eq!(kind(2561), Some(GO_TRAP));
    assert_eq!(kind(36_727), Some(GO_RITUAL));
    assert_eq!(kind(177_193), Some(GO_RITUAL));
    let s = |id: u32| d.spells.get(id).unwrap();
    assert_eq!(s(698).effects[0].effect, EFFECT_TRANS_DOOR);
    assert_eq!(s(1499).effects[0].effect, EFFECT_SUMMON_OBJECT_SLOT1);
    assert_eq!(s(7720).effects[0].effect, EFFECT_SUMMON_PLAYER);
    assert_eq!(linked_entry(&d.gameobject_templates[&29_784]), 31_442);
    assert!(!d.object_requirements.is_empty());
}
