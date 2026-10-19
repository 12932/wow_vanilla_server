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
#[ignore = "extended"]
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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
        ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(u) if u.guid == druid => Some(()),
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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

const NIGHTFALL: u32 = 18094;
const SHADOW_TRANCE: u32 = 17941;
const CORRUPTION: u32 = 172;
const SHADOW_BOLT: u32 = 686;
const FIREBALL: u32 = 133;
const IMPROVED_BLIZZARD: u32 = 11185;
const CHILLED: u32 = 12484;
const FROSTBOLT: u32 = 116;

/// Nightfall sure to go off, with a second's cooldown.
fn sure_nightfall(d: &mut crate::game::data::GameData) {
    if let Some(p) = d.spells.proc_events.get_mut(&NIGHTFALL) {
        p.chance = 100.0;
        p.cooldown = 1000;
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn nightfall_goes_off_on_corruptions_ticks_alone_by_its_charges_and_cooldown() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(sure_nightfall);
    let (_c, lock, map) = player(&realm, "P3A12", "Nightone", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, lock, 60, &[NIGHTFALL]);
            let kobold = dummy(m, lock, 60, 5.0);
            let trance = |m: &mut Map| {
                let had = has(m, lock, SHADOW_TRANCE);
                auras::remove_spell(m, lock, SHADOW_TRANCE, None);
                had
            };
            // A Shadow Bolt and a Fireball: nothing.
            spells::cast_triggered(m, lock, SHADOW_BOLT, Some(kobold));
            spells::cast_triggered(m, lock, FIREBALL, Some(kobold));
            let direct = trance(m);
            // Corruption's tick: Shadow Trance on the warlock.
            spells::cast_triggered(m, lock, CORRUPTION, Some(kobold));
            tick_now(m, kobold, CORRUPTION);
            let first = trance(m);
            // Two charges: the cooldown holds a second, then the last goes.
            if let Some(a) = auras::auras_mut(m, lock)
                && let Some(h) = a.holders.iter_mut().find(|h| h.spell == NIGHTFALL)
            {
                h.charges = 2;
                h.proc_ready_at = None;
            }
            tick_now(m, kobold, CORRUPTION);
            let charged = (trance(m), holder(m, lock, NIGHTFALL).map(|h| h.charges));
            m.now += Duration::from_millis(999);
            tick_now(m, kobold, CORRUPTION);
            let cooling = trance(m);
            m.now += Duration::from_millis(1);
            tick_now(m, kobold, CORRUPTION);
            let last = (trance(m), has(m, lock, NIGHTFALL));
            (direct, first, charged, cooling, last)
        })
        .await
        .unwrap();
    let (direct, first, charged, cooling, last) = out;
    assert!(
        !direct,
        "no trance from a direct spell, nor another family's"
    );
    assert!(first, "Corruption's tick gives Shadow Trance");
    assert_eq!(charged, (true, Some(1)), "a charge spent");
    assert!(!cooling, "nothing within its cooldown");
    assert_eq!(last, (true, false), "then the last charge, and it is gone");
}

#[tokio::test]
#[ignore = "extended"]
async fn improved_blizzard_chills_from_blizzard_alone() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3A13", "Chillone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[IMPROVED_BLIZZARD, BLIZZARD]);
            let kobold = dummy(m, mage, 60, 10.0);
            spells::cast_triggered(m, mage, FROSTBOLT, Some(kobold));
            let by_frostbolt = has(m, kobold, CHILLED);
            // Frostbolt's own chill, stronger, would keep Chilled off.
            auras::remove_spell(m, kobold, FROSTBOLT, None);
            let t = spells::Targets {
                dest: m.position_of(kobold),
                ..Default::default()
            };
            cast_fresh(m, mage, BLIZZARD, t);
            pass(m, 1_100);
            let by_blizzard = has(m, kobold, CHILLED);
            spells::interrupt(m, mage);
            (by_frostbolt, by_blizzard)
        })
        .await
        .unwrap();
    assert_eq!(out, (false, true), "Chilled from Blizzard, not Frostbolt");
}

const MEND_PET: u32 = 136;
const IMPROVED_MEND_PET: u32 = 19572;
const IMPROVED_MEND_PET_CLEANSE: u32 = 24406;
const REJUVENATION: u32 = 774;
const REGROWTH: u32 = 8936;
const DREAMWALKER_REJUVENATION: u32 = 28716;
const DREAMWALKER_MANA: u32 = 28722;
const DREAMWALKER_REGROWTH: u32 = 28744;
const BLESSING_OF_THE_CLAW: u32 = 28750;
const CORRUPTED_HEALING: u32 = 23401;
const CORRUPTED_HEALING_HURT: u32 = 23402;
const LESSER_HEAL: u32 = 2050;
const RENEW: u32 = 139;

