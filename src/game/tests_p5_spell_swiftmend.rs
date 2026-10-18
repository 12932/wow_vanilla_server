//! Phase 3, round 5, against Thistle Tea's `swiftmend_test.exs`,
//! `swiftmend_dbc_test.exs` and `druid_test.exs`: Swiftmend wants a
//! Rejuvenation or Regrowth on its target, whoever cast it, and eats the
//! one with the least time left for four or six of its ticks.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, has, health, pass, set_health};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_PERIODIC_HEAL};
use crate::game::map::spells::{self, checks, fail, mods::Mods};

const SWIFTMEND: u32 = 990_100;
const HEAL: u32 = 990_101;
const REJUVENATION: u32 = 990_102;
const REJUVENATION_2: u32 = 990_103;
const REGROWTH: u32 = 990_104;

fn hot(id: u32, flags: u64, amount: i32, duration: u32) -> SpellTemplate {
    let mut s = bare(id, 3);
    s.family_name = 7;
    s.family_flags = flags;
    s.duration_index = duration;
    let mut e = aura(AURA_PERIODIC_HEAL, 0, amount, 21);
    e.amplitude = 3_000;
    effect(&mut s, 0, e);
    s
}

fn druid(d: &mut GameData) {
    let heal = |id: u32| {
        let mut s = bare(id, 3);
        s.family_name = 7;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 10,
                base_points: 0,
                base_dice: 1,
                die_sides: 1,
                target_a: 21,
                ..Default::default()
            },
        );
        s
    };
    let mut swiftmend = heal(SWIFTMEND);
    swiftmend.script_name = "spell_druid_swiftmend".into();
    swiftmend.attributes[2] |= crate::game::data::spells::ATTR_EX2_CANT_CRIT;
    let mut plain = heal(HEAL);
    plain.attributes[2] |= crate::game::data::spells::ATTR_EX2_CANT_CRIT;
    insert(
        d,
        [
            swiftmend,
            plain,
            hot(REJUVENATION, 0x10, 100, LASTS_15S),
            hot(REJUVENATION_2, 0x10, 70, LASTS_LONG),
            hot(REGROWTH, 0x40, 80, LASTS_5S),
        ],
    );
}

fn why(m: &mut Map, caster: Guid, spell: u32, target: Guid) -> Option<u8> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    checks::check(m, caster, &s, &at(target), None, false, &Mods::default())
}

/// The check reads the target's own auras, whoever cast the HoT: none, and
/// it is refused TARGET_AURASTATE; a Rejuvenation or a Regrowth from
/// anyone, and it may go. An ordinary heal wants nothing (Thistle Tea's
/// `validate/6` tests).
#[tokio::test]
async fn swiftmend_wants_a_hot_on_its_target_from_anyone_and_a_plain_heal_none() {
    let realm = fast_realm(druid);
    let (_a, a, map) = kit::player(&realm, "P5SA", "Mender", Race::NightElf, Class::Druid).await;
    let (_b, b, _) = kit::player(&realm, "P5SB", "Mended", Race::NightElf, Class::Druid).await;
    let (_c, c, _) = kit::player(&realm, "P5SC", "Other", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[SWIFTMEND, HEAL]);
            let bare_target = why(m, a, SWIFTMEND, b);
            let plain = why(m, a, HEAL, b);
            spells::cast_triggered(m, c, REJUVENATION, Some(b));
            let rejuvenated = why(m, a, SWIFTMEND, b);
            auras::remove_spell(m, b, REJUVENATION, None);
            spells::cast_triggered(m, b, REGROWTH, Some(b));
            let regrown = why(m, a, SWIFTMEND, b);
            // The mender's own HoT doesn't do for someone else's lack.
            auras::remove_spell(m, b, REGROWTH, None);
            spells::cast_triggered(m, a, REJUVENATION, Some(a));
            let own = why(m, a, SWIFTMEND, b);
            (bare_target, plain, rejuvenated, regrown, own)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (
            Some(fail::TARGET_AURASTATE),
            None,
            None,
            None,
            Some(fail::TARGET_AURASTATE)
        )
    );
}

