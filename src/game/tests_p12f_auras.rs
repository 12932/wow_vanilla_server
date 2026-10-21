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
