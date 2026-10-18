//! Phase 3's PvP flag and honor, Thistle Tea's `PvpTest` and `HonorTest`
//! ported: the flag's five minutes once turned off, who is flagged by
//! touching whom, the land that flags (an enemy capital, a PvP realm's
//! land, a battleground, not one flown over), an arena's free-for-all, and
//! the honor ledger's bonus, dishonorable and zero awards and its weekly
//! settling.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::honor::{Award, AwardKind, Ledger};
use crate::game::map::combat::{Contact, pvp_contact};
use crate::game::map::player::{PLAYER_FLAGS_FFA_PVP, UNIT_FLAG_PVP};
use crate::game::map::pvp::{self, PLAYER_FLAGS_CONTESTED_PVP, PVP_LINGER};
use crate::game::map::{Map, MapKey};
use crate::game::registry::Team;

const STORMWIND: u32 = 1519;
const ORGRIMMAR: u32 = 1637;
const ELWYNN: u32 = 12;
const BARRENS: u32 = 17;
const STRANGLETHORN: u32 = 33;
const ARENA: u32 = 2177;
const TIGER: u32 = 3190;

fn data() -> GameData {
    let mut d = GameData::empty();
    for (id, team, flags) in [
        (STORMWIND, 2, AREA_FLAG_CAPITAL),
        (ORGRIMMAR, 4, AREA_FLAG_CAPITAL),
        (ELWYNN, 2, 0),
        (BARRENS, 4, 0),
        (STRANGLETHORN, 0, 0),
        (ARENA, 0, AREA_FLAG_ARENA),
    ] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                flags,
                team,
                name: format!("Area {id}"),
                ..Default::default()
            },
        );
    }
    d.creature_templates.insert(
        TIGER,
        CreatureTemplate {
            entry: TIGER,
            name: "Tiger".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 5,
            level_max: 5,
            faction: 7,
            unit_class: 1,
            health_max: 500,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 1,
            entries: vec![TIGER],
            map: 0,
            x: -8949.95 + 3.0,
            y: -132.493,
            z: 83.5312,
            ..Default::default()
        }],
    );
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

async fn player(realm: &Arc<Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    c.enter(name, Race::Human, Class::Warrior).await;
    c
}

fn flagged(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP))
}

/// Thistle Tea's `toggle/3`: turned off, the flag lingers five minutes;
/// turned off again, the five minutes don't start over; on again and
/// off, a full five minutes from then.
#[tokio::test]
async fn the_flag_lingers_five_minutes_once_turned_off() {
    let realm = harness::realm_with(data());
    let alice = player(&realm, "Alice").await;
    let g = alice.guid;
    let (first, again) = on_map(&realm, move |m| {
        pvp::toggle(m, g, Some(true));
        pvp::toggle(m, g, Some(false));
        let first = m.players[&g].pvp.until;
        m.now += Duration::from_secs(200);
        pvp::toggle(m, g, Some(false));
        (first, m.players[&g].pvp.until)
    })
    .await;
    assert_eq!(first, again, "not started over");
    let (start, refreshed) = on_map(&realm, move |m| {
        m.now += Duration::from_secs(50);
        pvp::toggle(m, g, Some(true));
        pvp::toggle(m, g, Some(false));
        (m.now, m.players[&g].pvp.until)
    })
    .await;
    assert_eq!(refreshed, Some(start + PVP_LINGER), "a full five minutes");
}

