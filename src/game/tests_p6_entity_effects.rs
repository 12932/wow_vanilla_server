//! Round 6's effect resolver tests, ported from Thistle Tea's
//! `test/game/world/entity/effect_resolver/*_test.exs`
//! (docs/parity/matrix.csv): a charge's swing delay, its refusals and its
//! one spline for all who see it; a spell's flight from its own caster; a
//! pet's kill in a battleground; Alterac Valley's bound spawns; a captured
//! animal's following; who a PvP contact flags.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::combat::{self, Contact, dist};
use crate::game::map::creature::Creature;
use crate::game::map::creature::Walk;
use crate::game::tests_parity_motion as motion;

/// Alice at the start and the wolf 20 yards east, Bob beside her to watch.
async fn charging() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Client,
    Guid,
    crate::game::map::MapHandle,
) {
    let mut d = motion::wolf_world();
    d.creature_spawns.get_mut(&0).unwrap()[0] =
        motion::spawn(80, motion::WOLF, motion::at(20.0, 0.0));
    let (realm, mut a, ga, map) = motion::enter(d).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(motion::START.0, motion::START.1 - 2.0, motion::START.2)
        .await;
    a.sees(motion::wolf()).await;
    b.sees(ga).await;
    (realm, a, b, ga, map)
}

/// A charge that doesn't attack (a helpful one, or one that ends the
/// fight) still puts its caster's swings off by the run, and swings at
/// nothing on arriving; its caster and those who see it are told the one
/// run, the same spline id, path and time (Thistle Tea's "nonattacking
/// charges still carry the swing delay without an arrival attack" and
/// "both owners receive the same movement and spline id broadcast to
/// observers").
#[tokio::test]
async fn a_charge_without_a_swing_still_waits_out_its_run_and_all_see_one_spline() {
    let (_realm, mut a, mut b, ga, map) = charging().await;
    let g = motion::wolf();
    let (attack, swing_in, spline) = map
        .query(move |m| {
            let now = m.now;
            crate::game::map::charge::start(m, ga, g, false);
            let p = &m.players[&ga];
            let c = p.charge.as_ref().expect("charging");
            (
                c.attack,
                [p.combat.next_swing, p.combat.next_offhand]
                    .map(|t| t.map(|t| t.duration_since(now))),
                c.spline_id,
            )
        })
        .await
        .unwrap();
    assert_eq!(attack, None, "no swing on arriving");
    for t in swing_in {
        assert!(
            t.is_some_and(|t| t >= Duration::from_millis(200 + 40 * 14)),
            "both hands put off by the run: {swing_in:?}"
        );
    }
    let mine = motion::next_move(&mut a, ga, |m| m.kind == 0).await;
    let seen = motion::next_move(&mut b, ga, |m| m.kind == 0).await;
    assert_eq!(mine.id, spline);
    assert_eq!(
        (seen.id, seen.duration, &seen.points),
        (mine.id, mine.duration, &mine.points),
        "Bob sees the run Alice is told"
    );
    tokio::time::sleep(Duration::from_millis(u64::from(mine.duration) + 300)).await;
    let (charging, attacking, there) = map
        .query(move |m| {
            let p = &m.players[&ga];
            (
                p.charge.is_some(),
                p.combat.attacking,
                dist(p.position(), motion::at(20.0, 0.0)),
            )
        })
        .await
        .unwrap();
    assert!(!charging && !attacking, "there, and not swinging");
    assert!(there < 6.0, "at the wolf: {there}");
}

