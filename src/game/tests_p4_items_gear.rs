//! Phase 4's gear, Thistle Tea's `EquipmentSpellsTest`, `EquipmentAurasTest`,
//! `EquipmentSetsTest`, `EquipmentTransitionsTest` and `ProficiencyTest`
//! ported: what worn items' spells, enchantments and sets give (once, and
//! from each source apart; through death, not cancelled; held to their
//! forms and to the outdoors; their procs' cooldowns kept), swapping gear
//! (the thirty-second cooldown a trinket put on starts, the lock on
//! weapons in a fight, the global cooldown), and what a player's spells
//! let them wear.
//!
//! Gear is put on and taken off within one map query, and its auras made
//! to match (`inventory::sync_gear`) and the stats derived there, so the
//! map's own update doesn't race the checks.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::crafts::ItemSet;
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{Enchantment, ProcEvent, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, *};
use crate::game::map::enchant;
use crate::game::map::inventory::{self, Proficiency};
use crate::game::map::{Map, MapKey, derive};
use crate::game::store::BAG_PLAYER;

/// Equipment slots.
const HEAD: u8 = 0;
const CHEST: u8 = 4;
const FINGER1: u8 = 10;
const FINGER2: u8 = 11;
const TRINKET1: u8 = 12;
const TRINKET2: u8 = 13;
const MAIN: u8 = 15;
const OFF: u8 = 16;
const RANGED: u8 = 17;
const BAG1: u8 = 19;
const PACK: u8 = 23;
const BANK: u8 = 39;

/// Items.
const RING: u32 = 7001;
const RING_TOO: u32 = 7002;
const VEST: u32 = 7003;
const MIXED_VEST: u32 = 7004;
const DEFENDER: u32 = 7005;
const MANA_VEST: u32 = 7006;
const CASTER_VEST: u32 = 7007;
const PROC_VEST: u32 = 7008;
const FIVE_SPELLS: u32 = 7009;
const QUIVER: u32 = 7010;
const FERAL_VEST: u32 = 7011;
const PLAIN_VEST: u32 = 7012;
const PLAIN_HELM: u32 = 7013;
const SWORD: u32 = 7014;
const PLAIN_RING: u32 = 7015;
/// Set pieces: two of one set, two of another with the same spell, an
/// outdoor set, a set with a proc, a modifier, a form's bonus.
const SET_HELM: u32 = 7101;
const SET_VEST: u32 = 7102;
const OTHER_HELM: u32 = 7103;
const OTHER_VEST: u32 = 7104;
const OUTDOOR_HELM: u32 = 7105;
const PROC_HELM: u32 = 7106;
const MOD_HELM: u32 = 7107;
const FERAL_HELM: u32 = 7108;
const SET: u32 = 501;
const OTHER_SET: u32 = 502;
const OUTDOOR_SET: u32 = 503;
const PROC_SET: u32 = 504;
const MOD_SET: u32 = 505;
const FERAL_SET: u32 = 506;
/// Trinkets and weapons for swapping.
const TRINKET: u32 = 7201;
const TRINKET_TOO: u32 = 7207;
const EXEMPT_TRINKET: u32 = 7202;
const RELIC: u32 = 7203;
const SHIELD: u32 = 7204;
const HELD: u32 = 7205;
const AXE: u32 = 7206;

/// Spells.
const CRIT_2: u32 = 9_600_001;
const MIXED: u32 = 9_600_002;
const DEFENSE: u32 = 9_600_003;
const CRIT_3: u32 = 9_600_004;
const MP5: u32 = 9_600_005;
const SPELL_HIT_CRIT: u32 = 9_600_006;
const PROC: u32 = 9_600_007;
const PROC_HIT: u32 = 9_600_008;
const HASTE: u32 = 9_600_009;
const HEALTH_5: u32 = 9_600_010;
const HEALTH_10: u32 = 9_600_011;
const HEALTH_20: u32 = 9_600_012;
const SKILL_1: u32 = 9_600_013;
const SPEED: u32 = 9_600_014;
const FLAT_MOD: u32 = 9_600_015;
const FERAL_AP: u32 = 9_600_016;
const FERAL_HEALTH: u32 = 9_600_017;
const CAT_FORM: u32 = 9_600_018;
const TRINKET_USE: u32 = 9_600_019;
const ON_USE_2: u32 = 9_600_020;
const ON_USE_3: u32 = 9_600_021;
const ON_USE_5: u32 = 9_600_022;
/// Weapon switch's global cooldown (mangos's
/// `SPELL_ID_WEAPON_SWITCH_COOLDOWN_1_5s`).
const WEAPON_SWITCH: u32 = 6119;
/// Enchantments: health, skill, a proc, armor and damage.
const ENCHANT_HEALTH: u32 = 801;
const ENCHANT_SKILL: u32 = 802;
const ENCHANT_HEALTH_20: u32 = 803;
const ENCHANT_ARMOR_DAMAGE: u32 = 804;

const DEFENSE_SKILL: u32 = 95;

