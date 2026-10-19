//! Phase 3, round 4, against Thistle Tea's `ai/wand_test.exs`: a wand's
//! shot of its own school without attack power, Wand Specialization on the
//! wand and not on spells, its school's stacked percentages on the shot
//! and its shown damage, its 150% crit past armor, the target's immunity,
//! and its repeat: stopped by another cast or by its target going out of
//! range, and toggled without losing the weapon's time.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::ranged::{self, SPELL_SHOOT};
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, auras, derive};
use crate::game::tests_p3_combat::*;

pub(crate) const WAND: u32 = 98_301;
const WAND_SPECIALIZATION: u32 = 98_302;
const FIRE_10: u32 = 98_303;
pub(crate) const SURE: u32 = 98_304;
const FIREBALL: u32 = 98_305;

fn effect(aura: u32, amount: i32, misc: i32) -> SpellEffect {
    SpellEffect {
        effect: 6,
        aura,
        base_points: amount - 1,
        base_dice: 1,
        misc_value: misc,
        target_a: 1,
        ..Default::default()
    }
}

pub(crate) fn world() -> GameData {
    let mut d = data();
    let mut damages = [(0.0, 0.0, 0); 5];
    damages[0] = (100.0, 100.0, 2);
    d.items.insert(
        WAND,
        ItemTemplate {
            entry: WAND,
            class: 2,
            subclass: 19,
            name: "Wand".into(),
            display_id: 1000,
            inventory_type: 26,
            delay: 1_500,
            damages,
            stackable: 1,
            ..Default::default()
        },
    );
    let mut shoot = SpellTemplate {
        id: SPELL_SHOOT,
        name: "Shoot".into(),
        dmg_class: 1,
        range_index: 1,
        equipped_item_class: 2,
        equipped_item_subclass_mask: 1 << 19,
        ..Default::default()
    };
    shoot.attributes[0] = spells_data::ATTR_RANGED;
    shoot.attributes[2] = spells_data::ATTR_EX2_AUTO_REPEAT;
    shoot.effects[0] = SpellEffect {
        effect: 58,
        target_a: 6,
        base_points: -1,
        ..Default::default()
    };
    d.spells.spells.insert(SPELL_SHOOT, shoot);
    // Wand Specialization: a quarter more, on a wand alone.
    let mut spec = SpellTemplate {
        id: WAND_SPECIALIZATION,
        name: "Wand Specialization".into(),
        equipped_item_class: 2,
        equipped_item_subclass_mask: 1 << 19,
        ..Default::default()
    };
    spec.effects[0] = effect(auras::AURA_MOD_DAMAGE_PERCENT_DONE, 25, 126);
    d.spells.spells.insert(WAND_SPECIALIZATION, spec);
    let mut fire = SpellTemplate {
        id: FIRE_10,
        name: "Fire Power".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    fire.effects[0] = effect(auras::AURA_MOD_DAMAGE_PERCENT_DONE, 10, 4);
    d.spells.spells.insert(FIRE_10, fire);
    // Sure to hit, never to crit.
    let mut sure = SpellTemplate {
        id: SURE,
        name: "Sure".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    sure.effects[0] = effect(auras::AURA_MOD_HIT_CHANCE, 100, 0);
    sure.effects[1] = effect(auras::AURA_MOD_SPELL_CRIT_CHANCE, -100, 0);
    d.spells.spells.insert(SURE, sure);
    // A fireball of two seconds.
    d.spells.cast_times.insert(6, 2_000);
    let mut fireball = SpellTemplate {
        id: FIREBALL,
        name: "Fireball".into(),
        school: 2,
        equipped_item_class: -1,
        range_index: 1,
        casting_time_index: 6,
        ..Default::default()
    };
    fireball.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(FIREBALL, fireball);
    d
}

use crate::game::data::spells as spells_data;

pub(crate) fn put(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6)
        .map(|i| (i, s.effects[i].base_points + 1))
        .collect();
    auras::apply_spell(m, g, g, &s, &effects, Default::default()).unwrap();
    derive::unit(m, g);
}

fn shown(m: &Map, g: Guid) -> (f32, f32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_f32(UNIT_FIELD_MINRANGEDDAMAGE),
        f.get_f32(UNIT_FIELD_MAXRANGEDDAMAGE),
    )
}

/// Alice, a mage at 60 with the wand, and a target in front of her
/// (armored past belief, parrying all).
pub(crate) async fn alice() -> (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    crate::game::map::MapHandle,
    Guid,
) {
    let (realm, a, g, map) = setup_with(world(), Class::Mage).await;
    let brute = map
        .query(move |m| {
            level(m, g, 60);
            equip(m, g, WAND, 17);
            m.players
                .get_mut(&g)
                .unwrap()
                .spells
                .extend([SPELL_SHOOT, FIREBALL]);
            put(m, g, SURE);
            let brute = spawn(m, g, BRUTE, 60, 10.0);
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 10_000);
            aura_of(m, brute, 98_310, auras::AURA_MOD_PARRY_PERCENT, 0, 100);
            brute
        })
        .await
        .unwrap();
    (realm, a, g, map, brute)
}

