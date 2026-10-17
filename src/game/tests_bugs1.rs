//! Bugs the mapping against Thistle Tea's tests turned up, through the
//! protocol and on the real data when it is here: the dead killed again,
//! containers looted twice, mana in druid forms, quests from items, the
//! rituals' cooldowns, pets' positive autocasts, and the smaller checks
//! after them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapKey};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// A creature of `entry` at `level`, `yards` in front of `g`, with
/// `health` and no armor.
fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32, health: u32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the creature");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &template,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
    c.fields.set_u32(UNIT_FIELD_HEALTH, health);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    let c = c.with_base();
    m.add_creature(c)
}

fn ready(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
    }
    let p = m.players.get_mut(&g).unwrap();
    for i in 0..5 {
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
        p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
    }
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn xp(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(PLAYER_XP)
}

#[tokio::test]
async fn the_dead_are_not_killed_again() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "BUGMAGE1");
    let mage = c.enter("Bugmagea", Race::Human, Class::Mage).await;
    let mut d = Client::connect(&realm, "BUGROGUE1");
    let rogue = d.enter("Bugroguea", Race::Undead, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 10, &[]);
            // A blow on a corpse: no second reward.
            let kobold = spawn(m, mage, 6, 10, 5.0, 100);
            crate::game::map::combat::deal_damage(m, mage, kobold, 100);
            let first = xp(m, mage);
            crate::game::map::combat::deal_damage(m, mage, kobold, 100);
            crate::game::map::combat::deal_damage_school(m, mage, kobold, 100, 4);
            crate::game::map::combat::kill(m, mage, kobold);
            let blows = xp(m, mage);
            // A Frostbolt in flight when its target dies lands on nothing.
            m.spells.travel = Some(true);
            let other = spawn(m, mage, 6, 10, 20.0, 100);
            spells::cast_triggered(m, mage, 116, Some(other));
            let flying = m.spells.in_flight.len();
            crate::game::map::combat::deal_damage(m, mage, other, 100);
            let second = xp(m, mage);
            m.now += Duration::from_secs(3);
            spells::update(m);
            let landed = xp(m, mage);
            m.spells.travel = None;
            // A player killed, then hit again: one honorable kill.
            ready(m, rogue, 60, &[]);
            ready(m, mage, 60, &[]);
            let h = health(m, rogue);
            crate::game::map::combat::deal_damage(m, mage, rogue, h);
            crate::game::map::combat::deal_damage(m, mage, rogue, h);
            crate::game::map::combat::kill(m, mage, rogue);
            let kills = m
                .realm
                .social
                .honor
                .lock()
                .victim_kills(guid::counter(mage), u64::from(guid::counter(rogue)));
            (first, blows, flying, second, landed, kills)
        })
        .await
        .unwrap();
    let (first, blows, flying, second, landed, kills) = out;
    assert!(first > 0, "the kill's experience");
    assert_eq!(blows, first, "a corpse hit again is worth nothing");
    assert_eq!(flying, 1, "the Frostbolt flies");
    assert!(second > first, "the second kobold's experience");
    assert_eq!(landed, second, "a missile on a corpse kills nothing");
    assert_eq!(kills, 1, "one honorable kill of a corpse hit twice");
}

