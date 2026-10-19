//! Round 6's combat tests, who is in a fight and what opens a window
//! (Thistle Tea's `periodic_combat_test.exs` and `reactive_test.exs`):
//! damage from the world or oneself, a heal on one out of a fight, and the
//! dead's avoidance.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::{Map, death};
use crate::game::tests_p3_combat::*;

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_IN_COMBAT))
}

/// Thistle Tea's "environmental and self damage do not establish combat"
/// and "healing and feedback after death do not establish combat":
/// falling, lava and one's own damage put a player in no fight, nor does
/// healing one who is in none (mangos's `SetInCombatWith` wants another,
/// and a healer joins only a fight).
#[tokio::test]
async fn the_world_ones_own_damage_and_a_heal_out_of_a_fight_start_none() {
    let (realm, _a, ga, map) = setup(Class::Priest).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 2.0, START.1, START.2).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            level(m, gb, 60);
            crate::game::map::environment::damage(m, ga, EnvironmentalDamageType::Fall, 10);
            crate::game::map::environment::damage(m, ga, EnvironmentalDamageType::Lava, 10);
            let world = in_combat(m, ga);
            combat::deal_damage(m, ga, ga, 10);
            let own = in_combat(m, ga);
            crate::game::map::spells::heal(m, ga, gb, 2050, 100, false);
            (world, own, in_combat(m, ga), in_combat(m, gb))
        })
        .await
        .unwrap();
    assert_eq!(r, (false, false, false, false), "no fight from any of it");
}

/// Thistle Tea's "dead players cannot open defensive windows": a dead
/// warrior's, hunter's or rogue's dodge, parry or block opens no window.
#[tokio::test]
async fn the_dead_open_no_defensive_window() {
    let mut seen = Vec::new();
    for (race, class) in [
        (Race::Human, Class::Warrior),
        (Race::Dwarf, Class::Hunter),
        (Race::Human, Class::Rogue),
    ] {
        let realm = crate::game::harness::realm_with(data());
        let mut a = crate::game::harness::Client::connect(&realm, "A");
        let ga = a.enter("Alice", race, class).await;
        let map = realm
            .maps
            .get(&realm, crate::game::map::MapKey::world(0));
        let r = map
            .query(move |m| {
                level(m, ga, 60);
                let brute = spawn(m, ga, BRUTE, 60, 3.0);
                death::kill(m, ga);
                [Outcome::Dodge, Outcome::Parry, Outcome::Block].map(|o| {
                    combat::after_outcome(m, brute, ga, o);
                    m.combat(ga)
                        .is_some_and(|c| c.defense_window.is_some() || c.hunter_parry_window.is_some())
                })
            })
            .await
            .unwrap();
        seen.push(r);
    }
    assert_eq!(seen, [[false; 3]; 3]);
}
