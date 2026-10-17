//! Phase 3's combat tests, part eight (Thistle Tea's `mana_shield_test.exs`):
//! Mana Shield's school, its mana a point and Improved Mana Shield's cut,
//! the mana there is, ordinary absorbs first, and what a swing or a spell
//! reports of it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_ADD_PCT_MODIFIER, AURA_MANA_SHIELD, AURA_SCHOOL_ABSORB};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::{Map, spells};
use crate::game::tests_p3_combat::{self as base, *};

const MANA_SHIELD: u32 = 1463;
const FIRE_SHIELD: u32 = 95_700;
const BARRIER: u32 = 11_426;
/// Improved Mana Shield at a tenth and a fifth; one of another family, one
/// of another mask.
const IMPROVED: [u32; 2] = [95_701, 95_702];
const OTHER_FAMILY: u32 = 95_703;
const OTHER_MASK: u32 = 95_704;
const FIREBALL: u32 = 95_705;
/// A shield that costs no mana, and a talent that takes all of it off.
const FREE_SHIELD: u32 = 95_706;
const ALL_OFF: u32 = 95_707;

fn shield(id: u32, school: i32, rate: f32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Shield {id}"),
        equipped_item_class: -1,
        family_name: 3,
        family_flags: 0x8000,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MANA_SHIELD,
        misc_value: school,
        base_points: 119,
        base_dice: 1,
        multiple_value: rate,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn talent(id: u32, family: u32, mask: u64, pct: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Talent {id}"),
        equipped_item_class: -1,
        family_name: family,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_ADD_PCT_MODIFIER,
        misc_value: spells::mods::OP_MULTIPLE_VALUE as i32,
        base_points: pct - 1,
        base_dice: 1,
        item_type: mask,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = base::data();
    let mut all = vec![
        shield(MANA_SHIELD, 1, 2.0),
        shield(FIRE_SHIELD, 4, 3.0),
        talent(IMPROVED[0], 3, 0x8000, -10),
        talent(IMPROVED[1], 3, 0x8000, -20),
        talent(OTHER_FAMILY, 8, 0x8000, -20),
        talent(OTHER_MASK, 3, 1, -20),
        shield(FREE_SHIELD, 1, 0.0),
        talent(ALL_OFF, 3, 0x8000, -100),
    ];
    let mut barrier = SpellTemplate {
        id: BARRIER,
        name: "Barrier".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    barrier.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_SCHOOL_ABSORB,
        misc_value: 127,
        base_points: 49,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    all.push(barrier);
    let mut fireball = SpellTemplate {
        id: FIREBALL,
        name: "Fireball".into(),
        range_index: 1,
        equipped_item_class: -1,
        school: 2,
        dmg_class: 1,
        ..Default::default()
    };
    fireball.effects[0] = SpellEffect {
        effect: 2,
        base_points: 29,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    all.push(fireball);
    for s in all {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn put(m: &mut Map, g: Guid, id: u32) {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    let points = s.effects[0].base_points + 1;
    let _ = auras::apply_spell(m, g, g, &s, &[(0, points)], auras::Context::default());
}

fn mana(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_POWER1)
}

fn set_mana(m: &mut Map, g: Guid, to: u32) {
    let f = m.unit_fields_mut(g).unwrap();
    f.set_u32(UNIT_FIELD_MAXPOWER1, 1_000);
    f.set_u32(UNIT_FIELD_POWER1, to);
}

/// What is left of the shield of `id` on `g`.
fn left(m: &Map, g: Guid, id: u32) -> Option<i32> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == id)
        .map(|h| h.effects.iter().map(|x| x.left).sum())
}

/// A fresh shield of `id` and `mana` on `g`, the talents and barriers gone.
fn fresh(m: &mut Map, g: Guid, id: u32, to: u32) {
    for s in [
        MANA_SHIELD,
        FIRE_SHIELD,
        IMPROVED[0],
        IMPROVED[1],
        OTHER_FAMILY,
        OTHER_MASK,
        BARRIER,
        FREE_SHIELD,
        ALL_OFF,
    ] {
        auras::remove_spell(m, g, s, None);
    }
    put(m, g, id);
    set_mana(m, g, to);
}

#[tokio::test]
async fn mana_shield_soaks_its_school_at_its_rate_and_improved_mana_shield_cuts_the_mana() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            // Other schools pass it by.
            fresh(m, ga, MANA_SHIELD, 200);
            for school in [2, 4, 8, 16, 32, 64] {
                out.push(auras::absorb(m, ga, 50, school));
            }
            out.push((mana(m, ga), left(m, ga, MANA_SHIELD).unwrap() as u32));
            // A fire shield of three mana a point.
            fresh(m, ga, FIRE_SHIELD, 200);
            out.push(auras::absorb(m, ga, 20, 4));
            out.push((mana(m, ga), left(m, ga, FIRE_SHIELD).unwrap() as u32));
            out.push(auras::absorb(m, ga, 20, 1));
            // A tenth and a fifth less mana; on a shield already up too.
            for (talent, want) in [(IMPROVED[0], 146), (IMPROVED[1], 152)] {
                fresh(m, ga, MANA_SHIELD, 200);
                put(m, ga, talent);
                set_mana(m, ga, 200);
                auras::absorb(m, ga, 30, 1);
                out.push((mana(m, ga), want));
            }
            let now = mana(m, ga);
            auras::remove_spell(m, ga, IMPROVED[1], None);
            set_mana(m, ga, now);
            auras::absorb(m, ga, 30, 1);
            out.push((mana(m, ga), 92));
            // Another family's or mask's: no cut.
            for other in [OTHER_FAMILY, OTHER_MASK] {
                fresh(m, ga, MANA_SHIELD, 200);
                put(m, ga, other);
                set_mana(m, ga, 200);
                auras::absorb(m, ga, 30, 1);
                out.push((mana(m, ga), 140));
            }
            out
        })
        .await
        .unwrap();
    for x in &r[..6] {
        assert_eq!(*x, (50, 0));
    }
    assert_eq!(r[6], (200, 120));
    assert_eq!(r[7], (0, 20));
    assert_eq!(r[8], (140, 100));
    assert_eq!(r[9], (20, 0));
    for (got, want) in &r[10..] {
        assert_eq!(got, want);
    }
}

