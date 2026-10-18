//! Phase 4's conditions, Thistle Tea's `CreatureGroupScriptsTest`,
//! `EntityContextTest` and `LocalFactsTest` ported: a script joining and
//! leaving a creature group, the group drawn into its leader's fight on its
//! clock, membership and deaths as conditions; the source's unit and npc
//! flags; an aura's effect index; quest availability (events, skills with
//! their bonuses, exclusive groups, breadcrumbs, nested conditions); the
//! boundary facts (events, level, spells, patch, map, time, exploration);
//! zero maxima; and a companion.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::scripts::{CF_SWAP_TARGETS, Condition, GROUP_AGGRO_TOGETHER, ScriptStep};
use crate::game::data::spells::SpellEffect;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::conditions::{MINI_PET, Truth, check};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_entity::{aura_effect, spell};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
/// A group's leader and its member (entries and spawns alike), and a
/// third creature.
const LEADER: u32 = 41;
const MEMBER: u32 = 42;
const BOAR: u32 = 43;
/// A dummy at effect index 1 only.
const MARKED: u32 = 97_100;
/// A spell the player knows, one they don't.
const KNOWN: u32 = 97_101;
const UNKNOWN_SPELL: u32 = 97_102;
/// An event's quest; a skill's (and its exclusive partner); a breadcrumb
/// and its target with a condition no row has; a quest with that condition
/// itself.
const EVENT_QUEST: u32 = 9_100;
const SKILL_QUEST: u32 = 9_101;
const PARTNER: u32 = 9_102;
const BREADCRUMB: u32 = 9_103;
const TARGET: u32 = 9_104;
const CONDITIONED: u32 = 9_105;
const EVENT: u32 = 12;
const OTHER_EVENT: u32 = 7;
const FIRST_AID: u32 = 129;
const AREA: u32 = 4_242;

fn cond(d: &mut GameData, id: u32, kind: i32, values: [i32; 4], flags: u32) {
    d.scripts.conditions.insert(
        id,
        Condition {
            entry: id,
            kind,
            values,
            flags,
        },
    );
}