/// A charge refuses a dead caster, a rooted one, one on a flight, one
/// aimed at itself or at no one there, and one already within reach: it
/// doesn't start, and its caster stays put (Thistle Tea's "charges reject
/// dead, rooted and taxi casters and targets in another world").
#[tokio::test]
async fn a_charge_refuses_the_dead_the_rooted_the_flying_and_no_target() {
    let (_realm, _a, _b, ga, map) = charging().await;
    let g = motion::wolf();
    let refused = map
        .query(move |m| {
            let mut refused = Vec::new();
            let mut try_it = |m: &mut Map, what: &'static str, target: Guid| {
                let from = m.players[&ga].position();
                crate::game::map::charge::start(m, ga, target, true);
                let p = &m.players[&ga];
                refused.push((what, p.charge.is_none() && dist(p.position(), from) < 0.01));
            };
            motion::apply(m, ga, ga, motion::ROOT);
            try_it(m, "rooted", g);
            crate::game::map::auras::remove_spell(m, ga, motion::ROOT, None);
            let here = m.players[&ga].position();
            let walk = Walk::new(vec![here, motion::at(0.0, 50.0)], 7.0, m.now, 1, true);
            m.players.get_mut(&ga).unwrap().flight = Some(crate::game::map::npc::Flight {
                walk,
                destination: 0,
                mount: 0,
            });
            try_it(m, "flying", g);
            m.players.get_mut(&ga).unwrap().flight = None;
            try_it(m, "itself", ga);
            try_it(m, "no one", guid::unit(motion::WOLF, 999));
            let near = motion::at(2.0, 0.0);
            let wolf_at = m.creature(g).unwrap().position;
            m.creature_mut(g).unwrap().position = near;
            try_it(m, "within reach", g);
            m.creature_mut(g).unwrap().position = wolf_at;
            crate::game::map::death::kill(m, ga);
            try_it(m, "dead", g);
            refused
        })
        .await
        .unwrap();
    for (what, it) in refused {
        assert!(it, "{what}: charged");
    }
}

/// A spell's flight is timed from where its own caster stands, whoever
/// that is (a creature, a totem), at five yards' worth at least, and a
/// target not in the map is reached at once (Thistle Tea's "foreign
/// launch and delivery use the actual caster's position" and "point-blank
/// missiles use the minimum flight distance").
#[tokio::test]
async fn a_spells_flight_is_timed_from_its_own_caster() {
    let (_realm, _a, _b, ga, map) = charging().await;
    let g = motion::wolf();
    let times = map
        .query(move |m| {
            let pos = |dx: f32| motion::at(dx, 0.0);
            m.creature_mut(g).unwrap().position = pos(40.0);
            let far = crate::game::map::spells::flight_ms(m, g, ga, 20.0);
            m.creature_mut(g).unwrap().position = pos(2.0);
            let near = crate::game::map::spells::flight_ms(m, g, ga, 20.0);
            let elsewhere = crate::game::map::spells::flight_ms(m, g, guid::player(9_999), 20.0);
            (far, near, elsewhere)
        })
        .await
        .unwrap();
    assert_eq!(times, (2_000, 250, 0));
}

/// A pet's kill in Warsong Gulch is its master's: his killing blow and
/// honorable kill (Thistle Tea's "captures pet ownership and dead
/// teammates near either their body or ghost"; the dead near by body or
/// corpse are tests_p3_world_bg's).
#[tokio::test]
async fn a_pets_kill_in_a_battleground_is_its_masters() {
    use crate::game::tests_p3_world_bg as bg;
    let realm = harness::realm_with(bg::gulch());
    let mut a = bg::player(&realm, "A", "Alice", Race::Human).await;
    let mut b = bg::player(&realm, "B", "Grom", Race::Orc).await;
    let map = bg::all_in(&realm, &mut [&mut a, &mut b]).await;
    let (alice, grom) = (a.guid, b.guid);
    for c in [&mut a, &mut b] {
        bg::put(&map, c, 1200.0, 1400.0, 340.0).await;
    }
    let scores = map
        .query(move |m| {
            let t = crate::game::data::CreatureTemplate {
                entry: 9_990,
                name: "Wolf".into(),
                faction: 2,
                ..Default::default()
            };
            let at = m.players[&grom].position();
            let mut c = Creature::new(guid::unit(9_990, 1), &t, 1, at, 0.0);
            c.owner = Some(grom);
            let pet = m.add_creature(c);
            combat::kill(m, pet, alice);
            [grom, alice].map(|g| {
                let s = bg::score(m, g);
                (s.killing_blows, s.honorable_kills, s.deaths)
            })
        })
        .await
        .unwrap();
    assert_eq!(scores, [(1, 1, 0), (0, 0, 1)], "Grom, Alice");
}

