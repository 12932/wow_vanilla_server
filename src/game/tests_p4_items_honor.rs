//! Phase 4 against Thistle Tea's honor tests (`test/game/core/honor`):
//! the damage a kill's honor is shared by (after shields, the killing blow
//! whole, once), none from a duel, Spirit of Redemption's at the first
//! defeat; a creature's honor at its first death, none for one Honorless
//! or a pet, each member's grey level their own; the ledger's levels, GM
//! rank points, weeks caught up and victims worth nothing; and Honorless
//! Target only for the living, on the ground, arriving where they must be
//! flagged.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellDuration, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::honor::{self, Award, AwardKind, Ledger};
use crate::game::map::auras::{AURA_NO_PVP_CREDIT, AURA_SCHOOL_ABSORB};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::pvp::{self, SPELL_HONORLESS_TARGET};
use crate::game::map::spells::redemption;
use crate::game::map::{Map, MapKey, stats};
use crate::game::tests_p3_combat::{aura_of, forced_swing};

/// Northshire, where every one of them is put.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const STORMWIND: u32 = 1519;
const UNDERCITY: u32 = 1497;
const LEADER: u32 = 700;
const PEASANT: u32 = 701;

fn template(entry: u32, level: u8, civilian: bool, leader: bool) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        display_probabilities: [100, 0, 0, 0],
        level_min: level,
        level_max: level,
        faction: 35,
        health_max: 10,
        civilian,
        racial_leader: leader,
        xp_multiplier: Some(1.0),
        ..Default::default()
    }
}

/// Honorless Target, thirty seconds of no PvP credit; the two capitals;
/// a racial leader and a level 20 civilian.
fn data() -> GameData {
    let mut d = GameData::empty();
    let mut s = SpellTemplate {
        id: SPELL_HONORLESS_TARGET,
        name: "Honorless Target".into(),
        equipped_item_class: -1,
        duration_index: 1,
        ..Default::default()
    };
    s.effects[0].effect = 6;
    s.effects[0].aura = AURA_NO_PVP_CREDIT;
    s.effects[0].target_a = 1;
    d.spells.spells.insert(SPELL_HONORLESS_TARGET, s);
    d.spells.durations.insert(
        1,
        SpellDuration {
            base: 30_000,
            per_level: 0,
            max: 30_000,
        },
    );
    // The spirit's form: unattackable while it lasts.
    super::tests_pvp::marker(&mut d, redemption::FORM);
    for (id, team) in [(STORMWIND, 2), (UNDERCITY, 4)] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map: 0,
                flags: AREA_FLAG_CAPITAL,
                team,
                name: format!("Capital {id}"),
                ..Default::default()
            },
        );
    }
    d.creature_templates
        .insert(LEADER, template(LEADER, 1, false, true));
    d.creature_templates
        .insert(PEASANT, template(PEASANT, 20, true, false));
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

/// `who` (name, race, class) at level 60, all in Northshire.
async fn cast(realm: &Arc<Realm>, who: &[(&str, Race, Class)]) -> (Vec<Client>, Vec<Guid>) {
    let mut clients = Vec::new();
    let mut guids = Vec::new();
    for (name, race, class) in who {
        let mut c = Client::connect(realm, name);
        guids.push(c.enter(name, *race, *class).await);
        clients.push(c);
    }
    let all = guids.clone();
    on_map(realm, move |m| {
        let data = m.realm.data.clone();
        for g in all {
            let p = m.players.get_mut(&g).unwrap();
            stats::set_level(p, &data, 60);
            p.movement.position = Vector3d {
                x: START.0,
                y: START.1,
                z: START.2,
            };
        }
    })
    .await;
    (clients, guids)
}

/// The honor each of `who` has today.
fn today(realm: &Realm, who: &[Guid]) -> Vec<u32> {
    let ledger = realm.social.honor.lock();
    who.iter()
        .map(|g| {
            ledger
                .entries
                .get(&guid::counter(*g))
                .and_then(|e| e.days.get(&ledger.day))
                .map_or(0, |d| d.contribution as u32)
        })
        .collect()
}