fn item(entry: u32, class: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class,
        subclass,
        inventory_type,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

/// An item giving `spells` while worn.
fn giving(entry: u32, inventory_type: u32, spells: &[u32]) -> ItemTemplate {
    let mut t = item(entry, 4, 1, inventory_type);
    for (i, s) in spells.iter().enumerate() {
        t.spells[i] = ItemSpell {
            id: *s,
            trigger: inventory::ITEM_SPELL_ON_EQUIP,
            ..Default::default()
        };
    }
    t
}

fn piece(entry: u32, inventory_type: u32, set: u32) -> ItemTemplate {
    ItemTemplate {
        set_id: set,
        ..item(entry, 4, 1, inventory_type)
    }
}

/// An item whose use casts `spell`, as a trinket's does.
fn on_use(entry: u32, inventory_type: u32, spell: u32) -> ItemTemplate {
    let mut t = item(entry, 4, 0, inventory_type);
    t.spells[0] = ItemSpell {
        id: spell,
        trigger: inventory::ITEM_SPELL_ON_USE,
        cooldown: 120_000,
        ..Default::default()
    };
    t
}

/// A passive aura spell of `effects` (kind, amount, misc) on its caster.
fn passive(id: u32, effects: &[(u32, i32, i32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    for (i, (kind, amount, misc)) in effects.iter().enumerate() {
        s.effects[i] = SpellEffect {
            effect: 6,
            aura: *kind,
            base_points: amount - 1,
            base_dice: 1,
            misc_value: *misc,
            target_a: 1,
            ..Default::default()
        };
    }
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let spells = [
        passive(CRIT_2, &[(AURA_MOD_CRIT_PERCENT, 2, 0)]),
        passive(
            MIXED,
            &[
                (AURA_MOD_ATTACK_POWER, 20, 0),
                (AURA_MOD_CRIT_PERCENT, 2, 0),
                (AURA_MOD_HIT_CHANCE, 1, 0),
            ],
        ),
        passive(
            DEFENSE,
            &[
                (AURA_MOD_SKILL, 5, DEFENSE_SKILL as i32),
                (AURA_MOD_PARRY_PERCENT, 2, 0),
            ],
        ),
        passive(MP5, &[(AURA_MOD_POWER_REGEN, 15, 0)]),
        passive(
            SPELL_HIT_CRIT,
            &[
                (AURA_MOD_SPELL_HIT_CHANCE, 2, 0),
                (AURA_MOD_SPELL_CRIT_CHANCE, 3, 0),
            ],
        ),
        passive(HASTE, &[(AURA_MOD_RANGED_HASTE, 15, 0)]),
        passive(HEALTH_5, &[(AURA_MOD_INCREASE_HEALTH, 5, 0)]),
        passive(HEALTH_10, &[(AURA_MOD_INCREASE_HEALTH, 10, 0)]),
        passive(HEALTH_20, &[(AURA_MOD_INCREASE_HEALTH, 20, 0)]),
        passive(SKILL_1, &[(AURA_MOD_SKILL, 1, DEFENSE_SKILL as i32)]),
        passive(FERAL_HEALTH, &[(AURA_MOD_INCREASE_HEALTH, 5, 0)]),
        passive(
            FERAL_AP,
            &[
                (AURA_MOD_ATTACK_POWER, 100, 0),
                (AURA_MOD_CRIT_PERCENT, 2, 0),
            ],
        ),
    ];
    for s in spells {
        d.spells.spells.insert(s.id, s);
    }
    // An ordinary crit buff, cast.
    let mut crit_3 = passive(CRIT_3, &[(AURA_MOD_CRIT_PERCENT, 3, 0)]);
    crit_3.attributes[0] = 0;
    d.spells.spells.insert(CRIT_3, crit_3);
    // Spells held to cat form.
    for id in [FERAL_AP, FERAL_HEALTH] {
        d.spells.spells.get_mut(&id).unwrap().stances = 1;
    }
    let mut cat = passive(CAT_FORM, &[(AURA_MOD_SHAPESHIFT, 1, 1)]);
    cat.attributes[0] = 0;
    d.spells.spells.insert(CAT_FORM, cat);
    // A run speed bonus for outdoors only.
    let mut speed = passive(SPEED, &[(AURA_MOD_SPEED_ALWAYS, 10, 0)]);
    speed.attributes[0] |= crate::game::data::spells::ATTR_ONLY_OUTDOORS;
    d.spells.spells.insert(SPEED, speed);
    // A proc on melee hits, at most once a second, and what it casts.
    let mut proc = passive(PROC, &[(AURA_PROC_TRIGGER_SPELL, 0, 0)]);
    proc.effects[0].trigger_spell = PROC_HIT;
    proc.proc_flags = crate::game::map::procs::PROC_DONE_MELEE;
    proc.proc_chance = 100;
    d.spells.spells.insert(PROC, proc);
    d.spells.proc_events.insert(
        PROC,
        ProcEvent {
            cooldown: 1000,
            ..Default::default()
        },
    );
    let mut hit = passive(PROC_HIT, &[]);
    hit.attributes[0] = 0;
    hit.effects[0] = SpellEffect {
        effect: 3,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(PROC_HIT, hit);
    // A flat modifier to the spells of family bit 0's damage.
    let mut flat = passive(FLAT_MOD, &[(AURA_ADD_FLAT_MODIFIER, 20, 0)]);
    flat.effects[0].item_type = 1;
    d.spells.spells.insert(FLAT_MOD, flat);
    // On-use spells, and the weapon switch's global cooldown.
    for id in [TRINKET_USE, ON_USE_2, ON_USE_3, ON_USE_5] {
        let mut s = passive(id, &[]);
        s.attributes[0] = 0;
        s.effects[0] = SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    let mut switch = passive(WEAPON_SWITCH, &[]);
    switch.attributes[0] = 0;
    switch.start_recovery_time = 1500;
    switch.start_recovery_category = 133;
    d.spells.spells.insert(WEAPON_SWITCH, switch);

    let mut five = item(FIVE_SPELLS, 4, 1, 5);
    for (i, (id, trigger)) in [
        (TRINKET_USE, 0),
        (ON_USE_2, 1),
        (ON_USE_3, 2),
        (ON_USE_2, 1),
        (ON_USE_5, 1),
    ]
    .into_iter()
    .enumerate()
    {
        five.spells[i] = ItemSpell {
            id,
            trigger,
            ..Default::default()
        };
    }
    // On-equip spells that are no auras: pretend they are, to see which
    // are taken.
    for id in [ON_USE_2, ON_USE_3, ON_USE_5] {
        d.spells.spells.get_mut(&id).unwrap().effects[0] = SpellEffect {
            effect: 6,
            aura: AURA_DUMMY,
            target_a: 1,
            ..Default::default()
        };
    }
    let mut quiver = giving(QUIVER, 18, &[HASTE]);
    quiver.class = 11;
    quiver.container_slots = 4;
    let mut ring = giving(RING, 11, &[CRIT_2]);
    ring.max_durability = 50;
    let sword = ItemTemplate {
        delay: 2000,
        damages: {
            let mut d = [(0.0, 0.0, 0); 5];
            d[0] = (10.0, 20.0, 0);
            d
        },
        ..item(SWORD, 2, 7, 13)
    };
    for t in [
        ring,
        giving(RING_TOO, 11, &[CRIT_2]),
        giving(VEST, 5, &[CRIT_2]),
        giving(MIXED_VEST, 5, &[MIXED]),
        giving(DEFENDER, 5, &[DEFENSE]),
        giving(MANA_VEST, 5, &[MP5]),
        giving(CASTER_VEST, 5, &[SPELL_HIT_CRIT]),
        giving(PROC_VEST, 5, &[PROC]),
        giving(FERAL_VEST, 5, &[FERAL_AP]),
        five,
        quiver,
        item(PLAIN_VEST, 4, 1, 5),
        item(PLAIN_HELM, 4, 1, 1),
        item(PLAIN_RING, 4, 0, 11),
        sword,
        piece(SET_HELM, 1, SET),
        piece(SET_VEST, 5, SET),
        piece(OTHER_HELM, 1, OTHER_SET),
        piece(OTHER_VEST, 5, OTHER_SET),
        piece(OUTDOOR_HELM, 1, OUTDOOR_SET),
        piece(PROC_HELM, 1, PROC_SET),
        piece(MOD_HELM, 1, MOD_SET),
        piece(FERAL_HELM, 1, FERAL_SET),
        on_use(TRINKET, 12, TRINKET_USE),
        on_use(TRINKET_TOO, 12, ON_USE_3),
        item(SHIELD, 4, 6, 14),
        item(HELD, 4, 0, 23),
        item(AXE, 2, 0, 13),
    ] {
        d.items.insert(t.entry, t);
    }
    let mut exempt = on_use(EXEMPT_TRINKET, 12, TRINKET_USE);
    exempt.flags = 0x80;
    d.items.insert(EXEMPT_TRINKET, exempt);
    let mut relic = on_use(RELIC, 28, ON_USE_5);
    relic.spells[0] = ItemSpell::default();
    d.items.insert(RELIC, relic);
    // Durable set pieces, to be broken.
    for e in [SET_HELM, SET_VEST] {
        d.items.get_mut(&e).unwrap().max_durability = 50;
    }
    for (id, items, spells) in [
        (SET, vec![SET_HELM, SET_VEST], vec![(HEALTH_5, 2)]),
        (OTHER_SET, vec![OTHER_HELM, OTHER_VEST], vec![(HEALTH_5, 2)]),
        (OUTDOOR_SET, vec![OUTDOOR_HELM], vec![(SPEED, 1)]),
        (PROC_SET, vec![PROC_HELM], vec![(PROC, 1)]),
        (MOD_SET, vec![MOD_HELM], vec![(FLAT_MOD, 1)]),
        (
            FERAL_SET,
            vec![FERAL_HELM],
            vec![(FERAL_HEALTH, 1), (HEALTH_10, 1)],
        ),
    ] {
        d.crafts.item_sets.insert(
            id,
            ItemSet {
                id,
                items,
                spells,
                ..Default::default()
            },
        );
    }
    for (id, effects) in [
        (
            ENCHANT_HEALTH,
            [(3, 0, 0, HEALTH_5), (0, 0, 0, 0), (0, 0, 0, 0)],
        ),
        (
            ENCHANT_SKILL,
            [(3, 0, 0, SKILL_1), (0, 0, 0, 0), (0, 0, 0, 0)],
        ),
        (
            ENCHANT_HEALTH_20,
            [(3, 0, 0, HEALTH_20), (0, 0, 0, 0), (0, 0, 0, 0)],
        ),
        (
            ENCHANT_ARMOR_DAMAGE,
            [(2, 5, 5, 0), (4, 8, 8, 0), (0, 0, 0, 0)],
        ),
    ] {
        d.spells.enchantments.insert(
            id,
            Enchantment {
                id,
                effects,
                name: format!("Enchant {id}"),
            },
        );
    }
    d
}

/// Run `f` on the map Alice (the first player) is on.
async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    let key = realm
        .registry
        .online(crate::game::guid::player(1))
        .map_or(MapKey::world(0), |o| o.map().key);
    realm
        .maps
        .get(realm, key)
        .query(f)
        .await
        .expect("the map answers")
}

/// Alice, of `class`, wearing and carrying nothing, knowing Parry, at
/// level 60 with her Defense.
async fn setup_as(class: Class) -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let race = if class == Class::Druid {
        Race::NightElf
    } else {
        Race::Human
    };
    let g = a.enter("Alice", race, class).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.spells.push(derive::SPELL_PARRY);
        crate::game::map::stats::set_level(p, &d, 60);
        crate::game::map::skills::learn_skill(p, DEFENSE_SKILL, 1);
        crate::game::map::skills::set_value(p, DEFENSE_SKILL, 300);
        p.refresh_inventory(&d);
        resync(m, g);
    })
    .await;
    a.clear();
    (realm, a, g)
}

async fn setup() -> (Arc<Realm>, Client, Guid) {
    setup_as(Class::Warrior).await
}

/// The gear's auras made to match what is worn, and the stats derived.
fn resync(m: &mut Map, g: Guid) {
    let d = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    inventory::sync_gear(m, g);
    derive::unit(m, g);
}

/// A new item of `entry` at `(bag, slot)`, its auras synced; its guid.
fn put(m: &mut Map, g: Guid, entry: u32, at: (u8, u8)) -> Guid {
    let d = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let guid = crate::game::guid::item(counter);
    let p = m.players.get_mut(&g).unwrap();
    assert!(p.item_at(at.0, at.1).is_none(), "{at:?} is free");
    p.items.push(crate::game::map::player::Item::new(
        &d, guid, g, entry, at.0, at.1, 1,
    ));
    resync(m, g);
    guid
}

fn wear(m: &mut Map, g: Guid, entry: u32, slot: u8) -> Guid {
    put(m, g, entry, (BAG_PLAYER, slot))
}

/// Take off what is in `slot`, gone.
fn off(m: &mut Map, g: Guid, slot: u8) {
    let p = m.players.get_mut(&g).unwrap();
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    resync(m, g);
}

/// Move what is worn in `from` to `to` directly (no checks), as gear is
/// moved about.
fn shift(m: &mut Map, g: Guid, from: u8, to: u8) {
    let p = m.players.get_mut(&g).unwrap();
    let i = p.item_at(BAG_PLAYER, from).unwrap();
    p.items[i].slot = to;
    resync(m, g);
}

/// The gear's auras: their item and spell, in order.
fn gear_auras(m: &Map, g: Guid) -> Vec<(Option<Guid>, u32)> {
    let mut v: Vec<(Option<Guid>, u32)> = m.players[&g]
        .auras
        .holders
        .iter()
        .filter(|h| h.equipped)
        .map(|h| (h.item, h.spell))
        .collect();
    v.sort_by_key(|(i, s)| (i.map(|g| g.guid()), *s));
    v
}

fn crit(m: &Map, g: Guid) -> (f32, f32) {
    let f = &m.players[&g].fields;
    (
        f.get_f32(PLAYER_CRIT_PERCENTAGE),
        f.get_f32(PLAYER_RANGED_CRIT_PERCENTAGE),
    )
}

fn max_health(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(UNIT_FIELD_MAXHEALTH)
}

fn health(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH)
}

