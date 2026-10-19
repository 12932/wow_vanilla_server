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
#[ignore = "extended"]
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
#[ignore = "extended"]
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

/// A creature's harmful cast at one it may no longer attack as the cast
/// ends fails, as a player's does (mangos's `Spell::cast` checks it
/// again): Keeper Remulos's Starfire, begun at Eranikus, landed after his
/// redemption and drew him back into the fight.
#[tokio::test]
#[ignore = "extended"]
async fn a_creatures_cast_at_one_it_may_no_longer_attack_fails() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3P9", "Pushnine", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            spells::cast(m, kobold, FROSTBOLT, at(mage), None);
            let began = casting(m, kobold);
            if let Some(f) = m.unit_fields_mut(mage) {
                let flags = f.get_u32(crate::game::fields::UNIT_FIELD_FLAGS);
                f.set_u32(crate::game::fields::UNIT_FIELD_FLAGS, flags | 0x2);
            }
            let before = health(m, mage);
            m.now += Duration::from_secs(5);
            spells::update(m);
            (began, casting(m, kobold), health(m, mage) == before)
        })
        .await
        .unwrap();
    assert_eq!(out.0, Some(FROSTBOLT), "the Frostbolt began");
    assert_eq!(out.1, None, "and ended");
    assert!(out.2, "without landing");
}

#[tokio::test]
#[ignore = "extended"]
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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

const ARCANE_MISSILES: u32 = 5143;
const EVOCATION: u32 = 12051;
const HEALING_POTION: u32 = 439;
const HEALING_POTION_2: u32 = 440;
const FROST_NOVA: u32 = 122;

/// The global cooldown `g` has left of the spells' category.
fn gcd_left(m: &Map, g: wow_world_messages::Guid) -> Option<Duration> {
    m.caster(g)
        .and_then(|c| c.gcd_until)
        .map(|t| t.saturating_duration_since(m.now))
}

/// Lasting modifiers of the global cooldown (op 21): -100, -20%, -2000
/// and +200 to Fire Blast's flags, and -100 to others.
fn gcd_spells(d: &mut crate::game::data::GameData) {
    for (id, kind, amount, mask) in [
        (
            990_950,
            crate::game::map::auras::AURA_ADD_FLAT_MODIFIER,
            -100,
            2u64,
        ),
        (
            990_951,
            crate::game::map::auras::AURA_ADD_PCT_MODIFIER,
            -20,
            2,
        ),
        (
            990_952,
            crate::game::map::auras::AURA_ADD_FLAT_MODIFIER,
            -2000,
            2,
        ),
        (
            990_953,
            crate::game::map::auras::AURA_ADD_FLAT_MODIFIER,
            200,
            2,
        ),
        (
            990_954,
            crate::game::map::auras::AURA_ADD_FLAT_MODIFIER,
            -100,
            4,
        ),
    ] {
        let mut s = made_up(id, 6, kind, 21, amount);
        s.duration_index = 21;
        s.family_name = 3;
        s.effects[0].item_type = mask;
        d.spells.spells.insert(id, s);
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn a_global_cooldown_takes_its_modifiers_not_haste_and_holds_what_it_should() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(gcd_spells);
    let (_c, mage, map) = player(&realm, "P3P7", "Gcdthree", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(
                m,
                mage,
                60,
                &[FIRE_BLAST, ARCANE_MISSILES, FROST_NOVA, EVOCATION],
            );
            let kobold = dummy(m, mage, 60, 5.0);
            let blast = |m: &mut Map| {
                cast_fresh(m, mage, FIRE_BLAST, at(kobold));
                gcd_left(m, mage)
            };
            let plain = blast(m);
            // Casting speed doesn't touch it.
            let mut haste = made_up(
                990_960,
                6,
                crate::game::map::auras::AURA_MOD_CASTING_SPEED,
                0,
                -50,
            );
            haste.duration_index = 21;
            apply_template(m, mage, mage, &haste);
            let hasted = blast(m);
            crate::game::map::auras::remove_spell(m, mage, 990_960, None);
            // Its modifiers do: flat, then percent; none below nothing; one
            // of other flags not at all.
            let mut by_mods = Vec::new();
            for set in [
                &[990_950][..],
                &[990_950, 990_951],
                &[990_952],
                &[990_953],
                &[990_954],
            ] {
                for id in set {
                    apply(m, mage, mage, *id);
                }
                by_mods.push(blast(m));
                for id in set {
                    crate::game::map::auras::remove_spell(m, mage, *id, None);
                }
            }
            // An instant's isn't given back by a cancel, nor a channel's.
            blast(m);
            spells::cancel(m, mage);
            let instant = gcd_left(m, mage);
            cast_fresh(m, mage, ARCANE_MISSILES, at(kobold));
            let channel = casting(m, mage);
            spells::cancel(m, mage);
            let channel_kept = gcd_left(m, mage);
            // A triggered cast in it changes nothing.
            blast(m);
            let before = gcd_left(m, mage);
            spells::cast_triggered(m, mage, FIRE_BLAST, Some(kobold));
            let triggered = gcd_left(m, mage) == before;
            (
                plain,
                hasted,
                by_mods,
                instant,
                channel,
                channel_kept,
                triggered,
            )
        })
        .await
        .unwrap();
    let (plain, hasted, by_mods, instant, channel, channel_kept, triggered) = out;
    let ms = |n: u64| Some(Duration::from_millis(n));
    assert_eq!(plain, ms(1500));
    assert_eq!(hasted, ms(1500), "casting speed doesn't touch it");
    assert_eq!(by_mods, vec![ms(1400), ms(1120), None, ms(1700), ms(1500)]);
    assert_eq!(instant, ms(1500), "an instant's stays after a cancel");
    assert_eq!(channel, Some(ARCANE_MISSILES));
    assert_eq!(channel_kept, ms(1500), "and a channel's");
    assert!(triggered, "a triggered cast changes nothing");
}