/// Lifetime honorable and dishonorable kills of `g`.
fn lifetime(realm: &Realm, g: Guid) -> (u32, u32) {
    let p = realm.social.honor.lock().project(guid::counter(g));
    (p.lifetime_honorable, p.lifetime_dishonorable)
}

/// Two level 60s, the victim unranked, by `share`.
fn worth(share: f32) -> u32 {
    (honor::kill_points(60, 60, 0, 0) * share).trunc() as u32
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn set_health(m: &mut Map, g: Guid, h: u32) {
    m.unit_fields_mut(g).unwrap().set_u32(UNIT_FIELD_HEALTH, h);
}

/// Thistle Tea's `combat_test`: a blow on a shield counts what got past
/// it; the killing blow counts whole, past the health it took, and once;
/// the dead are hurt no more.
#[tokio::test]
async fn honor_counts_damage_past_shields_and_the_killing_blow_whole_once() {
    let realm = harness::realm_with(data());
    let (_c, g) = cast(
        &realm,
        &[
            ("Alice", Race::Human, Class::Warrior),
            ("Bob", Race::Human, Class::Warrior),
            ("Grim", Race::Undead, Class::Warrior),
        ],
    )
    .await;
    let (alice, bob, grim) = (g[0], g[1], g[2]);
    let (lost, recorded, shield_left) = on_map(&realm, move |m| {
        // Grim a step in front of Alice, facing her.
        let p = m.players.get_mut(&grim).unwrap();
        p.movement.position.x += 1.0;
        p.movement.orientation = std::f32::consts::PI;
        m.players.get_mut(&alice).unwrap().movement.orientation = 0.0;
        let f = m.unit_fields_mut(alice).unwrap();
        f.set_f32(UNIT_FIELD_MINDAMAGE, 100.0);
        f.set_f32(UNIT_FIELD_MAXDAMAGE, 100.0);
        aura_of(m, grim, 95_700, AURA_SCHOOL_ABSORB, 127, 20);
        let before = health(m, grim);
        forced_swing(m, alice, grim, Outcome::Hit);
        let shield =
            crate::game::map::auras::auras(m, grim).is_some_and(|a| a.has(AURA_SCHOOL_ABSORB));
        (
            before - health(m, grim),
            m.players[&grim].honor_damage.by.clone(),
            shield,
        )
    })
    .await;
    assert!(lost > 0, "the blow got past the shield");
    assert!(!shield_left, "the shield was used up");
    assert_eq!(recorded, vec![(alice, lost)], "what got past it");
    // Bob's 120 on 50 health kills: his share is of 120, not 50.
    let after = on_map(&realm, move |m| {
        set_health(m, grim, 50);
        combat::deal_damage(m, bob, grim, 120);
        let alive = m.unit_view(grim).is_some_and(|u| u.alive);
        combat::deal_damage(m, bob, grim, 1);
        (alive, m.players[&grim].honor_damage.by.clone())
    })
    .await;
    assert_eq!(after, (false, vec![]), "dead, and nothing more counted");
    let total = (lost + 120) as f32;
    assert_eq!(
        today(&realm, &[alice, bob]),
        vec![worth(lost as f32 / total), worth(120.0 / total)]
    );
}

/// Thistle Tea's `combat_test`: a duel's blow that would kill leaves one
/// health and no honorable kill.
#[tokio::test]
async fn a_duel_lost_is_no_honorable_kill() {
    let realm = harness::realm_with(data());
    let (mut c, g) = cast(
        &realm,
        &[
            ("Alice", Race::Human, Class::Warrior),
            ("Grim", Race::Undead, Class::Warrior),
        ],
    )
    .await;
    let (alice, grim) = (g[0], g[1]);
    let left = on_map(&realm, move |m| {
        for (me, them) in [(alice, grim), (grim, alice)] {
            m.players.get_mut(&me).unwrap().duel = Some(crate::game::map::duel::Duel {
                opponent: them,
                flag: Guid::zero(),
                phase: crate::game::map::duel::Phase::Fighting,
                initiator: me == alice,
                out_of_bounds_since: None,
                started: None,
            });
        }
        set_health(m, grim, 50);
        combat::deal_damage(m, alice, grim, 200);
        (health(m, grim), m.unit_view(grim).is_some_and(|u| u.alive))
    })
    .await;
    assert_eq!(left, (1, true));
    c[0].wait_for("SMSG_DUEL_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_DUEL_COMPLETE(_)).then_some(())
    })
    .await;
    assert_eq!(lifetime(&realm, alice), (0, 0));
    assert_eq!(today(&realm, &[alice]), vec![0]);
}