#[tokio::test]
async fn mana_shield_soaks_no_more_than_the_mana_pays_for_and_at_no_rate_soaks_freely() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            // Sixteen mana at 1.6 a point: ten soaked, the shield kept.
            fresh(m, ga, MANA_SHIELD, 16);
            put(m, ga, IMPROVED[1]);
            set_mana(m, ga, 16);
            out.push((
                auras::absorb(m, ga, 30, 1),
                mana(m, ga),
                left(m, ga, MANA_SHIELD),
            ));
            out.push((
                auras::absorb(m, ga, 30, 1),
                mana(m, ga),
                left(m, ga, MANA_SHIELD),
            ));
            // Seventeen: still ten, one mana left that pays for nothing.
            fresh(m, ga, MANA_SHIELD, 17);
            put(m, ga, IMPROVED[1]);
            set_mana(m, ga, 17);
            out.push((
                auras::absorb(m, ga, 30, 1),
                mana(m, ga),
                left(m, ga, MANA_SHIELD),
            ));
            out.push((
                auras::absorb(m, ga, 1, 1),
                mana(m, ga),
                left(m, ga, MANA_SHIELD),
            ));
            // No rate, or all of it taken off: soaked with no mana at all.
            fresh(m, ga, FREE_SHIELD, 0);
            out.push((
                auras::absorb(m, ga, 30, 1),
                mana(m, ga),
                left(m, ga, FREE_SHIELD),
            ));
            fresh(m, ga, MANA_SHIELD, 0);
            put(m, ga, ALL_OFF);
            set_mana(m, ga, 0);
            out.push((
                auras::absorb(m, ga, 30, 1),
                mana(m, ga),
                left(m, ga, MANA_SHIELD),
            ));
            auras::remove_spell(m, ga, ALL_OFF, None);
            auras::remove_spell(m, ga, FREE_SHIELD, None);
            out
        })
        .await
        .unwrap();
    assert_eq!(r[0], ((20, 10), 0, Some(110)));
    assert_eq!(r[1], ((30, 0), 0, Some(110)));
    assert_eq!(r[2], ((20, 10), 1, Some(110)));
    assert_eq!(r[3], ((1, 0), 1, Some(110)));
    assert_eq!(r[4], ((0, 30), 0, Some(90)));
    assert_eq!(r[5], ((0, 30), 0, Some(90)));
}

