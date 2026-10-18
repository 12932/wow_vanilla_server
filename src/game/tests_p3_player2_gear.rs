//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/{durability,equipment_transitions}_test.exs`
//! (docs/parity/matrix.csv): gear broken and mended by spells, and what
//! equipping, taking off and expiring do, in a fight and out of one.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::{ItemSpell, ItemTemplate};
use crate::game::data::spells::{Enchantment, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::inventory::{BAG_START, PACK_START};
use crate::game::map::{Map, combat, enchant, procs};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_player2_ammo::handled;

/// A sword with five stamina; a trinket with a use; a bag; a timed
/// trinket.
const BLADE: u32 = 64_400;
const CHARM: u32 = 64_401;
const SACK: u32 = 64_402;
const FLEETING: u32 = 64_403;
/// Wear by a whole share of the main hand, and mending by points.
const SHATTER: u32 = 64_410;
const MEND: u32 = 64_411;
/// The charm's use, and the blade's enchantment's blow.
const CHARM_USE: u32 = 64_412;
const BLOW: u32 = 64_413;
const ENCHANT: u32 = 64_420;

fn item(entry: u32, class: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class,
        name: format!("Item {entry}"),
        display_id: 1000,
        inventory_type,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut blade = item(BLADE, 2, 13);
    blade.subclass = 7;
    blade.delay = 2000;
    blade.max_durability = 50;
    blade.stats[0] = (7, 5);
    blade.damages[0] = (10.0, 10.0, 0);
    let mut charm = item(CHARM, 4, 12);
    charm.spells[0] = ItemSpell {
        id: CHARM_USE,
        trigger: 0,
        ..Default::default()
    };
    let mut sack = item(SACK, 1, 18);
    sack.container_slots = 4;
    let mut fleeting = item(FLEETING, 4, 12);
    fleeting.duration = 3600;
    for t in [blade, charm, sack, fleeting] {
        d.items.insert(t.entry, t);
    }
    let spell = |id: u32, effect: SpellEffect| {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 1,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = effect;
        s
    };
    let wear = |effect: u32, points: i32| SpellEffect {
        effect,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        misc_value: 15,
        target_a: 1,
        ..Default::default()
    };
    let blow = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    for s in [
        spell(SHATTER, wear(115, 100)),
        spell(MEND, wear(111, -50_000)),
        spell(CHARM_USE, SpellEffect::default()),
        spell(BLOW, blow),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.enchantments.insert(
        ENCHANT,
        Enchantment {
            id: ENCHANT,
            effects: [
                (enchant::TYPE_COMBAT_SPELL, 100, 0, BLOW),
                (0, 0, 0, 0),
                (0, 0, 0, 0),
            ],
            name: "Blows".into(),
        },
    );
    d
}

/// Give `g` one of `entry`, in the pack; its guid.
fn give(m: &mut Map, g: Guid, entry: u32) -> Guid {
    crate::game::map::inventory::add_item(m, g, entry, 1, false).unwrap();
    let p = &m.players[&g];
    p.items
        .iter()
        .rev()
        .find(|i| i.entry == entry)
        .unwrap()
        .guid
}

fn at(m: &Map, g: Guid, item: Guid) -> Option<(u8, u8)> {
    let p = &m.players[&g];
    p.item_by_guid(item)
        .map(|i| (p.items[i].bag, p.items[i].slot))
}

fn health_of(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXHEALTH))
}

/// A blow of `g`'s main hand on `target`, its procs and all.
fn strike(m: &mut Map, g: Guid, target: Guid) {
    procs::trigger(
        m,
        procs::Event {
            actor: g,
            target,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex: procs::EX_NORMAL_HIT,
            school: 1,
            ..Default::default()
        },
    );
}

/// Wait until `f` says so, as the map has it.
async fn until<T: Send + 'static>(
    map: &crate::game::map::MapHandle,
    what: &str,
    f: impl Fn(&mut Map) -> Option<T> + Send + Sync + Clone + 'static,
) -> T {
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        if let Some(t) = on(map, f.clone()).await {
            return t;
        }
        assert!(tokio::time::Instant::now() < deadline, "never {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Whether `c` hears a durability log of `spell` within [`NONE_WITHIN`].
async fn logged(c: &mut Client, spell: u32) -> bool {
    c.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::SMSG_SPELLLOGEXECUTE(x) if x.spell == spell => Some(()),
        _ => None,
    })
    .await
    .is_some()
}

