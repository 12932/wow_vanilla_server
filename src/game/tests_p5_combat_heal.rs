//! Phase 3's fifth round of combat tests, healing taken (Thistle Tea's
//! `healing_received_test.exs`): the strongest cut and gain, by the stack;
//! a school's flat healing taken by the heal's coefficient; the order of
//! the caster's bonuses, the target's and the crit; Healing Way; the heal's
//! log; Lay on Hands.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells::outcome::{self, Kind, Landing};
use crate::game::tests_p3_combat::*;

/// An aura of `kind`, `misc` and `amount` from spell `id` on `g`, `stacks`
/// deep.
fn stacked(m: &mut Map, g: Guid, id: u32, kind: u32, misc: i32, amount: i32, stacks: u32) {
    aura_of(m, g, id, kind, misc, amount);
    if let Some(h) =
        auras::auras_mut(m, g).and_then(|a| a.holders.iter_mut().find(|h| h.spell == id))
    {
        h.stacks = stacks;
    }
}

/// A holy heal for `points` whose coefficient is `coefficient` (below
/// zero: worked out), of damage class `class`.
fn heal(id: u32, points: i32, coefficient: f32, class: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Heal {id}"),
        range_index: 1,
        equipped_item_class: -1,
        school: 1,
        dmg_class: class,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 10,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        target_a: 1,
        bonus_coefficient: coefficient,
        ..Default::default()
    };
    s
}

fn clear(m: &mut Map, g: Guid) {
    if let Some(a) = auras::auras_mut(m, g) {
        a.holders.retain(|h| h.spell < 90_600 || h.spell > 90_699);
    }
}

/// Thistle Tea's "counts stacks and clamps complete suppression",
/// "ignores school masks and unrelated auras" and "removing the strongest
/// reduction resumes the weaker one": a cut of a tenth ten deep takes all
/// of a heal, a gain beside it nothing back, and twenty deep no less than
/// all; a damage-taken aura changes no heal, nor does a heal below none
/// become one; the strongest of two cuts gone, the weaker counts.
#[tokio::test]
async fn healing_taken_counts_the_stacks_and_stops_at_nothing() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let taken = |m: &Map| outcome::healing_taken(m, ga, 200.0);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -10, 10);
            stacked(m, ga, 90_601, AURA_MOD_HEALING_PCT, 0, 50, 1);
            let suppressed = taken(m);
            clear(m, ga);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -10, 20);
            let beyond = taken(m);
            clear(m, ga);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -50, 1);
            stacked(m, ga, 90_601, AURA_MOD_DAMAGE_PERCENT_TAKEN, 127, -100, 1);
            let unrelated = taken(m);
            let negative = outcome::healing_taken(m, ga, -100.0);
            clear(m, ga);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -50, 1);
            stacked(m, ga, 90_601, AURA_MOD_HEALING_PCT, 0, -20, 1);
            let both = taken(m);
            auras::remove_spell(m, ga, 90_600, None);
            let weaker = taken(m);
            (suppressed, beyond, unrelated, negative, both, weaker)
        })
        .await
        .unwrap();
    assert_eq!(r.0, 0.0, "ten tenths, and a half more of nothing");
    assert_eq!(r.1, 0.0, "twenty tenths");
    assert_eq!(r.2, 100.0, "the damage-taken aura counts for nothing");
    assert_eq!(r.3, 0.0, "below none");
    assert_eq!(r.4, 100.0, "the strongest cut alone");
    assert_eq!(r.5, 160.0, "the weaker, once the strongest goes");
}

