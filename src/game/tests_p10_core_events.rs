//! Round 10, against Thistle Tea's `game_event/fireworks_launch_test.exs`:
//! a cheer speaker sends its fireworks up over the city nearest it on its
//! map (`cheer_speakers::sites`), none away from the cities, and only the
//! cheer speaker (180749) runs a show.

use wow_world_messages::vanilla::Vector3d;

use crate::game::cheer_speakers::{SPEAKER, sites};

fn at(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

#[test]
fn a_speaker_launches_over_the_city_nearest_it_on_its_map_and_none_away_from_them() {
    let stormwind = sites(0, at(-8862.0, 654.0, 96.0));
    let booty_bay = sites(0, at(-14_307.0, 508.0, 9.0));
    let orgrimmar = sites(1, at(1503.0, -4409.0, 22.0));
    assert_eq!(stormwind.len(), 117);
    assert!(booty_bay.contains(&[-14_358.03, 515.058, 34.2664, 3.68265]));
    assert!(!stormwind.iter().any(|s| booty_bay.contains(s)));
    assert!(!orgrimmar.is_empty());
    assert!(orgrimmar.iter().all(|s| s[0] > 1000.0));
    // Goldshire, and Stormwind's place on Kalimdor.
    assert!(sites(0, at(-9464.0, 62.0, 56.0)).is_empty());
    assert!(sites(1, at(-8862.0, 654.0, 96.0)).is_empty());
}

#[test]
fn the_cheer_speaker_is_180749() {
    assert_eq!(SPEAKER, 180_749);
    assert_ne!(SPEAKER, 180_754);
}

mod camps {
    //! Against Thistle Tea's `scourge_invasion/camp_test.exs`, on a map: a
    //! summoning circle raises its Necrotic Shard, calls minions of its
    //! camp's type out of the nearest free finders within 60 yards, each
    //! roaming a yard about its finder; the shard broken, a damaged one
    //! rises with four cultists about it, each that falls striking it; the
    //! damaged one fallen, the cultists leave and the circle and its
    //! campfire go.

    use std::time::Duration;

    use wow_world_messages::Guid;
    use wow_world_messages::vanilla::*;

    use crate::game::data::GameData;
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    use crate::game::fields::*;
    use crate::game::invasion::{DAMAGED_SHARD, NECROTIC_SHARD};
    use crate::game::map::necrotic_camps::{self as camps, CIRCLE, CULTIST, FINDER, Kind, Stage};
    use crate::game::map::{Map, combat, creature, objects};
    use crate::game::tests_p3_combat::{data, setup_with, spawn, template};
    use crate::game::tests_p3_entity_objects::object;

    const CAMPFIRE: u32 = 181_173;

    fn world() -> GameData {
        let mut d = data();
        for (entry, name) in [(CIRCLE, "Circle"), (CAMPFIRE, "Campfire")] {
            d.gameobject_templates
                .insert(entry, object(entry, 5, name, &[]));
        }
        let minions = Kind::ALL
            .iter()
            .flat_map(|k| k.minions().into_iter().chain(k.rares()));
        for entry in [NECROTIC_SHARD, DAMAGED_SHARD, CULTIST, FINDER]
            .into_iter()
            .chain(minions)
        {
            let mut t = template(entry, 6);
            t.faction = crate::game::tests_p3_combat::PACK;
            d.creature_templates.insert(entry, t);
        }
        for (id, effect) in [(28_234u32, 0u32), (28_132, 0), (28_041, 2)] {
            let mut s = SpellTemplate {
                id,
                name: format!("Spell {id}"),
                equipped_item_class: -1,
                ..Default::default()
            };
            s.effects[0] = SpellEffect {
                effect,
                base_points: if effect == 2 { 99 } else { 0 },
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            };
            d.spells.spells.insert(id, s);
        }
        d
    }

    fn of(m: &Map, entries: &[u32]) -> Vec<Guid> {
        m.creatures
            .iter()
            .filter(|(_, c)| entries.contains(&c.entry) && c.alive())
            .map(|(_, c)| c.guid)
            .collect()
    }

    fn put(m: &mut Map, g: Guid, entry: u32, yards: f32) -> Guid {
        let p = &m.players[&g];
        let (pos, o) = (p.position(), p.movement.orientation);
        let at = Vector3d {
            x: pos.x + yards * o.cos(),
            y: pos.y + yards * o.sin(),
            z: pos.z,
        };
        objects::summon(m, None, entry, at, o, 3_600_000, 0, 0, None).unwrap()
    }

    #[tokio::test]
    async fn a_camp_raises_its_shard_calls_its_minions_and_falls_with_its_damaged_shard() {
        let (_realm, _a, g, map) = setup_with(world(), Class::Warrior).await;
        let r = map
            .query(move |m| {
                m.spells.rolls = Some(false);
                let circle = put(m, g, CIRCLE, 0.0);
                let campfire = put(m, g, CAMPFIRE, 2.0);
                // Three finders within reach, one beyond.
                let finders: Vec<(Guid, Vector3d)> = [10.0, 20.0, 30.0, 70.0]
                    .into_iter()
                    .map(|yards| {
                        let f = spawn(m, g, FINDER, 20, yards);
                        (f, m.creature(f).unwrap().position)
                    })
                    .collect();
                let kind = m.camps[&circle].kind;
                camps::update(m);
                let shards = of(m, &[NECROTIC_SHARD]);
                let circle_at = m.gameobject(circle).unwrap().position;
                let shard_at = m.creature(shards[0]).unwrap().position;
                // Its calls, every five seconds, till every finder in reach
                // rests or is stood on.
                let mut calls = Vec::new();
                for _ in 0..4 {
                    let now = m.now;
                    m.camps.get_mut(&circle).unwrap().upkeep_at = now;
                    camps::update(m);
                    calls.push(m.camps[&circle].minions.len());
                }
                let minions: Vec<_> = m.camps[&circle]
                    .minions
                    .iter()
                    .map(|x| {
                        let c = m.creature(*x).unwrap();
                        (c.entry, c.position, c.movement_type, c.wander_distance)
                    })
                    .collect();
                // The shard broken: a damaged one and four cultists.
                combat::kill(m, g, shards[0]);
                let stage = m.camps[&circle].stage;
                let damaged = of(m, &[DAMAGED_SHARD]);
                let cultists = of(m, &[CULTIST]);
                let ring: Vec<f32> = cultists
                    .iter()
                    .map(|c| combat::dist(m.creature(*c).unwrap().position, circle_at))
                    .collect();
                let health =
                    |m: &Map, c: Guid| m.unit_fields(c).unwrap().get_u32(UNIT_FIELD_HEALTH);
                let whole = health(m, damaged[0]);
                combat::kill(m, g, cultists[0]);
                let struck = health(m, damaged[0]);
                // The damaged one fallen: the camp is lost.
                combat::kill(m, g, damaged[0]);
                let left = of(m, &[CULTIST]).len();
                (
                    kind,
                    shards.len(),
                    combat::dist(shard_at, circle_at),
                    calls,
                    minions,
                    finders,
                    (stage, damaged.len(), cultists.len(), ring),
                    (whole, struck),
                    (
                        left,
                        m.gameobject(circle).is_none(),
                        m.gameobject(campfire).is_none(),
                        m.camps.is_empty(),
                    ),
                )
            })
            .await
            .unwrap();
        let (kind, shards, shard_off, calls, minions, finders, broken, struck, fallen) = r;
        assert_eq!(shards, 1);
        assert!(shard_off < 0.1, "{shard_off}");
        // One to three a call, never beyond the three finders in reach.
        assert!((1..=3).contains(&calls[0]), "{calls:?}");
        assert_eq!(*calls.last().unwrap(), 3, "{calls:?}");
        for (entry, at, movement, wander) in &minions {
            let pair = kind.minions().into_iter().chain(kind.rares());
            assert!(pair.into_iter().any(|e| e == *entry), "{entry}");
            let on = finders[..3]
                .iter()
                .filter(|(_, f)| combat::dist(*f, *at) < 0.1)
                .count();
            assert_eq!(on, 1, "a minion on its own finder");
            assert_eq!((*movement, *wander), (creature::MOVEMENT_RANDOM, 1.0));
        }
        let (stage, damaged, cultists, ring) = broken;
        assert_eq!((stage, damaged, cultists), (Stage::Damaged, 1, 4));
        for d in ring {
            assert!((6.7..7.0).contains(&d), "{d}");
        }
        assert_eq!(struck.0 - struck.1, 100, "{struck:?}");
        assert_eq!(fallen, (0, true, true, true));
        let _ = Duration::ZERO;
    }
}
