//! Round 6's aura tests, ported from Thistle Tea's
//! `test/game/core/entity/appearance_test.exs` and others
//! (docs/parity/matrix.csv): which of a unit's transforms it shows.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{CUSTOM_POSITIVE, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness;
use crate::game::map::auras;
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{setup_with, template};
use crate::game::tests_p3_entity::on;

/// Disguises (helpful) and curses (harmful), each into a creature of its
/// own look: display 100 for the first, 200, 300.
const DISGUISE: u32 = 96_910;
const CURSE: u32 = 96_920;
const LOOKS: u32 = 3_800;

fn transform(id: u32, look: u32, helpful: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Transform {id}"),
        equipped_item_class: -1,
        custom_flags: if helpful { CUSTOM_POSITIVE } else { 0 },
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_TRANSFORM,
        misc_value: (LOOKS + look) as i32,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for look in 1..=3 {
        let mut t = template(LOOKS + look, 7);
        t.display_ids = [100 * look, 0, 0, 0];
        d.creature_templates.insert(LOOKS + look, t);
        for (base, helpful) in [(DISGUISE, true), (CURSE, false)] {
            d.spells
                .spells
                .insert(base + look, transform(base + look, look, helpful));
        }
    }
    d
}

fn put(m: &mut Map, g: Guid, id: u32) {
    let s = m.realm.data.spells.spells[&id].clone();
    auras::apply_spell(m, g, g, &s, &[(0, 0)], auras::Context::default()).expect("put on");
}

fn display(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(UNIT_FIELD_DISPLAYID)
}

/// Put `id` on `g` at the map's next update, and the display it shows.
async fn then_put(map: &MapHandle, g: Guid, id: u32) -> u32 {
    harness::an_update(map).await;
    on(map, move |m| {
        put(m, g, id);
        display(m, g)
    })
    .await
}

/// Take `id` off `g`, and the display it shows.
async fn take(map: &MapHandle, g: Guid, id: u32) -> u32 {
    on(map, move |m| {
        auras::remove_where(m, g, |h| h.spell == id);
        display(m, g)
    })
    .await
}

/// Thistle Tea's `Appearance`: "newer disguises replace the display and
/// reveal the older one on removal", "harmful transforms override both
/// earlier and later disguises", "newest harmful transform wins and
/// expiry restores its predecessor", "refreshing changes priority without
/// relying on holder position" and "applications at the same timestamp
/// retain application order": a unit shows its newest transform (a
/// refresh is new), a harmful one over any helpful (mangos's
/// `HandleAuraTransform` prefers the negative), those put on at once the
/// last of them; one gone, the next, and none its own look.
#[tokio::test]
async fn a_unit_shows_its_newest_transform_a_harmful_one_first() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let native = on(&map, move |m| display(m, ga)).await;
    // Newer over older, and back.
    assert_eq!(then_put(&map, ga, DISGUISE + 1).await, 100);
    assert_eq!(then_put(&map, ga, DISGUISE + 2).await, 200);
    assert_eq!(take(&map, ga, DISGUISE + 2).await, 100);
    assert_eq!(take(&map, ga, DISGUISE + 1).await, native);
    // A curse over the disguises before it and after it.
    then_put(&map, ga, DISGUISE + 1).await;
    assert_eq!(then_put(&map, ga, CURSE + 2).await, 200);
    assert_eq!(
        then_put(&map, ga, DISGUISE + 3).await,
        200,
        "the curse still"
    );
    assert_eq!(take(&map, ga, CURSE + 2).await, 300);
    take(&map, ga, DISGUISE + 1).await;
    take(&map, ga, DISGUISE + 3).await;
    // The newest curse, then the one before.
    then_put(&map, ga, CURSE + 1).await;
    assert_eq!(then_put(&map, ga, CURSE + 2).await, 200);
    assert_eq!(take(&map, ga, CURSE + 2).await, 100);
    take(&map, ga, CURSE + 1).await;
    // A refresh is the newest, the two still on.
    then_put(&map, ga, DISGUISE + 1).await;
    then_put(&map, ga, DISGUISE + 2).await;
    assert_eq!(then_put(&map, ga, DISGUISE + 1).await, 100, "refreshed");
    let on_her = on(&map, move |m| {
        auras::auras(m, ga).map_or(0, |a| {
            a.holders
                .iter()
                .filter(|h| matches!(h.spell, x if x == DISGUISE + 1 || x == DISGUISE + 2))
                .count()
        })
    })
    .await;
    assert_eq!(on_her, 2);
    take(&map, ga, DISGUISE + 1).await;
    take(&map, ga, DISGUISE + 2).await;
    // Two at once: the second.
    let both = on(&map, move |m| {
        put(m, ga, DISGUISE + 1);
        put(m, ga, DISGUISE + 2);
        display(m, ga)
    })
    .await;
    assert_eq!(both, 200);
}
