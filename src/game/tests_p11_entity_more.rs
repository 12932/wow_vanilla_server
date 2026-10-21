//! Round 11's last entity rows (docs/parity/matrix.csv): a feared
//! player's control and runs, a formation's leader in a fight or stunned,
//! a script's evade, a pet put away by a teleport until its
//! acknowledgement, a spell cast with points of its own, a triggered
//! spell's hit by its family, the power logs and who hears them, and a
//! disarm.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::{
    AiEvent, EF_REPEATABLE, EVENT_EVADE, GROUP_FORMATION, GroupMember, SF_ABORT_ON_FAILURE,
    ScriptStep,
};
use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::{CreatureTemplate, GameData, ItemTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, *};
use crate::game::map::combat::dist;
use crate::game::map::creature::{MOVEMENT_RANDOM, Motion};
use crate::game::map::player::{UNIT_FLAG_IN_COMBAT, UNIT_FLAG_STUNNED};
use crate::game::map::spells::{self, mods, outcome};
use crate::game::map::{Map, ai, creature_groups, player_motion, scripts};
use crate::game::tests_p3_cast_kit as cast_kit;
use crate::game::tests_p3_combat::{self as combat_kit, BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::{bob, gets_frame, on, step};
use crate::game::tests_p3_entity_objects::until;
use crate::game::tests_p3_spells_kit as spell_kit;
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_parity_motion as motion;
use crate::game::tests_pets;

/// tests_parity_motion's fear and stun, ten seconds each.
const FEAR: u32 = 92_001;
const STUN: u32 = 92_004;

/// Whether `g` may move, as an `SMSG_CLIENT_CONTROL_UPDATE` for it says.
fn control_of(m: &ServerOpcodeMessage, g: Guid) -> Option<bool> {
    match m {
        ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(c) if c.guid == g => Some(c.allow_movement),
        _ => None,
    }
}

/// An `SMSG_MONSTER_MOVE` of `g`'s: its stop if `stop`, else a run.
fn move_of(m: &ServerOpcodeMessage, g: Guid, stop: bool) -> Option<()> {
    match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == g
                && matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) == stop =>
        {
            Some(())
        }
        _ => None,
    }
}

/// Thistle Tea's "player control uses its explicit owner and movement
/// reaches owner and observers" (event_sink/movement_test.exs:299): a
/// player feared by the wolf is told alone that her client's control is
/// taken; her run reaches her and Bob beside her, and so does its stop
/// when the fear ends with a run under way; Bob is told nothing of her
/// control, given back or taken (mangos-classic's `UpdateClientControl`
/// to the client controlling the unit, the spline to everyone who sees
/// it).
#[tokio::test]
async fn a_feared_players_control_is_hers_alone_and_her_run_and_stop_reach_her_watchers() {
    let (realm, mut a, ga, map) = motion::enter(motion::wolf_world()).await;
    let (mut b, _gb) = bob(&realm, ga).await;
    let wolf = motion::wolf();
    a.sees(wolf).await;
    a.clear();
    b.clear();
    on(&map, move |m| motion::apply(m, wolf, ga, FEAR)).await;
    let allowed = a.wait_for("control taken", |m| control_of(m, ga)).await;
    assert!(!allowed, "her client may not move her");
    a.wait_for("her run", |m| move_of(m, ga, false)).await;
    b.wait_for("her run, watched", |m| move_of(m, ga, false))
        .await;
    // A fresh run under way as the fear ends.
    let running = on(&map, move |m| {
        let now = m.now;
        if let Some(d) = m.players.get_mut(&ga).and_then(|p| p.driven.as_mut()) {
            d.walk = None;
            d.next_move_at = now;
        }
        player_motion::update(m);
        let running = m.players[&ga]
            .driven
            .as_ref()
            .is_some_and(|d| d.walk.is_some());
        auras::remove_spell(m, ga, FEAR, None);
        running
    })
    .await;
    assert!(running, "running as it ends");
    a.wait_for("her stop", |m| move_of(m, ga, true)).await;
    b.wait_for("her stop, watched", |m| move_of(m, ga, true))
        .await;
    let allowed = a.wait_for("control back", |m| control_of(m, ga)).await;
    assert!(allowed, "her client moves her again");
    let told = b
        .gets(Duration::from_millis(200), |m| control_of(m, ga))
        .await;
    assert_eq!(told, None, "Bob is told nothing of her control");
}

