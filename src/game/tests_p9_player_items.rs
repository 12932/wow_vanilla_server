//! Round 9's player: items and what is on them (Thistle Tea's
//! `enchantments_test`, `item_transformation_test`, `item_properties_test`,
//! `item_durations_test`, `temporary_enchantments_test`,
//! `weapon_procs_test` and `spell_group_items_test`).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{Enchantment, SkillAbility, SpellEffect, SpellRank, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::inventory::{BANK_START, PACK_START, SLOT_MAIN_HAND};
use crate::game::map::procs::{self, Event};
use crate::game::map::{Map, MapHandle, MapKey, auras, enchant, loot};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{BRUTE, equip, level, spawn};

const BRACER: u32 = 63_900;
const KIT: u32 = 63_901;
const OLD: u32 = 63_902;
const NEW: u32 = 63_903;
const REAGENT: u32 = 63_904;
const SHORT: u32 = 63_905;
const SPENT: u32 = 63_906;
const SWORD: u32 = 63_907;
const JUNK: u32 = 63_908;
const TIMED: u32 = 63_909;
const SCROLL: u32 = 63_910;
const BLADE: u32 = 63_911;

const KIT_ENCHANT: u32 = 96_900;
const TO_NEW: u32 = 96_901;
const TO_NEW_WANTING: u32 = 96_902;
const SLOW_TO_NEW: u32 = 96_903;
const HURT: u32 = 96_904;
const HURT2: u32 = 96_905;
const LOW: u32 = 96_906;
const HIGH: u32 = 96_907;

const HEALTH_5: u32 = 41;
const COATING: u32 = 42;
const POISON: u32 = 43;
/// A suffix of three strength.
const OF_STRENGTH: u32 = 500;
const STRENGTH_3: u32 = 44;
const ENCHANTING: u32 = 333;
const ONE_SECOND: u32 = 7;
const SMSG_CAST_RESULT: u16 = 0x130;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn spells(first: ItemSpell) -> [ItemSpell; 5] {
    let mut s = [ItemSpell::default(); 5];
    s[0] = first;
    s
}

fn on_use(id: u32, charges: i32) -> [ItemSpell; 5] {
    spells(ItemSpell {
        id,
        charges,
        ..Default::default()
    })
}

fn spell(id: u32, effect: SpellEffect) -> SpellTemplate {
    crate::game::tests_p3_entity::spell(id, &[effect])
}

