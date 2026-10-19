//! Round 8's combat tests, a shield's block value (Thistle Tea's
//! `shield_block_test.exs`): its auras by their stacks, flat before the
//! percentages, each percentage on its own; and the block value of gear,
//! counted once while it is worn.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::{ItemSpell, ItemTemplate};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::map::auras::{self, AURA_MOD_SHIELD_BLOCKVALUE, AURA_MOD_SHIELD_BLOCKVALUE_PCT};
use crate::game::map::derive::SPELL_BLOCK;
use crate::game::map::{Map, inventory, spells};
use crate::game::tests_p3_combat::{self as p3, OFF_HAND, SHIELD};

/// Thirty block value, worn; a trinket that gives it, and one whose use
/// would.
const BULWARK: u32 = 96_800;
const WORN: u32 = 63_900;
const USED: u32 = 63_901;
const TRINKET_SLOT: u8 = 12;

fn trinket(entry: u32, trigger: u32) -> ItemTemplate {
    let mut t = ItemTemplate {
        entry,
        class: 4,
        subclass: 0,
        name: format!("Trinket {entry}"),
        display_id: 1000,
        inventory_type: 12,
        stackable: 1,
        ..Default::default()
    };
    t.spells[0] = ItemSpell {
        id: BULWARK,
        trigger,
        ..Default::default()
    };
    t
}

fn data() -> GameData {
    let mut d = p3::data();
    let mut bulwark = SpellTemplate {
        id: BULWARK,
        name: "Bulwark".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    bulwark.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_SHIELD_BLOCKVALUE,
        base_points: 29,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(BULWARK, bulwark);
    for t in [trinket(WORN, 1), trinket(USED, 0)] {
        d.items.insert(t.entry, t);
    }
    d
}

fn block_value(m: &mut Map, g: Guid) -> u32 {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.apply_stats(&data);
    p.derived.block_value
}

/// Alice of 140 strength with a shield of 20 block, knowing Block: a
/// block value of 20 + 140 / 20 - 1, 26.
async fn shield_bearer() -> (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    crate::game::map::MapHandle,
) {
    let (realm, a, g, map) = p3::setup_with(data(), Class::Warrior).await;
    map.query(move |m| {
        p3::level(m, g, 60);
        spells::learn(m, g, SPELL_BLOCK);
        p3::equip(m, g, SHIELD, OFF_HAND);
    })
    .await
    .unwrap();
    (realm, a, g, map)
}

/// Thistle Tea's "scales stacks and multiplies independent percentage
/// bonuses", as mangos's `GetShieldBlockValue` has it: ten block value
/// twice stacked and a tenth more twice stacked, with half again, make
/// (26 + 20) x 1.2 x 1.5, 82.
#[tokio::test]
async fn block_values_auras_go_by_their_stacks_and_each_percentage_multiplies() {
    let (_realm, _a, g, map) = shield_bearer().await;
    let r = map
        .query(move |m| {
            let base = block_value(m, g);
            p3::aura_of(m, g, 96_801, AURA_MOD_SHIELD_BLOCKVALUE, 0, 10);
            p3::aura_of(m, g, 96_802, AURA_MOD_SHIELD_BLOCKVALUE_PCT, 0, 10);
            p3::aura_of(m, g, 96_803, AURA_MOD_SHIELD_BLOCKVALUE_PCT, 0, 50);
            if let Some(a) = auras::auras_mut(m, g) {
                for h in a
                    .holders
                    .iter_mut()
                    .filter(|h| matches!(h.spell, 96_801 | 96_802))
                {
                    h.stacks = 2;
                }
            }
            (base, block_value(m, g))
        })
        .await
        .unwrap();
    assert_eq!(r, (26, 82));
}

/// Thistle Tea's "equipped block-value spells contribute once and
/// disappear on unequip": a trinket worn that gives thirty block value
/// adds it once, however often her gear is looked at again; one that
/// would on use adds nothing; taken off, it is gone.
#[tokio::test]
async fn gears_block_value_counts_once_while_worn_and_goes_with_it() {
    let (_realm, _a, g, map) = shield_bearer().await;
    let r = map
        .query(move |m| {
            p3::equip(m, g, WORN, TRINKET_SLOT);
            inventory::sync_gear(m, g);
            let worn = block_value(m, g);
            inventory::sync_gear(m, g);
            let again = block_value(m, g);
            p3::equip(m, g, USED, TRINKET_SLOT);
            inventory::sync_gear(m, g);
            let used = block_value(m, g);
            m.players
                .get_mut(&g)
                .unwrap()
                .items
                .retain(|i| i.entry != USED);
            inventory::sync_gear(m, g);
            (worn, again, used, block_value(m, g))
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (56, 56, 26, 26),
        "(worn, looked at again, a trinket of use, none)"
    );
}
