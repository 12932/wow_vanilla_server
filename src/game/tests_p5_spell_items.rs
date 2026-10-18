//! Phase 3, round 5, against Thistle Tea's `transporter_test.exs`: the
//! engineers' transporters (`item_scripts`), each try from a clean slate
//! and their outcomes counted.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, has};
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::map::auras::{self, AURA_DUMMY};
use crate::game::map::{Map, spells};

const GADGETZAN: u32 = 23453;
const ARRIVAL: u32 = 23448;
const EVERLOOK: u32 = 23442;
/// What they turn into: an arrival, a malfunction, a transform, Evil
/// Twin, and Everlook's other mishap.
const OUTCOMES: [u32; 5] = [23441, 23446, 23444, 23445, 23449];
const SKY: (f32, f32, f32) = (-7341.38, -3908.11, 150.7);
const EVERLOOK_AT: (f32, f32, f32) = (6700.0, -4600.0, 720.0);

fn transporters(d: &mut GameData) {
    let dummy = |id: u32| {
        let mut s = bare(id, 0);
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 3,
                target_a: 1,
                ..Default::default()
            },
        );
        s
    };
    let mut everlook = bare(EVERLOOK, 0);
    effect(
        &mut everlook,
        0,
        SpellEffect {
            effect: 5,
            target_a: 1,
            ..Default::default()
        },
    );
    d.spells.target_positions.insert(
        EVERLOOK,
        (1, EVERLOOK_AT.0, EVERLOOK_AT.1, EVERLOOK_AT.2, 0.0),
    );
    let mut spells = vec![dummy(GADGETZAN), dummy(ARRIVAL), everlook];
    for id in OUTCOMES {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, aura(AURA_DUMMY, 0, 0, 1));
        spells.push(s);
    }
    insert(d, spells);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Went {
    Outcome(u32),
    Sky,
    Everlook,
    Nothing,
}

/// `n` uses of `spell` by `g` from where they stand: what each did.
fn uses(m: &mut Map, g: Guid, spell: u32, n: usize) -> std::collections::BTreeMap<Went, usize> {
    let home = m.players[&g].movement.position;
    let mut out = std::collections::BTreeMap::new();
    for _ in 0..n {
        auras::remove_where(m, g, |h| OUTCOMES.contains(&h.spell));
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.position = home;
        }
        kit::fresh(m, g);
        spells::cast_triggered(m, g, spell, Some(g));
        let at = m.players[&g].movement.position;
        let near = |p: (f32, f32, f32)| (at.x - p.0).abs() < 1.0 && (at.y - p.1).abs() < 1.0;
        let went = if let Some(o) = OUTCOMES.iter().find(|o| has(m, g, **o)) {
            Went::Outcome(*o)
        } else if near(SKY) {
            Went::Sky
        } else if near(EVERLOOK_AT) {
            Went::Everlook
        } else {
            Went::Nothing
        };
        // Everlook's mishap comes with its teleport.
        let went = match (went, near(EVERLOOK_AT)) {
            (Went::Outcome(o), true) => Went::Outcome(o + 100_000),
            (w, _) => w,
        };
        *out.entry(went).or_default() += 1;
    }
    if let Some(p) = m.players.get_mut(&g) {
        p.movement.position = home;
    }
    out
}

fn share(out: &std::collections::BTreeMap<Went, usize>, w: Went, n: usize) -> f32 {
    *out.get(&w).unwrap_or(&0) as f32 / n as f32
}

/// Gadgetzan's: an arrival half the time, a malfunction's arrival a
/// quarter, a fall from the sky a quarter; its arrival a transform a
/// sixth, Evil Twin four sixths, nothing a sixth; Everlook's always there,
/// with its own mishap seven times in twelve none, three Evil Twin, two
/// the other; nothing for the dead (Thistle Tea's transporter tests).
#[tokio::test]
async fn the_transporters_arrive_malfunction_or_drop_their_travelers() {
    let realm = fast_realm(transporters);
    let (_a, g, map) =
        kit::player(&realm, "P5IA", "Traveler", Race::NightElf, Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let n = 1_200;
            let gadgetzan = uses(m, g, GADGETZAN, n);
            let arrival = uses(m, g, ARRIVAL, n);
            let everlook = uses(m, g, EVERLOOK, n);
            crate::game::map::death::kill(m, g);
            let dead = uses(m, g, GADGETZAN, 20);
            (gadgetzan, arrival, everlook, dead)
        })
        .await
        .unwrap();
    let (gadgetzan, arrival, everlook, dead) = out;
    let n = 1_200;
    let near = |a: f32, b: f32| (a - b).abs() < 0.06;
    assert!(
        near(share(&gadgetzan, Went::Outcome(23441), n), 0.5),
        "{gadgetzan:?}"
    );
    assert!(
        near(share(&gadgetzan, Went::Outcome(23446), n), 0.25),
        "{gadgetzan:?}"
    );
    assert!(near(share(&gadgetzan, Went::Sky, n), 0.25), "{gadgetzan:?}");
    assert!(
        near(share(&arrival, Went::Outcome(23444), n), 1.0 / 6.0),
        "{arrival:?}"
    );
    assert!(
        near(share(&arrival, Went::Outcome(23445), n), 4.0 / 6.0),
        "{arrival:?}"
    );
    assert!(
        near(share(&arrival, Went::Nothing, n), 1.0 / 6.0),
        "{arrival:?}"
    );
    assert!(
        near(share(&everlook, Went::Everlook, n), 7.0 / 12.0),
        "{everlook:?}"
    );
    assert!(
        near(share(&everlook, Went::Outcome(123_445), n), 3.0 / 12.0),
        "{everlook:?}"
    );
    assert!(
        near(share(&everlook, Went::Outcome(123_449), n), 2.0 / 12.0),
        "{everlook:?}"
    );
    assert_eq!(
        dead.keys().collect::<Vec<_>>(),
        [&Went::Nothing],
        "the dead"
    );
}