fn quest(id: u32) -> Quest {
    Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    // An aura, by spell, at any effect, at its effect 1, at 0 and at 2.
    let m = MARKED as i32;
    cond(&mut d, 1, 1, [m, -1, 0, 0], 0);
    cond(&mut d, 2, 1, [m, 1, 0, 0], 0);
    cond(&mut d, 3, 1, [m, 0, 0, 0], 0);
    cond(&mut d, 4, 1, [m, 2, 0, 0], 0);
    // Quests available: the event's, one not in the catalog, the skill's,
    // the breadcrumb, the one with a condition of its own.
    for (id, q) in [
        (10, EVENT_QUEST),
        (11, 99_999),
        (12, SKILL_QUEST),
        (13, BREADCRUMB),
        (14, CONDITIONED),
    ] {
        cond(&mut d, id, 19, [q as i32, 0, 0, 0], 0);
    }
    // The boundary facts: an event; level 1 equal, at least 0, at most 2,
    // equal 2; a spell known, one not; the patch, 10 and at least 11; this
    // map and another; the whole day; an area explored.
    cond(&mut d, 20, 12, [OTHER_EVENT as i32, 0, 0, 0], 0);
    cond(&mut d, 21, 15, [1, 0, 0, 0], 0);
    cond(&mut d, 22, 15, [0, 1, 0, 0], 0);
    cond(&mut d, 23, 15, [2, 2, 0, 0], 0);
    cond(&mut d, 24, 15, [2, 0, 0, 0], 0);
    cond(&mut d, 25, 17, [KNOWN as i32, 0, 0, 0], 0);
    cond(&mut d, 26, 17, [UNKNOWN_SPELL as i32, 1, 0, 0], 0);
    cond(&mut d, 27, 24, [10, 0, 0, 0], 0);
    cond(&mut d, 28, 24, [11, 1, 0, 0], 0);
    cond(&mut d, 29, 33, [0, 0, 0, 0], 0);
    cond(&mut d, 30, 33, [1, 0, 0, 0], 0);
    cond(&mut d, 31, 53, [0, 0, 23, 59], 0);
    cond(&mut d, 32, 59, [AREA as i32, 0, 0, 0], 0);
    // Full health and full mana.
    cond(&mut d, 40, 41, [100, 0, 0, 0], 0);
    cond(&mut d, 41, 42, [100, 0, 0, 0], 0);
    // A companion: a boar, any, a leader.
    cond(&mut d, 50, MINI_PET, [BOAR as i32, 0, 0, 0], 0);
    cond(&mut d, 51, MINI_PET, [0, 0, 0, 0], 0);
    cond(&mut d, 52, MINI_PET, [LEADER as i32, 0, 0, 0], 0);
    // The source's npc flags have 0x2; its unit flags 0x200, not 0x100.
    cond(&mut d, 60, 31, [i32::from(UNIT_NPC_FLAGS), 0x2, 0, 0], 0);
    cond(
        &mut d,
        61,
        31,
        [i32::from(UNIT_FIELD_FLAGS), 0x200, 0, 0],
        0,
    );
    cond(
        &mut d,
        62,
        31,
        [i32::from(UNIT_FIELD_FLAGS), 0x100, 0, 0],
        0,
    );
    // In the leader's group; its others dead; in the group, swapped.
    cond(&mut d, 70, 57, [LEADER as i32, 0, 0, 0], 0);
    cond(&mut d, 71, 58, [0, 0, 0, 0], 0);
    cond(&mut d, 72, 57, [LEADER as i32, 0, 0, 0], CF_SWAP_TARGETS);

    let mut marked = spell(
        MARKED,
        &[
            SpellEffect {
                effect: 3,
                target_a: 1,
                ..Default::default()
            },
            aura_effect(crate::game::map::auras::AURA_DUMMY, 1, 0, 1),
        ],
    );
    marked.duration_index = 0;
    d.spells.spells.insert(MARKED, marked);

    let event = quest(EVENT_QUEST);
    let mut skill = quest(SKILL_QUEST);
    skill.exclusive_group = 1;
    skill.required_skill = FIRST_AID;
    skill.required_skill_value = 50;
    let mut partner = quest(PARTNER);
    partner.exclusive_group = 1;
    let mut breadcrumb = quest(BREADCRUMB);
    breadcrumb.breadcrumb_for = TARGET;
    for q in [
        event,
        skill,
        partner,
        breadcrumb,
        quest(TARGET),
        quest(CONDITIONED),
    ] {
        d.npc.quests.insert(q.id, q);
    }
    d.world.event_quests.insert(EVENT_QUEST, EVENT);
    // Condition 42 has no row.
    d.scripts.quest_condition.insert(TARGET, 42);
    d.scripts.quest_condition.insert(CONDITIONED, 42);
    d.areas.insert(
        AREA,
        AreaInfo {
            id: AREA,
            explore_bit: 37,
            ..Default::default()
        },
    );
    // Factions: the players' (Human 1), and the creatures', hostile.
    for (id, ours, friends, enemies) in [(1, 1, 1, 8), (14, 8, 8, 1)] {
        d.faction_templates.insert(
            id,
            FactionTemplate {
                id,
                faction: id + 100,
                ours,
                friend_mask: friends,
                enemy_mask: enemies,
                ..Default::default()
            },
        );
    }
    for (entry, dx) in [(LEADER, 3.0), (MEMBER, 4.0), (BOAR, 5.0)] {
        d.creature_templates.insert(
            entry,
            CreatureTemplate {
                entry,
                name: format!("Beast {entry}"),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                display_scales: [1.0, 0.0, 0.0, 0.0],
                level_min: 1,
                level_max: 1,
                faction: 14,
                unit_class: 1,
                health_max: 500,
                speed_walk: 0.01,
                speed_run: 0.01,
                detection_range: 0.1,
                ..Default::default()
            },
        );
        d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
            guid: entry,
            entries: vec![entry],
            map: 0,
            x: START.0 + dx,
            y: START.1,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        });
    }
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

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    (realm, a, g)
}

fn creature(entry: u32) -> Guid {
    guid::unit(entry, entry)
}

fn step(command: u32, datalong: [u32; 4]) -> ScriptStep {
    ScriptStep {
        command,
        datalong,
        ..Default::default()
    }
}