/// tests_parity_motion's walker, a leader (spawn 90) and its follower
/// (91) three yards behind it, in formation.
const WALKER: u32 = 5003;

fn walker() -> CreatureTemplate {
    CreatureTemplate {
        entry: WALKER,
        name: "Walker".into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 2,
        level_max: 2,
        faction: 14,
        unit_class: 1,
        health_multiplier: 100.0,
        damage_multiplier: 0.01,
        base_attack_time: 2000,
        detection_range: 5.0,
        ..Default::default()
    }
}

fn formation_world() -> GameData {
    let mut d = motion::wolf_world();
    d.creature_templates.insert(WALKER, walker());
    let spawns = d.creature_spawns.get_mut(&0).unwrap();
    spawns.push(motion::spawn(90, WALKER, motion::at(20.0, -40.0)));
    spawns.push(motion::spawn(91, WALKER, motion::at(17.0, -40.0)));
    let member = |guid, distance, angle| GroupMember {
        guid,
        distance,
        angle,
        flags: GROUP_FORMATION,
    };
    d.scripts.groups.insert(
        90,
        vec![member(90, 0.0, 0.0), member(91, 3.0, std::f32::consts::PI)],
    );
    d.scripts.group_of.insert(90, 90);
    d.scripts.group_of.insert(91, 90);
    d
}

/// `g`'s walk under way: its spline id.
fn walking(m: &Map, g: Guid) -> Option<u32> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.spline_id),
        Motion::Idle { .. } => None,
    }
}

/// Thistle Tea's "reads published leader motion and excludes combat and
/// other worlds" (formation_environment_test.exs:23): a stunned leader
/// takes no step of its wander, and so leads no one; freed, its next
/// wander takes its follower along. A leader in a fight leads no one where
/// it chases: its follower keeps standing (Thistle Tea's `leader_ready?`
/// false in combat or stunned; mangos-classic's `FormationMovementGenerator`
/// following only a leader out of combat). Another world is another map
/// in wvs, whose leader can't lead this one's.
#[tokio::test]
async fn a_formations_leader_stunned_or_fighting_leads_no_one() {
    let (_realm, mut a, ga, map) = motion::enter(formation_world()).await;
    let (l, f) = (guid::unit(WALKER, 90), guid::unit(WALKER, 91));
    a.sees(l).await;
    a.sees(f).await;
    let wander = move |m: &mut Map| {
        let now = m.now;
        for g in [l, f] {
            m.creature_mut(g).unwrap().motion = Motion::Idle { until: None };
        }
        let c = m.creature_mut(l).unwrap();
        c.movement_type = MOVEMENT_RANDOM;
        c.wander_distance = 5.0;
        c.motion = Motion::Idle { until: Some(now) };
        ai::update_creatures(m);
        (walking(m, l).is_some(), walking(m, f).is_some())
    };
    let (stunned, held, freed) = on(&map, move |m| {
        motion::apply(m, l, l, STUN);
        let stunned = m
            .unit_fields(l)
            .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_STUNNED));
        let held = wander(m);
        auras::remove_spell(m, l, STUN, None);
        (stunned, held, wander(m))
    })
    .await;
    assert!(stunned, "the leader stunned");
    assert_eq!(held, (false, false), "neither walks");
    assert_eq!(freed, (true, true), "freed, it leads its follower");
    let out = on(&map, move |m| {
        for g in [l, f] {
            m.creature_mut(g).unwrap().motion = Motion::Idle { until: None };
        }
        ai::attacked(m, l, ga);
        let fighting = (
            m.creature(l).unwrap().combat.attacking,
            m.creature(f).unwrap().combat.attacking,
        );
        let now = m.now;
        let c = m.creature_mut(l).unwrap();
        let from = c.position;
        c.walk(vec![from, motion::at(20.0, -20.0)], true, now, 7);
        creature_groups::leader_walked(m, l);
        (fighting, walking(m, l), walking(m, f))
    })
    .await;
    assert_eq!(out.0, (true, false), "the leader fights, its follower not");
    assert_eq!(out.1, Some(7), "the leader chases");
    assert_eq!(out.2, None, "its follower keeps standing");
}