fn item(entry: u32, name: &str) -> crate::game::data::ItemTemplate {
    crate::game::data::ItemTemplate {
        entry,
        name: name.into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        ..Default::default()
    }
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

/// The items and gold a 1.12 loot window shows.
fn window(frame: &[u8]) -> (Guid, u32, Vec<u32>) {
    let u32_at = |at: usize| u32::from_le_bytes(frame[at..at + 4].try_into().unwrap());
    let items = (0..usize::from(frame[13]))
        .map(|n| u32_at(14 + n * 22 + 1))
        .collect();
    (
        Guid::new(u64::from_le_bytes(frame[0..8].try_into().unwrap())),
        u32_at(9),
        items,
    )
}

#[tokio::test]
async fn a_part_looted_container_keeps_what_is_left() {
    const CLAM: u32 = 9000;
    const MEAT: u32 = 9001;
    const PEARL: u32 = 9002;
    let mut d = crate::game::data::GameData::empty();
    let mut clam = item(CLAM, "Small Barnacled Clam");
    clam.flags = 0x4;
    clam.min_money_loot = 1;
    clam.max_money_loot = 1000;
    d.items.insert(CLAM, clam);
    d.items.insert(MEAT, item(MEAT, "Clam Meat"));
    d.items.insert(PEARL, item(PEARL, "Small Lustrous Pearl"));
    let entry = |item| crate::game::data::LootEntry {
        item,
        chance: 100.0,
        min: 1,
        max: 1,
        ..Default::default()
    };
    d.loot.item.insert(CLAM, vec![entry(MEAT), entry(PEARL)]);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "BUGCLAM");
    let g = a.enter("Bugclam", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let at = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, g, CLAM, 1, false).unwrap();
            let i = m.players[&g]
                .items
                .iter()
                .find(|i| i.entry == CLAM)
                .unwrap();
            (i.bag, i.slot)
        })
        .await
        .unwrap();
    let open = async |a: &mut Client| {
        a.clear();
        a.send(CMSG_OPEN_ITEM {
            bag_index: at.0,
            slot: at.1,
        })
        .await;
        window(&a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await)
    };
    let (clam, gold, first) = open(&mut a).await;
    assert_eq!(first, vec![MEAT, PEARL]);
    // The meat taken, the window closed: the pearl and the gold are left.
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: clam }).await;
    let (_, gold_again, again) = open(&mut a).await;
    assert_eq!((gold_again, again), (gold, vec![PEARL]), "reopened");
    // And after a logout too.
    a.send(CMSG_LOOT_RELEASE { guid: clam }).await;
    relog(&mut a).await;
    let (_, gold_later, later) = open(&mut a).await;
    assert_eq!((gold_later, later), (gold, vec![PEARL]), "after a logout");
    // Emptied, it is used up.
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_MONEY {}).await;
    a.send(CMSG_LOOT_RELEASE { guid: clam }).await;
    a.wait_for("SMSG_LOOT_RELEASE_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(_)).then_some(())
    })
    .await;
    let left = map
        .query(move |m| {
            let p = &m.players[&g];
            (p.count_of(CLAM), p.count_of(MEAT), p.count_of(PEARL))
        })
        .await
        .unwrap();
    assert_eq!(left, (0, 1, 1));
}

#[tokio::test]
async fn a_druid_regenerates_mana_in_bear_and_cat_forms() {
    let realm = harness::realm();
    let mut c = Client::connect(&realm, "BUGDRUID");
    let g = c.enter("Bugdruid", Race::NightElf, Class::Druid).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    let out = map
        .query(move |m| {
            let mut gained = Vec::new();
            // Rage in bear form, energy in cat form.
            for power in [1u8, 3] {
                let p = m.players.get_mut(&g).unwrap();
                p.fields.set_byte(UNIT_FIELD_BYTES_0, 3, power);
                p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
                p.fields.set_u32(UNIT_FIELD_POWER1, 100);
                p.fields.set_u32(UNIT_FIELD_POWER1 + 3, 0);
                p.mana_spent_at = None;
                p.regen_at = m.now;
                crate::game::map::stats::regen(m);
                let p = &m.players[&g];
                gained.push((
                    p.fields.get_u32(UNIT_FIELD_POWER1),
                    p.fields.get_u32(UNIT_FIELD_POWER1 + 3),
                ));
            }
            gained
        })
        .await
        .unwrap();
    assert!(out[0].0 > 100, "mana in bear form: {out:?}");
    assert!(out[1].0 > 100, "mana in cat form: {out:?}");
    assert!(out[1].1 > 0, "and energy in cat form: {out:?}");
}

#[tokio::test]
async fn only_an_item_the_player_has_that_starts_the_quest_starts_it() {
    use crate::game::data::npc::Quest;
    const LETTER: u32 = 9100;
    const RELIC: u32 = 9101;
    const OTHER: u32 = 9102;
    const FROM_LETTER: u32 = 800;
    const FROM_RELIC: u32 = 801;
    let mut d = crate::game::data::GameData::empty();
    let quest = |id| Quest {
        id,
        level: 1,
        min_level: 1,
        title: "From an item".into(),
        ..Default::default()
    };
    d.npc.quests.insert(FROM_LETTER, quest(FROM_LETTER));
    // The relic is asked for: it stays.
    let mut relic_quest = quest(FROM_RELIC);
    relic_quest.required_items[0] = (RELIC, 1);
    d.npc.quests.insert(FROM_RELIC, relic_quest);
    let mut letter = item(LETTER, "A Letter");
    letter.start_quest = FROM_LETTER;
    d.items.insert(LETTER, letter);
    let mut relic = item(RELIC, "A Relic");
    relic.start_quest = FROM_RELIC;
    d.items.insert(RELIC, relic);
    d.items.insert(OTHER, item(OTHER, "Not a Letter"));
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "BUGQUEST");
    let g = a.enter("Bugquest", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let held = map
        .query(move |m| {
            for e in [LETTER, RELIC, OTHER] {
                crate::game::map::inventory::add_item(m, g, e, 1, false).unwrap();
            }
            let p = &m.players[&g];
            [LETTER, RELIC, OTHER].map(|e| p.items.iter().find(|i| i.entry == e).unwrap().guid)
        })
        .await
        .unwrap();
    let [letter, relic, other] = held;
    let has = async |q| {
        map.query(move |m| m.players[&g].journal.has(q))
            .await
            .unwrap()
    };
    // An item GUID no one has, and an item that starts something else.
    for giver in [guid::item(0xFFF0), other] {
        a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
            guid: giver,
            quest_id: FROM_LETTER,
        })
        .await;
    }
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: letter,
        quest_id: FROM_RELIC,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!has(FROM_LETTER).await, "no item of the player's starts it");
    assert!(!has(FROM_RELIC).await, "the letter starts its own quest");
    // The letter starts its quest and goes; the relic is kept.
    for (giver, q) in [(letter, FROM_LETTER), (relic, FROM_RELIC)] {
        a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
            guid: giver,
            quest_id: q,
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has(FROM_LETTER).await && has(FROM_RELIC).await);
    let left = map
        .query(move |m| {
            let p = &m.players[&g];
            (p.count_of(LETTER), p.count_of(RELIC))
        })
        .await
        .unwrap();
    assert_eq!(left, (0, 1), "the letter is taken, the relic kept");
}

