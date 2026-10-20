//! Round 9, the core's entities on made-up data: Thistle Tea's
//! `boundary_result_test.exs` (a totem raised for the dead),
//! `item_test.exs` (a temporary enchantment's charges, and its slot beside
//! the permanent one's), `mob_test.exs` (an EventAI event gated to one
//! spawn, a concealed summon, a corpse a late spell finds, a spell's tap),
//! `liquid_spells_test.exs` (the slime left before its tick),
//! `loot_session_test.exs` (a looter's access gone with the window open),
//! `pvp_test.exs` (a battleground's flag) and `empathy_update_test.exs`
//! (Beast Lore's end).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::BroadcastText;
use crate::game::data::scripts::{
    AiEvent, Condition, EF_REPEATABLE, EVENT_AGGRO, EVENT_TIMER_OOC, ScriptStep, StepExt,
};
use crate::game::data::spells::{Enchantment, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::{CreatureSpawn, CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::ai::DYNFLAG_TAPPED;
use crate::game::map::creature::Creature;
use crate::game::map::enchant::{self, Enchant};
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::map::loot::{self, DYNFLAG_LOOTABLE, Loot, LootSlot};
use crate::game::map::player::{UNIT_FLAG_IN_COMBAT, UNIT_FLAG_PVP};
use crate::game::map::{Map, MapHandle, MapKey, auras, combat, death, environment, procs, spells};
use crate::game::registry::Team;
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_cast_kit as cast_kit;
use crate::game::tests_p3_combat::{self as base, BRUTE, START, SWORD};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_spells_kit as kit;

/// Run the map's updates until `f` holds.
async fn until(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&Map) -> bool + Send + Sync + Clone + 'static,
) {
    crate::game::tests_p3_entity_objects::until(map, what, f).await;
}

/// Wait until `who`'s earlier packets have been handled.
async fn handled(who: &mut Client) {
    who.send(CMSG_NAME_QUERY { guid: who.guid }).await;
    who.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

// ---------------------------------------------------------------------
// A totem raised for the dead (`boundary_result_test.exs`).

const TOTEM_SPELL: u32 = 998_101;
const TOTEM: u32 = 998_150;

/// A totem in the first slot, of a made-up creature.
fn totem_data(d: &mut GameData) {
    d.creature_templates.insert(
        TOTEM,
        CreatureTemplate {
            entry: TOTEM,
            name: "Made-up Totem".into(),
            display_ids: [4587, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 60,
            level_max: 60,
            faction: 35,
            ..Default::default()
        },
    );
    let mut totem = cast_kit::bare(TOTEM_SPELL, 3);
    totem.duration_index = cast_kit::LASTS_LONG;
    cast_kit::effect(
        &mut totem,
        0,
        SpellEffect {
            effect: 87,
            misc_value: TOTEM as i32,
            base_points: 99,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    cast_kit::insert(d, [totem]);
}

/// `g`'s totems: those in their slots, and those of the made-up kind
/// standing on the map.
fn totems(m: &Map, g: Guid) -> (usize, usize) {
    let slotted = m
        .players
        .get(&g)
        .map_or(0, |p| p.owned.totems.iter().flatten().count());
    let standing = m.creatures.iter().filter(|(_, c)| c.entry == TOTEM).count();
    (slotted, standing)
}

/// Thistle Tea's "despawns a totem whose owner died before its start
/// result arrived": a totem a spell raises for a shaman who has died (wvs
/// raises it in the cast, with no result to wait for) goes at its first
/// turn, and their slot is left empty, as mangos's `Totem::Update`
/// unsummons one whose owner isn't alive.
#[tokio::test]
async fn a_totem_raised_for_a_dead_shaman_goes_at_its_first_turn() {
    let realm = cast_kit::fast_realm(totem_data);
    let (_c, a, map) = cast_kit::human(&realm, "P9E1", "Totemic", Class::Warrior).await;
    let raised = on(&map, move |m| {
        kit::ready(m, a, 60, &[]);
        death::kill(m, a);
        spells::cast_triggered(m, a, TOTEM_SPELL, Some(a));
        totems(m, a)
    })
    .await;
    assert_eq!(raised, (1, 1), "raised in the cast, in its slot");
    until(&map, "the totem gone, its slot empty", move |m| {
        totems(m, a) == (0, 0)
    })
    .await;
}

/// Thistle Tea's "rejects a late totem after spirit release": one raised
/// for a shaman whose spirit is released (a ghost, with a point of health)
/// goes at its first turn too, as mangos's `IsAlive` is false for a ghost.
#[tokio::test]
async fn a_totem_raised_for_a_released_spirit_goes_at_its_first_turn() {
    let realm = cast_kit::fast_realm(totem_data);
    let (_c, a, map) = cast_kit::human(&realm, "P9E2", "Released", Class::Warrior).await;
    on(&map, move |m| {
        kit::ready(m, a, 60, &[]);
        death::kill(m, a);
        death::release(m, a);
    })
    .await;
    // No graveyard in the made-up world: the ghost stays where she fell.
    let map = realm.registry.online(a).expect("online").map();
    let raised = on(&map, move |m| {
        let ghost = death::is_ghost(m, a) && kit::health(m, a) > 0;
        spells::cast_triggered(m, a, TOTEM_SPELL, Some(a));
        (ghost, totems(m, a))
    })
    .await;
    assert_eq!(
        raised,
        (true, (1, 1)),
        "a ghost's totem, raised in the cast"
    );
    until(&map, "the ghost's totem gone, its slot empty", move |m| {
        totems(m, a) == (0, 0)
    })
    .await;
}

// ---------------------------------------------------------------------
// A weapon's enchantments (`item_test.exs`).

/// A point of damage at the target; a poison whose combat spell it is,
/// always.
const POISON_HIT: u32 = 98_201;
const POISON: u32 = 9_801;

fn enchant_data() -> GameData {
    let mut d = base::data();
    let mut hit = SpellTemplate {
        id: POISON_HIT,
        name: "Poison".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    hit.effects[0] = SpellEffect {
        effect: 2,
        base_points: 0,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(POISON_HIT, hit);
    d.spells.enchantments.insert(
        POISON,
        Enchantment {
            id: POISON,
            effects: [
                (enchant::TYPE_COMBAT_SPELL, 100, 0, POISON_HIT),
                (0, 0, 0, 0),
                (0, 0, 0, 0),
            ],
            name: "Poison".into(),
        },
    );
    d
}

/// The weapon in `g`'s main hand.
fn main_hand(m: &Map, g: Guid) -> Guid {
    m.players[&g]
        .items
        .iter()
        .find(|i| i.bag == BAG_PLAYER && i.slot == SLOT_MAIN_HAND)
        .expect("a weapon")
        .guid
}

/// `item`'s temporary enchantment.
fn temporary(m: &Map, g: Guid, item: Guid) -> Enchant {
    let p = &m.players[&g];
    enchant::get(&p.items[p.item_by_guid(item).unwrap()], enchant::TEMP_SLOT)
}

/// A melee hit of `g`'s on `victim`, its procs rolled to go off: what it
/// took off the victim's health.
fn hit(m: &mut Map, g: Guid, victim: Guid) -> u32 {
    let before = kit::health(m, victim);
    enchant::FORCED_PROC_ROLL.with(|f| f.set(Some(0.0)));
    procs::trigger(
        m,
        procs::Event {
            actor: g,
            target: victim,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex: procs::EX_NORMAL_HIT,
            damage: 10,
            school: 1,
            ..Default::default()
        },
    );
    enchant::FORCED_PROC_ROLL.with(|f| f.set(None));
    before.saturating_sub(kit::health(m, victim))
}

/// Thistle Tea's "zero means unlimited and stale procs cannot spend
/// replacement charges": a temporary enchantment of no charges goes off
/// on every hit and stays, its charges still none, as mangos's
/// `CastItemCombatSpell` spends only a charge there is; one of two
/// charges spends one a hit and goes with the last. (A proc spends the
/// charge of the enchantment on the weapon as it lands, there being no
/// proc in flight to be stale: the rest is Thistle Tea's tokens.)
#[tokio::test]
async fn a_temporary_enchantment_of_no_charges_lasts_and_one_of_two_spends_them() {
    let (_realm, _a, ga, map) = base::setup_with(enchant_data(), Class::Rogue).await;
    let (unlimited, charged) = on(&map, move |m| {
        base::level(m, ga, 60);
        base::equip(m, ga, SWORD, SLOT_MAIN_HAND);
        let sword = main_hand(m, ga);
        let brute = base::spawn(m, ga, BRUTE, 60, 3.0);
        enchant::enchant_item(m, ga, sword, POISON, Some((60_000, 0)));
        let unlimited: Vec<(u32, Enchant)> = (0..3)
            .map(|_| (hit(m, ga, brute), temporary(m, ga, sword)))
            .collect();
        enchant::enchant_item(m, ga, sword, POISON, Some((60_000, 2)));
        let charged: Vec<(u32, Enchant)> = (0..3)
            .map(|_| (hit(m, ga, brute), temporary(m, ga, sword)))
            .collect();
        (unlimited, charged)
    })
    .await;
    let poison = |charges| Enchant {
        id: POISON,
        duration_ms: 60_000,
        charges,
    };
    assert_eq!(
        unlimited,
        vec![(1, poison(0)); 3],
        "every hit poisons, and it stays"
    );
    assert_eq!(
        charged,
        vec![
            (1, poison(1)),
            (1, Enchant::default()),
            (0, Enchant::default())
        ],
        "a charge a hit, gone with the last"
    );
}

/// Thistle Tea's "permanent and temporary slots coexist through refresh
/// and expiry": a worn weapon's permanent enchantment (41) and temporary
/// one (263) are on it together, each in its slot, and others see both
/// on it; another permanent one (1883) takes the first's place and leaves
/// the temporary one; the temporary one's time up, the permanent one is
/// left, and others see the weapon with it alone.
#[tokio::test]
async fn a_permanent_and_a_temporary_enchantment_share_a_weapon_until_the_temporary_runs_out() {
    let (_realm, _a, ga, map) = base::setup_with(base::data(), Class::Warrior).await;
    let (both, replaced, expired) = on(&map, move |m| {
        base::equip(m, ga, SWORD, SLOT_MAIN_HAND);
        let sword = main_hand(m, ga);
        let state = move |m: &Map| {
            let p = &m.players[&ga];
            let item = &p.items[p.item_by_guid(sword).unwrap()];
            let at = PLAYER_VISIBLE_ITEM_1_0 + u16::from(SLOT_MAIN_HAND) * VISIBLE_ITEM_SIZE;
            (
                enchant::active(item)
                    .map(|(s, e)| (s, e.id))
                    .collect::<Vec<_>>(),
                [0, 1, 2].map(|i| p.fields.get_u32(at + i)),
            )
        };
        enchant::enchant_item(m, ga, sword, 41, None);
        enchant::enchant_item(m, ga, sword, 263, Some((1000, 0)));
        let both = state(m);
        enchant::enchant_item(m, ga, sword, 1883, None);
        let replaced = state(m);
        m.loot_state.enchant_tick_at = Some(m.now - Duration::from_secs(2));
        enchant::update(m);
        (both, replaced, state(m))
    })
    .await;
    assert_eq!(both, (vec![(0, 41), (1, 263)], [SWORD, 41, 263]));
    assert_eq!(replaced, (vec![(0, 1883), (1, 263)], [SWORD, 1883, 263]));
    assert_eq!(expired, (vec![(0, 1883)], [SWORD, 1883, 0]));
}

// ---------------------------------------------------------------------
// EventAI gated to a spawn (`mob_test.exs`).

const THUG: u32 = 38;
const THUG_LINE: u32 = 98_301;
const THUG_SCRIPT: u32 = 98_302;
const ITS_SPAWN: u32 = 98_303;
/// The spawn the event is for, and another of the same entry.
const NAMED: u32 = 80_152;
const OTHER: u32 = 7;

fn thug_spawn(guid: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![THUG],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    }
}

/// Two Defias Thugs, one entry's EventAI: an aggro event for any, and a
/// line out of combat gated by a condition on spawn 80,152.
fn thugs() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        THUG,
        CreatureTemplate {
            entry: THUG,
            name: "Defias Thug".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 10,
            level_max: 10,
            faction: 35,
            unit_class: 1,
            health_max: 500,
            speed_walk: 1.0,
            speed_run: 1.0,
            ai_name: "EventAI".into(),
            ..Default::default()
        },
    );
    d.creature_spawns
        .insert(0, vec![thug_spawn(OTHER, 3.0), thug_spawn(NAMED, 6.0)]);
    // The source's spawn is 80,152 (vmangos's `CONDITION_DB_GUID`).
    d.scripts.conditions.insert(
        ITS_SPAWN,
        Condition {
            entry: ITS_SPAWN,
            kind: 52,
            values: [NAMED as i32, 0, 0, 0],
            flags: 0,
        },
    );
    d.scripts.ai_events.insert(
        THUG,
        vec![
            AiEvent {
                id: 1,
                creature: THUG,
                kind: EVENT_AGGRO,
                chance: 100,
                ..Default::default()
            },
            AiEvent {
                id: 2,
                creature: THUG,
                condition: ITS_SPAWN,
                kind: EVENT_TIMER_OOC,
                chance: 100,
                flags: EF_REPEATABLE,
                params: [0, 0, 600_000, 600_000],
                actions: [THUG_SCRIPT, 0, 0],
                ..Default::default()
            },
        ],
    );
    d.scripts.ai_scripts.insert(
        THUG_SCRIPT,
        vec![ScriptStep {
            id: THUG_SCRIPT,
            command: 0,
            dataint: [THUG_LINE as i32, 0, 0, 0],
            ..Default::default()
        }],
    );
    d.npc.broadcast_texts.insert(
        THUG_LINE,
        BroadcastText {
            male: "This one's mine!".into(),
            chat_type: 1,
            ..Default::default()
        },
    );
    d
}

/// Who yelled the next line `c` hears.
fn yeller(m: &ServerOpcodeMessage) -> Option<Guid> {
    match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
            SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. } => Some(*sender1),
            _ => None,
        },
        _ => None,
    }
}

/// Thistle Tea's "specializes EventAI gates to the spawn's database
/// guid": an EventAI event whose condition names a spawn's database guid
/// fires for that spawn alone, not for another of its entry (vmangos's
/// `CONDITION_DB_GUID` on the event's source).
#[tokio::test]
async fn an_eventai_event_gated_to_a_spawns_guid_fires_for_that_spawn_alone() {
    let realm = harness::realm_with(thugs());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let (named, other) = (guid::unit(THUG, NAMED), guid::unit(THUG, OTHER));
    a.sees(named).await;
    a.sees(other).await;
    let first = a.wait_for("the named thug's line", yeller).await;
    assert_eq!(first, named, "spawn 80,152's");
    // Both thugs' first turns have come by now; a few more.
    let map = realm.registry.online(me).expect("online").map();
    for _ in 0..5 {
        harness::an_update(&map).await;
    }
    let mut rest = Vec::new();
    while let Some(g) = a.gets(NONE_WITHIN, yeller).await {
        rest.push(g);
    }
    assert!(!rest.contains(&other), "not the other thug's: {rest:?}");
}

// ---------------------------------------------------------------------
// A concealed summon (`mob_test.exs`).

const SUMMONER: u32 = 98_401;
const HIDDEN: u32 = 4_490;
const SUMMONER_SPAWN: u32 = 98_410;

/// A realm whose players aren't game masters (the test realm makes
/// everyone one, and a game master sees what is concealed).
fn players_realm(data: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, data, None)
}

