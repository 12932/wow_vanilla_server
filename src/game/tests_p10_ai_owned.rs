//! Round 10's creature AI tests of owned creatures' own events, ported
//! from Thistle Tea's `test/game/core/ai/companion_scripts_test.exs`,
//! `bt/pet_test.exs` and `bt/scripted_melee_test.exs`
//! (docs/parity/matrix.csv): a guardian, a companion and a creature's pet
//! run their EventAI (mangos-classic's `GuardianAI`, a `CreatureEventAI`),
//! a player's controlled pet and a charmed or possessed creature don't; a
//! companion a script told to stay stays; a guardian's fight starts its
//! scripted one; a charmed creature swings without its script's melee
//! spell.

use std::time::Duration;

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::creature::Motion;
use crate::game::map::pets::{self, Kind};
use crate::game::map::{Map, combat, event_ai};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{WOLF, effect, place, spell, template};

/// A creature whose spawning sets phase 7.
const MARKED: u32 = 3_800;
/// One told to stay by its out-of-combat timer, and one told to follow.
const STAYER: u32 = 3_801;
const FOLLOWER: u32 = 3_802;
/// A guardian with a fight's script: phase 3 as it starts, a timer 25
/// seconds in, phase 9 at its death.
const FIGHTER: u32 = 3_803;

const S_PHASE_7: u32 = 380_001;
const S_STAY: u32 = 380_002;
const S_FOLLOW: u32 = 380_003;
const S_PHASE_3: u32 = 380_004;
const S_PHASE_5: u32 = 380_005;
const S_PHASE_9: u32 = 380_006;

/// A script's melee spell: a mark on its victim.
const MARK: u32 = 95_380;

fn step(command: u32, d1: u32) -> ScriptStep {
    ScriptStep {
        command,
        datalong: [d1, 0, 0, 0],
        ..Default::default()
    }
}