#[tokio::test]
async fn ordinary_absorbs_go_first_and_a_used_up_shield_goes() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            // Either order, the barrier's 50 first, then 20 from the shield.
            for first in [MANA_SHIELD, BARRIER] {
                fresh(m, ga, first, 200);
                put(
                    m,
                    ga,
                    if first == BARRIER {
                        MANA_SHIELD
                    } else {
                        BARRIER
                    },
                );
                set_mana(m, ga, 200);
                let soaked = auras::absorb(m, ga, 70, 1);
                out.push((
                    soaked,
                    mana(m, ga),
                    left(m, ga, MANA_SHIELD),
                    left(m, ga, BARRIER),
                ));
            }
            // With no mana, the barrier still protects.
            fresh(m, ga, MANA_SHIELD, 0);
            put(m, ga, BARRIER);
            set_mana(m, ga, 0);
            out.push((
                auras::absorb(m, ga, 70, 1),
                mana(m, ga),
                left(m, ga, MANA_SHIELD),
                left(m, ga, BARRIER),
            ));
            // Used up: the shield goes, the rest gets through.
            fresh(m, ga, MANA_SHIELD, 300);
            out.push((
                auras::absorb(m, ga, 150, 1),
                mana(m, ga),
                left(m, ga, MANA_SHIELD),
                None,
            ));
            out
        })
        .await
        .unwrap();
    for x in &r[..2] {
        assert_eq!(*x, ((0, 70), 160, Some(100), None));
    }
    assert_eq!(r[2], ((20, 50), 0, Some(120), None));
    assert_eq!(r[3], ((30, 120), 60, None, None));
}

#[tokio::test]
async fn a_swing_reports_what_the_shield_soaked_and_a_spell_of_another_school_passes_it() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    let (whole, partial, fire) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            let f = m.unit_fields_mut(brute).unwrap();
            f.set_f32(UNIT_FIELD_MINDAMAGE, 30.0);
            f.set_f32(UNIT_FIELD_MAXDAMAGE, 30.0);
            m.unit_fields_mut(ga)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 0);
            let health = |m: &Map| m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let full = |m: &mut Map| {
                let f = m.unit_fields_mut(ga).unwrap();
                f.set_u32(UNIT_FIELD_MAXHEALTH, 500);
                f.set_u32(UNIT_FIELD_HEALTH, 500);
            };
            fresh(m, ga, MANA_SHIELD, 200);
            full(m);
            forced_swing(m, brute, ga, Outcome::Hit);
            let whole = (health(m), mana(m, ga));
            fresh(m, ga, MANA_SHIELD, 20);
            full(m);
            forced_swing(m, brute, ga, Outcome::Hit);
            let partial = (health(m), mana(m, ga));
            fresh(m, ga, MANA_SHIELD, 200);
            full(m);
            spells::cast_triggered(m, brute, FIREBALL, Some(ga));
            let fire = (health(m), mana(m, ga), left(m, ga, MANA_SHIELD));
            (whole, partial, fire)
        })
        .await
        .unwrap();
    assert_eq!(whole, (500, 140));
    assert_eq!(partial, (480, 0));
    assert_eq!(fire, (470, 200, Some(120)));
    // The logs: all soaked, then ten of thirty.
    let mut logs = Vec::new();
    for _ in 0..2 {
        let b = a
            .wait_frame("a swing", combat::SMSG_ATTACKERSTATEUPDATE, |_| true)
            .await;
        let n = b.len();
        let u = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let mut at = 4;
        for _ in 0..2 {
            at += 1 + b[at].count_ones() as usize;
        }
        // Damage, then (after school and float) the absorbed.
        logs.push((u(at), u(at + 17)));
        let _ = n;
    }
    assert_eq!(logs, vec![(0, 30), (20, 10)]);
}