#[tokio::test]
#[ignore = "extended"]
async fn cooldowns_hold_their_spell_and_category_for_the_longer() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3P8", "Coolone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(
                m,
                mage,
                60,
                &[FIREBALL, FROST_NOVA, HEALING_POTION, HEALING_POTION_2],
            );
            let kobold = dummy(m, mage, 60, 5.0);
            let data = m.realm.data.clone();
            let mods = spells::mods::Mods::default();
            let check = |m: &mut Map, id: u32, t: spells::Targets| {
                let s = data.spells.get(id).cloned().unwrap();
                if let Some(c) = m.caster_mut(mage) {
                    c.gcd_until = None;
                }
                spells::checks::check(m, mage, &s, &t, None, false, &mods)
            };
            // Fireball has none.
            cast_fresh(m, mage, FIREBALL, at(kobold));
            pass(m, 4_000);
            let fireball = m
                .caster(mage)
                .map(|c| (c.cooldowns.len(), c.category_cooldowns.len()));
            // A potion holds its category: another potion of it too.
            cast_fresh(m, mage, HEALING_POTION, spells::Targets::default());
            let other = check(m, HEALING_POTION_2, spells::Targets::default());
            m.now += Duration::from_millis(10_001);
            let later = check(m, HEALING_POTION_2, spells::Targets::default());
            // Frost Nova's 25 seconds, by its category time.
            cast_fresh(m, mage, FROST_NOVA, spells::Targets::default());
            let nova = m
                .caster(mage)
                .and_then(|c| c.category_cooldowns.get(&35).copied())
                .map(|t| t.saturating_duration_since(m.now));
            (fireball, other, later, nova)
        })
        .await
        .unwrap();
    let (fireball, other, later, nova) = out;
    assert_eq!(fireball, Some((0, 0)), "Fireball starts no cooldown");
    assert_eq!(
        other,
        Some(spells::fail::NOT_READY),
        "the potion's category holds the other"
    );
    assert_eq!(later, None, "until its time is up");
    assert_eq!(
        nova,
        Some(Duration::from_secs(25)),
        "Frost Nova's category time"
    );
}