/// The brute's evade script: it kneels (stand state 8).
const KNEEL_ON_EVADE: u32 = 98_640;

fn evade_world() -> GameData {
    let mut d = combat_kit::data();
    d.scripts.ai_scripts.insert(
        KNEEL_ON_EVADE,
        vec![ScriptStep {
            id: KNEEL_ON_EVADE,
            command: 28,
            datalong: [8, 0, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_events.insert(
        BRUTE,
        vec![AiEvent {
            kind: EVENT_EVADE,
            chance: 100,
            flags: EF_REPEATABLE,
            actions: [KNEEL_ON_EVADE, 0, 0],
            ..Default::default()
        }],
    );
    d
}

/// A creature's fight as its fields and lists have it: attacking, threat
/// listed, its target, its combat flag, and whether it evades.
fn fight_of(m: &Map, g: Guid) -> (bool, bool, Guid, bool, bool) {
    let c = m.creature(g).unwrap();
    (
        c.combat.attacking,
        !c.threat.is_empty(),
        c.fields.get_guid(UNIT_FIELD_TARGET),
        c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
        c.evading,
    )
}

fn stand(m: &Map, g: Guid) -> u8 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_byte(UNIT_FIELD_BYTES_1, 0))
}

/// Thistle Tea's "scripted evade runs the shared combat reset and its
/// callbacks" (mob/movement_inform_test.exs:50): a brute fighting Alice
/// that a script makes evade (command 33) leaves its fight, its threat
/// list empty, its target none and its combat flag off, and its EventAI
/// evade event has run (it kneels); a dead one does nothing (vmangos's
/// `SCRIPT_COMMAND_ENTER_EVADE_MODE` on a living creature,
/// mangos-classic's `EnterEvadeMode` with its `CombatStop` and
/// `DeleteThreatList`). wvs heals one that regenerates as it leashes, not
/// here asserted.
#[tokio::test]
async fn a_scripted_evade_ends_the_fight_and_runs_the_evade_event_of_the_living() {
    let (_realm, _a, ga, map) = setup_with(evade_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let g = spawn(m, ga, BRUTE, 20, 5.0);
        ai::attacked(m, g, ga);
        m.unit_fields_mut(g).unwrap().set_u32(UNIT_FIELD_HEALTH, 5);
        let fighting = fight_of(m, g);
        let ran = scripts::execute(m, &step(33, [0; 4]), g, Guid::zero());
        let evaded = (fight_of(m, g), stand(m, g));
        let dead = spawn(m, ga, BRUTE, 20, 6.0);
        ai::attacked(m, dead, ga);
        ai::die(m, dead, ga);
        let corpse = scripts::execute(
            m,
            &ScriptStep {
                data_flags: SF_ABORT_ON_FAILURE,
                ..step(33, [0; 4])
            },
            dead,
            Guid::zero(),
        );
        let c = m.creature(dead).unwrap();
        let left = (c.alive(), c.evading, stand(m, dead));
        (fighting, ran, evaded, corpse, left)
    })
    .await;
    let (fighting, ran, evaded, corpse, left) = out;
    assert_eq!(fighting, (true, true, ga, true, false), "fighting Alice");
    assert!(ran, "the script's evade");
    assert_eq!(
        evaded,
        ((false, false, Guid::zero(), false, true), 8),
        "out of its fight, home bound, its evade event run"
    );
    assert!(!corpse, "a corpse doesn't evade");
    assert!(!left.0 && !left.1, "dead, not evading");
    assert_ne!(left.2, 8, "its evade event not run");
}

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

/// Thistle Tea's "waits for the near teleport acknowledgement before
/// restoring a suspended pet" (player_test.exs:463): a hunter fighting the
/// wolf, her boar out, sent 120 yards within the map is out of her fight;
/// her boar goes, her client told its bar is empty, and
/// stays away through the map's updates until she acknowledges the
/// teleport; then it is back at her side, of its number (mangos-classic's
/// `TeleportTo`: `UnsummonPetTemporaryIfAny` past 50 yards, and its
/// `ResummonPetTemporaryUnSummonedIfAny` at `HandleMoveTeleportAckOpcode`).
#[tokio::test]
async fn a_far_teleport_within_the_map_puts_the_pet_away_until_it_is_acknowledged() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed_with(tests_pets::data()).await;
    h.sees(pet).await;
    let (number, fighting) = on(&map, move |m| {
        ai::attacked(m, wolf, gh);
        m.players
            .get_mut(&gh)
            .unwrap()
            .fields
            .set_guid(UNIT_FIELD_TARGET, wolf);
        let n = m
            .creature(pet)
            .unwrap()
            .fields
            .get_u32(UNIT_FIELD_PETNUMBER);
        (n, in_combat(m, gh))
    })
    .await;
    assert!(fighting, "in her fight");
    h.clear();
    let away = on(&map, move |m| {
        let here = m.players[&gh].position();
        let to = Vector3d {
            x: here.x + 120.0,
            ..here
        };
        m.teleport(gh, m.key, to, 0.0);
        let p = &m.players[&gh];
        (
            in_combat(m, gh),
            p.owned.pet,
            p.owned.unsummoned,
            m.creature(pet).is_some(),
            p.teleport.is_some(),
        )
    })
    .await;
    assert_eq!(
        away,
        (false, None, true, false, true),
        "out of her fight, her boar away, the teleport unacknowledged"
    );
    h.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    h.wait_for("its bar emptied", |m| match m {
        ServerOpcodeMessage::SMSG_PET_SPELLS(s) if s.pet.is_zero() => Some(()),
        _ => None,
    })
    .await;
    // Unacknowledged, it stays away update after update.
    let from = on(&map, |m| m.stats.updates).await;
    until(&map, "five updates", move |m| m.stats.updates >= from + 5).await;
    let still = on(&map, move |m| {
        let p = &m.players[&gh];
        (p.owned.pet, p.owned.unsummoned)
    })
    .await;
    assert_eq!(still, (None, true), "away until acknowledged");
    h.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: gh,
        movement_counter: h.teleport_order(),
        time: Default::default(),
    })
    .await;
    until(&map, "the boar back", move |m| {
        m.players[&gh]
            .owned
            .pet
            .is_some_and(|p| m.creature(p).is_some_and(|c| c.alive()))
    })
    .await;
    let (apart, back_number) = on(&map, move |m| {
        let p = &m.players[&gh];
        let c = m.creature(p.owned.pet.unwrap()).unwrap();
        (
            dist(c.position, p.position()),
            c.fields.get_u32(UNIT_FIELD_PETNUMBER),
        )
    })
    .await;
    assert!(apart < 5.0, "at her side: {apart}");
    assert_eq!(back_number, number, "her boar");
    h.wait_for("its bar again", |m| match m {
        ServerOpcodeMessage::SMSG_PET_SPELLS(s) if !s.pet.is_zero() => Some(()),
        _ => None,
    })
    .await;
}

