//! Round 9's player: quest objects, quest givers' rules and greetings
//! (Thistle Tea's `game_object_quests_test`, `quest_objects_test`,
//! `quest_required_condition_test`, `quest_rewards_test` and
//! `gossip_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::{GossipMenu, Quest};
use crate::game::data::scripts::{Condition, ScriptStep};
use crate::game::data::spells::SpellEffect;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness;
use crate::game::map::objects::{self, GO_GOOBER, GO_STATE_ACTIVE};
use crate::game::map::{Map, MapKey, auras};
use crate::game::store::Logged;
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with};
use crate::game::tests_p3_entity::{aura_effect, on, spell};
use crate::game::tests_p3_entity_objects::object;

const QUEST: u32 = 96_800;
const OTHER_QUEST: u32 = 96_801;
/// A quest object's, wanting a thing, so under way once taken.
const GOOBER_QUEST: u32 = 96_802;
const GOOBER: u32 = 98_800;
const TRAP: u32 = 98_801;
const SLOW_GOOBER: u32 = 98_802;
const BOX_GIVER: u32 = 98_803;
const BARE_GIVER: u32 = 98_804;
const STONE: u32 = 98_805;
const STONE_SPAWN: u32 = 880;
const GIVER_SPAWN: u32 = 881;
const BARE_SPAWN: u32 = 882;
/// A dummy aura its trap lands, and a second's spell of a quest object.
const SPRUNG: u32 = 97_800;
const SLOW_SPELL: u32 = 97_801;
const OPEN: u32 = 97_802;
const LESSON: u32 = 97_803;
const TAUGHT: u32 = 97_804;
const BANKED: u32 = 63_950;
const BANK_CONDITION: u32 = 9_801;
const EVENT_CONDITION: u32 = 9_802;
const EVENT: u32 = 4463;
const GREETER: u32 = 3_960;
const GREETER_SPAWN: u32 = 30_136;
const MENU: u32 = 9_960;
const SCRIPTED_MENU: u32 = 9_961;
const ONE_SECOND: u32 = 7;