/// Thistle Tea's "school flats use the coefficient before percentage
/// modifiers", "uses low-rank penalties and honors explicit zero
/// coefficients", "stacks both the received bonus and the periodic spell
/// independently", "unclassified heals retain percentages without
/// spell-power bonuses" and "Healing Way stacks affect only Healing Wave":
/// a holy heal takes the target's flat holy healing taken by its
/// coefficient, then the percentages; a nature one, the percentages alone;
/// a worked-out coefficient carries the low-rank penalty and a zero one
/// none of the flat; the flat's stacks and the heal's multiply; a heal of
/// no damage class takes no flat; Healing Way's three stacks of six
/// percent add to a Healing Wave only.
#[tokio::test]
async fn flat_healing_taken_goes_by_the_heals_school_coefficient_and_stacks() {
    let mut d = data();
    d.spells.cast_times.insert(90, 3500);
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let taken = |m: &Map, s: &SpellTemplate, amount: f32, stacks: u32| {
                outcome::spell_healing_taken(m, ga, amount, s, 0, Kind::Direct, stacks, 1.0)
            };
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -50, 1);
            stacked(m, ga, 90_601, AURA_MOD_HEALING_PCT, 0, 30, 1);
            stacked(m, ga, 90_602, AURA_MOD_HEALING, 2, 100, 1);
            let scaling = heal(90_650, 200, 0.5, 1);
            let holy = taken(m, &scaling, 200.0, 1);
            let nature = SpellTemplate {
                school: 3,
                ..scaling.clone()
            };
            let nature = taken(m, &nature, 200.0, 1);
            clear(m, ga);
            // A level-ten heal of three and a half seconds: its coefficient
            // worked out, five eighths for its rank.
            stacked(m, ga, 90_602, AURA_MOD_HEALING, 2, 100, 1);
            let low = SpellTemplate {
                spell_level: 10,
                casting_time_index: 90,
                ..heal(90_651, 200, -1.0, 1)
            };
            let low_rank = taken(m, &low, 200.0, 1);
            let zero = taken(m, &heal(90_652, 200, 0.0, 1), 200.0, 1);
            clear(m, ga);
            stacked(m, ga, 90_602, AURA_MOD_HEALING, 2, 100, 2);
            let stacks = taken(m, &scaling, 600.0, 3);
            clear(m, ga);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -50, 1);
            stacked(m, ga, 90_602, AURA_MOD_HEALING, 2, 100, 1);
            let unclassified = taken(m, &heal(90_653, 200, 0.5, 0), 200.0, 1);
            // Healing Way, three deep.
            stacked(m, ga, 29_203, AURA_DUMMY, 0, 6, 3);
            let way = |family: u32, flags: u64| {
                let s = SpellTemplate {
                    family_name: family,
                    family_flags: flags,
                    ..scaling.clone()
                };
                taken(m, &s, 200.0, 1)
            };
            let ways = [way(11, 0x40), way(11, 0x80), way(6, 0x40)];
            (holy, nature, low_rank, zero, stacks, unclassified, ways)
        })
        .await
        .unwrap();
    let (holy, nature, low_rank, zero, stacks, unclassified, ways) = r;
    assert_eq!(holy as u32, 162, "(200 + 100 x 0.5) x 0.5 x 1.3");
    assert_eq!(nature as u32, 130, "no holy flat");
    assert_eq!(low_rank as u32, 262, "200 + 100 x 5/8");
    assert_eq!(zero as u32, 200, "a zero coefficient");
    assert_eq!(stacks as u32, 900, "600 + 100 x 2 x 0.5 x 3");
    assert_eq!(unclassified as u32, 100, "the percentage alone");
    assert_eq!(
        ways.map(|w| w as u32),
        [147, 125, 125],
        "Healing Way on a Healing Wave only"
    );
}

/// Thistle Tea's "applies caster percentages before recipient flats and
/// critical healing" and "chain bounces scale both caster and recipient
/// bonuses": the caster's healing power by the coefficient and its
/// percentage come first, the target's flat by the coefficient next, the
/// crit's half last; a chain's later target takes its share of both.
#[tokio::test]
async fn the_casters_bonuses_come_before_the_targets_and_the_crit_after_both() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            stacked(m, ga, 90_603, AURA_MOD_HEALING_DONE, 127, 200, 1);
            stacked(m, ga, 90_604, AURA_MOD_HEALING_DONE_PERCENT, 127, 50, 1);
            stacked(m, ga, 90_602, AURA_MOD_HEALING, 2, 100, 1);
            let s = heal(90_650, 200, 0.5, 1);
            let crit = Landing {
                crit: true,
                ..Default::default()
            };
            let crit = outcome::healing(m, ga, ga, &s, 0, 200.0, &crit);
            let chain = Landing {
                chain: 0.5,
                ..Default::default()
            };
            let chain = outcome::healing(m, ga, ga, &s, 0, 200.0, &chain);
            (crit, chain)
        })
        .await
        .unwrap();
    assert_eq!(r.0, 750, "((200 + 200 x 0.5) x 1.5 + 100 x 0.5) x 1.5");
    assert_eq!(r.1, 250, "(200 + 100) x 0.5 x 1.5 + 100 x 0.5 x 0.5");
}

/// A heal over time of `kind` for `points` a second, of damage class
/// `class` and coefficient `coefficient`, stacking to `stack`.
fn hot(id: u32, kind: u32, points: i32, class: u32, coefficient: f32, stack: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Renew {id}"),
        range_index: 1,
        equipped_item_class: -1,
        school: 1,
        dmg_class: class,
        stack_amount: stack,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        target_a: 1,
        amplitude: 1000,
        bonus_coefficient: coefficient,
        ..Default::default()
    };
    s
}