/// Three dummy auras on the caster, each of 10 to 19.
const THREE_POINTS: u32 = 98_650;

fn points_world(d: &mut GameData) {
    let mut s = cast_kit::bare(THREE_POINTS, 0);
    s.dmg_class = 0;
    s.duration_index = cast_kit::LASTS_15S;
    for i in 0..3 {
        cast_kit::effect(
            &mut s,
            i,
            SpellEffect {
                effect: 6,
                aura: AURA_DUMMY,
                base_points: 9,
                base_dice: 1,
                die_sides: 10,
                target_a: 1,
                ..Default::default()
            },
        );
    }
    cast_kit::insert(d, [s]);
}

/// Thistle Tea's "custom points preserve unmodified effects and the cached
/// spell" (effect_resolver/custom_spell_test.exs:20): a spell of three
/// effects cast with points of 0 for its first and -15 for its third has
/// those, exactly, every cast; its second rolls its own dice, 10 to 19,
/// not always the same; and the spell's template is as it was
/// (mangos-classic's `CastCustomSpell`, which sets `m_currentBasePoints`
/// only for the points it is given).
#[tokio::test]
async fn a_spell_cast_with_points_keeps_its_other_effects_dice_and_its_template() {
    let realm = cast_kit::fast_realm(points_world);
    let (_c, a, map) = cast_kit::human(&realm, "P11M6", "Pointer", Class::Mage).await;
    let before = format!("{:?}", realm.data.spells.get(THREE_POINTS).unwrap());
    let casts = on(&map, move |m| {
        spell_kit::ready(m, a, 60, &[]);
        let mut casts = Vec::new();
        for _ in 0..40 {
            spells::scripts::cast_with_points(m, a, THREE_POINTS, a, [Some(0), None, Some(-15)]);
            let mut amounts: Vec<(u8, i32)> = spell_kit::holder(m, a, THREE_POINTS)
                .map(|h| h.effects.iter().map(|x| (x.effect, x.amount)).collect())
                .unwrap_or_default();
            amounts.sort_unstable();
            casts.push(amounts);
            auras::remove_spell(m, a, THREE_POINTS, None);
        }
        casts
    })
    .await;
    let mut rolled = Vec::new();
    for amounts in &casts {
        let [(0, first), (1, second), (2, third)] = amounts[..] else {
            panic!("three effects: {amounts:?}");
        };
        assert_eq!((first, third), (0, -15), "its points");
        assert!((10..=19).contains(&second), "its own dice: {second}");
        rolled.push(second);
    }
    rolled.sort_unstable();
    rolled.dedup();
    assert!(rolled.len() > 1, "rolled, not fixed: {rolled:?}");
    let after = format!("{:?}", realm.data.spells.get(THREE_POINTS).unwrap());
    assert_eq!(before, after, "its template as it was");
}