fn damage(id: u32) -> SpellTemplate {
    let mut s = spell(
        id,
        SpellEffect {
            effect: 2,
            base_points: 0,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    s.range_index = 1;
    s.proc_chance = 100;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut bracer = item(BRACER);
    (bracer.class, bracer.subclass, bracer.inventory_type) = (4, 1, 9);
    d.items.insert(BRACER, bracer);
    d.items.insert(
        KIT,
        ItemTemplate {
            stackable: 20,
            spells: on_use(KIT_ENCHANT, -1),
            ..item(KIT)
        },
    );
    let mut kit = spell(
        KIT_ENCHANT,
        SpellEffect {
            effect: 53,
            misc_value: HEALTH_5 as i32,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    kit.equipped_item_class = 4;
    kit.equipped_item_inventory_type_mask = 1 << 9;
    d.spells.spells.insert(KIT_ENCHANT, kit);
    // The kit's spell is an enchanter's recipe too, orange at 1.
    d.spells.skill_abilities.insert(
        KIT_ENCHANT,
        vec![SkillAbility {
            skill: ENCHANTING,
            spell: KIT_ENCHANT,
            trivial_low: 50,
            trivial_high: 100,
            ..Default::default()
        }],
    );
    for (id, effects) in [
        (HEALTH_5, [(5, 5, 5, 1), (0, 0, 0, 0), (0, 0, 0, 0)]),
        (STRENGTH_3, [(5, 3, 3, 4), (0, 0, 0, 0), (0, 0, 0, 0)]),
        // A coating of two spells on a hit, and a poison of one.
        (
            COATING,
            [(1, 100, 100, HURT), (1, 100, 100, HURT2), (0, 0, 0, 0)],
        ),
        (POISON, [(1, 100, 100, HURT2), (0, 0, 0, 0), (0, 0, 0, 0)]),
    ] {
        d.spells.enchantments.insert(
            id,
            Enchantment {
                id,
                effects,
                name: format!("Enchantment {id}"),
            },
        );
    }
    // An item that changes into another, worn and wearing out.
    let worn = |entry| ItemTemplate {
        class: 4,
        subclass: 1,
        inventory_type: 9,
        max_durability: 100,
        ..item(entry)
    };
    let mut old = worn(OLD);
    old.spells = on_use(TO_NEW, 0);
    d.items.insert(OLD, old);
    let mut new = worn(NEW);
    new.max_durability = 50;
    new.stats[0] = (7, 20);
    d.items.insert(NEW, new);
    // Used from the bags, as a worn item can't be.
    d.items.insert(
        SHORT,
        ItemTemplate {
            spells: on_use(TO_NEW_WANTING, 0),
            ..item(SHORT)
        },
    );
    d.items.insert(
        SPENT,
        ItemTemplate {
            spells: on_use(TO_NEW, 1),
            ..item(SPENT)
        },
    );
    d.items.insert(
        REAGENT,
        ItemTemplate {
            stackable: 20,
            ..item(REAGENT)
        },
    );
    d.spells.cast_times.insert(ONE_SECOND, 1000);
    for id in [TO_NEW, TO_NEW_WANTING, SLOW_TO_NEW] {
        let mut s = spell(
            id,
            SpellEffect {
                effect: crate::game::map::extra_effects::EFFECT_SUMMON_CHANGE_ITEM,
                misc_value: NEW as i32,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            },
        );
        if id == TO_NEW_WANTING {
            s.reagents[0] = (REAGENT, 3);
        }
        if id == SLOW_TO_NEW {
            s.casting_time_index = ONE_SECOND;
        }
        d.spells.spells.insert(id, s);
    }
    // A sword with a suffix of strength.
    d.items.insert(
        SWORD,
        ItemTemplate {
            class: 2,
            subclass: 7,
            inventory_type: 13,
            delay: 2000,
            max_durability: 50,
            ..item(SWORD)
        },
    );
    d.crafts
        .random_properties
        .insert(OF_STRENGTH, [STRENGTH_3, 0, 0, 0, 0]);
    d.items.insert(JUNK, item(JUNK));
    d.items.insert(
        TIMED,
        ItemTemplate {
            duration: 600,
            ..item(TIMED)
        },
    );
    // A blade with a chance on hit of its own.
    d.items.insert(
        BLADE,
        ItemTemplate {
            class: 2,
            subclass: 7,
            inventory_type: 13,
            delay: 2000,
            spells: spells(ItemSpell {
                id: HURT,
                trigger: 2,
                ..Default::default()
            }),
            ..item(BLADE)
        },
    );
    for s in [damage(HURT), damage(HURT2)] {
        d.spells.spells.insert(s.id, s);
    }
    // A scroll's buff, and its stronger rank.
    for (id, rank) in [(LOW, 1), (HIGH, 4)] {
        let s = spell(
            id,
            SpellEffect {
                effect: 6,
                aura: auras::AURA_MOD_STAT,
                misc_value: 0,
                base_points: rank * 2,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            },
        );
        d.spells.spells.insert(id, s);
        d.spells.ranks.insert(
            id,
            SpellRank {
                prev: if id == HIGH { LOW } else { 0 },
                first: LOW,
                rank: rank as u32,
                req: 0,
            },
        );
    }
    d.items.insert(
        SCROLL,
        ItemTemplate {
            stackable: 20,
            spells: on_use(LOW, -1),
            ..item(SCROLL)
        },
    );
    d
}

async fn setup() -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    // Nothing carried: each test gives what it needs where it wants it.
    on(&map, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
        p.apply_stats(&d);
        // Not a game master's casts, with no reagents asked.
        p.gm = false;
    })
    .await;
    (realm, a, g, map)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// Give `count` of `entry` at `at`; its guid.
fn give_at(m: &mut Map, g: Guid, entry: u32, count: u32, at: (u8, u8)) -> Guid {
    let d = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let item = crate::game::guid::item(counter);
    let p = m.players.get_mut(&g).unwrap();
    p.items.push(crate::game::map::player::Item::new(
        &d, item, g, entry, at.0, at.1, count,
    ));
    p.refresh_inventory(&d);
    item
}

fn holds(m: &Map, g: Guid, at: (u8, u8)) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    p.item_at(at.0, at.1)
        .map(|i| (p.items[i].entry, p.items[i].count()))
}

fn learn_skill(m: &mut Map, g: Guid, skill: u32, value: u32) {
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::skills::learn_skill(p, skill, 1);
    crate::game::map::skills::set_value(p, skill, value);
}

fn use_item(at: (u8, u8), targets: SpellCastTargets_SpellCastTargetFlags) -> CMSG_USE_ITEM {
    CMSG_USE_ITEM {
        bag_index: at.0,
        bag_slot: at.1,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: targets,
        },
    }
}

/// What `a` was last told of its inventory or its casts, once the map has
/// handled what it sent.
async fn answered(a: &mut Client) {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_)).then_some(())
    })
    .await;
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// What a blow of `g`'s that hit takes off `victim`, its procs sure.
fn blow(m: &mut Map, g: Guid, victim: Guid) -> u32 {
    let before = health(m, victim);
    enchant::FORCED_PROC_ROLL.with(|f| f.set(Some(0.0)));
    procs::trigger(
        m,
        Event {
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
    before.saturating_sub(health(m, victim))
}

/// Thistle Tea's "consumes an armor kit with the enchant without
/// profession gains": a kit used on worn bracers enchants them (their
/// bonus taken), one of the stack is spent, and Enchanting, though the
/// kit's spell is a recipe of it, doesn't rise (an item's cast, mangos's
/// `UpdateCraftSkill` of no cast item).
#[tokio::test]
async fn an_armor_kit_is_spent_with_its_enchant_and_teaches_nothing() {
    let (_realm, mut a, g, map) = setup().await;
    let (bracer, health) = on(&map, move |m| {
        equip(m, g, BRACER, 8);
        learn_skill(m, g, ENCHANTING, 1);
        give_at(m, g, KIT, 2, (BAG_PLAYER, PACK_START));
        let p = &m.players[&g];
        let bracer = p.items.iter().find(|i| i.entry == BRACER).unwrap().guid;
        (bracer, p.fields.get_u32(UNIT_FIELD_MAXHEALTH))
    })
    .await;
    a.send(use_item(
        (BAG_PLAYER, PACK_START),
        SpellCastTargets_SpellCastTargetFlags::empty()
            .set_item(SpellCastTargets_SpellCastTargetFlags_Item::Item { item: bracer }),
    ))
    .await;
    answered(&mut a).await;
    let after = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_by_guid(bracer).unwrap();
        (
            p.items[i].fields.get_u32(ITEM_FIELD_ENCHANTMENT),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            p.count_of(KIT),
            crate::game::map::skills::value(p, ENCHANTING),
        )
    })
    .await;
    assert_eq!(after, (HEALTH_5, health + 5, 1, Some(1)));
}

