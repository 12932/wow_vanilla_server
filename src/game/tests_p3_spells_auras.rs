//! Phase 3's auras, against the real data when it is here, as Thistle
//! Tea's tests of them have it: absorbs and what they scale with, Detect
//! Magic's flag, and a full aura bar's giving way.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::spells;
use crate::game::map::{Map, auras};

const FIRE_WARD: u32 = 543;
const FROST_WARD: u32 = 6143;
const SHADOW_WARD: u32 = 6229;
const MANA_SHIELD: u32 = 1463;
const POWER_WORD_SHIELD_10: u32 = 10901;
const WEAKENED_SOUL: u32 = 6788;
const DETECT_MAGIC: u32 = 2855;
const DISPEL_MAGIC: u32 = 527;

/// What `g`'s `spell` has left to absorb.
fn absorb_left(m: &Map, g: Guid, spell: u32) -> Option<i32> {
    holder(m, g, spell).and_then(|h| {
        h.effects
            .iter()
            .find(|x| matches!(x.kind, auras::AURA_SCHOOL_ABSORB | auras::AURA_MANA_SHIELD))
            .map(|x| x.left)
    })
}

/// A buff that stays, of `kind` with `misc` and `amount`, on `g`.
fn buff(m: &mut Map, g: Guid, id: u32, kind: u32, misc: i32, amount: i32) {
    let mut s = made_up(id, 6, kind, misc, amount);
    s.duration_index = 21;
    apply_template(m, g, g, &s);
}

#[tokio::test]
async fn wards_take_a_tenth_of_their_schools_power_and_mana_shield_none() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3A1", "Wardone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let cast = |m: &mut Map, spell: u32| {
                auras::remove_spell(m, mage, spell, None);
                spells::cast_triggered(m, mage, spell, None);
                absorb_left(m, mage, spell).unwrap_or(-1)
            };
            let plain: Vec<i32> = [FIRE_WARD, FROST_WARD, SHADOW_WARD, MANA_SHIELD]
                .into_iter()
                .map(|s| cast(m, s))
                .collect();
            // Fire 200, frost 300, shadow 400, arcane 500, and healing.
            buff(m, mage, 990_201, auras::AURA_MOD_DAMAGE_DONE, 4, 200);
            buff(m, mage, 990_202, auras::AURA_MOD_DAMAGE_DONE, 16, 300);
            buff(m, mage, 990_203, auras::AURA_MOD_DAMAGE_DONE, 32, 400);
            buff(m, mage, 990_204, auras::AURA_MOD_DAMAGE_DONE, 64, 500);
            buff(m, mage, 990_205, auras::AURA_MOD_HEALING_DONE, 0, 1000);
            let powered: Vec<i32> = [FIRE_WARD, FROST_WARD, SHADOW_WARD, MANA_SHIELD]
                .into_iter()
                .map(|s| cast(m, s))
                .collect();
            // Only the others: Fire Ward takes no blow, fire past it gets
            // through and it is gone.
            for s in [FROST_WARD, SHADOW_WARD, MANA_SHIELD] {
                auras::remove_spell(m, mage, s, None);
            }
            let fire = absorb_left(m, mage, FIRE_WARD).unwrap() as u32;
            let blow = auras::absorb(m, mage, 50, 1);
            let past = auras::absorb(m, mage, fire + 1, 4);
            let gone = !has(m, mage, FIRE_WARD);
            // Mana Shield pays its mana: two to the point.
            let shield = cast(m, MANA_SHIELD) as u32;
            set_power(m, mage, 0, 1_000);
            let through = auras::absorb(m, mage, shield + 20, 1);
            let mana = power(m, mage, 0);
            (plain, powered, blow, past, gone, shield, through, mana)
        })
        .await
        .unwrap();
    let (plain, powered, blow, past, gone, shield, through, mana) = out;
    assert!(plain.iter().all(|x| *x > 0), "each absorbs: {plain:?}");
    let gained: Vec<i32> = powered.iter().zip(&plain).map(|(a, b)| a - b).collect();
    assert_eq!(
        gained,
        vec![20, 30, 40, 0],
        "a tenth of the school's power; none for Mana Shield"
    );
    assert_eq!(blow, (50, 0), "a fire ward takes no physical blow");
    assert_eq!(past.0, 1, "fire past the ward gets through");
    assert!(gone, "and the ward is spent");
    assert_eq!(through, (20, shield), "Mana Shield takes what it holds");
    assert_eq!(mana, 1_000 - shield * 2, "for two mana a point");
}

