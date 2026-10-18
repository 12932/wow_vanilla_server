//! Phase 3's immunities, Thistle Tea's `EffectImmunityTest` ported: what
//! an effect, state, mechanic or school immunity keeps off and how it is
//! told, what passes it, and what an immunity that purges takes as it
//! comes.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells;
use crate::game::tests_p3_aura2::{add_spells, aura_spell, has, health, pass, put};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_spells_kit::missed;

/// A stun (of the stun mechanic), one that passes immunities, and one
/// that is a melee ability.
const STUN: u32 = 96_500;
const STUN_FREE: u32 = 96_501;
const STUN_MELEE: u32 = 96_502;
/// A physical root, a frost DoT, a shadow DoT, a bleed and a buff.
const ROOT: u32 = 96_503;
const FROST_DOT: u32 = 96_504;
const SHADOW_DOT: u32 = 96_505;
const BLEED: u32 = 96_506;
const BUFF: u32 = 96_507;
/// Stealth of the curse dispel type.
const STEALTHY: u32 = 96_508;
/// Immunities: to auras' effect, to stuns (a second), to the stun
/// mechanic and the same purging, to physical and to every school
/// purging, to every school not, to bleeds purging (Stoneform's mask), to
/// curses not purging and to no dispel type purging.
const EFFECT_IMMUNE: u32 = 96_510;
const STUN_IMMUNE: u32 = 96_511;
const MECHANIC_IMMUNE: u32 = 96_512;
const MECHANIC_PURGE: u32 = 96_513;
const PHYSICAL_PURGE: u32 = 96_514;
const ALL_PURGE: u32 = 96_515;
const ALL_IMMUNE: u32 = 96_516;
const STONEFORM: u32 = 96_517;
const CURSE_IMMUNE: u32 = 96_518;
const NO_DISPEL_PURGE: u32 = 96_519;
/// Weakened Soul's kind: a harmful immunity to shields (mechanic 16), of a
/// second; a helpful immunity to heals, a harmful one to auras; a shield,
/// a bandage's kind (19) and a heal.
const WEAKENED: u32 = 96_520;
const HEAL_IMMUNE: u32 = 96_521;
const AURA_IMMUNE_HARMFUL: u32 = 96_522;
const SHIELDING: u32 = 96_523;
const BANDAGING: u32 = 96_524;
const MENDING: u32 = 96_525;
/// A helpful aura cast.
const BUFF_CAST: u32 = 96_526;

const MECHANIC_STUN: u32 = 12;
const MECHANIC_BLEED: u32 = 15;
/// `SPELL_ATTR_EX_DISPEL_AURAS_ON_IMMUNITY`.
const ATTR_EX_PURGES: u32 = 0x8000;

