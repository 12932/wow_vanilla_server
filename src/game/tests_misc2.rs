//! The second round of player systems through the protocol: the quest
//! query's layout and its reputation, equipping by standing, the GM and
//! developer commands, the client's caches and tutorials, and the intro
//! cinematic's camera.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::*;
use crate::game::data::npc::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate, TalentInfo};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const STORMWIND: u32 = 72;
const WOLF: u32 = 3000;
const QUEST: u32 = 700;
const TABARD: u32 = 9100;
const SWORD: u32 = 9101;
const SPELL: u32 = 21000;
const TALENT: u32 = 21001;

fn creature(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction,
        unit_class: 1,
        health_multiplier: 1.0,
        health_max: 500,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.misc.factions.insert(
        STORMWIND,
        FactionDef {
            id: STORMWIND,
            index: 0,
            name: "Stormwind".into(),
            variants: [(1, 0, 0, 0x1), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(0, STORMWIND);
    d.creature_templates
        .insert(WOLF, creature(WOLF, "Wolf", 32));
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 20,
            entries: vec![WOLF],
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
    );
    d.npc.quests.insert(
        QUEST,
        Quest {
            id: QUEST,
            level: 3,
            min_level: 1,
            title: "Friend of Stormwind".into(),
            details: "Be liked.".into(),
            objectives: "Reach friendly.".into(),
            required_min_rep: (STORMWIND, 100),
            rep_objective: (STORMWIND, 3000),
            ..Default::default()
        },
    );
    let mut tabard = ItemTemplate {
        entry: TABARD,
        name: "Stormwind Tabard".into(),
        inventory_type: 19,
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        required_reputation_faction: STORMWIND,
        required_reputation_rank: RANK_FRIENDLY,
        ..Default::default()
    };
    tabard.class = 4;
    d.items.insert(TABARD, tabard);
    d.items.insert(
        SWORD,
        ItemTemplate {
            entry: SWORD,
            name: "Sword".into(),
            class: 2,
            inventory_type: 13,
            allowable_class: -1,
            allowable_race: -1,
            stackable: 1,
            quality: 2,
            delay: 2000,
            ..Default::default()
        },
    );
    d.spells.spells.insert(
        SPELL,
        SpellTemplate {
            id: SPELL,
            name: "Test Spell".into(),
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    let mut talent = SpellTemplate {
        id: TALENT,
        name: "Test Talent".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    talent.attributes[0] = 0x40;
    talent.effects[0] = SpellEffect {
        effect: 6,
        aura: 29,
        base_points: 0,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(TALENT, talent);
    d.spells.talents.insert(
        1,
        TalentInfo {
            id: 1,
            tab: 161,
            ranks: [TALENT, 0, 0, 0, 0],
            ..Default::default()
        },
    );
    // A warrior trainer teaching the test spell from level 1.
    let mut trainer = creature(3001, "Trainer", 12);
    trainer.trainer_class = 1;
    d.creature_templates.insert(3001, trainer);
    d.npc.trainers.insert(
        3001,
        vec![crate::game::data::npc::TrainerSpell {
            spell: SPELL,
            required_level: 1,
            ..Default::default()
        }],
    );
    // A wolf far off, where the intro's camera will be.
    d.creature_spawns.get_mut(&0).unwrap().push(CreatureSpawn {
        guid: 21,
        entries: vec![WOLF],
        map: 0,
        x: START.0 + 400.0,
        y: START.1 + 400.0,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    });
    d.tele = vec![
        Tele {
            name: "Goldshire".into(),
            map: 0,
            x: -9464.0,
            y: 62.0,
            z: 56.0,
            ..Default::default()
        },
        Tele {
            name: "Goldshire Inn".into(),
            map: 0,
            x: -9460.0,
            y: 30.0,
            z: 63.0,
            ..Default::default()
        },
    ];
    d
}

async fn setup() -> (Arc<Realm>, Client) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    (realm, a)
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

async fn gm(a: &mut Client, line: &str) {
    a.say(line).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
}

#[tokio::test]
async fn the_quest_query_is_laid_out_as_1_12_has_it() {
    let (_realm, mut a) = setup().await;
    a.send(CMSG_QUEST_QUERY { quest_id: QUEST }).await;
    let b = a
        .wait_frame("SMSG_QUEST_QUERY_RESPONSE", 0x5D, |_| true)
        .await;
    assert_eq!(u32_at(&b, 0), QUEST);
    // The reputation objective's faction and standing are full words.
    assert_eq!(u32_at(&b, 20), STORMWIND);
    assert_eq!(u32_at(&b, 24), 3000);
    // Fifteen words, ten reward pairs and the point, then the title.
    let title = &b[156..156 + "Friend of Stormwind".len()];
    assert_eq!(title, b"Friend of Stormwind");
}

#[tokio::test]
async fn a_quest_waits_on_a_standing_and_completes_with_one() {
    let (realm, a) = setup().await;
    let g = a.guid;
    let q = realm.data.npc.quests[&QUEST].clone();
    let (before, after) = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let before = crate::game::map::quests::can_take(&m.players[&g], &q, &d);
        crate::game::map::reputation::modify(m, g, STORMWIND, 200, false);
        (
            before,
            crate::game::map::quests::can_take(&m.players[&g], &q, &d),
        )
    })
    .await;
    assert!(!before && after);
    let done = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.journal.log.push(crate::game::store::Logged {
            quest: QUEST,
            ..Default::default()
        });
        crate::game::map::quests::check(m, g, QUEST);
        let q = m.realm.data.npc.quests[&QUEST].clone();
        let early =
            crate::game::map::quests::complete(&m.players[&g], &q, &m.players[&g].journal.log[0]);
        crate::game::map::reputation::modify(m, g, STORMWIND, 3000, false);
        let p = &m.players[&g];
        (
            early,
            crate::game::map::quests::complete(p, &q, &p.journal.log[0]),
        )
    })
    .await;
    assert_eq!(done, (false, true));
}

#[tokio::test]
async fn a_tabard_wants_its_standing() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, TABARD, 1, false).unwrap()
    })
    .await;
    let at = on_map(&realm, move |m| {
        let i = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == TABARD)
            .unwrap();
        (i.bag, i.slot)
    })
    .await;
    a.clear();
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: at.0,
        source_slot: at.1,
    })
    .await;
    let r = a
        .wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => Some(matches!(
                f.result,
                SMSG_INVENTORY_CHANGE_FAILURE_InventoryResult::CantEquipReputation { .. }
            )),
            _ => None,
        })
        .await;
    assert!(r);
    on_map(&realm, move |m| {
        crate::game::map::reputation::modify(m, g, STORMWIND, 3000, false)
    })
    .await;
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: at.0,
        source_slot: at.1,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let worn = on_map(&realm, move |m| {
        m.players[&g]
            .items
            .iter()
            .any(|i| i.entry == TABARD && i.slot == 18 && i.bag == crate::game::store::BAG_PLAYER)
    })
    .await;
    assert!(worn);
}