/// Thistle Tea's "rejects missing reagents, lost ownership, mismatched
/// spells, and exhausted source": an item's change wanting reagents not
/// carried, cast with no item, of an item another's now, or of an item
/// whose charges are spent, changes nothing.
#[tokio::test]
async fn a_change_without_its_reagents_item_or_charges_changes_nothing() {
    let (_realm, mut a, g, map) = setup().await;
    let pack = |n| (BAG_PLAYER, PACK_START + n);
    let old = on(&map, move |m| {
        give_at(m, g, SHORT, 1, pack(0));
        give_at(m, g, REAGENT, 2, pack(1));
        give_at(m, g, SPENT, 1, pack(2));
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_at(pack(2).0, pack(2).1).unwrap();
        p.items[i].fields.set_i32(ITEM_FIELD_SPELL_CHARGES, 0);
        give_at(m, g, OLD, 1, pack(3))
    })
    .await;
    a.clear();
    a.send(use_item(
        pack(0),
        SpellCastTargets_SpellCastTargetFlags::empty(),
    ))
    .await;
    let reagents = a
        .wait_frame("SMSG_CAST_RESULT", SMSG_CAST_RESULT, |f| {
            u32::from_le_bytes(f[0..4].try_into().unwrap()) == TO_NEW_WANTING
        })
        .await;
    assert_eq!(
        reagents[4..],
        [2, crate::game::map::spells::checks::fail::REAGENTS]
    );
    a.send(use_item(
        pack(2),
        SpellCastTargets_SpellCastTargetFlags::empty(),
    ))
    .await;
    answered(&mut a).await;
    let kept = on(&map, move |m| {
        // Cast with no item: nothing to change.
        crate::game::map::spells::cast_triggered(m, g, TO_NEW, Some(g));
        // Of an item no longer Alice's.
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(old).unwrap();
        let gone = p.items.remove(i);
        crate::game::map::extra_effects::change_item(m, g, old, NEW);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(gone);
        (
            holds(m, g, pack(0)),
            holds(m, g, pack(1)),
            holds(m, g, pack(2)),
            holds(m, g, pack(3)),
            m.players[&g].count_of(NEW),
        )
    })
    .await;
    assert_eq!(
        kept,
        (
            Some((SHORT, 1)),
            Some((REAGENT, 2)),
            Some((SPENT, 1)),
            Some((OLD, 1)),
            0
        )
    );
}

