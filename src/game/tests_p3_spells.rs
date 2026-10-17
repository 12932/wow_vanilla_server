//! Phase 3's casting, against the real data when it is here, as Thistle
//! Tea's tests of it have it: what damage does to a cast or a channel
//! under way.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells;
use crate::game::map::{Map, auras};

const FROSTBOLT: u32 = 116;
const HEARTHSTONE: u32 = 8690;
const FIRST_AID: u32 = 746;
const DRAIN_LIFE: u32 = 689;
const BLIZZARD: u32 = 10;
const HELLFIRE: u32 = 1949;
const CORRUPTION: u32 = 172;
const POWER_WORD_SHIELD: u32 = 17;

fn ends(m: &Map, g: wow_world_messages::Guid) -> Option<std::time::Instant> {
    m.caster(g).and_then(|c| c.casting.as_ref()).map(|c| c.ends)
}

/// `from`'s Frostbolt of `amount` on `to`.
fn hit(m: &mut Map, from: wow_world_messages::Guid, to: wow_world_messages::Guid, amount: u32) {
    let s = m.realm.data.spells.get(FROSTBOLT).cloned().unwrap();
    spells::spell_damage(m, from, to, &s, amount);
}

#[tokio::test]
async fn damage_cancels_a_hearthstone_and_a_bandage_and_pushes_back_only_what_it_should() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, mage, map) = player(&realm, "P3P1", "Pushone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(
                m,
                mage,
                60,
                &[HEARTHSTONE, FROSTBOLT, FIRST_AID, POWER_WORD_SHIELD],
            );
            let kobold = dummy(m, mage, 60, 5.0);
            // The Hearthstone's cast, which damage cancels (its interrupt
            // flags have 0x10 beside 0x2).
            spells::cast(m, mage, HEARTHSTONE, spells::Targets::default(), None);
            let hearth_began = casting(m, mage);
            hit(m, kobold, mage, 10);
            let hearth_after = casting(m, mage);
            // A bandage's channel, which damage cancels.
            cast_fresh(m, mage, FIRST_AID, at(mage));
            let bandage_began = casting(m, mage);
            hit(m, kobold, mage, 10);
            let bandage_after = (casting(m, mage), has(m, mage, FIRST_AID));
            // Lava's harm, the player's own, cancels it too.
            cast_fresh(m, mage, FIRST_AID, at(mage));
            crate::game::map::environment::damage(m, mage, EnvironmentalDamageType::Lava, 10);
            let bandage_lava = casting(m, mage);
            // A Frostbolt: a damage over time's tick doesn't push it back,
            // nor a blow a shield takes whole.
            cast_fresh(m, mage, FROSTBOLT, at(kobold));
            let start = ends(m, mage);
            // Pushback goes no further back than the cast's start.
            m.now += Duration::from_millis(1000);
            apply(m, kobold, mage, CORRUPTION);
            let now = m.now;
            if let Some(a) = auras::auras_mut(m, mage) {
                for h in a.holders.iter_mut().filter(|h| h.spell == CORRUPTION) {
                    for x in h.effects.iter_mut() {
                        x.next_tick = Some(now);
                    }
                }
            }
            let before_tick = health(m, mage);
            auras::update(m);
            let ticked = health(m, mage) < before_tick;
            let after_tick = ends(m, mage);
            auras::remove_spell(m, mage, CORRUPTION, None);
            apply(m, mage, mage, POWER_WORD_SHIELD);
            hit(m, kobold, mage, 10);
            let after_shield = ends(m, mage);
            auras::remove_spell(m, mage, POWER_WORD_SHIELD, None);
            // The world's harm pushes it back.
            crate::game::map::environment::damage(m, mage, EnvironmentalDamageType::Lava, 10);
            let after_lava = ends(m, mage);
            (
                hearth_began,
                hearth_after,
                bandage_began,
                bandage_after,
                bandage_lava,
                ticked,
                start == after_tick,
                start == after_shield,
                after_lava.zip(start).map(|(a, b)| a - b),
            )
        })
        .await
        .unwrap();
    let (
        hearth_began,
        hearth_after,
        bandage_began,
        bandage_after,
        bandage_lava,
        ticked,
        tick_kept,
        shield_kept,
        lava,
    ) = out;
    assert_eq!(hearth_began, Some(HEARTHSTONE), "the Hearthstone began");
    assert_eq!(hearth_after, None, "and damage cancelled it");
    let failed = c
        .wait_for("SMSG_SPELL_FAILURE", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_FAILURE(f) if f.spell == HEARTHSTONE => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(failed, SpellCastResult::Interrupted);
    assert_eq!(bandage_began, Some(FIRST_AID), "the bandage began");
    assert_eq!(bandage_after, (None, false), "and damage cancelled it");
    assert_eq!(bandage_lava, None, "as lava does");
    assert!(ticked, "the damage over time ticked");
    assert!(tick_kept, "a tick pushes nothing back");
    assert!(shield_kept, "nor a blow the shield took whole");
    assert_eq!(
        lava,
        Some(Duration::from_millis(1000)),
        "lava pushes it back a second"
    );
}