/// Two bolts of a hundred, a mage's (family 3, its flag 1) and a
/// warlock's (family 5, its flag 1); a hundred percent less spell hit; and
/// a hundred more chance to hit for the mage's spells of flag 1 (a flat
/// modifier of `OP_RESIST_MISS_CHANCE`).
const FIREBOLT: u32 = 98_660;
const SHADOWBOLT: u32 = 98_661;
const HIT_PENALTY: u32 = 98_662;
const MAGE_FOCUS: u32 = 98_663;

fn hit_world(d: &mut GameData) {
    let bolt = |id: u32, school: u32, family: u32| {
        let mut s = cast_kit::bare(id, school);
        s.family_name = family;
        s.family_flags = 1;
        cast_kit::effect(&mut s, 0, cast_kit::damage(100));
        s
    };
    let mut penalty = cast_kit::bare(HIT_PENALTY, 0);
    penalty.dmg_class = 0;
    penalty.duration_index = cast_kit::LASTS_LONG;
    cast_kit::effect(
        &mut penalty,
        0,
        cast_kit::aura(AURA_MOD_SPELL_HIT_CHANCE, 0, -100, 1),
    );
    let mut focus = cast_kit::bare(MAGE_FOCUS, 0);
    focus.dmg_class = 0;
    focus.family_name = 3;
    focus.duration_index = cast_kit::LASTS_LONG;
    cast_kit::effect(
        &mut focus,
        0,
        SpellEffect {
            item_type: 1,
            ..cast_kit::aura(
                AURA_ADD_FLAT_MODIFIER,
                mods::OP_RESIST_MISS_CHANCE as i32,
                100,
                1,
            )
        },
    );
    cast_kit::insert(
        d,
        [bolt(FIREBOLT, 2, 3), bolt(SHADOWBOLT, 5, 5), penalty, focus],
    );
}

