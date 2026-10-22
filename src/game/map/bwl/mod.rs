//! Blackwing Lair: its instance script and its bosses, as mangos-classic's
//! ScriptDevAI has them (`instance_blackwing_lair`, `boss_razorgore`,
//! `boss_vaelastrasz`, `boss_broodlord`, `boss_firemaw`, `boss_ebonroc`,
//! `boss_flamegor`, `boss_chromaggus`, `boss_victor_nefarius`,
//! `boss_nefarian`), on vmangos's world (its spawns, its spell ids, its
//! broadcast texts).
//!
//! The copy keeps mangos's encounter fields (0 Razorgore to 7 Nefarian, 8
//! the Scepter quest, 9 and 10 Chromaggus's breaths, 11 and 12 the
//! drakonids of Nefarian's tunnels). The doors stand as they say: the main
//! gate shuts while any encounter is under way, each boss's own door opens
//! at its death, and no player may work them. A boss's fights are its
//! timed casts, built here as EventAI events (mangos's `CombatAI` actions:
//! a cast that doesn't go is tried again, and only one that does is timed
//! anew), and what EventAI can't say is done by the copy.

use std::collections::BTreeMap;
use std::time::Duration;

use wow_world_messages::Guid;

pub mod chromaggus;
pub mod nefarian;
pub mod razorgore;
pub mod vael;

use super::instance_adapters::TIMERS;
use super::instance_script::{
    DONE, Event, FAILED, IN_PROGRESS, NOT_STARTED, Open, get_data, schedule_for, store,
};
use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::map::Map;
use crate::game::map::scripts;

pub const SPECIAL: u32 = 4;

// Encounter fields.
pub const RAZORGORE: u32 = 0;
pub const VAELASTRASZ: u32 = 1;
pub const LASHLAYER: u32 = 2;
pub const FIREMAW: u32 = 3;
pub const EBONROC: u32 = 4;
pub const FLAMEGOR: u32 = 5;
pub const CHROMAGGUS: u32 = 6;
pub const NEFARIAN: u32 = 7;
pub const QUEST_SCEPTER: u32 = 8;
pub const CHROMA_LBREATH: u32 = 9;
pub const CHROMA_RBREATH: u32 = 10;
pub const NEFA_LTUNNEL: u32 = 11;
pub const NEFA_RTUNNEL: u32 = 12;

// Creatures.
pub const NPC_RAZORGORE: u32 = 12435;
pub const NPC_VAELASTRASZ: u32 = 13020;
pub const NPC_LASHLAYER: u32 = 12017;
pub const NPC_FIREMAW: u32 = 11983;
pub const NPC_EBONROC: u32 = 14601;
pub const NPC_FLAMEGOR: u32 = 11981;
pub const NPC_CHROMAGGUS: u32 = 14020;
pub const NPC_NEFARIAN: u32 = 11583;
pub const NPC_VICTOR_NEFARIUS: u32 = 10162;

// Doors.
pub const DOOR_RAZORGORE_ENTER: u32 = 176964;
pub const DOOR_RAZORGORE_EXIT: u32 = 176965;
pub const DOOR_NEFARIAN: u32 = 176966;
pub const DOOR_CHROMAGGUS_ENTER: u32 = 179115;
pub const DOOR_CHROMAGGUS_SIDE: u32 = 179116;
pub const DOOR_CHROMAGGUS_EXIT: u32 = 179117;
pub const DOOR_VAELASTRASZ: u32 = 179364;
pub const DOOR_LASHLAYER: u32 = 179365;

/// The doors the copy works, which no player may use: the Chromaggus
/// side door opens by its lever's own script.
pub const DOOR_ENTRIES: [u32; 8] = [
    DOOR_RAZORGORE_ENTER,
    DOOR_RAZORGORE_EXIT,
    DOOR_NEFARIAN,
    DOOR_CHROMAGGUS_ENTER,
    DOOR_CHROMAGGUS_SIDE,
    DOOR_CHROMAGGUS_EXIT,
    DOOR_VAELASTRASZ,
    DOOR_LASHLAYER,
];

type Data = BTreeMap<u32, u32>;

fn v(d: &Data, field: u32) -> u32 {
    d.get(&field).copied().unwrap_or(0)
}