fn world() -> GameData {
    let mut d = data();
    for t in [
        // On QUEST, its trap linked, closing after a second, its page 5.
        object(
            GOOBER,
            GO_GOOBER,
            "Goober",
            &[(0, 47), (1, GOOBER_QUEST), (3, 1 << 16), (7, 5), (12, TRAP)],
        ),
        object(TRAP, 6, "Trap", &[(2, 2), (3, SPRUNG)]),
        object(SLOW_GOOBER, GO_GOOBER, "Slow Goober", &[(10, SLOW_SPELL)]),
        object(BOX_GIVER, 2, "Box", &[]),
        object(BARE_GIVER, 2, "Bare", &[]),
        object(STONE, 5, "Stone", &[]),
    ] {
        d.gameobject_templates.insert(t.entry, t);
    }
    // A long box: its model ten yards along its x, a yard either side.
    d.gameobject_templates.get_mut(&BOX_GIVER).unwrap().bounds =
        Some(([-1.0, -1.0, -1.0], [10.0, 1.0, 1.0]));
    let spawn_at = |guid, entry, dx: f32, dy: f32| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
        state: 1,
        ..Default::default()
    };
    d.gameobject_spawns.entry(0).or_default().extend([
        spawn_at(STONE_SPAWN, STONE, 3.0, 0.0),
        spawn_at(GIVER_SPAWN, BOX_GIVER, 0.0, 20.0),
        spawn_at(BARE_SPAWN, BARE_GIVER, 0.0, -20.0),
    ]);
    for (id, title) in [(QUEST, "Goober"), (OTHER_QUEST, "Boxed")] {
        d.npc.quests.insert(
            id,
            Quest {
                id,
                title: title.into(),
                min_level: 1,
                level: 1,
                ..Default::default()
            },
        );
    }
    d.npc.quests.insert(
        GOOBER_QUEST,
        Quest {
            id: GOOBER_QUEST,
            title: "Under way".into(),
            min_level: 1,
            level: 1,
            required_items: [(BANKED, 1), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d.npc.object_starts.insert(BOX_GIVER, vec![OTHER_QUEST]);
    d.npc.object_starts.insert(BARE_GIVER, vec![OTHER_QUEST]);
    d.npc.object_ends.insert(BOX_GIVER, vec![QUEST]);
    d.spells.spells.insert(
        SPRUNG,
        spell(SPRUNG, &[aura_effect(auras::AURA_DUMMY, 1, 0, 1)]),
    );
    let mut slow = spell(SLOW_SPELL, &[aura_effect(auras::AURA_DUMMY, 1, 0, 1)]);
    slow.casting_time_index = ONE_SECOND;
    d.spells.spells.insert(SLOW_SPELL, slow);
    d.spells.cast_times.insert(ONE_SECOND, 1000);
    d.crafts.locks.insert(
        47,
        vec![crate::game::data::crafts::LockKey {
            kind: crate::game::data::crafts::LOCK_KEY_SKILL,
            index: 19,
            skill: 0,
        }],
    );
    d.spells.spells.insert(
        OPEN,
        spell(
            OPEN,
            &[SpellEffect {
                effect: crate::game::map::professions::EFFECT_OPEN_LOCK,
                misc_value: 19,
                base_points: -1,
                base_dice: 1,
                target_a: 25,
                ..Default::default()
            }],
        ),
    );
    // A reward spell that teaches, as a trainer's would.
    d.spells.spells.insert(
        LESSON,
        spell(
            LESSON,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_LEARN_SPELL,
                trigger_spell: TAUGHT,
                target_a: 1,
                ..Default::default()
            }],
        ),
    );
    d.spells.spells.insert(
        TAUGHT,
        spell(TAUGHT, &[aura_effect(auras::AURA_DUMMY, 1, 0, 1)]),
    );
    d.npc.quests.get_mut(&QUEST).unwrap().reward_spell_cast = LESSON;
    // The other quest wants a thing carried or banked.
    d.items.insert(
        BANKED,
        ItemTemplate {
            entry: BANKED,
            name: "Banked".into(),
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d.scripts.conditions.insert(
        BANK_CONDITION,
        Condition {
            entry: BANK_CONDITION,
            kind: 23,
            values: [BANKED as i32, 1, 0, 0],
            flags: 0,
        },
    );
    d.scripts
        .quest_condition
        .insert(OTHER_QUEST, BANK_CONDITION);
    d.scripts.conditions.insert(
        EVENT_CONDITION,
        Condition {
            entry: EVENT_CONDITION,
            kind: 36,
            values: [EVENT as i32, 0, 0, 0],
            flags: 0,
        },
    );
    // A greeter of no menu, its spawn's own text; and one with a menu of
    // no script, beside a menu with one.
    let mut greeter = d.creature_templates[&BRUTE].clone();
    greeter.entry = GREETER;
    greeter.npc_flags = 1;
    greeter.faction = 35;
    d.creature_templates.insert(GREETER, greeter);
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: GREETER_SPAWN,
        entries: vec![GREETER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1 + 2.0,
        z: START.2,
        ..Default::default()
    });
    d.npc.npc_gossip.insert(GREETER_SPAWN, 1693);
    for (menu, text) in [(MENU, 2), (SCRIPTED_MENU, 3)] {
        d.npc.gossip_menus.insert(
            menu,
            GossipMenu {
                texts: vec![(text, 0)],
                ..Default::default()
            },
        );
    }
    d.scripts.gossip_menu_script.insert(SCRIPTED_MENU, 9_962);
    d
}

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

fn place(m: &mut Map, g: Guid, to: Vector3d) {
    m.players.get_mut(&g).unwrap().movement.position = to;
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn take(m: &mut Map, g: Guid, quest: u32) {
    m.players.get_mut(&g).unwrap().journal.add(Logged {
        quest,
        ..Default::default()
    });
}

fn state(m: &Map, o: Guid) -> u32 {
    m.gameobject(o).unwrap().fields.get_u32(GAMEOBJECT_STATE)
}

/// Thistle Tea's "linked traps wait for quest admission after Opening":
/// a quest object opened by its lock, or used by one not on its quest,
/// doesn't spring its trap; used by one on it, it goes off and springs it.
#[tokio::test]
async fn a_quest_objects_trap_waits_for_one_on_its_quest() {
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let seen = on(&map, move |m| {
        let goober = objects::summon(m, None, GOOBER, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap();
        objects::summon(m, None, TRAP, at(1.5, 0.0), 0.0, 0, 0, 0, None).unwrap();
        crate::game::map::spells::cast(
            m,
            g,
            OPEN,
            crate::game::map::spells::Targets {
                object: Some(goober),
                ..Default::default()
            },
            None,
        );
        let opened = has(m, g, SPRUNG);
        crate::game::map::loot::close(m, g, goober);
        objects::use_goober(m, g, goober);
        let unadmitted = (has(m, g, SPRUNG), state(m, goober));
        take(m, g, GOOBER_QUEST);
        objects::use_goober(m, g, goober);
        (opened, unadmitted, has(m, g, SPRUNG), state(m, goober))
    })
    .await;
    assert!(!seen.0, "opening it springs nothing");
    assert!(!seen.1.0 && seen.1.1 != GO_STATE_ACTIVE, "not on the quest");
    assert_eq!((seen.2, seen.3), (true, GO_STATE_ACTIVE), "on it: sprung");
}

/// Thistle Tea's "spell activation routes through the player quest
/// checks": a script's use of a quest object for a player not on its
/// quest shows its page alone, and doesn't set it off.
#[tokio::test]
async fn a_scripts_use_of_a_quest_object_asks_the_players_quest() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let goober = on(&map, move |m| {
        objects::summon(m, None, GOOBER, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    a.clear();
    let after = on(&map, move |m| {
        objects::used_by_script(m, g, goober);
        (
            state(m, goober),
            m.objects
                .goobers
                .get(&goober)
                .and_then(|s| s.active_until)
                .is_some(),
        )
    })
    .await;
    assert!(after.0 != GO_STATE_ACTIVE && !after.1, "not set off");
    a.wait_for("SMSG_GAMEOBJECT_PAGETEXT", |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_PAGETEXT(p) if p.guid == goober => Some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's "delayed object spells do not follow a user into another
/// copy": a quest object's spell of a second's cast, its user gone to
/// another map meanwhile, isn't cast at all.
#[tokio::test]
async fn a_quest_objects_slow_spell_doesnt_follow_its_user_away() {
    let (realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let pending = on(&map, move |m| {
        let goober =
            objects::summon(m, None, SLOW_GOOBER, at(1.0, 0.0), 0.0, 0, 0, 0, None).unwrap();
        objects::use_goober(m, g, goober);
        let pending = m.objects.pending_spells.len();
        m.teleport(g, MapKey::world(1), at(0.0, 0.0), 0.0);
        pending
    })
    .await;
    assert_eq!(pending, 1);
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    on(&map, move |m| {
        let now = m.now;
        for p in &mut m.objects.pending_spells {
            p.0 = now;
        }
    })
    .await;
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    let (left, cast) = (
        on(&map, |m| m.objects.pending_spells.len()).await,
        on(&kalimdor, move |m| has(m, g, SLOW_SPELL)).await,
    );
    assert_eq!((left, cast), (0, false), "dropped, never cast");
}

/// Thistle Tea's "uses the spawn identity in an instance and excludes wild
/// summons": a spawned object's `gameobject_scripts` run by its spawn's
/// guid; a summoned one of the same entry has none.
#[tokio::test]
async fn an_objects_script_is_its_spawns_and_a_summons_has_none() {
    let mut d = world();
    d.scripts.gameobject.insert(
        STONE_SPAWN,
        vec![ScriptStep {
            command: 80,
            ..Default::default()
        }],
    );
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let seen = on(&map, move |m| {
        let spawned = guid::game_object(STONE, STONE_SPAWN);
        let summoned = objects::summon(m, None, STONE, at(4.0, 0.0), 0.0, 0, 0, 0, None).unwrap();
        crate::game::map::scripts::object_scripts(m, g, spawned);
        crate::game::map::scripts::object_scripts(m, g, summoned);
        crate::game::map::scripts::update(m);
        (state(m, spawned), state(m, summoned))
    })
    .await;
    assert_eq!(seen, (0, 1), "the spawn's opened, the summon's not");
}

/// Thistle Tea's "uses expanded display bounds beyond origin distance": a
/// quest giver whose model reaches ten yards along it gives its quest to
/// one twelve yards from where it stands, by its end; one with no model's
/// box, from as far, doesn't.
#[tokio::test]
async fn a_long_quest_givers_reach_is_its_models() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let boxed = guid::game_object(BOX_GIVER, GIVER_SPAWN);
    let bare = guid::game_object(BARE_GIVER, BARE_SPAWN);
    a.sees(bare).await;
    on(&map, move |m| {
        // The quest's condition met: the item carried.
        crate::game::map::inventory::add_item(m, g, BANKED, 1, false).unwrap();
        place(m, g, at(12.0, -20.0));
    })
    .await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: bare,
        quest_id: OTHER_QUEST,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert!(!on(&map, move |m| m.players[&g].journal.has(OTHER_QUEST)).await);
    on(&map, move |m| place(m, g, at(12.0, 20.0))).await;
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: boxed,
        quest_id: OTHER_QUEST,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    assert!(on(&map, move |m| m.players[&g].journal.has(OTHER_QUEST)).await);
}

/// Thistle Tea's "acceptance re-evaluates a stale bank-backed menu before
/// mutation": a quest offered while its item was in the bank, the item
/// gone since, is refused as accepted (the reason, then the window
/// closed), with nothing logged.
#[tokio::test]
async fn a_quest_whose_banked_item_went_since_its_offer_is_refused() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let boxed = guid::game_object(BOX_GIVER, GIVER_SPAWN);
    // Its first move in before it is placed.
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let offered = on(&map, move |m| {
        place(m, g, at(1.0, 20.0));
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &d,
            guid::item(counter),
            g,
            BANKED,
            crate::game::store::BAG_PLAYER,
            crate::game::map::inventory::BANK_START,
            1,
        ));
        p.refresh_inventory(&d);
        crate::game::map::quests::condition_ok(m, g, boxed, OTHER_QUEST)
    })
    .await;
    assert!(offered, "offered with it banked");
    on(&map, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.retain(|i| i.entry != BANKED);
        p.refresh_inventory(&d);
    })
    .await;
    a.clear();
    a.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: boxed,
        quest_id: OTHER_QUEST,
    })
    .await;
    let reason = a
        .wait_for("SMSG_QUESTGIVER_QUEST_INVALID", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(i) => Some(i.msg),
            _ => None,
        })
        .await;
    assert_eq!(reason, QuestFailedReason::DontHaveReq);
    a.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    assert!(!on(&map, move |m| m.players[&g].journal.has(OTHER_QUEST)).await);
}