fn summoners() -> GameData {
    let mut d = GameData::empty();
    for (entry, name) in [(SUMMONER, "Summoner"), (HIDDEN, "Hidden")] {
        d.creature_templates.insert(
            entry,
            CreatureTemplate {
                entry,
                name: name.into(),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                display_scales: [1.0, 0.0, 0.0, 0.0],
                level_min: 10,
                level_max: 10,
                faction: 35,
                unit_class: 1,
                health_max: 500,
                speed_walk: 1.0,
                speed_run: 1.0,
                ..Default::default()
            },
        );
    }
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: SUMMONER_SPAWN,
            entries: vec![SUMMONER],
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }],
    );
    d
}

/// Thistle Tea's "a concealed summon arrives hidden from players": a
/// script's summon that comes concealed is out, but a player beside it
/// isn't told of it; the script's command showing it (Thistle Tea's
/// `set_concealed`), they are.
#[tokio::test]
async fn a_concealed_summon_arrives_unseen_by_a_player_until_a_script_shows_it() {
    let realm = players_realm(summoners());
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    let summoner = guid::unit(SUMMONER, SUMMONER_SPAWN);
    a.sees(summoner).await;
    let map = realm.registry.online(me).expect("online").map();
    let hidden = on(&map, move |m| {
        let at = m.players[&me].position();
        let step = ScriptStep {
            command: 10,
            datalong: [HIDDEN, 0, 0, 0],
            dataint: [0, 0, -1, 0],
            x: at.x,
            y: at.y + 4.0,
            z: at.z,
            ext: Some(Box::new(StepExt {
                concealed: true,
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(crate::game::map::scripts::execute(m, &step, summoner, me));
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == HIDDEN)
            .map(|(_, c)| c.guid)
            .expect("summoned")
    })
    .await;
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    assert!(
        !a.sees_within(hidden, NONE_WITHIN).await,
        "concealed from a player beside it"
    );
    on(&map, move |m| {
        let show = ScriptStep {
            command: crate::game::map::scripts::C_SET_CONCEALED,
            ..Default::default()
        };
        assert!(crate::game::map::scripts::execute(m, &show, hidden, hidden));
    })
    .await;
    a.sees(hidden).await;
}

// ---------------------------------------------------------------------
// A corpse a late spell finds, and a spell's tap (`mob_test.exs`).

/// A slow at a hostile unit, lasting; a fire bolt of ten.
const CHILL: u32 = 98_501;
const FIREBOLT: u32 = 98_502;
const CHILL_LASTS: u32 = 98_503;

fn spell_data() -> GameData {
    let mut d = base::data();
    d.spells.cast_times.insert(cast_kit::INSTANT, 0);
    d.spells.durations.insert(
        CHILL_LASTS,
        SpellDuration {
            base: 15_000,
            per_level: 0,
            max: 15_000,
        },
    );
    let mut chill = cast_kit::bare(CHILL, 4);
    chill.duration_index = CHILL_LASTS;
    cast_kit::effect(
        &mut chill,
        0,
        cast_kit::aura(auras::AURA_MOD_DECREASE_SPEED, 0, -50, 6),
    );
    let mut bolt = cast_kit::bare(FIREBOLT, 2);
    cast_kit::effect(&mut bolt, 0, cast_kit::damage(10));
    cast_kit::insert(&mut d, [chill, bolt]);
    d
}

/// What a creature shows of a fight: the slow on it, in combat, its
/// threat, its target, alive.
fn fight_of(m: &Map, c: Guid) -> (bool, bool, usize, Guid, bool) {
    let k = m.creature(c).expect("the corpse");
    (
        kit::has(m, c, CHILL),
        k.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
        k.threat.len(),
        k.fields.get_guid(UNIT_FIELD_TARGET),
        k.alive(),
    )
}

/// Thistle Tea's "does not reengage a finalized corpse when a spell proc
/// arrives late": a harmful spell that finds a creature already dead
/// (Winter's Chill proccing after the killing blow) puts nothing on it
/// and starts no fight: no aura, not in combat, no threat, no target,
/// then or after the map's updates (where on a live one it lands).
#[tokio::test]
async fn a_harmful_spell_that_finds_a_corpse_starts_no_fight() {
    let (_realm, _a, ga, map) = base::setup_with(spell_data(), Class::Mage).await;
    let brute = on(&map, move |m| {
        kit::ready(m, ga, 60, &[]);
        let b = base::spawn(m, ga, BRUTE, 20, 4.0);
        combat::kill(m, ga, b);
        b
    })
    .await;
    harness::an_update(&map).await;
    let at_once = on(&map, move |m| {
        spells::cast_triggered(m, ga, CHILL, Some(brute));
        fight_of(m, brute)
    })
    .await;
    let nothing = (false, false, 0, Guid::zero(), false);
    assert_eq!(at_once, nothing, "nothing on the corpse");
    // On one alive, it lands.
    let alive = on(&map, move |m| {
        let b = base::spawn(m, ga, BRUTE, 20, 5.0);
        spells::cast_triggered(m, ga, CHILL, Some(b));
        let (slowed, _, _, _, alive) = fight_of(m, b);
        (slowed, alive)
    })
    .await;
    assert_eq!(alive, (true, true), "a live one is slowed");
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    assert_eq!(
        on(&map, move |m| fight_of(m, brute)).await,
        nothing,
        "nor later"
    );
}

/// The fields of the next values block `c` is told of `of` that `want`
/// takes.
async fn values_of(
    c: &mut Client,
    of: Guid,
    what: &str,
    want: impl Fn(&BTreeMap<u16, u32>) -> bool,
) -> BTreeMap<u16, u32> {
    c.wait_for(what, |m| {
        harness::values(m)
            .into_iter()
            .filter(|(g, _)| *g == of)
            .map(|(_, mask)| crate::game::tests_bugs6::fields(&mask))
            .find(|f| want(f))
    })
    .await
}

/// The dynamic flags in the next values block `c` is told of `of` with
/// them.
async fn flags_of(c: &mut Client, of: Guid) -> u32 {
    values_of(c, of, "its dynamic flags", |f| {
        f.contains_key(&UNIT_DYNAMIC_FLAGS)
    })
    .await[&UNIT_DYNAMIC_FLAGS]
}

/// Thistle Tea's "projects a fireball tap before the first hostile
/// update": a player's damage spell that hits a fresh creature taps it
/// for them as it lands, before any update (mangos's `SetLootRecipient`
/// in `DealDamage`); another player is then told it is grey
/// (`UNIT_DYNFLAG_TAPPED`), its caster not.
#[tokio::test]
async fn a_damage_spell_taps_a_fresh_creature_as_it_lands() {
    let (realm, mut a, ga, map) = base::setup_with(spell_data(), Class::Mage).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let brute = on(&map, move |m| {
        kit::ready(m, ga, 60, &[FIREBOLT]);
        base::spawn(m, ga, BRUTE, 20, 6.0)
    })
    .await;
    a.sees(brute).await;
    b.sees(brute).await;
    a.clear();
    b.clear();
    let at_once = on(&map, move |m| {
        let fresh = m.creature(brute).unwrap().tapped_by;
        let cast = kit::cast_fresh(m, ga, FIREBOLT, kit::at(brute));
        (
            fresh,
            cast,
            kit::health(m, brute),
            m.creature(brute).unwrap().tapped_by,
        )
    })
    .await;
    assert_eq!(
        at_once,
        (None, true, 100_000 - 10, Some(ga)),
        "the bolt's ten, and her tap, as it lands"
    );
    assert_ne!(
        flags_of(&mut b, brute).await & DYNFLAG_TAPPED,
        0,
        "grey to Bob"
    );
    let hers = values_of(&mut a, brute, "its health", |f| {
        f.contains_key(&UNIT_FIELD_HEALTH)
    })
    .await;
    assert_eq!(
        hers.get(&UNIT_DYNAMIC_FLAGS).copied().unwrap_or(0) & DYNFLAG_TAPPED,
        0,
        "not to her: {hers:?}"
    );
}

// ---------------------------------------------------------------------
// The slime left before its tick (`liquid_spells_test.exs`).

const SLIME: u32 = crate::game::map::terrain::SPELL_NAXX_SLIME;

/// Naxxramas's slime as Thistle Tea's test has it: a tenth of every stat,
/// and a hundred nature every two seconds.
fn slime_data() -> GameData {
    let mut d = base::data();
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_MOD_TOTAL_STAT_PERCENTAGE,
        base_points: -91,
        base_dice: 1,
        misc_value: -1,
        target_a: 1,
        ..Default::default()
    };
    effects[1] = SpellEffect {
        effect: 6,
        aura: auras::AURA_PERIODIC_DAMAGE,
        base_points: 99,
        base_dice: 1,
        amplitude: 2000,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        SLIME,
        SpellTemplate {
            id: SLIME,
            name: "Slime".into(),
            school: 3,
            effects,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    d
}

/// `g`'s slime's next tick at `at`.
fn slime_tick_at(m: &mut Map, g: Guid, at: Instant) {
    if let Some(a) = auras::auras_mut(m, g) {
        for h in a.holders.iter_mut().filter(|h| h.spell == SLIME) {
            for x in h.effects.iter_mut().filter(|x| x.period.is_some()) {
                x.next_tick = Some(at);
            }
        }
    }
}

/// `g`'s slime, strength and the liquid spell they are under.
fn slimed(m: &Map, g: Guid) -> (bool, u32, Option<u32>) {
    let p = &m.players[&g];
    (
        p.auras.has_spell(SLIME),
        p.fields.get_u32(UNIT_FIELD_STAT0),
        p.misc.liquid_spell,
    )
}

/// Whether `c` is told of a tick of the slime.
async fn slime_ticked(c: &mut Client) -> bool {
    c.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::SMSG_PERIODICAURALOG(l) if l.spell == SLIME => Some(()),
        _ => None,
    })
    .await
    .is_some()
}

/// Thistle Tea's "leaving liquid removes its aura before an overdue
/// periodic tick": the slime's aura goes as she moves out of it, before
/// its next tick, however overdue (Thistle Tea reconciles the liquid at
/// each move and before the auras tick; mangos at `SetPosition`), not at
/// her next check of the environment: her strength is whole again, her
/// health as it was, and no tick is told. And where her check and an
/// overdue tick fall in one update, the check comes first. (The made-up
/// world has no terrain: no liquid anywhere, so she is out of the slime
/// wherever she moves.)
#[tokio::test]
async fn leaving_the_slime_takes_its_aura_before_an_overdue_tick() {
    let (_realm, mut a, ga, map) = base::setup_with(slime_data(), Class::Warrior).await;
    let (dry, slimed_in) = on(&map, move |m| {
        base::level(m, ga, 60);
        let dry = slimed(m, ga);
        // In the slime, its environment checks held off and its tick far.
        environment::liquid_spell(m, ga, Some(SLIME));
        m.players.get_mut(&ga).unwrap().misc.env_checked_at =
            Some(Instant::now() + Duration::from_secs(3600));
        slime_tick_at(m, ga, Instant::now() + Duration::from_secs(3600));
        (dry, slimed(m, ga))
    })
    .await;
    assert_eq!(dry, (false, 140, None));
    assert_eq!(
        slimed_in,
        (true, 14, Some(SLIME)),
        "a tenth of her strength"
    );
    // She steps out.
    a.clear();
    a.heartbeat(START.0 + 3.0, START.1, START.2).await;
    handled(&mut a).await;
    let (out, health, after) = on(&map, move |m| {
        let out = slimed(m, ga);
        let health = kit::health(m, ga);
        // Were it on still, an overdue tick would land now.
        slime_tick_at(m, ga, m.now - Duration::from_secs(8));
        auras::update(m);
        (out, health, (slimed(m, ga), kit::health(m, ga)))
    })
    .await;
    assert_eq!(out, (false, 140, None), "gone as she stepped out");
    assert_eq!(after, (out, health), "and no tick");
    assert!(!slime_ticked(&mut a).await, "none told");
    // On her again where she stands, out of any liquid, with a tick
    // overdue and her check due: the update's check takes it first.
    a.clear();
    let health = on(&map, move |m| {
        environment::liquid_spell(m, ga, Some(SLIME));
        slime_tick_at(m, ga, m.now - Duration::from_secs(8));
        m.players.get_mut(&ga).unwrap().misc.env_checked_at = None;
        kit::health(m, ga)
    })
    .await;
    harness::an_update(&map).await;
    let (checked, now) = on(&map, move |m| (slimed(m, ga), kit::health(m, ga))).await;
    assert_eq!(checked, (false, 140, None), "checked before the auras tick");
    assert!(now >= health, "no tick: {health} then {now}");
    assert!(!slime_ticked(&mut a).await, "none told");
}

// ---------------------------------------------------------------------
// A looter's access gone (`loot_session_test.exs`).

/// Whether `CMSG_LOOT` on `corpse` opens a window for `c`.
async fn opened(c: &mut Client, corpse: Guid) -> bool {
    c.clear();
    c.raw.clear();
    c.send(CMSG_LOOT { guid: corpse }).await;
    handled(c).await;
    c.backlog
        .iter()
        .any(|m| matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)))
        || c.raw
            .iter()
            .any(|f| u16::from_le_bytes([f[2], f[3]]) == loot::SMSG_LOOT_RESPONSE)
}