#[tokio::test]
async fn a_shield_holds_what_it_had_when_cast_and_goes_as_auras_go() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, priest, map) = player(&realm, "P3A2", "Shieldone", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, priest, 60, &[POWER_WORD_SHIELD_10]);
            let shield = |m: &mut Map| {
                auras::remove_spell(m, priest, WEAKENED_SOUL, None);
                auras::remove_spell(m, priest, POWER_WORD_SHIELD_10, None);
                spells::cast_triggered(m, priest, POWER_WORD_SHIELD_10, Some(priest));
                absorb_left(m, priest, POWER_WORD_SHIELD_10).unwrap_or(-1)
            };
            let base = shield(m);
            buff(m, priest, 990_211, auras::AURA_MOD_HEALING_DONE, 0, 500);
            let full = shield(m);
            // Shadow takes all but 25 of it.
            let _ = auras::absorb(m, priest, (full - 25) as u32, 32);
            let left = absorb_left(m, priest, POWER_WORD_SHIELD_10);
            // More healing power after doesn't fill it again.
            buff(m, priest, 990_212, auras::AURA_MOD_HEALING_DONE, 0, 500);
            let after_buff = absorb_left(m, priest, POWER_WORD_SHIELD_10);
            auras::remove_spell(m, priest, 990_211, None);
            auras::remove_spell(m, priest, 990_212, None);
            let again = shield(m);
            // Thirty seconds and it runs out; it can be taken off; death
            // takes it.
            m.now += Duration::from_millis(30_001);
            auras::update(m);
            let expired = !has(m, priest, POWER_WORD_SHIELD_10);
            shield(m);
            auras::cancel(m, priest, POWER_WORD_SHIELD_10);
            let cancelled = !has(m, priest, POWER_WORD_SHIELD_10);
            shield(m);
            crate::game::map::death::kill(m, priest);
            let died = !has(m, priest, POWER_WORD_SHIELD_10);
            (
                base, full, left, after_buff, again, expired, cancelled, died,
            )
        })
        .await
        .unwrap();
    let (base, full, left, after_buff, again, expired, cancelled, died) = out;
    assert!(base > 0);
    assert_eq!(full - base, 50, "a tenth of the healing power");
    assert_eq!(left, Some(25));
    assert_eq!(after_buff, Some(25), "a buff after doesn't fill it");
    assert_eq!(again, base, "a fresh cast holds what its caster has then");
    assert!(expired, "it runs out");
    assert!(cancelled, "it can be taken off");
    assert!(died, "and death takes it");
}

const AURAS_VISIBLE: u32 = 0x0800_0000;

fn flags(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS))
}

