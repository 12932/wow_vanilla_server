//! Phase 3's creature tests, ported from Thistle Tea's
//! `test/game/world/entity/mob` (docs/parity/matrix.csv): a summon's time
//! by its despawn type, what its summoner hears of it, a creature
//! despawned or respawned in a fight, and a player's pet killed.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::*;
use crate::game::data::{CreatureSpawn, GameData};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::event_ai;
use crate::game::map::scripts;
use crate::game::tests_p3_combat::{BRUTE, PACK, START, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};

const SUMMONER: u32 = 3710;
const SUMMONED: u32 = 3711;

fn world() -> GameData {
    let mut d = data();
    for (entry, name) in [(SUMMONER, "Summoner"), (SUMMONED, "Summoned")] {
        let mut t = template(entry, 7);
        t.name = name.into();
        t.faction = PACK;
        d.creature_templates.insert(entry, t);
    }
    // The summoner yells "born", "died" and "gone" as it hears of its
    // summons.
    for (script, text, line) in [(1, 300, "born"), (2, 301, "died"), (3, 302, "gone")] {
        d.scripts.ai_scripts.insert(
            script,
            vec![ScriptStep {
                id: script,
                command: 0,
                dataint: [text, 0, 0, 0],
                ..Default::default()
            }],
        );
        d.npc.broadcast_texts.insert(
            text as u32,
            crate::game::data::npc::BroadcastText {
                male: line.into(),
                chat_type: 1,
                ..Default::default()
            },
        );
    }
    let event = |id, kind, action| AiEvent {
        id,
        creature: SUMMONER,
        kind,
        chance: 100,
        flags: EF_REPEATABLE,
        params: [SUMMONED as i32, 0, 0, 0],
        actions: [action, 0, 0],
        ..Default::default()
    };
    d.scripts.ai_events.insert(
        SUMMONER,
        vec![
            event(1, EVENT_SUMMONED_UNIT, 1),
            event(2, EVENT_SUMMONED_JUST_DIED, 2),
            event(3, EVENT_SUMMONED_JUST_DESPAWN, 3),
        ],
    );
    d
}

/// `summoner` summons one beside it, for `ms` of despawn type `kind`
/// (none, 0 ms).
fn summon(m: &mut Map, summoner: Guid, ms: u64, kind: i32) -> Guid {
    let at = m.creature(summoner).unwrap().position;
    let despawn = (ms > 0 || kind != 0).then_some((Duration::from_millis(ms), kind));
    let g = scripts::summon(
        m,
        summoner,
        SUMMONED,
        Vector3d {
            x: at.x + 2.0,
            ..at
        },
        0.0,
        despawn,
    )
    .expect("summoned");
    event_ai::update_despawns(m);
    g
}

/// Its time has come.
fn due(m: &mut Map, g: Guid) {
    let past = m.now - Duration::from_millis(1);
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.despawn_at = Some(past);
    }
    event_ai::update_despawns(m);
}

fn kill(m: &mut Map, by: Guid, g: Guid) {
    crate::game::map::combat::deal_damage(m, by, g, 1_000_000);
}

/// The lines creatures yelled to a client, in order.
async fn lines(c: &mut Client) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(line) = c
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
                if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::MonsterYell { .. }) =>
            {
                Some(c.message.clone())
            }
            _ => None,
        })
        .await
    {
        out.push(line);
    }
    out
}

/// Thistle Tea's `summon_despawn_due` and `schedule` for typed summons:
/// one with no type goes at its time, fight or not; a timed-or-dead one
/// (1) killed lies as a corpse with no time and no respawn, and its time
/// coming leaves it so; a timed one (3) dead at its time goes, and one a
/// player owns but doesn't charm goes at its time though it fights.
#[tokio::test]
async fn a_summons_time_goes_by_its_type_dead_or_fighting() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let summoner = spawn(m, ga, SUMMONER, 20, 4.0);
        // No type: gone at its time, in a fight.
        let untyped = summon(m, summoner, 60_000, 0);
        crate::game::map::combat::engage(m, ga, untyped);
        due(m, untyped);
        let untyped_gone = m.creature(untyped).is_none();
        // 1, killed: a corpse, no time to go, nothing to bring it back.
        let one = summon(m, summoner, 60_000, 1);
        kill(m, ga, one);
        let corpse = m.creature(one).is_some_and(|c| !c.alive());
        let respawns = m.creature(one).and_then(|c| c.spawn_guid);
        due(m, one);
        let still_a_corpse = m.creature(one).is_some_and(|c| !c.alive());
        // 3, dead when its time comes: gone.
        let three = summon(m, summoner, 60_000, 3);
        kill(m, ga, three);
        let three_lies = m.creature(three).is_some();
        due(m, three);
        let three_gone = m.creature(three).is_none();
        // 3, a player's guardian (not charmed), fighting: gone at its time.
        let owned = summon(m, summoner, 60_000, 3);
        let now = m.now;
        if let Some(c) = m.creature_mut(owned) {
            c.owner = Some(ga);
            c.pet = Some(Box::new(crate::game::map::pets::PetState::new(
                crate::game::map::pets::Kind::Guardian,
                ga,
                now,
            )));
        }
        crate::game::map::combat::engage(m, ga, owned);
        due(m, owned);
        let owned_gone = m.creature(owned).is_none();
        (
            untyped_gone,
            (corpse, respawns, still_a_corpse),
            (three_lies, three_gone),
            owned_gone,
        )
    })
    .await;
    assert!(out.0, "an untyped summon goes at its time, fighting");
    assert_eq!(
        out.1,
        (true, None, true),
        "a timed-or-dead summon's corpse lies through its time, not to respawn"
    );
    assert_eq!(out.2, (true, true), "a timed one dead at its time goes");
    assert!(out.3, "an owned guardian isn't held for its fight");
}

