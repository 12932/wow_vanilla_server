//! Phase 3's combat tests, part eleven (Thistle Tea's
//! `weapon_bonuses_test.exs`): crit and hit an aura gives for one kind of
//! weapon, counted for the hand that holds it; generic stacks for both;
//! disarm and feral forms taking the weapon's away.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::tests_p3_combat::{self as base, *};

const DAGGER: u32 = 63_101;
const BOW: u32 = 63_102;

fn data() -> GameData {
    let mut d = base::data();
    // A human "druid", for the forms.
    d.player_level_stats
        .insert((1, 11, 60), [140, 0, 20, 20, 20]);
    d.player_class_stats.insert((11, 60), (1000, 1000));
    for (entry, subclass, inventory_type) in [(DAGGER, 15, 13), (BOW, 2, 15)] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class: 2,
                subclass,
                name: format!("Weapon {entry}"),
                display_id: 1000,
                inventory_type,
                delay: 2000,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    d
}

/// An aura of `kind` holding `amount`, for weapons of `subclasses` (none:
/// any), `stacks` deep, on `g`.
fn weapon_aura(
    m: &mut Map,
    g: Guid,
    id: u32,
    kind: u32,
    amount: i32,
    subclasses: Option<i32>,
    stacks: u32,
) {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let s = SpellTemplate {
        id,
        name: format!("Talent {id}"),
        equipped_item_class: if subclasses.is_some() { 2 } else { -1 },
        equipped_item_subclass_mask: subclasses.unwrap_or(0),
        effects,
        ..Default::default()
    };
    let _ = auras::apply_spell(m, g, g, &s, &[(0, amount)], auras::Context::default());
    if let Some(h) =
        auras::auras_mut(m, g).and_then(|a| a.holders.iter_mut().find(|h| h.spell == id))
    {
        h.stacks = stacks;
    }
    auras::recalc(m, g);
}

fn crits(m: &Map, g: Guid) -> (f32, f32) {
    let d = m.players[&g].derived;
    (d.crit, d.ranged_crit)
}

#[tokio::test]
async fn weapon_crit_talents_count_for_the_weapon_they_name_in_its_own_hand() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, 15);
            equip(m, ga, DAGGER, OFF_HAND);
            equip(m, ga, BOW, 17);
            let base = crits(m, ga);
            // Five for daggers, three for bows.
            weapon_aura(m, ga, 90_400, AURA_MOD_CRIT_PERCENT, 5, Some(1 << 15), 1);
            weapon_aura(m, ga, 90_401, AURA_MOD_CRIT_PERCENT, 3, Some(1 << 2), 1);
            let talents = crits(m, ga);
            // The dagger in the main hand: its five count.
            equip(m, ga, DAGGER, 15);
            let swapped = crits(m, ga);
            auras::remove_spell(m, ga, 90_400, None);
            let dagger_base = crits(m, ga);
            weapon_aura(m, ga, 90_400, AURA_MOD_CRIT_PERCENT, 5, Some(1 << 15), 1);
            // Disarmed, the main hand's talent goes.
            weapon_aura(m, ga, 90_402, AURA_MOD_DISARM, 0, None, 1);
            let disarmed = crits(m, ga);
            auras::remove_spell(m, ga, 90_402, None);
            auras::remove_spell(m, ga, 90_400, None);
            auras::remove_spell(m, ga, 90_401, None);
            // Two for anything, twice: for both.
            let plain = crits(m, ga);
            weapon_aura(m, ga, 90_403, AURA_MOD_CRIT_PERCENT, 2, None, 2);
            let generic = crits(m, ga);
            (
                base,
                talents,
                swapped,
                dagger_base,
                disarmed,
                (plain, generic),
            )
        })
        .await
        .unwrap();
    let (base, talents, swapped, dagger_base, disarmed, generic) = r;
    let close = |a: f32, b: f32| (a - b).abs() < 0.001;
    assert!(
        close(talents.0, base.0),
        "a sword's hand: {talents:?} {base:?}"
    );
    assert!(close(talents.1, base.1 + 3.0), "the bow's: {talents:?}");
    assert!(
        close(swapped.0, dagger_base.0 + 5.0),
        "{swapped:?} {dagger_base:?}"
    );
    assert!(disarmed.0 < swapped.0 - 4.9, "{disarmed:?}");
    let (plain, generic) = generic;
    assert!(
        close(generic.0, plain.0 + 4.0) && close(generic.1, plain.1 + 4.0),
        "{generic:?} {plain:?}"
    );
}

#[tokio::test]
async fn weapon_hit_talents_count_for_their_weapon_and_a_feral_form_takes_them_away() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            // A druid, for the forms.
            m.players.get_mut(&ga).unwrap().row.class = 11;
            equip(m, ga, DAGGER, 15);
            weapon_aura(m, ga, 90_410, AURA_MOD_HIT_CHANCE, 5, Some(1 << 15), 1);
            weapon_aura(m, ga, 90_411, AURA_MOD_CRIT_PERCENT, 5, Some(1 << 15), 1);
            weapon_aura(m, ga, 90_412, AURA_MOD_CRIT_PERCENT, 3, None, 1);
            let armed = (m.players[&ga].derived.hit, crits(m, ga).0);
            // In cat form, the paw holds no dagger: only the generic three.
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, FORM_CAT);
            auras::recalc(m, ga);
            let feral = (m.players[&ga].derived.hit, crits(m, ga).0);
            (armed, feral)
        })
        .await
        .unwrap();
    assert_eq!(r.0.0, 5.0);
    assert_eq!(r.1.0, 0.0, "no dagger in a paw");
    assert!((r.0.1 - r.1.1 - 5.0).abs() < 0.5, "{r:?}");
}