/// A harmful spell of one aura at its target.
fn harmful(id: u32, aura: u32, amplitude: u32) -> SpellTemplate {
    let mut s = aura_spell(id, Some(10_000), &[(aura, 0, amplitude)]);
    s.effects[0].target_a = 6;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut all = Vec::new();
    for id in [STUN, STUN_FREE, STUN_MELEE] {
        let mut s = harmful(id, AURA_MOD_STUN, 0);
        s.mechanic = MECHANIC_STUN;
        match id {
            STUN_FREE => s.attributes[0] |= sd::ATTR_NO_IMMUNITIES,
            STUN_MELEE => s.dmg_class = 2,
            _ => {}
        }
        all.push(s);
    }
    let mut frost = harmful(FROST_DOT, AURA_PERIODIC_DAMAGE, 1000);
    frost.school = 4;
    let mut shadow = harmful(SHADOW_DOT, AURA_PERIODIC_DAMAGE, 1000);
    shadow.school = 5;
    let mut bleed = harmful(BLEED, AURA_PERIODIC_DAMAGE, 1000);
    bleed.mechanic = MECHANIC_BLEED;
    let mut stealthy = aura_spell(STEALTHY, None, &[(AURA_MOD_STEALTH, 0, 0)]);
    stealthy.dispel = 2;
    all.extend([
        harmful(ROOT, AURA_MOD_ROOT, 0),
        frost,
        shadow,
        bleed,
        aura_spell(BUFF, Some(30_000), &[(AURA_MOD_STAT, 0, 0)]),
        stealthy,
    ]);
    for (id, aura, misc, purges, ms) in [
        (EFFECT_IMMUNE, AURA_EFFECT_IMMUNITY, 6, false, None),
        (STUN_IMMUNE, AURA_STATE_IMMUNITY, 12, false, Some(1_000)),
        (MECHANIC_IMMUNE, AURA_MECHANIC_IMMUNITY, 12, false, None),
        (MECHANIC_PURGE, AURA_MECHANIC_IMMUNITY, 12, true, None),
        (PHYSICAL_PURGE, AURA_SCHOOL_IMMUNITY, 1, true, None),
        (ALL_PURGE, AURA_SCHOOL_IMMUNITY, 127, true, None),
        (ALL_IMMUNE, AURA_SCHOOL_IMMUNITY, 127, false, None),
        (STONEFORM, AURA_MECHANIC_IMMUNITY_MASK, 0x4000, true, None),
        (CURSE_IMMUNE, AURA_DISPEL_IMMUNITY, 2, false, None),
        (NO_DISPEL_PURGE, AURA_DISPEL_IMMUNITY, 0, true, None),
    ] {
        let mut s = aura_spell(id, ms, &[(aura, misc, 0)]);
        if purges {
            s.attributes[1] |= ATTR_EX_PURGES;
        }
        all.push(s);
    }
    let mut weakened = aura_spell(WEAKENED, Some(1_000), &[(AURA_MECHANIC_IMMUNITY, 16, 0)]);
    weakened.attributes[0] |= sd::ATTR_DEBUFF;
    let heal_immune = aura_spell(HEAL_IMMUNE, None, &[(AURA_EFFECT_IMMUNITY, 10, 0)]);
    let mut aura_immune = aura_spell(AURA_IMMUNE_HARMFUL, None, &[(AURA_EFFECT_IMMUNITY, 6, 0)]);
    aura_immune.attributes[0] |= sd::ATTR_DEBUFF;
    let mut shielding = aura_spell(SHIELDING, Some(30_000), &[(AURA_SCHOOL_ABSORB, 127, 0)]);
    shielding.mechanic = 16;
    let mut bandaging = aura_spell(BANDAGING, Some(8_000), &[(AURA_PERIODIC_HEAL, 0, 1000)]);
    bandaging.mechanic = 19;
    let mut mending = SpellTemplate {
        id: MENDING,
        name: "Mending".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    mending.effects[0] = crate::game::data::spells::SpellEffect {
        effect: spells::EFFECT_HEAL,
        base_points: 9,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut buff_cast = aura_spell(BUFF_CAST, Some(30_000), &[(AURA_MOD_STAT, 0, 0)]);
    buff_cast.effects[0].base_points = 4;
    all.extend([
        buff_cast,
        weakened,
        heal_immune,
        aura_immune,
        shielding,
        bandaging,
        mending,
    ]);
    add_spells(&mut d, all);
    d
}

fn stunned(m: &Map, g: Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has(AURA_MOD_STUN))
}

/// Thistle Tea's `SpellEffect.receive` on immunities: an immunity to
/// auras' effect keeps a stun off and is told as one immune miss; a stun
/// immunity takes its place before a melee stun's avoidance roll (a
/// warrior who dodges everything is immune, not dodging); a mechanic
/// immunity is passed by a spell that ignores immunities; and a stun
/// immunity run out lets the stun on.
#[tokio::test]
async fn immunities_keep_their_effects_off_and_are_told_as_immune() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    a.clear();
    let brute = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 2.0);
            put(m, ga, ga, EFFECT_IMMUNE, &[0]).unwrap();
            spells::cast_triggered(m, brute, STUN, Some(ga));
            brute
        })
        .await
        .unwrap();
    assert_eq!(missed(&mut a, STUN).await, 7, "one immune miss");
    let (effect, avoided) = map
        .query(move |m| {
            let effect = stunned(m, ga);
            auras::remove_where(m, ga, |_| true);
            put(m, ga, ga, STUN_IMMUNE, &[0]).unwrap();
            // Every blow dodged.
            if let Some(p) = m.players.get_mut(&ga) {
                p.derived.dodge = 100.0;
            }
            spells::cast_triggered(m, brute, STUN_MELEE, Some(ga));
            (effect, stunned(m, ga))
        })
        .await
        .unwrap();
    assert!(!effect, "the effect immunity keeps the stun off");
    assert!(!avoided, "and so does the stun immunity");
    assert_eq!(missed(&mut a, STUN_MELEE).await, 7, "immune, not dodged");
    let (free, after) = map
        .query(move |m| {
            auras::remove_where(m, ga, |_| true);
            put(m, ga, ga, MECHANIC_IMMUNE, &[0]).unwrap();
            spells::cast_triggered(m, brute, STUN_FREE, Some(ga));
            let free = stunned(m, ga);
            auras::remove_where(m, ga, |_| true);
            put(m, ga, ga, STUN_IMMUNE, &[0]).unwrap();
            pass(m, 1_001);
            spells::cast_triggered(m, brute, STUN, Some(ga));
            (free, stunned(m, ga))
        })
        .await
        .unwrap();
    assert!(free, "a spell that ignores immunities passes");
    assert!(after, "the immunity run out lets it on");
}

