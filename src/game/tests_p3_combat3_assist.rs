//! Phase 3's combat tests, part three of this stage (Thistle Tea's
//! `world/combat/call_for_help_test.exs`): whom a creature drawn into a
//! fight calls, who comes when the call is due, who runs instead, and
//! whom it calls as it fights away from its spawn.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::assistance::{self, ASSIST_DELAY};
use crate::game::map::creature::{Creature, MOVEMENT_RANDOM};
use crate::game::tests_p3_combat::{START, faction, template};

/// The Defias's template, and one of another of their friends; a beast's
/// that answers no calls; a trogg's that runs from them; a Defias friendly
/// to the humans.
const DEFIAS: u32 = 17;
const DEFIAS_ALLY: u32 = 92;
const WILD: u32 = 32;
const TROGG: u32 = 36;
const FRIEND_OF_MAN: u32 = 117;

fn of(f: u32) -> u32 {
    3900 + f
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut add = |id: u32, flags: u32, ours: u32, friend_mask: u32, game_faction: u32| {
        let mut t = faction(id, ours, 0x1);
        t.flags = flags;
        t.faction = game_faction;
        t.friend_mask = friend_mask;
        t.friends = [15, 0, 0, 0];
        d.faction_templates.insert(id, t);
        let mut c = template(of(id), 7);
        c.faction = id;
        c.call_for_help_range = 5.0;
        d.creature_templates.insert(of(id), c);
    };
    add(DEFIAS, 1, 0x8, 0, 15);
    add(DEFIAS_ALLY, 1, 0x8, 0, 88);
    add(WILD, 16, 0, 0, 29);
    add(TROGG, 0x401, 0x8, 0, 45);
    add(FRIEND_OF_MAN, 1, 0x8, 0x1, 115);
    d
}