/// In Alterac Valley, a general's spawn killed by a player's pet counts for
/// the pet's master, and ends the battle; a copy of the general that is
/// no spawn of the valley's (a summon) counts for nothing (Thistle Tea's
/// "captures an objective incarnation and attributes a pet kill to its
/// admitted owner" and "ordinary world kills and summons bypass
/// battleground lookups").
#[tokio::test]
async fn alterac_counts_a_pets_kill_of_a_bound_spawn_for_its_master_and_no_summon() {
    use crate::game::tests_pvp as pvp;
    let realm = harness::realm_with(pvp::alterac());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = pvp::both_into(
        &realm,
        &mut a,
        &mut b,
        wow_world_messages::vanilla::Map::AlteracValley,
    )
    .await;
    let alice = a.guid;
    let general = map
        .query(|m| {
            m.creatures
                .iter()
                .find(|(_, c)| c.entry == 11946 && c.spawn_guid.is_some())
                .map(|(_, c)| (c.guid, c.position))
        })
        .await
        .flatten()
        .expect("Drek'Thar is in the valley");
    let summoned = map
        .query(move |m| {
            let t = m.realm.data.creature_template(11946).cloned().unwrap();
            let copy = m.add_creature(Creature::new(
                guid::unit(11946, 99_999),
                &t,
                60,
                general.1,
                0.0,
            ));
            combat::kill(m, alice, copy);
            let b = m.battleground.as_ref().unwrap();
            (b.playing(), b.scores[&alice].objectives[5])
        })
        .await
        .unwrap();
    assert_eq!(summoned, (true, 0), "a summoned Drek'Thar is no general");
    map.query(move |m| {
        let t = crate::game::data::CreatureTemplate {
            entry: 9_990,
            name: "Wolf".into(),
            faction: 1,
            ..Default::default()
        };
        let mut c = Creature::new(guid::unit(9_990, 1), &t, 60, general.1, 0.0);
        c.owner = Some(alice);
        let pet = m.add_creature(c);
        combat::kill(m, pet, general.0);
    })
    .await;
    assert_eq!(pvp::winner(&mut a).await, BattlegroundWinner::Alliance);
    let leaders = map
        .query(move |m| m.battleground.as_ref().unwrap().scores[&alice].objectives[5])
        .await
        .unwrap();
    assert_eq!(leaders, 1, "the pet's kill is Alice's");
}

/// A captured animal that has stopped following is set to follow its
/// captor again at the aura's look, unless it is fighting or going home
/// (Thistle Tea's "restores lost following without interrupting combat,
/// evade or another player's request"; the aura is its captor's, so no
/// one else's look moves it).
#[tokio::test]
async fn a_captured_animal_follows_again_unless_it_fights_or_evades() {
    use crate::game::tests_p4_items_quests2 as q;
    let (_realm, _a, g, map) = q::captured().await;
    let follows = |m: &Map| {
        crate::game::map::event_ai::scripted(m, q::wolf())
            .and_then(|s| s.follow)
            .map(|(whom, _, _)| whom)
    };
    for busy in ["fighting", "evading"] {
        map.query(move |m| {
            let c = m.creature_mut(q::wolf()).unwrap();
            match busy {
                "fighting" => c.combat.attacking = true,
                _ => c.evading = true,
            }
            q::look_now(m, g);
        })
        .await;
        // Until it has looked: its next look two seconds off.
        for _ in 0..200 {
            let next = map.query(move |m| q::next_look(m, g)).await.flatten();
            if next.is_some_and(|t| t > Duration::from_millis(1000)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let following = map.query(move |m| follows(m)).await.flatten();
        assert_eq!(following, None, "{busy}: not set following");
        map.query(move |m| {
            let c = m.creature_mut(q::wolf()).unwrap();
            c.combat.attacking = false;
            c.evading = false;
        })
        .await;
    }
    map.query(move |m| q::look_now(m, g)).await;
    for _ in 0..200 {
        if map.query(move |m| follows(m)).await.flatten() == Some(g) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never set following again");
}

/// Touching an unflagged creature, or helping oneself, flags no one,
/// flagged or not: a blow on a wolf, a buff on oneself (Thistle Tea's
/// "ordinary PvE and self assistance produce no PvP notifications"; a
/// flagged creature's are tests_p3_core_pvp's).
#[tokio::test]
async fn attacking_a_plain_creature_or_helping_oneself_flags_no_one() {
    let (_realm, _a, _b, ga, map) = charging().await;
    let g = motion::wolf();
    let flagged = map
        .query(move |m| {
            let flagged = |m: &Map| {
                m.players[&ga]
                    .fields
                    .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP)
            };
            combat::engage(m, ga, g);
            combat::pvp_contact(m, ga, g, Contact::Attack);
            let after_wolf = flagged(m);
            m.players.get_mut(&ga).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                true,
            );
            combat::pvp_contact(m, ga, ga, Contact::Assist);
            (after_wolf, m.players[&ga].combat.last_pvp.is_some())
        })
        .await
        .unwrap();
    assert_eq!(flagged, (false, false), "the wolf, and herself");
}
