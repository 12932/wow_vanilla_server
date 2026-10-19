//! Phase 3's combat tests, part three (Thistle Tea's `hostility_test.exs`):
//! who regards whom how, by faction templates, standings, war and forced
//! reactions; who may attack whom, by flags, PvP, duels and free for all;
//! whom an area spell may hit; and whom one may help.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::misc::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{self, Regard};
use crate::game::map::player::{PLAYER_FLAGS_FFA_PVP, UNIT_FLAG_PVP};
use crate::game::map::reputation::{self, FLAG_AT_WAR, Standing};
use crate::game::map::{Map, MapKey, spells};
use crate::game::registry::Team;
use crate::game::tests_p3_combat::{START, spawn};

const ALLIANCE: u32 = 1;
const HORDE: u32 = 2;
const DEFIAS: u32 = 17;
const FRIENDLY_DEFIAS: u32 = 99;
/// Neutral beasts whose faction (29) keeps standings, and one whose
/// faction (30) doesn't.
const WOLF: u32 = 32;
const WOLF_PLAIN: u32 = 34;
/// Guards of contested ground, with and without standings.
const GUARD: u32 = 33;
const GUARD_PLAIN: u32 = 36;
const NEUTRAL: u32 = 7;
const FRIENDLY: u32 = 35;

/// The faction with standings.
const BEASTS: u32 = 29;

/// A creature of each template: its entry is 3800 and the template.
fn of(template: u32) -> u32 {
    3800 + template
}

const HEAL: u32 = 95_400;

#[allow(clippy::too_many_arguments)]
fn faction(
    id: u32,
    faction: u32,
    flags: u32,
    ours: u32,
    friend_mask: u32,
    enemy_mask: u32,
    enemies: [u32; 4],
    friends: [u32; 4],
) -> FactionTemplate {
    FactionTemplate {
        id,
        faction,
        flags,
        ours,
        friend_mask,
        enemy_mask,
        enemies,
        friends,
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let none = [0; 4];
    for t in [
        faction(ALLIANCE, 1, 72, 3, 2, 12, none, none),
        faction(HORDE, 2, 72, 5, 4, 10, none, none),
        faction(DEFIAS, 15, 1, 8, 0, 1, none, [15, 0, 0, 0]),
        faction(FRIENDLY_DEFIAS, 99, 0, 8, 0, 1, none, [15, 0, 0, 0]),
        faction(WOLF, BEASTS, 16, 0, 0, 0, [28, 0, 0, 0], none),
        faction(WOLF_PLAIN, 30, 16, 0, 0, 0, [28, 0, 0, 0], none),
        faction(GUARD, BEASTS, 0x1000, 0, 0, 0, [28, 0, 0, 0], none),
        faction(GUARD_PLAIN, 30, 0x1000, 0, 0, 0, [28, 0, 0, 0], none),
        faction(NEUTRAL, 7, 0, 0, 0, 0, none, none),
        faction(FRIENDLY, 31, 0, 0, 1, 0, none, [31, 0, 0, 0]),
    ] {
        d.faction_templates.insert(t.id, t);
        let entry = of(t.id);
        d.creature_templates.insert(
            entry,
            CreatureTemplate {
                entry,
                name: format!("Creature {entry}"),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                level_min: 20,
                level_max: 20,
                faction: t.id,
                unit_class: 1,
                health_multiplier: 1.0,
                damage_multiplier: 1.0,
                base_attack_time: 2000,
                detection_range: 0.1,
                ..Default::default()
            },
        );
    }
    d.misc.factions.insert(
        BEASTS,
        FactionDef {
            id: BEASTS,
            index: 10,
            name: "Beasts".into(),
            variants: [
                (u32::MAX, 0, 0, 0),
                (0, 0, 0, 0),
                (0, 0, 0, 0),
                (0, 0, 0, 0),
            ],
            ..Default::default()
        },
    );
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut heal = SpellTemplate {
        id: HEAL,
        name: "Heal".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    heal.effects[0] = SpellEffect {
        effect: 10,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 21,
        ..Default::default()
    };
    d.spells.spells.insert(HEAL, heal);
    d
}

/// Three players at the start: Alice, Bob and Carol, all human.
async fn three() -> (
    std::sync::Arc<crate::game::Realm>,
    [Client; 3],
    [Guid; 3],
    crate::game::map::MapHandle,
) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let mut b = Client::connect(&realm, "B");
    let mut c = Client::connect(&realm, "C");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let gc = c.enter("Carol", Race::Human, Class::Warrior).await;
    for x in [&a, &b, &c] {
        x.heartbeat(START.0, START.1, START.2).await;
    }
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, [a, b, c], [ga, gb, gc], map)
}

