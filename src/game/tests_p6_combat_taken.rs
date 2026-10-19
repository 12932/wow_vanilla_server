//! Round 6's combat tests, damage taken and what its logs say (Thistle
//! Tea's `damage_received_test.exs`, `attack_damage_taken_test.exs` and
//! `damage_feedback_test.exs`): the target's modifiers before the crit,
//! the block and the absorbs, a tick's live reductions, and the damage the
//! logs carry.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::spells::outcome::{self, Kind, Landing};
use crate::game::map::{Map, spells};
use crate::game::tests_p3_combat::{self as base, *};

const BOLT: u32 = 91_400;
const STRIKE: u32 = 91_401;
const SHOT: u32 = 91_402;
const MAGIC: u32 = 91_403;
const BLEED: u32 = 91_404;
const STONESKIN: u32 = 91_405;
const HOLY: u32 = 1;

fn spell(id: u32, class: u32, school: u32, coefficient: f32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: class,
        school,
        duration_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 99,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        bonus_coefficient: coefficient,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = base::data();
    let mut bleed = spell(BLEED, 2, 0, 0.5);
    bleed.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_PERIODIC_DAMAGE,
        base_points: 99,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        amplitude: 1000,
        bonus_coefficient: 0.5,
        ..Default::default()
    };
    let mut stoneskin = spell(STONESKIN, 0, 0, 0.0);
    stoneskin.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_MELEE_DAMAGE_TAKEN,
        base_points: -31,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    for s in [
        spell(BOLT, 1, HOLY, 0.5),
        spell(STRIKE, 2, 0, -1.0),
        spell(SHOT, 3, 0, -1.0),
        spell(MAGIC, 1, 0, -1.0),
        bleed,
        stoneskin,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.durations.insert(
        1,
        SpellDuration {
            base: 4000,
            per_level: 0,
            max: 4000,
        },
    );
    d
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// An absorb of `amount` of every school on `g`.
fn ward(m: &mut Map, g: Guid, id: u32, amount: i32) {
    aura_of(m, g, id, AURA_SCHOOL_ABSORB, 127, amount);
}

fn clear(m: &mut Map, g: Guid) {
    if let Some(a) = auras::auras_mut(m, g) {
        a.holders.retain(|h| !(91_410..91_430).contains(&h.spell));
    }
}

/// `spell`'s hundred on `target`, a crit or not, dealt: its health lost.
fn hit(m: &mut Map, g: Guid, target: Guid, spell: u32, crit: bool) -> u32 {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let l = Landing {
        crit,
        ..Default::default()
    };
    let d = outcome::damage(m, g, target, &s, 0, 100.0, &l);
    let before = health(m, target);
    outcome::deal(m, g, target, &s, d, &l);
    before - health(m, target)
}

fn spell_log(m: &ServerOpcodeMessage, spell: u32) -> Option<(u32, u32)> {
    match m {
        ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d) if d.spell == spell => {
            Some((d.damage, d.absorbed_damage))
        }
        _ => None,
    }
}

/// Thistle Tea's "target bonuses precede critical hits and absorbed damage
/// agrees with health loss", "direct spells and weapon abilities report
/// modified damage and absorption" and "reduction precedes absorption and
/// can fully suppress weapon damage", as mangos's `CalculateSpellDamage`
/// has them: the target's flat (by the coefficient) and percentage come
/// first, then the crit, then the absorb, which is spent; a melee
/// ability's flat cut comes before the absorb and can take it all. The log
/// carries the damage the target lost and what was absorbed (mangos's
/// `SendSpellNonMeleeDamageLog`, the damage less the absorb, where Thistle
/// Tea's carries both together).
#[tokio::test]
async fn the_targets_modifiers_come_before_the_crit_and_the_absorb_and_the_log_says_what_was_lost()
{
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            aura_of(m, brute, 91_410, AURA_MOD_DAMAGE_TAKEN, 127, 40);
            aura_of(m, brute, 91_411, AURA_MOD_DAMAGE_PERCENT_TAKEN, 127, 50);
            ward(m, brute, 91_412, 50);
            let crit = hit(m, ga, brute, BOLT, true);
            let spent =
                auras::auras(m, brute).is_some_and(|a| a.holders.iter().all(|h| h.spell != 91_412));
            clear(m, brute);
            // A half, half again, and all of it, each class, through a
            // ward of thirty.
            let mut by_class = Vec::new();
            for pct in [-50, 50, -100] {
                for spell in [MAGIC, STRIKE, SHOT] {
                    aura_of(m, brute, 91_413, AURA_MOD_DAMAGE_PERCENT_TAKEN, 1, pct);
                    ward(m, brute, 91_414, 30);
                    by_class.push(hit(m, ga, brute, spell, false));
                    clear(m, brute);
                }
            }
            // Thirty less of a melee blow, through twenty absorbed; then
            // two hundred less.
            aura_of(m, brute, 91_415, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, -30);
            ward(m, brute, 91_416, 20);
            let cut = hit(m, ga, brute, STRIKE, false);
            clear(m, brute);
            aura_of(m, brute, 91_415, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, -200);
            let all = hit(m, ga, brute, STRIKE, false);
            (crit, spent, by_class, cut, all)
        })
        .await
        .unwrap();
    let (crit, spent, by_class, cut, all) = r;
    assert_eq!(
        crit, 220,
        "(100 + 40 x 0.5) x 1.5, crit: 270, less 50 absorbed"
    );
    assert!(spent, "the absorb spent");
    assert_eq!(
        by_class,
        [20, 20, 20, 120, 120, 120, 0, 0, 0],
        "less thirty absorbed"
    );
    assert_eq!((cut, all), (50, 0), "70 less 20 absorbed; nothing");
    let logged = a.wait_for("the crit's log", |m| spell_log(m, BOLT)).await;
    assert_eq!(logged, (220, 50), "the damage lost, and the absorbed");
}