const SUMMON_IMP: u32 = 688;
const HEALTH_FUNNEL: u32 = 755;

#[tokio::test]
#[ignore = "extended"]
async fn health_funnel_takes_its_health_a_second_and_ends_without_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, lock, map) = player(&realm, "P3P9", "Funnelone", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, lock, 60, &[HEALTH_FUNNEL]);
            spells::cast_triggered(m, lock, SUMMON_IMP, None);
            let pet = crate::game::map::pets::pet_of(m, lock).expect("the imp");
            set_health(m, lock, 1_000);
            assert!(cast_fresh(m, lock, HEALTH_FUNNEL, at(pet)), "the funnel");
            let paid = 1_000 - health(m, lock);
            let mut each = Vec::new();
            for _ in 0..3 {
                let before = health(m, lock);
                pass(m, 1_000);
                each.push(before - health(m, lock));
            }
            // Too little health for the next second: it ends.
            set_health(m, lock, 5);
            pass(m, 1_000);
            (paid, each, casting(m, lock), health(m, lock))
        })
        .await
        .unwrap();
    let (paid, each, after, left) = out;
    assert_eq!(paid, 11, "its cost at the start");
    assert_eq!(each, vec![5, 5, 5], "and five a second");
    assert_eq!(after, None, "without five to pay, it ends");
    assert_eq!(left, 5, "taking none");
}

const BLINK: u32 = 1953;
const PSYCHIC_SCREAM: u32 = 8122;
const POLYMORPH: u32 = 118;
const CHARGE: u32 = 100;
const CLAW: u32 = 1082;
const RENEW: u32 = 139;
const SHADOWFORM: u32 = 15473;
const EXORCISM: u32 = 879;
const TURN_UNDEAD: u32 = 2878;
const SLOW_FALL: u32 = 130;
const LESSER_HEAL: u32 = 2050;
const MIND_VISION: u32 = 10909;
const HAMMER_OF_JUSTICE: u32 = 853;
const SKELETAL_WARRIOR: u32 = 48;
const IMP: u32 = 416;

