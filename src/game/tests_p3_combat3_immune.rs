//! Phase 3's combat tests: immunity to damage (Thistle Tea's
//! `damage_immunity_test.exs`): by school mask, what pierces it, a swing
//! met by it, spells and ticks held off while heals land, and its auras
//! coming and going.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{
    ATTR_EX2_NO_SCHOOL_IMMUNITIES, ATTR_EX3_ALWAYS_HIT, ATTR_NO_IMMUNITIES, SpellDuration,
    SpellEffect, SpellTemplate,
};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, aura_of, forced_swing, spawn};
use crate::game::tests_p3_spells_kit::tick_now;

const BOLT: u32 = 95_600;
const BOLT_PIERCING: u32 = 95_601;
const BOLT_UNSTOPPABLE: u32 = 95_602;
const FIREBOLT: u32 = 95_603;
const ROOT: u32 = 95_604;
const HEAL: u32 = 95_605;
const DOT: u32 = 95_606;
const LEECH: u32 = 95_607;
const MANA_LEECH: u32 = 95_608;
const DOT_UNSTOPPABLE: u32 = 95_609;

const PHYSICAL: i32 = 1;
const FIRE: i32 = 4;

fn spell(id: u32, school: u32, effect: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        school,
        dmg_class: 1,
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 900,
        ..Default::default()
    };
    s.attributes[3] = ATTR_EX3_ALWAYS_HIT;
    s.effects[0] = effect;
    s
}

fn effect(effect: u32, aura: u32, points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        aura,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        amplitude: if aura == 0 || aura == 26 { 0 } else { 1000 },
        target_a,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.durations.insert(
        900,
        SpellDuration {
            base: 3000,
            per_level: 0,
            max: 3000,
        },
    );
    let mut add = |s: SpellTemplate| {
        d.spells.spells.insert(s.id, s);
    };
    add(spell(BOLT, 0, effect(2, 0, 20, 6)));
    let mut piercing = spell(BOLT_PIERCING, 0, effect(2, 0, 20, 6));
    piercing.attributes[2] = ATTR_EX2_NO_SCHOOL_IMMUNITIES;
    add(piercing);
    let mut unstoppable = spell(BOLT_UNSTOPPABLE, 0, effect(2, 0, 20, 6));
    unstoppable.attributes[0] = ATTR_NO_IMMUNITIES;
    add(unstoppable);
    add(spell(FIREBOLT, 2, effect(2, 0, 20, 6)));
    add(spell(ROOT, 0, effect(6, AURA_MOD_ROOT, 0, 6)));
    let mut heal = spell(HEAL, 1, effect(10, 0, 20, 6));
    heal.attributes[3] = 0;
    add(heal);
    add(spell(DOT, 0, effect(6, AURA_PERIODIC_DAMAGE, 20, 6)));
    add(spell(LEECH, 0, effect(6, AURA_PERIODIC_LEECH, 20, 6)));
    add(spell(
        MANA_LEECH,
        0,
        effect(6, AURA_PERIODIC_MANA_LEECH, 20, 6),
    ));
    let mut dot = spell(DOT_UNSTOPPABLE, 0, effect(6, AURA_PERIODIC_DAMAGE, 20, 6));
    dot.attributes[0] = ATTR_NO_IMMUNITIES;
    add(dot);
    d
}

async fn world() -> (crate::game::harness::Client, Guid, MapHandle) {
    let realm = crate::game::harness::realm_with(data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Mage).await;
    let map = realm.registry.online(g).unwrap().map();
    (a, g, map)
}

/// A creature of 100 health and 100 mana, 2 yards ahead.
fn target(m: &mut Map, g: Guid) -> Guid {
    let c = spawn(m, g, BRUTE, 20, 2.0);
    let f = m.unit_fields_mut(c).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 100);
    f.set_u32(UNIT_FIELD_HEALTH, 100);
    f.set_u32(UNIT_FIELD_MAXPOWER1, 100);
    f.set_u32(UNIT_FIELD_POWER1, 100);
    c
}

/// An immunity of `kind` for `mask` on `c`.
fn protect(m: &mut Map, c: Guid, id: u32, kind: u32, mask: i32) {
    aura_of(m, c, id, kind, mask, 0);
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn mana(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1))
}

fn hit(m: &mut Map, g: Guid, spell: u32, c: Guid) -> u32 {
    crate::game::map::spells::cast_triggered(m, g, spell, Some(c));
    100 - health(m, c)
}