/// A creature of `entry` at `(x, y)` yards from the caller's spot, with its
/// home there.
fn put(m: &mut Map, entry: u32, x: f32, y: f32) -> Guid {
    let data = m.realm.data.clone();
    let t = data.creature_template(entry).cloned().expect("a template");
    let at = Vector3d {
        x: START.0 + 40.0 + x,
        y: START.1 + y,
        z: START.2,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = Creature::new(guid::unit(entry, counter | 0x80_0000), &t, 20, at, 0.0).with_base();
    m.add_creature(c)
}

async fn scene() -> (Client, Guid, crate::game::map::MapHandle) {
    let realm = crate::game::harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.registry.online(g).unwrap().map();
    (a, g, map)
}

/// Those `caller`'s call against `enemy` took in.
fn called(m: &Map, caller: Guid) -> Vec<Guid> {
    m.creature(caller)
        .map(|c| {
            c.engagement
                .assist
                .iter()
                .flat_map(|a| a.helpers.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn fighting(m: &Map, g: Guid, enemy: Guid) -> bool {
    m.creature(g)
        .is_some_and(|c| c.combat.attacking && c.combat.victim == enemy)
}

/// The calls due now.
fn deliver(m: &mut Map, caller: Guid) {
    let now = m.now;
    if let Some(c) = m.creature_mut(caller) {
        for a in c.engagement.assist.iter_mut() {
            a.due = now;
        }
    }
    assistance::update(m);
}

#[tokio::test]
async fn a_creature_drawn_in_calls_its_own_template_within_ten_yards_and_no_other() {
    let (_a, g, map) = scene().await;
    let got = map
        .query(move |m| {
            let caller = put(m, of(DEFIAS), 0.0, 0.0);
            let kin = put(m, of(DEFIAS), 5.0, 0.0);
            let far = put(m, of(DEFIAS), 15.0, 0.0);
            // Friendly, but of another template; deaf to calls; running
            // from them.
            let ally = put(m, of(DEFIAS_ALLY), 0.0, 5.0);
            let beast = put(m, of(WILD), 0.0, -5.0);
            let trogg = put(m, of(TROGG), -5.0, 0.0);
            // Dead, and a pet.
            let dead = put(m, of(DEFIAS), 3.0, 3.0);
            crate::game::map::combat::deal_damage(m, g, dead, 1_000_000);
            crate::game::map::ai::forget(m, g);
            let pet = put(m, of(DEFIAS), -3.0, 3.0);
            m.creature_mut(pet).unwrap().owner = Some(g);
            crate::game::map::ai::attacked(m, caller, g);
            let called = called(m, caller);
            (
                called.contains(&kin),
                [far, ally, beast, trogg, dead, pet].map(|h| called.contains(&h)),
                called.len(),
            )
        })
        .await
        .unwrap();
    assert!(got.0, "its kind five yards off");
    assert_eq!(
        got.1, [false; 6],
        "beyond ten yards, other templates, the dead and pets"
    );
    assert_eq!(got.2, 1);
}

#[tokio::test]
async fn a_creature_friendly_to_the_enemy_is_not_called() {
    let (_a, g, map) = scene().await;
    let called = map
        .query(move |m| {
            let caller = put(m, of(FRIEND_OF_MAN), 0.0, 0.0);
            put(m, of(FRIEND_OF_MAN), 5.0, 0.0);
            crate::game::map::ai::attacked(m, caller, g);
            called(m, caller).len()
        })
        .await
        .unwrap();
    assert_eq!(called, 0);
}

#[tokio::test]
async fn the_called_come_when_it_is_due_wherever_the_caller_went_if_still_free() {
    let (_a, g, map) = scene().await;
    let got = map
        .query(move |m| {
            // Called, then the caller runs on 24 yards: the one called
            // comes, the one now near doesn't.
            let caller = put(m, of(DEFIAS), 0.0, 0.0);
            let kin = put(m, of(DEFIAS), 5.0, 0.0);
            let later = put(m, of(DEFIAS), 25.0, 0.0);
            crate::game::map::ai::attacked(m, caller, g);
            let due = m
                .creature(caller)
                .and_then(|c| c.engagement.assist.first().map(|a| a.due - m.now));
            let to = m.creature(later).unwrap().position;
            let c = m.creature_mut(caller).unwrap();
            (c.position, c.home) = (to, to);
            deliver(m, caller);
            let first = (fighting(m, kin, g), fighting(m, later, g));
            // Called, but busy by the time it is due; another come since
            // isn't taken in its place.
            let caller = put(m, of(DEFIAS), 0.0, 30.0);
            let busy = put(m, of(DEFIAS), 5.0, 30.0);
            crate::game::map::ai::attacked(m, caller, g);
            m.creature_mut(busy).unwrap().evading = true;
            let new = put(m, of(DEFIAS), 4.0, 30.0);
            deliver(m, caller);
            let second = (fighting(m, busy, g), fighting(m, new, g));
            (due, first, second)
        })
        .await
        .unwrap();
    assert_eq!(got.0, Some(ASSIST_DELAY), "a second and a half on");
    assert_eq!(got.1, (true, false));
    assert_eq!(got.2, (false, false));
}

#[tokio::test]
async fn fighting_away_from_its_spawn_it_calls_its_friends_within_its_range_and_trogs_run() {
    let (_a, g, map) = scene().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // Its range is five: a friend of another template four yards
            // off comes, one eight off doesn't.
            for (yards, range) in [(4.0, 5.0), (8.0, 5.0), (2.0, 0.0)] {
                let caller = put(m, of(DEFIAS), 0.0, 60.0 + yards * 10.0);
                let friend = put(m, of(DEFIAS_ALLY), yards, 60.0 + yards * 10.0);
                if range == 0.0 {
                    crate::game::map::event_ai::scripted_mut(m, caller)
                        .unwrap()
                        .call_for_help_range = Some(0.0);
                }
                crate::game::map::ai::attacked(m, caller, g);
                let c = m.creature_mut(caller).unwrap();
                c.engagement.assist.clear();
                c.home.x += 20.0;
                c.engagement.pulse_at = None;
                assistance::update(m);
                out.push(fighting(m, friend, g));
            }
            // Within its range of five, a trogg wandering four yards off
            // backs away and doesn't fight; one standing still, or eight
            // yards off, stays put.
            let caller = put(m, of(DEFIAS), 0.0, -60.0);
            let wanderer = put(m, of(TROGG), 4.0, -60.0);
            m.creature_mut(wanderer).unwrap().movement_type = MOVEMENT_RANDOM;
            let still = put(m, of(TROGG), -4.0, -60.0);
            let far = put(m, of(TROGG), 8.0, -60.0);
            m.creature_mut(far).unwrap().movement_type = MOVEMENT_RANDOM;
            // The player beside the caller.
            let at = m.creature(caller).unwrap().position;
            crate::game::map::movement::teleport_within(m, g, at, 0.0);
            assistance::call_friends(m, caller, g, 5.0);
            let moved = |m: &Map, t: Guid| {
                crate::game::map::event_ai::scripted(m, t)
                    .is_some_and(|s| s.scripted_move.is_some())
            };
            out.push(moved(m, wanderer) && !fighting(m, wanderer, g));
            out.push(moved(m, still) || moved(m, far));
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [true, false, false, true, false]);
}

#[tokio::test]
async fn only_a_free_creature_helps_and_a_fleeing_one_seeks_the_nearest_within_thirty_yards() {
    let (_a, g, map) = scene().await;
    let got = map
        .query(move |m| {
            let idle = put(m, of(DEFIAS), 0.0, 0.0);
            let free = assistance::available(m, idle);
            // Busy, dead, summoned, charmed, a pet, never assisting,
            // going home, invisible.
            let mut out = Vec::new();
            for case in 0..8 {
                let h = put(m, of(DEFIAS), 0.0, 3.0 * case as f32 + 3.0);
                match case {
                    0 => crate::game::map::ai::attacked(m, h, g),
                    1 => {
                        crate::game::map::combat::deal_damage(m, g, h, 1_000_000);
                        crate::game::map::ai::forget(m, g);
                    }
                    2 => m
                        .unit_fields_mut(h)
                        .unwrap()
                        .set_guid(crate::game::fields::UNIT_FIELD_SUMMONEDBY, g),
                    3 => m
                        .unit_fields_mut(h)
                        .unwrap()
                        .set_guid(crate::game::fields::UNIT_FIELD_CHARMEDBY, g),
                    4 => m.creature_mut(h).unwrap().owner = Some(g),
                    5 => m.creature_mut(h).unwrap().evading = true,
                    6 => crate::game::tests_p3_combat::aura_of(
                        m,
                        h,
                        95_480,
                        crate::game::map::auras::AURA_MOD_INVISIBILITY,
                        0,
                        1,
                    ),
                    _ => {}
                }
                out.push(assistance::available(m, h));
            }
            // Not selectable, pacified, stunned, not attackable.
            for flag in [0x2, 0x2_0000, 0x4_0000, 0x0200_0000] {
                let h = put(m, of(DEFIAS), 3.0, 0.0);
                m.unit_fields_mut(h)
                    .unwrap()
                    .set_u32(crate::game::fields::UNIT_FIELD_FLAGS, flag);
                out.push(assistance::available(m, h));
            }
            (free, out)
        })
        .await
        .unwrap();
    assert!(got.0);
    assert_eq!(
        got.1,
        [
            false, false, false, false, false, false, false, true, false, false, false, false
        ],
        "only the last of the eight cases, an idle one, is free"
    );
    let got = map
        .query(move |m| {
            let fleer = put(m, of(DEFIAS), 0.0, -80.0);
            crate::game::map::ai::attacked(m, fleer, g);
            let other = put(m, of(DEFIAS_ALLY), 15.0, -80.0);
            let near = put(m, of(DEFIAS), 21.0, -80.0);
            let farther = put(m, of(DEFIAS), 28.0, -80.0);
            let first = assistance::nearest_helper(m, fleer);
            m.creature_mut(near).unwrap().evading = true;
            let second = assistance::nearest_helper(m, fleer);
            m.creature_mut(farther).unwrap().evading = true;
            put(m, of(DEFIAS), 31.0, -80.0);
            let none = assistance::nearest_helper(m, fleer);
            (first == Some(near), second == Some(farther), none, other)
        })
        .await
        .unwrap();
    assert!(got.0, "the nearest of its template");
    assert!(got.1, "the next when it is busy");
    assert_eq!(got.2, None, "none past thirty yards");
}