#[tokio::test]
async fn gm_commands_give_items_quests_spells_levels_and_money() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    gm(&mut a, ".rested 300").await;
    gm(&mut a, &format!(".additem {SWORD} 2")).await;
    gm(&mut a, &format!(".addquest {QUEST}")).await;
    gm(&mut a, &format!(".learn {SPELL}")).await;
    gm(&mut a, ".character level 12").await;
    gm(&mut a, ".modify money 5000").await;
    let state = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.count_of(SWORD),
            p.journal.has(QUEST),
            p.spells.contains(&SPELL),
            p.level(),
            p.money(),
            p.row.state.rest_bonus as u32,
        )
    })
    .await;
    assert_eq!(state.0, 2);
    assert!(state.1 && state.2);
    assert_eq!(state.3, 12);
    assert_eq!(state.4, 5000);
    assert!(state.5 > 0);
    gm(&mut a, &format!(".unlearn {SPELL}")).await;
    gm(&mut a, ".modify money -2000").await;
    let (knows, money) = on_map(&realm, move |m| {
        (m.players[&g].spells.contains(&SPELL), m.players[&g].money())
    })
    .await;
    assert_eq!((knows, money), (false, 3000));
    gm(&mut a, ".talents reset").await;
    gm(&mut a, ".pos").await;
    a.system_message("zone").await;
}

/// `.talents learn`: each talent to its ranks, a point a rank, as the
/// client learns them; without the points, or past a talent's last rank,
/// refused and named; then reset.
#[tokio::test]
async fn gm_talents_learn_spends_points_as_the_client_does() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    // Level 1: no points.
    gm(&mut a, ".talents learn 1").await;
    a.system_message("refused 1:1").await;
    assert!(!on_map(&realm, move |m| m.players[&g].spells.contains(&TALENT)).await);
    gm(&mut a, ".character level 12").await;
    gm(&mut a, ".talents learn 1:1").await;
    a.system_message("learned 1 ranks, 2 points left").await;
    assert!(on_map(&realm, move |m| m.players[&g].spells.contains(&TALENT)).await);
    // Its one rank had, a second is refused; a word that isn't one is
    // its usage.
    gm(&mut a, ".talents learn 1:2").await;
    a.system_message("learned 0 ranks, 2 points left; refused 1:2")
        .await;
    gm(&mut a, ".talents learn arms").await;
    a.system_message("Usage: .talents").await;
    gm(&mut a, ".talents reset").await;
    a.system_message("Unlearned").await;
    assert!(!on_map(&realm, move |m| m.players[&g].spells.contains(&TALENT)).await);
}

