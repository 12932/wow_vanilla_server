//! Round 8's creature AI tests of pets, ported from Thistle Tea's
//! `test/game/core/ai/bt/pet/targets_test.exs` (docs/parity/matrix.csv):
//! whom a pet's autocast for a friend goes on: its master, its master's
//! party (a raid's subgroup) here, alive, near and in sight, then itself;
//! and a spell for itself or its master alone only on them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::map::{Map, auras, combat, pets};
use crate::game::social::groups;
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{WOLF, effect, enter, place, spell};

/// Fire Shield as Thistle Tea knows it (its visual, family and flags), on
/// a friend; one on the pet's master alone; one on the pet itself.
const SHIELD: u32 = 95_830;
const ON_MASTER: u32 = 95_831;
const ON_SELF: u32 = 95_832;

fn world() -> GameData {
    let mut d = crate::game::tests_pets::data();
    d.spells.ranges.insert(4, (0.0, 30.0));
    for (id, target) in [(SHIELD, 21), (ON_MASTER, 27), (ON_SELF, 1)] {
        let mut s = spell(
            id,
            "Shield",
            [
                SpellEffect {
                    aura: 4,
                    ..effect(6, 1, target)
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        );
        s.range_index = 4;
        if id == SHIELD {
            s.spell_visual = 289;
            s.family_name = 5;
            s.family_flags = 0x0080_0000;
        }
        d.spells.spells.insert(id, s);
    }
    d
}

/// The pet thinks a few times, free to cast.
fn think(m: &mut Map, pet: Guid) {
    for _ in 0..3 {
        m.now += Duration::from_secs(2);
        if let Some(c) = m.caster_mut(pet) {
            c.casting = None;
            c.gcd_until = None;
        }
        // Its next thought now, whatever the map's clock said before.
        let now = m.now;
        pets::state_mut(m, pet).unwrap().next_think = now;
        pets::update(m);
    }
}

fn has(m: &Map, who: Guid, spell: u32) -> bool {
    auras::auras(m, who).is_some_and(|a| a.has_spell(spell))
}

/// `who` put `east` yards east of where they stand.
fn shift(m: &mut Map, who: Guid, east: f32) {
    let p = m.players.get_mut(&who).unwrap();
    p.movement.position.x += east;
}

/// Thistle Tea's "Fire Shield automatically protects the owner instead of
/// wasting a self cast", "shares buffs with the current raid subgroup",
/// "skips obstructed, dead, distant, absent and foreign-world allies" and
/// "skips the owner's existing aura and shields another attacked party
/// member" (mangos's `PetAI::UpdateAllies`): a pet out of its own fight
/// shields its master under attack; its master shielded, a party member
/// under attack; not one gone to another subgroup of a raid, nor one
/// dead, out of its range or behind a wall.
#[tokio::test]
async fn a_pets_shield_goes_on_its_master_then_its_masters_party_near_alive_and_in_sight() {
    let (realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let (_b, gb, _) = enter(&realm, "B", "Bob", Race::Dwarf, Class::Warrior).await;
    let other = place(&map, gh, WOLF, 0x70_0003, 16.0).await;
    let lo = realm.registry.online(gh).unwrap();
    let bo = realm.registry.online(gb).unwrap();
    assert!(groups::join_directly(&realm, &lo, &bo));
    let shielded = map
        .query(move |m| {
            pets::state_mut(m, pet).unwrap().autocast = vec![SHIELD];
            // No one attacked: no one.
            think(m, pet);
            let calm = (has(m, gh, SHIELD), has(m, gb, SHIELD), has(m, pet, SHIELD));
            // Its master attacked.
            combat::start_attack(m, wolf, gh);
            combat::engage(m, wolf, gh);
            think(m, pet);
            let master = (has(m, gh, SHIELD), has(m, gb, SHIELD));
            // Bob attacked: his shield, his master's already on.
            combat::start_attack(m, other, gb);
            combat::engage(m, other, gb);
            think(m, pet);
            (calm, master, has(m, gb, SHIELD))
        })
        .await
        .unwrap();
    assert_eq!(shielded.0, (false, false, false), "no one attacked");
    assert_eq!(shielded.1, (true, false), "its master first");
    assert!(shielded.2, "then Bob, his master's on");
    // Bob, unshielded, out of reach in four ways.
    let again = |f: fn(&mut Map, Guid)| {
        let map = map.clone();
        async move {
            map.query(move |m| {
                auras::remove_spell(m, gb, SHIELD, None);
                f(m, gb);
                // Still attacked.
                if m.unit_view(gb).is_some_and(|u| u.alive) {
                    combat::start_attack(m, other, gb);
                    combat::engage(m, other, gb);
                }
                think(m, pet);
                has(m, gb, SHIELD)
            })
            .await
            .unwrap()
        }
    };
    let far = again(|m, gb| shift(m, gb, 100.0)).await;
    let walled = again(|m, gb| {
        shift(m, gb, -100.0);
        let b = m.position_of(gb).unwrap();
        let p = |x: Vector3d, dx: f32, dy: f32| Vector3d {
            x: x.x + dx,
            y: x.y + dy,
            z: x.z,
        };
        // A wall all about him.
        for (dx, dy, ex, ey) in [
            (-1.0, -1.0, 1.0, -1.0),
            (1.0, -1.0, 1.0, 1.0),
            (1.0, 1.0, -1.0, 1.0),
            (-1.0, 1.0, -1.0, -1.0),
        ] {
            m.spells.walls.push((p(b, dx, dy), p(b, ex, ey)));
        }
    })
    .await;
    // A raid, Bob in another subgroup: not his; back in his master's, his.
    m_clear_walls(&map).await;
    groups::convert_to_raid(&realm, &lo);
    groups::change_subgroup(&realm, &lo, "Bob", 1);
    let raid = again(|_, _| {}).await;
    groups::change_subgroup(&realm, &lo, "Bob", 0);
    let back = again(|_, _| {}).await;
    let dead = again(crate::game::map::death::kill).await;
    assert_eq!((far, walled), (false, false), "far, walled");
    assert_eq!((raid, back), (false, true), "his master's subgroup's only");
    assert!(!dead, "not dead");
}

async fn m_clear_walls(map: &crate::game::map::MapHandle) {
    map.query(|m| m.spells.walls.clear()).await.unwrap();
}

/// Thistle Tea's `candidates/3` "self, owner and area spells only consider
/// their actual recipients": a pet's spell on its master alone goes on its
/// master, not itself; one on itself, on itself, not its master.
#[tokio::test]
async fn a_pets_spell_for_its_master_or_itself_goes_on_them_alone() {
    let (_realm, _h, gh, map, pet, _wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            pets::state_mut(m, pet).unwrap().autocast = vec![ON_MASTER, ON_SELF];
            think(m, pet);
            (
                (has(m, gh, ON_MASTER), has(m, pet, ON_MASTER)),
                (has(m, gh, ON_SELF), has(m, pet, ON_SELF)),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, ((true, false), (false, true)));
}