/// Thistle Tea's "map event conditions use the scripted event boundary
/// with a nil source": a condition of a map's scripted event (36) is met
/// while the event runs, asked with no source, and not before.
#[tokio::test]
async fn a_map_events_condition_holds_while_it_runs_with_no_source() {
    use crate::game::map::conditions::{Truth, check};
    let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
    let seen = on(&map, move |m| {
        let before = check(m, EVENT_CONDITION, None, Some(g));
        crate::game::map::map_events::start(m, EVENT, g, g, Duration::from_secs(600));
        (before, check(m, EVENT_CONDITION, None, Some(g)))
    })
    .await;
    assert_eq!(seen, (Truth::Unmet, Truth::Met));
}

/// Thistle Tea's "game-object reward spells use the player as caster": a
/// quest turned in at an object casts its reward's spell (one that
/// teaches) from the player, at the player, who learns what it teaches.
#[tokio::test]
async fn an_objects_reward_spell_is_cast_by_the_player() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let boxed = guid::game_object(BOX_GIVER, GIVER_SPAWN);
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    on(&map, move |m| {
        place(m, g, at(1.0, 20.0));
        take(m, g, QUEST);
    })
    .await;
    a.clear();
    a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: boxed,
        quest_id: QUEST,
        reward: 0,
    })
    .await;
    let caster = a
        .wait_for("SMSG_SPELL_GO", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == LESSON => {
                Some((s.caster, s.hits.clone()))
            }
            _ => None,
        })
        .await;
    assert_eq!(caster, (g, vec![g]));
    assert!(on(&map, move |m| m.players[&g].spells.contains(&TAUGHT)).await);
}

/// Thistle Tea's "greets with the spawn's own text when the creature has
/// no menu" and "starts nothing for a greeting without a script": a
/// creature of no menu greets with its spawn's `npc_gossip` text (1693,
/// not the default 68); a menu with no script of its own shows its text
/// and starts no script.
#[tokio::test]
async fn a_spawn_greets_with_its_own_text_and_a_plain_menu_starts_nothing() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let greeter = guid::unit(GREETER, GREETER_SPAWN);
    a.sees(greeter).await;
    a.clear();
    a.send(CMSG_GOSSIP_HELLO { guid: greeter }).await;
    let text = a
        .wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => Some((g.title_text_id, g.gossips.len())),
            _ => None,
        })
        .await;
    assert_eq!(text, (1693, 0));
    a.clear();
    let runs = on(&map, move |m| {
        crate::game::map::npc::show_menu(m, g, greeter, MENU, 0);
        m.scripts.runs.len()
    })
    .await;
    assert_eq!(runs, 0, "no script started");
    let text = a
        .wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
            ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => Some(g.title_text_id),
            _ => None,
        })
        .await;
    assert_eq!(text, 2);
}