#[tokio::test]
#[ignore = "extended"]
async fn a_cast_is_checked_for_its_casters_state_its_form_its_target_and_its_reach() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, me, map) = player(&realm, "P3P10", "Checkone", Race::Human, Class::Mage).await;
    let (_d, friend, _) = player(&realm, "P3P11", "Checktwo", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, me, 60, &[]);
            if let Some(p) = m.players.get_mut(&me) {
                p.gm = false;
            }
            let kobold = dummy(m, me, 60, 10.0);
            let undead = spawn(m, me, SKELETAL_WARRIOR, 60, 10.0);
            let demon = spawn(m, me, IMP, 60, 10.0);
            let data = m.realm.data.clone();
            let mods = spells::mods::Mods::default();
            let check = |m: &mut Map, id: u32, t: spells::Targets| {
                let s = data.spells.get(id).cloned().unwrap();
                if let Some(p) = m.players.get_mut(&me)
                    && !p.spells.contains(&id)
                {
                    p.spells.push(id);
                }
                fresh(m, me);
                spells::checks::check(m, me, &s, &t, None, false, &mods)
            };
            let mut seen = Vec::new();
            // Stunned, feared, confused: held; Blink frees from the stun only, the
            // control its own immunities cancel (mangos's `CheckCasterAuras`).
            for (aura, label) in [
                (HAMMER_OF_JUSTICE, "stun"),
                (PSYCHIC_SCREAM, "fear"),
                (POLYMORPH, "confuse"),
            ] {
                apply(m, kobold, me, aura);
                seen.push((
                    label,
                    check(m, FROSTBOLT, at(kobold)),
                    check(m, BLINK, spells::Targets::default()),
                ));
                clear_all(m, me);
            }
            // Fire locked out: no Fireball, a Frostbolt yes.
            spells::cooldowns::lock_school(m, me, FIREBALL, 5_000);
            let locked = (
                check(m, FIREBALL, at(kobold)),
                check(m, FROSTBOLT, at(kobold)),
            );
            m.now += Duration::from_millis(5_001);
            let unlocked = check(m, FIREBALL, at(kobold));
            // Charge's least range, in Battle Stance: 8 yards beyond both
            // reaches (mangos's `GetMinMaxRange`).
            spells::cast_triggered(m, me, 2457, None);
            let reaches = [me, kobold]
                .iter()
                .filter_map(|&g| m.unit_view(g))
                .map(|u| u.combat_reach)
                .sum::<f32>();
            place(m, me, kobold, 8.0 + reaches - 0.5);
            let close = check(m, CHARGE, at(kobold));
            place(m, me, kobold, 8.0 + reaches + 0.5);
            let charge = check(m, CHARGE, at(kobold));
            clear_all(m, me);
            // Forms: Claw is a cat's; Fireball no shape's; Renew no shadow's.
            let form = |m: &mut Map, f: u32| {
                clear_all(m, me);
                if f != 0 {
                    spells::cast_triggered(m, me, f, None);
                }
            };
            let mut forms = Vec::new();
            place(m, me, kobold, 2.0);
            for (f, spell) in [
                (0, CLAW),
                (BEAR_FORM, CLAW),
                (CAT_FORM, CLAW),
                (CAT_FORM, FIREBALL),
                (SHADOWFORM, RENEW),
            ] {
                form(m, f);
                let t = if spell == RENEW { at(me) } else { at(kobold) };
                forms.push(check(m, spell, t));
            }
            form(m, 0);
            place(m, me, kobold, 10.0);
            // Creature types: Exorcism and Turn Undead.
            let types = (
                check(m, EXORCISM, at(undead)),
                check(m, EXORCISM, at(kobold)),
                check(m, TURN_UNDEAD, at(undead)),
                check(m, TURN_UNDEAD, at(demon)),
            );
            // Too little mana; no reagent.
            set_power(m, me, 0, 10);
            let mana = check(m, FIREBALL, at(kobold));
            set_power(m, me, 0, 100_000);
            let feather = check(m, SLOW_FALL, at(me));
            // Friend and foe; the dead.
            place(m, me, friend, 5.0);
            let friendly = check(m, FIREBALL, at(friend));
            let heal_foe = check(m, LESSER_HEAL, at(kobold));
            crate::game::map::combat::deal_damage(m, me, undead, 2_000_000);
            let dead = check(m, FIREBALL, at(undead));
            // A wall: Fireball doesn't see past it, Mind Vision does.
            let p = m.position_of(me).unwrap();
            let k = m.position_of(kobold).unwrap();
            let mid = Vector3d {
                x: (p.x + k.x) / 2.0,
                y: (p.y + k.y) / 2.0,
                z: p.z,
            };
            let side = Vector3d {
                x: -(k.y - p.y),
                y: k.x - p.x,
                z: 0.0,
            };
            m.spells.walls.push((
                Vector3d {
                    x: mid.x + side.x,
                    y: mid.y + side.y,
                    z: mid.z,
                },
                Vector3d {
                    x: mid.x - side.x,
                    y: mid.y - side.y,
                    z: mid.z,
                },
            ));
            let walled = (
                check(m, FIREBALL, at(kobold)),
                check(m, MIND_VISION, at(kobold)),
            );
            m.spells.walls.clear();
            // Out of reach, unless the target is large.
            place(m, me, kobold, 40.0);
            let far = check(m, FIREBALL, at(kobold));
            if let Some(c) = m.creature_mut(kobold) {
                c.fields
                    .set_f32(crate::game::fields::UNIT_FIELD_COMBATREACH, 4.0);
            }
            let large = check(m, FIREBALL, at(kobold));
            // The dead don't cast.
            crate::game::map::death::kill(m, me);
            let me_dead = check(m, FIREBALL, at(kobold));
            (
                seen, locked, unlocked, close, charge, forms, types, mana, feather, friendly,
                heal_foe, dead, walled, far, large, me_dead,
            )
        })
        .await
        .unwrap();
    use spells::fail::*;
    let (
        seen,
        locked,
        unlocked,
        close,
        charge,
        forms,
        types,
        mana,
        feather,
        friendly,
        heal_foe,
        dead,
        walled,
        far,
        large,
        me_dead,
    ) = out;
    assert_eq!(
        seen,
        vec![
            ("stun", Some(STUNNED), None),
            ("fear", Some(FLEEING), Some(FLEEING)),
            ("confuse", Some(CONFUSED), Some(CONFUSED))
        ]
    );
    assert_eq!(locked, (Some(SILENCED), None), "fire locked out, frost not");
    assert_eq!(unlocked, None, "until the lock is up");
    assert_eq!(
        (close, charge),
        (Some(TOO_CLOSE), None),
        "Charge's least range"
    );
    assert_eq!(
        forms,
        vec![
            Some(ONLY_SHAPESHIFT),
            Some(ONLY_SHAPESHIFT),
            None,
            Some(NOT_SHAPESHIFT),
            Some(NOT_SHAPESHIFT)
        ]
    );
    assert_eq!(
        types,
        (None, Some(BAD_TARGETS), None, Some(BAD_TARGETS)),
        "the creature types"
    );
    assert_eq!(mana, Some(NO_POWER));
    assert_eq!(feather, Some(REAGENTS));
    assert_eq!(friendly, Some(TARGET_FRIENDLY));
    assert_eq!(heal_foe, Some(TARGET_ENEMY));
    assert_eq!(dead, Some(TARGETS_DEAD));
    assert_eq!(
        walled,
        (Some(LINE_OF_SIGHT), None),
        "Mind Vision sees past a wall"
    );
    assert_eq!(
        (far, large),
        (Some(OUT_OF_RANGE), None),
        "a large target's reach counts"
    );
    assert_eq!(me_dead, Some(CASTER_DEAD));
}