/// Improved Mend Pet's and the Dreamwalker's chances made sure.
fn sure_heal_scripts(d: &mut crate::game::data::GameData) {
    if let Some(s) = d.spells.spells.get_mut(&IMPROVED_MEND_PET) {
        s.effects[0].base_points = 99;
    }
    if let Some(p) = d.spells.proc_events.get_mut(&DREAMWALKER_REJUVENATION) {
        p.chance = 100.0;
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn heals_over_time_go_off_by_their_ticks_and_direct_heals_by_their_cast() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(sure_heal_scripts);
    let (mut c, hunter, map) = player(&realm, "P3A14", "Mendone", Race::Dwarf, Class::Hunter).await;
    let (mut d, druid, _) = player(&realm, "P3A15", "Mendtwo", Race::Human, Class::Paladin).await;
    let (_e, priest, _) = player(&realm, "P3A16", "Mendthree", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, hunter, 60, &[IMPROVED_MEND_PET]);
            ready(m, druid, 60, &[]);
            ready(m, priest, 60, &[]);
            let wolf = dummy(m, hunter, 60, 3.0);
            // Mend Pet's tick: Improved Mend Pet's cleanse.
            apply(m, hunter, wolf, MEND_PET);
            tick_now(m, wolf, MEND_PET);
            // Rejuvenation's tick on one of mana: the Dreamwalker's mana;
            // Regrowth's cast: the Blessing of the Claw.
            apply(m, druid, druid, DREAMWALKER_REJUVENATION);
            apply(m, druid, druid, DREAMWALKER_REGROWTH);
            apply(m, druid, priest, REJUVENATION);
            tick_now(m, priest, REJUVENATION);
            spells::cast_triggered(m, druid, REGROWTH, Some(priest));
            let claw = has(m, priest, BLESSING_OF_THE_CLAW);
            // Corrupted Healing: a direct heal hurts, a heal over time
            // doesn't.
            apply(m, priest, priest, CORRUPTED_HEALING);
            apply(m, priest, priest, RENEW);
            tick_now(m, priest, RENEW);
            let by_renew = has(m, priest, CORRUPTED_HEALING_HURT);
            spells::cast_triggered(m, priest, LESSER_HEAL, Some(priest));
            let by_heal = has(m, priest, CORRUPTED_HEALING_HURT);
            (claw, by_renew, by_heal)
        })
        .await
        .unwrap();
    let (claw, by_renew, by_heal) = out;
    assert!(
        saw_go(&mut c, IMPROVED_MEND_PET_CLEANSE).await,
        "Mend Pet's tick cleanses"
    );
    assert!(
        saw_go(&mut d, DREAMWALKER_MANA).await,
        "Rejuvenation's tick gives mana"
    );
    assert!(claw, "Regrowth gives the Blessing of the Claw");
    assert!(!by_renew, "Renew's tick doesn't corrupt");
    assert!(by_heal, "a direct heal does");
}

/// The spells of `g`'s holders, in order.
fn spells_on(m: &Map, g: Guid) -> Vec<u32> {
    auras::auras(m, g)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| !h.passive)
                .map(|h| h.spell)
                .collect()
        })
        .unwrap_or_default()
}

fn clear(m: &mut Map, g: Guid) {
    auras::remove_where(m, g, |h| !h.passive);
}

#[tokio::test]
#[ignore = "extended"]
async fn food_drink_well_fed_and_flasks_replace_their_own_kind() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P3A17", "Eatone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let kobold = dummy(m, w, 60, 5.0);
            let mut seen = Vec::new();
            let run = |m: &mut Map, list: &[(Guid, u32)]| {
                clear(m, w);
                for (by, s) in list {
                    apply(m, *by, w, *s);
                }
                spells_on(m, w)
            };
            // A food replaces a food, a drink a drink; food and drink both,
            // and both are replaced by either.
            seen.push(run(m, &[(w, 433), (w, 430), (w, 434)]));
            seen.push(run(m, &[(w, 430), (w, 434), (w, 431)]));
            seen.push(run(m, &[(w, 434), (w, 431), (w, 2639)]));
            seen.push(run(m, &[(w, 2639), (w, 434)]));
            seen.push(run(m, &[(w, 2639), (w, 431)]));
            // Well Fed from two: the last.
            seen.push(run(m, &[(w, 24799), (kobold, 19705)]));
            // Flasks replace flasks; plain elixirs and Well Fed stay.
            seen.push(run(
                m,
                &[(w, 17628), (w, 11348), (w, 24799), (w, 17627), (w, 11396)],
            ));
            // Standing ends food, not Well Fed.
            run(m, &[(w, 2639), (w, 24799)]);
            auras::interrupt(m, w, auras::INTERRUPT_NOT_SEATED);
            let stood = spells_on(m, w);
            // A food that gives Well Fed, replaced, gives nothing.
            run(m, &[(w, 5005), (w, 435)]);
            pass(m, 11_000);
            let replaced = spells_on(m, w);
            // Death keeps the flask that outlasts it.
            run(m, &[(w, 17628), (w, 24799), (w, 11348)]);
            crate::game::map::death::kill(m, w);
            let died = spells_on(m, w);
            (seen, stood, replaced, died)
        })
        .await
        .unwrap();
    let (seen, stood, replaced, died) = out;
    assert_eq!(
        seen,
        vec![
            vec![430, 434],
            vec![434, 431],
            vec![2639],
            vec![434],
            vec![431],
            vec![19705],
            vec![11348, 24799, 17627, 11396],
        ]
    );
    assert_eq!(stood, vec![24799], "standing ends the food alone");
    assert_eq!(
        replaced,
        vec![435],
        "the replaced food's Well Fed never comes"
    );
    assert_eq!(died, vec![17628], "death keeps the flask alone");
}

const LINKENS_BOOMERANG: u32 = 15712;
const DISARM: u32 = 15752;
const KNOCKDOWN: u32 = 15753;
const SCORPID_SURPRISE: u32 = 6410;
const SCORPID_POISON: u32 = 6411;
const DIVINE_SHIELD: u32 = 642;

