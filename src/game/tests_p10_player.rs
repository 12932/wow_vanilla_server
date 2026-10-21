//! Round 10's player: a charge's swing at what is still selected, an
//! invalid lesson (Thistle Tea's `attacking_test`, `server_movement_test`
//! and `teaching_dbc_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::fields::UNIT_FIELD_TARGET;
use crate::game::harness;
use crate::game::map::combat::dist;
use crate::game::map::{Map, MapHandle, charge};
use crate::game::tests_p3_entity::{NONE_WITHIN, spell};
use crate::game::tests_parity_motion as motion;

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// The map's clock `by` on, and the charges' step at that time.
fn later(m: &mut Map, by: Duration) {
    m.now += by;
    charge::update(m);
}

fn charge_world() -> crate::game::data::GameData {
    let mut d = motion::wolf_world();
    d.creature_spawns.get_mut(&0).unwrap()[0] =
        motion::spawn(80, motion::WOLF, motion::at(20.0, 0.0));
    d
}

/// Thistle Tea's `start_selected` and `ServerMovement.start` and `cancel`:
/// a player's charge runs along its path (part of the way at 200 ms, by
/// its target at the end) and swings on arriving at what the client still
/// has selected, though nothing had begun a fight ("arrival uses client
/// selection even when combat has cleared the attack field"). Another
/// selected on the way, none, or the player themselves: no swing ("changing
/// or clearing selection prevents a late attack"). A target dead or gone
/// by then gets none either; cut short (a teleport), the player stays
/// where the run had taken them; taken over by another's control (Thistle
/// Tea's "incoming control invalidates an old charge arrival"), it is
/// over, with no swing.
#[tokio::test]
async fn a_charge_swings_on_arriving_only_at_what_is_still_selected() {
    let (_realm, mut a, ga, map) = motion::enter(charge_world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    type Change = fn(&mut Map, Guid, Guid);
    let changes: [(&str, Option<Change>); 5] = [
        ("kept", None),
        (
            "cleared",
            Some(|m, ga, _| {
                let p = m.players.get_mut(&ga).unwrap();
                p.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
            }),
        ),
        (
            "themselves",
            Some(|m, ga, _| {
                let p = m.players.get_mut(&ga).unwrap();
                p.fields.set_guid(UNIT_FIELD_TARGET, ga);
            }),
        ),
        (
            "another",
            Some(|m, ga, _| {
                let p = m.players.get_mut(&ga).unwrap();
                p.fields
                    .set_guid(UNIT_FIELD_TARGET, Guid::new(0xF130_0000_0000_0099));
            }),
        ),
        (
            "the target gone",
            Some(|m, _, g| {
                m.remove_creature(g);
            }),
        ),
    ];
    for (what, change) in changes {
        let out = on(&map, move |m| {
            crate::game::map::combat::stop_attack(m, ga);
            let p = m.players.get_mut(&ga).unwrap();
            p.charge = None;
            p.movement.position = motion::at(0.0, 0.0);
            p.fields.set_guid(UNIT_FIELD_TARGET, g);
            m.creature_mut(g)?.position = motion::at(20.0, 0.0);
            charge::start(m, ga, g, true);
            later(m, Duration::from_millis(200));
            let partway = m.players[&ga].position();
            if let Some(change) = change {
                change(m, ga, g);
            }
            later(m, Duration::from_secs(5));
            let p = &m.players[&ga];
            Some((partway, p.position(), p.combat.attacking, p.combat.victim))
        })
        .await;
        let Some((partway, end, attacking, victim)) = out else {
            panic!("{what}: the wolf is gone too soon");
        };
        let (start, wolf) = (motion::at(0.0, 0.0), motion::at(20.0, 0.0));
        assert!(
            dist(start, partway) > 0.5 && dist(partway, wolf) > 5.0,
            "{what}: part of the way: {partway:?}"
        );
        assert!(dist(end, wolf) < 5.0, "{what}: by the wolf: {end:?}");
        if what == "kept" {
            assert!(attacking && victim == g, "{what}: it swings");
        } else {
            assert!(!attacking, "{what}: no swing");
        }
    }
    // Cut short by a teleport where it stands: it stays there.
    let (partway, end) = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.teleport = None;
        p.movement.position = motion::at(0.0, 0.0);
        p.movement.orientation = 0.0;
        let creature = crate::game::tests_p3_combat::spawn(m, ga, motion::WOLF, 1, 40.0);
        charge::start(m, ga, creature, true);
        later(m, Duration::from_millis(300));
        let partway = m.players[&ga].position();
        crate::game::map::movement::teleport_within(m, ga, partway, 0.0);
        later(m, Duration::from_secs(5));
        (partway, m.players[&ga].position())
    })
    .await;
    assert!(dist(partway, end) < 0.01, "where the run took them");
    // Taken over by another's control on the way: no swing on arriving.
    let attacking = on(&map, move |m| {
        crate::game::map::combat::stop_attack(m, ga);
        let p = m.players.get_mut(&ga).unwrap();
        p.teleport = None;
        p.movement.position = motion::at(0.0, 0.0);
        p.movement.orientation = 0.0;
        let wolf = crate::game::tests_p3_combat::spawn(m, ga, motion::WOLF, 1, 20.0);
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_guid(UNIT_FIELD_TARGET, wolf);
        charge::start(m, ga, wolf, true);
        later(m, Duration::from_millis(200));
        let now = m.now;
        let p = m.players.get_mut(&ga).unwrap();
        p.owned.possessed_by = Some(crate::game::map::pets::possess::Possession {
            master: Guid::new(7),
            spell: 605,
            was_faction: 1,
            manual: true,
            next_think: now,
            next_cast_at: now,
            melee_fallback: false,
        });
        later(m, Duration::from_millis(50));
        let p = m.players.get_mut(&ga).unwrap();
        let over = p.charge.is_none();
        p.owned.possessed_by = None;
        later(m, Duration::from_secs(5));
        (over, m.players[&ga].combat.attacking)
    })
    .await;
    assert_eq!(attacking, (true, false), "possessed: over, no swing");
}

const BAD_LESSON: u32 = 97_960;

/// Thistle Tea's "invalid taught spells and missing reagents leave books
/// and skills untouched", as mangos's `addSpell` refuses a spell it
/// doesn't know: a lesson whose spell doesn't exist teaches nothing, and
/// says nothing.
#[tokio::test]
async fn a_lesson_of_a_spell_that_doesnt_exist_teaches_nothing() {
    let mut d = crate::game::data::GameData::empty();
    d.spells.spells.insert(
        BAD_LESSON,
        spell(
            BAD_LESSON,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_LEARN_SPELL,
                trigger_spell: 999_999,
                target_a: 1,
                ..Default::default()
            }],
        ),
    );
    let realm = harness::realm_with(d);
    let (mut a, g, map) =
        crate::game::tests_pets::enter(&realm, "A", "Reader", Race::Human, Class::Mage).await;
    a.clear();
    let learned = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, BAD_LESSON, Some(g));
        m.players[&g].spells.contains(&999_999)
    })
    .await;
    assert!(!learned, "not in the spellbook");
    let told = a
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_LEARNED_SPELL(s) if s.id == 999_999 => Some(()),
            _ => None,
        })
        .await;
    assert!(told.is_none(), "nor told");
}
