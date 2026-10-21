//! Round 11's entity AI rows (docs/parity/matrix.csv): what an aggressive
//! pet notices, and how possessed and formation units move.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::UNIT_FIELD_LEVEL;
use crate::game::map::Map;
use crate::game::map::pets::{self, Kind};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{WOLF, place_in};

/// A guardian that notices at eighteen yards, as the real data's do.
const WATCHER: u32 = 98_500;

fn watcher_data() -> GameData {
    let mut d = crate::game::tests_pets::data();
    let mut t = d.creature_templates[&WOLF].clone();
    t.entry = WATCHER;
    t.detection_range = 18.0;
    d.creature_templates.insert(WATCHER, t);
    d
}

/// A few of the pets' updates, a second apart.
fn think(m: &mut Map, times: u32) {
    for _ in 0..times {
        m.now += Duration::from_secs(1);
        pets::update(m);
    }
}

fn victim(m: &Map, g: Guid) -> Option<Guid> {
    m.combat(g).filter(|c| c.attacking).map(|c| c.victim)
}

/// Thistle Tea's "aggressive pets observe and check sight beyond twenty
/// yards" (ai_environment_test 174), as mangos-classic's
/// `PetAI::MoveInLineOfSight` looks: an aggressive guardian of 60 that
/// notices at 18 yards takes no wolf of its level 25 yards off, takes one
/// of 50 there (its radius a yard more a level below it, 28), but not
/// while it stands five yards above, nor while it is stunned (where every
/// pet looked twenty yards round, whatever the levels, at any height).
#[tokio::test]
async fn an_aggressive_pet_notices_as_far_as_the_levels_say() {
    let (_realm, _h, gh, map, _pet, wolf) = tamed_with(watcher_data()).await;
    let out = map
        .query(move |m| {
            m.remove_creature(wolf);
            let p = m.players[&gh].position();
            let g = pets::testing::spawn(m, gh, WATCHER, Kind::Guardian, 60, p, 0.0);
            pets::state_mut(m, g).unwrap().react = pets::REACT_AGGRESSIVE;
            let far = place_in(m, gh, WOLF, 0x70_0040, 25.0);
            let level = |m: &mut Map, l: u32| {
                m.creature_mut(far)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_LEVEL, l)
            };
            level(m, 60);
            think(m, 2);
            let even = victim(m, g);
            level(m, 50);
            // Up on a ledge, then stunned, then free on the ground.
            m.creature_mut(far).unwrap().position.z += 5.0;
            think(m, 2);
            let above = victim(m, g);
            m.creature_mut(far).unwrap().position.z -= 5.0;
            let stunned = crate::game::map::player::UNIT_FLAG_STUNNED;
            let flags = |m: &mut Map, on: bool| {
                m.creature_mut(far).unwrap().fields.set_flag(
                    crate::game::fields::UNIT_FIELD_FLAGS,
                    stunned,
                    on,
                )
            };
            flags(m, true);
            think(m, 2);
            let held = victim(m, g);
            flags(m, false);
            think(m, 2);
            (even, above, held, victim(m, g), far)
        })
        .await
        .unwrap();
    let (even, above, held, lower, far) = out;
    assert_eq!(even, None, "of its level, 25 yards is beyond its 18");
    assert_eq!(above, None, "five yards above, out of its reach");
    assert_eq!(held, None, "a stunned one is let be");
    assert_eq!(lower, Some(far), "ten levels below it, within its 28");
}
