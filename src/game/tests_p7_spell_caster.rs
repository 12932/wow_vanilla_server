//! Round 7, against Thistle Tea's `caster_state_test.exs`,
//! `cast_validation_test.exs`'s caster state and
//! `casting_immunity_test.exs`: what frees a caster to cast out of a stun,
//! a fear or a confusion (mangos-classic's `CheckCasterAuras` and
//! `SpellCancelsAuraEffect`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, cast_fresh, has};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::{Targets, checks, checks::fail, mods::Mods};

/// Controls: a physical stun, a shadow fear, a nature stun that is a
/// poison, an arcane confusion that is magic, a silence, Recklessness's
/// kind (no fleeing), and a confusion whose other effect is a stun's
/// mechanic, one whose own effect is, and one whose spell is.
const STUN: u32 = 997_001;
const FEAR: u32 = 997_002;
const POISON_STUN: u32 = 997_003;
const MAGIC_CONFUSE: u32 = 997_004;
const SILENCE: u32 = 997_005;
const RECKLESS: u32 = 997_006;
const CONFUSE_BESIDE: u32 = 997_007;
const CONFUSE_OWN: u32 = 997_008;
const CONFUSE_WHOLE: u32 = 997_009;
/// Escapes: immunity to physical; to every school; to every school
/// without purging; to poison; Blink's (a stun's mechanic); immunity to
/// the kinds of stun, fear and confusion, and that without purging.
const PHYSICAL: u32 = 997_020;
const EVERY_SCHOOL: u32 = 997_021;
const EVERY_SCHOOL_KEPT: u32 = 997_022;
const POISON_PROOF: u32 = 997_023;
const BLINK: u32 = 997_024;
const STATE_STUN: u32 = 997_025;
const STATE_FEAR: u32 = 997_026;
const STATE_CONFUSE: u32 = 997_027;
/// A fire bolt of silence's prevention, the same ignoring the caster's
/// restrictions, and Ice Block's kind (its own stun, and every school).
const FIRE: u32 = 997_030;
const FIRE_REGARDLESS: u32 = 997_031;
const ICE_BLOCK: u32 = 997_032;

/// `SPELL_ATTR_EX_IMMUNITY_PURGES_EFFECT`.
const PURGES: u32 = 0x8000;
/// `SPELL_ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS`.
const IGNORE_RESTRICTIONS: u32 = 0x1000_0000;
const STUN_MECHANIC: u32 = 12;

fn control(id: u32, school: u32, kind: u32) -> SpellTemplate {
    let mut s = bare(id, school);
    s.duration_index = LASTS_LONG;
    effect(&mut s, 0, aura(kind, 0, 0, 6));
    s
}

fn escape(id: u32, effects: &[(u32, i32)], purges: bool) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.prevention_type = checks::PREVENTION_SILENCE;
    s.duration_index = LASTS_5S;
    if purges {
        s.attributes[1] |= PURGES;
    }
    for (i, (kind, misc)) in effects.iter().enumerate() {
        effect(&mut s, i, aura(*kind, *misc, 0, 1));
    }
    s
}

