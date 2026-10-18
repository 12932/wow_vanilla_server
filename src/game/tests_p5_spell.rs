//! Phase 3, round 5, against Thistle Tea's spell tests that fit no file of
//! their own: the send-event effect's runs (`send_event_test.exs`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, fresh};
use crate::game::data::GameData;
use crate::game::data::scripts::{ScriptStep, ScriptTable};
use crate::game::data::spells::SpellEffect;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::spells::{self, Targets};

const EVENT: u32 = 900;
const EMPTY_EVENT: u32 = 901;
const AT_NOTHING: u32 = 990_300;
const AT_SELF: u32 = 990_301;
const AT_UNIT: u32 = 990_302;
const EMPTY: u32 = 990_303;

fn events(d: &mut GameData) {
    let send = |id: u32, event: u32, target: u32| {
        let mut s = bare(id, 0);
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 61,
                misc_value: event as i32,
                target_a: target,
                ..Default::default()
            },
        );
        s
    };
    insert(
        d,
        [
            send(AT_NOTHING, EVENT, 0),
            send(AT_SELF, EVENT, 1),
            send(AT_UNIT, EVENT, 25),
            send(EMPTY, EMPTY_EVENT, 0),
        ],
    );
    // A step long after the start, so the run waits to be read.
    d.scripts.event.insert(
        EVENT,
        vec![ScriptStep {
            id: EVENT,
            delay_ms: 600_000,
            ..Default::default()
        }],
    );
}

/// The event runs `spell` started at `t`: their source and target.
fn runs(m: &mut Map, g: Guid, spell: u32, t: Targets) -> Vec<(Guid, Guid)> {
    m.scripts.runs.clear();
    fresh(m, g);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    spells::cast_triggered_with(m, g, s.id, t);
    m.scripts
        .runs
        .iter()
        .filter(|r| r.table == ScriptTable::Event)
        .map(|r| (r.source, r.target))
        .collect()
}

/// An event with no target of its own runs once from the caster, aimed at
/// the object, else the unit picked, else at nothing (the caster picked
/// counts as nothing); one at the caster runs on the caster; one at a unit
/// runs at it; an event with no steps runs nothing (Thistle Tea's
/// `Casting.start/4`, `SpellEffect.receive/4`, `cast_events/4` and
/// `target/4`, vmangos's `EffectSendEvent`).
#[tokio::test]
async fn an_events_run_is_aimed_at_the_object_the_unit_or_nothing() {
    let realm = fast_realm(events);
    let (_a, a, map) = human(&realm, "P5EA", "Eventer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let object = guid::game_object(1, 77);
            let unit = |g| Targets {
                unit: Some(g),
                ..Default::default()
            };
            let nothing = runs(m, a, AT_NOTHING, Targets::default());
            let selected = runs(m, a, AT_NOTHING, unit(kobold));
            let only_self = runs(m, a, AT_NOTHING, unit(a));
            let with_object = runs(
                m,
                a,
                AT_NOTHING,
                Targets {
                    unit: Some(kobold),
                    object: Some(object),
                    ..Default::default()
                },
            );
            let at_self = runs(m, a, AT_SELF, unit(kobold));
            let at_unit = runs(m, a, AT_UNIT, unit(kobold));
            let empty = runs(m, a, EMPTY, unit(kobold));
            (
                kobold,
                object,
                nothing,
                selected,
                only_self,
                with_object,
                at_self,
                at_unit,
                empty,
            )
        })
        .await
        .unwrap();
    let (kobold, object, nothing, selected, only_self, with_object, at_self, at_unit, empty) = out;
    let z = Guid::zero();
    assert_eq!(nothing, [(a, z)], "once, at nothing");
    assert_eq!(selected, [(a, kobold)], "at the unit picked");
    assert_eq!(only_self, [(a, z)], "the caster picked is nothing");
    assert_eq!(with_object, [(a, object)], "the object first");
    assert_eq!(at_self, [(a, a)], "a caster's event on the caster");
    assert_eq!(at_unit, [(a, kobold)], "a unit's at it");
    assert!(empty.is_empty(), "no steps, no run");
}