#[tokio::test]
async fn an_immunity_to_damage_goes_by_its_school_mask_and_only_no_immunities_pierces_it() {
    let (_a, g, map) = world().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // Physical and fire, not frost: a physical bolt and a fire one
            // do nothing.
            let c = target(m, g);
            protect(m, c, 95_610, AURA_DAMAGE_IMMUNITY, PHYSICAL | FIRE);
            let a = auras::auras(m, c).unwrap();
            out.push(
                a.immune_by(AURA_DAMAGE_IMMUNITY, 1) as u32
                    + a.immune_by(AURA_DAMAGE_IMMUNITY, 4) as u32
                    + a.immune_by(AURA_DAMAGE_IMMUNITY, 16) as u32
                    + a.immune_by(AURA_SCHOOL_IMMUNITY, 1) as u32,
            );
            out.push(hit(m, g, BOLT, c));
            out.push(hit(m, g, FIREBOLT, c));
            // A spell piercing school immunities meets this one still.
            out.push(hit(m, g, BOLT_PIERCING, c));
            // It pierces a school immunity.
            let c = target(m, g);
            protect(m, c, 95_611, AURA_SCHOOL_IMMUNITY, PHYSICAL);
            out.push(hit(m, g, BOLT_PIERCING, c));
            // No immunities pierces both.
            for kind in [AURA_SCHOOL_IMMUNITY, AURA_DAMAGE_IMMUNITY] {
                let c = target(m, g);
                protect(m, c, 95_612, kind, PHYSICAL);
                out.push(hit(m, g, BOLT_UNSTOPPABLE, c));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [2, 0, 0, 0, 20, 20, 20]);
}

#[tokio::test]
async fn the_immune_take_neither_a_swing_nor_harm_nor_control_but_heals_and_other_schools_land() {
    let (_a, g, map) = world().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // A swing: nothing, and the shield is untouched.
            let c = spawn(m, g, BRUTE, 20, 2.0);
            protect(m, c, 95_610, AURA_DAMAGE_IMMUNITY, PHYSICAL);
            aura_of(m, c, 95_613, AURA_SCHOOL_ABSORB, PHYSICAL, 50);
            let before = health(m, c);
            forced_swing(m, g, c, Outcome::Hit);
            out.push(before - health(m, c));
            let shield = auras::auras(m, c).and_then(|a| {
                a.effects()
                    .find(|(_, x)| x.kind == AURA_SCHOOL_ABSORB)
                    .map(|(_, x)| x.left)
            });
            out.push(shield.unwrap_or(0) as u32);
            // A spell, a damage over time and a root: none takes; fire
            // does.
            let c = target(m, g);
            protect(m, c, 95_610, AURA_DAMAGE_IMMUNITY, PHYSICAL);
            out.push(hit(m, g, BOLT, c));
            crate::game::map::spells::cast_triggered(m, g, DOT, Some(c));
            crate::game::map::spells::cast_triggered(m, g, ROOT, Some(c));
            let a = auras::auras(m, c).unwrap();
            out.push(a.has_spell(DOT) as u32 + a.has_spell(ROOT) as u32);
            out.push(hit(m, g, FIREBOLT, c));
            // A heal lands.
            m.unit_fields_mut(c).unwrap().set_u32(UNIT_FIELD_HEALTH, 50);
            crate::game::map::spells::cast_triggered(m, g, HEAL, Some(c));
            out.push(health(m, c));
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [0, 50, 0, 0, 20, 70]);
}

#[tokio::test]
async fn ticks_already_on_pause_under_an_immunity_unless_they_pierce_it_and_resume_after() {
    let (_a, g, map) = world().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for spell in [DOT, LEECH, MANA_LEECH] {
                let c = target(m, g);
                crate::game::map::spells::cast_triggered(m, g, spell, Some(c));
                protect(m, c, 95_610, AURA_DAMAGE_IMMUNITY, PHYSICAL);
                tick_now(m, c, spell);
                let paused = (
                    health(m, c),
                    mana(m, c),
                    auras::auras(m, c).unwrap().has_spell(spell),
                );
                auras::remove_spell(m, c, 95_610, None);
                tick_now(m, c, spell);
                out.push((paused, (health(m, c), mana(m, c))));
            }
            let c = target(m, g);
            crate::game::map::spells::cast_triggered(m, g, DOT_UNSTOPPABLE, Some(c));
            protect(m, c, 95_610, AURA_DAMAGE_IMMUNITY, PHYSICAL);
            tick_now(m, c, DOT_UNSTOPPABLE);
            out.push(((health(m, c), mana(m, c), true), (0, 0)));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            ((100, 100, true), (80, 100)),
            ((100, 100, true), (80, 100)),
            ((100, 100, true), (100, 80)),
            ((80, 100, true), (0, 0)),
        ]
    );
}

#[tokio::test]
async fn an_immunity_lasts_while_any_of_its_auras_does_and_ends_with_the_last_or_a_death() {
    let (_a, g, map) = world().await;
    let got = map
        .query(move |m| {
            let c = target(m, g);
            protect(m, c, 95_610, AURA_DAMAGE_IMMUNITY, PHYSICAL);
            protect(m, c, 95_614, AURA_DAMAGE_IMMUNITY, PHYSICAL);
            auras::remove_spell(m, c, 95_610, None);
            let one_left = hit(m, g, BOLT, c);
            auras::remove_spell(m, c, 95_614, None);
            let none_left = hit(m, g, BOLT, c);
            // Killed by fire, it is immune no more.
            let d = target(m, g);
            protect(m, d, 95_610, AURA_DAMAGE_IMMUNITY, PHYSICAL);
            combat::deal_damage(m, g, d, 1000);
            let after_death =
                auras::auras(m, d).is_some_and(|a| a.immune_by(AURA_DAMAGE_IMMUNITY, 1));
            (one_left, none_left, after_death)
        })
        .await
        .unwrap();
    assert_eq!(got, (0, 20, false));
}
