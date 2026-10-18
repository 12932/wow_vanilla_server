//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/teaching_dbc_test.exs` (docs/parity/matrix.csv):
//! who may use an item, and a recipe book that teaches what is known.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{GameData, ItemTemplate};
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, inventory};
use crate::game::tests_p3_combat::setup_with;
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

/// A recipe and the spell it teaches; a book for a cook of 125, and one for
/// level 60; and a trinket with a use.
const RECIPE: u32 = 64_500;
const TEACH: u32 = 95_500;
const DISH: u32 = 95_501;
const EXPERT_RECIPE: u32 = 64_501;
const HIGH_RECIPE: u32 = 64_502;
const TRINKET: u32 = 64_503;
const TRINKET_USE: u32 = 95_502;
const COOKING: u32 = 185;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut teach = SpellTemplate {
        id: TEACH,
        name: "Recipe".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    teach.effects[0] = SpellEffect {
        effect: crate::game::map::spells::EFFECT_LEARN_SPELL,
        trigger_spell: DISH,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(TEACH, teach);
    d.spells.spells.insert(
        DISH,
        SpellTemplate {
            id: DISH,
            name: "Dish".into(),
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    let mut trinket_use = SpellTemplate {
        id: TRINKET_USE,
        name: "Trinket".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    trinket_use.effects[0] = SpellEffect {
        effect: 6,
        aura: 4,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(TRINKET_USE, trinket_use);
    let book = |entry: u32, skill_rank: u32, level: u32| {
        let mut t = ItemTemplate {
            entry,
            class: 9,
            name: format!("Book {entry}"),
            display_id: 1000,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            required_skill: if skill_rank > 0 { COOKING } else { 0 },
            required_skill_rank: skill_rank,
            required_level: level,
            ..Default::default()
        };
        t.spells[0] = ItemSpell {
            id: TEACH,
            trigger: 0,
            charges: -1,
            ..Default::default()
        };
        t
    };
    d.items.insert(RECIPE, book(RECIPE, 0, 0));
    d.items.insert(EXPERT_RECIPE, book(EXPERT_RECIPE, 125, 0));
    d.items.insert(HIGH_RECIPE, book(HIGH_RECIPE, 0, 60));
    let mut trinket = ItemTemplate {
        entry: TRINKET,
        class: 4,
        inventory_type: 12,
        name: "Trinket".into(),
        display_id: 1000,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    trinket.spells[0] = ItemSpell {
        id: TRINKET_USE,
        trigger: 0,
        ..Default::default()
    };
    d.items.insert(TRINKET, trinket);
    d
}

/// `CMSG_USE_ITEM` of the item of `entry` `g` has: the inventory failure,
/// or the cast result's code, if either.
async fn used(
    a: &mut Client,
    map: &MapHandle,
    g: Guid,
    entry: u32,
) -> (Option<String>, Option<u8>) {
    let (bag, slot) = on(map, move |m| {
        let p = &m.players[&g];
        let i = p.items.iter().position(|i| i.entry == entry).unwrap();
        (p.items[i].bag, p.items[i].slot)
    })
    .await;
    a.clear();
    a.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
    handled(a).await;
    let failure = a
        .gets(Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => Some(
                format!("{:?}", f.result)
                    .split([' ', '{'])
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            ),
            _ => None,
        })
        .await;
    // The cast result is laid out by hand: the spell, 2 for a failure, why.
    let result = tokio::time::timeout(
        Duration::from_millis(1),
        a.wait_frame("SMSG_CAST_RESULT", 0x130, |b| b.len() >= 6 && b[4] == 2),
    )
    .await
    .ok()
    .map(|b| b[5]);
    (failure, result)
}

fn count(m: &Map, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

/// Thistle Tea's teaching "teaches on completion, consumes once, and
/// rejects a duplicate", "insufficient skill prevents the item cast from
/// starting" and mangos's `CanUseItem` at `HandleUseItemOpcode`: a recipe
/// teaches its dish and is spent; a second of it, its dish known, is
/// refused with "already learned" (0x62) and kept. A book for a cook of
/// 125 is refused to one of 124 ("not skilled enough") and to one who
/// doesn't cook, and one for level 60 to the low; each kept.
#[tokio::test]
async fn a_recipe_teaches_once_and_asks_for_its_skill_and_level() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| {
        for entry in [RECIPE, RECIPE, EXPERT_RECIPE, HIGH_RECIPE] {
            inventory::add_item(m, ga, entry, 1, false).unwrap();
        }
    })
    .await;
    let (failure, _) = used(&mut a, &map, ga, RECIPE).await;
    assert_eq!(failure, None);
    let (known, left) = on(&map, move |m| {
        (m.players[&ga].spells.contains(&DISH), count(m, ga, RECIPE))
    })
    .await;
    assert_eq!((known, left), (true, 1), "taught, and spent");
    let (_, result) = used(&mut a, &map, ga, RECIPE).await;
    assert_eq!(result, Some(0x62), "already learned");
    assert_eq!(on(&map, move |m| count(m, ga, RECIPE)).await, 1, "kept");
    let (failure, _) = used(&mut a, &map, ga, EXPERT_RECIPE).await;
    assert_eq!(
        failure.as_deref(),
        Some("NoRequiredProficiency"),
        "no cooking"
    );
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        crate::game::map::skills::learn_skill(p, COOKING, 2);
        crate::game::map::skills::set_value(p, COOKING, 124);
    })
    .await;
    let (failure, _) = used(&mut a, &map, ga, EXPERT_RECIPE).await;
    assert_eq!(failure.as_deref(), Some("CantEquipSkill"), "a cook of 124");
    let (failure, _) = used(&mut a, &map, ga, HIGH_RECIPE).await;
    assert_eq!(failure.as_deref(), Some("CantEquipLevelI"), "level 1");
    let kept = on(&map, move |m| {
        (count(m, ga, EXPERT_RECIPE), count(m, ga, HIGH_RECIPE))
    })
    .await;
    assert_eq!(kept, (1, 1));
}

/// mangos's `HandleUseItemOpcode`: gear is used worn, not from the bags.
#[tokio::test]
async fn a_trinket_is_used_worn_not_from_the_bags() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| {
        inventory::add_item(m, ga, TRINKET, 1, false).unwrap();
    })
    .await;
    let (failure, _) = used(&mut a, &map, ga, TRINKET).await;
    assert_eq!(failure.as_deref(), Some("ItemNotFound"), "in the bags");
    let has = move |m: &mut Map| m.players[&ga].auras.has_spell(TRINKET_USE);
    assert!(!on(&map, has).await);
    on(&map, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .items
            .retain(|i| i.entry != TRINKET);
        crate::game::tests_p3_combat::equip(m, ga, TRINKET, 12);
    })
    .await;
    let (failure, _) = used(&mut a, &map, ga, TRINKET).await;
    assert_eq!(failure, None, "worn");
    assert!(on(&map, has).await, "its use on");
}
