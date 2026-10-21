//! Round 12f: the auras mangos-classic has and wvs didn't. Roots and
//! pacify-silences break by chance on damage, as fears do (auras 26 and
//! 60, `HandleRemoveByDamageChanceProc`; SITREP #59 by mangos's rule);
//! Winter's Chill's aura 179 raises the crit chance of harmful magic at its
//! bearer (`GetCritTakenChance`).

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::{LASTS_15S, aura, bare, damage, effect, fast_realm, human, insert};
use super::tests_p3_spells_kit::{dummy, ready};
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::spells::mods::Mods;
use crate::game::map::spells::outcome;
use crate::game::map::{Map, auras, spells};

// ---- roots and pacify-silences break by chance on damage -----------------

const ROOT: u32 = 995_100;
const SILENCE: u32 = 995_101;
const FLAGLESS_ROOT: u32 = 995_102;
const BOLT: u32 = 995_103;
/// Every damage taken, as Frost Nova's and Entangling Roots' `procFlags`
/// (664232, 0xA22A8).
const TAKEN_DAMAGE: u32 = 0x000A_22A8;

fn break_kit(d: &mut GameData) {
    let mut root = bare(ROOT, 4);
    root.duration_index = LASTS_15S;
    root.proc_flags = TAKEN_DAMAGE;
    root.proc_chance = 100;
    effect(&mut root, 0, aura(auras::AURA_MOD_ROOT, 0, 0, 6));
    let mut silence = root.clone();
    silence.id = SILENCE;
    effect(
        &mut silence,
        0,
        aura(auras::AURA_MOD_PACIFY_SILENCE, 0, 0, 6),
    );
    let mut flagless = root.clone();
    flagless.id = FLAGLESS_ROOT;
    flagless.proc_flags = 0;
    let mut bolt = bare(BOLT, 2);
    effect(&mut bolt, 0, damage(100));
    insert(d, [root, silence, flagless, bolt]);
}

/// A root or a pacify-silence whose spell's procFlags name the blow breaks
/// by the fear's chance (100 damage on a level-10 unit is sure), as
/// mangos-classic's `HandleRemoveByDamageChanceProc` has them; one whose
/// spell names no blow holds.
#[tokio::test]
async fn a_root_and_a_silence_break_by_chance_at_a_blow() {
    let realm = fast_realm(break_kit);
    let (_c, mage, map) = human(&realm, "P12FROOT", "Rooter", Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, mage, 60, &[BOLT]);
            let held = |m: &mut Map, spell: u32| {
                let k = dummy(m, mage, 10, 5.0);
                let s = m.realm.data.spells.get(spell).cloned().unwrap();
                auras::apply_spell(m, mage, k, &s, &[(0, 0)], auras::Context::default())
                    .expect("it takes");
                let before = auras::auras(m, k).is_some_and(|a| a.has_spell(spell));
                spells::cast_triggered(m, mage, BOLT, Some(k));
                let hurt = m.unit_fields(k).unwrap().get_u32(UNIT_FIELD_HEALTH) < 1_000_000;
                (
                    before,
                    hurt,
                    auras::auras(m, k).is_some_and(|a| a.has_spell(spell)),
                )
            };
            (held(m, ROOT), held(m, SILENCE), held(m, FLAGLESS_ROOT))
        })
        .await
        .unwrap();
    assert_eq!(got.0, (true, true, false), "the root broken by the blow");
    assert_eq!(got.1, (true, true, false), "the silence broken by the blow");
    assert_eq!(got.2, (true, true, true), "no blow named, it holds");
}

// ---- Winter's Chill: aura 179 ----------------------------------------------

const CHILL: u32 = 995_110;
const FROSTBOLT: u32 = 995_111;
const FIREBALL: u32 = 995_112;
const FROST_HEAL: u32 = 995_113;

fn chill_kit(d: &mut GameData) {
    // Winter's Chill's aura at 100% (12579's is 2 a stack), frost's mask.
    let mut chill = bare(CHILL, 4);
    chill.duration_index = LASTS_15S;
    effect(
        &mut chill,
        0,
        aura(auras::AURA_MOD_ATTACKER_SPELL_CRIT_CHANCE, 1 << 4, 100, 6),
    );
    let mut frostbolt = bare(FROSTBOLT, 4);
    effect(&mut frostbolt, 0, damage(100));
    let mut fireball = bare(FIREBALL, 2);
    effect(&mut fireball, 0, damage(100));
    let mut heal = bare(FROST_HEAL, 4);
    effect(
        &mut heal,
        0,
        crate::game::data::spells::SpellEffect {
            effect: 10,
            base_points: 99,
            base_dice: 1,
            target_a: 21,
            ..Default::default()
        },
    );
    insert(d, [chill, frostbolt, fireball, heal]);
}