/// Tick `spell`'s aura on `g` now; the health it gave.
fn tick(m: &mut Map, g: Guid, spell: u32) -> u32 {
    let now = m.now;
    if let Some(a) = auras::auras_mut(m, g) {
        for h in a.holders.iter_mut().filter(|h| h.spell == spell) {
            for e in h.effects.iter_mut() {
                e.next_tick = Some(now);
            }
        }
    }
    let before = health(m, g);
    auras::tick_due(m, g, spell, g);
    health(m, g) - before
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// Take `g` to `missing` short of whole health.
fn hurt(m: &mut Map, g: Guid, missing: u32) {
    let f = m.unit_fields_mut(g).unwrap();
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
    f.set_u32(UNIT_FIELD_HEALTH, max - missing);
}

/// The effect's amount a heal over time holds.
fn snapshot(m: &Map, g: Guid, spell: u32) -> Option<i32> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| h.effects[0].amount)
}

/// Thistle Tea's "uses current modifiers on every tick without changing
/// the snapshot", "flat changes affect active HoTs without rewriting their
/// snapshot", "scales stacked HoTs and preserves ticks under total
/// suppression" and "modifies percentage-health ticks and caps threat at
/// effective healing": each tick of a heal over time takes the healing
/// taken of the moment, a percentage cut, a flat gain by its coefficient
/// or a flat cut of no more than half, and the amount it holds stays as
/// it landed; two stacks heal twice over, a cut of all of it heals nothing
/// but leaves the aura ticking; a tick of a share of health takes the cut
/// too, and the threat it gives a creature is half what it healed, no more.
#[tokio::test]
async fn each_tick_of_a_heal_over_time_takes_the_healing_taken_of_the_moment() {
    let mut d = data();
    for s in [
        hot(90_760, AURA_PERIODIC_HEAL, 200, 0, -1.0, 1),
        hot(90_761, AURA_PERIODIC_HEAL, 200, 1, 0.2, 1),
        hot(90_762, AURA_PERIODIC_HEAL, 200, 0, -1.0, 3),
        hot(90_763, AURA_OBS_MOD_HEALTH, 20, 0, -1.0, 1),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let apply = |m: &mut Map, s: &SpellTemplate, points: i32| {
                let _ = auras::apply_spell(m, ga, ga, s, &[(0, points)], auras::Context::default());
            };
            level(m, ga, 60);
            hurt(m, ga, 900);
            let renew = hot(90_760, AURA_PERIODIC_HEAL, 200, 0, -1.0, 1);
            apply(m, &renew, 200);
            let first = tick(m, ga, 90_760);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -50, 1);
            let cut = tick(m, ga, 90_760);
            let kept = snapshot(m, ga, 90_760);
            clear(m, ga);
            auras::remove_spell(m, ga, 90_760, None);
            // Its coefficient a fifth: a flat hundred is twenty more a
            // tick; a flat cut of a thousand takes half.
            hurt(m, ga, 900);
            let scaled = hot(90_761, AURA_PERIODIC_HEAL, 200, 1, 0.2, 1);
            apply(m, &scaled, 200);
            stacked(m, ga, 90_602, AURA_MOD_HEALING, 2, 100, 1);
            let flat = tick(m, ga, 90_761);
            clear(m, ga);
            stacked(m, ga, 90_602, AURA_MOD_HEALING, 2, -1000, 1);
            let half = tick(m, ga, 90_761);
            clear(m, ga);
            let plain = tick(m, ga, 90_761);
            auras::remove_spell(m, ga, 90_761, None);
            // Two stacks; then all of it cut.
            hurt(m, ga, 900);
            let stacking = hot(90_762, AURA_PERIODIC_HEAL, 200, 0, -1.0, 3);
            apply(m, &stacking, 200);
            apply(m, &stacking, 200);
            let two = tick(m, ga, 90_762);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -100, 1);
            let none = tick(m, ga, 90_762);
            let ticking = snapshot(m, ga, 90_762).is_some();
            clear(m, ga);
            let again = tick(m, ga, 90_762);
            auras::remove_spell(m, ga, 90_762, None);
            // A fifth of health a tick, halved, thirty short of whole; a
            // brute fighting the healer.
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            crate::game::map::ai::add_threat(m, brute, ga, 1.0);
            let before = threat(m, brute, ga);
            hurt(m, ga, 30);
            let share = hot(90_763, AURA_OBS_MOD_HEALTH, 20, 0, -1.0, 1);
            apply(m, &share, 20);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -50, 1);
            let topped = tick(m, ga, 90_763);
            let threat = threat(m, brute, ga) - before;
            (
                (first, cut, kept),
                (flat, half, plain),
                (two, none, ticking, again),
                (topped, threat),
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, (200, 100, Some(200)), "the cut of the moment");
    assert_eq!(r.1, (220, 100, 200), "the flat by the coefficient");
    assert_eq!(r.2, (400, 0, true, 400), "stacks, and all cut");
    assert_eq!(r.3.0, 30, "to whole");
    assert!((r.3.1 - 15.0).abs() < 0.01, "half the thirty: {}", r.3.1);
}

