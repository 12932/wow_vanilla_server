//! Round 12's weapon and defense skill-ups, as mangos-classic's
//! `UpdateCombatSkills` has them: one curve of the skill, its cap and the
//! level, whatever the creature's level; a gray creature trains a weapon,
//! a player's pet or charm trains no weapon but does train defense.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::derive::SKILL_DEFENSE;
use crate::game::map::{Map, pets, skills};
use crate::game::tests_p3_combat::{self as p3, BRUTE, SWORD};

const SWORDS: u32 = 43;

/// Alice at 60 with a sword, swords and defense at `skill`.
fn trained(m: &mut Map, g: Guid, skill: u32) {
    p3::level(m, g, 60);
    p3::equip(m, g, SWORD, 15);
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.skills_maxed = false;
    for s in [SKILL_DEFENSE, SWORDS] {
        skills::learn_skill(p, s, 300);
        skills::set_value(p, s, skill);
    }
    p.apply_stats(&data);
}

fn heal(m: &mut Map, g: Guid) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_HEALTH, 1_000);
    }
}

/// Up to `n` landed blows of `attacker` on `victim`; what `skill` of
/// `who` came to.
fn blows(m: &mut Map, attacker: Guid, victim: Guid, who: Guid, skill: u32, n: u32) -> u32 {
    for _ in 0..n {
        p3::forced_swing(m, attacker, victim, Outcome::Hit);
        heal(m, victim);
        heal(m, attacker);
    }
    skills::value(&m.players[&who], skill).unwrap()
}

/// mangos's curve: at 280 of 300 at level 60 a blow trains about 21.7%
/// of the time, at 299 one in two thousand, at 1 of 300 87.7%; intellect
/// adds up to a tenth to a weapon's (750 of it at 60), none to defense.
#[test]
fn the_skill_up_curve_is_mangos() {
    let c = combat::combat_skill_chance;
    assert!(
        (c(280, 300, 60, None) - 21.7).abs() < 0.1,
        "{}",
        c(280, 300, 60, None)
    );
    assert!((c(299, 300, 60, None) - 100.0 / 2_100.0).abs() < 0.001);
    assert!((c(1, 300, 60, None) - 87.7).abs() < 0.1);
    assert_eq!(c(300, 300, 60, Some(750)), 0.0);
    let bonus = c(280, 300, 60, Some(750)) - c(280, 300, 60, None);
    assert!((bonus - 10.0).abs() < 0.01, "{bonus}");
    assert!((c(280, 300, 60, Some(375)) - c(280, 300, 60, None) - 5.0).abs() < 0.01);
}

/// mangos has no level gate: a creature gray to Alice trains her new
/// weapon (Thistle Tea's refused it).
#[tokio::test]
async fn a_gray_creature_trains_a_weapon() {
    let (_realm, _a, g, map) = p3::setup_with(p3::data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            trained(m, g, 1);
            let gray = p3::spawn(m, g, BRUTE, 20, 3.0);
            blows(m, g, gray, g, SWORDS, 20)
        })
        .await
        .unwrap();
    assert!(r > 1, "swords {r}");
}

/// mangos's `UpdateWeaponSkill` refuses a player's pet or charm; its
/// blows on a player train their defense (`ProcSkillsAndReactives` asks
/// only that the attacker not be a player nor a critter).
#[tokio::test]
async fn a_players_charm_trains_defense_and_no_weapon() {
    let (_realm, _a, g, map) = p3::setup_with(p3::data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            trained(m, g, 1);
            let c = p3::spawn(m, g, BRUTE, 60, 3.0);
            pets::charmed(m, g, c, false);
            let swords = blows(m, g, c, g, SWORDS, 20);
            let defense = blows(m, c, g, g, SKILL_DEFENSE, 20);
            (swords, defense)
        })
        .await
        .unwrap();
    assert_eq!(r.0, 1, "no weapon skill from a charm");
    assert!(r.1 > 1, "defense from a charm's blows: {}", r.1);
}