/// Thistle Tea's "overkill reports the hit amount and god mode reports no
/// damage": a hundred on a target of ten logs a hundred, and kills it.
#[tokio::test]
async fn a_blow_past_the_targets_health_logs_all_of_it() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let dead = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            if let Some(c) = m.creature_mut(brute) {
                c.fields.set_u32(UNIT_FIELD_HEALTH, 10);
            }
            hit(m, ga, brute, MAGIC, false);
            m.unit_view(brute).is_some_and(|u| !u.alive)
        })
        .await
        .unwrap();
    assert!(dead);
    let logged = a
        .wait_for("the killing blow's log", |m| spell_log(m, MAGIC))
        .await;
    assert_eq!(logged, (100, 0));
}

/// Thistle Tea's "melee and ranged autoattacks report modified health
/// damage" and "school percentages apply before block and are not applied
/// twice": a swing's log carries what the victim lost, after its
/// percentages and its absorb; a percentage taken comes before the block,
/// which takes its value off the doubled blow.
#[tokio::test]
async fn a_swing_logs_what_the_victim_lost_and_its_percentages_come_before_the_block() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            level(m, ga, 60);
            // A sword of no damage of its own: every blow the same.
            equip(m, ga, SWORD, crate::game::map::inventory::SLOT_MAIN_HAND);
            crate::game::map::derive::unit(m, ga);
            spawn(m, ga, BRUTE, 60, 3.0)
        })
        .await
        .unwrap();
    let mut lost = Vec::new();
    for pct in [-50, 50, -100] {
        a.clear();
        let health_lost = map
            .query(move |m| {
                aura_of(m, brute, 91_420, AURA_MOD_DAMAGE_PERCENT_TAKEN, 1, pct);
                ward(m, brute, 91_421, 30);
                let before = health(m, brute);
                forced_swing(m, ga, brute, Outcome::Hit);
                clear(m, brute);
                before - health(m, brute)
            })
            .await
            .unwrap();
        let frame = a
            .wait_frame("the swing", combat::SMSG_ATTACKERSTATEUPDATE, |_| true)
            .await;
        lost.push((health_lost, swing_log(&frame).1));
    }
    for (health_lost, logged) in &lost {
        assert_eq!(health_lost, logged, "{lost:?}");
    }
    assert_eq!(lost[2].0, 0, "all of it taken off");
    assert!(lost[1].0 > lost[0].0 * 2, "{lost:?}");
    let r = map
        .query(move |m| {
            aura_of(m, brute, 91_422, AURA_MOD_DAMAGE_PERCENT_TAKEN, 1, 100);
            let swung = |m: &mut Map, o: Outcome| {
                let before = health(m, brute);
                forced_swing(m, ga, brute, o);
                before - health(m, brute)
            };
            let whole = swung(m, Outcome::Hit);
            let blocked = swung(m, Outcome::Block);
            (whole, blocked, combat::block_value(m, brute))
        })
        .await
        .unwrap();
    let (whole, blocked, value) = r;
    assert_eq!(blocked, whole - value, "doubled, then {value} blocked");
}