/// Thistle Tea's `contact/4`: attacking a flagged player flags and
/// contests; being attacked by one flags without contesting; helping one
/// at rest flags without a fight; helping someone unflagged in a creature
/// fight doesn't; duellists and free-for-all opponents flag no one; a
/// flagged creature flags its attacker, and its helper only in a fight,
/// and no one it attacks.
#[tokio::test]
async fn touching_the_flagged_flags_you_by_how() {
    let realm = harness::realm_with(data());
    let clients = [
        player(&realm, "Alice").await,
        player(&realm, "Bob").await,
        player(&realm, "Cid").await,
    ];
    let (a, b, c) = (clients[0].guid, clients[1].guid, clients[2].guid);
    let tiger = crate::game::guid::unit(TIGER, 1);
    let reset = move |m: &mut Map| {
        for g in [a, b, c] {
            let p = m.players.get_mut(&g).unwrap();
            p.pvp = Default::default();
            p.combat.last_pvp = None;
            p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, false);
            p.fields
                .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP, false);
        }
    };
    let flag = move |m: &mut Map, g: Guid| {
        m.unit_fields_mut(g)
            .unwrap()
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
    };
    let contested = |m: &Map, g: Guid| {
        m.players[&g]
            .fields
            .has_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP)
    };
    let seen = on_map(&realm, move |m| {
        let mut seen = Vec::new();
        // Alice strikes flagged Bob: flagged, contested; Bob, struck,
        // flagged and not contested.
        reset(m);
        flag(m, b);
        crate::game::map::combat::deal_damage(m, a, b, 1);
        seen.push((flagged(m, a), contested(m, a)));
        reset(m);
        flag(m, a);
        pvp_contact(m, b, a, Contact::Attacked);
        seen.push((flagged(m, b), contested(m, b)));
        // Bob helps flagged Cid at rest.
        reset(m);
        flag(m, c);
        pvp_contact(m, b, c, Contact::Assist);
        seen.push((flagged(m, b), m.players[&b].combat.attackers.is_empty()));
        // Bob helps unflagged Cid in a creature's fight.
        reset(m);
        pvp_contact(m, b, c, Contact::Assist);
        seen.push((flagged(m, b), false));
        // A free-for-all fight.
        reset(m);
        for g in [a, b] {
            m.players.get_mut(&g).unwrap().fields.set_flag(
                PLAYER_FLAGS,
                PLAYER_FLAGS_FFA_PVP,
                true,
            );
        }
        flag(m, b);
        crate::game::map::combat::deal_damage(m, a, b, 1);
        seen.push((m.players[&a].combat.last_pvp.is_some(), contested(m, a)));
        for g in [a, b] {
            m.players.get_mut(&g).unwrap().fields.set_flag(
                PLAYER_FLAGS,
                PLAYER_FLAGS_FFA_PVP,
                false,
            );
        }
        // A flagged creature: its attacker flagged, not contested; its
        // helper at rest not, in a fight so; struck by it, no one.
        reset(m);
        flag(m, tiger);
        pvp_contact(m, a, tiger, Contact::Attack);
        seen.push((flagged(m, a), contested(m, a)));
        reset(m);
        flag(m, tiger);
        pvp_contact(m, b, tiger, Contact::Assist);
        let at_rest = flagged(m, b);
        m.unit_fields_mut(tiger).unwrap().set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            true,
        );
        pvp_contact(m, b, tiger, Contact::Assist);
        seen.push((at_rest, flagged(m, b)));
        reset(m);
        pvp_contact(m, c, tiger, Contact::Attacked);
        seen.push((flagged(m, c), false));
        seen
    })
    .await;
    assert_eq!(
        seen,
        vec![
            (true, true),
            (true, false),
            (true, true),
            (false, false),
            (false, false),
            (true, false),
            (false, true),
            (false, false),
        ]
    );
}

/// Thistle Tea's duels: duellists flag each other neither way.
#[tokio::test]
async fn a_duels_blows_flag_no_one() {
    let realm = harness::realm_with(data());
    let clients = [player(&realm, "Alice").await, player(&realm, "Bob").await];
    let (a, b) = (clients[0].guid, clients[1].guid);
    let flags = on_map(&realm, move |m| {
        for (me, them) in [(a, b), (b, a)] {
            m.players.get_mut(&me).unwrap().duel = Some(crate::game::map::duel::Duel {
                opponent: them,
                flag: Guid::zero(),
                phase: crate::game::map::duel::Phase::Fighting,
                initiator: me == a,
                out_of_bounds_since: None,
            });
        }
        m.unit_fields_mut(b)
            .unwrap()
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
        pvp_contact(m, a, b, Contact::Attack);
        pvp_contact(m, a, b, Contact::Attacked);
        (flagged(m, a), m.players[&a].combat.last_pvp)
    })
    .await;
    assert_eq!(flags, (false, None));
}