fn defense_bonus(m: &Map, g: Guid) -> u32 {
    let p = &m.players[&g];
    crate::game::map::skills::value_with_bonuses(p, DEFENSE_SKILL)
        - crate::game::map::skills::value(p, DEFENSE_SKILL).unwrap()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.001
}

/// Thistle Tea's `spell_ids/1`: of an item's five spells, those it gives
/// while worn (trigger 1), each once; nothing of an item with none.
#[tokio::test]
async fn an_item_gives_its_equip_spells_from_all_five_slots_once() {
    let (realm, _a, g) = setup().await;
    let (five, plain) = on_map(&realm, move |m| {
        let item = wear(m, g, FIVE_SPELLS, CHEST);
        let five = gear_auras(m, g);
        off(m, g, CHEST);
        wear(m, g, PLAIN_VEST, CHEST);
        (
            five.into_iter()
                .map(|(i, s)| (i == Some(item), s))
                .collect::<Vec<_>>(),
            gear_auras(m, g),
        )
    })
    .await;
    assert_eq!(five, vec![(true, ON_USE_2), (true, ON_USE_5)]);
    assert_eq!(plain, vec![]);
}

/// Thistle Tea's `usable_equipped_items/2`: what is worn gives its spells,
/// a bag on the bag bar too (a quiver's haste); not a broken item, nor
/// what is carried or in the bank.
#[tokio::test]
async fn worn_gear_and_bags_give_their_spells_but_not_broken_carried_or_banked_gear() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let vest = wear(m, g, VEST, CHEST);
        let ring = wear(m, g, RING, FINGER1);
        let quiver = wear(m, g, QUIVER, BAG1);
        put(m, g, RING_TOO, (BAG_PLAYER, PACK));
        put(m, g, RING_TOO, (BAG_PLAYER, BANK));
        let all = gear_auras(m, g);
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(ring).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
        resync(m, g);
        (vest, ring, quiver, all, gear_auras(m, g))
    })
    .await;
    let (vest, ring, quiver, all, broken) = got;
    let mut want = vec![
        (Some(vest), CRIT_2),
        (Some(ring), CRIT_2),
        (Some(quiver), HASTE),
    ];
    want.sort_by_key(|(i, _)| i.unwrap().guid());
    assert_eq!(all, want);
    want.retain(|(i, _)| *i != Some(ring));
    assert_eq!(broken, want, "the broken ring's crit gone");
}

/// Thistle Tea's identical passives: the same crit from two rings stands
/// twice, melee and ranged, and each ring takes its own off.
#[tokio::test]
async fn the_same_passive_from_two_items_stacks_and_goes_with_each() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let bare = crit(m, g);
        let r1 = wear(m, g, RING, FINGER1);
        let r2 = wear(m, g, RING_TOO, FINGER2);
        let both = (
            crit(m, g),
            gear_auras(m, g).len(),
            m.players[&g].derived.crit,
        );
        resync(m, g);
        let again = (crit(m, g), gear_auras(m, g).len());
        off(m, g, FINGER1);
        let one = (crit(m, g), gear_auras(m, g));
        off(m, g, FINGER2);
        let none = (crit(m, g), gear_auras(m, g).len());
        (bare, both, again, one, none, r1, r2)
    })
    .await;
    let (bare, both, again, one, none, _r1, r2) = got;
    assert!(
        close(both.0.0, bare.0 + 4.0) && close(both.0.1, bare.1 + 4.0),
        "{both:?}"
    );
    assert_eq!(both.1, 2);
    assert!(close(both.2, both.0.0), "the combat table's crit");
    assert_eq!(
        (again.0, again.1),
        (both.0, both.1),
        "a resync changes nothing"
    );
    assert!(close(one.0.0, bare.0 + 2.0), "{one:?}");
    assert_eq!(one.1, vec![(Some(r2), CRIT_2)]);
    assert_eq!(none, (bare, 0));
}

/// Thistle Tea's mixed equip spells: one spell's attack power, crit and
/// hit apply once, to the fields and to the combat table, and again only
/// once on a resync.
#[tokio::test]
async fn an_equip_spells_attack_power_crit_and_hit_apply_once() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let read = |m: &Map| {
            let p = &m.players[&g];
            (
                p.fields.get_u32(UNIT_FIELD_ATTACK_POWER_MODS),
                p.derived.attack_power,
                p.derived.crit,
                p.derived.hit,
                p.derived.ranged_hit,
            )
        };
        let bare = read(m);
        wear(m, g, MIXED_VEST, CHEST);
        let on = read(m);
        resync(m, g);
        (bare, on, read(m))
    })
    .await;
    let (bare, on, again) = got;
    assert_eq!(on.0, bare.0 + 20);
    assert_eq!(on.1, bare.1 + 20);
    assert!(close(on.2, bare.2 + 2.0), "{on:?}");
    assert_eq!((on.3, on.4), (1.0, 1.0), "melee and ranged hit");
    assert_eq!(again, on);
}