#[tokio::test]
async fn a_creatures_cast_isnt_pushed_back_and_pushback_may_be_resisted() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3P2", "Pushtwo", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[FROSTBOLT]);
            let kobold = dummy(m, mage, 60, 5.0);
            // A creature's Frostbolt at the mage, hit by the mage.
            spells::cast(m, kobold, FROSTBOLT, at(mage), None);
            let kobold_start = ends(m, kobold);
            m.now += Duration::from_millis(1000);
            hit(m, mage, kobold, 10);
            let kobold_kept = kobold_start.is_some() && ends(m, kobold) == kobold_start;
            spells::interrupt(m, kobold);
            // Pushback resisted whole.
            let mut s = made_up(990_001, 6, auras::AURA_RESIST_PUSHBACK, 127, 100);
            s.duration_index = 21;
            apply_template(m, mage, mage, &s);
            let cast = cast_fresh(m, mage, FROSTBOLT, at(kobold));
            let start = ends(m, mage);
            m.now += Duration::from_millis(1000);
            hit(m, kobold, mage, 10);
            let resisted = cast && start.is_some() && ends(m, mage) == start;
            spells::interrupt(m, mage);
            auras::remove_spell(m, mage, 990_001, None);
            // Half the time, rolled.
            let mut s = made_up(990_002, 6, auras::AURA_RESIST_PUSHBACK, 127, 50);
            s.duration_index = 21;
            apply_template(m, mage, mage, &s);
            m.spells.rolls = Some(true);
            let mut kept = 0;
            for _ in 0..400 {
                assert!(cast_fresh(m, mage, FROSTBOLT, at(kobold)), "cast");
                let start = ends(m, mage);
                m.now += Duration::from_millis(1000);
                hit(m, kobold, mage, 1);
                set_health(m, mage, 5_000);
                if ends(m, mage) == start {
                    kept += 1;
                }
            }
            (kobold_kept, resisted, kept)
        })
        .await
        .unwrap();
    let (kobold_kept, resisted, kept) = out;
    assert!(kobold_kept, "a creature's cast isn't pushed back");
    assert!(resisted, "a whole resist keeps the cast");
    assert!(
        (120..=280).contains(&kept),
        "half resisted, rolled: {kept} of 400"
    );
}