/// `g`'s standing with the beasts: `standing` from neutral's start, and at
/// war or not.
fn stand(m: &mut Map, g: Guid, standing: i32, war: bool) {
    let p = m.players.get_mut(&g).unwrap();
    p.row.state.reputation.retain(|s| s.faction != BEASTS);
    p.row.state.reputation.push(Standing {
        faction: BEASTS,
        standing,
        flags: if war { FLAG_AT_WAR } else { 0 },
    });
}

/// A reaction `rank` forced on `g` with `faction`.
fn force(m: &mut Map, g: Guid, faction: u32, rank: i32) {
    crate::game::tests_p3_combat::aura_of(
        m,
        g,
        95_410 + faction,
        crate::game::map::auras::AURA_FORCE_REACTION,
        faction as i32,
        rank,
    );
}

fn unforce(m: &mut Map, g: Guid, faction: u32) {
    crate::game::map::auras::remove_spell(m, g, 95_410 + faction, None);
}

/// Make `g` of the Horde.
fn horde(m: &mut Map, g: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    p.team = Team::Horde;
    p.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, HORDE);
}

fn flag(m: &mut Map, g: Guid, on: bool) {
    m.unit_fields_mut(g)
        .unwrap()
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, on);
}

fn ffa(m: &mut Map, g: Guid, on: bool) {
    m.players
        .get_mut(&g)
        .unwrap()
        .fields
        .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_FFA_PVP, on);
}

fn duel(m: &mut Map, a: Guid, b: Guid) {
    use crate::game::map::duel::{Duel, Phase};
    let arbiter = Guid::new(0xF110_0000_0000_0001);
    for (me, them, initiator) in [(a, b, true), (b, a, false)] {
        m.players.get_mut(&me).unwrap().duel = Some(Duel {
            opponent: them,
            flag: arbiter,
            phase: Phase::Fighting,
            initiator,
            out_of_bounds_since: None,
            started: None,
        });
    }
}

#[test]
fn an_explicit_friend_faction_outweighs_the_masks() {
    let d = data();
    let (friendly, defias) = (
        &d.faction_templates[&FRIENDLY_DEFIAS],
        &d.faction_templates[&DEFIAS],
    );
    assert!(friendly.friendly_to(defias));
    assert!(!friendly.hostile_to(defias));
}