/// Thistle Tea's "spell completion breaks and restores equipment through
/// the owner transaction" and "broken enchanted weapons lose stats and
/// procs, then regain them on repair": a spell that wears the main hand by
/// all of it breaks the blade, its stamina gone and its enchantment's
/// blows with it, with no log; one of negative points mends it whole (as
/// mangos's `DurabilityPointsLoss` takes a negative loss), logged, and all
/// comes back.
#[tokio::test]
async fn a_spell_breaks_a_weapon_and_another_mends_it() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (target, bare) = on(&map, move |m| {
        let bare = health_of(m, ga);
        crate::game::tests_p3_combat::equip(m, ga, BLADE, 15);
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == BLADE).unwrap();
        enchant::set(
            &mut p.items[i],
            enchant::PERM_SLOT,
            enchant::Enchant {
                id: ENCHANT,
                ..Default::default()
            },
        );
        (spawn(m, ga, BRUTE, 20, 3.0), bare)
    })
    .await;
    let target_health = move |m: &Map| {
        m.creature(target)
            .unwrap()
            .fields
            .get_u32(UNIT_FIELD_HEALTH)
    };
    let whole = until(&map, "the stamina", move |m| {
        let h = health_of(m, ga);
        (h > bare).then_some(h)
    })
    .await;
    assert_eq!(whole, bare + 50, "five stamina");
    let struck = on(&map, move |m| {
        let before = target_health(m);
        strike(m, ga, target);
        before - target_health(m)
    })
    .await;
    assert_eq!(struck, 10, "the enchantment's blow");
    let durability = move |m: &Map| {
        let p = &m.players[&ga];
        p.items
            .iter()
            .find(|i| i.entry == BLADE)
            .unwrap()
            .fields
            .get_u32(ITEM_FIELD_DURABILITY)
    };
    a.clear();
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, SHATTER, Some(ga));
    })
    .await;
    let broken = until(&map, "the stamina gone", move |m| {
        (health_of(m, ga) == bare).then(|| durability(m))
    })
    .await;
    assert_eq!(broken, 0, "broken");
    assert!(!logged(&mut a, SHATTER).await, "a share isn't logged");
    let struck = on(&map, move |m| {
        let before = target_health(m);
        strike(m, ga, target);
        before - target_health(m)
    })
    .await;
    assert_eq!(struck, 0, "no blows from a broken blade");
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, MEND, Some(ga));
    })
    .await;
    let mended = until(&map, "the stamina back", move |m| {
        (health_of(m, ga) == whole).then(|| durability(m))
    })
    .await;
    assert_eq!(mended, 50, "mended whole");
    assert!(logged(&mut a, MEND).await, "points are logged");
    let struck = on(&map, move |m| {
        let before = target_health(m);
        strike(m, ga, target);
        before - target_health(m)
    })
    .await;
    assert_eq!(struck, 10, "the blows back");
}

/// `CMSG_AUTOEQUIP_ITEM_SLOT`, handled.
async fn equip_to(a: &mut Client, item: Guid, slot: u8) {
    a.send(CMSG_AUTOEQUIP_ITEM_SLOT {
        guid: item,
        destination_slot: slot,
    })
    .await;
    handled(a).await;
}