#[tokio::test]
async fn a_channel_is_cut_short_by_others_harm_alone_and_its_auras_with_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, lock, map) = player(&realm, "P3P3", "Pushthree", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, lock, 60, &[DRAIN_LIFE, HELLFIRE, FROSTBOLT]);
            let kobold = dummy(m, lock, 60, 5.0);
            spells::cast(m, lock, DRAIN_LIFE, at(kobold), None);
            let start = ends(m, lock);
            let aura_start = holder(m, kobold, DRAIN_LIFE).and_then(|h| h.expires);
            m.now += Duration::from_millis(1000);
            // Lava and the caster's own harm cut nothing.
            crate::game::map::environment::damage(m, lock, EnvironmentalDamageType::Lava, 10);
            let after_lava = ends(m, lock);
            hit(m, lock, lock, 10);
            let after_own = ends(m, lock);
            // A creature's blow takes a second off the channel, and off its
            // aura on the creature.
            hit(m, kobold, lock, 10);
            let cut = start.zip(ends(m, lock)).map(|(a, b)| a - b);
            let aura_cut = aura_start
                .zip(holder(m, kobold, DRAIN_LIFE).and_then(|h| h.expires))
                .map(|(a, b)| a - b);
            spells::interrupt(m, lock);
            // Hellfire, on the warlock: its own aura is cut short too.
            cast_fresh(m, lock, HELLFIRE, spells::Targets::default());
            let hellfire = casting(m, lock);
            let own_start = holder(m, lock, HELLFIRE).and_then(|h| h.expires);
            m.now += Duration::from_millis(1000);
            hit(m, kobold, lock, 10);
            let own_cut = own_start
                .zip(holder(m, lock, HELLFIRE).and_then(|h| h.expires))
                .map(|(a, b)| a - b);
            let slot = holder(m, lock, HELLFIRE).and_then(|h| h.slot);
            spells::interrupt(m, lock);
            (
                start == after_lava,
                start == after_own,
                cut,
                aura_cut,
                hellfire,
                own_cut,
                slot,
            )
        })
        .await
        .unwrap();
    let (lava_kept, own_kept, cut, aura_cut, hellfire, own_cut, slot) = out;
    assert!(lava_kept, "lava doesn't cut a channel short");
    assert!(own_kept, "nor the caster's own harm");
    assert_eq!(
        cut,
        Some(Duration::from_millis(1000)),
        "a creature's blow does"
    );
    assert_eq!(aura_cut, cut, "and the channel's aura on the creature");
    // Drain Life's five seconds, a second gone and a second cut.
    let left = c
        .wait_for("MSG_CHANNEL_UPDATE", |m| match m {
            ServerOpcodeMessage::MSG_CHANNEL_UPDATE(u) if u.time > 0 => Some(u.time),
            _ => None,
        })
        .await;
    assert_eq!(left, 3_000, "the client is told what is left");
    assert_eq!(hellfire, Some(HELLFIRE));
    assert_eq!(
        own_cut,
        Some(Duration::from_millis(1000)),
        "Hellfire's own aura is cut"
    );
    let slot = slot.expect("Hellfire's aura has a slot") as u8;
    let shown = c
        .wait_for("SMSG_UPDATE_AURA_DURATION", |m| match m {
            ServerOpcodeMessage::SMSG_UPDATE_AURA_DURATION(d)
                if d.aura_slot == slot && d.aura_duration < 15_000 =>
            {
                Some(d.aura_duration)
            }
            _ => None,
        })
        .await;
    assert_eq!(
        shown, 13_000,
        "and its client told: fifteen seconds, less two"
    );
}

#[tokio::test]
async fn a_blizzard_cut_to_nothing_ends_and_takes_its_area() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, mage, map) = player(&realm, "P3P4", "Pushfour", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[BLIZZARD]);
            let kobold = dummy(m, mage, 60, 10.0);
            let dest = m.position_of(kobold);
            let t = spells::Targets {
                dest,
                ..Default::default()
            };
            spells::cast(m, mage, BLIZZARD, t, None);
            let began = casting(m, mage);
            let areas = |m: &Map| {
                m.spells.areas.iter().filter(|a| a.caster == mage).count()
                    + m.spells
                        .dynamic
                        .values()
                        .filter(|d| d.caster == mage)
                        .count()
            };
            let had = areas(m);
            let start = ends(m, mage);
            // A tick of a damage over time cuts nothing.
            apply(m, kobold, mage, CORRUPTION);
            let now = m.now;
            if let Some(a) = auras::auras_mut(m, mage) {
                for h in a.holders.iter_mut().filter(|h| h.spell == CORRUPTION) {
                    for x in h.effects.iter_mut() {
                        x.next_tick = Some(now);
                    }
                }
            }
            auras::update(m);
            auras::remove_spell(m, mage, CORRUPTION, None);
            let tick_kept = ends(m, mage) == start;
            // Seven and a half seconds in, a blow takes the half left.
            m.now += Duration::from_millis(7_500);
            hit(m, kobold, mage, 10);
            let cut_to = ends(m, mage).map(|e| e.saturating_duration_since(m.now));
            pass(m, 0);
            (began, had, tick_kept, cut_to, casting(m, mage), areas(m))
        })
        .await
        .unwrap();
    let (began, had, tick_kept, cut_to, after, left) = out;
    assert_eq!(began, Some(BLIZZARD));
    assert_eq!(had, 2, "Blizzard's area and its object");
    assert!(
        tick_kept,
        "a damage over time's tick doesn't cut the channel"
    );
    assert_eq!(cut_to, Some(Duration::ZERO), "cut to nothing");
    assert_eq!(after, None, "it ends");
    assert_eq!(left, 0, "and its area with it");
    c.wait_for("MSG_CHANNEL_UPDATE 0", |m| match m {
        ServerOpcodeMessage::MSG_CHANNEL_UPDATE(u) if u.time == 0 => Some(()),
        _ => None,
    })
    .await;
}