/// Thistle Tea's purges: an immunity to a school that purges takes the
/// harmful auras of its school as it comes, the root among them (the
/// player told they are free), and leaves another school's and the
/// helpful; one to every school takes a DoT, which ticks no more. An
/// immunity that doesn't purge takes nothing, and neither does a purge
/// take an aura that ignores immunities. Stoneform's mask takes a bleed
/// and keeps it off, a stun still landing; a mechanic immunity that
/// doesn't purge leaves the stun there.
#[tokio::test]
async fn an_immunity_that_purges_takes_what_it_keeps_off() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, brute, ga, ROOT, &[0]).unwrap();
            put(m, brute, ga, FROST_DOT, &[10]).unwrap();
            put(m, ga, ga, BUFF, &[3]).unwrap();
            brute
        })
        .await
        .unwrap();
    a.wait_frame("SMSG_FORCE_MOVE_ROOT", 0x0E8, |_| true).await;
    let school = map
        .query(move |m| {
            put(m, ga, ga, PHYSICAL_PURGE, &[0]).unwrap();
            (has(m, ga, ROOT), has(m, ga, FROST_DOT), has(m, ga, BUFF))
        })
        .await
        .unwrap();
    assert_eq!(school, (false, true, true), "the root alone");
    a.wait_frame("SMSG_FORCE_MOVE_UNROOT", 0x0EA, |_| true)
        .await;
    let got = map
        .query(move |m| {
            auras::remove_where(m, ga, |_| true);
            put(m, brute, ga, SHADOW_DOT, &[10]).unwrap();
            put(m, ga, ga, ALL_PURGE, &[0]).unwrap();
            let before = health(m, ga);
            pass(m, 1_500);
            let dot = (has(m, ga, SHADOW_DOT), health(m, ga) == before);
            auras::remove_where(m, ga, |_| true);
            // No purge without the attribute, nor of what ignores immunities.
            put(m, brute, ga, STUN, &[0]).unwrap();
            put(m, ga, ga, ALL_IMMUNE, &[0]).unwrap();
            let unpurged = has(m, ga, STUN);
            auras::remove_where(m, ga, |_| true);
            put(m, brute, ga, STUN_FREE, &[0]).unwrap();
            put(m, ga, ga, ALL_PURGE, &[0]).unwrap();
            let free = has(m, ga, STUN_FREE);
            auras::remove_where(m, ga, |_| true);
            put(m, brute, ga, STUN, &[0]).unwrap();
            put(m, ga, ga, MECHANIC_IMMUNE, &[0]).unwrap();
            let mechanic = has(m, ga, STUN);
            auras::remove_where(m, ga, |_| true);
            put(m, brute, ga, STUN_FREE, &[0]).unwrap();
            put(m, ga, ga, MECHANIC_PURGE, &[0]).unwrap();
            let mechanic_free = has(m, ga, STUN_FREE);
            auras::remove_where(m, ga, |_| true);
            // Stoneform.
            put(m, brute, ga, BLEED, &[10]).unwrap();
            put(m, ga, ga, STONEFORM, &[0]).unwrap();
            let bled = has(m, ga, BLEED);
            let again = put(m, brute, ga, BLEED, &[10]);
            let stun = put(m, brute, ga, STUN, &[0]);
            (
                dot,
                (unpurged, free, mechanic, mechanic_free),
                (bled, again, stun),
            )
        })
        .await
        .unwrap();
    assert_eq!(got.0, (false, true), "the DoT gone, and no tick");
    assert_eq!(
        got.1,
        (true, true, true, true),
        "no purge without the attribute, nor of what ignores immunities"
    );
    assert_eq!(
        got.2,
        (false, Err(Refused::Immune), Ok(())),
        "Stoneform's bleed"
    );
}

