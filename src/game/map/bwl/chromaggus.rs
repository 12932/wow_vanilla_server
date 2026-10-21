//! Chromaggus, as mangos-classic's `boss_chromaggus` runs him, with its
//! core's spell scripts for him (`SpellEffects.cpp`): the two breaths he
//! uses are picked once for the copy (mangos's Dragondog Breath Selection
//! and `InitiateBreath`, kept in the copy's fields 9 and 10), the left
//! thirty seconds into the fight and every sixty, the right at sixty and
//! every sixty. Elemental Shield at the pull and every 45 seconds leaves
//! him weak to one school at random; Brood Affliction every seven seconds
//! puts one of the five afflictions on fifteen of those he fights (round
//! again if fewer), and one who bears all five is mutated; Frenzy every
//! fifteen; enraged below 20 percent.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;

use super::*;

pub const ELEMENTAL_SHIELD: u32 = 22276;
pub const SHIELDS: [u32; 5] = [22277, 22278, 22279, 22280, 22281];
pub const BROOD_AFFLICTION: u32 = 23173;
/// Blue, Black, Red, Bronze, Green.
pub const AFFLICTIONS: [u32; 5] = [23153, 23154, 23155, 23170, 23169];
pub const CHROMATIC_MUTATION: u32 = 23174;
const MUTATION_SELF: [u32; 2] = [23175, 23177];
pub const FRENZY: u32 = 23128;
pub const ENRAGE: u32 = 23537;
/// The breaths, left and right, in the order of their selection: Frost
/// Burn, Incinerate, Time Lapse, Corrosive Acid, Ignite Flesh.
pub const LEFT_BREATHS: [u32; 5] = [23187, 23308, 23310, 23313, 23315];
pub const RIGHT_BREATHS: [u32; 5] = [23189, 23309, 23312, 23314, 23316];

const EMOTE_GENERIC_FRENZY: u32 = 1191;
const EMOTE_FRENZY_KILL: u32 = 7797;
const EMOTE_SHIMMER: u32 = 9793;

const T_BREATH_LEFT: u32 = TIMERS + 6300;
const T_BREATH_RIGHT: u32 = TIMERS + 6301;
/// Brood Affliction's work, once a cast however many its area reached.
const T_BROOD: u32 = TIMERS + 6302;

/// The two breaths, picked the first time he is seen, kept for the copy:
/// five for the right, and one of the other four for the left.
fn select_breaths(map: &mut Map) {
    if get_data(map, CHROMA_LBREATH) != 0 && get_data(map, CHROMA_RBREATH) != 0 {
        return;
    }
    let mut rng = rand::rng();
    let right = rng.random_range(0..5usize);
    let left = (right + rng.random_range(1..5usize)) % 5;
    set_data(map, CHROMA_LBREATH, LEFT_BREATHS[left]);
    set_data(map, CHROMA_RBREATH, RIGHT_BREATHS[right]);
}

pub fn creature_event(map: &mut Map, g: Guid, entry: u32, event: Event) {
    if entry != NPC_CHROMAGGUS {
        return;
    }
    match event {
        Event::Spawned => select_breaths(map),
        Event::Aggro => {
            set_data(map, CHROMAGGUS, IN_PROGRESS);
            crate::game::map::instance_script::cancel_for(map, g);
            schedule_for(map, T_BREATH_LEFT, g, Duration::from_secs(30));
            schedule_for(map, T_BREATH_RIGHT, g, Duration::from_secs(60));
        }
        Event::Death => set_data(map, CHROMAGGUS, DONE),
        _ => {}
    }
}

/// A breath due: cast, it is due again in sixty seconds; it didn't go, it
/// is tried again.
pub fn creature_timer(map: &mut Map, key: u32, g: Guid) -> bool {
    let field = match key {
        T_BREATH_LEFT => CHROMA_LBREATH,
        T_BREATH_RIGHT => CHROMA_RBREATH,
        _ => return false,
    };
    if !map
        .creature(g)
        .is_some_and(|c| c.alive() && c.combat.attacking)
    {
        return true;
    }
    let spell = get_data(map, field);
    let went = spell != 0 && scripts::cast(map, g, g, spell, 0);
    let again = if went { 60_000 } else { 500 };
    schedule_for(map, key, g, Duration::from_millis(again));
    true
}

