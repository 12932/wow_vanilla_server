//! Round 8, against Thistle Tea's `stealth_casting_test.exs`: what a cast
//! breaks of its caster's stealth and invisibility, and when (mangos-
//! classic's `RemoveAurasOnCast`: `AURA_INTERRUPT_FLAG_ACTION` as a cast
//! begins, `ACTION_LATE` at its launch, but what the spell allows).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, casting, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_EX_ALLOW_WHILE_STEALTHED, ATTR_EX_FAILURE_BREAKS_STEALTH, ATTR_EX2_ALLOW_WHILE_INVISIBLE,
    ATTR_EX3_ALWAYS_HIT, ATTR_ONLY_STEALTHED, SpellTemplate,
};
use crate::game::map::Map;
use crate::game::map::auras::*;

/// 1.12's Stealth (its interrupt flags 0x13C03: a hostile action and a
/// cast's launch, not its start); an invisibility a cast's start breaks,
/// and one its launch does.
const STEALTH: u32 = 998_401;
const EARLY_INVISIBILITY: u32 = 998_402;
const LATE_INVISIBILITY: u32 = 998_403;
/// A poison's kind (three seconds, allowed in stealth); a cast of three
/// seconds stealth doesn't allow; one that invisibility allows.
const POISON: u32 = 998_404;
const PLAIN: u32 = 998_405;
const UNSEEN: u32 = 998_406;
/// Sap's kind, instant and of a second; Improved Sap's third rank; Pick
/// Pocket's kind (allowed in stealth, whose failure breaks it); what keeps
/// a target from every school.
const SAP: u32 = 998_407;
const SLOW_SAP: u32 = 998_408;
const IMPROVED_SAP: u32 = 14_095;
const PICK_POCKET: u32 = 998_409;
const UNTOUCHABLE: u32 = 998_410;
const SAP_ICON: u32 = 249;

fn lasting(id: u32, kind: u32, flags: u32, dispel: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    s.aura_interrupt_flags = flags;
    s.dispel = dispel;
    effect(&mut s, 0, aura(kind, 0, 5, 1));
    s
}

fn data(d: &mut GameData) {
    let stealth = lasting(STEALTH, AURA_MOD_STEALTH, 0x1_3C03, 5);
    let early = lasting(
        EARLY_INVISIBILITY,
        AURA_MOD_INVISIBILITY,
        INTERRUPT_CAST | INTERRUPT_ACTION,
        6,
    );
    let late = lasting(
        LATE_INVISIBILITY,
        AURA_MOD_INVISIBILITY,
        INTERRUPT_ACTION_COMPLETE,
        6,
    );
    let mut plain = bare(PLAIN, 0);
    plain.casting_time_index = CAST_3S;
    effect(&mut plain, 0, aura(AURA_DUMMY, 0, 0, 1));
    let mut poison = plain.clone();
    poison.id = POISON;
    poison.attributes[1] |= ATTR_EX_ALLOW_WHILE_STEALTHED;
    let mut unseen = plain.clone();
    unseen.id = UNSEEN;
    unseen.attributes[2] |= ATTR_EX2_ALLOW_WHILE_INVISIBLE;
    let mut sap = bare(SAP, 0);
    sap.spell_icon = SAP_ICON;
    sap.attributes[0] |= ATTR_ONLY_STEALTHED;
    sap.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    sap.duration_index = LASTS_5S;
    effect(&mut sap, 0, aura(AURA_MOD_STUN, 0, 0, 6));
    let mut slow_sap = sap.clone();
    slow_sap.id = SLOW_SAP;
    slow_sap.casting_time_index = CAST_2S;
    let improved = lasting(IMPROVED_SAP, AURA_DUMMY, 0, 0);
    let mut pick = bare(PICK_POCKET, 0);
    pick.attributes[1] |= ATTR_EX_ALLOW_WHILE_STEALTHED | ATTR_EX_FAILURE_BREAKS_STEALTH;
    pick.dmg_class = 0;
    effect(&mut pick, 0, aura(AURA_DUMMY, 0, 0, 6));
    let mut untouchable = bare(UNTOUCHABLE, 0);
    untouchable.duration_index = LASTS_LONG;
    effect(&mut untouchable, 0, aura(AURA_SCHOOL_IMMUNITY, 127, 0, 1));
    insert(
        d,
        [
            stealth,
            early,
            late,
            plain,
            poison,
            unseen,
            sap,
            slow_sap,
            improved,
            pick,
            untouchable,
        ],
    );
}

/// Which of stealth and the two invisibilities `g` has.
fn hidden(m: &Map, g: Guid) -> [bool; 3] {
    [STEALTH, EARLY_INVISIBILITY, LATE_INVISIBILITY].map(|s| kit::has(m, g, s))
}

