//! Bugs found while mapping wvs's spells against Thistle Tea's, the fifth
//! batch: a helpful proc's damage that missed, the Warsong flag taken up by
//! the immune, the mounted and the stunned, activation-time modifiers that
//! changed no aura's ticks, a percentage of mana that made no threat, proc
//! cooldowns and charges spent on procs that came to nothing, and Nightfall
//! on a killing tick.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::{Map, auras, procs};

const HOLY_SHIELD: u32 = 20925;
const OBSIDIAN_ARMOR: u32 = 27539;
/// Obsidian Armor's answer to fire.
const OBSIDIAN_FIRE: u32 = 27533;
const FIREBALL: u32 = 133;
const NIGHTFALL: u32 = 18094;
const SHADOW_TRANCE: u32 = 17941;
const CORRUPTION: u32 = 172;

/// A magic spell of the physical school, made up: Obsidian Armor has no
/// answer to it.
const PHYSICAL_BOLT: u32 = 995_001;

/// A block taken from `attacker`, for `g`'s procs.
fn block(m: &mut Map, attacker: Guid, g: Guid) {
    procs::trigger(
        m,
        procs::Event {
            actor: attacker,
            target: g,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex: procs::EX_BLOCK | procs::EX_NORMAL_HIT,
            damage: 10,
            school: 1,
            ..Default::default()
        },
    );
}

/// A harmful spell's hit from `attacker` on `g`, for their procs.
fn spell_hit(m: &mut Map, attacker: Guid, g: Guid, spell: u32) {
    let school = m
        .realm
        .data
        .spells
        .get(spell)
        .map_or(1, |s| s.school_mask());
    procs::trigger(
        m,
        procs::Event {
            actor: attacker,
            target: g,
            actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
            target_flags: procs::PROC_TAKEN_NEGATIVE_SPELL,
            ex: procs::EX_NORMAL_HIT,
            spell: Some(spell),
            damage: 10,
            school,
            ..Default::default()
        },
    );
}

/// Holy Shield's holy damage, from a helpful aura, can't miss, as Thistle
/// Tea's `ProcDamage.hit?` and mangos's `SpellHitResult` (with the aura's
/// own spell) have it: a low paladin's blocks against a creature fifty
/// levels above them strike back every time, where a spell of theirs would
/// land one time in five.
#[tokio::test]
async fn holy_shields_damage_never_misses() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, paladin, map) = player(&realm, "B5HS", "Shieldone", Race::Human, Class::Paladin).await;
    let hits = map
        .query(move |m| {
            ready(m, paladin, 10, &[]);
            let ogre = dummy(m, paladin, 60, 3.0);
            m.spells.rolls = Some(true);
            let mut hits = 0;
            for _ in 0..40 {
                // A fresh shield each time: its four charges.
                apply(m, paladin, paladin, HOLY_SHIELD);
                let before = health(m, ogre);
                block(m, ogre, paladin);
                if health(m, ogre) < before {
                    hits += 1;
                }
            }
            m.spells.rolls = None;
            hits
        })
        .await
        .unwrap();
    assert_eq!(hits, 40, "every block strikes back");
}

/// A proc that comes to nothing starts no cooldown and spends no charge, as
/// Thistle Tea's reactions and mangos's `SPELL_AURA_PROC_FAILED` have it:
/// Obsidian Armor has no answer to a physical spell, so its ten seconds
/// don't start, and the fire that follows is answered.
#[tokio::test]
async fn a_proc_that_comes_to_nothing_starts_no_cooldown() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&OBSIDIAN_ARMOR) {
            s.proc_chance = 100;
        }
        let mut bolt = made_up(PHYSICAL_BOLT, 2, 0, 0, 10);
        bolt.dmg_class = 1;
        bolt.effects[0].target_a = 6;
        d.spells.spells.insert(PHYSICAL_BOLT, bolt);
    });
    let (_c, warrior, map) = player(&realm, "B5OA", "Armorone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, warrior, 60, &[]);
            let kobold = dummy(m, warrior, 60, 5.0);
            apply(m, warrior, warrior, OBSIDIAN_ARMOR);
            let charged = |m: &mut Map| {
                if let Some(a) = auras::auras_mut(m, warrior) {
                    for h in a.holders.iter_mut().filter(|h| h.spell == OBSIDIAN_ARMOR) {
                        h.charges = 2;
                    }
                }
            };
            charged(m);
            spell_hit(m, kobold, warrior, PHYSICAL_BOLT);
            let after_physical =
                holder(m, warrior, OBSIDIAN_ARMOR).map(|h| (h.charges, h.proc_ready_at.is_some()));
            spell_hit(m, kobold, warrior, FIREBALL);
            let after_fire =
                holder(m, warrior, OBSIDIAN_ARMOR).map(|h| (h.charges, h.proc_ready_at.is_some()));
            (after_physical, after_fire, has(m, warrior, OBSIDIAN_FIRE))
        })
        .await
        .unwrap();
    let (after_physical, after_fire, answered) = out;
    assert_eq!(
        after_physical,
        Some((2, false)),
        "no answer, no cooldown and no charge"
    );
    assert!(answered, "the fire is answered");
    assert_eq!(after_fire, Some((1, true)), "and that spends them");
}

/// Nightfall gives no Shadow Trance for a Corruption tick that kills: a
/// tick's procs come after its damage (mangos's `PeriodicTick`), and the
/// class script wants a living victim (mangos's
/// `HandleOverrideClassScriptAuraProc`, Thistle Tea's `ClassScript`), so
/// nothing came of it: its charge stays.
#[tokio::test]
async fn nightfall_gives_no_shadow_trance_for_a_killing_tick() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&NIGHTFALL) {
            s.proc_chance = 100;
        }
    });
    let (_c, lock, map) = player(&realm, "B5NF", "Nightone", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, lock, 60, &[CORRUPTION]);
            apply(m, lock, lock, NIGHTFALL);
            if let Some(a) = auras::auras_mut(m, lock) {
                for h in a.holders.iter_mut().filter(|h| h.spell == NIGHTFALL) {
                    h.charges = 3;
                }
            }
            let charges = |m: &Map| holder(m, lock, NIGHTFALL).map_or(0, |h| h.charges);
            // A tick that leaves its victim standing: a Shadow Trance.
            let kobold = dummy(m, lock, 60, 5.0);
            apply(m, lock, kobold, CORRUPTION);
            tick_now(m, kobold, CORRUPTION);
            let lived = (has(m, lock, SHADOW_TRANCE), charges(m));
            auras::remove_spell(m, lock, SHADOW_TRANCE, None);
            // A tick that kills.
            let doomed = dummy(m, lock, 60, 5.0);
            apply(m, lock, doomed, CORRUPTION);
            set_health(m, doomed, 1);
            tick_now(m, doomed, CORRUPTION);
            let killed = (
                health(m, doomed) == 0,
                has(m, lock, SHADOW_TRANCE),
                charges(m),
            );
            (lived, killed)
        })
        .await
        .unwrap();
    let (lived, killed) = out;
    assert_eq!(lived, (true, 2), "a tick procs Nightfall");
    assert_eq!(
        killed,
        (true, false, 2),
        "a killing tick doesn't, and spends nothing"
    );
}