#[tokio::test]
async fn gm_commands_act_on_the_selected_creature() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    let wolf = guid::unit(WOLF, 20);
    a.sees(wolf).await;
    a.send(CMSG_SET_SELECTION { target: wolf }).await;
    gm(&mut a, ".damage 100").await;
    let (health, threat) = on_map(&realm, move |m| {
        let c = m.creature(wolf).unwrap();
        (
            c.fields.get_u32(UNIT_FIELD_HEALTH),
            c.threat.iter().any(|(t, _)| *t == g),
        )
    })
    .await;
    assert!(health < 500 && threat, "{health}");
    gm(&mut a, ".threat").await;
    a.system_message("Alice").await;
    gm(&mut a, ".guid").await;
    a.system_message("Target").await;
    // Moved to the GM, then killed and respawned.
    a.heartbeat(START.0 + 10.0, START.1, START.2).await;
    gm(&mut a, ".move").await;
    let at = on_map(&realm, move |m| m.creature(wolf).unwrap().position.x).await;
    assert!((at - (START.0 + 10.0)).abs() < 0.1, "{at}");
    gm(&mut a, ".kill").await;
    assert!(!on_map(&realm, move |m| m.creature(wolf).unwrap().alive()).await);
    gm(&mut a, ".respawn").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let back = on_map(&realm, move |m| m.creature(wolf).is_some_and(|c| c.alive())).await;
    assert!(back);
}

/// `.god`: blows land and cost health, which the player and whoever sees
/// them are told of, but never below 1, and the player doesn't die; off,
/// they do.
#[tokio::test]
async fn undying_takes_its_blows_but_stays_at_one_health() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(g).await;
    a.say(".god on").await;
    a.system_message("Undying on").await;
    let wolf = guid::unit(WOLF, 20);
    let (before, after, alive) = on_map(&realm, move |m| {
        let before = m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH);
        crate::game::map::combat::deal_damage(m, wolf, g, 1_000_000);
        let p = &m.players[&g];
        (
            before,
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.death == crate::game::map::death::Death::Alive,
        )
    })
    .await;
    assert!(before > 1);
    assert_eq!((after, alive), (1, true));
    // The health it cost is sent, to the player and to whoever sees them.
    for c in [&mut a, &mut b] {
        c.wait_for("health 1", |m| {
            harness::values(m)
                .into_iter()
                .find_map(|(of, mask)| match mask {
                    UpdateMask::Player(p) if of == g => p.unit_health().filter(|h| *h == 1),
                    _ => None,
                })
        })
        .await;
    }
    // At 1, another blow and a kill leave it there.
    let (health, alive) = on_map(&realm, move |m| {
        crate::game::map::combat::deal_damage(m, wolf, g, 50);
        crate::game::map::combat::kill(m, wolf, g);
        let p = &m.players[&g];
        (
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.death == crate::game::map::death::Death::Alive,
        )
    })
    .await;
    assert_eq!((health, alive), (1, true));
    a.say(".god off").await;
    a.system_message("Undying off").await;
    let (health, alive) = on_map(&realm, move |m| {
        crate::game::map::combat::deal_damage(m, wolf, g, 50);
        let p = &m.players[&g];
        (
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.death == crate::game::map::death::Death::Alive,
        )
    })
    .await;
    assert_eq!((health, alive), (0, false));
}

#[tokio::test]
async fn god_mode_takes_no_damage() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    gm(&mut a, ".tgm on").await;
    let (before, after) = on_map(&realm, move |m| {
        let before = m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH);
        crate::game::map::combat::deal_damage(m, guid::unit(WOLF, 20), g, 50);
        (before, m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH))
    })
    .await;
    assert_eq!(before, after);
    gm(&mut a, ".tgm off").await;
    let hurt = on_map(&realm, move |m| {
        crate::game::map::combat::deal_damage(m, guid::unit(WOLF, 20), g, 5);
        m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH)
    })
    .await;
    assert!(hurt < before);
}