/// Thistle Tea's skill and parry passives: gear's Defense and parry stand
/// beside an ordinary crit aura (parry from both, dodge from the
/// Defense), and go with the gear, leaving the aura.
#[tokio::test]
async fn gears_skill_and_parry_stand_beside_ordinary_auras() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        wear(m, g, SWORD, MAIN);
        let s = m.realm.data.spells.get(CRIT_3).unwrap().clone();
        auras::apply_spell(m, g, g, &s, &[(0, 3)], Context::default()).unwrap();
        derive::unit(m, g);
        let read = |m: &Map| {
            let f = &m.players[&g].fields;
            (
                defense_bonus(m, g),
                f.get_f32(PLAYER_PARRY_PERCENTAGE),
                f.get_f32(PLAYER_DODGE_PERCENTAGE),
                f.get_f32(PLAYER_CRIT_PERCENTAGE),
            )
        };
        let bare = read(m);
        wear(m, g, DEFENDER, CHEST);
        let on = read(m);
        resync(m, g);
        let again = read(m);
        off(m, g, CHEST);
        let spells: Vec<(u32, bool)> = m.players[&g]
            .auras
            .holders
            .iter()
            .map(|h| (h.spell, h.equipped))
            .collect();
        (bare, on, again, read(m), spells)
    })
    .await;
    let (bare, on, again, removed, spells) = got;
    assert_eq!(on.0, 5, "the Defense bonus");
    // Two percent of parry, and a fifth of a percent each of parry and
    // dodge for five Defense.
    assert!(close(on.1, bare.1 + 2.2), "{bare:?} {on:?}");
    assert!(close(on.2, bare.2 + 0.2), "{bare:?} {on:?}");
    assert!(close(on.3, bare.3), "the crit aura untouched");
    assert_eq!(again, on);
    assert_eq!(removed, bare);
    assert_eq!(spells, vec![(CRIT_3, false)]);
}

/// Thistle Tea's equip auras through death: a gear passive can't be
/// cancelled, stays on through death, and gives the same after the
/// resurrection.
#[tokio::test]
async fn gear_passives_outlast_death_and_cant_be_cancelled() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let ring = wear(m, g, RING, FINGER1);
        let on = crit(m, g);
        auras::cancel(m, g, CRIT_2);
        let cancelled = gear_auras(m, g);
        crate::game::map::death::kill(m, g);
        let dead = gear_auras(m, g);
        crate::game::map::death::resurrect(m, g, 1.0);
        derive::unit(m, g);
        (ring, on, cancelled, dead, gear_auras(m, g), crit(m, g))
    })
    .await;
    let (ring, on, cancelled, dead, revived, revived_crit) = got;
    let want = vec![(Some(ring), CRIT_2)];
    assert_eq!(cancelled, want);
    assert_eq!(dead, want);
    assert_eq!(revived, want);
    assert_eq!(revived_crit, on);
}

/// Thistle Tea's mana regeneration from gear: a vest's mana every five
/// seconds comes while casting, inside the five-second rule, and stops as
/// it comes off.
#[tokio::test]
async fn gears_mana_regeneration_runs_while_casting_and_stops_unworn() {
    let (realm, _a, g) = setup_as(Class::Mage).await;
    let got = on_map(&realm, move |m| {
        let tick = |m: &mut Map| {
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_u32(UNIT_FIELD_STAT0 + 4, 0);
            p.fields.set_u32(UNIT_FIELD_POWER1, 0);
            p.mana_spent_at = Some(m.now);
            p.regen_at = m.now;
            crate::game::map::stats::regen(m);
            m.players[&g].fields.get_u32(UNIT_FIELD_POWER1)
        };
        wear(m, g, MANA_VEST, CHEST);
        let worn = tick(m);
        off(m, g, CHEST);
        (worn, tick(m))
    })
    .await;
    // Fifteen in five seconds: six a two-second tick.
    assert_eq!(got, (6, 0));
}

/// Thistle Tea's spell hit and crit passives: gear's reach the spell's hit
/// and crit chances.
#[tokio::test]
async fn gears_spell_hit_and_crit_reach_the_spells() {
    let (realm, _a, g) = setup_as(Class::Mage).await;
    let got = on_map(&realm, move |m| {
        let read = |m: &Map| {
            let d = &m.players[&g].derived;
            (d.spell_hit, d.spell_crit[2])
        };
        let bare = read(m);
        wear(m, g, CASTER_VEST, CHEST);
        (bare, read(m))
    })
    .await;
    let (bare, on) = got;
    assert_eq!(on.0, bare.0 + 2.0);
    assert!(close(on.1, bare.1 + 3.0), "{bare:?} {on:?}");
}

/// A melee hit by `g`, for its procs.
fn hits(m: &mut Map, g: Guid) {
    crate::game::map::procs::trigger(
        m,
        crate::game::map::procs::Event {
            actor: g,
            target: crate::game::guid::unit(1, 1),
            actor_flags: crate::game::map::procs::PROC_DONE_MELEE,
            ex: crate::game::map::procs::EX_NORMAL_HIT,
            school: 1,
            ..Default::default()
        },
    );
}

/// When `g`'s proc of `spell` may go off again, if it has one.
fn proc_ready(m: &Map, g: Guid, spell: u32) -> Option<Option<Duration>> {
    let now = m.now;
    m.players[&g]
        .auras
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| h.proc_ready_at.map(|t| t.saturating_duration_since(now)))
}

/// Thistle Tea's gear procs: a vest's proc goes off on a hit and waits its
/// second; a resync, or the vest moved to another slot, keeps the wait;
/// after it, it goes off again; the vest off, nothing.
#[tokio::test]
async fn a_gear_procs_cooldown_outlasts_a_resync_and_a_move() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        wear(m, g, PROC_VEST, CHEST);
        let fresh = proc_ready(m, g, PROC);
        hits(m, g);
        let fired = proc_ready(m, g, PROC);
        resync(m, g);
        let resynced = proc_ready(m, g, PROC);
        shift(m, g, CHEST, HEAD);
        let moved = proc_ready(m, g, PROC);
        m.now += Duration::from_millis(1000);
        hits(m, g);
        let again = proc_ready(m, g, PROC);
        off(m, g, HEAD);
        (fresh, fired, resynced, moved, again, proc_ready(m, g, PROC))
    })
    .await;
    let (fresh, fired, resynced, moved, again, gone) = got;
    let second = Some(Some(Duration::from_secs(1)));
    assert_eq!(fresh, Some(None));
    assert_eq!(fired, second);
    assert_eq!(resynced, second);
    assert_eq!(moved, second);
    assert_eq!(again, second, "it went off again after its second");
    assert_eq!(gone, None);
}

/// Thistle Tea's `EquipmentSetsTest`: two sets with the same spell give it
/// each; a piece not of a set, or of a set unknown, gives nothing.
#[tokio::test]
async fn two_sets_with_the_same_spell_give_it_each() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let bare = max_health(m, g);
        wear(m, g, SET_HELM, HEAD);
        let one = gear_auras(m, g).len();
        wear(m, g, SET_VEST, CHEST);
        wear(m, g, OTHER_HELM, 6);
        let worn = (gear_auras(m, g).len(), max_health(m, g));
        wear(m, g, OTHER_VEST, 7);
        (bare, one, worn, gear_auras(m, g), max_health(m, g))
    })
    .await;
    let (bare, one, worn, auras, all) = got;
    assert_eq!(one, 0, "one piece of two");
    assert_eq!(worn, (1, bare + 5));
    assert_eq!(auras.len(), 2);
    assert!(auras.iter().all(|(_, s)| *s == HEALTH_5));
    assert_ne!(auras[0].0, auras[1].0, "held by a piece of each");
    assert_eq!(all, bare + 10);
}

/// Thistle Tea's set counts: a broken piece worn counts (mangos's sets are
/// "still active for broken items"), one carried or in the bank doesn't.
#[tokio::test]
async fn a_broken_piece_counts_for_its_set_and_a_carried_one_doesnt() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let helm = wear(m, g, SET_HELM, HEAD);
        put(m, g, SET_VEST, (BAG_PLAYER, PACK));
        put(m, g, SET_VEST, (BAG_PLAYER, BANK));
        let carried = gear_auras(m, g).len();
        let vest = wear(m, g, SET_VEST, CHEST);
        let p = m.players.get_mut(&g).unwrap();
        for item in [helm, vest] {
            let i = p.item_by_guid(item).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
        }
        resync(m, g);
        (carried, gear_auras(m, g))
    })
    .await;
    assert_eq!(got.0, 0);
    assert_eq!(got.1.len(), 1);
    assert_eq!(got.1[0].1, HEALTH_5);
}