#[tokio::test]
#[ignore = "extended"]
async fn a_boomerang_and_a_scorpid_surprise_roll_their_rare_extras() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P3A18", "Rareone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let kobold = dummy(m, w, 60, 5.0);
            let (mut disarmed, mut knocked, mut poisoned) = (0, 0, 0);
            for _ in 0..620 {
                spells::cast_triggered(m, w, LINKENS_BOOMERANG, Some(kobold));
                disarmed += has(m, kobold, DISARM) as u32;
                knocked += has(m, kobold, KNOCKDOWN) as u32;
                clear(m, kobold);
                // Each a first knockdown, not a diminished one.
                if let Some(c) = m.caster_mut(kobold) {
                    c.diminishing.clear();
                }
                spells::cast_triggered(m, w, SCORPID_SURPRISE, Some(w));
                poisoned += has(m, w, SCORPID_POISON) as u32;
                clear(m, w);
                set_health(m, kobold, 1_000_000);
            }
            // On one shielded, nothing; on the dead, nothing.
            apply(m, kobold, kobold, DIVINE_SHIELD);
            let before = health(m, kobold);
            for _ in 0..40 {
                spells::cast_triggered(m, w, LINKENS_BOOMERANG, Some(kobold));
            }
            let shielded = (
                health(m, kobold) == before,
                has(m, kobold, DISARM) || has(m, kobold, KNOCKDOWN),
            );
            clear(m, kobold);
            set_health(m, kobold, 5);
            spells::cast_triggered(m, w, LINKENS_BOOMERANG, Some(kobold));
            let killed = (
                m.creature(kobold).is_none_or(|c| !c.alive()),
                has(m, kobold, DISARM) || has(m, kobold, KNOCKDOWN),
            );
            (disarmed, knocked, poisoned, shielded, killed)
        })
        .await
        .unwrap();
    let (disarmed, knocked, poisoned, shielded, killed) = out;
    assert!(
        (5..=45).contains(&disarmed),
        "disarmed one in 31: {disarmed} of 620"
    );
    assert!(
        (25..=100).contains(&knocked),
        "knocked down one in 11: {knocked} of 620"
    );
    assert!(
        (25..=100).contains(&poisoned),
        "poisoned one in 11: {poisoned} of 620"
    );
    assert_eq!(shielded, (true, false), "Divine Shield takes nothing");
    assert_eq!(
        killed,
        (true, false),
        "the killing hit leaves nothing on the dead"
    );
}

const MIND_CONTROL: u32 = 605;

#[tokio::test]
#[ignore = "extended"]
async fn mind_control_takes_a_creature_and_lets_it_go_on_its_master_unless_dead() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, priest, map) = player(&realm, "P3A19", "Mindone", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, priest, 60, &[MIND_CONTROL]);
            let kobold = dummy(m, priest, 60, 5.0);
            let fields = |m: &Map, g: Guid| {
                m.unit_fields(g).map(|f| {
                    (
                        f.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                        f.get_u32(UNIT_FIELD_FLAGS) & 0x0100_0008,
                    )
                })
            };
            let before = fields(m, kobold);
            spells::cast_triggered(m, priest, MIND_CONTROL, Some(kobold));
            let held = fields(m, kobold);
            let mine = m
                .unit_fields(priest)
                .map(|f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE));
            // Let go: as it was, and on the priest with its health's threat.
            spells::interrupt(m, priest);
            crate::game::map::pets::update(m);
            let freed = fields(m, kobold);
            let c = m.creature(kobold).unwrap();
            let turned = (
                c.combat.victim == priest,
                c.threat.iter().find(|(t, _)| *t == priest).map(|(_, v)| *v),
            );
            // One killed while held turns on no one.
            let other = dummy(m, priest, 60, 6.0);
            fresh(m, priest);
            spells::cast_triggered(m, priest, MIND_CONTROL, Some(other));
            crate::game::map::combat::deal_damage(m, priest, other, 2_000_000);
            crate::game::map::pets::update(m);
            let dead = m
                .creature(other)
                .map(|c| (c.alive(), c.combat.victim == priest));
            (before, held, mine, freed, turned, dead)
        })
        .await
        .unwrap();
    let (before, held, mine, freed, turned, dead) = out;
    let (faction, flags) = before.unwrap();
    assert_eq!(flags, 0, "a creature of its own");
    assert_eq!(
        held,
        Some((mine.unwrap(), 0x0100_0008)),
        "the priest's, possessed"
    );
    assert_eq!(freed, Some((faction, 0)), "let go, as it was");
    assert_eq!(
        turned,
        (true, Some(1_000_000.0)),
        "and on the priest with its health's threat"
    );
    assert_eq!(
        dead,
        Some((false, false)),
        "killed while held, it turns on no one"
    );
}

const THICKSKULL: u32 = 829;
const FEARLESS: u32 = 830;
const HUNTERS_WILL: u32 = 1542;
const HAMMER_OF_JUSTICE: u32 = 853;
const HAND_OF_THAURISSAN: u32 = 17492;
const FEAR: u32 = 5782;

fn stunned(m: &Map, g: Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOD_STUN))
}

