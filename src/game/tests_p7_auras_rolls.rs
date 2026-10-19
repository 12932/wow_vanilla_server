//! Round 7's aura tests that turn on a roll, ported from Thistle Tea's
//! `dispel_test.exs`, `dispel_resistance_test.exs`,
//! `mechanic_resistance_test.exs` and `percent_damage_test.exs`
//! (docs/parity/matrix.csv): the rolls lined up through
//! `MapSpells::queued_rolls`, in hundredths of a percent.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{CUSTOM_NEGATIVE, CUSTOM_POSITIVE, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::spells::mods;
use crate::game::map::{Map, auras};
use crate::game::tests_p3_combat::{BRUTE, level, setup_with, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_spells_kit::pass;

fn spell(id: u32, effect: u32, aura: u32, misc: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect,
        aura,
        misc_value: misc,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn put(m: &mut Map, caster: Guid, target: Guid, id: u32, amount: i32) {
    let s = m.realm.data.spells.spells[&id].clone();
    auras::apply_spell(
        m,
        caster,
        target,
        &s,
        &[(0, amount)],
        auras::Context::default(),
    )
    .expect("put on");
}

fn queue(m: &Map, rolls: &[i32]) {
    m.spells.queued_rolls.lock().unwrap().extend(rolls);
}

fn has(m: &Map, g: Guid, id: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(id))
}

/// Warlock family spells (5), their first flag (`1`) or a flag of the high
/// word (`1 << 32`): an affliction, and the caster's modifiers of the
/// chance it resists a dispel.
const AFFLICTION: u32 = 97_600;
const HIGH_AFFLICTION: u32 = 97_601;
const FLAT_RESIST: u32 = 97_602;
const PCT_RESIST: u32 = 97_603;
const HIGH_RESIST: u32 = 97_604;
/// A dispel of magic, and Devour Magic with its heal.
const DISPEL: u32 = 97_605;
const DEVOUR_MAGIC: u32 = 19505;
const DEVOUR_HEAL: u32 = 19658;
/// A mechanic resistance and an effect of the mechanic.
const STUN_RESIST: u32 = 97_606;
const GOUGE_RESIST: u32 = 97_607;
const STUNNING: u32 = 97_608;
/// A tenth of the target's health in fire a second.
const SEARING: u32 = 97_609;
const WARD: u32 = 97_610;

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut affliction = spell(AFFLICTION, 6, auras::AURA_MOD_STAT, 0);
    affliction.family_name = 5;
    affliction.family_flags = 1;
    affliction.dispel = 1;
    affliction.custom_flags = CUSTOM_NEGATIVE;
    affliction.stack_amount = 3;
    affliction.effects[0].amplitude = 0;
    let mut high = affliction.clone();
    high.id = HIGH_AFFLICTION;
    high.family_flags = 1 << 32;
    let modifier = |id: u32, kind: u32, mask: u64| {
        let mut s = spell(id, 6, kind, mods::OP_RESIST_DISPEL_CHANCE as i32);
        s.family_name = 5;
        s.effects[0].item_type = mask;
        s.custom_flags = CUSTOM_POSITIVE;
        s
    };
    let mut dispel = spell(DISPEL, 38, 0, 1);
    dispel.effects[0].target_a = 1;
    let mut devour = dispel.clone();
    devour.id = DEVOUR_MAGIC;
    devour.script_name = "spell_warlock_devour_magic".into();
    let mut heal = spell(DEVOUR_HEAL, 10, 0, 0);
    heal.effects[0].base_points = 99;
    let mut stunning = spell(STUNNING, 6, auras::AURA_MOD_STUN, 0);
    stunning.effects[0].mechanic = 12;
    stunning.custom_flags = CUSTOM_NEGATIVE;
    let mut searing = spell(SEARING, 6, auras::AURA_PERIODIC_DAMAGE_PERCENT, 0);
    searing.school = 2;
    searing.effects[0].amplitude = 1000;
    searing.custom_flags = CUSTOM_NEGATIVE;
    for s in [
        affliction,
        high,
        modifier(FLAT_RESIST, auras::AURA_ADD_FLAT_MODIFIER, 1),
        modifier(PCT_RESIST, auras::AURA_ADD_PCT_MODIFIER, 1),
        modifier(HIGH_RESIST, auras::AURA_ADD_FLAT_MODIFIER, 1 << 32),
        dispel,
        devour,
        heal,
        spell(STUN_RESIST, 6, auras::AURA_MOD_MECHANIC_RESISTANCE, 12),
        spell(GOUGE_RESIST, 6, auras::AURA_MOD_MECHANIC_RESISTANCE, 7),
        stunning,
        searing,
        spell(WARD, 6, auras::AURA_SCHOOL_ABSORB, 0x7F),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Thistle Tea's `DispelResistance` "combines flat and percent modifiers
/// and bounds the result": the chance an affliction resists a dispel is
/// its caster's flat modifiers by their percent ones (40 and half again,
/// 60), none below nothing and certain at most, from modifiers of the
/// spell's own family flags only, the high word's too (mangos's
/// `ApplySpellMod` on a chance of 0).
#[tokio::test]
async fn a_dispel_resistance_is_its_flat_chance_by_its_percent_bounded() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warlock).await;
    let out = on(&map, move |m| {
        let chance = |m: &Map, spell: u32| {
            let s = m.realm.data.spells.spells[&spell].clone();
            mods::for_spell(m, ga, &s).dispel_resistance()
        };
        let with = |m: &mut Map, mods: &[(u32, i32)], spell: u32| {
            auras::remove_where(m, ga, |h| {
                matches!(h.spell, FLAT_RESIST | PCT_RESIST | HIGH_RESIST)
            });
            for (id, amount) in mods {
                put(m, ga, ga, *id, *amount);
            }
            chance(m, spell)
        };
        [
            with(m, &[(FLAT_RESIST, 40), (PCT_RESIST, 50)], AFFLICTION),
            with(m, &[(FLAT_RESIST, 200)], AFFLICTION),
            with(m, &[(FLAT_RESIST, -10)], AFFLICTION),
            with(m, &[(HIGH_RESIST, 40)], AFFLICTION),
            with(m, &[(HIGH_RESIST, 40)], HIGH_AFFLICTION),
        ]
    })
    .await;
    assert_eq!(out, [60.0, 100.0, 0.0, 0.0, 40.0]);
}

