//! Round 7, against Thistle Tea's `test/game/core/entity_test.exs` and its
//! neighbours (docs/parity/matrix.csv): what a player's death takes of
//! what they control, and what a blow they live through leaves.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::spells::SpellEffect;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::harness;
use crate::game::map::{Map, MapHandle, auras, combat, pets, spells};
use crate::game::tests_p3_entity::on;

/// A totem in the first slot and its creature, and a charm of a creature
/// (Enslave Demon's aura) that lasts.
const TOTEM_SPELL: u32 = 997_701;
const TOTEM: u32 = 997_750;
const CHARM: u32 = 997_702;

fn controls(d: &mut GameData) {
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
    let mut totem = bare(TOTEM_SPELL, 3);
    totem.duration_index = LASTS_LONG;
    effect(
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
    let mut charm = bare(CHARM, 5);
    charm.duration_index = LASTS_LONG;
    effect(&mut charm, 0, aura(auras::AURA_MOD_CHARM, 0, 0, 6));
    insert(d, [totem, charm]);
}

/// Run the map's updates until `f` holds.
async fn until(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&Map) -> bool + Send + Sync + Clone + 'static,
) {
    crate::game::tests_p3_entity_objects::until(map, what, f).await;
}

/// What `g` controls: its totems and its charm, and whether the charmed
/// creature still serves it.
fn controlled(m: &Map, g: Guid, kobold: Guid) -> (usize, Option<Guid>, bool) {
    let o = &m.players[&g].owned;
    let serves = pets::state(m, kobold).is_some_and(|s| s.owner == g);
    (o.totems.iter().flatten().count(), o.charm, serves)
}

/// Thistle Tea's `take_damage_with_absorb/4` "clears all player totem
/// slots only on lethal damage" and "queues a charm release when a player
/// dies while controlling a unit": a blow a shaman lives through leaves
/// their totems standing and their charmed creature theirs; the killing
/// blow takes the totems down and lets the creature go, as mangos's
/// `SetDeathState(JUST_DIED)` (`UnsummonAllTotems`, `BreakCharmOutgoing`).
#[tokio::test]
async fn a_blow_lived_through_keeps_totems_and_a_charm_and_death_ends_both() {
    let realm = fast_realm(controls);
    let (_c, a, map) = human(&realm, "P7C1", "Totemic", Class::Warrior).await;
    let kobold = on(&map, move |m| {
        kit::ready(m, a, 60, &[]);
        spells::cast_triggered(m, a, TOTEM_SPELL, Some(a));
        let kobold = kit::dummy(m, a, 60, 5.0);
        kit::apply(m, a, kobold, CHARM);
        kobold
    })
    .await;
    let first = on(&map, move |m| controlled(m, a, kobold)).await;
    assert_eq!(
        first,
        (1, Some(kobold), true),
        "a totem up, a creature charmed"
    );
    let other = on(&map, move |m| kit::dummy(m, a, 60, 8.0)).await;
    on(&map, move |m| combat::deal_damage(m, other, a, 10)).await;
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    let hurt = on(&map, move |m| {
        (
            controlled(m, a, kobold),
            m.players[&a].death == crate::game::map::death::Death::Alive,
        )
    })
    .await;
    assert_eq!(hurt, (first, true), "a blow lived through takes nothing");
    on(&map, move |m| {
        let health = kit::health(m, a);
        combat::deal_damage(m, other, a, health);
    })
    .await;
    until(&map, "the totem down and the creature let go", move |m| {
        controlled(m, a, kobold) == (0, None, false)
    })
    .await;
}

/// Thistle Tea's `take_damage_with_absorb/4` "does not dismiss the pet on
/// non-lethal damage": a hunter's pet stays out through a blow its master
/// lives through (its going at the master's death is
/// `tests_p3_creature2::a_hunters_pet_keeps_its_mood_growth_stance_and_autocast_put_away_and_goes_at_its_masters_death`).
#[tokio::test]
async fn a_hunters_pet_stays_out_through_a_blow_its_master_lives_through() {
    let (_realm, _h, gh, map, pet, wolf) = crate::game::tests_pets::tamed().await;
    on(&map, move |m| combat::deal_damage(m, wolf, gh, 5)).await;
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    let out = on(&map, move |m| {
        (
            m.players[&gh].owned.pet,
            m.players[&gh].owned.unsummoned,
            m.creature(pet).is_some(),
        )
    })
    .await;
    assert_eq!(out, (Some(pet), false, true), "the pet stays out");
}