/// Thistle Tea's `combat_test` (mangos's `Unit::Kill`, which rewards the
/// kill before Spirit of Redemption takes the priest, and nothing at the
/// spirit's own end): the honor is had when the priest falls, and none
/// when the spirit dies.
#[tokio::test]
async fn spirit_of_redemption_gives_its_honor_when_the_priest_falls_and_none_at_its_end() {
    let realm = harness::realm_with(data());
    let (_c, g) = cast(
        &realm,
        &[
            ("Alice", Race::Human, Class::Warrior),
            ("Bob", Race::Human, Class::Warrior),
            ("Hilda", Race::Undead, Class::Priest),
        ],
    )
    .await;
    let (alice, bob, hilda) = (g[0], g[1], g[2]);
    let saved = on_map(&realm, move |m| {
        m.players
            .get_mut(&hilda)
            .unwrap()
            .spells
            .push(redemption::TALENT);
        set_health(m, hilda, 100);
        combat::deal_damage(m, alice, hilda, 200);
        (
            redemption::spirit(m, hilda),
            m.unit_view(hilda).is_some_and(|u| u.alive),
        )
    })
    .await;
    assert_eq!(saved, (true, true), "a spirit, not dead");
    assert_eq!(today(&realm, &[alice]), vec![worth(1.0)]);
    // Bob's blow on the spirit is nothing; its end kills her, for no one.
    let dead = on_map(&realm, move |m| {
        combat::deal_damage(m, bob, hilda, 200);
        redemption::aura_gone(m, hilda, redemption::STATE[1], true);
        m.unit_view(hilda).is_some_and(|u| !u.alive)
    })
    .await;
    assert!(dead);
    assert_eq!(today(&realm, &[alice, bob]), vec![worth(1.0), 0]);
    assert_eq!(lifetime(&realm, alice), (1, 0));
}

/// Thistle Tea's `contribution_test`: players' and creatures' damage is
/// kept until a minute passes with none; then it starts over. A blow of
/// nothing changes nothing, its time neither.
#[tokio::test]
async fn damage_is_remembered_until_a_minute_passes_without_any() {
    let realm = harness::realm_with(data());
    let (_c, g) = cast(
        &realm,
        &[
            ("Alice", Race::Human, Class::Warrior),
            ("Bob", Race::Human, Class::Warrior),
            ("Grim", Race::Undead, Class::Warrior),
        ],
    )
    .await;
    let (alice, bob, grim) = (g[0], g[1], g[2]);
    let creature = guid::unit(3500, 1);
    let got = on_map(&realm, move |m| {
        let start = m.now;
        pvp::damaged(m, alice, grim, 100);
        m.now = start + Duration::from_secs(30);
        pvp::damaged(m, creature, grim, 50);
        m.now = start + Duration::from_secs(80);
        pvp::damaged(m, bob, grim, 200);
        let kept = m.players[&grim].honor_damage.clone();
        // Nothing: not counted, and the minute isn't started again.
        m.now = start + Duration::from_secs(100);
        pvp::damaged(m, alice, grim, 0);
        let unchanged = m.players[&grim].honor_damage.last == kept.last
            && m.players[&grim].honor_damage.by == kept.by;
        let at = |s: u64, ms: u64| start + Duration::from_secs(s) + Duration::from_millis(ms);
        let h = &m.players[&grim].honor_damage;
        let (minute, past) = (
            h.current(at(140, 0)).to_vec(),
            h.current(at(140, 1)).to_vec(),
        );
        m.now = at(140, 1);
        pvp::damaged(m, alice, grim, 100);
        (
            kept.by,
            unchanged,
            minute,
            past,
            m.players[&grim].honor_damage.by.clone(),
        )
    })
    .await;
    let all = vec![(alice, 100), (Guid::zero(), 50), (bob, 200)];
    assert_eq!(got.0, all);
    assert!(got.1, "a blow of nothing changes nothing");
    assert_eq!(got.2, all, "a minute after the last, still kept");
    assert!(got.3.is_empty(), "past the minute, forgotten");
    assert_eq!(got.4, vec![(alice, 100)], "and started over");
}

