//! The paladin's and shaman's talents the Phase 1 audits found wvs left
//! out, on the real data: Divine Favor's spells and charge, Improved Seal
//! of Righteousness on the seal, Improved Lay on Hands' armor; the
//! totems' party (a raid's subgroup) and their shaman's modifiers
//! (Enhancing, Guardian and Restorative Totems, Earth's Grasp), Elemental
//! Weapons on Rockbiter and Flametongue.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_combat::equip;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, health};
use crate::game::Realm;
use crate::game::fields::*;
use crate::game::map::spells::{self, Targets, mods};
use crate::game::map::{Map, MapHandle};
use crate::game::social::groups;

const DIVINE_FAVOR: u32 = 20216;
const HOLY_LIGHT: u32 = 635;
const FLASH_OF_LIGHT_HEAL: u32 = 19993;
const FLASH_OF_LIGHT: u32 = 19750;
const HOLY_SHOCK: u32 = 20473;
const SEAL_OF_RIGHTEOUSNESS: u32 = 20154;
const IMPROVED_SOR_5: u32 = 20332;
const LAY_ON_HANDS: u32 = 633;
const IMPROVED_LOH_2: u32 = 20235;
const LOH_ARMOR_2: u32 = 20236;
const STRENGTH_OF_EARTH_5: u32 = 25361;
const STRENGTH_OF_EARTH_AURA_5: u32 = 25362;
const STONESKIN_6: u32 = 10408;
const STONESKIN_AURA_6: u32 = 10405;
const MANA_SPRING_4: u32 = 10497;
const MANA_SPRING_AURA_4: u32 = 10494;
const STONECLAW_6: u32 = 10428;
const STONECLAW_TOTEM: u32 = 7399;
const ROCKBITER_7: u32 = 16316;
const ROCKBITER_AURA_7: u32 = 16313;
const FLAMETONGUE_PROC_4: u32 = 16344;
const ENHANCING_TOTEMS_2: u32 = 16295;
const GUARDIAN_TOTEMS_2: u32 = 16293;
const RESTORATIVE_TOTEMS_5: u32 = 16208;
const EARTHS_GRASP_2: u32 = 16130;
const ELEMENTAL_WEAPONS_3: u32 = 29080;
/// The four totems a shaman carries.
const TOTEMS: [u32; 4] = [5175, 5176, 5177, 5178];
const WORN_SHORTSWORD: u32 = 25;
const MAIN_HAND: u8 = 15;

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// A level-60 shaman with their totems, knowing `talents`.
fn shaman(m: &mut Map, g: Guid, talents: &[u32]) {
    kit::ready(m, g, 60, talents);
    for t in TOTEMS {
        let _ = crate::game::map::inventory::add_item(m, g, t, 1, false);
    }
}

/// Totems' turns, three of them, a second and more apart.
fn totems_think(m: &mut Map) {
    for _ in 0..3 {
        kit::pass(m, 1_100);
        crate::game::map::pets::update(m);
    }
}

/// The first effect's amount of `spell`'s aura on `g`.
fn amount(m: &Map, g: Guid, spell: u32) -> Option<i32> {
    kit::holder(m, g, spell).map(|h| h.effects[0].amount)
}

fn realm() -> Option<Arc<Realm>> {
    kit::have_data().then(kit::realm)
}

/// A totem's aura reaches its shaman's subgroup in a raid, not the other
/// subgroups (mangos's `AREA_AURA_PARTY`).
#[tokio::test]
#[ignore = "extended"]
async fn a_totems_aura_reaches_its_shamans_subgroup_not_the_whole_raid() {
    let Some(realm) = realm() else { return };
    let (_a, s, map) = kit::player(&realm, "TPSTG1", "Totema", Race::Orc, Class::Shaman).await;
    let (_b, near, _) = kit::player(&realm, "TPSTG2", "Totemb", Race::Orc, Class::Warrior).await;
    let (_c, far, _) = kit::player(&realm, "TPSTG3", "Totemc", Race::Orc, Class::Warrior).await;
    let lo = realm.registry.online(s).unwrap();
    for g in [near, far] {
        let o = realm.registry.online(g).unwrap();
        assert!(groups::join_directly(&realm, &lo, &o));
    }
    groups::convert_to_raid(&realm, &lo);
    groups::change_subgroup(&realm, &lo, "Totemc", 1);
    let out = on(&map, move |m| {
        shaman(m, s, &[]);
        let at_shaman = m.players[&s].position();
        for g in [near, far] {
            kit::ready(m, g, 60, &[]);
            super::tests_p6_wspell::put(m, g, at_shaman);
        }
        assert!(cast_fresh(m, s, STRENGTH_OF_EARTH_5, Targets::default()));
        totems_think(m);
        [s, near, far].map(|g| kit::has(m, g, STRENGTH_OF_EARTH_AURA_5))
    })
    .await;
    assert_eq!(out, [true, true, false], "shaman, their subgroup, another");
}