#[tokio::test]
async fn detect_magic_shows_a_units_auras_while_one_is_on_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3A3", "Detectone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            let other = dummy(m, mage, 60, 6.0);
            // On a creature and on a player.
            spells::cast_triggered(m, mage, DETECT_MAGIC, Some(kobold));
            apply(m, kobold, mage, DETECT_MAGIC);
            let on = (
                flags(m, kobold) & AURAS_VISIBLE,
                flags(m, mage) & AURAS_VISIBLE,
            );
            // Refreshed at a minute, it holds past its first two.
            m.now += Duration::from_secs(60);
            apply(m, kobold, mage, DETECT_MAGIC);
            m.now += Duration::from_secs(61);
            auras::update(m);
            let refreshed = flags(m, mage) & AURAS_VISIBLE;
            m.now += Duration::from_secs(60);
            auras::update(m);
            let ran_out = flags(m, mage) & AURAS_VISIBLE;
            // Two casters' on the kobold: it shows while either's is on.
            spells::cast_triggered(m, mage, DETECT_MAGIC, Some(kobold));
            apply(m, other, kobold, DETECT_MAGIC);
            let two = auras::auras(m, kobold).map_or(0, |a| {
                a.holders.iter().filter(|h| h.spell == DETECT_MAGIC).count()
            });
            auras::remove_spell(m, kobold, DETECT_MAGIC, Some(mage));
            let one_left = flags(m, kobold) & AURAS_VISIBLE;
            // Dispelled, it goes.
            spells::cast_triggered(m, kobold, DISPEL_MAGIC, Some(kobold));
            let dispelled = (
                has(m, kobold, DETECT_MAGIC),
                flags(m, kobold) & AURAS_VISIBLE,
            );
            // Death takes it.
            apply(m, kobold, mage, DETECT_MAGIC);
            crate::game::map::death::kill(m, mage);
            let died = (has(m, mage, DETECT_MAGIC), flags(m, mage) & AURAS_VISIBLE);
            (on, refreshed, ran_out, two, one_left, dispelled, died)
        })
        .await
        .unwrap();
    let (on, refreshed, ran_out, two, one_left, dispelled, died) = out;
    assert_eq!(
        on,
        (AURAS_VISIBLE, AURAS_VISIBLE),
        "a creature's and a player's"
    );
    assert_eq!(refreshed, AURAS_VISIBLE, "a refresh holds it");
    assert_eq!(ran_out, 0, "until it runs out");
    assert_eq!(two, 2, "one of each caster's");
    assert_eq!(one_left, AURAS_VISIBLE, "one left shows it still");
    assert_eq!(dispelled, (false, 0), "dispelled, it goes");
    assert_eq!(died, (false, 0), "and death takes it");
}

/// Made-up debuffs, worth 3 to the aura bar (damage taken), and periodic
/// damage worth 1, in the data so their worth can be looked up.
fn bar_spells(d: &mut crate::game::data::GameData) {
    for i in 0..20 {
        let mut s = made_up(990_300 + i, 6, auras::AURA_MOD_DAMAGE_TAKEN, 127, 1);
        s.duration_index = 21;
        d.spells.spells.insert(s.id, s);
        let mut s = made_up(990_400 + i, 6, auras::AURA_PERIODIC_DAMAGE, 0, 1);
        s.effects[0].amplitude = 1000;
        s.school = 0;
        d.spells.spells.insert(s.id, s);
    }
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).cloned().unwrap()
}

/// `caster`'s `id` on `target` for a minute, as a debuff.
fn debuff(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let s = template(m, id);
    let _ = auras::apply_spell(
        m,
        caster,
        target,
        &s,
        &[(0, 1)],
        auras::Context {
            duration: Some(Some(60_000)),
            ..Default::default()
        },
    );
}

fn slotted(m: &Map, g: Guid, positive: bool) -> usize {
    auras::auras(m, g).map_or(0, |a| {
        a.holders
            .iter()
            .filter(|h| h.positive == positive && h.slot.is_some())
            .count()
    })
}

const FAERIE_FIRE: u32 = 770;
const ENTANGLING_ROOTS: u32 = 339;
const DEADLY_POISON: u32 = 2818;
const BLIZZARD: u32 = 10;