/// Thistle Tea's "revalidates the current position and preserves broken
/// items", as mangos-classic has it (round 11c): a broken item moved to the
/// bank while its change is cast isn't carried as the cast ends, so it
/// isn't changed (`CheckItems`: `HasItemCount` of the cast item, not
/// counting the bank, `SPELL_FAILED_ITEM_NOT_READY`); it stays as it was,
/// broken, and gives nothing. Thistle Tea changes it in the bank.
#[tokio::test]
async fn a_broken_item_banked_mid_change_is_not_changed() {
    let (_realm, _a, g, map) = setup().await;
    let bank = (BAG_PLAYER, BANK_START);
    let stamina = on(&map, move |m| {
        let old = give_at(m, g, OLD, 1, (BAG_PLAYER, PACK_START));
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(old).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
        crate::game::map::spells::cast(
            m,
            g,
            SLOW_TO_NEW,
            crate::game::map::spells::Targets {
                unit: Some(g),
                ..Default::default()
            },
            Some(old),
        );
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(old).unwrap();
        (p.items[i].bag, p.items[i].slot) = bank;
        p.refresh_inventory(&d);
        let now = m.now;
        if let Some(c) = m.caster_mut(g).and_then(|c| c.casting.as_mut()) {
            c.ends = now;
        }
        m.players[&g].fields.get_u32(UNIT_FIELD_STAT0 + 2)
    })
    .await;
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let changed = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_at(bank.0, bank.1).unwrap();
        (
            p.items[i].entry,
            p.items[i].fields.get_u32(ITEM_FIELD_DURABILITY),
            p.items[i].fields.get_u32(ITEM_FIELD_MAXDURABILITY),
            p.count_of(OLD) == 0,
            p.fields.get_u32(UNIT_FIELD_STAT0 + 2),
        )
    })
    .await;
    assert_eq!(changed, (OLD, 0, 100, false, stamina));
}