/// Thistle Tea's `Dispel` "rolls each stack separately and keeps failed
/// stacks on partial success": a dispel of three at an affliction of
/// three stacks, its caster's 30% to resist, rolls for each stack: the one
/// that comes under 30% stays, the two others go; the stack left keeps
/// its time.
#[tokio::test]
async fn a_dispel_rolls_each_stack_and_keeps_those_that_resist() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Priest).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        put(m, brute, brute, FLAT_RESIST, 30);
        put(m, brute, ga, AFFLICTION, -5);
        auras::set_stacks(m, ga, AFFLICTION, brute, None, 3);
        let expires = |m: &Map| {
            auras::auras(m, ga)
                .and_then(|a| a.holders.iter().find(|h| h.spell == AFFLICTION))
                .map(|h| (h.stacks, h.expires))
        };
        let before = expires(m);
        queue(m, &[2999, 3000, 9999]);
        let s = m.realm.data.spells.spells[&DISPEL].clone();
        crate::game::map::spells::scripts::cast_template_with_points(
            m,
            ga,
            s,
            ga,
            [Some(3), None, None],
        );
        (before, expires(m))
    })
    .await;
    let (before, after) = out;
    assert_eq!(before.map(|b| b.0), Some(3));
    assert_eq!(
        after,
        before.map(|(_, t)| (1, t)),
        "one stack, its time kept"
    );
}

/// Thistle Tea's `Dispel` "resistance emits failure feedback without
/// triggering Devour Magic healing": Devour Magic at an affliction whose
/// caster holds it on for certain takes nothing, its caster is told the
/// dispel failed (`SMSG_DISPEL_FAILED`), and it heals no one.
#[tokio::test]
async fn devour_magic_resisted_is_told_and_heals_nothing() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warlock).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        put(m, brute, brute, FLAT_RESIST, 100);
        put(m, brute, ga, AFFLICTION, -5);
        let f = m.unit_fields_mut(ga).unwrap();
        f.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
        f.set_u32(UNIT_FIELD_HEALTH, 500);
        let s = m.realm.data.spells.spells[&DEVOUR_MAGIC].clone();
        crate::game::map::spells::scripts::cast_template_with_points(
            m,
            ga,
            s,
            ga,
            [Some(1), None, None],
        );
        (
            has(m, ga, AFFLICTION),
            m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_HEALTH),
        )
    })
    .await;
    assert_eq!(out, (true, 500), "(still afflicted, unhealed)");
    let failed = a
        .wait_for("SMSG_DISPEL_FAILED", |m| match m {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_DISPEL_FAILED(f) => {
                Some(f.spells.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(failed, [AFFLICTION]);
}

/// Thistle Tea's `MechanicResistance` "uses an exclusive percentage
/// boundary": a stun against 25% resistance to stuns is resisted by a roll
/// under 25% (24.99), not by one at it; a resistance to another mechanic
/// resists nothing of it.
#[tokio::test]
async fn a_mechanic_resistance_resists_below_its_chance_only() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let stun = m.realm.data.spells.spells[&STUNNING].clone();
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        put(m, brute, brute, STUN_RESIST, 25);
        let other = spawn(m, ga, BRUTE, 20, 4.0);
        put(m, other, other, GOUGE_RESIST, 100);
        let resisted = |m: &Map, g: Guid, roll: i32| {
            queue(m, &[roll]);
            let r = crate::game::map::spells::outcome::effect_resisted(m, g, &stun, 0);
            m.spells.queued_rolls.lock().unwrap().clear();
            r
        };
        [
            resisted(m, brute, 2499),
            resisted(m, brute, 2500),
            resisted(m, other, 0),
        ]
    })
    .await;
    assert_eq!(out, [true, false, false]);
}

/// Thistle Tea's `PercentDamage` "school resistance reduces damage before
/// absorption": a tick of a tenth of a unit's health in fire, against its
/// fire resistance, has a share resisted (a roll at the bottom), the rest
/// dealt; what it is shielded from comes off what is left.
#[tokio::test]
async fn a_percent_ticks_resisted_share_comes_off_before_its_shield() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let out = on(&map, move |m| {
        level(m, ga, 60);
        let brute = spawn(m, ga, BRUTE, 60, 3.0);
        put(m, brute, brute, WARD, 10);
        put(m, ga, brute, SEARING, 10);
        // The pools and resistance after the auras (which work them out
        // again).
        let f = m.unit_fields_mut(brute).unwrap();
        f.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
        f.set_u32(UNIT_FIELD_HEALTH, 1000);
        f.set_u32(UNIT_FIELD_RESISTANCES + 2, 300);
        m.spells.rolls = Some(true);
        queue(m, &[0]);
        pass(m, 1000);
        m.spells.rolls = None;
        let health = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
        (health, has(m, brute, WARD))
    })
    .await;
    let (health, warded) = out;
    let lost = 1000 - health;
    // A hundred a tick: a quarter, a half or three quarters resisted, ten
    // of the rest shielded, the rest dealt.
    assert!([15, 40, 65].contains(&lost), "a share resisted: {lost}");
    assert!(!warded, "the shield took its ten of what was left");
}