#[tokio::test]
async fn the_ritual_of_dooms_cooldown_waits_for_its_portal() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "BUGLOCK");
    let g = c.enter("Buglock", Race::Human, Class::Warlock).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const DOOM: u32 = 18540;
            ready(m, g, 60, &[DOOM]);
            crate::game::map::inventory::add_item(m, g, 16583, 3, false).unwrap();
            let portal = |m: &Map| {
                m.objects
                    .summoned
                    .iter()
                    .find(|s| s.owner == Some(g) && s.spell == DOOM)
                    .map(|s| s.guid)
            };
            let waiting = |m: &Map| {
                m.caster(g)
                    .is_some_and(|c| c.pending_cooldowns.contains(&DOOM))
            };
            let cooling = |m: &Map| {
                m.caster(g)
                    .is_some_and(|c| c.cooldowns.get(&DOOM).is_some_and(|t| *t > m.now))
            };
            // Cast, then the channel stopped short: the portal goes, and
            // there is no cooldown.
            let first = spells::cast_now(m, g, DOOM, Default::default());
            let raised = (portal(m).is_some(), waiting(m));
            spells::cancel(m, g);
            let stopped = (portal(m).is_some(), waiting(m), cooling(m));
            // Cast again; the ritual done, its portal goes and the hour
            // starts.
            m.now += Duration::from_secs(2);
            ready(m, g, 60, &[]);
            let second = spells::cast_now(m, g, DOOM, Default::default());
            let o = portal(m);
            if let Some(o) = o {
                m.objects.rituals.get_mut(&o).unwrap().completed = true;
                crate::game::map::objects::despawn(m, o);
            }
            let done = (waiting(m), cooling(m));
            (first, raised, stopped, second, o.is_some(), done)
        })
        .await
        .unwrap();
    let (first, raised, stopped, second, again, done) = out;
    assert!(first, "the ritual is cast");
    assert_eq!(raised, (true, true), "its portal up, its cooldown waiting");
    assert_eq!(stopped, (false, false, false), "stopped short, no cooldown");
    assert!(second && again, "so it can be cast again");
    assert_eq!(done, (false, true), "done, the hour starts");
}

#[tokio::test]
async fn a_pet_autocasts_its_positive_spells() {
    if !have_data() {
        return;
    }
    const FIRE_SHIELD: u32 = 2947;
    const PARANOIA: u32 = 19480;
    let realm = realm();
    let mut c = Client::connect(&realm, "BUGIMP");
    let g = c.enter("Bugimp", Race::Human, Class::Warlock).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            spells::cast_triggered(m, g, 688, None);
            let imp = crate::game::map::pets::pet_of(m, g).expect("the imp");
            let think = |m: &mut Map| {
                for _ in 0..3 {
                    m.now += Duration::from_secs(2);
                    if let Some(c) = m.caster_mut(imp) {
                        c.casting = None;
                        c.gcd_until = None;
                    }
                    if let Some(f) = m.unit_fields_mut(imp) {
                        f.set_u32(UNIT_FIELD_POWER1, 10_000);
                    }
                    crate::game::map::pets::update(m);
                }
            };
            let has = |m: &Map, who: Guid, spell: u32| {
                crate::game::map::auras::auras(m, who).is_some_and(|a| a.has_spell(spell))
            };
            // Out of a fight: its own buff, and no shield for one not
            // under attack.
            crate::game::map::pets::state_mut(m, imp).unwrap().autocast =
                vec![FIRE_SHIELD, PARANOIA];
            think(m);
            let calm = (has(m, imp, PARANOIA), has(m, g, FIRE_SHIELD));
            // Its master attacked: a shield on them.
            let kobold = spawn(m, g, 6, 60, 4.0, 100_000);
            crate::game::map::combat::start_attack(m, kobold, g);
            crate::game::map::combat::engage(m, kobold, g);
            think(m);
            let attacked = has(m, g, FIRE_SHIELD);
            (calm, attacked)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, false), "Paranoia out of a fight, no shield");
    assert!(out.1, "Fire Shield on its master under attack");
}

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