/// Thistle Tea's enchantment skill bonus: an enchantment's Defense shows as
/// the skill's bonus, the skill's own value left; it goes as the item
/// comes off.
#[tokio::test]
async fn an_enchantments_skill_bonus_shows_and_goes() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let value = crate::game::map::skills::value(&m.players[&g], DEFENSE_SKILL);
        let ring = wear(m, g, PLAIN_RING, FINGER1);
        assert!(enchant::enchant_item(m, g, ring, ENCHANT_SKILL, None));
        resync(m, g);
        let on = (
            defense_bonus(m, g),
            crate::game::map::skills::value(&m.players[&g], DEFENSE_SKILL),
        );
        off(m, g, FINGER1);
        (value, on, defense_bonus(m, g))
    })
    .await;
    let (value, on, off) = got;
    assert_eq!(on, (1, value));
    assert_eq!(off, 0);
}

fn enchanted(m: &mut Map, g: Guid, entry: u32, slot: u8, id: u32) -> Guid {
    let item = wear(m, g, entry, slot);
    assert!(enchant::enchant_item(m, g, item, id, None));
    resync(m, g);
    item
}

/// Thistle Tea's identical enchantments: two rings of the same
/// enchantment give its health twice, the current health left where it
/// is, a resync changing nothing; each ring takes its own off.
#[tokio::test]
async fn the_same_enchantment_on_two_items_counts_twice_and_goes_with_each() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let bare = (max_health(m, g), health(m, g));
        enchanted(m, g, PLAIN_RING, FINGER1, ENCHANT_HEALTH);
        enchanted(m, g, PLAIN_RING, FINGER2, ENCHANT_HEALTH);
        let both = (max_health(m, g), health(m, g), gear_auras(m, g).len());
        resync(m, g);
        let again = (max_health(m, g), health(m, g), gear_auras(m, g).len());
        off(m, g, FINGER1);
        let one = (max_health(m, g), gear_auras(m, g).len());
        off(m, g, FINGER2);
        (
            bare,
            both,
            again,
            one,
            (max_health(m, g), gear_auras(m, g).len()),
        )
    })
    .await;
    let (bare, both, again, one, none) = got;
    assert_eq!(both, (bare.0 + 10, bare.1, 2));
    assert_eq!(again, both);
    assert_eq!(one, (bare.0 + 5, 1));
    assert_eq!(none, (bare.0, 0));
}

/// Thistle Tea's enchantments through death: their health stays on, and
/// a resurrection at full health brings back all of it.
#[tokio::test]
async fn an_enchantments_passive_outlasts_death() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let bare = max_health(m, g);
        enchanted(m, g, PLAIN_RING, FINGER1, ENCHANT_HEALTH);
        enchanted(m, g, PLAIN_RING, FINGER2, ENCHANT_HEALTH);
        crate::game::map::death::kill(m, g);
        let dead = (health(m, g), gear_auras(m, g).len());
        crate::game::map::death::resurrect(m, g, 1.0);
        derive::unit(m, g);
        (bare, dead, health(m, g), max_health(m, g))
    })
    .await;
    let (bare, dead, health, max) = got;
    assert_eq!(dead, (0, 2));
    assert_eq!((health, max), (bare + 10, bare + 10));
}

/// Thistle Tea's replaced enchantment: a new enchantment in the slot puts
/// its spell in place of the old one's.
#[tokio::test]
async fn a_replaced_enchantment_replaces_its_spell() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let bare = max_health(m, g);
        let ring = enchanted(m, g, PLAIN_RING, FINGER1, ENCHANT_HEALTH);
        assert!(enchant::enchant_item(m, g, ring, ENCHANT_HEALTH_20, None));
        resync(m, g);
        (bare, max_health(m, g), gear_auras(m, g), ring)
    })
    .await;
    let (bare, max, auras, ring) = got;
    assert_eq!(max, bare + 20);
    assert_eq!(auras, vec![(Some(ring), HEALTH_20)]);
}

/// Thistle Tea's enchantment armor and weapon damage: the sword's damage
/// takes the enchantment's five, the armor its eight, the other hands
/// nothing; derived again, nothing more.
#[tokio::test]
async fn an_enchantments_armor_and_weapon_damage_count_once() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let read = |m: &Map| {
            let f = &m.players[&g].fields;
            (
                f.get_f32(UNIT_FIELD_MINDAMAGE),
                f.get_f32(UNIT_FIELD_MAXDAMAGE),
                f.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE),
                f.get_f32(UNIT_FIELD_MINRANGEDDAMAGE),
                f.get_u32(UNIT_FIELD_RESISTANCES),
            )
        };
        wear(m, g, SWORD, MAIN);
        let bare = read(m);
        let sword = m.players[&g].items[m.players[&g].item_at(BAG_PLAYER, MAIN).unwrap()].guid;
        assert!(enchant::enchant_item(
            m,
            g,
            sword,
            ENCHANT_ARMOR_DAMAGE,
            None
        ));
        resync(m, g);
        let on = read(m);
        resync(m, g);
        (bare, on, read(m))
    })
    .await;
    let (bare, on, again) = got;
    assert!(
        close(on.0, bare.0 + 5.0) && close(on.1, bare.1 + 5.0),
        "{bare:?} {on:?}"
    );
    assert_eq!((on.2, on.3), (bare.2, bare.3));
    assert_eq!(on.4, bare.4 + 8);
    assert_eq!(again, on);
}

/// Thistle Tea's outdoor set bonuses: a set's outdoor-only speed goes
/// indoors, comes back outside (as mangos's `UpdateArea` casts a set's
/// outdoor passives again), and a resync there changes nothing.
#[tokio::test]
async fn an_outdoor_set_bonus_goes_indoors_and_comes_back_outside() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let has = |m: &Map| m.players[&g].auras.has_spell(SPEED);
        m.spells.outdoors_override.insert(g, true);
        wear(m, g, OUTDOOR_HELM, HEAD);
        let outside = has(m);
        m.spells.outdoors_override.insert(g, false);
        let now = m.now;
        crate::game::map::spells::environment::reconcile(m, g, now);
        resync(m, g);
        let inside = has(m);
        m.spells.outdoors_override.insert(g, true);
        crate::game::map::spells::environment::reconcile(m, g, now);
        inventory::sync_gear_all(m);
        let back = gear_auras(m, g);
        resync(m, g);
        (outside, inside, back, gear_auras(m, g))
    })
    .await;
    let (outside, inside, back, again) = got;
    assert!(outside);
    assert!(!inside, "gone indoors, and not put back by a resync");
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].1, SPEED);
    assert_eq!(again, back);
}

/// Thistle Tea's sets beside enchantments and auras: the same health spell
/// cast by another, from two enchantments and from a set stands four
/// times; the set's goes as the set breaks up, the rest staying.
#[tokio::test]
async fn a_set_bonus_stands_apart_from_enchantments_and_auras_of_its_spell() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        m.players.get_mut(&g).unwrap().items.clear();
        resync(m, g);
        let bare = max_health(m, g);
        let s = d.spells.get(HEALTH_5).unwrap().clone();
        let mut cast = s.clone();
        cast.attributes[0] = 0;
        auras::apply_spell(m, gb, g, &cast, &[(0, 5)], Context::default()).unwrap();
        enchanted(m, g, PLAIN_RING, FINGER1, ENCHANT_HEALTH);
        enchanted(m, g, PLAIN_RING, FINGER2, ENCHANT_HEALTH);
        wear(m, g, SET_HELM, HEAD);
        wear(m, g, SET_VEST, CHEST);
        let all = (max_health(m, g), m.players[&g].auras.holders.len());
        resync(m, g);
        let again = (max_health(m, g), m.players[&g].auras.holders.len());
        off(m, g, CHEST);
        let broken_up = (max_health(m, g), m.players[&g].auras.holders.len());
        (bare, all, again, broken_up)
    })
    .await;
    let (bare, all, again, broken_up) = got;
    assert_eq!(all, (bare + 20, 4));
    assert_eq!(again, all);
    assert_eq!(broken_up, (bare + 15, 3));
}