/// What lands: with no HoT, nothing beyond its own point; with several,
/// the one nearest its end is eaten (Regrowth, six of its ticks) and the
/// rest are kept; with Rejuvenation alone, four of its ticks; an expired
/// HoT gives nothing (Thistle Tea's `receive/4` tests and druid's
/// `consume_swiftmend_hot/3`).
#[tokio::test]
async fn swiftmend_eats_the_hot_nearest_its_end_for_its_ticks() {
    let realm = fast_realm(druid);
    let (_a, a, map) = kit::player(&realm, "P5SD", "Swifty", Race::NightElf, Class::Druid).await;
    let (_b, b, _) = kit::player(&realm, "P5SE", "Other", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let mend = |m: &mut Map| {
                set_health(m, a, 100);
                spells::cast_triggered(m, a, SWIFTMEND, Some(a));
                health(m, a) - 100
            };
            let nothing = mend(m);
            for hot in [REJUVENATION, REJUVENATION_2, REGROWTH] {
                spells::cast_triggered(m, b, hot, Some(a));
            }
            let regrowth = mend(m);
            let kept = (
                has(m, a, REJUVENATION),
                has(m, a, REJUVENATION_2),
                has(m, a, REGROWTH),
            );
            let rejuvenation = mend(m);
            let after = (has(m, a, REJUVENATION), has(m, a, REJUVENATION_2));
            // The long one left runs out (made to): nothing from it.
            pass(m, 1);
            let now = m.now;
            if let Some(x) = auras::auras_mut(m, a) {
                for h in x.holders.iter_mut().filter(|h| h.spell == REJUVENATION_2) {
                    h.expires = Some(now);
                }
            }
            let expired = mend(m);
            (nothing, regrowth, kept, rejuvenation, after, expired)
        })
        .await
        .unwrap();
    let (nothing, regrowth, kept, rejuvenation, after, expired) = out;
    assert_eq!(nothing, 1, "its point alone");
    assert_eq!(regrowth, 80 * 6 + 1, "six of Regrowth's ticks");
    assert_eq!(kept, (true, true, false), "the others kept");
    assert_eq!(rejuvenation, 100 * 4 + 1, "the nearer Rejuvenation's four");
    assert_eq!(after, (false, true));
    assert_eq!(expired, 1, "nothing from a HoT run out");
}

const REJUVENATIONS: [u32; 11] = [
    774, 1058, 1430, 2090, 2091, 3627, 8910, 9839, 9840, 9841, 25299,
];
const REGROWTHS: [u32; 9] = [8936, 8938, 8939, 8940, 8941, 9750, 9856, 9857, 9858];

/// Every rank of Rejuvenation and Regrowth, cast by another druid, ticks
/// every three seconds and makes the real Swiftmend (18562) castable; it
/// eats it for four or six of its ticks, and is refused again after
/// (Thistle Tea's `swiftmend_dbc_test.exs`).
#[tokio::test]
#[ignore = "extended"]
async fn every_real_rank_of_rejuvenation_and_regrowth_feeds_swiftmend() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&18562) {
            s.attributes[2] |= crate::game::data::spells::ATTR_EX2_CANT_CRIT;
        }
    });
    let (_a, a, map) = kit::player(&realm, "P5SF", "Realmend", Race::NightElf, Class::Druid).await;
    let (_b, b, _) = kit::player(&realm, "P5SG", "Realhot", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[18562]);
            kit::ready(m, b, 60, &[]);
            let mut out = Vec::new();
            for (ids, ticks) in [(&REJUVENATIONS[..], 4), (&REGROWTHS[..], 6)] {
                for id in ids {
                    spells::cast_triggered(m, b, *id, Some(a));
                    let x = kit::holder(m, a, *id).and_then(|h| {
                        h.effects
                            .iter()
                            .find(|x| x.kind == AURA_PERIODIC_HEAL)
                            .cloned()
                    });
                    let (amount, period) = x.map_or((0, None), |x| (x.amount, x.period));
                    let may = why(m, a, 18562, a);
                    set_health(m, a, 100);
                    spells::cast_triggered(m, a, 18562, Some(a));
                    let healed = health(m, a) as i32 - 100;
                    let gone = !has(m, a, *id);
                    let again = why(m, a, 18562, a);
                    out.push((*id, period, may, healed - amount * ticks, gone, again));
                }
            }
            out
        })
        .await
        .unwrap();
    for (id, period, may, rest, gone, again) in out {
        assert_eq!(
            period,
            Some(std::time::Duration::from_secs(3)),
            "{id} ticks every three seconds"
        );
        assert_eq!(may, None, "{id} makes Swiftmend castable");
        assert_eq!(rest, 1, "{id}: its ticks and Swiftmend's point");
        assert!(gone, "{id} eaten");
        assert_eq!(again, Some(fail::TARGET_AURASTATE), "{id}: none left");
    }
}