/// The totems' auras take their shaman's modifiers: Enhancing Totems'
/// Strength of Earth, Guardian Totems' Stoneskin, Restorative Totems' Mana
/// Spring (mangos's `CalculateSpellEffectValue` through the totem's
/// modifier owner).
#[tokio::test]
#[ignore = "extended"]
async fn totem_auras_take_their_shamans_talents() {
    let Some(realm) = realm() else { return };
    let (_a, plain, map) = kit::player(&realm, "TPSTT1", "Totemd", Race::Orc, Class::Shaman).await;
    let (_b, keen, _) = kit::player(&realm, "TPSTT2", "Toteme", Race::Orc, Class::Shaman).await;
    let out = on(&map, move |m| {
        shaman(m, plain, &[]);
        shaman(
            m,
            keen,
            &[ENHANCING_TOTEMS_2, GUARDIAN_TOTEMS_2, RESTORATIVE_TOTEMS_5],
        );
        let mut got = Vec::new();
        for g in [plain, keen] {
            let mut of = Vec::new();
            for (totem, aura) in [
                (STRENGTH_OF_EARTH_5, STRENGTH_OF_EARTH_AURA_5),
                (STONESKIN_6, STONESKIN_AURA_6),
                (MANA_SPRING_4, MANA_SPRING_AURA_4),
            ] {
                assert!(cast_fresh(m, g, totem, Targets::default()));
                totems_think(m);
                of.push(amount(m, g, aura).unwrap_or(0));
            }
            got.push(of);
        }
        got
    })
    .await;
    let (plain, keen) = (&out[0], &out[1]);
    assert!(plain.iter().all(|a| *a != 0), "{out:?}");
    assert_eq!(keen[0], plain[0] * 115 / 100, "Strength of Earth: {out:?}");
    assert_eq!(keen[1], plain[1] * 120 / 100, "Stoneskin: {out:?}");
    assert_eq!(keen[2], plain[2] * 125 / 100, "Mana Spring: {out:?}");
}

/// Earth's Grasp raises Stoneclaw Totem's health by half at two ranks
/// (mangos's `EffectSummonTotem`: the effect's points, all effects'
/// modifiers in them).
#[tokio::test]
#[ignore = "extended"]
async fn earths_grasp_raises_stoneclaws_health() {
    let Some(realm) = realm() else { return };
    let (_a, plain, map) = kit::player(&realm, "TPSEG1", "Totemf", Race::Orc, Class::Shaman).await;
    let (_b, keen, _) = kit::player(&realm, "TPSEG2", "Totemg", Race::Orc, Class::Shaman).await;
    let out = on(&map, move |m| {
        shaman(m, plain, &[]);
        shaman(m, keen, &[EARTHS_GRASP_2]);
        [plain, keen].map(|g| {
            assert!(cast_fresh(m, g, STONECLAW_6, Targets::default()));
            m.creatures
                .iter()
                .map(|(_, c)| c)
                .find(|c| c.owner == Some(g) && c.entry == STONECLAW_TOTEM)
                .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_MAXHEALTH))
        })
    })
    .await;
    assert!(out[0] > 0, "{out:?}");
    assert_eq!(out[1], out[0] * 150 / 100, "Stoneclaw's health: {out:?}");
}

/// Elemental Weapons raises Rockbiter's attack power and Flametongue's
/// strike (its damage modifier touches 10444, as mangos casts the strike
/// with its caster's modifiers).
#[tokio::test]
#[ignore = "extended"]
async fn elemental_weapons_raises_rockbiter_and_flametongue() {
    let Some(realm) = realm() else { return };
    let (_a, plain, map) = kit::player(&realm, "TPSEW1", "Weapona", Race::Orc, Class::Shaman).await;
    let (_b, keen, _) = kit::player(&realm, "TPSEW2", "Weaponb", Race::Orc, Class::Shaman).await;
    let out = on(&map, move |m| {
        m.spells.rolls = Some(false);
        let mut got = Vec::new();
        for (g, talents) in [(plain, &[][..]), (keen, &[ELEMENTAL_WEAPONS_3][..])] {
            shaman(m, g, talents);
            equip(m, g, WORN_SHORTSWORD, MAIN_HAND);
            let weapon = m.players[&g]
                .item_at(crate::game::store::BAG_PLAYER, MAIN_HAND)
                .map(|i| m.players[&g].items[i].guid);
            assert!(cast_fresh(
                m,
                g,
                ROCKBITER_7,
                Targets {
                    item: weapon,
                    ..Default::default()
                }
            ));
            crate::game::map::inventory::sync_gear(m, g);
            let ap = amount(m, g, ROCKBITER_AURA_7).unwrap_or(0);
            let kobold = kit::dummy(m, g, 60, 3.0);
            let before = health(m, kobold);
            spells::cast_triggered(m, g, FLAMETONGUE_PROC_4, Some(kobold));
            kit::pass(m, 2_000);
            got.push((ap, before - health(m, kobold)));
        }
        got
    })
    .await;
    let ((ap, fire), (keen_ap, keen_fire)) = (out[0], out[1]);
    assert!(ap > 0 && fire > 0, "{out:?}");
    assert_eq!(keen_ap, ap * 120 / 100, "Rockbiter's attack power: {out:?}");
    assert_eq!(keen_fire, fire * 115 / 100, "Flametongue's strike: {out:?}");
}