/// The target's aura 179 adds to the crit chance of harmful magic of its
/// schools cast at it, and to nothing else (mangos-classic's
/// `CalculateSpellCritChance`, through `GetCritTakenChance` for a spell not
/// positive): with no crit of their own, the mage's frost bolt crits on the
/// chilled, their fireball and frost heal don't.
#[tokio::test]
async fn winters_chill_raises_the_crit_chance_of_its_schools_at_its_bearer() {
    let realm = fast_realm(chill_kit);
    let (_c, mage, map) = human(&realm, "P12FCHILL", "Chiller", Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            let crits = |m: &mut Map, spell: u32| {
                if let Some(p) = m.players.get_mut(&mage) {
                    p.derived.spell_crit = [0.0; 7];
                }
                let s = m.realm.data.spells.get(spell).cloned().unwrap();
                outcome::roll_crit(m, mage, k, &s, &Mods::default())
            };
            let unchilled = crits(m, FROSTBOLT);
            let s = m.realm.data.spells.get(CHILL).cloned().unwrap();
            auras::apply_spell(m, mage, k, &s, &[(0, 100)], auras::Context::default())
                .expect("the chill takes");
            (
                unchilled,
                crits(m, FROSTBOLT),
                crits(m, FIREBALL),
                crits(m, FROST_HEAL),
            )
        })
        .await
        .unwrap();
    assert_eq!(got, (false, true, false, false));
}

// ---- Seal of the Crusader's per-hit damage cut ----------------------------

/// Seal of the Crusader's rank 1, as made up: 40% faster swings and the
/// dummy whose damage cut mangos does in code.
const CRUSADER: u32 = 21082;
/// Another seal of the same shape, which isn't the Crusader's.
const OTHER_SEAL: u32 = 995_120;

fn crusader_kit(d: &mut GameData) {
    let mut seal = bare(CRUSADER, 1);
    seal.duration_index = LASTS_15S;
    effect(&mut seal, 0, aura(auras::AURA_MOD_HASTE, 0, 40, 1));
    effect(&mut seal, 1, aura(auras::AURA_DUMMY, 0, 0, 1));
    let mut other = seal.clone();
    other.id = OTHER_SEAL;
    insert(d, [seal, other]);
}

/// Seal of the Crusader divides the main hand's own damage by 1.4, before
/// attack power's bonus, so its faster swings deal no more a second
/// (mangos-classic's `HandleAuraDummy` for 21082 and its ranks, on
/// `m_modAttackBaseDPSPct`): a fist's 1 to 2 become 0.71 to 1.43. Another
/// haste of the same shape cuts nothing; the seal gone, the damage comes
/// back.
#[tokio::test]
async fn seal_of_the_crusader_cuts_the_main_hands_damage_a_hit() {
    let realm = fast_realm(crusader_kit);
    let (_c, pally, map) = human(&realm, "P12FSOTC", "Crusader", Class::Paladin).await;
    let got = map
        .query(move |m| {
            ready(m, pally, 60, &[]);
            let damage = |m: &Map| {
                let f = m.unit_fields(pally).unwrap();
                (
                    f.get_f32(UNIT_FIELD_MINDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXDAMAGE),
                )
            };
            let bare = damage(m);
            spells::cast_triggered(m, pally, OTHER_SEAL, Some(pally));
            let other = damage(m);
            auras::remove_spell(m, pally, OTHER_SEAL, None);
            spells::cast_triggered(m, pally, CRUSADER, Some(pally));
            let sealed = damage(m);
            let held = auras::auras(m, pally).is_some_and(|a| a.has_spell(CRUSADER));
            auras::remove_spell(m, pally, CRUSADER, None);
            (bare, other, sealed, held, damage(m))
        })
        .await
        .unwrap();
    let (bare, other, sealed, held, after) = got;
    assert!(held, "the seal on");
    assert_eq!(other, bare, "another haste cuts nothing");
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        near(bare.0 - sealed.0, 1.0 - 1.0 / 1.4) && near(bare.1 - sealed.1, 2.0 - 2.0 / 1.4),
        "the fist's own damage over 1.4: {bare:?} to {sealed:?}"
    );
    assert_eq!(after, bare, "back when it goes");
}