#[tokio::test]
async fn a_full_debuff_bar_gives_way_by_worth_and_age() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(bar_spells);
    let (_c, mage, map) = player(&realm, "P3A4", "Barone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[BLIZZARD]);
            let kobold = dummy(m, mage, 60, 5.0);
            // Sixteen damages over time fill the bar; Faerie Fire, worth
            // more, takes the oldest's place.
            for i in 0..16 {
                debuff(m, mage, kobold, 990_400 + i);
            }
            let full = slotted(m, kobold, false);
            let first_slot = holder(m, kobold, 990_400).and_then(|h| h.slot);
            apply(m, mage, kobold, FAERIE_FIRE);
            let swapped = (
                has(m, kobold, 990_400),
                holder(m, kobold, FAERIE_FIRE).and_then(|h| h.slot),
                m.creature(kobold)
                    .map(|c| c.fields.get_u32(UNIT_FIELD_AURA + 32)),
            );
            // A tick: fifteen of them hurt, the one gone doesn't.
            let before = health(m, kobold);
            let now = m.now;
            if let Some(a) = auras::auras_mut(m, kobold) {
                for h in a.holders.iter_mut() {
                    for x in h.effects.iter_mut().filter(|x| x.period.is_some()) {
                        x.next_tick = Some(now);
                    }
                }
            }
            auras::update(m);
            let hurt = before - health(m, kobold);
            // A ground spell's damage lands on a full bar, the bar as it was.
            let t = spells::Targets {
                dest: m.position_of(kobold),
                ..Default::default()
            };
            cast_fresh(m, mage, BLIZZARD, t);
            let before = health(m, kobold);
            pass(m, 1_100);
            let blizzard = health(m, kobold) < before;
            let still = slotted(m, kobold, false);
            spells::interrupt(m, mage);
            (full, first_slot, swapped, hurt, blizzard, still)
        })
        .await
        .unwrap();
    let (full, first_slot, swapped, hurt, blizzard, still) = out;
    assert_eq!(full, 16, "sixteen debuffs show");
    assert_eq!(first_slot, Some(32), "the first in the first debuff slot");
    assert_eq!(
        swapped,
        (false, Some(32), Some(FAERIE_FIRE)),
        "Faerie Fire takes the oldest damage over time's place"
    );
    assert_eq!(hurt, 15, "fifteen ticks, none from the one gone");
    assert!(blizzard, "Blizzard hurts the kobold");
    assert_eq!(still, 16, "and the bar is as it was");
}

#[tokio::test]
async fn a_refresh_on_a_full_bar_stacks_in_place_and_a_root_given_way_frees() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(bar_spells);
    let (_c, mage, map) = player(&realm, "P3A5", "Bartwo", Race::Human, Class::Mage).await;
    let (mut d, druid, _) = player(&realm, "P3A6", "Barthree", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            // Deadly Poison among fifteen more, refreshed: two stacks, in its
            // slot.
            spells::cast_triggered(m, mage, DEADLY_POISON, Some(kobold));
            let slot = holder(m, kobold, DEADLY_POISON).and_then(|h| h.slot);
            for i in 0..15 {
                debuff(m, mage, kobold, 990_400 + i);
            }
            spells::cast_triggered(m, mage, DEADLY_POISON, Some(kobold));
            let refreshed = holder(m, kobold, DEADLY_POISON).map(|h| (h.stacks, h.slot));
            let count = slotted(m, kobold, false);
            // All run out; one more is the first again.
            m.now += Duration::from_secs(61);
            auras::update(m);
            let empty = auras::auras(m, kobold).is_none_or(|a| a.holders.is_empty());
            spells::cast_triggered(m, mage, DEADLY_POISON, Some(kobold));
            let fresh = holder(m, kobold, DEADLY_POISON).map(|h| (h.stacks, h.slot));
            // Roots on a player, and fifteen debuffs worth more: one more
            // takes the root's place, and the player is free.
            spells::cast_triggered(m, mage, ENTANGLING_ROOTS, Some(druid));
            let rooted = has(m, druid, ENTANGLING_ROOTS);
            for i in 0..16 {
                debuff(m, mage, druid, 990_300 + i);
            }
            let freed = !has(m, druid, ENTANGLING_ROOTS);
            let recovers = m.caster(druid).map(|c| {
                c.diminishing
                    .iter()
                    .map(|(_, n, at)| (*n, at.map(|t| t.saturating_duration_since(m.now))))
                    .collect::<Vec<_>>()
            });
            (
                slot, refreshed, count, empty, fresh, rooted, freed, recovers,
            )
        })
        .await
        .unwrap();
    let (slot, refreshed, count, empty, fresh, rooted, freed, recovers) = out;
    assert_eq!(slot, Some(32));
    assert_eq!(refreshed, Some((2, Some(32))), "two stacks, in its slot");
    assert_eq!(count, 16);
    assert!(empty, "all ran out");
    assert_eq!(fresh, Some((1, Some(32))), "one again, first again");
    assert!(rooted, "rooted");
    assert!(freed, "the root gave way");
    d.wait_for("SMSG_FORCE_MOVE_UNROOT", |m| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(_) => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(
        recovers,
        Some(vec![(1, Some(Duration::from_secs(15)))]),
        "the root's returns recover from when it went"
    );
}