#[tokio::test]
#[ignore = "extended"]
async fn an_immunity_to_an_auras_kind_keeps_it_off_and_a_purging_one_takes_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, pally, map) =
        player(&realm, "P3A20", "Stateone", Race::Human, Class::Paladin).await;
    let out = map
        .query(move |m| {
            ready(m, pally, 60, &[]);
            let ogre = dummy(m, pally, 60, 5.0);
            // Thickskull: no stun lands, a stun with damage hurts alone.
            apply(m, ogre, ogre, THICKSKULL);
            spells::cast_triggered(m, pally, HAMMER_OF_JUSTICE, Some(ogre));
            let hammered = stunned(m, ogre);
            let before = health(m, ogre);
            spells::cast_triggered(m, pally, HAND_OF_THAURISSAN, Some(ogre));
            let hand = (health(m, ogre) < before, stunned(m, ogre));
            // Fearless keeps the fear off, not the rest of Fear.
            apply(m, ogre, ogre, FEARLESS);
            spells::cast_triggered(m, pally, FEAR, Some(ogre));
            let feared = auras::auras(m, ogre).is_some_and(|a| a.has(auras::AURA_MOD_FEAR));
            // Two of them: one gone, still kept off; both gone, it lands.
            apply(m, pally, ogre, HUNTERS_WILL);
            auras::remove_spell(m, ogre, THICKSKULL, None);
            spells::cast_triggered(m, pally, HAMMER_OF_JUSTICE, Some(ogre));
            let one_left = stunned(m, ogre);
            auras::remove_spell(m, ogre, HUNTERS_WILL, None);
            spells::cast_triggered(m, pally, HAMMER_OF_JUSTICE, Some(ogre));
            let none_left = stunned(m, ogre);
            // A stun on: a plain immunity leaves it, a purging one takes it.
            apply(m, ogre, ogre, THICKSKULL);
            let kept = stunned(m, ogre);
            auras::remove_spell(m, ogre, THICKSKULL, None);
            apply(m, pally, ogre, HUNTERS_WILL);
            let purged = stunned(m, ogre);
            (hammered, hand, feared, one_left, none_left, kept, purged)
        })
        .await
        .unwrap();
    let (hammered, hand, feared, one_left, none_left, kept, purged) = out;
    assert!(!hammered, "Thickskull keeps the Hammer of Justice off");
    assert_eq!(
        missed(&mut c, HAMMER_OF_JUSTICE).await,
        7,
        "immune, the caster is told"
    );
    assert_eq!(
        hand,
        (true, false),
        "the Hand's damage lands, its stun doesn't"
    );
    assert!(!feared, "Fearless keeps the fear off");
    assert!(!one_left, "one immunity left keeps it off");
    assert!(none_left, "none left, it lands");
    assert!(kept, "a plain immunity leaves a stun on");
    assert!(!purged, "a purging one takes it");
}

const FAERIE_FIRE_FERAL: u32 = 16857;
const STEALTH: u32 = 1784;
const LESSER_INVISIBILITY: u32 = 66;

#[tokio::test]
#[ignore = "extended"]
async fn faerie_fire_takes_stealth_and_invisibility_and_keeps_them_off() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, rogue, map) = player(&realm, "P3A21", "Fireone", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, rogue, 60, &[STEALTH]);
            let druid = dummy(m, rogue, 60, 5.0);
            spells::cast_triggered(m, rogue, STEALTH, None);
            apply(m, rogue, rogue, LESSER_INVISIBILITY);
            let hidden = (has(m, rogue, STEALTH), has(m, rogue, LESSER_INVISIBILITY));
            apply(m, druid, rogue, FAERIE_FIRE);
            let lit = (has(m, rogue, STEALTH), has(m, rogue, LESSER_INVISIBILITY));
            // Under it, no stealth; after it, stealth again.
            spells::cast_triggered(m, rogue, STEALTH, None);
            let under = has(m, rogue, STEALTH);
            auras::remove_spell(m, rogue, FAERIE_FIRE, None);
            apply(m, druid, rogue, FAERIE_FIRE_FERAL);
            spells::cast_triggered(m, rogue, STEALTH, None);
            let feral = has(m, rogue, STEALTH);
            auras::remove_spell(m, rogue, FAERIE_FIRE_FERAL, None);
            spells::cast_triggered(m, rogue, STEALTH, None);
            let after = has(m, rogue, STEALTH);
            (hidden, lit, under, feral, after)
        })
        .await
        .unwrap();
    let (hidden, lit, under, feral, after) = out;
    assert_eq!(hidden, (true, true), "stealthed and invisible");
    assert_eq!(lit, (false, false), "Faerie Fire takes both");
    assert!(!under, "and keeps stealth off");
    assert!(!feral, "as its feral kind does");
    assert!(after, "gone, stealth again");
}

/// Made-up slows: snares of -40 for 20 s, -70 for 5 s, -50 for 20 s and
/// -50 for 4 s; attack-speed slows of -10 for 30 s and -20 for 10 s.
fn slow_spells(d: &mut crate::game::data::GameData) {
    let slow = |id: u32, kind: u32, amount: i32, duration: u32| {
        let mut s = made_up(id, 6, kind, 0, amount);
        s.duration_index = duration;
        s
    };
    for s in [
        slow(990_900, auras::AURA_MOD_DECREASE_SPEED, -40, 18),
        slow(990_901, auras::AURA_MOD_DECREASE_SPEED, -70, 28),
        slow(990_902, auras::AURA_MOD_DECREASE_SPEED, -50, 18),
        slow(990_903, auras::AURA_MOD_DECREASE_SPEED, -50, 35),
        slow(990_904, auras::AURA_MOD_HASTE, -10, 2),
        slow(990_905, auras::AURA_MOD_HASTE, -20, 1),
    ] {
        d.spells.spells.insert(s.id, s);
    }
}