/// Thistle Tea's "preserves rolled loot through full bags and retries": a
/// loot slot's rolled suffix stays through a take refused for full bags,
/// and the item taken after has that suffix.
#[tokio::test]
async fn a_rolled_suffix_stays_on_its_loot_through_full_bags() {
    let (_realm, mut a, g, map) = setup().await;
    let corpse = on(&map, move |m| {
        for s in 0..16 {
            give_at(m, g, JUNK, 1, (BAG_PLAYER, PACK_START + s));
        }
        let c = spawn(m, g, BRUTE, 10, 2.0);
        m.creature_mut(c).unwrap().loot = Some(loot::Loot {
            items: vec![loot::LootSlot {
                item: SWORD,
                count: 1,
                random_property: OF_STRENGTH,
                ..Default::default()
            }],
            owner: Some(g),
            ..Default::default()
        });
        loot::show(m, g, c);
        c
    })
    .await;
    let _ = corpse;
    a.clear();
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
        _ => None,
    })
    .await;
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_at(BAG_PLAYER, PACK_START).unwrap();
        let junk = p.items[i].guid;
        crate::game::map::inventory::take_item(m, g, i, 1);
        junk
    })
    .await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == SWORD => Some(()),
        _ => None,
    })
    .await;
    let property = on(&map, move |m| {
        let p = &m.players[&g];
        p.items
            .iter()
            .find(|i| i.entry == SWORD)
            .map(|i| i.fields.get_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID))
    })
    .await;
    assert_eq!(property, Some(OF_STRENGTH));
}

/// Thistle Tea's "recomputes property bonuses once and removes them for
/// broken or unequipped gear": a worn sword's suffix gives its strength
/// once, none while broken, again repaired, none put away.
#[tokio::test]
async fn a_suffix_gives_its_strength_once_and_only_worn_and_whole() {
    let (_realm, _a, g, map) = setup().await;
    let seen = on(&map, move |m| {
        let d = m.realm.data.clone();
        let strength = |m: &Map| m.players[&g].fields.get_u32(UNIT_FIELD_STAT0);
        let base = strength(m);
        equip(m, g, SWORD, SLOT_MAIN_HAND);
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_at(BAG_PLAYER, SLOT_MAIN_HAND).unwrap();
        enchant::set_random_property(&d, &mut p.items[i], OF_STRENGTH);
        p.apply_stats(&d);
        p.apply_stats(&d);
        let worn = strength(m);
        let wear = |m: &mut Map, durability| {
            let p = m.players.get_mut(&g).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, durability);
            p.apply_stats(&d);
            strength(m)
        };
        let broken = wear(m, 0);
        let repaired = wear(m, 50);
        let p = m.players.get_mut(&g).unwrap();
        p.items[i].slot = PACK_START;
        p.refresh_inventory(&d);
        p.apply_stats(&d);
        (base, worn, broken, repaired, strength(m))
    })
    .await;
    let base = seen.0;
    assert_eq!(seen, (base, base + 3, base, base + 3, base));
}

/// Thistle Tea's "the old owner's timer cannot remove a transferred
/// item": a timed item traded runs down for its new owner alone, who is
/// told of its time; it goes from their bags when it runs out, and the
/// giver is told nothing of it.
#[tokio::test]
async fn a_timed_item_traded_runs_out_for_its_new_owner_alone() {
    let (realm, mut a, ga, map) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    let timed = on(&map, move |m| {
        give_at(m, ga, TIMED, 1, (BAG_PLAYER, PACK_START + 1))
    })
    .await;
    a.send(CMSG_INITIATE_TRADE { guid: gb }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    a.wait_for("the window open", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::OpenWindow).then_some(())
        }
        _ => None,
    })
    .await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag: BAG_PLAYER,
        slot: PACK_START + 1,
    })
    .await;
    answered(&mut a).await;
    answered(&mut b).await;
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    answered(&mut a).await;
    b.clear();
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    b.wait_for("SMSG_ITEM_TIME_UPDATE", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_TIME_UPDATE(t) if t.guid == timed => Some(()),
        _ => None,
    })
    .await;
    a.clear();
    on(&map, move |m| {
        let p = m.players.get_mut(&gb).unwrap();
        let i = p.item_by_guid(timed).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 1);
    })
    .await;
    b.wait_for("SMSG_DESTROY_OBJECT", |m| match m {
        ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) if d.guid == timed => Some(()),
        _ => None,
    })
    .await;
    let told_a = a
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) if d.guid == timed => Some(()),
            _ => None,
        })
        .await;
    assert!(told_a.is_none());
    let left = on(&map, move |m| {
        (
            m.players[&ga].item_by_guid(timed).is_some(),
            m.players[&gb].item_by_guid(timed).is_some(),
        )
    })
    .await;
    assert_eq!(left, (false, false));
}