/// The main gate shuts whenever an encounter is under way (mangos closes it
/// at any `IN_PROGRESS` or `SPECIAL`, and opens it again once none is).
fn main_gate_open(d: &Data) -> bool {
    !(RAZORGORE..=NEFARIAN).any(|f| matches!(v(d, f), IN_PROGRESS | SPECIAL))
}

/// Razorgore's way out: open once he is dead, but shut while Vaelastrasz
/// is fought (mangos uses it at each of his fight's states but the
/// intro's).
fn razorgore_exit_open(d: &Data) -> bool {
    v(d, RAZORGORE) == DONE && v(d, VAELASTRASZ) != IN_PROGRESS
}

/// Nefarian's door shuts behind the raid while his fight lasts.
fn nefarian_door_open(d: &Data) -> bool {
    !matches!(v(d, NEFARIAN), IN_PROGRESS | SPECIAL)
}

pub fn doors() -> &'static [(u32, Open)] {
    &[
        (DOOR_RAZORGORE_ENTER, Open::Rule(main_gate_open)),
        (DOOR_RAZORGORE_EXIT, Open::Rule(razorgore_exit_open)),
        (DOOR_VAELASTRASZ, Open::Done(VAELASTRASZ)),
        (DOOR_LASHLAYER, Open::Done(LASHLAYER)),
        (DOOR_CHROMAGGUS_EXIT, Open::Done(CHROMAGGUS)),
        (DOOR_NEFARIAN, Open::Rule(nefarian_door_open)),
    ]
}

/// A field set, as `instance_blackwing_lair::SetData` has it: a boss's
/// field won stays won; the doors follow the data.
pub fn set_data(map: &mut Map, field: u32, value: u32) {
    if vael::hook(map, field) || nefarian::hook(map, field) {
        return;
    }
    let old = get_data(map, field);
    if field <= NEFARIAN && old == DONE {
        return;
    }
    if old == value {
        return;
    }
    store(map, field, value);
    changed(map, field, old, value);
}

/// What a field's change brings.
fn changed(map: &mut Map, field: u32, _old: u32, new: u32) {
    match field {
        RAZORGORE => razorgore::changed(map, new),
        NEFARIAN => nefarian::changed(map, new),
        LASHLAYER if new == DONE => suppression_off(map),
        _ => {}
    }
}

/// The suppression room's devices, with Broodlord Lashlayer dead: gone for
/// the copy's life (mangos's `OnObjectCreate`, `go_suppression`).
pub const GO_SUPPRESSION_DEVICE: u32 = 179784;

fn suppression_off(map: &mut Map) {
    let devices: Vec<Guid> = map
        .gameobjects
        .iter()
        .filter(|(_, o)| o.entry == GO_SUPPRESSION_DEVICE)
        .map(|(_, o)| o.guid)
        .collect();
    for d in devices {
        crate::game::map::objects::despawn_then(map, d, None);
    }
}

/// Whether the map is a copy of the lair.
fn here(map: &Map) -> bool {
    map.key.instance != 0
        && map.instance.script == Some(super::instance_script::Script::BlackwingLair)
}

/// A dummy effect the lair's script owns: the Orb of Domination's.
/// Whether it was one.
pub fn dummy(
    map: &mut Map,
    caster: Guid,
    _target: Guid,
    s: &crate::game::data::spells::SpellTemplate,
) -> bool {
    if !here(map) {
        return false;
    }
    match s.id {
        razorgore::DRAGON_ORB => {
            razorgore::orb_used(map, caster);
            true
        }
        id => chromaggus::dummy(map, caster, id),
    }
}

/// An object a spell worked (effect 86) the lair's script owns: an egg
/// Razorgore broke. Whether it was one.
pub fn object_activated(map: &mut Map, caster: Guid, object: Guid, spell: u32) -> bool {
    here(map) && razorgore::egg_broken(map, caster, object, spell)
}

/// A blow that would have killed `g`, whom a floor kept up.
pub fn death_prevented(map: &mut Map, g: Guid) {
    if here(map) {
        razorgore::death_prevented(map, g);
    }
}