/// Thistle Tea's creature group scripts: a script puts a creature in a
/// leader's group, at a distance and angle with its flags, and takes it
/// out again; in it, it is a member (the leader too, and swapped), and a
/// member's death leaves the leader's group dead; out of one, a creature
/// is no member, and its group (none) dead, as vmangos has it; a source
/// not here is unknown.
#[tokio::test]
async fn a_script_makes_a_group_whose_membership_and_deaths_are_conditions() {
    let (realm, _a, g) = setup().await;
    let (leader, member) = (creature(LEADER), creature(MEMBER));
    let seen = on_map(&realm, move |m| {
        let judge = |m: &Map| {
            [70, 71, 72]
                .map(|id| check(m, id, Some(leader), Some(member)))
                .to_vec()
        };
        let alone = judge(m);
        let join = ScriptStep {
            x: 4.0,
            o: 1.5,
            ..step(78, [GROUP_AGGRO_TOGETHER, 0, 0, 0])
        };
        assert!(crate::game::map::scripts::execute(m, &join, member, leader));
        let placed = m.script_groups.get(&member).copied();
        let joined = judge(m);
        crate::game::map::combat::kill(m, g, member);
        let dead = judge(m);
        assert!(crate::game::map::scripts::execute(
            m,
            &step(79, [0; 4]),
            member,
            member
        ));
        let left = (
            crate::game::map::creature_groups::members_of(m, member),
            check(m, 72, Some(leader), Some(member)),
        );
        let away = check(m, 71, Some(guid::unit(LEADER, 999)), None);
        (alone, placed, joined, dead, left, away)
    })
    .await;
    use Truth::*;
    let (alone, placed, joined, dead, left, away) = seen;
    assert_eq!(alone, vec![Unmet, Met, Unmet], "no group");
    assert_eq!(
        placed,
        Some((creature(LEADER), 4.0, 1.5, GROUP_AGGRO_TOGETHER))
    );
    assert_eq!(joined, vec![Met, Unmet, Met], "in the group");
    assert_eq!(dead, vec![Met, Met, Met], "the member dead");
    assert_eq!(left, (Vec::new(), Unmet), "out of it again");
    assert_eq!(away, Unknown, "a source not here");
}

/// Thistle Tea's group attack and assistance: a leader drawn into a fight
/// draws its group in once, on its own leash clock; a second enemy of the
/// leader's in the same fight isn't the member's; the member fights on
/// when the leader dies.
#[tokio::test]
async fn a_group_joins_its_leaders_fight_once_on_its_clock() {
    let (realm, _a, alice) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    let (leader, member) = (creature(LEADER), creature(MEMBER));
    let seen = on_map(&realm, move |m| {
        crate::game::map::creature_groups::join_with(
            m,
            member,
            leader,
            2.0,
            0.0,
            GROUP_AGGRO_TOGETHER,
        );
        crate::game::map::ai::attacked(m, leader, alice);
        let foes = |m: &Map, c: Guid| -> Vec<Guid> {
            m.creature(c)
                .map(|c| c.threat.iter().map(|(g, _)| *g).collect())
                .unwrap_or_default()
        };
        let clock = |m: &Map, c: Guid| m.creature(c).and_then(|c| c.engagement.clock);
        let drawn = (
            foes(m, member),
            clock(m, member).is_some() && clock(m, member) == clock(m, leader),
        );
        crate::game::map::ai::attacked(m, leader, bob);
        let second = (foes(m, leader), foes(m, member));
        crate::game::map::combat::kill(m, alice, leader);
        let fighting = m
            .creature(member)
            .is_some_and(|c| c.alive() && c.combat.attacking);
        (drawn, second, fighting)
    })
    .await;
    let (drawn, second, fighting) = seen;
    assert_eq!(drawn, (vec![alice], true), "drawn in on the leader's clock");
    assert_eq!(second, (vec![alice, bob], vec![alice]), "drawn in once");
    assert!(fighting, "the member fights on");
}

/// Thistle Tea's entity context: a creature source's npc flags and unit
/// flags, through has-flag.
#[tokio::test]
async fn the_sources_npc_and_unit_flags_have_their_flags() {
    let (realm, _a, _g) = setup().await;
    let boar = creature(BOAR);
    let seen = on_map(&realm, move |m| {
        let f = m.unit_fields_mut(boar).unwrap();
        f.set_u32(UNIT_FIELD_FLAGS, 0x200);
        f.set_u32(UNIT_NPC_FLAGS, 0x2);
        [60, 61, 62].map(|id| check(m, id, Some(boar), None))
    })
    .await;
    assert_eq!(seen, [Truth::Met, Truth::Met, Truth::Unmet]);
}

/// Thistle Tea's aura presence: by spell at any effect, at the effect it
/// has, not at one it hasn't.
#[tokio::test]
async fn an_aura_condition_keeps_its_effect_index() {
    let (realm, _a, g) = setup().await;
    let seen = on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, MARKED, Some(g));
        [1, 2, 3, 4].map(|id| check(m, id, None, Some(g)))
    })
    .await;
    use Truth::*;
    assert_eq!(seen, [Met, Met, Unmet, Unmet]);
}