/// Thistle Tea's set auras through death: a set's passive can't be
/// cancelled, stays on through death, and comes off with the set.
#[tokio::test]
async fn a_set_bonus_outlasts_death_and_cant_be_cancelled() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        let bare = max_health(m, g);
        wear(m, g, SET_HELM, HEAD);
        wear(m, g, SET_VEST, CHEST);
        auras::cancel(m, g, HEALTH_5);
        let cancelled = gear_auras(m, g).len();
        crate::game::map::death::kill(m, g);
        let dead = gear_auras(m, g).len();
        crate::game::map::death::resurrect(m, g, 1.0);
        derive::unit(m, g);
        let revived = (health(m, g), max_health(m, g));
        off(m, g, HEAD);
        (bare, cancelled, dead, revived, max_health(m, g))
    })
    .await;
    let (bare, cancelled, dead, revived, removed) = got;
    assert_eq!((cancelled, dead), (1, 1));
    assert_eq!(revived, (bare + 5, bare + 5));
    assert_eq!(removed, bare);
}

/// Thistle Tea's set procs: a set's proc goes off on a hit, keeps its wait
/// through a resync and an unrelated change of gear, and goes with the
/// set.
#[tokio::test]
async fn a_set_proc_fires_keeps_its_cooldown_and_goes_with_the_set() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        wear(m, g, PROC_HELM, HEAD);
        hits(m, g);
        let fired = proc_ready(m, g, PROC);
        resync(m, g);
        wear(m, g, PLAIN_RING, FINGER1);
        let kept = proc_ready(m, g, PROC);
        off(m, g, HEAD);
        m.now += Duration::from_secs(2);
        hits(m, g);
        (fired, kept, proc_ready(m, g, PROC))
    })
    .await;
    let second = Some(Some(Duration::from_secs(1)));
    assert_eq!(got, (second, second, None));
}

/// Thistle Tea's set spell modifiers: a set's flat modifier is told to the
/// client as it comes, and its end (nothing) as it goes.
#[tokio::test]
async fn a_set_spell_modifier_is_told_as_it_comes_and_goes() {
    let (realm, mut a, g) = setup().await;
    let told = |amount: i32| [&[0u8, 0][..], &amount.to_le_bytes()].concat();
    on_map(&realm, move |m| {
        wear(m, g, MOD_HELM, HEAD);
    })
    .await;
    let on = a
        .wait_frame("SMSG_SET_FLAT_SPELL_MODIFIER", 0x266, |_| true)
        .await;
    assert_eq!(on, told(20));
    on_map(&realm, move |m| off(m, g, HEAD)).await;
    let gone = a
        .wait_frame("SMSG_SET_FLAT_SPELL_MODIFIER", 0x266, |_| true)
        .await;
    assert_eq!(gone, told(0));
}

/// Shift `g` into cat form, or out of it.
fn cat(m: &mut Map, g: Guid, on: bool) {
    if on {
        let s = m.realm.data.spells.get(CAT_FORM).unwrap().clone();
        auras::apply_spell(m, g, g, &s, &[(0, 1)], Context::default()).unwrap();
    } else {
        auras::remove_spell(m, g, CAT_FORM, None);
    }
    derive::unit(m, g);
}

/// Thistle Tea's form restrictions on sets: a set's bonus held to cat form
/// holds only in it, its other bonus all along.
#[tokio::test]
async fn a_set_bonus_held_to_a_form_holds_only_in_it() {
    let (realm, _a, g) = setup_as(Class::Druid).await;
    let got = on_map(&realm, move |m| {
        let bare = max_health(m, g);
        wear(m, g, FERAL_HELM, HEAD);
        let humanoid = max_health(m, g);
        cat(m, g, true);
        let shifted = max_health(m, g);
        cat(m, g, false);
        (bare, humanoid, shifted, max_health(m, g))
    })
    .await;
    let (bare, humanoid, shifted, restored) = got;
    assert_eq!(humanoid, bare + 10);
    assert_eq!(shifted, bare + 15);
    assert_eq!(restored, bare + 10);
}

/// Thistle Tea's form restrictions on items: a vest's attack power and
/// crit held to cat form give nothing out of it, and come and go with the
/// form, its stats and its auras.
#[tokio::test]
async fn an_items_spell_held_to_a_form_holds_only_in_it() {
    let (realm, _a, g) = setup_as(Class::Druid).await;
    let got = on_map(&realm, move |m| {
        let read = |m: &Map| {
            (
                m.players[&g].fields.get_u32(UNIT_FIELD_ATTACK_POWER_MODS),
                m.players[&g].auras.has_spell(FERAL_AP),
            )
        };
        wear(m, g, FERAL_VEST, CHEST);
        let humanoid = read(m);
        cat(m, g, true);
        let shifted = read(m);
        cat(m, g, false);
        (humanoid, shifted, read(m))
    })
    .await;
    assert_eq!(got, ((0, false), (100, true), (0, false)));
}

/// Wait until `a`'s earlier packets have been handled.
async fn handled(a: &mut Client) {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// How long `g` waits for `spell`.
fn waits(m: &Map, g: Guid, spell: u32) -> Option<Duration> {
    let now = m.now;
    m.players[&g]
        .caster
        .cooldowns
        .get(&spell)
        .filter(|t| **t > now)
        .map(|t| t.duration_since(now))
}

fn fight(m: &mut Map, g: Guid, on: bool) {
    m.players.get_mut(&g).unwrap().fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        on,
    );
}

/// Swap what is at `from` with `to` as the client asks.
async fn swap(a: &mut Client, from: u8, to: u8) {
    a.send(CMSG_SWAP_INV_ITEM {
        source_slot: ItemSlot::try_from(from).unwrap(),
        destination_slot: ItemSlot::try_from(to).unwrap(),
    })
    .await;
    handled(a).await;
}

/// The refusal `a` was sent since it last looked, by its name.
async fn refusal(a: &mut Client) -> Option<String> {
    handled(a).await;
    let r = a
        .gets(Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
                Some(format!("{:?}", f.result))
            }
            _ => None,
        })
        .await;
    a.clear();
    r.map(|r| r.split([' ', '{']).next().unwrap().to_owned())
}

/// Thistle Tea's equipment transitions: a trinket put on waits thirty
/// seconds; carried about, nothing; an exempt one (its template's 0x80)
/// waits nothing; moved to the other trinket slot it waits again once its
/// wait is over.
#[tokio::test]
async fn a_trinket_put_on_waits_thirty_seconds_and_moving_it_again() {
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        put(m, g, TRINKET, (BAG_PLAYER, PACK));
        put(m, g, EXEMPT_TRINKET, (BAG_PLAYER, PACK + 1));
    })
    .await;
    swap(&mut a, PACK, PACK + 2).await;
    let carried = on_map(&realm, move |m| waits(m, g, TRINKET_USE)).await;
    swap(&mut a, PACK + 1, TRINKET1).await;
    let exempt = on_map(&realm, move |m| waits(m, g, TRINKET_USE)).await;
    swap(&mut a, TRINKET1, PACK + 1).await;
    swap(&mut a, PACK + 2, TRINKET1).await;
    let worn = on_map(&realm, move |m| {
        let w = waits(m, g, TRINKET_USE);
        m.players
            .get_mut(&g)
            .unwrap()
            .caster
            .cooldowns
            .remove(&TRINKET_USE);
        w
    })
    .await;
    swap(&mut a, TRINKET1, TRINKET2).await;
    let moved = on_map(&realm, move |m| waits(m, g, TRINKET_USE)).await;
    assert_eq!(carried, None);
    assert_eq!(exempt, None);
    assert!(thirty(worn), "{worn:?}");
    assert!(thirty(moved), "{moved:?}");
}

/// About thirty seconds: the map's clock runs on between queries.
fn thirty(wait: Option<Duration>) -> bool {
    wait.is_some_and(|w| {
        w <= crate::game::map::items::EQUIP_COOLDOWN && w > Duration::from_secs(29)
    })
}