/// Snares stack, as mangos-classic has them: each stays on as long as it
/// lasts, and the strongest slows (the user's #27). Attack-speed slows
/// stack too, a weaker or shorter one beside a stronger (#30; Thistle
/// Tea's `Slow`, after vmangos, passes it over).
#[tokio::test]
#[ignore = "extended"]
async fn snares_and_attack_speed_slows_stack_and_the_strongest_snare_slows() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(slow_spells);
    let (_c, w, map) = player(&realm, "P3A22", "Slowone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let a = dummy(m, w, 60, 5.0);
            let b = dummy(m, w, 60, 6.0);
            let on = |m: &mut Map, list: &[(Guid, u32)]| {
                clear(m, w);
                for (by, s) in list {
                    apply(m, *by, w, *s);
                }
                let mut on = spells_on(m, w);
                on.sort_unstable();
                (on, run_speed(m, w))
            };
            let both = on(m, &[(a, 990_900), (b, 990_901)]);
            let either_order = on(m, &[(a, 990_901), (b, 990_900)]);
            let haste = on(m, &[(a, 990_904), (b, 990_905), (a, 990_904)]).0;
            // One snare gone, the other slows on.
            clear(m, w);
            apply(m, a, w, 990_900);
            apply(m, b, w, 990_901);
            auras::remove_spell(m, w, 990_901, None);
            let left = (spells_on(m, w), run_speed(m, w));
            // Frostbolt on one already snared: its chill goes on too.
            clear(m, w);
            apply(m, a, w, 990_901);
            let before = health(m, w);
            spells::cast_triggered(m, a, FROSTBOLT, Some(w));
            let frostbolt = (health(m, w) < before, spells_on(m, w).len());
            (both, either_order, haste, left, frostbolt)
        })
        .await
        .unwrap();
    let (both, either_order, haste, left, frostbolt) = out;
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    assert_eq!(both.0, vec![990_900, 990_901], "both snares on");
    assert!(
        near(both.1, 7.0 * 0.3),
        "the strongest, -70%, slows: {}",
        both.1
    );
    assert_eq!(either_order, both, "whichever came first");
    assert_eq!(
        haste,
        vec![990_904, 990_905],
        "the attack-speed slows stand together"
    );
    assert_eq!(left.0, vec![990_900], "the other stays");
    assert!(near(left.1, 7.0 * 0.6), "and slows by its -40%: {}", left.1);
    assert_eq!(
        frostbolt,
        (true, 2),
        "Frostbolt hurts and chills beside the snare"
    );
}

const CAT_FORM: u32 = 768;
const BEAR_FORM: u32 = 5487;
const TRAVEL_FORM: u32 = 783;
const AQUATIC_FORM: u32 = 1066;
const MOONKIN_FORM: u32 = 24858;
const DIRE_BEAR_FORM: u32 = 9634;
const GHOST_WOLF: u32 = 2645;
const BATTLE_STANCE: u32 = 2457;
const SHADOWFORM: u32 = 15473;
const WATER_WALKING: u32 = 546;
const DAZED: u32 = 1604;

fn held_down(m: &Map, g: Guid) -> (bool, bool) {
    auras::auras(m, g).map_or((false, false), |a| {
        (
            a.has(auras::AURA_MOD_ROOT),
            a.has(auras::AURA_MOD_DECREASE_SPEED),
        )
    })
}

#[tokio::test]
#[ignore = "extended"]
async fn a_shapeshift_frees_from_roots_and_snares_and_a_stance_doesnt() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, druid, map) = player(&realm, "P3A23", "Shiftone", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            ready(m, druid, 60, &[]);
            let kobold = dummy(m, druid, 60, 5.0);
            let held = |m: &mut Map, form: u32| {
                clear(m, druid);
                apply(m, kobold, druid, ENTANGLING_ROOTS);
                spells::cast_triggered(m, kobold, FROSTBOLT, Some(druid));
                apply(m, druid, druid, WATER_WALKING);
                let before = held_down(m, druid);
                spells::cast_triggered(m, druid, form, None);
                (before, held_down(m, druid), has(m, druid, WATER_WALKING))
            };
            let forms: Vec<_> = [
                CAT_FORM,
                BEAR_FORM,
                TRAVEL_FORM,
                AQUATIC_FORM,
                MOONKIN_FORM,
                DIRE_BEAR_FORM,
            ]
            .into_iter()
            .map(|f| held(m, f))
            .collect();
            let stances: Vec<_> = [GHOST_WOLF, BATTLE_STANCE, SHADOWFORM, STEALTH]
                .into_iter()
                .map(|f| held(m, f))
                .collect();
            // Rooted in cat form, the form again: still rooted.
            clear(m, druid);
            spells::cast_triggered(m, druid, CAT_FORM, None);
            apply(m, kobold, druid, ENTANGLING_ROOTS);
            spells::cast_triggered(m, druid, CAT_FORM, None);
            let again = has(m, druid, ENTANGLING_ROOTS);
            // A daze isn't one a form frees from.
            clear(m, druid);
            apply(m, kobold, druid, DAZED);
            spells::cast_triggered(m, druid, BEAR_FORM, None);
            let dazed = has(m, druid, DAZED);
            (forms, stances, again, dazed)
        })
        .await
        .unwrap();
    let (forms, stances, again, dazed) = out;
    // Moonkin's a stance to Water Walking, not to a root.
    for (i, f) in forms.iter().enumerate() {
        let walking = i == 4;
        assert_eq!(
            *f,
            ((true, true), (false, false), walking),
            "form {i} frees"
        );
    }
    // Ghost Wolf's a shape to Water Walking, not to a root.
    for (i, s) in stances.iter().enumerate() {
        let walking = i != 0;
        assert_eq!(
            *s,
            ((true, true), (true, true), walking),
            "stance {i} keeps them"
        );
    }
    assert!(again, "the form again keeps a root had in it");
    assert!(dazed, "a daze stays");
}

const PROWL: u32 = 5215;
const SHADOWMELD: u32 = 20580;
const RAVAGE: u32 = 6785;
const ASPECT_OF_THE_BEAST: u32 = 13161;

/// `g`'s form byte and visibility byte.
fn form_and_vis(m: &Map, g: Guid) -> (u8, u8) {
    m.unit_fields(g).map_or((0, 0), |f| {
        (
            f.get_byte(UNIT_FIELD_BYTES_1, 2),
            f.get_byte(UNIT_FIELD_BYTES_1, 3),
        )
    })
}