#[tokio::test]
async fn a_players_standing_and_war_decide_how_a_faction_with_standings_regards_them() {
    let (_realm, _c, [ga, _, _], map) = three().await;
    let r = map
        .query(move |m| {
            let wolf = spawn(m, ga, of(WOLF), 20, 3.0);
            let plain = spawn(m, ga, of(WOLF_PLAIN), 20, 3.0);
            let regard = |m: &Map, a: Guid, b: Guid| combat::regard(m, a, b);
            let rank = |m: &Map, a: Guid, b: Guid| reputation::reaction_rank(m, a, b);
            let mut out = Vec::new();
            // Neutral, at war: the player is hostile, the beasts neutral.
            stand(m, ga, 0, true);
            out.push((regard(m, ga, wolf), regard(m, wolf, ga)));
            // Hostile; honored.
            stand(m, ga, -5000, true);
            out.push((regard(m, ga, wolf), regard(m, wolf, ga)));
            stand(m, ga, 10_000, false);
            out.push((regard(m, ga, wolf), regard(m, wolf, ga)));
            let honored = (rank(m, wolf, ga), rank(m, ga, wolf));
            // Honored but at war: no friendlier than neutral to the beasts.
            stand(m, ga, 10_000, true);
            out.push((regard(m, ga, wolf), regard(m, wolf, ga)));
            stand(m, ga, 42_500, true);
            let exalted_war = (rank(m, wolf, ga), rank(m, ga, wolf));
            // A forced reaction outweighs standing and war, both ways, with
            // or without standings.
            stand(m, ga, -5000, true);
            force(m, ga, 30, RANK_FRIENDLY as i32);
            out.push((regard(m, ga, plain), regard(m, plain, ga)));
            unforce(m, ga, 30);
            stand(m, ga, 10_000, false);
            force(m, ga, 30, RANK_HOSTILE as i32);
            out.push((regard(m, ga, plain), regard(m, plain, ga)));
            unforce(m, ga, 30);
            stand(m, ga, 0, true);
            force(m, ga, BEASTS, RANK_REVERED as i32);
            let forced = (rank(m, wolf, ga), rank(m, ga, wolf));
            unforce(m, ga, BEASTS);
            (out, honored, exalted_war, forced)
        })
        .await
        .unwrap();
    let (out, honored, exalted_war, forced) = r;
    use Regard::*;
    assert_eq!(
        out,
        vec![
            (Hostile, Neutral),
            (Hostile, Hostile),
            (Friendly, Friendly),
            (Hostile, Neutral),
            (Friendly, Friendly),
            (Hostile, Hostile),
        ]
    );
    assert_eq!(honored, (Some(RANK_HONORED), Some(RANK_FRIENDLY)));
    assert_eq!(exalted_war, (Some(RANK_NEUTRAL), Some(RANK_HOSTILE)));
    assert_eq!(forced, (Some(RANK_REVERED), Some(RANK_REVERED)));
}

#[tokio::test]
async fn contested_guards_turn_on_a_contested_player_unless_a_reaction_is_forced() {
    let (_realm, _c, [ga, _, _], map) = three().await;
    let (contested, forced) = map
        .query(move |m| {
            let guard = spawn(m, ga, of(GUARD), 20, 3.0);
            let plain = spawn(m, ga, of(GUARD_PLAIN), 20, 3.0);
            stand(m, ga, 10_000, false);
            m.players.get_mut(&ga).unwrap().fields.set_flag(
                PLAYER_FLAGS,
                crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
                true,
            );
            let contested = (combat::regard(m, ga, guard), combat::regard(m, guard, ga));
            force(m, ga, 30, RANK_FRIENDLY as i32);
            let forced = (combat::regard(m, ga, plain), combat::regard(m, plain, ga));
            (contested, forced)
        })
        .await
        .unwrap();
    assert_eq!(contested, (Regard::Hostile, Regard::Hostile));
    assert_eq!(forced, (Regard::Friendly, Regard::Friendly));
}