fn threat(m: &Map, creature: Guid, g: Guid) -> f32 {
    m.creature(creature)
        .and_then(|c| c.threat.iter().find(|(t, _)| *t == g).map(|(_, v)| *v))
        .unwrap_or(0.0)
}

const DRAIN: u32 = 90_770;
const SIPHON: u32 = 90_771;
const MEND: u32 = 90_772;
const LAY_ON_HANDS: u32 = 90_773;

fn transfers() -> crate::game::data::GameData {
    let mut d = data();
    let mut drain = hot(DRAIN, AURA_PERIODIC_LEECH, 100, 1, 0.0, 1);
    drain.school = 5;
    drain.effects[0].target_a = 6;
    let mut siphon = heal(SIPHON, 100, 0.0, 1);
    siphon.school = 5;
    siphon.effects[0].effect = 9;
    siphon.effects[0].target_a = 6;
    let mend = heal(MEND, 200, 0.0, 1);
    let mut lay = heal(LAY_ON_HANDS, 0, 0.0, 1);
    lay.effects[0].effect = 67;
    for s in [drain, siphon, mend, lay] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Thistle Tea's "transferred healing uses the receiving owner's
/// modifiers", "modifies direct healing and its feedback and effective
/// threat" and "suppresses maximum-health heals", as mangos's
/// `PeriodicTick`, `EffectHealthLeech`, `EffectHeal` and
/// `EffectHealMaxHealth` have them: what a drain gives back is healing its
/// caster takes, halved by their cut, and makes no threat; a heal's log
/// says what it healed after the cut, and its threat is half that, split
/// among the creatures fighting its target; Lay on
/// Hands's whole health takes the cut too.
#[tokio::test]
async fn drained_health_direct_heals_and_lay_on_hands_take_the_healers_cut() {
    let (_realm, mut a, ga, map) = setup_with(transfers(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let max = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            stacked(m, ga, 90_600, AURA_MOD_HEALING_PCT, 0, -50, 1);
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            let other = spawn(m, ga, BRUTE, 20, 4.0);
            crate::game::map::ai::add_threat(m, other, ga, 1.0);
            let threat_of = |m: &Map| threat(m, other, ga);
            // A drain over time, ticked.
            hurt(m, ga, 900);
            let s = m.realm.data.spells.get(DRAIN).cloned().unwrap();
            let _ = auras::apply_spell(m, ga, brute, &s, &[(0, 100)], auras::Context::default());
            let before = (health(m, ga), threat_of(m));
            let now = m.now;
            if let Some(h) = auras::auras_mut(m, brute)
                .and_then(|a| a.holders.iter_mut().find(|h| h.spell == DRAIN))
            {
                h.effects[0].next_tick = Some(now);
            }
            auras::tick_due(m, brute, DRAIN, ga);
            let drained = (health(m, ga) - before.0, threat_of(m) - before.1);
            // A siphon at once.
            let before = (health(m, ga), threat_of(m));
            crate::game::map::spells::cast_triggered(m, ga, SIPHON, Some(brute));
            let siphoned = (health(m, ga) - before.0, threat_of(m) - before.1);
            // A heal of two hundred.
            let before = (health(m, ga), threat_of(m));
            crate::game::map::spells::cast_triggered(m, ga, MEND, Some(ga));
            let mended = (health(m, ga) - before.0, threat_of(m) - before.1);
            // Lay on Hands, nearly dead.
            m.unit_fields_mut(ga).unwrap().set_u32(UNIT_FIELD_HEALTH, 1);
            crate::game::map::spells::cast_triggered(m, ga, LAY_ON_HANDS, Some(ga));
            let laid = health(m, ga) - 1;
            (max, drained, siphoned, mended, laid)
        })
        .await
        .unwrap();
    let (max, drained, siphoned, mended, laid) = r;
    assert_eq!(drained, (50, 0.0), "a drain's tick, halved, no threat");
    assert_eq!(siphoned, (50, 0.0), "a siphon, halved, no threat");
    assert_eq!(mended.0, 100, "a heal, halved");
    // Half of it, split between the two creatures fighting the healer.
    assert!(
        (mended.1 - 25.0).abs() < 0.01,
        "a quarter of the heal: {}",
        mended.1
    );
    assert_eq!(laid, max / 2, "Lay on Hands, halved");
    let logged = a
        .wait_for("the heal's log", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLHEALLOG(l) if l.id == MEND => Some(l.damage),
            _ => None,
        })
        .await;
    assert_eq!(logged, 100, "the log says what it healed");
}