/// Thistle Tea's "temporary creatures publish one birth, death, and corpse
/// departure" and "timed removal of a living summon does not report
/// death": a summoner hears once of a summon's coming, once of its death
/// however often it is struck dead, and once of its corpse's going; one
/// whose time takes it alive is heard to go, not to die.
#[tokio::test]
async fn a_summoner_hears_of_its_summons_coming_death_and_going_once_each() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let summoner = on(&map, move |m| spawn(m, ga, SUMMONER, 20, 4.0)).await;
    a.sees(summoner).await;
    a.clear();
    on(&map, move |m| {
        let s = summon(m, summoner, 0, 0);
        kill(m, ga, s);
        kill(m, ga, s);
        crate::game::map::combat::deal_damage(m, ga, s, 10);
        let now = m.now;
        if let Some(c) = m.creature_mut(s)
            && let crate::game::map::creature::Life::Dead { corpse_until, .. } = &mut c.life
        {
            *corpse_until = now;
        }
        crate::game::map::ai::update_dead(m);
    })
    .await;
    assert_eq!(lines(&mut a).await, ["born", "died", "gone"]);
    on(&map, move |m| {
        let s = summon(m, summoner, 60_000, 3);
        due(m, s);
    })
    .await;
    assert_eq!(
        lines(&mut a).await,
        ["born", "gone"],
        "taken alive, it didn't die"
    );
}

/// Thistle Tea's `Respawn.despawn` and `Respawn.force`: a spawned creature
/// despawned in a fight leaves it, dead, untargeting and with no threat;
/// one forced back to life in a fight
/// comes back whole and out of it, and can be fought again.
#[tokio::test]
async fn a_despawned_creature_leaves_its_fight_and_one_respawned_comes_back_whole() {
    let mut d = world();
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 61,
        entries: vec![BRUTE],
        map: 0,
        x: START.0 + 3.0,
        y: START.1,
        z: START.2,
        respawn_min: 300,
        respawn_max: 300,
        ..Default::default()
    });
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let brute = crate::game::guid::unit(BRUTE, 61);
    a.sees(brute).await;
    let fighting = |m: &Map, c: Guid| {
        let x = m.creature(c).unwrap();
        (
            x.alive(),
            x.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            ),
            x.fields.get_guid(UNIT_FIELD_TARGET),
            x.threat.len(),
        )
    };
    let out = on(&map, move |m| {
        crate::game::map::ai::attacked(m, brute, ga);
        crate::game::map::combat::engage(m, ga, brute);
        let before = fighting(m, brute);
        scripts::despawn(m, brute);
        (before, fighting(m, brute))
    })
    .await;
    assert_eq!(out.0, (true, true, ga, 1), "fighting Alice");
    assert_eq!(
        out.1,
        (false, false, Guid::zero(), 0),
        "gone from its fight"
    );
    // Back, then forced back again while it fights.
    let out = on(&map, move |m| {
        crate::game::map::ai::respawn_now(m, brute);
        crate::game::map::ai::attacked(m, brute, ga);
        crate::game::map::combat::deal_damage(m, ga, brute, 100);
        let hurt = fighting(m, brute);
        crate::game::map::ai::respawn_forced(m, brute, true);
        let c = m.creature(brute).unwrap();
        let whole = c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        let back = fighting(m, brute);
        crate::game::map::ai::attacked(m, brute, ga);
        let again = fighting(m, brute);
        (hurt, whole, back, again)
    })
    .await;
    assert_eq!(out.0.3, 1, "it had Alice on its list");
    assert!(out.1, "forced back, it is whole");
    assert_eq!(
        out.2,
        (true, false, Guid::zero(), 0),
        "and out of the fight"
    );
    assert_eq!(out.3.3, 1, "and fights her again when attacked");
}

/// Thistle Tea's "player-owned pets grant no kill credit": a player's pet
/// killed gives its killer no experience.
#[tokio::test]
async fn a_players_pet_is_worth_nothing_to_its_killer() {
    let (realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        let pet = spawn(m, ga, BRUTE, 1, 2.0);
        let now = m.now;
        if let Some(c) = m.creature_mut(pet) {
            c.owner = Some(ga);
            c.pet = Some(Box::new(crate::game::map::pets::PetState::new(
                crate::game::map::pets::Kind::Guardian,
                ga,
                now,
            )));
        }
        let xp = m.players[&gb].fields.get_u32(PLAYER_XP);
        kill(m, gb, pet);
        (
            m.creature(pet).is_none_or(|c| !c.alive()),
            xp,
            m.players[&gb].fields.get_u32(PLAYER_XP),
        )
    })
    .await;
    assert!(out.0, "the pet is dead");
    assert_eq!(out.1, out.2, "Bob gained nothing");
    assert!(
        b.gets(NONE_WITHIN, |m| matches!(
            m,
            ServerOpcodeMessage::SMSG_LOG_XPGAIN(_)
        )
        .then_some(()))
            .await
            .is_none()
    );
}