fn clear_all(m: &mut Map, g: wow_world_messages::Guid) {
    auras::remove_where(m, g, |h| !h.passive);
}

/// `target` `yards` before `viewer`.
fn place(
    m: &mut Map,
    viewer: wow_world_messages::Guid,
    target: wow_world_messages::Guid,
    yards: f32,
) {
    let (p, o) = {
        let v = m.unit_view(viewer).unwrap();
        (v.position, v.orientation)
    };
    let to = Vector3d {
        x: p.x + yards * o.cos(),
        y: p.y + yards * o.sin(),
        z: p.z,
    };
    if let Some(x) = m.players.get_mut(&target) {
        x.movement.position = to;
    } else if let Some(c) = m.creature_mut(target) {
        c.position = to;
    }
}

const BEAR_FORM: u32 = 5487;
const CAT_FORM: u32 = 768;

const ARCANE_EXPLOSION: u32 = 1449;
const HOLY_WRATH: u32 = 2812;
const CHAIN_LIGHTNING: u32 = 421;
const ARCTIC_REACH: u32 = 16758;

#[tokio::test]
#[ignore = "extended"]
async fn an_area_takes_its_count_its_kinds_and_its_reach() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, me, map) = player(&realm, "P3P12", "Areaone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, me, 60, &[]);
            let data = m.realm.data.clone();
            let s = |id: u32| data.spells.get(id).cloned().unwrap();
            let hit = |m: &Map, id: u32, t: spells::Targets| {
                let mods = spells::mods::for_spell(m, me, &s(id));
                spells::targets::effect_targets(m, me, &s(id), 0, &t, &mods)
            };
            // Six about the caster: Psychic Scream takes two, Arcane
            // Explosion all.
            let six: Vec<_> = (0..6)
                .map(|i| dummy(m, me, 60, 2.0 + i as f32 * 0.3))
                .collect();
            let scream = hit(m, PSYCHIC_SCREAM, spells::Targets::default()).len();
            let explosion = hit(m, ARCANE_EXPLOSION, spells::Targets::default()).len();
            for g in &six {
                m.remove_creature(*g);
            }
            // Holy Wrath: the undead, not the humanoid.
            let undead = spawn(m, me, SKELETAL_WARRIOR, 60, 3.0);
            let kobold = dummy(m, me, 60, 4.0);
            let wrath = hit(m, HOLY_WRATH, spells::Targets::default());
            m.remove_creature(undead);
            m.remove_creature(kobold);
            // Chain Lightning jumps ten yards from each.
            let chain: Vec<_> = [5.0, 14.0, 23.0]
                .into_iter()
                .map(|d| dummy(m, me, 60, d))
                .collect();
            let lightning = hit(m, CHAIN_LIGHTNING, at(chain[0]));
            for g in &chain {
                m.remove_creature(*g);
            }
            // Frost Nova's ten yards, a fifth more with Arctic Reach.
            let ring: Vec<_> = [9.0, 11.0, 13.0]
                .into_iter()
                .map(|d| dummy(m, me, 60, d))
                .collect();
            let nova = hit(m, FROST_NOVA, spells::Targets::default());
            apply(m, me, me, ARCTIC_REACH);
            let reached = hit(m, FROST_NOVA, spells::Targets::default());
            (
                scream, explosion, wrath, undead, lightning, chain, nova, reached, ring,
            )
        })
        .await
        .unwrap();
    let (scream, explosion, wrath, undead, lightning, chain, nova, reached, ring) = out;
    assert_eq!(scream, 2, "Psychic Scream takes two");
    assert_eq!(explosion, 6, "Arcane Explosion all six");
    assert_eq!(wrath, vec![undead], "Holy Wrath the undead alone");
    // Its third may be a creature of the world's nearer the second.
    assert_eq!(lightning.len(), 3, "Chain Lightning's three");
    assert_eq!(lightning[..2], chain[..2], "jumping ten yards from each");
    assert_eq!(nova, vec![ring[0]], "Frost Nova's ten yards");
    assert_eq!(
        reached,
        vec![ring[0], ring[1]],
        "and twelve with Arctic Reach"
    );
}

