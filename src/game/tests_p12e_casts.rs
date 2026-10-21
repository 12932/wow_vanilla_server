//! Round 12e: casts mangos-classic's `Spell::CheckCast` and
//! `Spell::CheckTarget` refuse before they cost anything: a charge from a
//! rooted caster; a crowd control at a creature someone else tapped; a
//! drain of a power the target hasn't.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at};
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::harness::Client;
use crate::game::map::auras::AURA_MOD_ROOT;
use crate::game::map::spells;

/// A charge of 8 to 25 yards that gives 9 rage, with a 15-second
/// cooldown; a root.
const CHARGE: u32 = 997_801;
const ROOT: u32 = 997_802;
const RANGE_8_25: u32 = 990_801;
/// A polymorph only for what its caster tapped; a mana burn.
const SHEEP: u32 = 997_803;
const BURN: u32 = 997_804;

fn data(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_8_25, (8.0, 25.0));
    let mut charge = bare(CHARGE, 0);
    charge.range_index = RANGE_8_25;
    charge.recovery_time = 15_000;
    effect(
        &mut charge,
        0,
        SpellEffect {
            effect: spells::EFFECT_CHARGE,
            target_a: 6,
            ..Default::default()
        },
    );
    effect(
        &mut charge,
        1,
        SpellEffect {
            effect: 30,
            base_points: 89,
            base_dice: 1,
            misc_value: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut root = bare(ROOT, 4);
    root.duration_index = LASTS_LONG;
    effect(&mut root, 0, aura(AURA_MOD_ROOT, 0, 0, 1));
    let mut sheep = bare(SHEEP, 6);
    sheep.duration_index = LASTS_5S;
    sheep.range_index = RANGE_8_25;
    sheep.attributes[2] |= crate::game::map::spells::checks::ATTR_EX2_CANNOT_CAST_ON_TAPPED;
    effect(
        &mut sheep,
        0,
        aura(crate::game::map::auras::AURA_MOD_CONFUSE, 0, 0, 6),
    );
    let mut burn = bare(BURN, 6);
    burn.range_index = RANGE_8_25;
    effect(
        &mut burn,
        0,
        SpellEffect {
            effect: 62,
            base_points: 99,
            base_dice: 1,
            misc_value: 0,
            target_a: 6,
            ..Default::default()
        },
    );
    insert(d, [charge, root, sheep, burn]);
}

/// The result of `spell`'s cast as the client hears it: none for a cast
/// that went, else the failure.
async fn cast_result(c: &mut Client, spell: u32) -> Option<u8> {
    let b = c
        .wait_frame(
            "SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |b| b[..4] == spell.to_le_bytes(),
        )
        .await;
    (b[4] != 0).then(|| b[5])
}

/// A rooted warrior's Charge is refused as rooted (0x5f), and costs
/// nothing: no rage, no cooldown. Freed, it goes, and gives its rage.
/// Before, the rooted one's went off on the spot: its rage given, its
/// cooldown begun, and no run.
#[tokio::test]
async fn a_rooted_charge_is_refused_and_costs_nothing() {
    let realm = fast_realm(data);
    let (mut c, w, map) = human(&realm, "P12EC", "Charger", Class::Warrior).await;
    let kobold = map
        .query(move |m| {
            kit::ready(m, w, 60, &[CHARGE]);
            kit::set_power(m, w, 1, 0);
            kit::apply(m, w, w, ROOT);
            kit::dummy(m, w, 60, 15.0)
        })
        .await
        .unwrap();
    c.sees(kobold).await;
    c.clear();
    let (rage, cooling) = map
        .query(move |m| {
            kit::cast_fresh(m, w, CHARGE, at(kobold));
            let cooling = m
                .caster(w)
                .is_some_and(|x| x.cooldowns.contains_key(&CHARGE));
            (kit::power(m, w, 1), cooling)
        })
        .await
        .unwrap();
    assert_eq!(
        cast_result(&mut c, CHARGE).await,
        Some(crate::game::map::spells::checks::FAIL_ROOTED),
        "refused, rooted"
    );
    assert_eq!((rage, cooling), (0, false), "no rage, no cooldown");
    let rage = map
        .query(move |m| {
            crate::game::map::auras::remove_spell(m, w, ROOT, None);
            kit::cast_fresh(m, w, CHARGE, at(kobold));
            kit::power(m, w, 1)
        })
        .await
        .unwrap();
    assert_eq!(cast_result(&mut c, CHARGE).await, None, "freed, it goes");
    assert_eq!(rage, 90, "and gives its rage");
}

/// A mage's polymorph at a kobold another player tapped is refused (0x0e,
/// `SPELL_FAILED_CANT_CAST_ON_TAPPED`), at one no one tapped it goes, and
/// at one the mage tapped too. Before, it went at anyone's.
#[tokio::test]
async fn a_polymorph_is_refused_at_someone_elses_tapped_creature() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P12EK", "Sheeper", Class::Mage).await;
    let (_d, b, _) = human(&realm, "P12EL", "Tapper", Class::Warrior).await;
    let (theirs, free) = map
        .query(move |m| {
            kit::ready(m, a, 60, &[SHEEP]);
            let theirs = kit::dummy(m, a, 60, 15.0);
            let free = kit::dummy(m, a, 60, -15.0);
            let realm = m.realm.clone();
            crate::game::map::tap::take(&realm, m.creature_mut(theirs).unwrap(), b);
            (theirs, free)
        })
        .await
        .unwrap();
    c.sees(theirs).await;
    c.sees(free).await;
    c.clear();
    map.query(move |m| kit::cast_fresh(m, a, SHEEP, at(theirs)))
        .await
        .unwrap();
    assert_eq!(
        cast_result(&mut c, SHEEP).await,
        Some(crate::game::map::spells::checks::FAIL_CANT_CAST_ON_TAPPED),
        "another's tapped kobold: refused"
    );
    map.query(move |m| kit::cast_fresh(m, a, SHEEP, at(free)))
        .await
        .unwrap();
    assert_eq!(
        cast_result(&mut c, SHEEP).await,
        None,
        "an untapped one: it goes"
    );
    let mine = map
        .query(move |m| {
            let realm = m.realm.clone();
            crate::game::map::tap::take(&realm, m.creature_mut(free).unwrap(), a);
            kit::cast_fresh(m, a, SHEEP, at(free));
            kit::has(m, free, SHEEP)
        })
        .await
        .unwrap();
    assert_eq!(
        cast_result(&mut c, SHEEP).await,
        None,
        "one of his own: it goes"
    );
    assert!(mine);
}