/// Thistle Tea's `territory/6`: on a normal realm only an enemy capital
/// flags, not a friendly one nor contested land, and leaving it the flag
/// lingers; on a PvP realm contested and enemy land flag, one's own not;
/// a battleground's copy always.
#[tokio::test]
async fn the_land_that_flags() {
    let realm = harness::realm_with(data());
    let _alice = player(&realm, "Alice").await;
    let normal = on_map(&realm, move |m| {
        [ORGRIMMAR, STORMWIND, STRANGLETHORN, BARRENS]
            .map(|zone| pvp::enforced(m, Team::Alliance, zone))
    })
    .await;
    assert_eq!(normal, [true, false, false, false]);

    let mut cfg = harness::test_config();
    cfg.pvp_realm = true;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let pvp_realm = Realm::new(cfg, store, startup, data(), None);
    let _alice_too = player(&pvp_realm, "Alice").await;
    let pvp = on_map(&pvp_realm, move |m| {
        [STRANGLETHORN, BARRENS, ELWYNN, STORMWIND]
            .map(|zone| pvp::enforced(m, Team::Alliance, zone))
    })
    .await;
    assert_eq!(pvp, [true, true, false, false]);

    // Into Orgrimmar and out: flagged there, and five minutes after.
    let bob = player(&realm, "Bob").await;
    let g = bob.guid;
    let (inside, after, until) = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().zone = ORGRIMMAR;
        pvp::update(m);
        let inside = flagged(m, g);
        m.players.get_mut(&g).unwrap().zone = BARRENS;
        pvp::update(m);
        (inside, flagged(m, g), m.players[&g].pvp.until)
    })
    .await;
    assert!(inside && after);
    assert!(until.is_some_and(|t| t > Instant::now() + PVP_LINGER - Duration::from_secs(5)));
}

/// Thistle Tea's flying: an enemy capital flown over on a taxi doesn't
/// flag.
#[tokio::test]
async fn an_enemy_capital_flown_over_doesnt_flag() {
    let realm = harness::realm_with(data());
    let alice = player(&realm, "Alice").await;
    let g = alice.guid;
    let flags = on_map(&realm, move |m| {
        let now = m.now;
        let at = m.players[&g].position();
        let p = m.players.get_mut(&g).unwrap();
        p.flight = Some(crate::game::map::npc::Flight {
            walk: crate::game::map::creature::Walk::new(vec![at, at], 30.0, now, 1, true),
            destination: 2,
            mount: 0,
        });
        p.zone = ORGRIMMAR;
        pvp::update(m);
        let flying = flagged(m, g);
        // Landed there: flagged.
        m.players.get_mut(&g).unwrap().flight = None;
        pvp::update(m);
        (flying, flagged(m, g))
    })
    .await;
    assert_eq!(flags, (false, true));
}

/// Thistle Tea's arena: an arena's subzone is free-for-all, the land
/// around it not.
#[test]
fn an_arena_is_free_for_all() {
    let d = data();
    assert!(crate::game::map::world::area_is_ffa(&d, ARENA));
    assert!(!crate::game::map::world::area_is_ffa(&d, STRANGLETHORN));
}