/// Shoot at `target` and fire the first shot now: the damage done.
pub(crate) fn shot(m: &mut Map, g: Guid, target: Guid) -> u32 {
    ranged::start(
        m,
        g,
        SPELL_SHOOT,
        Targets {
            unit: Some(target),
            ..Default::default()
        },
    );
    let now = m.now;
    if let Some(c) = m.combat_mut(g)
        && let Some(a) = c.auto_repeat.as_mut()
    {
        a.next_at = now;
    }
    let before = m.unit_fields(target).unwrap().get_u32(UNIT_FIELD_HEALTH);
    ranged::update(m);
    before - m.unit_fields(target).unwrap().get_u32(UNIT_FIELD_HEALTH)
}

#[tokio::test]
async fn a_wand_shoots_its_own_school_and_damage_with_its_specialization_but_spells_dont_take_it() {
    let (_realm, _a, g, map, brute) = alice().await;
    let r = map
        .query(move |m| {
            let plain = shown(m, g);
            let landed = shot(m, g, brute);
            ranged::cancel(m, g, false);
            m.combat_mut(g).unwrap().ranged_ready_at = None;
            put(m, g, WAND_SPECIALIZATION);
            let specialized = shown(m, g);
            let landed_specialized = shot(m, g, brute);
            ranged::cancel(m, g, false);
            m.combat_mut(g).unwrap().ranged_ready_at = None;
            let fireball = m.realm.data.spells.get(FIREBALL).unwrap().clone();
            let spell_share = (
                spells::outcome::done_multiplier(m, g, brute, &fireball),
                m.unit_fields(g)
                    .unwrap()
                    .get_f32(PLAYER_FIELD_MOD_DAMAGE_DONE_PCT + 2),
            );
            auras::remove_spell(m, g, WAND_SPECIALIZATION, None);
            // Ten percent fire, three stacks.
            put(m, g, FIRE_10);
            for h in auras::auras_mut(m, g)
                .unwrap()
                .holders
                .iter_mut()
                .filter(|h| h.spell == FIRE_10)
            {
                h.stacks = 3;
            }
            derive::unit(m, g);
            let fire = shown(m, g);
            let landed_fire = shot(m, g, brute);
            (
                (plain, landed),
                (specialized, landed_specialized),
                spell_share,
                (fire, landed_fire),
            )
        })
        .await
        .unwrap();
    let ((plain, landed), (specialized, landed_specialized), spell_share, (fire, landed_fire)) = r;
    // No attack power on it, armor past belief no matter: fire.
    assert_eq!(plain, (100.0, 100.0));
    assert_eq!(landed, 100);
    assert_eq!(specialized, (125.0, 125.0));
    assert_eq!(landed_specialized, 125);
    assert_eq!(
        spell_share,
        (1.0, 1.0),
        "Wand Specialization isn't a spell's"
    );
    assert!((fire.0 - 130.0).abs() < 0.01, "{fire:?}");
    assert_eq!(landed_fire, 130);
}

#[tokio::test]
async fn a_wands_shot_on_one_immune_to_its_school_does_nothing() {
    let (_realm, _a, g, map, brute) = alice().await;
    let r = map
        .query(move |m| {
            aura_of(m, brute, 98_311, auras::AURA_SCHOOL_IMMUNITY, 4, 1);
            shot(m, g, brute)
        })
        .await
        .unwrap();
    assert_eq!(r, 0);
}

#[tokio::test]
async fn another_cast_or_a_target_out_of_range_stops_a_wand_and_shooting_again_waits_for_the_weapon()
 {
    let (_realm, _a, g, map, brute) = alice().await;
    let r = map
        .query(move |m| {
            let repeating = |m: &Map| m.combat(g).unwrap().auto_repeat.is_some();
            // A shot, stopped, started again: the weapon's 1.5 s still
            // stands from the shot.
            shot(m, g, brute);
            let fired_at = m.now;
            ranged::cancel(m, g, false);
            ranged::start(
                m,
                g,
                SPELL_SHOOT,
                Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
            );
            let next = m.combat(g).unwrap().auto_repeat.unwrap().next_at;
            let waits = next.duration_since(fired_at);
            // A fireball begun: the wand stops.
            let cast = spells::cast(
                m,
                g,
                FIREBALL,
                Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
                None,
            );
            let after_cast = repeating(m);
            spells::interrupt(m, g);
            // Out of range: stopped, not waiting.
            ranged::start(
                m,
                g,
                SPELL_SHOOT,
                Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
            );
            let p = m.players[&g].position();
            m.creature_mut(brute).unwrap().position = Vector3d { x: p.x + 40.0, ..p };
            let now = m.now;
            if let Some(c) = m.combat_mut(g)
                && let Some(a) = c.auto_repeat.as_mut()
            {
                a.next_at = now;
            }
            ranged::update(m);
            (waits, cast, after_cast, repeating(m))
        })
        .await
        .unwrap();
    let (waits, cast, after_cast, out_of_range) = r;
    assert_eq!(waits, Duration::from_millis(1_500));
    assert!(cast);
    assert!(!after_cast, "the cast stops the wand");
    assert!(!out_of_range, "out of range, it stops");
}