/// Thistle Tea's quest availability: an event's quest only with its event
/// on; one not in the catalog unknown; a skill's met with its bonus, not
/// without it, nor once its exclusive partner is rewarded; a breadcrumb to
/// a quest with a condition not judged, and a quest with one, unknown.
#[tokio::test]
async fn a_quests_availability_wants_its_event_skill_group_and_conditions() {
    let (realm, _a, g) = setup().await;
    let events = realm.clone();
    let event = |on: bool| {
        let mut e = events.social.events.lock();
        if on {
            e.insert(EVENT);
        } else {
            e.remove(&EVENT);
        }
    };
    event(false);
    let off = on_map(&realm, move |m| check(m, 10, None, Some(g))).await;
    event(true);
    let seen = on_map(&realm, move |m| {
        let on = check(m, 10, None, Some(g));
        let missing = check(m, 11, None, Some(g));
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, FIRST_AID, 1);
        crate::game::map::skills::set_value(p, FIRST_AID, 49);
        let slot = (0..128u16)
            .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == FIRST_AID)
            .unwrap();
        let bonus = PLAYER_SKILL_INFO_1_1 + slot * 3 + 2;
        p.fields.set_u32(bonus, 1);
        let with_bonus = check(m, 12, None, Some(g));
        m.players.get_mut(&g).unwrap().fields.set_u32(bonus, 0);
        let without = check(m, 12, None, Some(g));
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(bonus, 1);
        p.journal.rewarded.insert(PARTNER);
        let partnered = check(m, 12, None, Some(g));
        (
            on,
            missing,
            with_bonus,
            without,
            partnered,
            check(m, 13, None, Some(g)),
            check(m, 14, None, Some(g)),
        )
    })
    .await;
    event(false);
    use Truth::*;
    assert_eq!(off, Unmet, "the event off");
    assert_eq!(seen, (Met, Unknown, Met, Unmet, Unmet, Unknown, Unknown));
}

/// Thistle Tea's boundary facts: a game event; the level by each
/// comparison; a spell known and one not; the content patch; the map; the
/// time of day; an area explored.
#[tokio::test]
async fn events_level_spells_patch_map_time_and_exploration_are_judged() {
    let (realm, _a, g) = setup().await;
    realm.social.events.lock().insert(OTHER_EVENT);
    let seen = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.push(KNOWN);
        let word = PLAYER_EXPLORED_ZONES_1 + 1;
        let explored = p.fields.get_u32(word);
        let unexplored = check(m, 32, None, Some(g));
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(word, explored | 1 << 5);
        (
            (20..=31)
                .map(|id| check(m, id, None, Some(g)))
                .collect::<Vec<_>>(),
            unexplored,
            check(m, 32, None, Some(g)),
        )
    })
    .await;
    use Truth::*;
    assert_eq!(
        seen.0,
        vec![
            Met, Met, Met, Met, Unmet, Met, Met, Met, Unmet, Met, Unmet, Met
        ]
    );
    assert_eq!((seen.1, seen.2), (Unmet, Met), "the area explored");
}

/// Thistle Tea's zero maxima: health of no maximum isn't judged; mana of
/// none is full (a warrior's).
#[tokio::test]
async fn zero_maxima_are_unknown_health_and_full_mana() {
    let (realm, _a, g) = setup().await;
    let boar = creature(BOAR);
    let seen = on_map(&realm, move |m| {
        let f = m.unit_fields_mut(boar).unwrap();
        f.set_u32(UNIT_FIELD_HEALTH, 0);
        f.set_u32(UNIT_FIELD_MAXHEALTH, 0);
        (
            check(m, 40, None, Some(boar)),
            m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_MAXPOWER1),
            check(m, 41, None, Some(g)),
        )
    })
    .await;
    assert_eq!(seen, (Truth::Unknown, 0, Truth::Met));
}

/// Thistle Tea's noncombat pet: of its own entry, or any for 0; not of
/// another; none, none; a target with no companions unknown.
#[tokio::test]
async fn a_companion_is_of_its_entry_or_any() {
    let (realm, _a, g) = setup().await;
    let boar = creature(BOAR);
    let seen = on_map(&realm, move |m| {
        let none = check(m, 51, None, Some(g));
        m.players.get_mut(&g).unwrap().owned.critter = Some(boar);
        (
            [50, 51, 52].map(|id| check(m, id, None, Some(g))),
            none,
            check(m, 51, None, Some(boar)),
        )
    })
    .await;
    use Truth::*;
    assert_eq!(seen, ([Met, Met, Unmet], Unmet, Unknown));
}
