//! Round 7's player tests of items, ported from Thistle Tea's
//! `test/game/world/entity/player/item_location_limits_test.exs`,
//! `disenchant_test.exs` and others (docs/parity/matrix.csv): gear bound
//! to another map taken off, and a disenchant of an item gone.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::data::{GameData, ItemTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::player::Item;
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_entity::on;

/// A sword of Stamina and a box with a three-second spell to use, both
/// bound to Shadowfang Keep (map 33).
const BOUND_SWORD: u32 = 64_210;
const BOUND_BOX: u32 = 64_211;
const BOX_SPELL: u32 = 96_410;
const MAIN_HAND: u8 = 15;

fn bound_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut sword = ItemTemplate {
        entry: BOUND_SWORD,
        name: "Keep Sword".into(),
        class: 2,
        subclass: 7,
        inventory_type: 13,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        map_bound: 33,
        ..Default::default()
    };
    sword.stats[0] = (7, 50);
    d.items.insert(BOUND_SWORD, sword);
    let mut boxed = ItemTemplate {
        entry: BOUND_BOX,
        name: "Keep Box".into(),
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        map_bound: 33,
        ..Default::default()
    };
    boxed.spells[0].id = BOX_SPELL;
    d.items.insert(BOUND_BOX, boxed);
    let mut s = crate::game::tests_p3_entity::spell(
        BOX_SPELL,
        &[SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        }],
    );
    s.casting_time_index = 14;
    d.spells.cast_times.insert(14, 3_000);
    d.spells.spells.insert(BOX_SPELL, s);
    d
}

fn max_health(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(UNIT_FIELD_MAXHEALTH)
}

fn visible_main_hand(m: &Map, g: Guid) -> u32 {
    m.players[&g]
        .fields
        .get_u32(PLAYER_VISIBLE_ITEM_1_0 + u16::from(MAIN_HAND) * VISIBLE_ITEM_SIZE)
}

/// Thistle Tea's `ItemLocationLimits.reconcile` "removing equipped gear
/// recomputes its stats and visible fields" and "removal cancels an item
/// cast, closes its loot window and projects destruction": gear bound to
/// another map comes off and goes, its Stamina's health with it and the
/// weapon no longer shown; a bound box being used, its window open, stops
/// the cast and closes the window (`SMSG_LOOT_RELEASE_RESPONSE`, as
/// mangos's `Loot` releases its looters when it goes), and goes.
#[tokio::test]
async fn gear_bound_elsewhere_goes_with_its_stats_cast_and_window() {
    let realm = harness::realm_with(bound_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let sword = guid::item(964_210);
    let boxed = guid::item(964_211);
    // Cleared before the items go on, not after: the map's own look at
    // them, each second, may take them off before the test's below.
    a.clear();
    let before = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        // Worn and carried before the first look (a login's) is taken.
        p.misc.items_login_checked = true;
        p.items
            .retain(|i| !(i.bag == BAG_PLAYER && i.slot == MAIN_HAND));
        p.refresh_inventory(&data);
        let bare = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        p.items.push(Item::new(
            &data,
            sword,
            g,
            BOUND_SWORD,
            BAG_PLAYER,
            MAIN_HAND,
            1,
        ));
        p.items
            .push(Item::new(&data, boxed, g, BOUND_BOX, BAG_PLAYER, 30, 1));
        p.refresh_inventory(&data);
        p.equip_fields();
        let worn = (max_health(m, g), visible_main_hand(m, g));
        crate::game::map::spells::learn(m, g, BOX_SPELL);
        crate::game::map::spells::cast(
            m,
            g,
            BOX_SPELL,
            crate::game::map::spells::Targets::default(),
            Some(boxed),
        );
        let casting = m
            .caster(g)
            .and_then(|c| c.casting.as_ref())
            .is_some_and(|c| c.item == Some(boxed));
        let p = m.players.get_mut(&g).unwrap();
        p.misc.item_loot = Some((boxed, Default::default()));
        p.looting = Some(boxed);
        (bare, worn, casting)
    })
    .await;
    let (bare, worn, casting) = before;
    assert_eq!(worn.0, bare + 500, "50 Stamina's health");
    assert_eq!(worn.1, BOUND_SWORD, "the sword shown");
    assert!(casting, "using the box");
    on(&map, move |m| {
        m.players.get_mut(&g).unwrap().misc.items_checked_at = None;
        crate::game::map::item_rules::limit_location(m, g);
    })
    .await;
    for item in [sword, boxed] {
        a.wait_for("SMSG_DESTROY_OBJECT", |m| match m {
            ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) => (d.guid == item).then_some(()),
            _ => None,
        })
        .await;
    }
    a.wait_for("SMSG_LOOT_RELEASE_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) => (r.guid == boxed).then_some(()),
        _ => None,
    })
    .await;
    let after = on(&map, move |m| {
        let p = &m.players[&g];
        (
            max_health(m, g),
            visible_main_hand(m, g),
            m.caster(g).and_then(|c| c.casting.as_ref()).is_some(),
            p.misc.item_loot.is_some(),
            p.looting,
            p.items.iter().any(|i| i.guid == sword || i.guid == boxed),
        )
    })
    .await;
    assert_eq!(after, (bare, 0, false, false, None, false));
}

/// Thistle Tea's `Disenchant.complete` "rejects a sold or transferred
/// target at completion": the item gone from the enchanter's bags (sold,
/// traded) when the disenchant goes off, nothing is rolled and no window
/// opens.
#[tokio::test]
async fn a_disenchant_of_an_item_gone_does_nothing() {
    use crate::game::tests_p4_items_crafts::{BRACER, DISENCHANT, ENCHANTING, setup, template};
    let (realm, mut a, g) = setup().await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.clear();
    let out = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, g, BRACER, 1, false).unwrap();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, ENCHANTING, 1);
        let i = p.items.iter().position(|i| i.entry == BRACER).unwrap();
        let bracer = p.items[i].guid;
        crate::game::map::inventory::take_item(m, g, i, 1);
        let s = template(m, DISENCHANT);
        let t = crate::game::map::spells::Targets {
            item: Some(bracer),
            ..Default::default()
        };
        crate::game::map::professions::execute(m, g, &s, &t, None);
        let p = &m.players[&g];
        (
            p.looting,
            p.misc.item_loot.is_some(),
            m.loot_state.item_loot.len(),
        )
    })
    .await;
    assert_eq!(out, (None, false, 0), "no window, nothing rolled");
    assert!(
        a.gets(std::time::Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)).then_some(())
        })
        .await
        .is_none()
    );
}