#[tokio::test]
async fn immunity_flags_keep_players_and_creatures_apart_each_on_its_own_side() {
    let (_realm, _c, [ga, _, _], map) = three().await;
    let r = map
        .query(move |m| {
            let pet = spawn(m, ga, of(DEFIAS), 20, 3.0);
            let ally = spawn(m, ga, of(ALLIANCE), 20, 3.0);
            let controlled = spawn(m, ga, of(ALLIANCE), 20, 3.0);
            m.creature_mut(controlled).unwrap().owner = Some(ga);
            let set = |m: &mut Map, flags: u32| {
                m.unit_fields_mut(pet)
                    .unwrap()
                    .set_u32(UNIT_FIELD_FLAGS, flags);
            };
            let can = |m: &Map, a: Guid, b: Guid| combat::can_attack(m, a, b);
            // A companion: immune to both sides.
            set(m, 0x300);
            let companion = [
                can(m, ga, pet),
                can(m, ally, pet),
                can(m, pet, ga),
                can(m, pet, ally),
            ];
            // Immune to players: players and theirs can't, creatures can.
            set(m, 0x100);
            let to_players = [can(m, ga, pet), can(m, controlled, pet), can(m, ally, pet)];
            // Immune to creatures: the other way about.
            set(m, 0x200);
            let to_npcs = [can(m, ga, pet), can(m, controlled, pet), can(m, ally, pet)];
            (companion, to_players, to_npcs)
        })
        .await
        .unwrap();
    assert_eq!(r.0, [false; 4]);
    assert_eq!(r.1, [false, false, true]);
    assert_eq!(r.2, [true, true, false]);
}

#[tokio::test]
async fn an_enemy_player_must_be_flagged_a_friend_never_and_free_for_all_spares_ones_group() {
    let (_realm, [mut a, mut b, _c], [ga, gb, gc], map) = three().await;
    let r = map
        .query(move |m| {
            horde(m, gb);
            let can = |m: &Map, x: Guid, y: Guid| combat::can_attack(m, x, y);
            let hostile = reputation::reaction_rank(m, ga, gb) == Some(RANK_HOSTILE);
            let unflagged = can(m, ga, gb);
            flag(m, ga, true);
            let only_attacker = can(m, ga, gb);
            flag(m, ga, false);
            flag(m, gb, true);
            let target_flagged = can(m, ga, gb);
            // Both flagged, the same side: no.
            flag(m, ga, true);
            flag(m, gc, true);
            let friends = can(m, ga, gc);
            flag(m, ga, false);
            flag(m, gc, false);
            // Free for all: a faction mate too, both in it.
            ffa(m, ga, true);
            ffa(m, gc, true);
            let free = can(m, ga, gc);
            ffa(m, gc, false);
            let half = can(m, ga, gc);
            ffa(m, gc, true);
            (
                hostile,
                unflagged,
                only_attacker,
                target_flagged,
                friends,
                free,
                half,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, false, false, true, false, true, false));
    // Grouped, the free for all spares them.
    let _ = (&mut a, &mut b);
    let (mut a, mut c) = (a, _c);
    group(&mut a, &mut c, "Carol").await;
    let grouped = map
        .query(move |m| combat::can_attack(m, ga, gc))
        .await
        .unwrap();
    assert!(!grouped, "a group mate is spared");
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    leader
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
}

#[tokio::test]
async fn a_player_attacks_a_neutral_creature_only_where_its_faction_keeps_no_standing_or_war_or_force_allows()
 {
    let (_realm, _c, [ga, _, _], map) = three().await;
    let r = map
        .query(move |m| {
            let friendly = spawn(m, ga, of(FRIENDLY), 20, 3.0);
            let wolf = spawn(m, ga, of(WOLF), 20, 3.0);
            let plain = spawn(m, ga, of(WOLF_PLAIN), 20, 3.0);
            let neutral = spawn(m, ga, of(NEUTRAL), 20, 3.0);
            let pet = spawn(m, ga, of(ALLIANCE), 20, 3.0);
            m.creature_mut(pet).unwrap().owner = Some(ga);
            let can = |m: &Map, x: Guid, y: Guid| combat::can_attack(m, x, y);
            let friendly_neutral = can(m, ga, friendly);
            stand(m, ga, 0, false);
            let peace = can(m, ga, wolf);
            stand(m, ga, 0, true);
            let war = can(m, ga, wolf);
            stand(m, ga, 5_000, false);
            force(m, ga, BEASTS, RANK_NEUTRAL as i32);
            let forced = can(m, ga, wolf);
            let creatures = can(m, plain, neutral);
            let pets = can(m, pet, plain);
            (friendly_neutral, peace, war, forced, creatures, pets)
        })
        .await
        .unwrap();
    assert_eq!(r, (false, false, true, true, false, true));
}

#[tokio::test]
async fn no_one_attacks_the_unattackable_the_unselectable_or_a_passenger_on_a_flight() {
    let (_realm, _c, [ga, gb, _], map) = three().await;
    let r = map
        .query(move |m| {
            let defias = spawn(m, ga, of(DEFIAS), 20, 3.0);
            let mut out = Vec::new();
            for flags in [0x2, 0x1_0000, 0x0200_0000, 0x0010_0000] {
                m.unit_fields_mut(defias)
                    .unwrap()
                    .set_u32(UNIT_FIELD_FLAGS, flags);
                out.push(combat::can_attack(m, ga, defias));
            }
            m.unit_fields_mut(defias)
                .unwrap()
                .set_u32(UNIT_FIELD_FLAGS, 0);
            let before = combat::can_attack(m, defias, ga);
            m.unit_fields_mut(ga)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, 0x0010_0000, true);
            let flying = combat::can_attack(m, defias, ga);
            // A friend may still help one in flight.
            let helped = combat::can_assist(m, gb, ga) && !combat::can_attack(m, gb, ga);
            m.unit_fields_mut(ga)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, 0x0010_0000, false);
            (out, before, flying, helped)
        })
        .await
        .unwrap();
    assert_eq!(r.0, vec![false; 4]);
    assert!(r.1);
    assert!(!r.2);
    assert!(r.3);
}