/// The credit `c` was sent: its honor and the victim's rank.
async fn credit(c: &mut Client) -> (i32, u32) {
    let f = c
        .wait_frame("SMSG_PVP_CREDIT", pvp::SMSG_PVP_CREDIT, |_| true)
        .await;
    let honor = i32::from_le_bytes(f[0..4].try_into().unwrap());
    let rank = u32::from_le_bytes(f[12..16].try_into().unwrap());
    (honor, rank)
}

/// Thistle Tea's `creature_test`: a racial leader is worth 488 at rank 19
/// however low; only its first death counts.
#[tokio::test]
async fn a_racial_leader_is_worth_its_honor_grey_and_only_at_its_first_death() {
    let realm = harness::realm_with(data());
    let (mut c, g) = cast(&realm, &[("Grim", Race::Undead, Class::Warrior)]).await;
    let grim = g[0];
    on_map(&realm, move |m| {
        let at = m.players[&grim].position();
        let leader = crate::game::map::scripts::summon(m, grim, LEADER, at, 0.0, None).unwrap();
        combat::deal_damage(m, grim, leader, 100);
        combat::deal_damage(m, grim, leader, 100);
    })
    .await;
    assert_eq!(credit(&mut c[0]).await, (488, 19));
    assert_eq!(lifetime(&realm, grim), (1, 0));
    assert_eq!(today(&realm, &[grim]), vec![488]);
}

/// Thistle Tea's `creature_test`: a creature Honorless, or a pet, is worth
/// nothing.
#[tokio::test]
async fn an_honorless_creature_and_a_pet_are_worth_nothing() {
    let realm = harness::realm_with(data());
    let (_c, g) = cast(
        &realm,
        &[
            ("Grim", Race::Undead, Class::Warrior),
            ("Alice", Race::Human, Class::Warrior),
        ],
    )
    .await;
    let (grim, alice) = (g[0], g[1]);
    on_map(&realm, move |m| {
        let at = m.players[&grim].position();
        let protected = crate::game::map::scripts::summon(m, grim, LEADER, at, 0.0, None).unwrap();
        aura_of(m, protected, 95_701, AURA_NO_PVP_CREDIT, 0, 0);
        assert!(
            crate::game::map::auras::auras(m, protected).is_some_and(|a| a.has(AURA_NO_PVP_CREDIT))
        );
        combat::deal_damage(m, grim, protected, 100);
        let pet = crate::game::map::scripts::summon(m, grim, LEADER, at, 0.0, None).unwrap();
        m.creature_mut(pet).unwrap().owner = Some(alice);
        combat::deal_damage(m, grim, pet, 100);
    })
    .await;
    assert_eq!(lifetime(&realm, grim), (0, 0));
}

/// Thistle Tea's `creature_test`: a civilian grey to one member and not to
/// another is a dishonorable kill for the first alone.
#[tokio::test]
async fn a_civilian_is_dishonorable_for_the_members_it_is_grey_to_alone() {
    let realm = harness::realm_with(data());
    let (mut c, g) = cast(
        &realm,
        &[
            ("Alice", Race::Human, Class::Warrior),
            ("Bob", Race::Human, Class::Warrior),
        ],
    )
    .await;
    let (alice, bob) = (g[0], g[1]);
    let (lead, rest) = c.split_at_mut(1);
    lead[0].send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    rest[0]
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    rest[0].send(CMSG_GROUP_ACCEPT {}).await;
    lead[0]
        .wait_for("SMSG_GROUP_LIST", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_LIST(_)).then_some(())
        })
        .await;
    on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        stats::set_level(m.players.get_mut(&bob).unwrap(), &data, 10);
        let at = m.players[&alice].position();
        let peasant = crate::game::map::scripts::summon(m, alice, PEASANT, at, 0.0, None).unwrap();
        combat::deal_damage(m, alice, peasant, 100);
    })
    .await;
    // Level 20 is grey to 60 (47 and under), not to 10.
    assert_eq!(lifetime(&realm, alice), (0, 1));
    assert_eq!(lifetime(&realm, bob), (0, 0));
}