/// Thistle Tea's "revoked source access blocks visibility, money,
/// reservations, and a pending transfer": a player whose right to a
/// corpse's loot goes while its window is open (it was anyone's, it is
/// another's now) takes neither its money nor its item from the window,
/// nor opens it again; the one it is now takes both. (Thistle Tea's
/// reservations and pending transfers are its own; wvs's take is the
/// transfer, and asks `Loot::may_loot` as it is made.)
#[tokio::test]
async fn one_whose_right_to_a_corpse_goes_with_its_window_open_takes_nothing_from_it() {
    let (realm, mut a, ga, map) = base::setup_with(base::data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let corpse = on(&map, move |m| {
        let c = base::spawn(m, ga, BRUTE, 20, 2.0);
        combat::kill(m, ga, c);
        let k = m.creature_mut(c).unwrap();
        crate::game::map::tap::clear(k);
        k.loot = Some(Loot {
            gold: 50,
            items: vec![LootSlot {
                item: SWORD,
                count: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
        k.fields
            .set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_LOOTABLE, true);
        c
    })
    .await;
    assert!(opened(&mut b, corpse).await, "anyone's: Bob looks in");
    on(&map, move |m| {
        loot::loot_mut(m, corpse).unwrap().owner = Some(ga);
    })
    .await;
    let had = on(&map, move |m| m.players[&gb].money()).await;
    b.send(CMSG_LOOT_MONEY {}).await;
    b.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    handled(&mut b).await;
    let left = on(&map, move |m| {
        let l = loot::loot_ref(m, corpse).unwrap();
        (
            m.players[&gb].money() - had,
            m.players[&gb].count_of(SWORD),
            l.gold,
            l.items[0].taken,
        )
    })
    .await;
    assert_eq!(left, (0, 0, 50, false), "Bob takes nothing");
    assert!(!opened(&mut b, corpse).await, "nor looks in again");
    assert!(opened(&mut a, corpse).await, "Alice's now");
    let hers = on(&map, move |m| m.players[&ga].money()).await;
    a.send(CMSG_LOOT_MONEY {}).await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    handled(&mut a).await;
    let took = on(&map, move |m| {
        (
            m.players[&ga].money() - hers,
            m.players[&ga].count_of(SWORD),
        )
    })
    .await;
    assert_eq!(took, (50, 1), "she takes both");
}

// ---------------------------------------------------------------------
// A battleground's flag (`pvp_test.exs`).

/// Thistle Tea's "battlegrounds enforce the flag independently of zone
/// data": in a battleground's copy, either side is held to the PvP flag
/// before any area is looked up, its zone known to the data or not, as
/// mangos's `UpdatePvP` in a battleground; and both players who go in are
/// flagged there, in a zone the made-up data knows nothing of, though
/// neither was outside.
#[tokio::test]
async fn a_battleground_flags_both_sides_whatever_its_zone_data() {
    let realm = harness::realm_with(crate::game::tests_pvp::arathi());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Grom", Race::Orc, Class::Warrior).await;
    let flagged = |m: &Map, g: Guid| {
        m.players
            .get(&g)
            .is_some_and(|p| p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP))
    };
    let world = realm.maps.get(&realm, MapKey::world(0));
    let outside = on(&world, move |m| {
        (
            crate::game::map::pvp::enforced(m, Team::Alliance, 0),
            flagged(m, ga),
        )
    })
    .await;
    assert_eq!(outside, (false, false), "zone 0 of the world, unflagged");
    let bg = crate::game::tests_pvp::both_into(
        &realm,
        &mut a,
        &mut b,
        wow_world_messages::vanilla::Map::ArathiBasin,
    )
    .await;
    let copy = on(&bg, move |m| {
        (
            m.key.instance != 0,
            crate::game::map::pvp::enforced(m, Team::Alliance, 0),
            crate::game::map::pvp::enforced(m, Team::Horde, 0),
            [ga, gb].map(|g| m.realm.data.area(m.players[&g].zone).is_none()),
        )
    })
    .await;
    assert_eq!(
        copy,
        (true, true, true, [true, true]),
        "a copy, enforced for either side, no zone data"
    );
    until(&bg, "both flagged in the battleground", move |m| {
        flagged(m, ga) && flagged(m, gb)
    })
    .await;
}

// ---------------------------------------------------------------------
// Beast Lore's end (`empathy_update_test.exs`).

const LORE: u32 = 98_701;

fn empathy_world() -> GameData {
    use crate::game::tests_pets::{data, effect, spell};
    let mut d = data();
    let none = SpellEffect::default();
    let mut e = effect(6, 0, 6);
    e.aura = auras::AURA_EMPATHY;
    let mut s = spell(LORE, "Lore", [e, none, none]);
    s.duration_index = 4;
    d.spells.spells.insert(s.id, s);
    d
}

/// The fields of `c`'s create of `of`.
async fn create_of(c: &mut Client, of: Guid) -> BTreeMap<u16, u32> {
    c.wait_for("its create", |m| {
        let objects = match m {
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
            ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
            _ => return None,
        };
        objects.iter().find_map(|o| match &o.update_type {
            Object_UpdateType::CreateObject { guid3, mask2, .. }
            | Object_UpdateType::CreateObject2 { guid3, mask2, .. }
                if *guid3 == of =>
            {
                Some(crate::game::tests_bugs6::fields(mask2))
            }
            _ => None,
        })
    })
    .await
}

/// Thistle Tea's "removal clears the caster's tooltip flag and stops
/// disclosing fields": Beast Lore's Empathy taken off a creature takes
/// its tooltip flag (`UNIT_DYNFLAG_SPECIALINFO`) off for its caster, and
/// for another, as mangos's `HandleAuraEmpathy` removes it; neither the
/// block that tells them so nor a later change of its resistances and
/// damage discloses those to the caster any more.
#[tokio::test]
async fn beast_lores_end_takes_the_tooltip_flag_and_the_special_fields_from_its_caster() {
    use crate::game::map::pets::DYNFLAG_SPECIALINFO;
    use crate::game::tests_pets::{BOAR, enter};
    let realm = harness::realm_with(empathy_world());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut b, _gb, _) = enter(&realm, "B", "Bo", Race::Dwarf, Class::Hunter).await;
    let fire = UNIT_FIELD_RESISTANCES + 2;
    let boar = on(&map, move |m| {
        let t = m.realm.data.creature_template(BOAR).cloned().unwrap();
        let at = m.players[&ga].position();
        let mut c = Creature::new(guid::unit(BOAR, 0x71_0102), &t, 1, at, 0.0);
        c.fields.set_u32(fire, 12);
        m.add_creature(c.with_base())
    })
    .await;
    create_of(&mut a, boar).await;
    create_of(&mut b, boar).await;
    a.clear();
    b.clear();
    on(&map, move |m| {
        let s = m.realm.data.spells.spells[&LORE].clone();
        auras::apply_spell(m, ga, boar, &s, &[(0, 0)], Default::default()).expect("put on");
    })
    .await;
    let told = values_of(&mut a, boar, "its special information", |f| {
        f.contains_key(&fire)
    })
    .await;
    assert_eq!(told.get(&fire), Some(&12), "disclosed while it is on");
    assert_ne!(
        flags_of(&mut b, boar).await & DYNFLAG_SPECIALINFO,
        0,
        "its flag up"
    );
    a.clear();
    b.clear();
    on(&map, move |m| auras::remove_spell(m, boar, LORE, Some(ga))).await;
    let ended = values_of(&mut a, boar, "the flag's end", |f| {
        f.contains_key(&UNIT_DYNAMIC_FLAGS)
    })
    .await;
    assert_eq!(
        ended[&UNIT_DYNAMIC_FLAGS] & DYNFLAG_SPECIALINFO,
        0,
        "off for the caster"
    );
    assert!(
        !ended.contains_key(&fire) && !ended.contains_key(&UNIT_FIELD_MAXDAMAGE),
        "not disclosed with it: {ended:?}"
    );
    assert_eq!(
        flags_of(&mut b, boar).await & DYNFLAG_SPECIALINFO,
        0,
        "off for another"
    );
    a.clear();
    on(&map, move |m| {
        let f = &mut m.creature_mut(boar).unwrap().fields;
        f.set_u32(fire, 30);
        f.set_f32(UNIT_FIELD_MAXDAMAGE, 99.0);
        f.set_u32(UNIT_FIELD_HEALTH, 5);
    })
    .await;
    let later = values_of(&mut a, boar, "its health", |f| {
        f.contains_key(&UNIT_FIELD_HEALTH)
    })
    .await;
    assert!(
        !later.contains_key(&fire) && !later.contains_key(&UNIT_FIELD_MAXDAMAGE),
        "a later change not disclosed: {later:?}"
    );
}