/// Thistle Tea's dispel immunities: one that doesn't purge leaves the
/// stealth of its type there, and keeps it as it was when cast again; one
/// of no type, purging, takes nothing of no type.
#[tokio::test]
async fn a_dispel_immunity_keeps_what_was_there_unless_it_purges_its_type() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            put(m, ga, ga, STEALTHY, &[5]).unwrap();
            put(m, brute, ga, CURSE_IMMUNE, &[0]).unwrap();
            let kept = has(m, ga, STEALTHY);
            m.now += Duration::from_secs(1);
            let again = put(m, ga, ga, STEALTHY, &[5]);
            auras::remove_where(m, ga, |_| true);
            put(m, brute, ga, STUN, &[0]).unwrap();
            put(m, ga, ga, NO_DISPEL_PURGE, &[0]).unwrap();
            (kept, again, has(m, ga, STUN))
        })
        .await
        .unwrap();
    assert_eq!(got, (true, Err(Refused::Immune), true));
}

/// Thistle Tea's friendly immunities: a harmful immunity to a mechanic
/// (Weakened Soul's) keeps a helpful spell of it off (a shield), not one of
/// another (a bandage), until it runs out. An effect immunity of a helpful
/// aura keeps off a harmful spell's effect and lets a heal through; one of
/// a harmful aura keeps off a helpful aura and lets a stun on.
#[tokio::test]
async fn immunities_on_helpful_spells_are_the_harmful_auras() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 2.0);
            put(m, brute, ga, WEAKENED, &[0]).unwrap();
            let shield = put(m, ga, ga, SHIELDING, &[100]);
            let bandage = put(m, ga, ga, BANDAGING, &[10]);
            pass(m, 1_000);
            let later = put(m, ga, ga, SHIELDING, &[100]);
            auras::remove_where(m, ga, |_| true);
            // A helpful immunity to heals: a heal goes through.
            put(m, ga, ga, HEAL_IMMUNE, &[0]).unwrap();
            crate::game::tests_p3_aura2::set(m, ga, crate::game::fields::UNIT_FIELD_HEALTH, 10);
            spells::cast_triggered(m, ga, MENDING, Some(ga));
            let healed = health(m, ga) > 10;
            auras::remove_where(m, ga, |_| true);
            // A harmful immunity to auras: a helpful one is kept off, a stun
            // isn't.
            put(m, brute, ga, AURA_IMMUNE_HARMFUL, &[0]).unwrap();
            spells::cast_triggered(m, ga, BUFF_CAST, Some(ga));
            let buffed = has(m, ga, BUFF_CAST);
            spells::cast_triggered(m, brute, STUN, Some(ga));
            ((shield, bandage, later), healed, buffed, stunned(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        (Err(Refused::Immune), Ok(()), Ok(())),
        "the shield kept off till it runs out"
    );
    assert!(got.1, "a heal through a helpful immunity to heals");
    assert!(!got.2, "a helpful aura kept off by a harmful immunity");
    assert!(got.3, "a stun lets through");
}