#[tokio::test]
async fn an_unflagged_players_area_spares_players_but_not_creatures_or_a_duels_opponent() {
    let (_realm, _c, [ga, gb, gc], map) = three().await;
    let r = map
        .query(move |m| {
            let defias = spawn(m, ga, of(DEFIAS), 20, 3.0);
            horde(m, gb);
            flag(m, gb, true);
            let area = |m: &Map, x: Guid, y: Guid| combat::can_attack_in_area(m, x, y);
            let unflagged = (combat::can_attack(m, ga, gb), area(m, ga, gb));
            flag(m, ga, true);
            let flagged = area(m, ga, gb);
            flag(m, ga, false);
            let creature = area(m, ga, defias);
            duel(m, ga, gc);
            let dueling = (combat::can_attack(m, ga, gc), area(m, ga, gc));
            let rank = reputation::reaction_rank(m, ga, gc);
            (unflagged, flagged, creature, dueling, rank)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (true, false), "a single blow may; an area mayn't");
    assert!(r.1);
    assert!(r.2);
    assert_eq!(r.3, (true, true));
    assert_eq!(r.4, Some(RANK_HOSTILE), "duelists are enemies");
}

#[tokio::test]
async fn no_one_outside_a_duel_helps_its_fighters() {
    let (_realm, _c, [ga, gb, gc], map) = three().await;
    let r = map
        .query(move |m| {
            for g in [ga, gb, gc] {
                m.players.get_mut(&g).unwrap().spells.push(HEAL);
            }
            let check = |m: &mut Map, caster: Guid, target: Guid| {
                let data = m.realm.data.clone();
                let s = data.spells.get(HEAL).unwrap();
                let mods = spells::mods::for_spell(m, caster, s);
                let t = spells::Targets {
                    unit: Some(target),
                    ..Default::default()
                };
                spells::checks::check(m, caster, s, &t, None, false, &mods)
            };
            let before = check(m, gb, ga);
            duel(m, ga, gc);
            let outside = check(m, gb, ga);
            let own = check(m, ga, ga);
            let out_of = check(m, ga, gb);
            m.players.get_mut(&ga).unwrap().duel = None;
            m.players.get_mut(&gc).unwrap().duel = None;
            let after = check(m, gb, ga);
            (before, outside, own, out_of, after)
        })
        .await
        .unwrap();
    let bad = Some(spells::checks::fail::BAD_TARGETS);
    assert_eq!(r, (None, bad, None, bad, None));
}