/// Thistle Tea's "depletion suppresses later effects from the same
/// enchantment snapshot": a coating of two spells with one charge left
/// sets off only the first on the blow that spends it, and is gone.
#[tokio::test]
async fn a_coatings_last_charge_stops_its_other_spell_on_that_blow() {
    let (_realm, _a, g, map) = setup().await;
    let (hit, left) = on(&map, move |m| {
        level(m, g, 60);
        equip(m, g, SWORD, SLOT_MAIN_HAND);
        let w = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == SWORD)
            .unwrap()
            .guid;
        enchant::enchant_item(m, g, w, COATING, Some((60_000, 1)));
        let brute = spawn(m, g, BRUTE, 60, 3.0);
        let hit = blow(m, g, brute);
        let p = &m.players[&g];
        let i = p.item_by_guid(w).unwrap();
        (hit, enchant::get(&p.items[i], enchant::TEMP_SLOT).id)
    })
    .await;
    assert_eq!((hit, left), (1, 0), "one spell's point, the coating gone");
}

/// Thistle Tea's "the victim routes one request to the attacker and
/// ordinary attack feedback cannot proc twice": one blow that hit sets off
/// the blade's own chance on hit and its poison once each, one charge
/// spent.
#[tokio::test]
async fn one_blow_sets_off_the_weapon_and_its_poison_once() {
    let (_realm, _a, g, map) = setup().await;
    let seen = on(&map, move |m| {
        level(m, g, 60);
        equip(m, g, BLADE, SLOT_MAIN_HAND);
        let w = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == BLADE)
            .unwrap()
            .guid;
        enchant::enchant_item(m, g, w, POISON, Some((60_000, 3)));
        let brute = spawn(m, g, BRUTE, 60, 3.0);
        let hit = blow(m, g, brute);
        let p = &m.players[&g];
        let i = p.item_by_guid(w).unwrap();
        (hit, enchant::get(&p.items[i], enchant::TEMP_SLOT).charges)
    })
    .await;
    assert_eq!(seen, (2, 2), "a point each, a charge spent");
}

/// Thistle Tea's "an active upgrade preserves the item, mana, and
/// cooldowns": a scroll's buff read under its stronger rank is refused
/// (AURA_BOUNCED) before it costs: the scroll kept, no cooldown, no aura.
#[tokio::test]
async fn a_weaker_scroll_under_its_stronger_rank_is_refused_and_kept() {
    let (_realm, mut a, g, map) = setup().await;
    on(&map, move |m| {
        give_at(m, g, SCROLL, 3, (BAG_PLAYER, PACK_START));
        crate::game::map::spells::cast_triggered(m, g, HIGH, Some(g));
    })
    .await;
    a.clear();
    a.send(use_item(
        (BAG_PLAYER, PACK_START),
        SpellCastTargets_SpellCastTargetFlags::empty()
            .set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Unit { unit_target: g }),
    ))
    .await;
    let refused = a
        .wait_frame("SMSG_CAST_RESULT", SMSG_CAST_RESULT, |f| {
            u32::from_le_bytes(f[0..4].try_into().unwrap()) == LOW
        })
        .await;
    assert_eq!(
        refused[4..6],
        [2, crate::game::map::spells::checks::fail::AURA_BOUNCED]
    );
    let after = on(&map, move |m| {
        let p = &m.players[&g];
        (
            p.count_of(SCROLL),
            p.caster.cooldowns.contains_key(&LOW),
            auras::auras(m, g).is_some_and(|a| a.has_spell(LOW)),
            auras::auras(m, g).is_some_and(|a| a.has_spell(HIGH)),
        )
    })
    .await;
    assert_eq!(after, (3, false, false, true));
}