/// Thistle Tea's kept cooldowns: a trinket's use already waiting, five
/// seconds or two minutes, waits as long, the trinket put on.
#[tokio::test]
async fn a_trinket_put_on_keeps_a_wait_already_running() {
    let (realm, mut a, g) = setup().await;
    let mut got = Vec::new();
    for wait in [Duration::from_secs(5), Duration::from_secs(120)] {
        on_map(&realm, move |m| {
            m.players.get_mut(&g).unwrap().items.clear();
            put(m, g, TRINKET, (BAG_PLAYER, PACK));
            let now = m.now;
            m.players
                .get_mut(&g)
                .unwrap()
                .caster
                .cooldowns
                .insert(TRINKET_USE, now + wait);
        })
        .await;
        swap(&mut a, PACK, TRINKET1).await;
        let left = on_map(&realm, move |m| {
            let now = m.now;
            m.players[&g].caster.cooldowns[&TRINKET_USE].saturating_duration_since(now)
        })
        .await;
        got.push((wait, left));
    }
    for (wait, left) in got {
        assert!(
            left <= wait && left + Duration::from_secs(1) > wait,
            "{wait:?} {left:?}"
        );
    }
}

/// Thistle Tea's swap of two trinkets: each changes its slot, so each
/// waits its thirty seconds, as mangos's `SwapItem` equips both
/// (`ApplyEquipCooldown` on each).
#[tokio::test]
async fn swapping_two_trinkets_starts_both_their_waits() {
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        put(m, g, TRINKET, (BAG_PLAYER, TRINKET1));
        put(m, g, TRINKET_TOO, (BAG_PLAYER, TRINKET2));
        m.players.get_mut(&g).unwrap().caster.cooldowns.clear();
    })
    .await;
    swap(&mut a, TRINKET1, TRINKET2).await;
    let got = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let at = |s| p.item_at(BAG_PLAYER, s).map(|i| p.items[i].entry);
        (
            at(TRINKET1),
            at(TRINKET2),
            waits(m, g, TRINKET_USE),
            waits(m, g, ON_USE_3),
        )
    })
    .await;
    assert_eq!((got.0, got.1), (Some(TRINKET_TOO), Some(TRINKET)));
    assert!(thirty(got.2) && thirty(got.3), "{got:?}");
}

/// Thistle Tea's combat timer on swaps: in a fight a weapon or a relic put
/// on locks weapons for the weapon switch's second and a half; a shield or
/// a held thing doesn't, nor a weapon out of a fight.
#[tokio::test]
async fn in_a_fight_weapons_and_relics_lock_the_weapons_and_shields_dont() {
    let (realm, mut a, g) = setup().await;
    let mut got = Vec::new();
    for (fighting, entry, slot) in [
        (true, RELIC, RANGED),
        (true, SHIELD, OFF),
        (true, HELD, OFF),
        (false, SWORD, MAIN),
        (true, SWORD, MAIN),
    ] {
        on_map(&realm, move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.items.clear();
            p.caster.weapon_change_until = None;
            p.caster.gcd_until = None;
            fight(m, g, fighting);
            put(m, g, entry, (BAG_PLAYER, PACK));
        })
        .await;
        swap(&mut a, PACK, slot).await;
        let locked = on_map(&realm, move |m| {
            let now = m.now;
            let p = &m.players[&g];
            (
                p.item_at(BAG_PLAYER, slot).is_some(),
                p.caster
                    .weapon_change_until
                    .filter(|t| *t > now)
                    .map(|t| t.duration_since(now)),
            )
        })
        .await;
        got.push((entry, locked));
    }
    let lock = |d: Option<Duration>| d.is_some_and(|d| d <= Duration::from_millis(1500));
    assert!(got[0].1.0 && lock(got[0].1.1), "the relic: {got:?}");
    assert_eq!(got[1].1, (true, None), "the shield");
    assert_eq!(got[2].1, (true, None), "the held thing");
    assert_eq!(got[3].1, (true, None), "a weapon out of a fight");
    assert!(
        got[4].1.0 && lock(got[4].1.1),
        "a weapon in a fight: {got:?}"
    );
}

/// Thistle Tea's swap and a running global cooldown: a weapon put on in a
/// fight leaves a global cooldown already running as it is, and a weapon
/// swap while the lock holds doesn't lengthen it.
#[tokio::test]
async fn a_weapon_swap_keeps_a_running_global_cooldown_and_lock() {
    let (realm, _a, g) = setup().await;
    let got = on_map(&realm, move |m| {
        fight(m, g, true);
        let now = m.now;
        let gcd = now + Duration::from_millis(500);
        m.players.get_mut(&g).unwrap().caster.gcd_until = Some(gcd);
        crate::game::map::spells::class_scripts::weapon_equipped(m, g, SWORD);
        let first = (
            m.players[&g].caster.gcd_until,
            m.players[&g].caster.weapon_change_until,
        );
        m.now += Duration::from_millis(500);
        crate::game::map::spells::class_scripts::weapon_equipped(m, g, AXE);
        let second = m.players[&g].caster.weapon_change_until;
        (now, gcd, first, second)
    })
    .await;
    let (now, gcd, (gcd_after, lock), lock_after) = got;
    assert_eq!(gcd_after, Some(gcd), "the running global cooldown kept");
    assert_eq!(lock, Some(now + Duration::from_millis(1500)));
    assert_eq!(lock_after, lock, "not lengthened");
}

/// Thistle Tea's trinkets in a fight: one isn't put on or taken off, but
/// bags' contents still move.
#[tokio::test]
async fn in_a_fight_a_trinket_stays_as_it_is_but_the_bags_move() {
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        put(m, g, TRINKET, (BAG_PLAYER, PACK));
        put(m, g, TRINKET, (BAG_PLAYER, TRINKET2));
        fight(m, g, true);
    })
    .await;
    swap(&mut a, PACK, TRINKET1).await;
    let on = refusal(&mut a).await;
    swap(&mut a, TRINKET2, PACK + 1).await;
    let off = refusal(&mut a).await;
    swap(&mut a, PACK, PACK + 2).await;
    let moved = refusal(&mut a).await;
    let at = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (
            p.item_at(BAG_PLAYER, TRINKET1).is_some(),
            p.item_at(BAG_PLAYER, TRINKET2).is_some(),
            p.item_at(BAG_PLAYER, PACK + 2).is_some(),
        )
    })
    .await;
    assert_eq!(on.as_deref(), Some("NotInCombat"));
    assert_eq!(off.as_deref(), Some("NotInCombat"));
    assert_eq!(moved, None);
    assert_eq!(at, (false, true, true));
}

/// Thistle Tea's weapon lock: in a fight a second weapon waits for the
/// first's lock to be over; a shield doesn't wait.
#[tokio::test]
async fn in_a_fight_another_weapon_waits_for_the_lock_and_a_shield_doesnt() {
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        put(m, g, SWORD, (BAG_PLAYER, MAIN));
        put(m, g, AXE, (BAG_PLAYER, PACK));
        put(m, g, SHIELD, (BAG_PLAYER, PACK + 1));
        fight(m, g, true);
        crate::game::map::spells::class_scripts::weapon_equipped(m, g, SWORD);
    })
    .await;
    swap(&mut a, PACK, MAIN).await;
    let weapon = refusal(&mut a).await;
    swap(&mut a, PACK + 1, OFF).await;
    let shield = refusal(&mut a).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().caster.weapon_change_until = None;
    })
    .await;
    swap(&mut a, PACK, MAIN).await;
    let later = refusal(&mut a).await;
    let worn = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let at = |s| p.item_at(BAG_PLAYER, s).map(|i| p.items[i].entry);
        (at(MAIN), at(OFF))
    })
    .await;
    assert_eq!(weapon.as_deref(), Some("CantDoRightNow"));
    assert_eq!(shield, None);
    assert_eq!(later, None);
    assert_eq!(worn, (Some(AXE), Some(SHIELD)));
}

