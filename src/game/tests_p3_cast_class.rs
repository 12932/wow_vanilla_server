//! Phase 3's class spells, on the real data, as Thistle Tea's class tests
//! have them: the warrior's strikes, Execute, Shield Wall and Charge, a
//! dodged ability, a cancelled Heroic Strike, and the troll's and the
//! orc's racials.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::spells;

const HEROIC_STRIKE: u32 = 78;
const OVERPOWER: u32 = 7384;
const STRIKE_NOW: u32 = 990_500;
const EXECUTE: u32 = 5308;
const SHIELD_WALL: u32 = 871;
const CHARGE: u32 = 100;
const HAMSTRING: u32 = 1715;
const FROSTBOLT: u32 = 116;
const BERSERKING: u32 = 26296;
const BERSERKING_BUFF: u32 = 26635;
const BLOOD_FURY: u32 = 20572;
const BLOOD_FURY_BUFF: u32 = 23234;
const BLOOD_FURY_PENALTY: u32 = 23230;

fn rage(m: &Map, g: wow_world_messages::Guid) -> u32 {
    power(m, g, 1)
}

/// The warrior's weapon strikes in fixed rolls: Heroic Strike's weapon
/// (its fields' average, attack power in them) and its 11; Overpower's
/// weapon by the normalized speed of 2.4 for a one-hander and its 5;
/// Execute's 125 and three a point of rage, all the rage spent, and kept
/// whole when it is dodged; a dodged Hamstring neither hurts nor slows
/// (Thistle Tea's melee ability resolution).
#[tokio::test]
#[ignore = "extended"]
async fn the_warriors_strikes_are_weapon_and_attack_power_and_their_bonus() {
    if !have_data() {
        return;
    }
    // Heroic Strike cast at once, not on the next swing (whose roll is
    // the swing's).
    let realm = realm_plus(|d| {
        let mut s = d.spells.get(HEROIC_STRIKE).cloned().unwrap();
        s.id = STRIKE_NOW;
        s.attributes[0] &= !crate::game::data::spells::ATTR_ON_NEXT_SWING;
        d.spells.spells.insert(s.id, s);
    });
    let (_c, w, map) = player(&realm, "P3K1", "Strikeone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[STRIKE_NOW, OVERPOWER, EXECUTE, HAMSTRING]);
            let k = dummy(m, w, 60, 2.0);
            let fields = |m: &Map| {
                let f = m.unit_fields(w).unwrap();
                (f.get_f32(UNIT_FIELD_MINDAMAGE) + f.get_f32(UNIT_FIELD_MAXDAMAGE)) / 2.0
            };
            let weapon = fields(m);
            let hurt = |m: &mut Map, spell: u32| {
                set_health(m, k, 100_000);
                set_power(m, w, 1, 1_000);
                cast_fresh(m, w, spell, at(k));
                100_000 - health(m, k)
            };
            let heroic = hurt(m, STRIKE_NOW);
            let normal = {
                let p = m.players.get(&w).unwrap();
                let data = m.realm.data.clone();
                let g = crate::game::map::derive::gear(p, &data);
                let wpn = g.main.unwrap();
                (wpn.min + wpn.max) / 2.0 + p.derived.attack_power as f32 / 14.0 * 2.4
            };
            let overpower = {
                // Overpower wants Battle Stance and a dodge just made.
                apply(m, w, w, 2457);
                crate::game::map::combat::after_outcome(
                    m,
                    w,
                    k,
                    crate::game::map::combat::Outcome::Dodge,
                );
                hurt(m, OVERPOWER)
            };
            // Execute at 20 rage, on the kobold under a fifth.
            set_health(m, k, 100_000);
            if let Some(f) = m.unit_fields_mut(k) {
                f.set_u32(UNIT_FIELD_HEALTH, 100_000 / 10);
            }
            fresh(m, w);
            set_power(m, w, 1, 350);
            let before = health(m, k);
            spells::cast(m, w, EXECUTE, at(k), None);
            let execute = (before - health(m, k), rage(m, w));
            (weapon, heroic, normal, overpower, execute)
        })
        .await
        .unwrap();
    let (weapon, heroic, normal, overpower, execute) = out;
    assert_eq!(
        heroic,
        (weapon + 11.0) as u32,
        "Heroic Strike: the weapon and 11"
    );
    assert_eq!(overpower, (normal + 5.0) as u32, "Overpower, normalized");
    // 35 rage, 15 of it the cost: the 20 left is 60 more.
    assert_eq!(execute, (125 + 200 * 3 / 10, 0), "Execute spends the rest");
}

