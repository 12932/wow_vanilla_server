//! Phase 5's single-target auras, Thistle Tea's `SingleTargetTest`
//! ported: a caster's limited aura on a new target ends its last of that
//! kind elsewhere (the same family and icon, any rank; both polymorphs;
//! both judgements), not another caster's, an ordinary aura, or one that
//! didn't land; and its caster leaving ends it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{CUSTOM_SINGLE_TARGET_AURA, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::tests_p3_aura2::{add_spells, aura_spell, put};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};

const LIMITED: u32 = 96_980;
const LIMITED_2: u32 = 96_981;
const OTHER_ICON: u32 = 96_982;
const OTHER_FAMILY: u32 = 96_983;
const ORDINARY: u32 = 96_984;
const POLYMORPH: u32 = 96_985;
const POLYMORPH_PIG: u32 = 96_986;
const JUDGEMENT: u32 = 96_987;
const JUDGEMENT_2: u32 = 96_988;
const CURSE: u32 = 96_989;
const CURSE_2: u32 = 96_990;
const ROOT_IMMUNE: u32 = 96_991;

fn limited(id: u32, family: u32, flags: u64, icon: u32, kind: u32) -> SpellTemplate {
    let mut s = aura_spell(id, Some(10_000), &[(kind, 0, 0)]);
    s.effects[0].target_a = 6;
    s.mechanic = 7;
    s.family_name = family;
    s.family_flags = flags;
    s.spell_icon = icon;
    s.custom_flags = CUSTOM_SINGLE_TARGET_AURA;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut ordinary = limited(ORDINARY, 9, 1, 1, AURA_MOD_ROOT);
    ordinary.custom_flags = 0;
    let mut polymorphs = Vec::new();
    for (id, icon) in [(POLYMORPH, 10), (POLYMORPH_PIG, 11)] {
        let mut s = limited(id, 3, 0, icon, AURA_MOD_CONFUSE);
        s.mechanic = 17;
        s.prevention_type = 1;
        polymorphs.push(s);
    }
    let mut judgements = Vec::new();
    for (id, icon) in [(JUDGEMENT, 20), (JUDGEMENT_2, 21)] {
        let mut s = limited(id, 10, 0x400, icon, AURA_MOD_DECREASE_SPEED);
        s.base_level = 1;
        judgements.push(s);
    }
    add_spells(
        &mut d,
        [
            limited(LIMITED, 9, 1, 1, AURA_MOD_ROOT),
            limited(LIMITED_2, 9, 1, 1, AURA_MOD_ROOT),
            limited(OTHER_ICON, 9, 1, 2, AURA_MOD_ROOT),
            limited(OTHER_FAMILY, 8, 1, 1, AURA_MOD_ROOT),
            ordinary,
            limited(CURSE, 5, 0x400, 30, AURA_MOD_DECREASE_SPEED),
            limited(CURSE_2, 5, 0x400, 31, AURA_MOD_DECREASE_SPEED),
            aura_spell(ROOT_IMMUNE, None, &[(AURA_MECHANIC_IMMUNITY, 7, 0)]),
        ]
        .into_iter()
        .chain(polymorphs)
        .chain(judgements),
    );
    d
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Thistle Tea's limits: Alice's root on a second brute ends it on the
/// first, a second rank's too; another caster's stands beside, as do one of
/// another icon or family, and an ordinary aura; one the second brute is
/// immune to ends nothing. Two polymorphs share a limit, two judgements
/// too, two curses of their own icons don't. Alice gone from the world,
/// her limited auras end and her ordinary one stays.
#[tokio::test]
async fn a_limited_aura_on_a_new_target_ends_its_kind_elsewhere() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let one = spawn(m, ga, BRUTE, 1, 3.0);
            let two = spawn(m, ga, BRUTE, 1, 4.0);
            let other = spawn(m, ga, BRUTE, 1, 5.0);
            let mut out = Vec::new();
            let fresh = |m: &mut Map| {
                for g in [one, two] {
                    auras::remove_where(m, g, |_| true);
                }
            };
            for (second, by, ends) in [
                (LIMITED, ga, true),
                (LIMITED_2, ga, true),
                (LIMITED, other, false),
                (OTHER_ICON, ga, false),
                (OTHER_FAMILY, ga, false),
                (ORDINARY, ga, false),
            ] {
                fresh(m);
                let _ = put(m, ga, one, LIMITED, &[0]);
                let _ = put(m, by, two, second, &[0]);
                out.push((second, by == ga, !has(m, one, LIMITED), ends));
            }
            // Immune: nothing lands, nothing ends.
            fresh(m);
            let _ = put(m, ga, one, LIMITED, &[0]);
            let _ = put(m, two, two, ROOT_IMMUNE, &[0]);
            let _ = put(m, ga, two, LIMITED, &[0]);
            let immune = (has(m, one, LIMITED), has(m, two, LIMITED));
            // The kinds.
            let mut kinds = Vec::new();
            for (first, second, shared) in [
                (POLYMORPH, POLYMORPH_PIG, true),
                (JUDGEMENT, JUDGEMENT_2, true),
                (CURSE, CURSE_2, false),
            ] {
                fresh(m);
                let _ = put(m, ga, one, first, &[0]);
                let _ = put(m, ga, two, second, &[0]);
                kinds.push((first, !has(m, one, first), shared));
            }
            // Alice gone.
            fresh(m);
            let _ = put(m, ga, one, LIMITED, &[0]);
            let _ = put(m, ga, two, ORDINARY, &[0]);
            crate::game::map::spells::single_target::caster_gone(m, ga);
            let gone = (has(m, one, LIMITED), has(m, two, ORDINARY));
            (out, immune, kinds, gone)
        })
        .await
        .unwrap();
    let (out, immune, kinds, gone) = got;
    for (second, alices, ended, ends) in out {
        assert_eq!(ended, ends, "{second} (Alice's: {alices})");
    }
    assert_eq!(immune, (true, false), "an immune target ends nothing");
    for (first, ended, shared) in kinds {
        assert_eq!(ended, shared, "{first}");
    }
    assert_eq!(
        gone,
        (false, true),
        "her limited aura ends, the ordinary stays"
    );
}
