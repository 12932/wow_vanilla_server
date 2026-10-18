//! Phase 3, round 4, against Thistle Tea's `combat_test.exs`: the melee
//! reach's floor, a creature's damage range from its template, the combat
//! flag's end, extra attacks in a lethal batch, reactions to a blow,
//! Retaliation, and a miss's hit info.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, Outcome, UnitView};
use crate::game::map::creature::Creature;
use crate::game::map::{Map, auras, procs};
use crate::game::tests_p3_combat::*;

fn view(reach: f32) -> UnitView {
    UnitView {
        guid: Guid::zero(),
        position: Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        orientation: 0.0,
        level: 1,
        faction: 0,
        combat_reach: reach,
        alive: true,
        is_player: false,
        standing: true,
        running: false,
    }
}

#[test]
fn the_melee_reach_is_both_reaches_and_four_thirds_but_never_under_five_yards() {
    assert!((combat::melee_range(&view(2.0), &view(3.0)) - 6.333).abs() < 0.001);
    assert_eq!(combat::melee_range(&view(1.5), &view(1.5)), 5.0);
    assert_eq!(combat::melee_range(&view(0.2), &view(0.0)), 5.0);
}

#[test]
fn a_creatures_damage_range_is_its_class_damage_by_the_template_multiplier_once() {
    let mut d = data();
    let mut t = d.creature_templates[&BRUTE].clone();
    t.damage_multiplier = 2.0;
    t.damage_variance = 0.1;
    d.creature_templates.insert(BRUTE, t.clone());
    let c = Creature::from_spawn(
        &d,
        &CreatureSpawn {
            entries: vec![BRUTE],
            ..Default::default()
        },
        &mut rand::rng(),
    )
    .unwrap();
    let f = &c.fields;
    // 30 a blow at 20, doubled, a tenth either way.
    assert!((f.get_f32(UNIT_FIELD_MINDAMAGE) - 54.0).abs() < 0.01);
    assert!((f.get_f32(UNIT_FIELD_MAXDAMAGE) - 66.0).abs() < 0.01);
}

#[tokio::test]
async fn the_combat_flag_goes_when_the_fight_does_or_six_seconds_after_the_last_blow() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let flag = |m: &Map, u: Guid| {
                m.unit_fields(u).unwrap().has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                )
            };
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            forced_swing(m, g, brute, Outcome::Hit);
            let fighting = (flag(m, g), flag(m, brute));
            combat::leave_combat(m, g);
            let left = flag(m, g);
            // Again, and the clock runs out.
            forced_swing(m, g, brute, Outcome::Hit);
            combat::stop_attack(m, g);
            let again = flag(m, g);
            // No creature holds her on its threat list.
            crate::game::map::ai::forget(m, g);
            let past = m.now - (combat::COMBAT_TIMEOUT + Duration::from_millis(100));
            m.players.get_mut(&g).unwrap().combat.last_action = Some(past);
            m.players.get_mut(&g).unwrap().combat.attackers.clear();
            combat::update_players(m);
            (fighting, left, again, flag(m, g))
        })
        .await
        .unwrap();
    assert_eq!(r, ((true, true), false, true, false));
}

#[tokio::test]
async fn a_creatures_extra_attacks_land_and_the_rest_of_a_lethal_batch_dont_swing_at_the_corpse() {
    let (_realm, mut a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let f = m.unit_fields_mut(g).unwrap();
            f.set_u32(UNIT_FIELD_RESISTANCES, 0);
            let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
            f.set_u32(UNIT_FIELD_HEALTH, max.min(1_000));
            // Two extra attacks on a full victim: both land, and the swing.
            combat::grant_extra_attacks(m, brute, 1, 2);
            forced_swing(m, brute, g, Outcome::Hit);
            let after_two = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH);
            (max.min(1_000), after_two)
        })
        .await
        .unwrap();
    let (before, after) = r;
    assert_eq!(
        before - after,
        300,
        "two extra blows and the swing, 100 each"
    );
    for n in 0..3 {
        a.wait_frame(
            &format!("blow {n}"),
            combat::SMSG_ATTACKERSTATEUPDATE,
            |_| true,
        )
        .await;
    }
    // Three owed, the victim dying at the second: two blows and no more.
    let r = map
        .query(move |m| {
            let brute = m.creatures.iter().map(|(_, c)| c.guid).next().unwrap();
            m.unit_fields_mut(g)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 150);
            combat::grant_extra_attacks(m, brute, 1, 3);
            forced_swing(m, brute, g, Outcome::Hit);
            m.players[&g].death != crate::game::map::death::Death::Alive
        })
        .await
        .unwrap();
    assert!(r, "dead");
    for n in 0..2 {
        a.wait_frame(
            &format!("lethal blow {n}"),
            combat::SMSG_ATTACKERSTATEUPDATE,
            |_| true,
        )
        .await;
    }
    let more = tokio::time::timeout(
        Duration::from_millis(300),
        a.wait_frame(
            "a blow at the corpse",
            combat::SMSG_ATTACKERSTATEUPDATE,
            |_| true,
        ),
    )
    .await;
    assert!(more.is_err(), "no blow at the corpse");
}