/// The copy's timer for him: Brood Affliction's work. Whether it was it.
pub fn timer(map: &mut Map, key: u32) -> bool {
    if key != T_BROOD {
        return false;
    }
    if let Some(c) = map
        .creatures
        .iter()
        .find(|(_, c)| c.entry == NPC_CHROMAGGUS && c.alive())
        .map(|(_, c)| c.guid)
    {
        brood_affliction(map, c);
    }
    true
}

/// His spells' dummy effects. Whether it was one.
pub fn dummy(map: &mut Map, caster: Guid, spell: u32) -> bool {
    match spell {
        ELEMENTAL_SHIELD => {
            for s in SHIELDS {
                crate::game::map::auras::remove_spell(map, caster, s, None);
            }
            let s = SHIELDS[rand::rng().random_range(0..SHIELDS.len())];
            crate::game::map::spells::cast_triggered(map, caster, s, Some(caster));
            true
        }
        BROOD_AFFLICTION => {
            crate::game::map::instance_script::schedule(map, T_BROOD, Duration::ZERO);
            true
        }
        _ => false,
    }
}

fn has(map: &Map, g: Guid, spell: u32) -> bool {
    crate::game::map::auras::auras(map, g).is_some_and(|a| a.has_spell(spell))
}

/// One affliction of the five, on fifteen of the players he fights (round
/// the list again if there are fewer); one who bears all five is mutated
/// (mangos's spell 23173).
fn brood_affliction(map: &mut Map, caster: Guid) {
    let affliction = AFFLICTIONS[rand::rng().random_range(0..AFFLICTIONS.len())];
    let players: Vec<Guid> = map
        .creature(caster)
        .map(|c| c.threat.iter().map(|(u, _)| *u).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|u| {
            map.players
                .get(u)
                .is_some_and(|p| p.death == crate::game::map::death::Death::Alive)
        })
        .collect();
    if players.is_empty() {
        return;
    }
    for t in players.iter().cycle().take(15).copied().collect::<Vec<_>>() {
        if !has(map, t, CHROMATIC_MUTATION) {
            crate::game::map::spells::cast_triggered(map, caster, affliction, Some(t));
        }
        if AFFLICTIONS.iter().all(|a| has(map, t, *a)) {
            crate::game::map::auras::remove_where(map, t, |_| true);
            crate::game::map::spells::cast_triggered(map, caster, CHROMATIC_MUTATION, Some(t));
            for s in MUTATION_SELF {
                crate::game::map::spells::cast_triggered(map, t, s, Some(t));
            }
        }
    }
}

fn emote(text: u32) -> ScriptStep {
    ScriptStep {
        command: 0,
        dataint: [text as i32, 0, 0, 0],
        ..Default::default()
    }
}

/// His timed casts, as mangos's `CombatAI` actions.
pub(super) fn install(b: &mut Build) {
    let c = NPC_CHROMAGGUS;
    b.data.scripts.ai_events.remove(&c);
    // Enraged below 20 percent, once a fight.
    b.event(
        c,
        1,
        EVENT_HP,
        0,
        [20, 0, 0, 0],
        vec![
            ScriptStep {
                data_flags: SF_TARGET_SELF,
                ..cast_step(ENRAGE, At::Me)
            },
            emote(EMOTE_GENERIC_FRENZY),
        ],
    );
    b.event(
        c,
        2,
        EVENT_TIMER_IN_COMBAT,
        EF_REPEATABLE | EF_CHECK_RESULT,
        [0, 0, 45_000, 45_000],
        vec![cast_step(ELEMENTAL_SHIELD, At::Me), emote(EMOTE_SHIMMER)],
    );
    b.timer(c, 3, BROOD_AFFLICTION, At::Me, 7000, (7000, 7000));
    b.event(
        c,
        4,
        EVENT_TIMER_IN_COMBAT,
        EF_REPEATABLE | EF_CHECK_RESULT,
        [15_000, 15_000, 15_000, 15_000],
        vec![cast_step(FRENZY, At::Me), emote(EMOTE_FRENZY_KILL)],
    );
    b.home_fails(c, 90, CHROMAGGUS);
}
