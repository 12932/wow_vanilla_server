//! Phase 3's fourth round of a corpse's loot and skin, ported from Thistle
//! Tea's `test/game/world/entity/mob/corpse_test.exs`
//! (docs/parity/matrix.csv): skinning waits for the corpse's loot, is the
//! skinner's, is had once, and fails to be tried again.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::data::{GameData, ItemTemplate, LootEntry};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::professions::{SKILL_SKINNING, UNIT_FLAG_SKINNABLE};
use crate::game::map::spells::Targets;
use crate::game::tests_p3_combat::{data, setup_with, spawn};
use crate::game::tests_p3_entity::spell;
use crate::game::tests_p4_world::{SMSG_CAST_RESULT, on};

const SKINNING: u32 = 98_300;
const HIDE: u32 = 98_301;
const TROPHY: u32 = 98_302;
/// A beast with a trophy on it and a hide to skin, and one whose skin has
/// no rows.
const BEAST: u32 = 98_310;
const BARE_BEAST: u32 = 98_311;

const FAIL_TARGET_NOT_LOOTED: u8 = 0x70;
const FAIL_TARGET_UNSKINNABLE: u8 = 0x74;
const FAIL_TRY_AGAIN: u8 = 0x7a;

fn world() -> GameData {
    let mut d = data();
    for entry in [HIDE, TROPHY] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                stackable: 20,
                ..Default::default()
            },
        );
    }
    let row = |item| LootEntry {
        item,
        chance: 100.0,
        min: 1,
        max: 1,
        ..Default::default()
    };
    for (entry, skin) in [(BEAST, 77), (BARE_BEAST, 78)] {
        let mut t = crate::game::tests_p3_combat::template(entry, 1);
        t.loot_id = 76;
        t.skinning_loot_id = skin;
        d.creature_templates.insert(entry, t);
    }
    d.loot.creature.insert(76, vec![row(TROPHY)]);
    d.loot.skinning.insert(77, vec![row(HIDE)]);
    d.spells.spells.insert(
        SKINNING,
        spell(
            SKINNING,
            &[SpellEffect {
                effect: 95,
                target_a: 25,
                ..Default::default()
            }],
        ),
    );
    d
}

fn skinnable(m: &Map, c: Guid) -> bool {
    m.creature(c)
        .is_some_and(|c| c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE))
}

/// `g` skins `c`: whether it took, by the corpse's loot now being skins.
fn skin(m: &mut Map, g: Guid, c: Guid) {
    if let Some(x) = m.caster_mut(g) {
        x.gcd_until = None;
        x.cooldowns.clear();
    }
    crate::game::map::spells::cast(
        m,
        g,
        SKINNING,
        Targets {
            unit: Some(c),
            ..Default::default()
        },
        None,
    );
}

fn skinner(m: &mut Map, g: Guid, value: u32) {
    let p = m.players.get_mut(&g).unwrap();
    p.spells.push(SKINNING);
    crate::game::map::skills::learn_skill(p, SKILL_SKINNING, 4);
    crate::game::map::skills::set_value(p, SKILL_SKINNING, value);
}

/// A cast refused with `code`.
fn refused(code: u8) -> impl FnMut(&[u8]) -> bool {
    move |b: &[u8]| b.len() >= 6 && b[4] == 2 && b[5] == code
}