#[tokio::test]
async fn gm_commands_for_weather_mail_battlegrounds_and_instances() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Mage).await;
    a.send(CMSG_ZONEUPDATE {
        area: Area::ElwynnForest,
    })
    .await;
    gm(&mut a, ".weather rain 0.8 permanent").await;
    // The zone's fair weather on entering it, then the rain.
    let grade = a
        .wait_for("rain", |m| match m {
            ServerOpcodeMessage::SMSG_WEATHER(w) if w.weather_type == WeatherType::Rain => {
                Some(w.grade)
            }
            _ => None,
        })
        .await;
    assert!((grade - 0.8).abs() < 0.01);
    gm(&mut a, ".mail Bob hello there").await;
    b.wait_for("SMSG_RECEIVED_MAIL", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_RECEIVED_MAIL(_)).then_some(())
    })
    .await;
    let body = realm
        .social
        .mail
        .lock()
        .mails
        .values()
        .find(|m| m.receiver == guid::counter(gb))
        .map(|m| m.body.clone());
    assert_eq!(body.as_deref(), Some("hello there"));
    a.clear();
    gm(&mut a, ".bg join warsong").await;
    let status = a
        .wait_frame("SMSG_BATTLEFIELD_STATUS", 0x2D4, |_| true)
        .await;
    // Invited (2) to Warsong Gulch (489).
    assert_eq!(u32_at(&status, 4), 489);
    assert_eq!(u32_at(&status, 13), 2);
    gm(&mut a, ".bg info").await;
    a.system_message("match").await;
    gm(&mut a, ".instance info").await;
    a.system_message("raid binds").await;
    gm(&mut a, ".invasion").await;
    a.system_message("The invasion is off").await;
}

#[tokio::test]
async fn debug_commands_reset_cooldowns_and_unlock_the_world() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.caster
            .cooldowns
            .insert(SPELL, std::time::Instant::now() + Duration::from_secs(60));
    })
    .await;
    gm(&mut a, ".debug cooldowns").await;
    let id = a
        .wait_for("SMSG_CLEAR_COOLDOWN", |m| match m {
            ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(c) => Some(c.id),
            _ => None,
        })
        .await;
    assert_eq!(id, SPELL);
    gm(&mut a, ".debug explore").await;
    gm(&mut a, ".debug taxi").await;
    gm(&mut a, ".debug spells").await;
    gm(&mut a, &format!(".debug reputation set {STORMWIND} 9000")).await;
    on_map(&realm, move |m| {
        crate::game::map::skills::learn_skill(m.players.get_mut(&g).unwrap(), 182, 1)
    })
    .await;
    gm(&mut a, ".debug professions").await;
    gm(&mut a, ".debug skill 182 120").await;
    let state = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.fields.get_u32(PLAYER_EXPLORED_ZONES_1),
            p.row.state.taxi_mask.first().copied(),
            p.spells.contains(&SPELL),
            crate::game::map::reputation::standing(p, &m.realm.data, STORMWIND),
            crate::game::map::skills::value(p, 182),
        )
    })
    .await;
    assert_eq!(state.0, u32::MAX);
    // `.debug taxi` teaches the world's nodes, and this one has none
    // (tests_p8_player_taxi has them taught).
    assert_eq!(state.1, None);
    assert!(state.2);
    assert_eq!(state.3, 9000);
    assert_eq!(state.4, Some(120));
    // The starting weapon off, so the main hand is free.
    on_map(&realm, move |m| {
        if let Some(i) = m.players[&g].item_at(crate::game::store::BAG_PLAYER, 15) {
            let n = m.players[&g].items[i].count();
            crate::game::map::inventory::take_item(m, g, i, n);
        }
    })
    .await;
    gm(&mut a, ".debug random equipment").await;
    let armed = on_map(&realm, move |m| {
        m.players[&g]
            .items
            .iter()
            .any(|i| i.entry == SWORD && i.bag == crate::game::store::BAG_PLAYER && i.slot == 15)
    })
    .await;
    assert!(armed);
    gm(&mut a, ".debug events").await;
    a.system_message("Events on").await;
}

#[tokio::test]
async fn a_moderator_may_look_but_not_change_the_world() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().gm_level = crate::accounts::MODERATOR;
    })
    .await;
    a.say(&format!(".additem {SWORD}")).await;
    a.system_message("permission").await;
    a.say(".pos").await;
    a.system_message("zone").await;
    let n = on_map(&realm, move |m| m.players[&g].count_of(SWORD)).await;
    assert_eq!(n, 0);
    // A player's dot line is only said.
    on_map(&realm, move |m| m.players.get_mut(&g).unwrap().gm_level = 0).await;
    a.say(&format!(".additem {SWORD}")).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let n = on_map(&realm, move |m| m.players[&g].count_of(SWORD)).await;
    assert_eq!(n, 0);
}