#[tokio::test]
async fn a_full_buff_bar_keeps_its_lasting_buffs_and_death_its_passives() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(bar_spells);
    let (_c, mage, map) = player(&realm, "P3A7", "Barfour", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            // Thirty-two lasting buffs: a timed one of the mage's own is
            // refused.
            for i in 0..32 {
                let mut s = made_up(990_500 + i, 6, auras::AURA_DUMMY, 0, 1);
                s.duration_index = 21;
                apply_template(m, mage, mage, &s);
            }
            let int_before = m.players[&mage].fields.get_u32(UNIT_FIELD_STAT0 + 3);
            let mut own = made_up(990_540, 6, auras::AURA_MOD_STAT, 3, 50);
            own.duration_index = 28;
            apply_template(m, mage, mage, &own);
            let refused = (
                has(m, mage, 990_540),
                m.players[&mage].fields.get_u32(UNIT_FIELD_STAT0 + 3) == int_before,
            );
            // Seventeen debuffs: sixteen show.
            for i in 0..17 {
                debuff(m, kobold, mage, 990_300 + i);
            }
            let shown = (slotted(m, mage, true), slotted(m, mage, false));
            let passives: Vec<u32> = auras::auras(m, mage)
                .map(|a| {
                    a.holders
                        .iter()
                        .filter(|h| h.passive)
                        .map(|h| h.spell)
                        .collect()
                })
                .unwrap_or_default();
            let passive_slots = auras::auras(m, mage).map_or(0, |a| {
                a.holders
                    .iter()
                    .filter(|h| h.passive && h.slot.is_some())
                    .count()
            });
            crate::game::map::death::kill(m, mage);
            let after: Vec<u32> = auras::auras(m, mage)
                .map(|a| a.holders.iter().map(|h| h.spell).collect())
                .unwrap_or_default();
            (refused, shown, passives, passive_slots, after)
        })
        .await
        .unwrap();
    let (refused, shown, passives, passive_slots, after) = out;
    assert_eq!(
        refused,
        (false, true),
        "the timed buff is refused, and gives nothing"
    );
    assert_eq!(shown, (32, 16), "thirty-two buffs and sixteen debuffs show");
    assert!(!passives.is_empty(), "a mage has passives");
    assert_eq!(passive_slots, 0, "which take no slot");
    assert_eq!(after, passives, "death leaves only the passives");
}

const EVOCATION: u32 = 12051;
const POLYMORPH: u32 = 118;

fn channel_spell(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_CHANNEL_SPELL))
}

#[tokio::test]
async fn a_self_channel_ends_when_its_aura_is_taken_or_gives_way() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, mage, map) = player(&realm, "P3A8", "Evoone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[EVOCATION]);
            // Taken off.
            cast_fresh(m, mage, EVOCATION, spells::Targets::default());
            let began = (
                casting(m, mage),
                has(m, mage, EVOCATION),
                channel_spell(m, mage),
            );
            auras::remove_spell(m, mage, EVOCATION, None);
            let taken = (casting(m, mage), channel_spell(m, mage));
            // Given way to a buff on a full bar.
            cast_fresh(m, mage, EVOCATION, spells::Targets::default());
            for i in 0..32 {
                let mut s = made_up(990_600 + i, 6, auras::AURA_DUMMY, 0, 1);
                s.duration_index = 21;
                apply_template(m, mage, mage, &s);
            }
            let gave_way = (
                has(m, mage, EVOCATION),
                casting(m, mage),
                channel_spell(m, mage),
            );
            // Run its course, it ends as it should.
            for i in 0..32 {
                auras::remove_spell(m, mage, 990_600 + i, None);
            }
            cast_fresh(m, mage, EVOCATION, spells::Targets::default());
            pass(m, 8_000);
            let ran = (casting(m, mage), has(m, mage, EVOCATION));
            (began, taken, gave_way, ran)
        })
        .await
        .unwrap();
    let (began, taken, gave_way, ran) = out;
    assert_eq!(began, (Some(EVOCATION), true, EVOCATION));
    assert_eq!(taken, (None, 0), "its aura taken off, the channel ends");
    assert_eq!(gave_way, (false, None, 0), "as when its aura gives way");
    assert_eq!(ran, (None, false), "run its course, it ends");
    for _ in 0..3 {
        c.wait_for("MSG_CHANNEL_UPDATE 0", |m| match m {
            ServerOpcodeMessage::MSG_CHANNEL_UPDATE(u) if u.time == 0 => Some(()),
            _ => None,
        })
        .await;
    }
}