#[tokio::test]
async fn an_objects_gossip_options_show_and_are_chosen() {
    use crate::game::data::npc::{GossipMenu, GossipOption};
    const STATUE: u32 = 9200;
    const MENU: u32 = 7000;
    const DEEPER: u32 = 7001;
    let mut d = crate::game::data::GameData::empty();
    let mut statue = crate::game::data::GameObjectTemplate {
        entry: STATUE,
        kind: 2,
        display_id: 1,
        name: "Old Statue".into(),
        size: 1.0,
        ..Default::default()
    };
    statue.data[3] = MENU;
    d.gameobject_templates.insert(STATUE, statue);
    d.gameobject_spawns.insert(
        0,
        vec![crate::game::data::GameObjectSpawn {
            guid: 600,
            entry: STATUE,
            map: 0,
            x: START.0 + 1.0,
            y: START.1,
            z: START.2,
            state: 1,
            ..Default::default()
        }],
    );
    // A gossip option flagged for a gossip NPC, as the real menus' are.
    d.npc.gossip_menus.insert(
        MENU,
        GossipMenu {
            texts: vec![(100, 0)],
            options: vec![GossipOption {
                id: 0,
                text: "Read the inscription.".into(),
                kind: 1,
                npc_flag: 1,
                action_menu: DEEPER as i32,
                ..Default::default()
            }],
        },
    );
    d.npc.gossip_menus.insert(
        DEEPER,
        GossipMenu {
            texts: vec![(101, 0)],
            options: Vec::new(),
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "BUGGOSSIP");
    a.enter("Buggossip", Race::Human, Class::Warrior).await;
    let statue = guid::game_object(STATUE, 600);
    a.sees(statue).await;
    a.send(CMSG_GAMEOBJ_USE { guid: statue }).await;
    let shown = a
        .wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => {
                Some(g.gossips.iter().map(|o| o.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .await;
    assert_eq!(shown, vec![0], "the object's gossip option");
    a.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: statue,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    let next = a
        .wait_for("the menu it leads to", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => Some(g.title_text_id),
            _ => None,
        })
        .await;
    assert_eq!(next, 101);
}

#[tokio::test]
async fn disarm_trap_spends_the_trap_and_opens_no_loot() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "BUGROGUE2");
    let g = c.enter("Bugrogueb", Race::Human, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[1842]);
            // Thieves' Tools.
            crate::game::map::inventory::add_item(m, g, 5060, 1, false).unwrap();
            let p = &m.players[&g];
            let (at, o) = (p.position(), p.movement.orientation);
            let spot = Vector3d {
                x: at.x + 2.0 * o.cos(),
                y: at.y + 2.0 * o.sin(),
                z: at.z,
            };
            // A Freezing Trap nobody set.
            let trap = crate::game::map::objects::summon(m, None, 2561, spot, o, 0, 0, 0, None)
                .expect("the trap");
            let cast = spells::cast_now(
                m,
                g,
                1842,
                spells::Targets {
                    object: Some(trap),
                    ..Default::default()
                },
            );
            (cast, m.gameobject(trap).is_some(), m.players[&g].looting)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (true, false, None),
        "cast, the trap gone, no loot window"
    );
}

#[tokio::test]
async fn a_locked_objects_loot_is_for_those_who_opened_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut a = Client::connect(&realm, "BUGHERB1");
    let herbalist = a.enter("Bugherba", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "BUGHERB2");
    let other = b.enter("Bugherbb", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            let at = m.players[&herbalist].position();
            for g in [herbalist, other] {
                m.players.get_mut(&g).unwrap().movement.position = at;
            }
            let spot = Vector3d {
                x: at.x + 1.0,
                ..at
            };
            // Peacebloom, locked to herbalism.
            let node = crate::game::map::objects::summon(m, None, 1618, spot, 0.0, 0, 0, 0, None)
                .expect("the herb");
            let locked = crate::game::map::professions::object_lock(&m.realm.data, 1618) != 0;
            // Gathered by one; the other asks for its loot window.
            crate::game::map::loot::open_object(m, herbalist, node);
            let gathered = m.players[&herbalist].looting;
            crate::game::map::loot::open(m, other, node);
            (locked, gathered == Some(node), m.players[&other].looting)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, None), "only the herbalist looks in it");
}
