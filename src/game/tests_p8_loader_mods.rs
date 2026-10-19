//! Phase 8's loader round, the spell modifiers (Thistle Tea's
//! `world/loader/spell_*_modifiers_dbc_test.exs`, `spell_radius_dbc_test.exs`,
//! `spell_range_dbc_test.exs`, `power_cost_dbc_test.exs`): the real
//! talents and gear passives, and what they change of the real spells.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::auras;
use crate::game::map::spells::outcome::{self, Landing};
use crate::game::map::spells::{self, mods, targets};
use crate::game::map::{Map, MapHandle};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn spell(d: &GameData, id: u32) -> SpellTemplate {
    d.spells
        .get(id)
        .cloned()
        .unwrap_or_else(|| panic!("spell {id}"))
}

/// The most radius of `s`'s effects for `g`, with its modifiers.
fn radius(m: &Map, g: Guid, s: &SpellTemplate) -> f32 {
    let mods = mods::for_spell(m, g, s);
    (0..3)
        .filter(|i| s.effects[*i].effect != 0)
        .map(|i| targets::radius(m, s, i, &mods))
        .fold(0.0, f32::max)
}

/// The most range of `s` for `g`, with its modifiers.
fn range(m: &Map, g: Guid, s: &SpellTemplate) -> f32 {
    let (_, base) = m.realm.data.spells.range(s);
    mods::for_spell(m, g, s).apply(mods::OP_RANGE, base)
}

/// Thistle Tea's radii (`spell_radius_dbc_test.exs`): Improved Frost Nova's
/// ranks are no radius talent, but Arctic Reach (16758) takes Frost Nova
/// to 12 yards; Improved Battle Shout (12838) Battle Shout to 30;
/// Totemic Mastery (16189) and the totem's +5 (21895) Strength of Earth
/// to 30; 23549 Flamestrike to 6.25; 27790 Holy Nova to 12. Each back
/// once the talent is gone.
#[tokio::test]
#[ignore = "extended"]
async fn talents_take_their_spells_radii_as_far_as_they_should() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LRAD", "Radiant", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let d = m.realm.data.clone();
        let mut seen = Vec::new();
        for (talent, id) in [
            (16_758, 122),
            (12_838, 6673),
            (16_189, 8076),
            (21_895, 8076),
            (23_549, 2120),
            (27_790, 15_237),
        ] {
            let s = spell(&d, id);
            let before = radius(m, g, &s);
            apply(m, g, g, talent);
            let with = radius(m, g, &s);
            auras::remove_spell(m, g, talent, None);
            seen.push((talent, with, radius(m, g, &s) == before));
        }
        seen
    })
    .await;
    assert_eq!(
        out,
        vec![
            (16_758, 12.0, true),
            (12_838, 30.0, true),
            (16_189, 30.0, true),
            (21_895, 30.0, true),
            (23_549, 6.25, true),
            (27_790, 12.0, true),
        ]
    );
}

/// Thistle Tea's ranges (`spell_range_dbc_test.exs`): Flame Throwing's
/// Fireball 41 yards, Arctic Reach's Frostbolt 36, Nature's Reach's Wrath
/// 36, Shadow Reach's Shadow Word: Pain 36, Grim Reach's Shadow Bolt 36,
/// Destructive Reach's Corruption 36, Hawk Eye's Auto Shot 41, 23560's
/// Health Funnel 30, 24462's Lightning Bolt 35, 24482's Smite 35,
/// 27790's Smite 36 and 29000's Lightning Bolt 36; each back to its own
/// once the modifier is gone.
#[tokio::test]
#[ignore = "extended"]
async fn talents_and_gear_take_their_spells_ranges_as_far_as_they_should() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LRANGE", "Ranger", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let d = m.realm.data.clone();
        let mut seen = Vec::new();
        for (modifier, id) in [
            (12_353, 133),
            (16_758, 116),
            (16_820, 5176),
            (17_325, 589),
            (17_918, 686),
            (18_219, 172),
            (19_500, 75),
            (23_560, 136),
            (24_462, 403),
            (24_482, 585),
            (27_790, 585),
            (29_000, 403),
        ] {
            let s = spell(&d, id);
            let own = d.spells.range(&s).1;
            apply(m, g, g, modifier);
            let with = range(m, g, &s);
            auras::remove_spell(m, g, modifier, None);
            seen.push((modifier, with, range(m, g, &s) == own));
        }
        seen
    })
    .await;
    assert_eq!(
        out,
        vec![
            (12_353, 41.0, true),
            (16_758, 36.0, true),
            (16_820, 36.0, true),
            (17_325, 36.0, true),
            (17_918, 36.0, true),
            (18_219, 36.0, true),
            (19_500, 41.0, true),
            (23_560, 30.0, true),
            (24_462, 35.0, true),
            (24_482, 35.0, true),
            (27_790, 36.0, true),
            (29_000, 36.0, true),
        ]
    );
}