/// A mana burn at a kobold of rage is refused (bad targets) before it
/// costs anything; at one of mana it goes. Before, it went off at either,
/// paid for, and burned nothing from the one of rage.
#[tokio::test]
async fn a_mana_burn_is_refused_at_a_unit_without_mana() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P12EB", "Burner", Class::Mage).await;
    let (raging, mana) = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BURN]);
            let raging = kit::dummy(m, a, 60, 15.0);
            let mana = kit::dummy(m, a, 60, -15.0);
            if let Some(f) = m.unit_fields_mut(raging) {
                f.set_byte(crate::game::fields::UNIT_FIELD_BYTES_0, 3, 1);
            }
            (raging, mana)
        })
        .await
        .unwrap();
    c.sees(raging).await;
    c.sees(mana).await;
    c.clear();
    map.query(move |m| kit::cast_fresh(m, a, BURN, at(raging)))
        .await
        .unwrap();
    assert_eq!(
        cast_result(&mut c, BURN).await,
        Some(crate::game::map::spells::fail::BAD_TARGETS),
        "a kobold of rage: refused"
    );
    map.query(move |m| kit::cast_fresh(m, a, BURN, at(mana)))
        .await
        .unwrap();
    assert_eq!(
        cast_result(&mut c, BURN).await,
        None,
        "one of mana: it goes"
    );
}