#[tokio::test]
#[ignore = "extended"]
async fn stealth_marks_the_visibility_byte_not_the_form() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, druid, map) = player(&realm, "P3A24", "Prowlone", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            ready(m, druid, 60, &[]);
            spells::cast_triggered(m, druid, CAT_FORM, None);
            spells::cast_triggered(m, druid, PROWL, None);
            let prowling = form_and_vis(m, druid);
            let ravage = m.realm.data.spells.get(RAVAGE).cloned().unwrap();
            let ravage_ok = spells::checks::form_error(&ravage, auras::form(m, druid));
            auras::remove_spell(m, druid, PROWL, None);
            let after = form_and_vis(m, druid);
            clear(m, druid);
            spells::cast_triggered(m, druid, SHADOWMELD, None);
            let melded = form_and_vis(m, druid);
            clear(m, druid);
            apply(m, druid, druid, ASPECT_OF_THE_BEAST);
            let untrackable = form_and_vis(m, druid);
            clear(m, druid);
            let plain = form_and_vis(m, druid);
            // The player's own flags: stealthed, invisible, both, neither.
            let bytes2 = |m: &Map| m.players[&druid].fields.get_byte(PLAYER_FIELD_BYTES2, 1) & 0x60;
            spells::cast_triggered(m, druid, PROWL, None);
            let stealthed = bytes2(m);
            apply(m, druid, druid, LESSER_INVISIBILITY);
            let both = bytes2(m);
            auras::remove_spell(m, druid, PROWL, None);
            let invisible = bytes2(m);
            clear(m, druid);
            let neither = bytes2(m);
            (
                prowling,
                ravage_ok,
                after,
                melded,
                untrackable,
                plain,
                (stealthed, both, invisible, neither),
            )
        })
        .await
        .unwrap();
    let (prowling, ravage_ok, after, melded, untrackable, plain, glows) = out;
    assert_eq!(
        glows,
        (0x20, 0x60, 0x40, 0),
        "stealthed, and invisible, to their own client"
    );
    assert_eq!(prowling.0, auras::FORM_CAT, "still a cat, prowling");
    assert_eq!(prowling.1 & 0x02, 0x02, "creeping");
    assert_eq!(ravage_ok, None, "and Ravage is a cat's");
    assert_eq!(after, (auras::FORM_CAT, plain.1), "no longer creeping");
    assert_eq!(melded, (0, plain.1 | 0x02), "Shadowmeld creeps in no form");
    assert_eq!(
        untrackable.1 & 0x04,
        0x04,
        "Aspect of the Beast's untrackable"
    );
    assert_eq!(plain.1 & 0x06, 0, "and none of it after");
}

#[test]
fn stealth_is_seen_within_a_distance_of_levels_detection_and_skill() {
    use crate::game::map::spells::stealth::detection_distance as d;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    // A level 10 player with 50 detection against a level 10 rogue at 50:
    // 24 yards in front, 15 behind; at level 60, capped at 30, 21 behind.
    assert!(near(d(10, true, true, 10, 50, 50, false), 24.0));
    assert!(near(d(10, true, true, 10, 50, 50, true), 15.0));
    assert!(near(d(60, true, true, 10, 50, 50, false), 30.0));
    assert!(near(d(60, true, true, 10, 50, 50, true), 21.0));
    // Three levels above, 13.5; four, the skill counts twice: 21.
    assert!(near(d(13, true, true, 10, 0, 50, false), 13.5));
    assert!(near(d(14, true, true, 10, 0, 50, false), 21.0));
    // A level 10 creature: five sixths of a yard.
    assert!(near(d(10, false, true, 10, 0, 50, false), 5.0 / 6.0));
    // Level 10 players at 50: nine in front, nothing behind.
    assert!(near(d(10, true, true, 10, 0, 50, false), 9.0));
    assert!(d(10, true, true, 10, 0, 50, true) <= 0.0);
}

const DETECT_LESSER_INVISIBILITY: u32 = 132;
const HUNTERS_MARK: u32 = 1130;

/// `target` `yards` before `viewer`, facing away or towards.
fn place(m: &mut Map, viewer: Guid, target: Guid, yards: f32) {
    let (p, o) = {
        let v = m.unit_view(viewer).unwrap();
        (v.position, v.orientation)
    };
    let at = Vector3d {
        x: p.x + yards * o.cos(),
        y: p.y + yards * o.sin(),
        z: p.z,
    };
    if let Some(x) = m.players.get_mut(&target) {
        x.movement.position = at;
    } else if let Some(c) = m.creature_mut(target) {
        c.position = at;
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn who_sees_through_stealth_and_invisibility() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, rogue, map) = player(&realm, "P3A25", "Hideone", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, rogue, 60, &[]);
            let kobold = dummy(m, rogue, 60, 5.0);
            let sees = |m: &Map, v: Guid| crate::game::map::spells::stealth::sees(m, v, rogue);
            spells::cast_triggered(m, rogue, STEALTH, None);
            // A level 60 kobold: within a bump, seen; at four yards, not.
            place(m, kobold, rogue, 1.4);
            let bumped = sees(m, kobold);
            place(m, kobold, rogue, 4.0);
            let apart = sees(m, kobold);
            // Its Hunter's Mark on the rogue: its marker sees it.
            apply(m, kobold, rogue, HUNTERS_MARK);
            let marked = sees(m, kobold);
            auras::remove_spell(m, rogue, HUNTERS_MARK, None);
            // Stunned, it sees no one stealthed, even close.
            place(m, kobold, rogue, 1.0);
            apply(m, kobold, kobold, HAMMER_OF_JUSTICE);
            let stunned = sees(m, kobold);
            clear(m, kobold);
            // Invisible too: no bump sees it; detection as strong does.
            apply(m, rogue, rogue, LESSER_INVISIBILITY);
            let invisible_close = sees(m, kobold);
            apply(m, kobold, kobold, DETECT_LESSER_INVISIBILITY);
            let lesser = sees(m, kobold);
            // A stronger one of another spell beside it; gone, the first
            // still sees.
            let mut strong = made_up(990_970, 6, auras::AURA_MOD_INVISIBILITY_DETECTION, 0, 1000);
            strong.duration_index = 21;
            apply_template(m, kobold, kobold, &strong);
            let detected = sees(m, kobold);
            auras::remove_spell(m, kobold, 990_970, None);
            let after = sees(m, kobold);
            (
                bumped,
                apart,
                marked,
                stunned,
                invisible_close,
                lesser,
                detected,
                after,
            )
        })
        .await
        .unwrap();
    let (bumped, apart, marked, stunned, invisible_close, lesser, detected, after) = out;
    assert!(bumped, "within a bump, seen");
    assert!(!apart, "four yards off, not");
    assert!(marked, "its marker sees it");
    assert!(!stunned, "a stunned viewer sees no one stealthed");
    assert!(!invisible_close, "the invisible aren't bumped into");
    assert!(
        lesser,
        "Detect Lesser Invisibility sees Lesser Invisibility"
    );
    assert!(detected, "as a stronger detection does");
    assert!(after, "the one left still sees it");
}