const SLICE_AND_DICE: u32 = 5171;

#[tokio::test]
#[ignore = "extended"]
async fn slice_and_dice_goes_at_the_combo_target_and_spends_its_points() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, rogue, map) = player(&realm, "P3P13", "Sliceone", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, rogue, 60, &[SLICE_AND_DICE]);
            let kobold = dummy(m, rogue, 60, 3.0);
            if let Some(c) = m.caster_mut(rogue) {
                c.combo_target = kobold;
                c.combo_points = 3;
            }
            let data = m.realm.data.clone();
            let s = data.spells.get(SLICE_AND_DICE).cloned().unwrap();
            let mods = spells::mods::Mods::default();
            let checked = spells::checks::check(m, rogue, &s, &at(kobold), None, false, &mods);
            let cast = cast_fresh(m, rogue, SLICE_AND_DICE, at(kobold));
            let sliced = holder(m, rogue, SLICE_AND_DICE).and_then(|h| h.duration);
            let points = m.caster(rogue).map(|c| c.combo_points);
            (checked, cast, sliced, points, kobold)
        })
        .await
        .unwrap();
    let (checked, cast, sliced, points, kobold) = out;
    // As the client asks it.
    c.clear();
    let k = kobold;
    map.query(move |m| {
        if let Some(c) = m.caster_mut(rogue) {
            c.combo_target = k;
            c.combo_points = 2;
        }
        fresh(m, rogue);
    })
    .await
    .unwrap();
    c.send(CMSG_CAST_SPELL {
        spell: SLICE_AND_DICE,
        targets: spells::wire_targets(&at(kobold)),
    })
    .await;
    let b = c
        .wait_frame("SMSG_CAST_RESULT", spells::SMSG_CAST_RESULT, |b| {
            b[..4] == SLICE_AND_DICE.to_le_bytes()
        })
        .await;
    assert_eq!((b[4] != 0).then(|| b[5]), None, "the client's cast goes");
    // Aimed at no one, or at the rogue: at the combo target.
    let out2 = map
        .query(move |m| {
            let mut seen = Vec::new();
            for t in [spells::Targets::default(), at(rogue)] {
                auras::remove_spell(m, rogue, SLICE_AND_DICE, None);
                if let Some(c) = m.caster_mut(rogue) {
                    c.combo_target = k;
                    c.combo_points = 1;
                }
                seen.push((
                    cast_fresh(m, rogue, SLICE_AND_DICE, t),
                    has(m, rogue, SLICE_AND_DICE),
                ));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        out2,
        vec![(true, true), (true, true)],
        "no target or oneself: the combo target"
    );
    assert_eq!(checked, None, "at the combo target, it may go");
    assert!(cast, "and goes");
    assert!(
        sliced.is_some_and(|d| d > 9_000),
        "longer for three points: {sliced:?}"
    );
    assert_eq!(points, Some(0), "spending them");
}

const AIMED_SHOT: u32 = 19434;
const RAPID_FIRE: u32 = 3045;

#[tokio::test]
#[ignore = "extended"]
async fn a_ranged_ability_is_sped_by_ranged_haste_and_a_spell_by_casting_speed() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, hunter, map) = player(&realm, "P3P14", "Aimone", Race::Dwarf, Class::Hunter).await;
    let out = map
        .query(move |m| {
            ready(m, hunter, 60, &[]);
            // The starting quiver's haste on, now rather than at the next update.
            crate::game::map::inventory::sync_gear_all(m);
            let data = m.realm.data.clone();
            let mods = spells::mods::Mods::default();
            let aimed = data.spells.get(AIMED_SHOT).cloned().unwrap();
            let bolt = data.spells.get(FROSTBOLT).cloned().unwrap();
            let before = (
                spells::cast_time(m, hunter, &aimed, &mods),
                spells::cast_time(m, hunter, &bolt, &mods),
            );
            apply(m, hunter, hunter, RAPID_FIRE);
            let after = (
                spells::cast_time(m, hunter, &aimed, &mods),
                spells::cast_time(m, hunter, &bolt, &mods),
            );
            (before, after)
        })
        .await
        .unwrap();
    let (before, after) = out;
    // Three seconds, less the starting quiver's 10% ranged haste.
    assert_eq!(
        before.0,
        (3000.0 / 1.1) as u32,
        "Aimed Shot's three seconds by the quiver"
    );
    assert!(
        after.0.abs_diff((3000.0 / 1.1 / 1.4) as u32) <= 1,
        "Rapid Fire's 40% speeds it: {after:?}"
    );
    assert_eq!(after.1, before.1, "not a spell's cast");
}