/// Thistle Tea's `tick/2` on a death: a contested player's thirty seconds
/// and the flag's five minutes run out though they died fighting.
#[tokio::test]
async fn a_death_holds_no_timer() {
    let realm = harness::realm_with(data());
    let clients = [player(&realm, "Alice").await, player(&realm, "Bob").await];
    let (a, b) = (clients[0].guid, clients[1].guid);
    let left = on_map(&realm, move |m| {
        m.unit_fields_mut(b)
            .unwrap()
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
        crate::game::map::combat::deal_damage(m, a, b, 1);
        crate::game::map::death::kill(m, a);
        m.now += PVP_LINGER + Duration::from_secs(1);
        pvp::update(m);
        crate::game::map::combat::update_players(m);
        pvp::update(m);
        (
            m.players[&a]
                .fields
                .has_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP),
            flagged(m, a),
        )
    })
    .await;
    assert_eq!(left, (false, false));
}

fn award(kind: AwardKind, points: f32, victim: Option<u64>) -> Award {
    Award {
        kind,
        points,
        victim,
        victim_rank: 0,
    }
}

/// Thistle Tea's `Honor.award/3`: bonus honor adds contribution, no kill
/// and no victim; a dishonorable kill costs rank points at once, never
/// below none, the highest rank and the week's contribution kept; an
/// award of nothing makes no kill.
#[test]
fn honor_awards_count_what_they_are() {
    let mut l = Ledger::new(100, 4);
    l.register(1, 0, 60);
    l.award(1, award(AwardKind::Bonus, 396.0, None));
    let p = l.project(1);
    assert_eq!(
        (
            p.this_week_contribution,
            p.today_honorable,
            p.lifetime_honorable
        ),
        (396, 0, 0)
    );
    assert!(l.entries[&1].days[&100].victims.is_empty());

    let mut l = Ledger::new(100, 4);
    l.register(1, 0, 60);
    l.entries.get_mut(&1).unwrap().rank_points = 5_050.0;
    l.entries.get_mut(&1).unwrap().highest_rank = 7;
    l.award(1, award(AwardKind::Honorable, 188.0, Some(9)));
    l.award(1, award(AwardKind::Dishonorable, 100.0, None));
    let e = &l.entries[&1];
    assert_eq!((e.rank_points, e.highest_rank), (4_950.0, 7));
    assert_eq!(e.lifetime_dishonorable_kills, 1);
    assert_eq!(
        (e.days[&100].contribution, e.days[&100].dishonorable_kills),
        (188.0, 1)
    );
    l.entries.get_mut(&1).unwrap().rank_points = 50.0;
    l.award(1, award(AwardKind::Dishonorable, 100.0, None));
    assert_eq!(l.entries[&1].rank_points, 0.0);

    let mut l = Ledger::new(100, 4);
    l.register(1, 0, 60);
    let before = l.entries[&1].clone();
    l.award(1, award(AwardKind::Honorable, 0.0, Some(9)));
    assert_eq!(l.entries[&1], before, "no phantom kill");
}

/// Thistle Tea's `settle/3`: the week settles once, its kills and
/// contribution last week's, its last day yesterday's still, and the new
/// week's credit its own.
#[test]
fn a_week_settles_once_and_keeps_yesterday() {
    // Day 98 a Thursday: a week turning on Thursdays starts on it.
    let mut l = Ledger::new(98, 4);
    l.register(1, 0, 60);
    l.award(1, award(AwardKind::Honorable, 100.0, Some(7)));
    l.advance(104);
    l.award(1, award(AwardKind::Honorable, 200.0, Some(8)));
    assert!(l.advance(105), "the week settled");
    l.award(1, award(AwardKind::Bonus, 396.0, None));
    let p = l.project(1);
    // No standing for two kills: fifteen a week are wanted to be ranked.
    assert_eq!(
        (
            p.last_week_kills,
            p.last_week_contribution,
            p.last_week_standing
        ),
        (2, 300, 0)
    );
    assert_eq!(p.yesterday_contribution, 200);
    assert_eq!((p.this_week_kills, p.this_week_contribution), (0, 396));
    let settled = l.entries[&1].clone();
    assert!(!l.advance(105), "once");
    assert_eq!(l.entries[&1], settled);
}