#[tokio::test]
async fn a_crowd_control_a_full_bar_refuses_spends_no_returns() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(bar_spells);
    let (_c, mage, map) = player(&realm, "P3A9", "Polyone", Race::Human, Class::Mage).await;
    let (_d, target, _) = player(&realm, "P3A10", "Polytwo", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            for i in 0..16 {
                debuff(m, mage, target, 990_300 + i);
            }
            spells::cast_triggered(m, mage, POLYMORPH, Some(target));
            let refused = !has(m, target, POLYMORPH);
            let returns = m.caster(target).map(|c| c.diminishing.clone());
            // With room, it goes on and spends one.
            auras::remove_spell(m, target, 990_300, None);
            spells::cast_triggered(m, mage, POLYMORPH, Some(target));
            let on = has(m, target, POLYMORPH);
            let spent = m
                .caster(target)
                .map(|c| c.diminishing.iter().map(|(_, n, _)| *n).sum::<u8>());
            (refused, returns, on, spent)
        })
        .await
        .unwrap();
    let (refused, returns, on, spent) = out;
    assert!(refused, "a full bar refuses the Polymorph");
    assert_eq!(returns, Some(Vec::new()), "and it spends no returns");
    assert!(on, "with room it goes on");
    assert_eq!(spent, Some(1), "and spends one");
}

const SHIELD_BLOCK: u32 = 2565;
const IMPROVED_SHIELD_BLOCK: u32 = 12945;