const FEAR_WARD: u32 = 6346;
const FEAR: u32 = 5782;
const CONFLAGRATE: u32 = 17962;
const IMMOLATE: u32 = 348;
const FEIGN_DEATH: u32 = 5384;
const SINISTER_STRIKE: u32 = 1752;
const RESURRECTION: u32 = 2006;

fn feared(m: &Map, g: wow_world_messages::Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOD_FEAR))
}

#[tokio::test]
#[ignore = "extended"]
async fn fear_ward_is_spent_by_the_fear_it_keeps_off_and_conflagrate_wants_immolate() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P3P15", "Wardone", Race::Human, Class::Warrior).await;
    let (_d, lock, _) = player(&realm, "P3P16", "Wardtwo", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            ready(m, lock, 60, &[CONFLAGRATE]);
            let kobold = dummy(m, w, 60, 5.0);
            // Fear Ward: a Frostbolt doesn't spend it, a Fear does, kept off.
            apply(m, w, w, FEAR_WARD);
            spells::cast_triggered(m, kobold, FROSTBOLT, Some(w));
            let after_bolt = has(m, w, FEAR_WARD);
            spells::cast_triggered(m, kobold, FEAR, Some(w));
            let warded = (feared(m, w), has(m, w, FEAR_WARD));
            if let Some(c) = m.caster_mut(w) {
                c.diminishing.clear();
            }
            spells::cast_triggered(m, kobold, FEAR, Some(w));
            let next = feared(m, w);
            // Conflagrate: refused without the warlock's own Immolate.
            let target = dummy(m, lock, 60, 5.0);
            let data = m.realm.data.clone();
            let s = data.spells.get(CONFLAGRATE).cloned().unwrap();
            let mods = spells::mods::Mods::default();
            let check = |m: &mut Map| {
                fresh(m, lock);
                spells::checks::check(m, lock, &s, &at(target), None, false, &mods)
            };
            let none = check(m);
            apply(m, kobold, target, IMMOLATE);
            let anothers = check(m);
            apply(m, lock, target, IMMOLATE);
            let own = check(m);
            (after_bolt, warded, next, none, anothers, own)
        })
        .await
        .unwrap();
    let (after_bolt, warded, next, none, anothers, own) = out;
    assert!(after_bolt, "a Frostbolt doesn't spend Fear Ward");
    assert_eq!(warded, (false, false), "the Fear kept off, the ward spent");
    assert!(next, "the next Fear lands");
    use spells::fail::TARGET_AURASTATE;
    assert_eq!(
        (none, anothers, own),
        (Some(TARGET_AURASTATE), Some(TARGET_AURASTATE), None)
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn an_emote_ends_feign_death_and_a_weapon_spell_unarmed_wants_its_weapon() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (c, hunter, map) = player(&realm, "P3P17", "Feignone", Race::Dwarf, Class::Hunter).await;
    map.query(move |m| {
        ready(m, hunter, 60, &[]);
        spells::cast_triggered(m, hunter, FEIGN_DEATH, None);
    })
    .await
    .unwrap();
    let feigning = map
        .query(move |m| has(m, hunter, FEIGN_DEATH))
        .await
        .unwrap();
    c.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Wave,
        emote: 0,
        target: wow_world_messages::Guid::zero(),
    })
    .await;
    let mut ended = false;
    for _ in 0..200 {
        if !map
            .query(move |m| has(m, hunter, FEIGN_DEATH))
            .await
            .unwrap()
        {
            ended = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(feigning, "feigning");
    assert!(ended, "a wave ends it");
    // Unarmed, a weapon's strike: the weapon is wanted.
    let unarmed = map
        .query(move |m| {
            let p = m.players.get_mut(&hunter).unwrap();
            p.items
                .retain(|i| !(i.bag == crate::game::store::BAG_PLAYER && i.slot < 19));
            p.gm = false;
            p.spells.push(SINISTER_STRIKE);
            let kobold = dummy(m, hunter, 60, 2.0);
            let s = m.realm.data.spells.get(SINISTER_STRIKE).cloned().unwrap();
            let mods = spells::mods::Mods::default();
            spells::checks::check(m, hunter, &s, &at(kobold), None, false, &mods)
        })
        .await
        .unwrap();
    assert_eq!(unarmed, Some(spells::fail::EQUIPPED_ITEM_CLASS));
}

#[tokio::test]
#[ignore = "extended"]
async fn resurrection_is_at_the_dead_it_is_aimed_at() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, priest, map) = player(&realm, "P3P18", "Raiseone", Race::Human, Class::Priest).await;
    let (_d, dead, _) = player(&realm, "P3P19", "Raisetwo", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, priest, 60, &[RESURRECTION]);
            place(m, priest, dead, 5.0);
            let s = m.realm.data.spells.get(RESURRECTION).cloned().unwrap();
            let mods = spells::mods::Mods::default();
            let living = spells::checks::check(m, priest, &s, &at(dead), None, false, &mods);
            crate::game::map::death::kill(m, dead);
            let ok = spells::checks::check(m, priest, &s, &at(dead), None, false, &mods);
            spells::cast_triggered(m, priest, RESURRECTION, Some(dead));
            let offered = m.players[&dead].resurrect_offer.map(|o| o.0);
            (living, ok, offered)
        })
        .await
        .unwrap();
    let (living, ok, offered) = out;
    assert_eq!(
        living,
        Some(spells::fail::TARGET_NOT_DEAD),
        "not on the living"
    );
    assert_eq!(ok, None, "on the dead");
    assert_eq!(offered, Some(priest), "who is offered it");
}