/// Thistle Tea's "uses live reductions for physical ability ticks and
/// preserves the snapshot", as mangos's `PeriodicTick` has it
/// (`MeleeDamageBonusTaken` at each tick, its flat whole): a bleed's tick
/// takes the melee damage taken the target has then, the amount it holds
/// unchanged; the reduction gone, the whole tick again.
#[tokio::test]
async fn a_bleeds_tick_takes_the_reductions_the_target_has_then() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            spells::cast_triggered(m, ga, BLEED, Some(brute));
            let tick = |m: &mut Map| {
                let now = m.now;
                if let Some(h) = auras::auras_mut(m, brute)
                    .and_then(|a| a.holders.iter_mut().find(|h| h.spell == BLEED))
                {
                    h.effects[0].next_tick = Some(now);
                }
                let before = health(m, brute);
                auras::tick_due(m, brute, BLEED, ga);
                before - health(m, brute)
            };
            let first = tick(m);
            aura_of(m, brute, 91_423, AURA_MOD_MELEE_DAMAGE_TAKEN, 0, -40);
            let reduced = tick(m);
            let held = auras::auras(m, brute)
                .and_then(|a| a.holders.iter().find(|h| h.spell == BLEED))
                .map(|h| h.effects[0].amount);
            clear(m, brute);
            (first, reduced, held, tick(m))
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (100, 60, Some(100), 100),
        "less forty, live; the hundred held"
    );
}

/// Thistle Tea's "refresh, removal, expiry and death never retain
/// reduction": thirty less of melee damage, cast again, is thirty still;
/// gone, as it is taken off, expires or its bearer dies, none.
#[tokio::test]
async fn melee_damage_taken_is_its_auras_and_goes_with_them() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let strike = m.realm.data.spells.get(STRIKE).cloned().unwrap();
            let taken = |m: &Map, g: Guid| {
                outcome::taken_of(m, ga, g, &strike, 0, Kind::Direct, 1, 100.0) as u32
            };
            let mut seen = Vec::new();
            for end in 0..3 {
                let brute = spawn(m, ga, BRUTE, 60, 3.0);
                spells::cast_triggered(m, brute, STONESKIN, Some(brute));
                spells::cast_triggered(m, brute, STONESKIN, Some(brute));
                seen.push(taken(m, brute));
                match end {
                    0 => auras::remove_spell(m, brute, STONESKIN, None),
                    1 => {
                        let now = m.now;
                        if let Some(h) = auras::auras_mut(m, brute)
                            .and_then(|a| a.holders.iter_mut().find(|h| h.spell == STONESKIN))
                        {
                            h.expires = Some(now);
                        }
                        auras::update(m);
                    }
                    _ => combat::kill(m, ga, brute),
                }
                seen.push(taken(m, brute));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        [70, 100, 70, 100, 70, 100],
        "refreshed, not doubled; gone"
    );
}

/// Thistle Tea's "bypass spells retain resistance and absorption while
/// ignoring target bonuses", as mangos's `IGNORE_DAMAGE_TAKEN_MODIFIERS`
/// has it: a fire spell of any class with the attribute takes none of the
/// target's damage taken, flat or percentage, melee or ranged, but is
/// resisted in part by its fire resistance and absorbed by its ward.
#[tokio::test]
async fn a_spell_ignoring_the_targets_modifiers_is_still_resisted_and_absorbed() {
    const FIRE: u32 = 2;
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.spells.rolls = Some(true);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            for (id, kind, amount) in [
                (91_424, AURA_MOD_DAMAGE_TAKEN, 1000),
                (91_425, AURA_MOD_DAMAGE_PERCENT_TAKEN, 100),
                (91_426, AURA_MOD_MELEE_DAMAGE_TAKEN, 1000),
                (91_427, AURA_MOD_RANGED_DAMAGE_TAKEN, 1000),
                (91_428, AURA_MOD_MELEE_DAMAGE_TAKEN_PCT, 100),
                (91_429, AURA_MOD_RANGED_DAMAGE_TAKEN_PCT, 100),
            ] {
                aura_of(m, brute, id, kind, 127, amount);
            }
            let c = m.creature_mut(brute).unwrap();
            c.base.resistances[FIRE as usize] = 300;
            c.fields.set_u32(UNIT_FIELD_RESISTANCES + FIRE as u16, 300);
            let mut out = Vec::new();
            for class in 0..4 {
                let mut s = spell(91_430 + class, class, FIRE, -1.0);
                s.attributes[4] |=
                    crate::game::data::spells::ATTR_EX4_IGNORE_DAMAGE_TAKEN_MODIFIERS;
                let mut resisted = 0;
                for _ in 0..20 {
                    let d = outcome::damage(m, ga, brute, &s, 0, 100.0, &Landing::default());
                    assert_eq!(d.amount + d.resisted, 100, "class {class}");
                    resisted = resisted.max(d.resisted);
                }
                let d = outcome::damage(m, ga, brute, &s, 0, 100.0, &Landing::default());
                aura_of(m, brute, 91_419, AURA_SCHOOL_ABSORB, 127, 20);
                let amount = d.amount;
                let before = health(m, brute);
                outcome::deal(m, ga, brute, &s, d, &Landing::default());
                out.push((
                    resisted > 0,
                    before - health(m, brute) + 20 == amount.max(20),
                ));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, [(true, true); 4], "resisted in part, twenty absorbed");
}