/// Thistle Tea's "foreign triggers match the child family mask using the
/// original hit snapshot" (effect_resolver/triggered_hit_dbc_test.exs:288):
/// a caster a hundred percent less likely to hit with spells, and a
/// hundred more with a mage's of flag 1, has a mage's triggered bolt of
/// that flag land (96%, even levels; no miss told) and a warlock's of the
/// same flag resisted (the least, 1%; its miss told as a resist), the
/// rolls fixed (mangos-classic's `MagicSpellHitChance` with
/// `m_modSpellHitChance` and `SPELLMOD_RESIST_MISS_CHANCE` of the spell's
/// family mask).
#[tokio::test]
async fn a_triggered_spells_hit_takes_only_the_modifiers_of_its_own_family() {
    let realm = cast_kit::fast_realm(hit_world);
    let (mut c, a, map) = cast_kit::human(&realm, "P11M7", "Hitter", Class::Mage).await;
    let out = on(&map, move |m| {
        m.spells.rolls = Some(false);
        spell_kit::ready(m, a, 60, &[]);
        spell_kit::apply(m, a, a, HIT_PENALTY);
        spell_kit::apply(m, a, a, MAGE_FOCUS);
        let kobold = spell_kit::dummy(m, a, 60, 5.0);
        let data = m.realm.data.clone();
        let chance = |m: &Map, id: u32| {
            let s = data.spells.get(id).unwrap();
            outcome::spell_hit_chance(m, a, kobold, s, &mods::for_spell(m, a, s))
        };
        let chances = (chance(m, FIREBOLT), chance(m, SHADOWBOLT));
        let mut hurt = Vec::new();
        for id in [FIREBOLT, SHADOWBOLT] {
            let before = spell_kit::health(m, kobold);
            spells::cast_triggered(m, a, id, Some(kobold));
            hurt.push(before - spell_kit::health(m, kobold));
        }
        (chances, hurt)
    })
    .await;
    let (chances, hurt) = out;
    assert_eq!(chances, (9_600, 100), "the mage's 96%, the warlock's 1%");
    assert!(hurt[0] > 0, "the mage's bolt lands");
    assert_eq!(hurt[1], 0, "the warlock's doesn't");
    assert_eq!(spell_kit::missed(&mut c, SHADOWBOLT).await, 2, "resisted");
    let fire_missed = gets_frame(&mut c, outcome::SMSG_SPELLLOGMISS, |b| {
        b[..4] == FIREBOLT.to_le_bytes()
    })
    .await;
    assert!(!fire_missed, "no miss told of the mage's");
    let fire_go = c
        .wait_for("the mage's go", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == FIREBOLT => {
                Some((g.hits.len(), g.misses.len()))
            }
            _ => None,
        })
        .await;
    assert_eq!(fire_go, (1, 0), "the mage's go: a hit, no miss");
}

/// Fifty mana; fifty drained, half of it back; fifty a second leeched,
/// half of it back.
const ENERGIZE: u32 = 98_670;
const DRAIN: u32 = 98_671;
const LEECH: u32 = 98_672;
const LASTS_3S: u32 = 900;

