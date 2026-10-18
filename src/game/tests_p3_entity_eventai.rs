//! Phase 3's EventAI tests, ported from Thistle Tea's
//! `test/game/world/entity/mob` (docs/parity/matrix.csv): what a creature's
//! EventAI hears: its kill, a spell that lands on it or that it lands, one
//! it shrugs off, a point reached, who comes into sight, and home again.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::data::spells::SpellEffect;
use crate::game::harness::Client;
use crate::game::map::auras::{self, AURA_SCHOOL_IMMUNITY};
use crate::game::map::spells::{self, Targets};
use crate::game::tests_p3_combat::{PACK, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{NONE_WITHIN, aura_effect, on, spell};

const HEARER: u32 = 3_760;
const SEER: u32 = 3_761;
/// A spell of no harm (a dummy) that lands on its target; Alice's frost.
const TOUCH: u32 = 99_100;
const FROST: u32 = 99_101;
const FROST_PROOF: u32 = 99_102;

fn yell(d: &mut GameData, script: u32, text: i32, line: &str) {
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

fn world() -> GameData {
    let mut d = data();
    for (entry, name) in [(HEARER, "Hearer"), (SEER, "Seer")] {
        let mut t = template(entry, 7);
        t.name = name.into();
        t.faction = PACK;
        d.creature_templates.insert(entry, t);
    }
    for (script, text, line) in [
        (21, 320, "gloat"),
        (22, 321, "touched"),
        (23, 322, "frozen"),
        (24, 323, "there"),
        (25, 324, "home"),
        (26, 325, "seen"),
        (27, 326, "landed"),
    ] {
        yell(&mut d, script, text, line);
    }
    let event = |id, creature, kind, params: [i32; 4], action| AiEvent {
        id,
        creature,
        kind,
        chance: 100,
        flags: EF_REPEATABLE,
        params,
        actions: [action, 0, 0],
        ..Default::default()
    };
    d.scripts.ai_events.insert(
        HEARER,
        vec![
            event(1, HEARER, EVENT_KILL, [0, 0, 0, 0], 21),
            event(2, HEARER, EVENT_HIT_BY_SPELL, [TOUCH as i32, 0, 0, 0], 22),
            event(3, HEARER, EVENT_HIT_BY_SPELL, [FROST as i32, 0, 0, 0], 23),
            event(4, HEARER, EVENT_MOVEMENT_INFORM, [8, 3, 0, 0], 24),
            event(5, HEARER, EVENT_REACHED_HOME, [0, 0, 0, 0], 25),
            event(
                7,
                HEARER,
                EVENT_SPELL_HIT_TARGET,
                [TOUCH as i32, 0, 0, 0],
                27,
            ),
        ],
    );
    d.scripts
        .ai_events
        .insert(SEER, vec![event(6, SEER, EVENT_OOC_LOS, [0, 10, 0, 0], 26)]);
    let mut touch = spell(
        TOUCH,
        &[SpellEffect {
            effect: 3,
            target_a: 25,
            ..Default::default()
        }],
    );
    touch.attributes[1] = crate::game::data::spells::ATTR_EX_NO_THREAT;
    let mut frost = spell(
        FROST,
        &[SpellEffect {
            effect: 2,
            base_points: 9,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        }],
    );
    frost.school = 4;
    for s in [
        touch,
        frost,
        spell(
            FROST_PROOF,
            &[aura_effect(AURA_SCHOOL_IMMUNITY, 1, 1 << 4, 1)],
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
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

/// Thistle Tea's "noncombat spell hits invoke EventAI after reception",
/// "resisted spells do not invoke spell-hit transformations", "notifies a
/// creature caster only for cast hits", "the killing blow fires the kill
/// event" and "an idle owner wakes at arrival and executes the EventAI
/// action": a creature's EventAI hears a spell of no harm that lands on it,
/// not one it shrugs off; its own spell landing; its killing blow; and a
/// point of its movement reached.
#[tokio::test]
async fn a_creatures_eventai_hears_what_lands_its_kill_and_its_arrival() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Mage).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    // Four hearers: one Alice touches and freezes, one that touches Bob,
    // one that kills him, and one that walks.
    let [touched, toucher, killer, walker] = on(&map, move |m| {
        // Alice outlasts whatever fights her.
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 100_000_000);
        f.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 100_000_000);
        [3.0, 4.0, 5.0, 6.0].map(|yards| spawn(m, ga, HEARER, 20, yards))
    })
    .await;
    a.sees(walker).await;
    a.clear();
    on(&map, move |m| {
        // A touch of no harm lands; frost on it proofed against frost
        // doesn't.
        spells::cast_triggered(m, ga, TOUCH, Some(touched));
        let proof = m.realm.data.spells.get(FROST_PROOF).cloned().unwrap();
        auras::apply(m, touched, touched, &proof, 0, 1);
        spells::cast(
            m,
            ga,
            FROST,
            Targets {
                unit: Some(touched),
                ..Default::default()
            },
            None,
        );
    })
    .await;
    assert_eq!(
        lines(&mut a).await,
        ["touched"],
        "the touch, not the frost it shrugged off"
    );
    // Another's own touch on Bob lands; a third kills him.
    on(&map, move |m| {
        spells::cast_triggered(m, toucher, TOUCH, Some(gb))
    })
    .await;
    assert_eq!(lines(&mut a).await, ["landed"], "its own spell landed");
    on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, killer, gb, 10_000_000);
    })
    .await;
    assert_eq!(lines(&mut a).await, ["gloat"], "its kill");
    // A point of its movement reached.
    on(&map, move |m| {
        crate::game::map::event_ai::movement_inform(m, walker, 8, 3)
    })
    .await;
    assert_eq!(lines(&mut a).await, ["there"], "its point reached");
    let _ = &mut b;
}

/// Thistle Tea's "clears evading once the creature is home, without a
/// pending broadcast" and "an announcer in sight wakes the creature at its
/// next EventAI slot": an evading creature is done evading once home, and
/// says so; and one out of a fight sees a player come within its range.
#[tokio::test]
async fn a_creature_is_done_evading_at_home_and_sees_who_comes_near() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Mage).await;
    let hearer = on(&map, move |m| spawn(m, ga, HEARER, 20, 3.0)).await;
    a.sees(hearer).await;
    a.clear();
    on(&map, move |m| {
        crate::game::map::ai::attacked(m, hearer, ga);
        let home = m.creature(hearer).unwrap().home;
        // Off its spot, then sent home.
        m.creature_mut(hearer).unwrap().position = Vector3d {
            x: home.x + 6.0,
            ..home
        };
        crate::game::map::ai::enter_evade(m, hearer);
    })
    .await;
    let evading = on(&map, move |m| m.creature(hearer).unwrap().evading).await;
    assert!(evading, "evading on its way");
    crate::game::tests_p3_entity_objects::until(&map, "home and done evading", move |m| {
        !m.creature(hearer).unwrap().evading
    })
    .await;
    let line = a
        .wait_for("the hearer home", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(line, "home");
    // A seer twenty yards off; Alice, no game master here, comes within
    // ten (a game master goes unseen).
    let seer = on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().gm = false;
        spawn(m, ga, SEER, 20, 20.0)
    })
    .await;
    a.sees(seer).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(lines(&mut a).await.is_empty(), "too far yet");
    let at = on(&map, move |m| m.position_of(seer).unwrap()).await;
    a.heartbeat(at.x - 5.0, at.y, at.z).await;
    let line = a
        .wait_for("the seer's yell", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => Some(c.message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(line, "seen");
    let _ = Guid::zero();
}