/// The fields whose fight went with the copy (its summons, its timers), as
/// it loads: back to the start, as mangos's `Load` puts what was under way.
/// A fight's middle stage (`SPECIAL`) is under way too here, its summons
/// gone with it. Vaelastrasz's lost fight starts over too: his intro (the
/// way to his fight) is only for one not started, and a copy loaded again
/// lays him down on the floor with no menu.
pub fn demote(data: &mut Data) {
    for f in RAZORGORE..=NEFA_RTUNNEL {
        if let Some(x) = data.get_mut(&f)
            && (matches!(*x, IN_PROGRESS | SPECIAL) || (f == VAELASTRASZ && *x == FAILED))
            && f != QUEST_SCEPTER
        {
            *x = NOT_STARTED;
        }
    }
}

/// The bosses whose fight is no more than its field: (entry, field).
const PLAIN: [(u32, u32); 4] = [
    (NPC_LASHLAYER, LASHLAYER),
    (NPC_FIREMAW, FIREMAW),
    (NPC_EBONROC, EBONROC),
    (NPC_FLAMEGOR, FLAMEGOR),
];

/// A creature's event in the lair.
pub fn creature_event(map: &mut Map, g: Guid, entry: u32, _spawn: Option<u32>, event: Event) {
    if let Some((_, field)) = PLAIN.iter().find(|(e, _)| *e == entry) {
        match event {
            Event::Aggro => set_data(map, *field, IN_PROGRESS),
            Event::Death => set_data(map, *field, DONE),
            _ => {}
        }
    }
    if entry == NPC_LASHLAYER {
        lashlayer_event(map, g, event);
    }
    razorgore::creature_event(map, g, entry, event);
    vael::creature_event(map, g, entry, event);
    chromaggus::creature_event(map, g, entry, event);
    nefarian::creature_event(map, g, entry, event);
}

/// Whether spell `spell` of the lair may land on `g` (a class call, on its
/// class alone).
pub fn target_allowed(map: &Map, spell: u32, g: Guid) -> bool {
    !here(map) || nefarian::class_call_reaches(map, spell, g)
}

/// A living player into area trigger `id` here.
pub fn area_trigger(map: &mut Map, player: Guid, id: u32) {
    vael::area_trigger(map, player, id);
}

// Broodlord Lashlayer: his yell at the pull, and his leash: below the
// floor of his hall (z 448.6) he gives up the fight, and says so.

const SAY_LASHLAYER_AGGRO: u32 = 9967;
const SAY_LASHLAYER_LEASH: u32 = 9968;
const LASHLAYER_LEASH_Z: f32 = 448.60;
const T_LASHLAYER_LEASH: u32 = TIMERS + 6000;

fn lashlayer_event(map: &mut Map, g: Guid, event: Event) {
    if event == Event::Aggro {
        scripts::talk(map, g, SAY_LASHLAYER_AGGRO, Guid::zero(), None);
        schedule_for(map, T_LASHLAYER_LEASH, g, Duration::from_secs(1));
    }
}

/// A timed step for one creature of the lair.
pub fn creature_timer(map: &mut Map, key: u32, g: Guid) {
    if razorgore::creature_timer(map, key, g)
        || vael::creature_timer(map, key, g)
        || chromaggus::creature_timer(map, key, g)
        || nefarian::creature_timer(map, key, g)
    {
        return;
    }
    if key == T_LASHLAYER_LEASH {
        let Some(c) = map.creature(g).filter(|c| c.alive()) else {
            return;
        };
        if !c.combat.attacking || c.evading {
            return;
        }
        if c.position.z < LASHLAYER_LEASH_Z {
            scripts::talk(map, g, SAY_LASHLAYER_LEASH, Guid::zero(), None);
            let s = ScriptStep {
                command: 33,
                ..Default::default()
            };
            scripts::execute(map, &s, g, g);
            return;
        }
        schedule_for(map, T_LASHLAYER_LEASH, g, Duration::from_secs(1));
    }
}

/// A timed step of the copy.
pub fn timer(map: &mut Map, key: u32) {
    let _ = razorgore::timer(map, key)
        || vael::timer(map, key)
        || chromaggus::timer(map, key)
        || nefarian::timer(map, key);
}

/// Objects as the copy's data leaves them, once its spawns are in.
pub fn settle_spawns(map: &mut Map) {
    razorgore::settle(map);
    if get_data(map, LASHLAYER) == DONE {
        suppression_off(map);
    }
}

// The bosses' timed casts, as EventAI.