/// Thistle Tea's "waits for ordinary loot, claims once, and makes the skin
/// private" and "empty skin tables cannot be rolled again": a corpse is
/// skinnable once its loot is taken, refused before; its skins are the
/// skinner's alone; once skinned it is no more; and a skin of no rows is
/// had (with nothing in it) all the same, and not again.
#[tokio::test]
async fn a_corpse_is_skinned_once_after_its_loot_and_the_skin_is_the_skinners() {
    let (realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bea", Race::Human, Class::Warrior).await;
    let beast = on(&map, move |m| {
        skinner(m, g, 300);
        skinner(m, gb, 300);
        let beast = spawn(m, g, BEAST, 20, 2.0);
        crate::game::map::ai::attacked(m, beast, g);
        crate::game::map::combat::kill(m, g, beast);
        beast
    })
    .await;
    a.clear();
    let before = on(&map, move |m| {
        skin(m, gb, beast);
        skinnable(m, beast)
    })
    .await;
    b.wait_frame(
        "not looted",
        SMSG_CAST_RESULT,
        refused(FAIL_TARGET_NOT_LOOTED),
    )
    .await;
    assert!(!before, "not skinnable with its trophy on it");
    // Alice takes the trophy: Bea may skin it.
    on(&map, move |m| {
        crate::game::map::loot::open(m, g, beast);
    })
    .await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: beast }).await;
    let mut ready = false;
    for _ in 0..100 {
        ready = on(&map, move |m| skinnable(m, beast)).await;
        if ready {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(ready, "skinnable once looted");
    let skins = on(&map, move |m| {
        skin(m, gb, beast);
        let l = m.creature(beast).unwrap().loot.clone().unwrap();
        (
            skinnable(m, beast),
            l.items.iter().map(|s| s.item).collect::<Vec<u32>>(),
            l.may_loot(gb),
            l.may_loot(g),
        )
    })
    .await;
    assert_eq!(
        skins,
        (false, vec![HIDE], true, false),
        "Bea's hide, not Alice's"
    );
    b.clear();
    on(&map, move |m| skin(m, gb, beast)).await;
    b.wait_frame(
        "skinned",
        SMSG_CAST_RESULT,
        refused(FAIL_TARGET_UNSKINNABLE),
    )
    .await;
    // A skin of no rows.
    let bare = on(&map, move |m| {
        let bare = spawn(m, g, BARE_BEAST, 20, 2.0);
        crate::game::map::combat::kill(m, g, bare);
        if let Some(c) = m.creature_mut(bare) {
            c.loot = None;
            c.fields
                .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE, true);
        }
        skin(m, gb, bare);
        let items = m
            .creature(bare)
            .and_then(|c| c.loot.as_ref())
            .map(|l| l.items.len());
        (bare, skinnable(m, bare), items)
    })
    .await;
    assert_eq!((bare.1, bare.2), (false, Some(0)), "skinned, of nothing");
    b.clear();
    on(&map, move |m| skin(m, gb, bare.0)).await;
    b.wait_frame(
        "not again",
        SMSG_CAST_RESULT,
        refused(FAIL_TARGET_UNSKINNABLE),
    )
    .await;
}

/// Thistle Tea's "leaves a failed or out-of-range attempt available for
/// retry": a skinner whose roll falls short is told to try again, and the
/// corpse is left to be skinned.
#[tokio::test]
async fn a_failed_skinning_leaves_the_corpse_to_try_again() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let beast = on(&map, move |m| {
        let beast = spawn(m, g, BARE_BEAST, 20, 2.0);
        crate::game::map::combat::kill(m, g, beast);
        if let Some(c) = m.creature_mut(beast) {
            c.loot = None;
            c.fields
                .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE, true);
        }
        // Just enough skill for a level 20 corpse: a roll falls short often.
        let level = 20;
        skinner(m, g, level * 5);
        beast
    })
    .await;
    let mut failed = false;
    for _ in 0..60 {
        a.clear();
        let taken = on(&map, move |m| {
            skin(m, g, beast);
            !skinnable(m, beast)
        })
        .await;
        if taken {
            // Put it back to try again.
            on(&map, move |m| {
                if let Some(c) = m.creature_mut(beast) {
                    c.loot = None;
                    c.fields
                        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE, true);
                }
            })
            .await;
            continue;
        }
        a.wait_frame("try again", SMSG_CAST_RESULT, refused(FAIL_TRY_AGAIN))
            .await;
        assert!(
            on(&map, move |m| skinnable(m, beast)).await,
            "left to try again"
        );
        failed = true;
        break;
    }
    assert!(failed, "a roll short of the corpse's need at some point");
}