/// Made-up warrior spells for charge modifiers: a 2-charge and a
/// charge-less block like Shield Block's, one of other flags, a strike of
/// its flags, and modifiers of -1, -10, +25% and a charged +1.
fn charge_spells(d: &mut crate::game::data::GameData) {
    let block = |id: u32, charges: u32, flags: u64| {
        let mut s = made_up(id, 6, auras::AURA_MOD_BLOCK_PERCENT, 0, 75);
        s.family_name = 4;
        s.family_flags = flags;
        s.proc_charges = charges;
        s.duration_index = 28;
        s
    };
    for s in [
        block(990_700, 2, 0x1000),
        block(990_701, 0, 0x1000),
        block(990_702, 1, 0x2),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let mut strike = made_up(990_703, 2, 0, 0, 10);
    strike.family_name = 4;
    strike.family_flags = 0x1000;
    strike.range_index = 4;
    strike.effects[0].target_a = 6;
    d.spells.spells.insert(strike.id, strike);
    let modifier = |id: u32, kind: u32, amount: i32, charges: u32| {
        let mut s = made_up(id, 6, kind, 4, amount);
        s.family_name = 4;
        s.effects[0].item_type = 0x1000;
        s.duration_index = 21;
        s.proc_charges = charges;
        s
    };
    for s in [
        modifier(990_710, auras::AURA_ADD_FLAT_MODIFIER, -1, 0),
        modifier(990_711, auras::AURA_ADD_FLAT_MODIFIER, -10, 0),
        modifier(990_712, auras::AURA_ADD_PCT_MODIFIER, 25, 0),
        modifier(990_713, auras::AURA_ADD_FLAT_MODIFIER, 1, 1),
    ] {
        d.spells.spells.insert(s.id, s);
    }
}

fn charges(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    holder(m, g, spell).map(|h| h.charges)
}

#[tokio::test]
async fn charge_modifiers_add_to_their_spells_charges_flat_then_percent() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(charge_spells);
    let (_c, w, map) = player(&realm, "P3A11", "Chargeone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[SHIELD_BLOCK, IMPROVED_SHIELD_BLOCK]);
            let kobold = dummy(m, w, 60, 5.0);
            let cast = |m: &mut Map, by: Guid, spell: u32| {
                spells::cast_triggered(m, by, spell, Some(w));
                charges(m, w, spell)
            };
            // Improved Shield Block's charge, and the count the client sees.
            let block = cast(m, w, SHIELD_BLOCK);
            let apps = holder(m, w, SHIELD_BLOCK).and_then(|h| h.slot).map(|slot| {
                m.players[&w]
                    .fields
                    .get_u32(UNIT_FIELD_AURAAPPLICATIONS + (slot / 4) as u16)
                    >> ((slot % 4) * 8)
                    & 0xFF
            });
            // Spent, then cast again; another's cast replaces it, unmodified.
            auras::spend_charge(m, w, SHIELD_BLOCK, w);
            let spent = charges(m, w, SHIELD_BLOCK);
            m.now += Duration::from_secs(2);
            let again = cast(m, w, SHIELD_BLOCK);
            let expires = holder(m, w, SHIELD_BLOCK).map(|h| {
                (
                    h.expires.map(|t| t.saturating_duration_since(m.now)),
                    h.duration.map(|d| Duration::from_millis(u64::from(d))),
                )
            });
            apply(m, kobold, w, SHIELD_BLOCK);
            let others = charges(m, w, SHIELD_BLOCK);
            let held = auras::auras(m, w).map_or(0, |a| {
                a.holders.iter().filter(|h| h.spell == SHIELD_BLOCK).count()
            });
            // Flat, then a quarter more: (2 + 1) * 1.25, truncated.
            apply(m, w, w, 990_712);
            let percent = cast(m, w, 990_700);
            auras::remove_spell(m, w, 990_712, None);
            // A charge-less spell of its flags gets one; one of other flags
            // none.
            let given = cast(m, w, 990_701);
            let other_flags = cast(m, w, 990_702);
            // Less than none left is no limit.
            auras::remove_spell(m, w, IMPROVED_SHIELD_BLOCK, None);
            apply(m, w, w, 990_710);
            let minus_one = cast(m, w, SHIELD_BLOCK);
            auras::remove_spell(m, w, 990_710, None);
            apply(m, w, w, 990_711);
            let minus_ten = cast(m, w, SHIELD_BLOCK);
            auras::remove_spell(m, w, 990_711, None);
            // A charged modifier is spent by the aura it adds a charge to,
            // not by a strike of its flags.
            spells::learn(m, w, 990_700);
            spells::learn(m, w, 990_703);
            apply(m, w, w, 990_713);
            cast_fresh(m, w, 990_703, at(kobold));
            let after_strike = has(m, w, 990_713);
            auras::remove_spell(m, w, 990_700, None);
            cast_fresh(m, w, 990_700, spells::Targets::default());
            let charged = (charges(m, w, 990_700), has(m, w, 990_713));
            (
                block,
                apps,
                spent,
                again,
                expires,
                others,
                held,
                percent,
                given,
                other_flags,
                minus_one,
                minus_ten,
                after_strike,
                charged,
            )
        })
        .await
        .unwrap();
    let (
        block,
        apps,
        spent,
        again,
        expires,
        others,
        held,
        percent,
        given,
        other_flags,
        minus_one,
        minus_ten,
        after_strike,
        charged,
    ) = out;
    assert_eq!(block, Some(2), "Improved Shield Block's extra charge");
    assert_eq!(apps, Some(1), "the client counts two");
    assert_eq!(spent, Some(1), "one spent");
    assert_eq!(again, Some(2), "cast again, two");
    let (left, full) = expires.unwrap();
    assert_eq!(left, full, "and its whole time again");
    assert_eq!(
        (others, held),
        (Some(1), 1),
        "another's replaces it, unmodified"
    );
    assert_eq!(percent, Some(3), "(2 + 1) and a quarter, truncated");
    assert_eq!(given, Some(1), "a charge-less spell of its flags gets one");
    assert_eq!(other_flags, Some(1), "one of other flags keeps its own");
    assert_eq!(minus_one, Some(0), "none left is no limit");
    assert_eq!(minus_ten, Some(0), "nor less");
    assert!(after_strike, "a strike doesn't spend a charge modifier");
    assert_eq!(
        charged,
        (Some(3), false),
        "the aura does, and has its charge"
    );
}
