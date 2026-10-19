//! Round 7, against Thistle Tea's `spell_effect_test.exs`: a leap, extra
//! attacks, an item filled from a corpse, a pet's threat, a drain of
//! happiness, and Shatter.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, power};
use crate::game::data::spells::SpellEffect;
use crate::game::data::{GameData, ItemTemplate};
use crate::game::fields::*;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, mods::Mods, outcome};

/// Blink's kind (twenty yards forward); an extra attack's; Filling Empty
/// Jar's kind (an item from an enemy's corpse); Growl's kind (320
/// threat); Dismiss Pet's drain of 50,000 happiness; a fireball, a frost
/// root, Shatter's top rank (the override 913) and fifty more crit.
const LEAP: u32 = 997_401;
const EXTRA_ATTACK: u32 = 997_402;
const FILL_JAR: u32 = 997_403;
const GROWL: u32 = 997_404;
const DRAIN_HAPPINESS: u32 = 997_405;
const FIREBALL: u32 = 997_406;
const FROST_ROOT: u32 = 997_407;
const SHATTER: u32 = 997_408;
const SHARP: u32 = 997_409;
const JAR: u32 = 11_947;
const RADIUS_20: u32 = 999;

fn data(d: &mut GameData) {
    d.spells.radii.insert(RADIUS_20, 20.0);
    let mut leap = bare(LEAP, 6);
    effect(
        &mut leap,
        0,
        SpellEffect {
            effect: spells::EFFECT_LEAP,
            target_a: 1,
            radius_index: RADIUS_20,
            ..Default::default()
        },
    );
    let mut extra = bare(EXTRA_ATTACK, 0);
    effect(
        &mut extra,
        0,
        SpellEffect {
            effect: 19,
            base_points: 0,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut jar = bare(FILL_JAR, 0);
    jar.attributes[2] |= crate::game::data::spells::ATTR_EX2_ALLOW_DEAD_TARGET;
    effect(
        &mut jar,
        0,
        SpellEffect {
            effect: spells::EFFECT_CREATE_ITEM,
            base_points: 0,
            base_dice: 1,
            item_type: u64::from(JAR),
            target_a: 6,
            target_b: 1,
            ..Default::default()
        },
    );
    let mut growl = bare(GROWL, 0);
    effect(
        &mut growl,
        0,
        SpellEffect {
            effect: 63,
            base_points: 319,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut drain = bare(DRAIN_HAPPINESS, 0);
    effect(
        &mut drain,
        0,
        SpellEffect {
            effect: 8,
            base_points: 49_999,
            base_dice: 1,
            misc_value: 4,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut fireball = bare(FIREBALL, 2);
    fireball.family_name = 3;
    effect(&mut fireball, 0, damage(100));
    let mut root = bare(FROST_ROOT, 4);
    root.duration_index = LASTS_LONG;
    effect(&mut root, 0, aura(AURA_MOD_ROOT, 0, 0, 6));
    let mut shatter = bare(SHATTER, 4);
    shatter.family_name = 3;
    shatter.duration_index = LASTS_LONG;
    effect(
        &mut shatter,
        0,
        aura(AURA_OVERRIDE_CLASS_SCRIPTS, 913, 0, 1),
    );
    let mut sharp = bare(SHARP, 0);
    sharp.duration_index = LASTS_LONG;
    effect(&mut sharp, 0, aura(AURA_MOD_SPELL_CRIT_CHANCE, 0, 60, 1));
    insert(
        d,
        [
            leap, extra, jar, growl, drain, fireball, root, shatter, sharp,
        ],
    );
    d.items.insert(
        JAR,
        ItemTemplate {
            entry: JAR,
            class: 12,
            name: "Filled Cursed Ooze Jar".into(),
            display_id: 1000,
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
}

/// Blink's kind takes its caster its radius forward, facing as they were
/// (Thistle Tea's "leap effect emits a teleport event ahead of the
/// caster"; mangos's `EffectLeap`); an extra attack's effect owes its
/// caster a swing (its "add-extra-attacks effects queue a batch").
#[tokio::test]
async fn a_leap_goes_its_radius_forward_and_an_extra_attack_is_owed() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SE1", "Leaper", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let from = m.players[&a].position();
            let o = m.players[&a].movement.orientation;
            spells::cast_triggered(m, a, LEAP, Some(a));
            let to = m.players[&a].position();
            let owed = m.combat(a).map_or(0, |c| c.extra_attacks);
            spells::cast_triggered(m, a, EXTRA_ATTACK, Some(a));
            (
                from,
                o,
                to,
                m.players[&a].movement.orientation,
                owed,
                m.combat(a).map_or(0, |c| c.extra_attacks),
            )
        })
        .await
        .unwrap();
    let (from, o, to, facing, before, after) = out;
    assert!(
        (to.x - (from.x + 20.0 * o.cos())).abs() < 0.01,
        "{from:?} {to:?}"
    );
    assert!(
        (to.y - (from.y + 20.0 * o.sin())).abs() < 0.01,
        "{from:?} {to:?}"
    );
    assert!((facing - o).abs() < 1e-4);
    assert_eq!((before, after), (0, 1));
}

/// An item filled from an enemy's corpse is its caster's (Thistle Tea's
/// "an item filled from an enemy corpse is created for the caster, not
/// the corpse").
#[tokio::test]
async fn an_item_filled_from_a_corpse_is_its_casters() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SE2", "Filler", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[FILL_JAR]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            crate::game::map::combat::deal_damage(m, a, kobold, 2_000_000);
            let dead = !m.unit_view(kobold).is_some_and(|u| u.alive);
            spells::cast_triggered(m, a, FILL_JAR, Some(kobold));
            (dead, m.players[&a].carried_count_of(JAR))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, 1));
}

/// Growl's threat is its caster's, a pet's, not its owner's (Thistle Tea's
/// "growl adds threat for the pet rather than its owner"); a drain of
/// happiness takes it and gives its caster nothing, of mana or of
/// happiness (its "dismiss pet drains happiness without granting mana").
#[tokio::test]
async fn a_pets_growl_is_its_own_threat_and_a_happiness_drain_gives_nothing() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SE3", "Owner", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let pet = kit::dummy(m, a, 60, 2.0);
            crate::game::map::ai::add_threat(m, kobold, a, 10.0);
            spells::cast_triggered(m, pet, GROWL, Some(kobold));
            let threat = |g| {
                m.creature(kobold)
                    .and_then(|c| c.threat.iter().find(|(t, _)| *t == g).map(|(_, v)| *v))
            };
            let threats = (threat(a), threat(pet));
            if let Some(f) = m.unit_fields_mut(kobold) {
                f.set_u32(UNIT_FIELD_MAXPOWER1 + 4, 1_050_000);
                f.set_u32(UNIT_FIELD_POWER1 + 4, 166_500);
            }
            let (mana, happiness) = (power(m, a, 0), power(m, a, 4));
            spells::cast_triggered(m, a, DRAIN_HAPPINESS, Some(kobold));
            (
                threats,
                power(m, kobold, 4),
                power(m, a, 0) - mana,
                power(m, a, 4) - happiness,
            )
        })
        .await
        .unwrap();
    assert_eq!(out, ((Some(10.0), Some(320.0)), 116_500, 0, 0));
}

/// Shatter's crit chance (its override, fifty for the top rank) counts
/// against a target frozen by frost, not one free (Thistle Tea's
/// "Shatter raises spell crit chance against a frozen target").
#[tokio::test]
async fn shatter_adds_its_crit_against_the_frozen() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SE4", "Shatterer", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[FIREBALL]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, a, a, SHATTER);
            kit::apply(m, a, a, SHARP);
            let s = m.realm.data.spells.get(FIREBALL).unwrap().clone();
            let crit =
                |m: &crate::game::map::Map| outcome::roll_crit(m, a, kobold, &s, &Mods::default());
            let free = crit(m);
            kit::apply(m, a, kobold, FROST_ROOT);
            let frozen = (outcome::frozen(m, kobold), crit(m));
            let before = kit::health(m, kobold);
            kit::cast_fresh(m, a, FIREBALL, at(kobold));
            (free, frozen, before - kit::health(m, kobold))
        })
        .await
        .unwrap();
    let (free, frozen, dealt) = out;
    assert!(!free, "not against the free");
    assert_eq!(frozen, (true, true), "against the frozen");
    assert!(dealt >= 150, "a crit's damage: {dealt}");
}