const CHALLENGING_SHOUT: u32 = 1161;
const TAUNT: u32 = 355;
const MOCKING_BLOW: u32 = 694;

fn threat_of(m: &Map, c: Guid, g: Guid) -> f32 {
    m.creature(c)
        .and_then(|c| c.threat.iter().find(|(t, _)| *t == g).map(|(_, v)| *v))
        .unwrap_or(-1.0)
}

fn victim(m: &Map, c: Guid) -> Guid {
    m.creature(c).map(|c| c.combat.victim).unwrap_or_default()
}

#[tokio::test]
#[ignore = "extended"]
async fn a_taunt_borrows_threat_while_it_lasts_and_the_newest_holds() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_a, w1, map) = player(&realm, "P3A26", "Tauntone", Race::Human, Class::Warrior).await;
    let (_b, w2, _) = player(&realm, "P3A27", "Taunttwo", Race::Human, Class::Warrior).await;
    let (_c, w3, _) = player(&realm, "P3A28", "Tauntthree", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            for w in [w1, w2, w3] {
                ready(m, w, 60, &[]);
            }
            let kobold = dummy(m, w1, 60, 3.0);
            for (w, t) in [(w1, 500.0), (w2, 50.0), (w3, 600.0)] {
                crate::game::map::ai::add_threat(m, kobold, w, t);
            }
            if let Some(c) = m.creature_mut(kobold) {
                c.combat.victim = w1;
            }
            // The shout: level with the victim, on it.
            apply(m, w2, kobold, CHALLENGING_SHOUT);
            let shouted = (threat_of(m, kobold, w2), victim(m, kobold));
            // Earned meanwhile stays; refreshed, nothing more borrowed.
            crate::game::map::ai::add_threat(m, kobold, w2, 25.0);
            m.now += Duration::from_secs(1);
            apply(m, w2, kobold, CHALLENGING_SHOUT);
            let refreshed = threat_of(m, kobold, w2);
            // A Taunt from the third, newer: on it; at its end, back to the
            // shout's.
            m.now += Duration::from_secs(1);
            spells::cast_triggered(m, w3, TAUNT, Some(kobold));
            let taunted = victim(m, kobold);
            pass(m, 3_100);
            // Its choice at the next turn of its fight.
            let back = crate::game::map::ai::overtake(m, kobold, victim(m, kobold));
            // At the shout's end, the loan goes back.
            pass(m, 3_000);
            let ended = (threat_of(m, kobold, w2), has(m, kobold, CHALLENGING_SHOUT));
            // The shout, then Mocking Blow's shorter taunt, both the second's:
            // the loan stays while either does.
            apply(m, w2, kobold, CHALLENGING_SHOUT);
            let lent = threat_of(m, kobold, w2);
            m.now += Duration::from_secs(1);
            apply(m, w2, kobold, MOCKING_BLOW);
            pass(m, 4_100);
            let during = threat_of(m, kobold, w2);
            pass(m, 2_000);
            let after = threat_of(m, kobold, w2);
            // A dead creature borrows nothing.
            let dead = dummy(m, w1, 60, 4.0);
            crate::game::map::ai::add_threat(m, dead, w2, 10.0);
            crate::game::map::combat::deal_damage(m, w1, dead, 2_000_000);
            apply(m, w2, dead, CHALLENGING_SHOUT);
            let corpse = m.spells.temp_threat.contains_key(&(dead, w2));
            (
                shouted, refreshed, taunted, back, ended, lent, during, after, corpse,
            )
        })
        .await
        .unwrap();
    let (shouted, refreshed, taunted, back, ended, lent, during, after, corpse) = out;
    assert_eq!(
        shouted,
        (500.0, w2),
        "level with the victim, and on the shouter"
    );
    assert_eq!(
        refreshed, 525.0,
        "what it earned stays; a refresh borrows no more"
    );
    assert_eq!(taunted, w3, "the newer Taunt holds");
    assert_eq!(back, w2, "and at its end, the shout again");
    assert_eq!(
        ended,
        (75.0, false),
        "the loan back at the shout's end, the 25 kept"
    );
    assert!(lent > 75.0, "lent again: {lent}");
    assert_eq!(during, lent, "held while either taunt lasts");
    assert_eq!(after, 75.0, "given back when both are gone");
    assert!(!corpse, "a dead creature lends nothing");
}