fn event(creature: u32, kind: u32, params: [i32; 4], action: u32) -> AiEvent {
    AiEvent {
        id: creature * 100 + kind,
        creature,
        kind,
        chance: 100,
        params,
        actions: [action, 0, 0],
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = crate::game::tests_pets::data();
    for entry in [MARKED, STAYER, FOLLOWER, FIGHTER] {
        d.creature_templates
            .insert(entry, template(entry, "Owned", 35, 1));
    }
    let s = &mut d.scripts;
    s.ai_events.insert(
        MARKED,
        vec![event(MARKED, EVENT_SPAWNED, [0; 4], S_PHASE_7)],
    );
    let timer = [0, 0, 60_000, 60_000];
    s.ai_events
        .insert(STAYER, vec![event(STAYER, EVENT_TIMER_OOC, timer, S_STAY)]);
    s.ai_events.insert(
        FOLLOWER,
        vec![event(FOLLOWER, EVENT_TIMER_OOC, timer, S_FOLLOW)],
    );
    s.ai_events.insert(
        FIGHTER,
        vec![
            event(FIGHTER, EVENT_AGGRO, [0; 4], S_PHASE_3),
            event(
                FIGHTER,
                EVENT_TIMER_IN_COMBAT,
                [25_000, 25_000, 25_000, 25_000],
                S_PHASE_5,
            ),
            event(FIGHTER, EVENT_DEATH, [0; 4], S_PHASE_9),
        ],
    );
    for (id, steps) in [
        (S_PHASE_7, vec![step(44, 7)]),
        (S_STAY, vec![step(88, u32::from(pets::COMMAND_STAY))]),
        (S_FOLLOW, vec![step(88, u32::from(pets::COMMAND_FOLLOW))]),
        (S_PHASE_3, vec![step(44, 3)]),
        (S_PHASE_5, vec![step(44, 5)]),
        (S_PHASE_9, vec![step(44, 9)]),
    ] {
        s.ai_scripts.insert(id, steps);
    }
    d.spells.spells.insert(
        MARK,
        spell(
            MARK,
            "Mark",
            [
                SpellEffect {
                    aura: 4,
                    ..effect(6, 1, 6)
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        ),
    );
    d
}

/// A second on, the queued spawned events and every pet's step run.
fn update(m: &mut Map) {
    m.now += Duration::from_secs(1);
    event_ai::update_despawns(m);
    pets::update(m);
}

fn moving(m: &Map, g: Guid) -> bool {
    matches!(m.creature(g).unwrap().motion, Motion::Moving(_))
}

fn command(m: &Map, g: Guid) -> u8 {
    pets::state(m, g).unwrap().command
}

/// Thistle Tea's `EventAI.events/1` "owned guardians and critters retain
/// events while controlled pets suppress them": a player's guardian and
/// companion and a creature's pet run their spawned events (mangos-classic's
/// `GuardianAI`); a player's hunter pet and demon, and a creature charmed
/// or possessed, run none (their master's `PetAI`).
#[tokio::test]
async fn owned_guardians_companions_and_a_creatures_pet_run_their_events_a_players_pets_none() {
    let (_realm, _h, gh, map, _pet, _wolf) = tamed_with(world()).await;
    let npc = place(&map, gh, WOLF, 0x70_0031, 30.0).await;
    let charmed = place(&map, gh, MARKED, 0x70_0032, 5.0).await;
    let possessed = place(&map, gh, MARKED, 0x70_0033, 6.0).await;
    let phases = map
        .query(move |m| {
            let at = m.players[&gh].position();
            let spawn = |m: &mut Map, owner: Guid, kind: Kind| {
                pets::testing::spawn(m, owner, MARKED, kind, 10, at, 0.0)
            };
            let own = [
                spawn(m, gh, Kind::Guardian),
                spawn(m, gh, Kind::Critter),
                spawn(m, npc, Kind::Demon),
                spawn(m, npc, Kind::Hunter),
            ];
            let players = [spawn(m, gh, Kind::Hunter), spawn(m, gh, Kind::Demon)];
            pets::charmed(m, gh, charmed, false);
            pets::charmed(m, gh, possessed, true);
            update(m);
            (
                own.map(|g| event_ai::phase(m, g)),
                players.map(|g| event_ai::phase(m, g)),
                [charmed, possessed].map(|g| event_ai::phase(m, g)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        phases.0, [7; 4],
        "a guardian, a companion, a creature's pets"
    );
    assert_eq!(phases.1, [0; 2], "a player's hunter pet and demon");
    assert_eq!(phases.2, [0; 2], "charmed and possessed");
}

/// Thistle Tea's companion trees "timed stay commands run before idle
/// following" and "a staying mini-pet still runs events that resume
/// following": a guardian, a creature's pet and a companion told to stay
/// by their own timer stay as their masters walk on; a companion staying
/// whose timer tells it to follow goes after its owner.
#[tokio::test]
async fn owned_creatures_stay_on_their_own_timers_word_and_a_companion_follows_on_its() {
    let (_realm, _h, gh, map, _pet, _wolf) = tamed_with(world()).await;
    let npc = place(&map, gh, WOLF, 0x70_0034, 3.0).await;
    let out = map
        .query(move |m| {
            let at = m.players[&gh].position();
            let guardian = pets::testing::spawn(m, gh, STAYER, Kind::Guardian, 10, at, 0.0);
            let critter = pets::testing::spawn(m, gh, STAYER, Kind::Critter, 1, at, 0.0);
            let theirs = pets::testing::spawn(m, npc, STAYER, Kind::Demon, 10, at, 0.0);
            m.players
                .get_mut(&gh)
                .unwrap()
                .owned
                .guardians
                .push(guardian);
            m.players.get_mut(&gh).unwrap().owned.critter = Some(critter);
            update(m);
            let told = [guardian, critter, theirs].map(|g| command(m, g));
            // Their masters walk 15 yards on.
            m.players.get_mut(&gh).unwrap().movement.position.x += 15.0;
            m.creature_mut(npc).unwrap().position.x += 15.0;
            update(m);
            update(m);
            let still = [guardian, critter, theirs].map(|g| !moving(m, g));
            // A companion staying, its timer says follow.
            let follower = pets::testing::spawn(m, gh, FOLLOWER, Kind::Critter, 1, at, 0.0);
            m.players.get_mut(&gh).unwrap().owned.critter = Some(follower);
            let s = pets::state_mut(m, follower).unwrap();
            s.command = pets::COMMAND_STAY;
            s.stay_at = Some(at);
            update(m);
            (told, still, command(m, follower), moving(m, follower))
        })
        .await
        .unwrap();
    assert_eq!(out.0, [pets::COMMAND_STAY; 3], "each told to stay");
    assert_eq!(out.1, [true; 3], "each stays as its master goes");
    assert_eq!(out.2, pets::COMMAND_FOLLOW, "told to follow");
    assert!(out.3, "and after its owner");
}

/// Thistle Tea's pet tree "an ordered attack starts the guardian's
/// scripted fight": a guardian drawn into its master's fight runs its
/// aggro event once and starts its fight's timers from then; a second
/// victim in the same fight runs no aggro again; its death runs its death
/// event (mangos's `GuardianAI::JustDied`).
#[tokio::test]
async fn a_guardians_fight_starts_its_scripted_one_once_and_its_death_runs_its_death_event() {
    let (_realm, _h, gh, map, _pet, wolf) = tamed_with(world()).await;
    let other = place(&map, gh, WOLF, 0x70_0035, 8.0).await;
    let out = map
        .query(move |m| {
            let at = m.players[&gh].position();
            let g = pets::testing::spawn(m, gh, FIGHTER, Kind::Guardian, 10, at, 0.0);
            m.players.get_mut(&gh).unwrap().owned.guardians.push(g);
            update(m);
            let idle = event_ai::phase(m, g);
            combat::start_attack(m, wolf, gh);
            combat::engage(m, wolf, gh);
            update(m);
            update(m);
            let fighting = event_ai::phase(m, g);
            let started = m.now;
            let timer = event_ai::scripted(m, g).and_then(|s| s.timers.get(1).copied().flatten());
            // Another of its fight's foes: no aggro again.
            event_ai::scripted_mut(m, g).unwrap().phase = 0;
            combat::start_attack(m, other, g);
            combat::engage(m, other, g);
            pets::ai::attack(m, g, other);
            update(m);
            let again = event_ai::phase(m, g);
            combat::kill(m, wolf, g);
            (
                idle,
                fighting,
                timer.map(|t| t - started),
                again,
                event_ai::phase(m, g),
            )
        })
        .await
        .unwrap();
    let (idle, fighting, timer, again, dead) = out;
    assert_eq!(idle, 0, "no fight, no aggro");
    assert_eq!(fighting, 3, "its aggro event");
    assert!(
        timer.is_some_and(|t| t >= Duration::from_secs(23)),
        "its fight's timer from its aggro: {timer:?}"
    );
    assert_eq!(again, 0, "one aggro a fight");
    assert_eq!(dead, 9, "its death event");
}

/// Thistle Tea's `melee_attack_with_context/3` "controlled creatures use
/// ordinary swings": a creature a script gave a melee spell casts it for
/// its swing, but charmed it swings as anyone does.
#[tokio::test]
async fn a_charmed_creature_swings_without_its_scripts_melee_spell() {
    let (_realm, _h, gh, map, _pet, _wolf) = tamed_with(world()).await;
    let own = place(&map, gh, WOLF, 0x70_0036, 2.0).await;
    let held = place(&map, gh, WOLF, 0x70_0037, 2.5).await;
    // Each one's foe: the wolf's of the players' faction, the charmed
    // one's a wolf.
    let theirs = place(&map, gh, WOLF, 0x70_0038, 3.0).await;
    let its = place(&map, gh, WOLF, 0x70_0039, 3.0).await;
    let out = map
        .query(move |m| {
            m.unit_fields_mut(theirs)
                .unwrap()
                .set_u32(UNIT_FIELD_FACTIONTEMPLATE, 1);
            for g in [own, held] {
                event_ai::scripted_mut(m, g).unwrap().melee_spell = Some(MARK);
            }
            pets::charmed(m, gh, held, false);
            // Swings until one lands: a charmed creature rolls on a
            // player's side of the table since 12g, so one may miss, be
            // dodged or parried.
            let swing = |m: &mut Map, g: Guid, victim: Guid| {
                let mut out = (false, false);
                for _ in 0..20 {
                    m.now += Duration::from_secs(3);
                    combat::start_attack(m, g, victim);
                    combat::try_swing(m, g);
                    let f = m.unit_fields(victim).unwrap();
                    out = (
                        m.creature(victim).unwrap().auras.has_spell(MARK),
                        f.get_u32(UNIT_FIELD_HEALTH) < f.get_u32(UNIT_FIELD_MAXHEALTH),
                    );
                    if out.0 || out.1 {
                        break;
                    }
                }
                out
            };
            (swing(m, own, theirs), swing(m, held, its))
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, false), "its own swing is the spell");
    assert_eq!(out.1, (false, true), "charmed, a blow");
}