fn hide(m: &mut Map, g: Guid) {
    for s in [STEALTH, EARLY_INVISIBILITY, LATE_INVISIBILITY] {
        if !kit::has(m, g, s) {
            kit::apply(m, g, g, s);
        }
    }
}

/// What `spell`'s cast leaves of `a`'s stealth and invisibilities as it
/// begins and once it is cast.
fn through_a_cast(m: &mut Map, a: Guid, spell: u32) -> ([bool; 3], [bool; 3]) {
    hide(m, a);
    cast_fresh(m, a, spell, Default::default());
    assert_eq!(casting(m, a), Some(spell));
    let begun = hidden(m, a);
    pass(m, 3_100);
    (begun, hidden(m, a))
}

/// A cast's start breaks what an action breaks at once, its launch what
/// an action breaks late, and 1.12's Stealth is of the late: a cast of
/// three seconds keeps it till it goes off; a poison's, which stealth
/// allows, keeps it after; one invisibility allows keeps both
/// invisibilities (Thistle Tea's "an invisibility-compatible cast
/// preserves early and late invisibility independently").
#[tokio::test]
async fn a_cast_breaks_stealth_at_its_launch_but_what_stealth_allows() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8SC1", "Sneaky", Class::Rogue).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[POISON, PLAIN, UNSEEN]);
            let plain = through_a_cast(m, a, PLAIN);
            remove_where(m, a, |_| true);
            let poison = through_a_cast(m, a, POISON);
            remove_where(m, a, |_| true);
            let unseen = through_a_cast(m, a, UNSEEN);
            (plain, poison, unseen)
        })
        .await
        .unwrap();
    let (plain, poison, unseen) = out;
    assert_eq!(
        plain,
        ([true, false, true], [false, false, false]),
        "a plain cast"
    );
    assert_eq!(
        poison,
        ([true, false, true], [true, false, false]),
        "a poison"
    );
    assert_eq!(
        unseen,
        ([true, true, true], [false, true, true]),
        "one invisibility allows"
    );
}

/// Sap with Improved Sap's third rank keeps stealth nine times in ten,
/// and both invisibilities break whatever it rolls; a failed roll leaves
/// no stealth (Thistle Tea's "a successful Sap roll survives launch while
/// early and late invisibility still break" and "a failed Sap roll and
/// cancellation never recreate stealth"); stealth gone as one of a cast
/// time is made isn't brought back by a lucky roll (its "a successful roll
/// does not restore stealth removed during preparation").
#[tokio::test]
async fn improved_sap_keeps_stealth_by_its_chance_and_nothing_else() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8SC2", "Sapper", Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(true);
            kit::ready(m, a, 60, &[SAP, SLOW_SAP]);
            kit::apply(m, a, a, IMPROVED_SAP);
            let mut outcomes = Vec::new();
            for _ in 0..150 {
                hide(m, a);
                let kobold = kit::dummy(m, a, 60, 3.0);
                assert!(cast_fresh(m, a, SAP, at(kobold)));
                assert!(kit::has(m, kobold, SAP), "sapped");
                outcomes.push(hidden(m, a));
            }
            // Revealed while a Sap of a cast time is made.
            let mut kept_after_reveal = false;
            for _ in 0..20 {
                hide(m, a);
                let kobold = kit::dummy(m, a, 60, 3.0);
                cast_fresh(m, a, SLOW_SAP, at(kobold));
                remove_where(m, a, |h| h.spell == STEALTH);
                pass(m, 2_100);
                kept_after_reveal |= kit::has(m, a, STEALTH);
            }
            (outcomes, kept_after_reveal)
        })
        .await
        .unwrap();
    let (outcomes, kept_after_reveal) = out;
    assert!(
        outcomes.iter().all(|o| matches!(o, [_, false, false])),
        "{outcomes:?}"
    );
    let kept = outcomes.iter().filter(|o| o[0]).count();
    assert!((110..150).contains(&kept), "kept {kept} of 150");
    assert!(!kept_after_reveal);
}

/// Pick Pocket's kind keeps stealth as it lands, and breaks it when it
/// fails, whatever stealth allows (Thistle Tea's "a resisted
/// failure-breaks-stealth spell overrides preservation";
/// mangos-classic's `SPELL_ATTR_EX_FAILURE_BREAKS_STEALTH`).
#[tokio::test]
async fn a_failed_pick_pocket_breaks_stealth() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8SC3", "Pocket", Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[PICK_POCKET]);
            kit::apply(m, a, a, STEALTH);
            let kobold = kit::dummy(m, a, 60, 3.0);
            cast_fresh(m, a, PICK_POCKET, at(kobold));
            let landed = kit::has(m, a, STEALTH);
            let wary = kit::dummy(m, a, 60, 3.0);
            kit::apply(m, wary, wary, UNTOUCHABLE);
            cast_fresh(m, a, PICK_POCKET, at(wary));
            (landed, kit::has(m, a, STEALTH))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false));
}