fn data(d: &mut GameData) {
    let mut poison = control(POISON_STUN, 3, AURA_MOD_STUN);
    poison.dispel = 4;
    let mut magic = control(MAGIC_CONFUSE, 6, AURA_MOD_CONFUSE);
    magic.dispel = 1;
    let mut beside = control(CONFUSE_BESIDE, 6, AURA_MOD_CONFUSE);
    effect(
        &mut beside,
        1,
        SpellEffect {
            mechanic: STUN_MECHANIC,
            ..aura(AURA_DUMMY, 0, 0, 6)
        },
    );
    let mut own = control(CONFUSE_OWN, 6, AURA_MOD_CONFUSE);
    own.effects[0].mechanic = STUN_MECHANIC;
    let mut whole = control(CONFUSE_WHOLE, 6, AURA_MOD_CONFUSE);
    whole.mechanic = STUN_MECHANIC;
    let mut fire = bare(FIRE, 2);
    fire.prevention_type = checks::PREVENTION_SILENCE;
    effect(&mut fire, 0, damage(10));
    let mut regardless = fire.clone();
    regardless.id = FIRE_REGARDLESS;
    regardless.attributes[3] |= IGNORE_RESTRICTIONS;
    let ice = escape(
        ICE_BLOCK,
        &[
            (AURA_MOD_STUN, 0),
            (AURA_SCHOOL_IMMUNITY, 1),
            (AURA_SCHOOL_IMMUNITY, 126),
        ],
        true,
    );
    insert(
        d,
        [
            control(STUN, 0, AURA_MOD_STUN),
            control(FEAR, 5, AURA_MOD_FEAR),
            poison,
            magic,
            control(SILENCE, 5, AURA_MOD_SILENCE),
            control(RECKLESS, 0, AURA_PREVENTS_FLEEING),
            beside,
            own,
            whole,
            escape(PHYSICAL, &[(AURA_SCHOOL_IMMUNITY, 1)], true),
            escape(
                EVERY_SCHOOL,
                &[(AURA_SCHOOL_IMMUNITY, 1), (AURA_SCHOOL_IMMUNITY, 126)],
                true,
            ),
            escape(
                EVERY_SCHOOL_KEPT,
                &[(AURA_SCHOOL_IMMUNITY, 1), (AURA_SCHOOL_IMMUNITY, 126)],
                false,
            ),
            escape(POISON_PROOF, &[(AURA_DISPEL_IMMUNITY, 4)], true),
            escape(
                BLINK,
                &[(AURA_MECHANIC_IMMUNITY, STUN_MECHANIC as i32)],
                true,
            ),
            escape(
                STATE_STUN,
                &[(AURA_STATE_IMMUNITY, AURA_MOD_STUN as i32)],
                true,
            ),
            escape(
                STATE_FEAR,
                &[(AURA_STATE_IMMUNITY, AURA_MOD_FEAR as i32)],
                true,
            ),
            escape(
                STATE_CONFUSE,
                &[(AURA_STATE_IMMUNITY, AURA_MOD_CONFUSE as i32)],
                true,
            ),
            fire,
            regardless,
            ice,
        ],
    );
}

/// Why `g` can't begin `spell` at itself now, if it can't.
fn refused(m: &mut Map, g: Guid, spell: u32) -> Option<u8> {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    checks::check(m, g, &s, &Targets::default(), None, false, &Mods::default())
}

/// `g` under exactly `controls`, from `from`.
fn under(m: &mut Map, g: Guid, from: Guid, controls: &[u32]) {
    remove_where(m, g, |h| !h.passive);
    for c in controls {
        kit::apply(m, from, g, *c);
    }
}

/// An immunity that purges frees its caster from the controls it would
/// keep off, and only those, each holder looked at: a physical immunity
/// from a physical stun but not a shadow fear; every school from both,
/// but not without the purge; a poison's immunity from a poison's stun,
/// not a magic confusion beside it; an immunity to a kind (stun, fear,
/// confusion) from that kind, not without the purge; a mechanic's
/// immunity from a control whose own effect or spell has that mechanic,
/// not from one whose other effect has it (Thistle Tea's
/// `CasterState.validate`, mangos-classic's `SpellCancelsAuraEffect`).
#[tokio::test]
async fn an_immunity_that_purges_frees_from_what_it_keeps_off() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SC1", "Escaper", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let mut out = Vec::new();
            let mut case = |m: &mut Map, controls: &[u32], spell: u32| {
                under(m, a, kobold, controls);
                out.push(refused(m, a, spell));
            };
            // Schools.
            case(m, &[STUN, FEAR], PHYSICAL);
            case(m, &[STUN, FEAR], EVERY_SCHOOL);
            case(m, &[STUN, FEAR], EVERY_SCHOOL_KEPT);
            // Dispel types.
            case(m, &[POISON_STUN], POISON_PROOF);
            case(m, &[POISON_STUN, MAGIC_CONFUSE], POISON_PROOF);
            // Kinds.
            case(m, &[STUN], STATE_STUN);
            case(m, &[FEAR], STATE_FEAR);
            case(m, &[MAGIC_CONFUSE], STATE_CONFUSE);
            // Mechanics: the effect's own, or the spell's.
            case(m, &[CONFUSE_BESIDE], BLINK);
            case(m, &[CONFUSE_OWN], BLINK);
            case(m, &[CONFUSE_WHOLE], BLINK);
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![
            Some(fail::FLEEING),
            None,
            Some(fail::STUNNED),
            None,
            Some(fail::CONFUSED),
            None,
            None,
            None,
            Some(fail::CONFUSED),
            None,
            None,
        ]
    );
}