fn award(kind: AwardKind, points: f32, victim: Option<u64>) -> Award {
    Award {
        kind,
        points,
        victim,
        victim_rank: 0,
    }
}

/// Thistle Tea's `ledger_test`: a character known again at a new level
/// keeps their honor.
#[test]
fn a_new_level_keeps_the_honor() {
    let mut l = Ledger::new(100, 4);
    l.register(1, 0, 50);
    l.award(1, award(AwardKind::Bonus, 100.0, None));
    let before = l.entries[&1].clone();
    l.register(1, 0, 51);
    let e = &l.entries[&1];
    assert_eq!(e.level, 51);
    assert_eq!(e.days, before.days);
    assert_eq!(
        (e.rank_points, e.highest_rank),
        (before.rank_points, before.highest_rank)
    );
}

/// Thistle Tea's `ledger_test`: a GM's rank points are capped by the
/// level, the highest rank stays when they are taken away, and the day's
/// honor is untouched; one honor doesn't know of changes nothing.
#[test]
fn a_gms_rank_points_are_capped_by_the_level_and_keep_the_highest_rank() {
    let mut l = Ledger::new(100, 4);
    l.register(1, 0, 30);
    l.award(1, award(AwardKind::Bonus, 100.0, None));
    let days = l.entries[&1].days.clone();
    l.set_rank_points(1, 65_000.0);
    assert_eq!(
        (l.entries[&1].rank_points, l.entries[&1].highest_rank),
        (7_150.0, 7)
    );
    l.set_rank_points(1, 0.0);
    let e = &l.entries[&1];
    assert_eq!((e.rank_points, e.highest_rank), (0.0, 7));
    assert_eq!(e.days, days);
    l.set_rank_points(2, 100.0);
    assert!(!l.entries.contains_key(&2));
}

/// Thistle Tea's `ledger_test`: three weeks gone by are each settled, the
/// first ranked, the two after decaying; the kills of a lifetime and the
/// highest rank kept. Loaded from the store three weeks on, the same.
#[test]
fn weeks_gone_by_are_each_caught_up() {
    let mut l = Ledger::new(100, 4);
    let start = l.week_start;
    assert_eq!(start, 98);
    l.day = start;
    l.register(1, 0, 60);
    for i in 0..15 {
        l.award(1, award(AwardKind::Honorable, 188.0, Some(i)));
    }
    // Ranked alone: 3,000; then 2,700 and 2,430.
    l.advance(start + 21);
    let e = &l.entries[&1];
    assert_eq!(e.rank_points, 2_430.0);
    assert_eq!(e.last_standing, 0);
    assert_eq!(e.last_week.honorable_kills, 0);
    assert_eq!(e.lifetime_honorable_kills, 15);
    assert_eq!(e.highest_rank, 6);
    assert!(e.days.is_empty());
    // Kept once its first week was settled, loaded two weeks on: the two
    // are caught up at load.
    let mut once = Ledger::new(start, 4);
    once.register(1, 0, 60);
    for i in 0..15 {
        once.award(1, award(AwardKind::Honorable, 188.0, Some(i)));
    }
    once.advance(start + 7);
    let records = vec![(
        honor::KIND.to_owned(),
        1,
        serde_json::to_string(&once.entries[&1]).unwrap(),
    )];
    let loaded = Ledger::load(&records, start + 21, 4);
    assert_eq!(loaded.entries[&1].rank_points, 2_430.0);
    assert_eq!(loaded.entries[&1].highest_rank, 6);
}