fn power_world() -> GameData {
    let mut d = combat_kit::data();
    d.spells.durations.insert(
        LASTS_3S,
        SpellDuration {
            base: 3000,
            per_level: 0,
            max: 3000,
        },
    );
    let spell = |id: u32, effect: SpellEffect| {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            school: 5,
            dmg_class: 1,
            family_name: 5,
            family_flags: 1,
            range_index: 1,
            equipped_item_class: -1,
            duration_index: LASTS_3S,
            ..Default::default()
        };
        s.attributes[3] = ATTR_EX3_ALWAYS_HIT;
        s.effects[0] = effect;
        s
    };
    let hit = |effect: u32, aura: u32, target_a: u32| SpellEffect {
        effect,
        aura,
        base_points: 49,
        base_dice: 1,
        die_sides: 1,
        multiple_value: 0.5,
        amplitude: if aura != 0 { 1000 } else { 0 },
        target_a,
        ..Default::default()
    };
    let mut energize = spell(ENERGIZE, hit(30, 0, 1));
    energize.dmg_class = 0;
    energize.attributes[3] = 0;
    for s in [
        energize,
        spell(DRAIN, hit(8, 0, 6)),
        spell(LEECH, hit(6, AURA_PERIODIC_MANA_LEECH, 6)),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// The power logs of Alice's that `c` is told of: her energize, her drain
/// (`SMSG_SPELLLOGEXECUTE`) and her leech's tick, within `within`.
async fn power_logs(c: &mut Client, alice: Guid, within: Duration) -> [bool; 3] {
    let energize = c
        .gets(within, |m| match m {
            ServerOpcodeMessage::SMSG_SPELLENERGIZELOG(l)
                if l.spell == ENERGIZE && l.caster == alice =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    let drain = c
        .gets(within, |m| match m {
            ServerOpcodeMessage::SMSG_SPELLLOGEXECUTE(l)
                if l.spell == DRAIN && l.caster == alice =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    let leech = c
        .gets(within, |m| match m {
            ServerOpcodeMessage::SMSG_PERIODICAURALOG(l)
                if l.spell == LEECH && l.caster == alice =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    [energize.is_some(), drain.is_some(), leech.is_some()]
}

/// Thistle Tea's "resource combat logs reach the owner and observers only
/// in the same world" (power_transfer_test.exs:128): Alice's energize of
/// herself, her drain of a brute's mana and a tick of her mana leech on it
/// are told to her and to Bob beside her, and to no one on another map
/// (mangos-classic's `SendEnergizeSpellLog`, `SendSpellLogExecute` and
/// `SendPeriodicAuraLog`, each to the unit's viewers and itself).
#[tokio::test]
async fn the_power_logs_reach_the_caster_and_her_watchers_but_not_another_map() {
    let realm = harness::realm_with(power_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    a.heartbeat(
        combat_kit::START.0,
        combat_kit::START.1,
        combat_kit::START.2,
    )
    .await;
    let map = realm.registry.online(ga).unwrap().map();
    let (mut b, _gb) = bob(&realm, ga).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Orc, Class::Warrior).await;
    let elsewhere = realm.registry.online(gc).unwrap().map().key;
    assert_ne!(elsewhere, map.key, "Carol on another map");
    let brute = on(&map, move |m| {
        spell_kit::ready(m, ga, 60, &[]);
        let g = spawn(m, ga, BRUTE, 60, 5.0);
        let f = m.unit_fields_mut(g).unwrap();
        f.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
        f.set_u32(UNIT_FIELD_POWER1, 1000);
        f.set_byte(UNIT_FIELD_BYTES_0, 3, 0);
        g
    })
    .await;
    a.sees(brute).await;
    b.sees(brute).await;
    a.clear();
    b.clear();
    c.clear();
    on(&map, move |m| {
        spells::cast_triggered(m, ga, ENERGIZE, Some(ga));
        spells::cast_triggered(m, ga, DRAIN, Some(brute));
        spells::cast_triggered(m, ga, LEECH, Some(brute));
        spell_kit::tick_now(m, brute, LEECH);
    })
    .await;
    assert_eq!(
        power_logs(&mut a, ga, harness::WAIT).await,
        [true; 3],
        "Alice is told"
    );
    assert_eq!(
        power_logs(&mut b, ga, harness::WAIT).await,
        [true; 3],
        "Bob is told"
    );
    assert_eq!(
        power_logs(&mut c, ga, Duration::from_millis(100)).await,
        [false; 3],
        "Carol is told nothing"
    );
}

/// An axe of three seconds, 40 to 60; a disarm of ten seconds.
const SLOW_AXE: u32 = 63_010;
const DISARM: u32 = 98_680;
const LASTS_10S: u32 = 901;

fn disarm_world() -> GameData {
    let mut d = combat_kit::data();
    let mut damages = [(0.0, 0.0, 0); 5];
    damages[0] = (40.0, 60.0, 0);
    d.items.insert(
        SLOW_AXE,
        ItemTemplate {
            entry: SLOW_AXE,
            class: 2,
            subclass: 0,
            name: "Slow Axe".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 3000,
            damages,
            stackable: 1,
            ..Default::default()
        },
    );
    d.spells.durations.insert(
        LASTS_10S,
        SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    let mut s = SpellTemplate {
        id: DISARM,
        name: "Disarm".into(),
        equipped_item_class: -1,
        duration_index: LASTS_10S,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_DISARM,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(DISARM, s);
    d
}

/// A player's main hand as their fields have it: disarmed, its damage and
/// its time.
fn main_hand(m: &Map, g: Guid) -> (bool, f32, f32, u32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED),
        f.get_f32(UNIT_FIELD_MINDAMAGE),
        f.get_f32(UNIT_FIELD_MAXDAMAGE),
        f.get_u32(UNIT_FIELD_BASEATTACKTIME),
    )
}

/// Thistle Tea's "boomerang damage permits all four control outcomes and
/// normal recovery" (effect_resolver/conditional_triggers_dbc_test.exs:36),
/// its disarm: a warrior with a three-second axe disarmed has the
/// disarmed flag, other main-hand damage, and two seconds between
/// swings; when it runs out the flag goes and the axe's damage and time
/// are back; disarmed again and killed, the disarm goes with her death
/// (mangos-classic's `HandleAuraModDisarm`: `UNIT_FLAG_DISARMED`, a
/// player's `SetAttackTime(BASE_ATTACK, BASE_ATTACK_TIME)` and back to
/// the weapon's, `UpdateDamagePhysical`; `RemoveAllAurasOnDeath`).
#[tokio::test]
async fn a_disarm_takes_the_main_hands_damage_and_time_until_it_runs_out_or_she_dies() {
    let (_realm, _a, ga, map) = setup_with(disarm_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        combat_kit::level(m, ga, 60);
        combat_kit::equip(m, ga, SLOW_AXE, 15);
        let armed = main_hand(m, ga);
        spell_kit::apply(m, ga, ga, DISARM);
        let disarmed = main_hand(m, ga);
        m.now += Duration::from_secs(11);
        auras::update(m);
        let expired = (spell_kit::has(m, ga, DISARM), main_hand(m, ga));
        spell_kit::apply(m, ga, ga, DISARM);
        let again = spell_kit::has(m, ga, DISARM);
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        crate::game::map::combat::deal_damage(m, brute, ga, 1_000_000);
        let dead = (
            m.unit_fields(ga)
                .map_or(1, |f| f.get_u32(UNIT_FIELD_HEALTH)),
            spell_kit::has(m, ga, DISARM),
            main_hand(m, ga).0,
        );
        (armed, disarmed, expired, again, dead)
    })
    .await;
    let (armed, disarmed, expired, again, dead) = out;
    assert!(!armed.0, "armed");
    assert_eq!(armed.3, 3000, "the axe's three seconds");
    assert!(disarmed.0, "the disarmed flag");
    assert_ne!((disarmed.1, disarmed.2), (armed.1, armed.2), "other damage");
    assert_eq!(disarmed.3, 2000, "two seconds a swing");
    assert_eq!(expired, (false, armed), "run out: the axe's again");
    assert!(again, "disarmed again");
    assert_eq!(dead, (0, false, false), "dead, the disarm gone");
}