/// Improved Seal of Righteousness raises the seal itself, whose amount
/// makes its strikes, not only its Judgement (1.12's tooltip; mangos's
/// `spell_affect` 0x8000400): 15% more at five ranks.
#[tokio::test]
#[ignore = "extended"]
async fn improved_seal_of_righteousness_raises_the_seal() {
    let Some(realm) = realm() else { return };
    let (_a, plain, map) =
        kit::player(&realm, "TPSSR1", "Sealera", Race::Human, Class::Paladin).await;
    let (_b, keen, _) = kit::player(&realm, "TPSSR2", "Sealerb", Race::Human, Class::Paladin).await;
    let out = on(&map, move |m| {
        kit::ready(m, plain, 60, &[SEAL_OF_RIGHTEOUSNESS]);
        kit::ready(m, keen, 60, &[SEAL_OF_RIGHTEOUSNESS, IMPROVED_SOR_5]);
        let mut got = Vec::new();
        for g in [plain, keen] {
            assert!(cast_fresh(m, g, SEAL_OF_RIGHTEOUSNESS, at(g)));
            got.push(amount(m, g, SEAL_OF_RIGHTEOUSNESS).unwrap_or(0));
        }
        got
    })
    .await;
    assert!(out[0] > 0, "{out:?}");
    assert_eq!(out[1], out[0] * 115 / 100, "{out:?}");
}

/// Divine Favor's crit is Holy Light's (a direct heal by vmangos's
/// `spell_effect_mod`, its flag 0x80000000 added to the mask as mangos's
/// `spell_affect` 0xC0200000 had it) and Flash of Light's heal's (19993,
/// 0x2000 in the DBC's mask), and its one charge goes with the heal that
/// used it: a Holy Light's, a Holy Shock's.
#[tokio::test]
#[ignore = "extended"]
async fn divine_favor_crits_holy_light_flash_and_shock_and_goes_with_the_first() {
    let Some(realm) = realm() else { return };
    let (_c, p, map) = kit::player(&realm, "TPSDF1", "Favora", Race::Human, Class::Paladin).await;
    let out = on(&map, move |m| {
        kit::ready(
            m,
            p,
            60,
            &[DIVINE_FAVOR, HOLY_LIGHT, FLASH_OF_LIGHT, HOLY_SHOCK],
        );
        let data = m.realm.data.clone();
        let crit = |m: &Map, id: u32| {
            mods::for_spell(m, p, data.spells.get(id).unwrap()).flat[mods::OP_CRITICAL_CHANCE]
        };
        assert!(cast_fresh(m, p, DIVINE_FAVOR, at(p)));
        let crits = (crit(m, HOLY_LIGHT), crit(m, FLASH_OF_LIGHT_HEAL));
        kit::set_health(m, p, 100);
        assert!(cast_fresh(m, p, HOLY_LIGHT, at(p)));
        kit::pass(m, 3_000);
        let after_light = kit::has(m, p, DIVINE_FAVOR);
        crate::game::map::auras::remove_spell(m, p, DIVINE_FAVOR, None);
        assert!(cast_fresh(m, p, DIVINE_FAVOR, at(p)));
        assert!(cast_fresh(m, p, HOLY_SHOCK, at(p)));
        kit::pass(m, 100);
        let after_shock = kit::has(m, p, DIVINE_FAVOR);
        (crits, after_light, after_shock)
    })
    .await;
    assert_eq!(
        out.0,
        (100, 100),
        "Holy Light's and Flash of Light's heals' crit"
    );
    assert!(!out.1, "Divine Favor spent by a Holy Light");
    assert!(!out.2, "Divine Favor spent by a Holy Shock");
}

/// Improved Lay on Hands' armor goes on the one healed, a friend or the
/// paladin themself (mangos's 20233 at the victim of the proc).
#[tokio::test]
#[ignore = "extended"]
async fn improved_lay_on_hands_armors_whom_it_heals() {
    let Some(realm) = realm() else { return };
    let (_a, p, map) = kit::player(&realm, "TPSLH1", "Layera", Race::Human, Class::Paladin).await;
    let (_b, friend, _) =
        kit::player(&realm, "TPSLH2", "Layerb", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        kit::ready(m, p, 60, &[LAY_ON_HANDS, IMPROVED_LOH_2]);
        kit::ready(m, friend, 60, &[]);
        kit::set_health(m, friend, 100);
        assert!(cast_fresh(m, p, LAY_ON_HANDS, at(friend)));
        kit::pass(m, 100);
        let friend_armored = kit::has(m, friend, LOH_ARMOR_2);
        let healed = health(m, friend) > 100;
        kit::set_health(m, p, 100);
        assert!(cast_fresh(m, p, LAY_ON_HANDS, at(p)));
        kit::pass(m, 100);
        (healed, friend_armored, kit::has(m, p, LOH_ARMOR_2))
    })
    .await;
    assert!(out.0, "Lay on Hands healed");
    assert!(out.1, "the friend's armor");
    assert!(out.2, "the paladin's own");
}