/// Thistle Tea's "same-slot requests leave the equip cooldown and packets
/// untouched", "ignores unowned items, missing items, and storage
/// destinations" and "equips bags into an explicit bag-bar slot": a charm
/// put on waits its thirty seconds, and asked into the slot it is in
/// changes nothing; an item not there, or a slot past the equipment,
/// changes nothing and says nothing; a bag goes into the bag slot asked.
#[tokio::test]
async fn equipping_into_a_slot_takes_what_fits_and_ignores_the_rest() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (charm, sack) = on(&map, move |m| (give(m, ga, CHARM), give(m, ga, SACK))).await;
    a.clear();
    equip_to(&mut a, charm, 13).await;
    a.wait_for("the charm's wait", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_COOLDOWN(x) if x.guid == charm => Some(()),
        _ => None,
    })
    .await;
    let ready = move |m: &Map| m.players[&ga].caster.cooldowns.get(&CHARM_USE).copied();
    let (place, until_ready) = on(&map, move |m| (at(m, ga, charm), ready(m))).await;
    assert_eq!(place, Some((BAG_PLAYER, 13)));
    let until_ready = until_ready.expect("thirty seconds");
    equip_to(&mut a, charm, 13).await;
    let again = on(&map, move |m| ready(m)).await;
    assert_eq!(again, Some(until_ready), "the wait as it was");
    // Nothing for an item not there, nor for a slot past the equipment.
    for (item, slot) in [
        (charm, 23),
        (charm, 39),
        (charm, 63),
        (charm, 255),
        (crate::game::guid::item(999_999), 13),
    ] {
        a.clear();
        equip_to(&mut a, item, slot).await;
        let place = on(&map, move |m| at(m, ga, charm)).await;
        assert_eq!(place, Some((BAG_PLAYER, 13)), "{slot}");
        let said = a
            .gets(Duration::from_millis(1), |m| match m {
                ServerOpcodeMessage::SMSG_ITEM_COOLDOWN(_)
                | ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
                _ => None,
            })
            .await;
        assert!(said.is_none(), "nothing said for {slot}");
    }
    equip_to(&mut a, sack, BAG_START + 3).await;
    let place = on(&map, move |m| at(m, ga, sack)).await;
    assert_eq!(
        place,
        Some((BAG_PLAYER, BAG_START + 3)),
        "the bag slot asked"
    );
}

/// The name of the failure `c` is sent, waiting for it.
async fn failure(c: &mut Client) -> String {
    c.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(x) => {
            let s = format!("{:?}", x.result);
            Some(s.split([' ', '{']).next().unwrap_or("").to_string())
        }
        _ => None,
    })
    .await
}

/// Thistle Tea's "rejects both direct unequip and auto-store of combat
/// trinkets" and "system removal can expire equipment while the owner is
/// fighting": in a fight a trinket comes off neither by a move nor put
/// away, each refused as not in combat, but a timed one runs out.
#[tokio::test]
async fn a_trinket_stays_on_in_a_fight_but_a_timed_one_runs_out() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (charm, fleeting) = on(&map, move |m| {
        let charm = give(m, ga, CHARM);
        let fleeting = give(m, ga, FLEETING);
        for (item, slot) in [(charm, 13), (fleeting, 12)] {
            crate::game::map::inventory::move_item(
                m,
                ga,
                at(m, ga, item).unwrap(),
                (BAG_PLAYER, slot),
            );
        }
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let c = m.creature_mut(brute).unwrap();
        c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 0.0);
        c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 0.0);
        combat::engage(m, ga, brute);
        (charm, fleeting)
    })
    .await;
    let fighting = on(&map, move |m| {
        m.players[&ga].fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
    .await;
    assert!(fighting);
    a.clear();
    a.send(CMSG_SWAP_INV_ITEM {
        source_slot: ItemSlot::Trinket2,
        destination_slot: ItemSlot::Inventory0,
    })
    .await;
    assert_eq!(failure(&mut a).await, "NotInCombat", "moved off");
    a.send(CMSG_AUTOSTORE_BAG_ITEM {
        source_bag: BAG_PLAYER,
        source_slot: 13,
        destination_bag: BAG_PLAYER,
    })
    .await;
    assert_eq!(failure(&mut a).await, "NotInCombat", "put away");
    let place = on(&map, move |m| at(m, ga, charm)).await;
    assert_eq!(place, Some((BAG_PLAYER, 13)), "still on");
    let _ = PACK_START;
    let gone = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_by_guid(fleeting).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 1);
        let then = m.now - Duration::from_secs(2);
        m.loot_state.items_tick_at = Some(then);
        crate::game::map::items::update(m);
        at(m, ga, fleeting)
    })
    .await;
    assert_eq!(gone, None, "run out in the fight");
}