/// `id`'s first effect's damage from `g` on `foe` at a base of 100, with
/// `g`'s modifiers.
fn hit_of(m: &Map, g: Guid, foe: Guid, id: u32) -> u32 {
    let s = spell(&m.realm.data, id);
    let l = Landing {
        mods: mods::for_spell(m, g, &s),
        ..Default::default()
    };
    let i = s
        .effects
        .iter()
        .position(|e| matches!(e.effect, 2 | 17 | 58 | 121))
        .unwrap_or(0);
    outcome::damage(m, g, foe, &s, i, 100.0, &l).amount
}

/// Thistle Tea's flat damage (`spell_amount_modifiers_dbc_test.exs`):
/// 21229 adds 45 to Frostbolt's first rank and its last (10181); 22855
/// 8 to the imp's Firebolt (3110); 28844 75 to Revenge (6572). Improved
/// Bloodrage (12301, 12818) makes Bloodrage's 10 rage 12 and 15. Improved
/// Fireball's DoT (21230) adds 24 to Fireball's burn, nothing to its hit.
#[tokio::test]
#[ignore = "extended"]
async fn flat_bonuses_add_to_damage_rage_and_a_burn_alone() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LFLAT", "Flatter", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let foe = dummy(m, g, 60, 5.0);
        let mut added = Vec::new();
        for (id, modifier) in [
            (116, 21_229),
            (10_181, 21_229),
            (3110, 22_855),
            (6572, 28_844),
        ] {
            let before = hit_of(m, g, foe, id);
            apply(m, g, g, modifier);
            let with = hit_of(m, g, foe, id);
            auras::remove_spell(m, g, modifier, None);
            added.push((id, with as i64 - before as i64));
        }
        let mut rage = Vec::new();
        for talent in [12_301, 12_818] {
            apply(m, g, g, talent);
            set_power(m, g, 1, 0);
            fresh(m, g);
            spells::cast_triggered(m, g, 2687, Some(g));
            rage.push(power(m, g, 1));
            auras::remove_spell(m, g, talent, None);
            auras::remove_spell(m, g, 29_131, None);
        }
        let plain_hit = hit_of(m, g, foe, 133);
        let burn = |m: &mut Map| {
            auras::remove_spell(m, foe, 133, None);
            spells::cast_triggered(m, g, 133, Some(foe));
            holder(m, foe, 133)
                .and_then(|h| {
                    h.effects
                        .iter()
                        .find(|x| x.kind == auras::AURA_PERIODIC_DAMAGE)
                        .map(|x| x.amount)
                })
                .unwrap_or(0)
        };
        let plain_burn = burn(m);
        apply(m, g, g, 21_230);
        let hit = hit_of(m, g, foe, 133);
        let burned = burn(m);
        (added, rage, hit == plain_hit, burned - plain_burn)
    })
    .await;
    assert_eq!(
        out.0,
        vec![(116, 45), (10_181, 45), (3110, 8), (6572, 75)],
        "flat damage"
    );
    assert_eq!(out.1, vec![120, 150], "Improved Bloodrage");
    assert!(out.2, "Fireball's hit as it was");
    assert_eq!(out.3, 24, "its burn 24 more");
}

/// The amount of `kind` in `g`'s holder of `spell`.
fn amount(m: &Map, g: Guid, spell: u32, kind: u32) -> Option<i32> {
    holder(m, g, spell).and_then(|h| h.effects.iter().find(|x| x.kind == kind).map(|x| x.amount))
}