/// Thistle Tea's `ledger_test`: no honor for a victim of one's own side,
/// a grey one, one unknown, nor one killed ten times today already.
#[test]
fn friends_greys_strangers_and_the_tenth_kill_are_worth_nothing() {
    let mut l = Ledger::new(100, 4);
    l.register(1, 0, 60);
    l.register(2, 0, 60);
    // Grey at 60: 47 and below (mangos's `IsTrivialLevelDifference`;
    // Thistle Tea's 51).
    l.register(3, 1, 47);
    l.register(4, 1, 60);
    for victim in [2, 3, 5] {
        assert_eq!(l.player_kill(1, victim, 1.0), None, "victim {victim}");
    }
    for _ in 0..10 {
        assert!(l.player_kill(1, 4, 1.0).is_some());
    }
    let before = l.entries[&1].clone();
    assert_eq!(l.player_kill(1, 4, 1.0), None);
    assert_eq!(l.entries[&1], before);
    assert_eq!(before.lifetime_honorable_kills, 10);
}

/// When `g`'s Honorless Target ends, if they have it.
fn honorless(m: &Map, g: Guid) -> Option<Option<std::time::Instant>> {
    crate::game::map::auras::auras(m, g).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.spell == SPELL_HONORLESS_TARGET)
            .map(|h| h.expires)
    })
}

/// `g` arrives (as at a teleport's end) in `zone`.
fn arrive(m: &mut Map, g: Guid, zone: u32) {
    m.players.get_mut(&g).unwrap().zone = zone;
    pvp::arrived(m, g);
}

/// Thistle Tea's `protection_test`: arriving in one's own capital, flagged
/// by choice, is no Honorless Target; nor in the enemy's, dead or a ghost,
/// or flying.
#[tokio::test]
async fn honorless_target_is_only_for_the_living_on_the_ground_where_they_must_be_flagged() {
    let realm = harness::realm_with(data());
    let (_c, g) = cast(
        &realm,
        &[
            ("Alice", Race::Human, Class::Warrior),
            ("Grim", Race::Undead, Class::Warrior),
            ("Gash", Race::Undead, Class::Warrior),
            ("Gorm", Race::Undead, Class::Warrior),
        ],
    )
    .await;
    let (alice, grim, gash, gorm) = (g[0], g[1], g[2], g[3]);
    let got = on_map(&realm, move |m| {
        pvp::toggle(m, alice, Some(true));
        arrive(m, alice, STORMWIND);
        crate::game::map::death::kill(m, grim);
        arrive(m, grim, STORMWIND);
        let now = m.now;
        m.players.get_mut(&gash).unwrap().death =
            crate::game::map::death::Death::Ghost { since: now };
        arrive(m, gash, STORMWIND);
        let at = m.players[&gorm].position();
        m.players.get_mut(&gorm).unwrap().flight = Some(crate::game::map::npc::Flight {
            walk: crate::game::map::creature::Walk::new(vec![at, at], 30.0, now, 1, true),
            destination: 2,
            mount: 0,
        });
        arrive(m, gorm, STORMWIND);
        let none = [alice, grim, gash, gorm].map(|g| honorless(m, g).is_none());
        // Landed: Gorm is.
        m.players.get_mut(&gorm).unwrap().flight = None;
        arrive(m, gorm, STORMWIND);
        (none, honorless(m, gorm).is_some())
    })
    .await;
    assert_eq!(got, ([true; 4], true));
}

/// Thistle Tea's `protection_test`: an Honorless Target arriving home
/// keeps it as it was, not made new.
#[tokio::test]
async fn a_safe_arrival_doesnt_renew_honorless_target() {
    let realm = harness::realm_with(data());
    let (_c, g) = cast(&realm, &[("Grim", Race::Undead, Class::Warrior)]).await;
    let grim = g[0];
    let got = on_map(&realm, move |m| {
        arrive(m, grim, STORMWIND);
        let first = honorless(m, grim);
        m.now += Duration::from_secs(14);
        arrive(m, grim, UNDERCITY);
        let home = honorless(m, grim);
        // Back in the enemy's: made new.
        arrive(m, grim, STORMWIND);
        (first, home, honorless(m, grim), m.now)
    })
    .await;
    let first = got.0.flatten().expect("an Honorless Target, for a while");
    assert_eq!(got.1, Some(Some(first)), "home, the same");
    assert_eq!(got.2, Some(Some(got.3 + Duration::from_secs(30))));
}