const FROST_ARMOR: u32 = 97_101;

#[tokio::test]
async fn a_blow_on_the_living_meets_its_reactions_and_a_killing_blow_none() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            // A shield of 7 on the brute, a blow from Alice it lives through.
            aura_of(m, brute, FROST_ARMOR, auras::AURA_DAMAGE_SHIELD, 1, 7);
            let max = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            forced_swing(m, g, brute, Outcome::Hit);
            let hurt = max - m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH);
            // The killing blow: the dead's shield is gone with it.
            m.unit_fields_mut(g)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, max);
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 1);
            forced_swing(m, g, brute, Outcome::Hit);
            let dead = !m.unit_view(brute).unwrap().alive;
            let hurt_dead = max - m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH);
            (hurt, dead, hurt_dead)
        })
        .await
        .unwrap();
    assert_eq!(r, (7, true, 0));
}

const RETALIATION: u32 = 20230;
const RETALIATION_STRIKE: u32 = 22858;

fn with_retaliation() -> GameData {
    let mut d = data();
    let mut s = SpellTemplate {
        id: RETALIATION,
        name: "Retaliation".into(),
        equipped_item_class: -1,
        proc_flags: procs::PROC_TAKEN_MELEE,
        proc_chance: 100,
        proc_charges: 30,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(RETALIATION, s);
    let mut strike = SpellTemplate {
        id: RETALIATION_STRIKE,
        name: "Retaliation".into(),
        equipped_item_class: -1,
        range_index: 1,
        dmg_class: 2,
        ..Default::default()
    };
    strike.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(RETALIATION_STRIKE, strike);
    d
}

#[tokio::test]
async fn retaliation_strikes_back_at_a_blow_from_the_front_for_a_charge_and_not_from_behind() {
    let (_realm, _a, g, map) = setup_with(with_retaliation(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let s = m.realm.data.spells.get(RETALIATION).unwrap().clone();
            auras::apply_spell(m, g, g, &s, &[(0, 0)], Default::default()).unwrap();
            let charges = |m: &Map| {
                auras::auras(m, g)
                    .and_then(|a| a.holders.iter().find(|h| h.spell == RETALIATION))
                    .map(|h| h.charges)
            };
            let start = charges(m);
            let front = spawn(m, g, BRUTE, 20, 2.0);
            let health = |m: &Map, u: Guid| m.unit_fields(u).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let before = health(m, front);
            forced_swing(m, front, g, Outcome::Hit);
            let struck = before - health(m, front);
            let after_front = charges(m);
            // One behind her.
            let back = spawn(m, g, BRUTE, 20, -2.0);
            let before = health(m, back);
            forced_swing(m, back, g, Outcome::Hit);
            (
                start,
                after_front,
                struck,
                charges(m),
                before - health(m, back),
            )
        })
        .await
        .unwrap();
    let (start, after_front, struck, after_back, struck_back) = r;
    assert_eq!(start, Some(30));
    assert_eq!(after_front, Some(29));
    assert!(struck > 0, "the strike back");
    assert_eq!(after_back, Some(29));
    assert_eq!(struck_back, 0);
}

#[tokio::test]
async fn a_missed_swing_says_so_with_the_swings_own_flag_and_nothing_dealt() {
    let (_realm, mut a, g, map) = setup(Class::Warrior).await;
    map.query(move |m| {
        let brute = spawn(m, g, BRUTE, 20, 2.0);
        forced_swing(m, brute, g, Outcome::Miss);
    })
    .await
    .unwrap();
    let frame = a
        .wait_frame("the miss", combat::SMSG_ATTACKERSTATEUPDATE, |_| true)
        .await;
    let (hit_info, damage, state, _) = swing_log(&frame);
    assert_eq!(
        (hit_info, damage, state),
        (
            combat::HITINFO_NORMALSWING2 | combat::HITINFO_MISS,
            0,
            combat::VICTIMSTATE_UNAFFECTED
        )
    );
}