/// Thistle Tea's aura modifiers (`spell_aura_modifiers_dbc_test.exs`):
/// Improved Seal of the Crusader's third rank (20337) makes the seal's
/// attack power 15% more and leaves its 40% haste; Improved Enslave
/// Demon's fifth (18825) takes Enslave Demon's slows from 40% and 30% to
/// 30% and 20%; Bonescythe Breastplate (28107) gives Slice and Dice's
/// haste 6% more. A trade's cast (Smelt Mithril, 10097, a tradeskill)
/// takes no casting slow; Fireball does.
#[tokio::test]
#[ignore = "extended"]
async fn talents_change_their_auras_amounts_and_trades_take_no_cast_slow() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    assert!(spell(&d, 10_097).attr(0, crate::game::data::spells::ATTR_TRADESKILL));
    assert!(!spell(&d, 133).attr(0, crate::game::data::spells::ATTR_TRADESKILL));
    let (_c, g, map) = player(&realm, "P8LAURAMOD", "Auramod", Race::Human, Class::Paladin).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let foe = dummy(m, g, 60, 5.0);
        let with = |m: &mut Map, talent: u32, id: u32, on: Guid, kinds: &[u32]| {
            auras::remove_spell(m, on, id, None);
            fresh(m, g);
            spells::cast_triggered(m, g, id, Some(on));
            let plain: Vec<Option<i32>> = kinds.iter().map(|k| amount(m, on, id, *k)).collect();
            auras::remove_spell(m, on, id, None);
            apply(m, g, g, talent);
            fresh(m, g);
            spells::cast_triggered(m, g, id, Some(on));
            let more: Vec<Option<i32>> = kinds.iter().map(|k| amount(m, on, id, *k)).collect();
            auras::remove_spell(m, g, talent, None);
            auras::remove_spell(m, on, id, None);
            (plain, more)
        };
        let seal = with(
            m,
            20_337,
            20_308,
            g,
            &[auras::AURA_MOD_ATTACK_POWER, auras::AURA_MOD_ATTACKSPEED],
        );
        let enslave = with(
            m,
            18_825,
            1098,
            foe,
            &[auras::AURA_MOD_HASTE, auras::AURA_MOD_CASTING_SPEED],
        );
        let bonescythe = with(m, 28_107, 6774, g, &[auras::AURA_MOD_HASTE]);
        if let Some(f) = m.unit_fields_mut(g) {
            f.set_f32(UNIT_MOD_CAST_SPEED, 1.5);
        }
        let s = |id| spell(&m.realm.data, id);
        let none = mods::Mods::default();
        let times = (
            spells::cast_time(m, g, &s(10_097), &none),
            m.realm.data.spells.cast_time(&s(10_097)),
            spells::cast_time(m, g, &s(133), &none),
            m.realm.data.spells.cast_time(&s(133)),
        );
        (seal, enslave, bonescythe, times)
    })
    .await;
    let (seal, enslave, bonescythe, times) = out;
    let ap = seal.0[0].unwrap();
    assert_eq!(seal.1[0], Some(ap * 115 / 100), "{seal:?}");
    assert_eq!(seal.0[1], seal.1[1], "the haste as it was");
    assert_eq!(seal.1[1], Some(40));
    assert_eq!(enslave.0, vec![Some(-40), Some(-30)]);
    assert_eq!(enslave.1, vec![Some(-30), Some(-20)]);
    // 30 and 6% more: 31.8, 31 in mangos's whole numbers (Thistle Tea
    // keeps the fraction).
    assert_eq!(bonescythe.0, vec![Some(30)]);
    assert_eq!(bonescythe.1, vec![Some(31)]);
    assert_eq!(times.0, times.1, "a trade's cast takes no slow");
    assert_eq!(times.2 as f32, times.3 as f32 * 1.5, "Fireball does");
}

/// Thistle Tea's Burst of Knowledge (`power_cost_dbc_test.exs`): 15646
/// takes 100 off every school's spell's mana (its school mask 126), into
/// the power cost modifier fields; Greater Heal (10396) costs 100 less
/// while it lasts, and its cost again once it is cancelled or runs out.
#[tokio::test]
#[ignore = "extended"]
async fn burst_of_knowledge_takes_a_hundred_off_while_it_lasts() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let burst = spell(&d, 15_646);
    let e = &burst.effects[0];
    assert_eq!(
        (e.aura, e.base_points, e.misc_value),
        (auras::AURA_MOD_POWER_COST_SCHOOL, -101, 126)
    );
    let (_c, g, map) = player(&realm, "P8LBURST", "Burster", Race::Human, Class::Priest).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let heal = spell(&m.realm.data, 10_396);
        let cost = |m: &Map| spells::power_cost(m, g, &heal);
        let field = |m: &Map| {
            (0..7)
                .map(|i| {
                    m.unit_fields(g)
                        .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER_COST_MODIFIER + i) as i32)
                })
                .collect::<Vec<_>>()
        };
        let base = cost(m);
        apply(m, g, g, 15_646);
        let active = (cost(m), field(m));
        auras::cancel(m, g, 15_646);
        let cancelled = (cost(m), field(m));
        apply(m, g, g, 15_646);
        let lasts = holder(m, g, 15_646).and_then(|h| h.duration);
        pass(m, u64::from(lasts.unwrap_or(0)) + 1);
        let expired = (cost(m), field(m), has(m, g, 15_646));
        (base, active, cancelled, lasts, expired)
    })
    .await;
    let (base, active, cancelled, lasts, expired) = out;
    assert!(base > 100, "{base}");
    assert_eq!(active.0, base - 100);
    assert_eq!(active.1, vec![0, -100, -100, -100, -100, -100, -100]);
    assert_eq!(cancelled, (base, vec![0; 7]));
    assert_eq!(lasts, Some(10_000));
    assert_eq!(expired, (base, vec![0; 7], false));
}