/// The ids of what is built here, clear of the world's and Thistle Tea's.
pub(super) const BASE: u32 = 96_120_000;

/// Where a cast goes.
#[derive(Debug, Clone, Copy)]
pub(super) enum At {
    /// Itself (mangos's `DoCastSpellIfCan(nullptr, ..)` of an area spell
    /// about it, or one on itself).
    Me,
    /// Its victim.
    Victim,
}

pub(super) struct Build<'a> {
    pub(super) data: &'a mut GameData,
    next: u32,
}

impl Build<'_> {
    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    /// One event, its steps the action.
    fn event(
        &mut self,
        entry: u32,
        n: u32,
        kind: u32,
        flags: u32,
        params: [i32; 4],
        steps: Vec<ScriptStep>,
    ) {
        self.event_chance(entry, n, kind, flags, 100, params, steps);
    }

    /// [`Build::event`], by a chance in a hundred.
    #[allow(clippy::too_many_arguments)]
    fn event_chance(
        &mut self,
        entry: u32,
        n: u32,
        kind: u32,
        flags: u32,
        chance: u32,
        params: [i32; 4],
        steps: Vec<ScriptStep>,
    ) {
        let action = self.id();
        self.data.scripts.ai_scripts.insert(action, steps);
        let e = AiEvent {
            id: entry * 100 + n,
            creature: entry,
            kind,
            chance,
            flags,
            params,
            actions: [action, 0, 0],
            ..Default::default()
        };
        self.data
            .scripts
            .ai_events
            .entry(entry)
            .or_default()
            .push(e);
    }

    /// `spell` cast first `first` ms into the fight, then every `repeat`
    /// (from, to) ms after one that went; one that didn't is tried again.
    fn timer(&mut self, entry: u32, n: u32, spell: u32, at: At, first: u32, repeat: (u32, u32)) {
        self.timer_between(entry, n, spell, at, (first, first), repeat);
    }

    /// [`Build::timer`], first between `first`'s two.
    fn timer_between(
        &mut self,
        entry: u32,
        n: u32,
        spell: u32,
        at: At,
        first: (u32, u32),
        repeat: (u32, u32),
    ) {
        self.event(
            entry,
            n,
            EVENT_TIMER_IN_COMBAT,
            EF_REPEATABLE | EF_CHECK_RESULT,
            [
                first.0 as i32,
                first.1 as i32,
                repeat.0 as i32,
                repeat.1 as i32,
            ],
            vec![cast_step(spell, at)],
        );
    }

    /// Home from a fight, its encounter failed (mangos's
    /// `JustReachedHome`).
    fn home_fails(&mut self, entry: u32, n: u32, field: u32) {
        self.event(
            entry,
            n,
            EVENT_REACHED_HOME,
            EF_REPEATABLE,
            [0; 4],
            vec![ScriptStep {
                command: 37,
                datalong: [field, FAILED, 0, 0],
                ..Default::default()
            }],
        );
    }

    /// Wing Buffet's hit takes half the threat on whom it hits.
    fn buffet_threat(&mut self, entry: u32, n: u32) {
        self.event(
            entry,
            n,
            EVENT_SPELL_HIT_TARGET,
            EF_REPEATABLE,
            [WING_BUFFET as i32, 0, 0, 0],
            vec![ScriptStep {
                command: 29,
                x: -50.0,
                ..Default::default()
            }],
        );
    }
}

fn cast_step(spell: u32, at: At) -> ScriptStep {
    let mut s = ScriptStep {
        command: 15,
        datalong: [spell, 0, 0, 0],
        data_flags: SF_ABORT_ON_FAILURE,
        ..Default::default()
    };
    match at {
        At::Me => s.data_flags |= SF_TARGET_SELF,
        At::Victim => s.target_type = 1,
    }
    s
}

const SHADOW_FLAME: u32 = 22539;
const WING_BUFFET: u32 = 23339;
const FLAME_BUFFET: u32 = 23341;
const THRASH: u32 = 3391;
const SHADOW_OF_EBONROC: u32 = 23340;
const FRENZY: u32 = 23342;
const EMOTE_GENERIC_FRENZY: u32 = 1191;
const LASHLAYER_CLEAVE: u32 = 15284;
const KNOCK_AWAY: u32 = 18670;
const BLAST_WAVE: u32 = 23331;
const MORTAL_STRIKE: u32 = 24573;