/// Proficiency spells: Staves, Wands, Cloth, Dual Wield, a second Parry
/// (as 3124 and 18848 are beside 3127), and a specialization.
const STAVES: u32 = 227;
const WANDS: u32 = 5009;
const CLOTH: u32 = 9078;
const DUAL_WIELD: u32 = 674;
const OTHER_PARRY: u32 = 3124;
const SPECIALIZATION: u32 = 9788;
const FISHING: u32 = 356;
const STAFF_BITS: u32 = 1 << 10;
const WAND_BITS: u32 = 1 << 19;
const CLOTH_BITS: u32 = 1 << 1;

fn proficiency_spell(id: u32, class: i32, mask: u32) -> SpellTemplate {
    let mut s = passive(id, &[]);
    s.effects[0] = SpellEffect {
        effect: crate::game::map::spells::EFFECT_PROFICIENCY,
        target_a: 1,
        ..Default::default()
    };
    s.equipped_item_class = class;
    s.equipped_item_subclass_mask = mask as i32;
    s
}

/// The gear's world with the proficiency spells, so a player's come from
/// what they know.
fn proficiency_data() -> GameData {
    let mut d = data();
    for s in [
        proficiency_spell(STAVES, 2, STAFF_BITS),
        proficiency_spell(WANDS, 2, WAND_BITS),
        proficiency_spell(CLOTH, 4, CLOTH_BITS),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let mut dual = passive(DUAL_WIELD, &[]);
    dual.effects[0] = SpellEffect {
        effect: 40,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(DUAL_WIELD, dual);
    let mut parry = passive(OTHER_PARRY, &[]);
    parry.effects[0] = SpellEffect {
        effect: 22,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(OTHER_PARRY, parry);
    d.spells
        .spells
        .insert(SPECIALIZATION, passive(SPECIALIZATION, &[]));
    d
}

/// Thistle Tea's `from_spellbook/1`: each proficiency spell's subclasses
/// of its item class add up, weapons apart from armor; Dual Wield comes
/// of its effect, not without it.
#[test]
fn proficiency_spells_add_up_their_subclasses_and_dual_wield_its_effect() {
    let d = proficiency_data();
    let prof = Proficiency::from_spells(&d, &[STAVES, WANDS, CLOTH]);
    assert_eq!(prof.weapons, STAFF_BITS | WAND_BITS);
    assert_eq!(prof.armor, CLOTH_BITS);
    assert!(!prof.dual_wield);
    assert!(Proficiency::from_spells(&d, &[DUAL_WIELD]).dual_wield);
    // A staff (10) is covered, a dagger (15) not; a miscellaneous weapon
    // (14) and armor (0) want no proficiency.
    let staff = item(1, 2, 10, 17);
    let dagger = item(2, 2, 15, 13);
    assert!(prof.covers(&staff));
    assert!(!prof.covers(&dagger));
    let none = Proficiency::from_spells(&d, &[]);
    assert!(none.covers(&item(3, 2, 14, 13)));
    assert!(none.covers(&item(4, 4, 0, 11)));
    assert!(!none.covers(&staff));
}

/// Thistle Tea's skill-derived proficiency: Fishing known lets a fishing
/// pole (weapon subclass 20) be held, beside the spells' weapons; without
/// it, a pole isn't.
#[tokio::test]
async fn fishing_known_lets_a_pole_be_held_beside_the_spells_weapons() {
    let realm = harness::realm_with(proficiency_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let pole = item(5, 2, 20, 17);
        let p = m.players.get_mut(&g).unwrap();
        p.spells = vec![STAVES, CLOTH];
        let without = Proficiency::of(p, &d);
        let refused = inventory::proficient(p, &without, &pole);
        crate::game::map::skills::learn_skill(p, FISHING, 1);
        let with = Proficiency::of(p, &d);
        (
            without.weapons,
            refused,
            with.weapons,
            inventory::proficient(p, &with, &pole),
        )
    })
    .await;
    assert_eq!(got.0, STAFF_BITS);
    assert_eq!(got.1, Err(inventory::Fail::NoProficiency));
    assert_eq!(got.2, STAFF_BITS | 1 << 20);
    assert_eq!(got.3, Ok(()));
}

/// Thistle Tea's required skills and spells: a recipe asking for Tailoring
/// at rank 0 wants Tailoring still; a pole asking for Fishing at 50 wants
/// 50 (short of it, mangos's "not skilled enough" where Thistle Tea says
/// no proficiency); an item asking for a specialization wants it, armor or
/// recipe; a world without proficiency spells lets anything be worn.
#[tokio::test]
async fn an_items_required_skill_rank_and_spell_are_asked_for() {
    let realm = harness::realm_with(proficiency_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        use inventory::Fail;
        let d = m.realm.data.clone();
        let recipe = ItemTemplate {
            required_skill: 164,
            required_skill_rank: 0,
            ..item(6, 9, 0, 0)
        };
        let pole = ItemTemplate {
            required_skill: FISHING,
            required_skill_rank: 50,
            ..item(7, 2, 20, 17)
        };
        let special: Vec<ItemTemplate> = [4, 9]
            .into_iter()
            .map(|class| ItemTemplate {
                required_spell: SPECIALIZATION,
                ..item(8, class, 0, 0)
            })
            .collect();
        let p = m.players.get_mut(&g).unwrap();
        p.spells = vec![CLOTH];
        let mut out: Vec<(&str, Result<(), Fail>)> = Vec::new();
        let check = |p: &crate::game::map::player::Player, t: &ItemTemplate| {
            inventory::proficient(p, &Proficiency::of(p, &d), t)
        };
        out.push(("recipe unskilled", check(p, &recipe)));
        crate::game::map::skills::learn_skill(p, 164, 1);
        crate::game::map::skills::set_value(p, 164, 1);
        out.push(("recipe skilled", check(p, &recipe)));
        crate::game::map::skills::learn_skill(p, FISHING, 1);
        crate::game::map::skills::set_value(p, FISHING, 49);
        out.push(("pole at 49", check(p, &pole)));
        crate::game::map::skills::set_value(p, FISHING, 50);
        out.push(("pole at 50", check(p, &pole)));
        for t in &special {
            out.push(("unspecialized", check(p, t)));
        }
        p.spells.push(SPECIALIZATION);
        for t in &special {
            out.push(("specialized", check(p, t)));
        }
        p.spells.retain(|s| *s != SPECIALIZATION);
        for t in special.iter().chain([&recipe, &pole]) {
            out.push((
                "anything goes",
                inventory::proficient(p, &Proficiency::ALL, t),
            ));
        }
        out
    })
    .await;
    use inventory::Fail;
    assert_eq!(
        got,
        vec![
            ("recipe unskilled", Err(Fail::NoProficiency)),
            ("recipe skilled", Ok(())),
            ("pole at 49", Err(Fail::Skill)),
            ("pole at 50", Ok(())),
            ("unspecialized", Err(Fail::NoProficiency)),
            ("unspecialized", Err(Fail::NoProficiency)),
            ("specialized", Ok(())),
            ("specialized", Ok(())),
            ("anything goes", Ok(())),
            ("anything goes", Ok(())),
            ("anything goes", Ok(())),
            ("anything goes", Ok(())),
        ]
    );
}

/// Thistle Tea's defensive capabilities: parrying comes of any Parry
/// learned (mangos's `EffectParry` for every spell of the effect), blocking
/// of Block; neither without.
#[tokio::test]
async fn parry_and_block_come_of_what_is_learned() {
    let realm = harness::realm_with(proficiency_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().items.clear();
        wear(m, g, SWORD, MAIN);
        wear(m, g, SHIELD, OFF);
        let read = |m: &mut Map, spells: &[u32]| {
            let p = m.players.get_mut(&g).unwrap();
            p.spells = spells.to_vec();
            derive::unit(m, g);
            let f = &m.players[&g].fields;
            (
                f.get_f32(PLAYER_PARRY_PERCENTAGE) > 0.0,
                f.get_f32(PLAYER_BLOCK_PERCENTAGE) > 0.0,
            )
        };
        (
            read(m, &[CLOTH, derive::SPELL_PARRY, derive::SPELL_BLOCK]),
            read(m, &[CLOTH, OTHER_PARRY]),
            read(m, &[CLOTH]),
        )
    })
    .await;
    assert_eq!(got, ((true, true), (true, false), (false, false)));
}