/// A dodged ability does nothing: Hamstring neither hurts nor slows, and
/// an Execute dodged spends no more than its cost back less; Shield Wall
/// takes three quarters off a blow; Charge's rage goes to the warrior;
/// and a Heroic Strike queued is cancelled by the client's cancel
/// (Thistle Tea's warrior tests).
#[tokio::test]
#[ignore = "extended"]
async fn a_dodged_ability_does_nothing_and_shield_wall_charge_and_a_cancel_do_theirs() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (c, w, map) = player(&realm, "P3K2", "Striketwo", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(
                m,
                w,
                60,
                &[HAMSTRING, EXECUTE, SHIELD_WALL, CHARGE, HEROIC_STRIKE],
            );
            apply(m, w, w, 2457);
            let k = dummy(m, w, 60, 2.0);
            if let Some(c) = m.creature_mut(k) {
                c.derived.dodge = 100.0;
            }
            if let Some(p) = m.players.get_mut(&w) {
                p.derived.hit = 100.0;
            }
            set_power(m, w, 1, 1_000);
            cast_fresh(m, w, HAMSTRING, at(k));
            let hamstrung = (health(m, k), has(m, k, HAMSTRING));
            if let Some(f) = m.unit_fields_mut(k) {
                f.set_u32(UNIT_FIELD_HEALTH, 100_000);
            }
            set_power(m, w, 1, 350);
            cast_fresh(m, w, EXECUTE, at(k));
            let executed = (health(m, k), rage(m, w));
            // Shield Wall, and a frost bolt of 100.
            apply(m, w, w, SHIELD_WALL);
            let bolt = m.realm.data.spells.get(FROSTBOLT).cloned().unwrap();
            let walled = spells::outcome::taken(m, w, &bolt, 100.0) as u32;
            auras::remove_spell(m, w, SHIELD_WALL, None);
            // Charge from 15 yards, at no rage, out of combat.
            if let Some(c) = m.creature_mut(k) {
                c.derived.dodge = 0.0;
            }
            let far = dummy(m, w, 60, 15.0);
            if let Some(p) = m.players.get_mut(&w) {
                p.fields.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    false,
                );
            }
            set_power(m, w, 1, 0);
            let charged = cast_fresh(m, w, CHARGE, at(far));
            let charge = (charged, rage(m, w), rage_of_creature(m, far));
            // Heroic Strike queued.
            set_power(m, w, 1, 1_000);
            cast_fresh(m, w, HEROIC_STRIKE, at(k));
            let queued = m.players[&w].combat.queued;
            (hamstrung, executed, walled, charge, queued)
        })
        .await
        .unwrap();
    let (hamstrung, executed, walled, charge, queued) = out;
    assert_eq!(
        hamstrung,
        (1_000_000, false),
        "a dodged Hamstring does nothing"
    );
    // 35 rage less the 15 it cost, and back four fifths of that.
    assert_eq!(executed.0, 100_000, "a dodged Execute hurts nothing");
    assert!(executed.1 >= 200, "and keeps the rage: {}", executed.1);
    assert_eq!(walled, 25, "Shield Wall takes three quarters off");
    assert!(charge.0, "the charge went");
    assert_eq!(charge.1, 90, "nine rage, the warrior's");
    assert_eq!(charge.2, 0, "none to the target");
    assert_eq!(queued, Some(HEROIC_STRIKE));
    c.send(CMSG_CANCEL_CAST { id: HEROIC_STRIKE }).await;
    let mut gone = false;
    for _ in 0..200 {
        gone = map
            .query(move |m| m.players[&w].combat.queued.is_none())
            .await
            .unwrap();
        if gone {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(gone, "the cancel takes the queued strike off");
}

fn rage_of_creature(m: &Map, g: wow_world_messages::Guid) -> u32 {
    power(m, g, 1)
}

/// Berserking's haste by the whole share of health lost: 10% at full, a
/// third of a percent more for each lost, 30% at 40% health and below;
/// Blood Fury a quarter of the orc's attack power and its healing
/// penalty, a player's own: a creature casting it gains neither (Thistle
/// Tea's `Racial`, mangos's Blood Fury dummy).
#[tokio::test]
#[ignore = "extended"]
async fn berserking_hastes_by_the_health_lost_and_blood_fury_is_a_players() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_t, troll, tmap) = player(&realm, "P3K3", "Berserker", Race::Troll, Class::Warrior).await;
    let haste = tmap
        .query(move |m| {
            ready(m, troll, 60, &[BERSERKING]);
            let max = m.unit_fields(troll).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            [100, 99, 97, 70, 41, 40, 1]
                .map(|pct| {
                    set_health(m, troll, (max * pct).div_ceil(100).max(1));
                    auras::remove_spell(m, troll, BERSERKING_BUFF, None);
                    set_power(m, troll, 1, 1_000);
                    cast_fresh(m, troll, BERSERKING, Default::default());
                    holder(m, troll, BERSERKING_BUFF)
                        .and_then(|h| h.effects.first().map(|x| x.amount))
                        .unwrap_or(-1)
                })
                .to_vec()
        })
        .await
        .unwrap();
    assert_eq!(haste, [10, 10, 11, 20, 29, 30, 30]);
    let (_o, orc, omap) = player(&realm, "P3K4", "Furious", Race::Orc, Class::Warrior).await;
    let out = omap
        .query(move |m| {
            ready(m, orc, 60, &[BLOOD_FURY]);
            let ap = m.players[&orc].derived.attack_power;
            cast_fresh(m, orc, BLOOD_FURY, Default::default());
            let buff =
                holder(m, orc, BLOOD_FURY_BUFF).and_then(|h| h.effects.first().map(|x| x.amount));
            let penalty = has(m, orc, BLOOD_FURY_PENALTY);
            let k = dummy(m, orc, 60, 5.0);
            cast_fresh(m, k, BLOOD_FURY, Default::default());
            let creature = (has(m, k, BLOOD_FURY_BUFF), has(m, k, BLOOD_FURY_PENALTY));
            (ap, buff, penalty, creature)
        })
        .await
        .unwrap();
    let (ap, buff, penalty, creature) = out;
    assert!(ap > 0);
    assert_eq!(
        buff,
        Some(ap as i32 * 25 / 100),
        "a quarter of the attack power"
    );
    assert!(penalty, "and the healing penalty");
    assert_eq!(
        creature,
        (false, false),
        "a creature's Blood Fury does nothing"
    );
}