/// The lair's bosses' events, into the game data, over any the world
/// has for them.
pub fn install(data: &mut GameData) {
    if !data.is_loaded() {
        return;
    }
    for entry in [NPC_LASHLAYER, NPC_FIREMAW, NPC_EBONROC, NPC_FLAMEGOR] {
        data.scripts.ai_events.remove(&entry);
    }
    let mut b = Build { data, next: BASE };
    // Broodlord Lashlayer.
    b.timer(
        NPC_LASHLAYER,
        1,
        LASHLAYER_CLEAVE,
        At::Victim,
        8000,
        (7000, 7000),
    );
    b.timer(
        NPC_LASHLAYER,
        2,
        KNOCK_AWAY,
        At::Victim,
        12000,
        (15000, 30000),
    );
    b.timer(NPC_LASHLAYER, 3, BLAST_WAVE, At::Me, 20000, (8000, 16000));
    b.timer(
        NPC_LASHLAYER,
        4,
        MORTAL_STRIKE,
        At::Victim,
        30000,
        (25000, 35000),
    );
    // Firemaw.
    b.timer(NPC_FIREMAW, 1, SHADOW_FLAME, At::Me, 18000, (15000, 18000));
    b.timer(NPC_FIREMAW, 2, WING_BUFFET, At::Me, 30000, (30000, 35000));
    b.timer(NPC_FIREMAW, 3, FLAME_BUFFET, At::Me, 5000, (5000, 5000));
    b.timer(NPC_FIREMAW, 4, THRASH, At::Me, 6000, (2000, 6000));
    b.buffet_threat(NPC_FIREMAW, 5);
    // Ebonroc.
    b.timer(
        NPC_EBONROC,
        1,
        SHADOW_OF_EBONROC,
        At::Victim,
        45000,
        (25000, 35000),
    );
    b.timer(NPC_EBONROC, 2, SHADOW_FLAME, At::Me, 18000, (15000, 18000));
    b.timer(NPC_EBONROC, 3, WING_BUFFET, At::Me, 30000, (30000, 35000));
    b.timer(NPC_EBONROC, 4, THRASH, At::Me, 6000, (2000, 6000));
    b.buffet_threat(NPC_EBONROC, 5);
    // Flamegor: his Frenzy with its emote.
    b.event(
        NPC_FLAMEGOR,
        1,
        EVENT_TIMER_IN_COMBAT,
        EF_REPEATABLE | EF_CHECK_RESULT,
        [10000, 10000, 10000, 15000],
        vec![
            cast_step(FRENZY, At::Me),
            ScriptStep {
                command: 0,
                dataint: [EMOTE_GENERIC_FRENZY as i32, 0, 0, 0],
                ..Default::default()
            },
        ],
    );
    b.timer(NPC_FLAMEGOR, 2, SHADOW_FLAME, At::Me, 18000, (15000, 18000));
    b.timer(NPC_FLAMEGOR, 3, WING_BUFFET, At::Me, 30000, (30000, 35000));
    b.timer(NPC_FLAMEGOR, 4, THRASH, At::Me, 6000, (2000, 6000));
    b.buffet_threat(NPC_FLAMEGOR, 5);
    for (entry, field) in PLAIN {
        b.home_fails(entry, 90, field);
    }
    razorgore::install(&mut b);
    vael::install(&mut b);
    chromaggus::install(&mut b);
    nefarian::install(&mut b);
    b.home_fails(NPC_RAZORGORE, 90, RAZORGORE);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy loaded again: what was under way starts over, and so does a
    /// lost Vaelastrasz (his intro wants him not started); another lost
    /// fight, and a won one, stay as they were.
    #[test]
    fn a_lost_vaelastrasz_starts_over_as_the_copy_loads() {
        let mut d: Data = [
            (RAZORGORE, FAILED),
            (VAELASTRASZ, FAILED),
            (LASHLAYER, IN_PROGRESS),
            (FIREMAW, DONE),
        ]
        .into();
        demote(&mut d);
        assert_eq!(d[&VAELASTRASZ], NOT_STARTED);
        assert_eq!(d[&LASHLAYER], NOT_STARTED);
        assert_eq!((d[&RAZORGORE], d[&FIREMAW]), (FAILED, DONE));
    }
}