const SHADOW_WORD_PAIN: u32 = 589;
const DISPEL_MAGIC_2: u32 = 988;
const CURE_POISON: u32 = 526;

fn dispel_resist(d: &mut crate::game::data::GameData) {
    let mut s = made_up(990_990, 6, auras::AURA_ADD_FLAT_MODIFIER, 28, 100);
    s.family_name = 8;
    s.duration_index = 21;
    s.effects[0].item_type = 0x1_0000;
    d.spells.spells.insert(s.id, s);
}

#[tokio::test]
#[ignore = "extended"]
async fn a_dispel_takes_its_kind_a_stack_a_try_unless_held_on() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(dispel_resist);
    let (_c, w, map) = player(&realm, "P3A29", "Cureone", Race::Human, Class::Warrior).await;
    let (_d, priest, _) = player(&realm, "P3A30", "Curetwo", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            ready(m, priest, 60, &[]);
            // Out of the Battle Stance a warrior logs in in.
            auras::remove_spell(m, w, crate::game::map::spells::BATTLE_STANCE, None);
            let k1 = dummy(m, w, 60, 5.0);
            let k2 = dummy(m, w, 60, 6.0);
            let stacks = |m: &Map| holder(m, w, DEADLY_POISON).map_or(0, |h| h.stacks);
            // Two Shadow Word: Pains and three stacks of poison.
            apply(m, k1, w, SHADOW_WORD_PAIN);
            apply(m, k2, w, SHADOW_WORD_PAIN);
            for _ in 0..3 {
                apply(m, k1, w, DEADLY_POISON);
            }
            let before = (spells_on(m, w).len(), stacks(m));
            // Dispel Magic's two tries take both pains, not the poison.
            spells::cast_triggered(m, priest, DISPEL_MAGIC_2, Some(w));
            let magic = (has(m, w, SHADOW_WORD_PAIN), stacks(m));
            // A cure of poison: a stack a try.
            spells::cast_triggered(m, priest, CURE_POISON, Some(w));
            let cured = stacks(m);
            // None of its kind: nothing.
            auras::remove_spell(m, w, DEADLY_POISON, None);
            apply(m, k1, w, SHADOW_WORD_PAIN);
            spells::cast_triggered(m, priest, CURE_POISON, Some(w));
            let wrong = has(m, w, SHADOW_WORD_PAIN);
            // A poison its caster's talent holds on resists.
            apply(m, k1, k1, 990_990);
            apply(m, k1, w, DEADLY_POISON);
            spells::cast_triggered(m, priest, CURE_POISON, Some(w));
            let held = has(m, w, DEADLY_POISON);
            (before, magic, cured, wrong, held)
        })
        .await
        .unwrap();
    let (before, magic, cured, wrong, held) = out;
    assert_eq!(before, (3, 3), "two pains and a poison of three");
    assert_eq!(magic, (false, 3), "both pains dispelled, the poison kept");
    assert_eq!(cured, 2, "a stack cured");
    assert!(wrong, "a cure of poison takes no magic");
    assert!(held, "held on by its caster's talent");
}

const BROWN_HORSE: u32 = 458;
const SWIFT_PALOMINO: u32 = 23227;

fn run_speed(m: &Map, g: Guid) -> f32 {
    m.players[&g].speeds.run
}

#[tokio::test]
#[ignore = "extended"]
async fn a_mount_runs_by_its_own_bonus_and_a_slow_multiplies() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P3A31", "Rideone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let lasting = |m: &mut Map, id: u32, kind: u32, amount: i32| {
                let mut s = made_up(id, 6, kind, 0, amount);
                s.duration_index = 21;
                apply_template(m, w, w, &s);
            };
            let plain = run_speed(m, w);
            apply(m, w, w, BROWN_HORSE);
            let horse = run_speed(m, w);
            apply(m, w, w, SWIFT_PALOMINO);
            let swift = (run_speed(m, w), has(m, w, BROWN_HORSE));
            auras::remove_spell(m, w, SWIFT_PALOMINO, None);
            let off = run_speed(m, w);
            // A run bonus on foot, then a mount: the mount's alone; a snare
            // multiplies; off the mount, the foot bonus and the snare.
            lasting(m, 990_995, auras::AURA_MOD_INCREASE_SPEED, 100);
            let sprinting = run_speed(m, w);
            apply(m, w, w, BROWN_HORSE);
            let mounted = run_speed(m, w);
            lasting(m, 990_996, auras::AURA_MOD_DECREASE_SPEED, -50);
            let snared = run_speed(m, w);
            auras::remove_spell(m, w, BROWN_HORSE, None);
            let afoot = run_speed(m, w);
            (plain, horse, swift, off, sprinting, mounted, snared, afoot)
        })
        .await
        .unwrap();
    let (plain, horse, swift, off, sprinting, mounted, snared, afoot) = out;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(near(plain, 7.0));
    assert!(near(horse, 11.2), "a 60% mount: {horse}");
    assert!(
        near(swift.0, 14.0) && !swift.1,
        "the swift one takes its place: {swift:?}"
    );
    assert!(near(off, 7.0), "off it, afoot again");
    assert!(near(sprinting, 14.0), "a 100% run bonus: {sprinting}");
    assert!(
        near(mounted, 11.2),
        "mounted, the mount's bonus alone: {mounted}"
    );
    assert!(near(snared, 5.6), "a snare halves it: {snared}");
    assert!(near(afoot, 7.0), "afoot, the bonus and the snare: {afoot}");
}
