//! Round 7, against Thistle Tea's `spell_magnet_test.exs`: what a totem
//! shrugs off.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, health, set_health};
use crate::game::data::spells::SpellEffect;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::map::spells;

/// A totem of a hundred health in the first slot, its creature, a heal of
/// twenty, and a shaman's totem heal of twenty (Healing Stream's family
/// flags).
const TOTEM_SPELL: u32 = 997_901;
const TOTEM: u32 = 997_950;
const HEAL: u32 = 997_902;
const TOTEM_HEAL: u32 = 997_903;

fn data(d: &mut GameData) {
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
    let heal = |id: u32| {
        let mut s = bare(id, 3);
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 10,
                base_points: 19,
                base_dice: 1,
                target_a: 21,
                ..Default::default()
            },
        );
        s
    };
    let mut totem_heal = heal(TOTEM_HEAL);
    totem_heal.family_name = 11;
    totem_heal.family_flags = 0x2000;
    insert(d, [totem, heal(HEAL), totem_heal]);
}

/// A totem takes no one's heal, its owner's neither, but its own and a
/// shaman's totem spell's (Thistle Tea's "totem regeneration immunity
/// preserves self casts and Shaman totem healing").
#[tokio::test]
async fn a_totem_is_healed_only_by_itself_and_its_shamans_totems() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7TT1", "Totemic", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            spells::cast_triggered(m, a, TOTEM_SPELL, Some(a));
            let totem = m.players[&a].owned.totems[0].expect("a totem");
            set_health(m, totem, 50);
            spells::cast_triggered(m, a, HEAL, Some(totem));
            let owners = health(m, totem);
            spells::cast_triggered(m, totem, HEAL, Some(totem));
            let own = health(m, totem);
            spells::cast_triggered(m, a, TOTEM_HEAL, Some(totem));
            (owners, own, health(m, totem))
        })
        .await
        .unwrap();
    assert_eq!(out, (50, 70, 90));
}