/// The same immunities without the purge free from nothing: each kind's
/// control holds (Thistle Tea's state-immunity purges, "through matching
/// control only").
#[tokio::test]
async fn an_immunity_that_doesnt_purge_frees_from_nothing() {
    let realm = fast_realm(|d| {
        data(d);
        for id in [STATE_STUN, STATE_FEAR, STATE_CONFUSE] {
            let mut s = d.spells.spells[&id].clone();
            s.id = id + 100;
            s.attributes[1] &= !PURGES;
            d.spells.spells.insert(s.id, s);
        }
    });
    let (_c, a, map) = human(&realm, "P7SC2", "Held", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            [
                (STUN, STATE_STUN),
                (FEAR, STATE_FEAR),
                (MAGIC_CONFUSE, STATE_CONFUSE),
            ]
            .map(|(control, escape)| {
                under(m, a, kobold, &[control]);
                (refused(m, a, escape), refused(m, a, escape + 100))
            })
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [
            (None, Some(fail::STUNNED)),
            (None, Some(fail::FLEEING)),
            (None, Some(fail::CONFUSED)),
        ]
    );
}

/// A fear that Recklessness's kind keeps from taking hold doesn't stop a
/// cast; a fear alone does (Thistle Tea's "fear suppressed by
/// recklessness permits casting"; mangos-classic's `SetFleeing` sets no
/// fleeing under `SPELL_AURA_PREVENTS_FLEEING`, and `CheckCasterAuras`
/// reads the flag).
#[tokio::test]
async fn a_fear_kept_from_taking_hold_doesnt_stop_a_cast() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SC3", "Reckless", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            under(m, a, kobold, &[FEAR]);
            let feared = refused_at(m, a, FIRE, kobold);
            under(m, a, kobold, &[FEAR, RECKLESS]);
            (feared, refused_at(m, a, FIRE, kobold))
        })
        .await
        .unwrap();
    assert_eq!(out, (Some(fail::FLEEING), None));
}

/// A stun and a silence stop a fire bolt; a triggered one, or one that
/// ignores its caster's restrictions, goes through them. A school locked
/// out stops it until the lockout ends, but not a triggered one (Thistle
/// Tea's "control and school lockout bypasses require explicit spell
/// permissions"; mangos-classic's `CheckCasterAuras` and
/// `SPELL_ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS`).
#[tokio::test]
async fn only_a_triggered_cast_or_one_ignoring_restrictions_goes_through_control() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SC4", "Restrained", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let s = m.realm.data.spells.get(FIRE).unwrap().clone();
            let t = kit::at(kobold);
            let check = |m: &mut Map, s: &SpellTemplate, triggered: bool| {
                checks::check(m, a, s, &t, None, triggered, &Mods::default())
            };
            under(m, a, kobold, &[STUN, SILENCE]);
            let controlled = (
                check(m, &s, false),
                check(m, &s, true),
                refused_at(m, a, FIRE_REGARDLESS, kobold),
            );
            under(m, a, kobold, &[]);
            let now = m.now;
            m.caster_mut(a)
                .unwrap()
                .school_lockout
                .push((s.school_mask(), now + std::time::Duration::from_secs(5)));
            let locked = (check(m, &s, false), check(m, &s, true));
            kit::pass(m, 5_000);
            (controlled, locked, check(m, &s, false))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (
            (Some(fail::STUNNED), None, None),
            (Some(fail::SILENCED), None),
            None
        )
    );
}

fn refused_at(m: &mut Map, g: Guid, spell: u32, at: Guid) -> Option<u8> {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    checks::check(m, g, &s, &kit::at(at), None, false, &Mods::default())
}

/// Ice Block's kind, cast stunned: its every-school immunity frees its
/// caster to cast it, takes the stun off as it lands, and its own stun
/// holds them, the cast done (Thistle Tea's "school immunity escapes
/// control and a self stun does not interrupt its own completed cast").
#[tokio::test]
async fn ice_block_is_cast_out_of_a_stun_and_its_own_stun_holds() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SC5", "Iceblock", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[ICE_BLOCK]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            under(m, a, kobold, &[STUN]);
            let began = cast_fresh(m, a, ICE_BLOCK, Targets::default());
            let stunned = auras(m, a).is_some_and(|x| x.has(AURA_MOD_STUN));
            (
                began,
                kit::casting(m, a),
                has(m, a, STUN),
                has(m, a, ICE_BLOCK),
                stunned,
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (true, None, false, true, true));
}