const FIREBALL: u32 = 133;
const KICK: u32 = 1766;
const RESTORATION: u32 = 11359;
const FIRE_BLAST: u32 = 2136;

#[tokio::test]
async fn a_cast_given_up_gives_back_its_global_cooldown_and_kick_stops_what_silence_could() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, mage, map) = player(&realm, "P3P5", "Gcdone", Race::Human, Class::Mage).await;
    let (_d, rogue, _) = player(&realm, "P3P6", "Gcdtwo", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[FIREBALL, FIRE_BLAST, RESTORATION]);
            ready(m, rogue, 60, &[KICK]);
            let kobold = dummy(m, mage, 60, 5.0);
            let data = m.realm.data.clone();
            // Cast and given up: the global cooldown goes with it.
            cast_fresh(m, mage, FIREBALL, at(kobold));
            let on = m.caster(mage).unwrap().gcd_until.is_some_and(|t| t > m.now);
            spells::cancel(m, mage);
            let off = m.caster(mage).unwrap().gcd_until.is_some_and(|t| t > m.now);
            let again = spells::cast(m, mage, FIREBALL, at(kobold), None);
            spells::interrupt(m, mage);
            // Another category's spell isn't held by the spells' cooldown.
            cast_fresh(m, mage, FIRE_BLAST, at(kobold));
            let restoration = data.spells.get(RESTORATION).cloned().unwrap();
            let fireball = data.spells.get(FIREBALL).cloned().unwrap();
            let mods = spells::mods::Mods::default();
            let none = spells::Targets::default();
            let other = (
                spells::checks::check(m, mage, &fireball, &at(kobold), None, false, &mods),
                spells::checks::check(m, mage, &restoration, &none, None, false, &mods),
            );
            // Kick stops a creature's Fireball, and locks its fire out.
            spells::cast(m, kobold, FIREBALL, at(mage), None);
            spells::cast_triggered(m, rogue, KICK, Some(kobold));
            let kicked = (
                casting(m, kobold),
                m.caster(kobold).map(|c| !c.school_lockout.is_empty()),
            );
            // Not the Hearthstone, which no silence stops.
            spells::cast(m, kobold, HEARTHSTONE, spells::Targets::default(), None);
            let hearth = casting(m, kobold);
            spells::cast_triggered(m, rogue, KICK, Some(kobold));
            let kept = casting(m, kobold);
            spells::interrupt(m, kobold);
            (on, off, again, other, kicked, hearth, kept)
        })
        .await
        .unwrap();
    let (on, off, again, other, kicked, hearth, kept) = out;
    assert!(on && !off, "the global cooldown given back");
    assert!(again, "and the Fireball begun again at once");
    assert_eq!(
        other,
        (Some(spells::fail::NOT_READY), None),
        "Fire Blast's cooldown holds Fireball, not Restoration's category"
    );
    assert_eq!(
        kicked,
        (None, Some(true)),
        "Kick stops the Fireball and locks fire"
    );
    assert_eq!(hearth, Some(HEARTHSTONE));
    assert_eq!(kept, Some(HEARTHSTONE), "but not the Hearthstone");
    let b = c
        .wait_frame("SMSG_CAST_RESULT", spells::SMSG_CAST_RESULT, |b| {
            b[..4] == FIREBALL.to_le_bytes() && b[4] != 0
        })
        .await;
    assert_eq!(
        b[5],
        spells::fail::INTERRUPTED,
        "the client told it was interrupted"
    );
}