#[tokio::test]
async fn go_takes_a_facing_and_tele_lists_what_fits() {
    let (realm, mut a) = setup().await;
    let g = a.guid;
    gm(&mut a, ".tele gold").await;
    let listed = a.system_message("Places").await;
    assert!(listed.contains("Goldshire Inn"));
    gm(
        &mut a,
        &format!(".go xyz {} {} {} here 1.5", START.0 + 5.0, START.1, START.2),
    )
    .await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    let o = on_map(&realm, move |m| m.players[&g].movement.orientation).await;
    assert!((o - 1.5).abs() < 0.01, "{o}");
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(data).unwrap();
    z.finish().unwrap()
}

#[tokio::test]
async fn the_clients_caches_and_tutorials_are_kept_for_the_account() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    // Macros (an account's cache, 4) uploaded, then asked for back.
    let macros = b"MACRO 1 \"Charge\" Ability_Warrior_Charge".to_vec();
    let mut body = Vec::new();
    body.extend_from_slice(&4u32.to_le_bytes());
    body.extend_from_slice(&(macros.len() as u32).to_le_bytes());
    body.extend_from_slice(&zlib(&macros));
    a.send_raw(0x20B, &body).await;
    a.send(CMSG_TUTORIAL_FLAG { tutorial_flag: 33 }).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    a.send(CMSG_REQUEST_ACCOUNT_DATA { data_type: 4 }).await;
    let frame = a
        .wait_frame("SMSG_UPDATE_ACCOUNT_DATA", 0x20C, |_| true)
        .await;
    assert_eq!(
        crate::game::map::account_data::parse_update(&frame),
        Some((4, macros.clone()))
    );
    // Another character of the account: the macros' digest, and the
    // tutorial seen.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let _ = ga;
    a.clear();
    let gb = a.enter("Anna", Race::Human, Class::Mage).await;
    let times = a
        .wait_frame("SMSG_ACCOUNT_DATA_TIMES", 0x209, |_| true)
        .await;
    assert_eq!(
        &times[4 * 16..5 * 16],
        &crate::game::map::account_data::digest(&macros)
    );
    assert_eq!(&times[0..16], &[0u8; 16]);
    let tutorials = on_map(&realm, move |m| m.players[&gb].row.state.tutorials[1]).await;
    assert_eq!(tutorials, 0x2);
}

#[tokio::test]
async fn the_intro_cinematic_shows_what_its_camera_passes() {
    let mut d = data();
    // The human intro's camera, far from the start.
    d.races.insert(
        1,
        RaceInfo {
            cinematic: 81,
            ..Default::default()
        },
    );
    d.misc.cinematic_waypoints.insert(
        81,
        vec![
            (0, START.0 + 400.0, START.1 + 400.0, START.2),
            (30_000, START.0 + 400.0, START.1 + 400.0, START.2),
        ],
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let far = guid::unit(WOLF, 21);
    // 565 yards off, but the camera is there.
    a.sees(far).await;
    let view = on_map(&realm, move |m| m.players[&g].misc.view).await;
    assert!(view.is_some());
    a.send(CMSG_COMPLETE_CINEMATIC {}).await;
    a.forgets(far).await;
    let view = on_map(&realm, move |m| m.players[&g].misc.view).await;
    assert!(view.is_none());
    let _: Guid = g;
}

/// A character that starts nowhere near its intro's path (`[spawn]` puts
/// new characters elsewhere): the camera isn't followed, so its view stays
/// with it and what stands by it is seen from the first, not forgotten
/// for what the camera would fly past and sent again as the client ends
/// the intro.
#[tokio::test]
async fn an_intro_far_from_the_character_isnt_followed() {
    let mut d = data();
    d.races.insert(
        1,
        RaceInfo {
            cinematic: 81,
            ..Default::default()
        },
    );
    d.misc.cinematic_waypoints.insert(
        81,
        vec![
            (0, START.0 + 3000.0, START.1, START.2),
            (30_000, START.0 + 3000.0, START.1 + 100.0, START.2),
        ],
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(guid::unit(WOLF, 20)).await;
    let (camera, view) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.misc.camera.is_some(), p.misc.view)
    })
    .await;
    assert!(!camera && view.is_none(), "no camera to follow");
    a.send(CMSG_COMPLETE_CINEMATIC {}).await;
    harness::an_update(&realm.registry.online(g).expect("online").map()).await;
    let wolf = guid::unit(WOLF, 20);
    let kept = on_map(&realm, move |m| m.players[&g].known.contains(&wolf)).await;
    assert!(kept, "the wolf by her still known");
}
