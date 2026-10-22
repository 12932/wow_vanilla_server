//! Instance scripts: a dungeon or raid copy's encounter data, and what it
//! does: doors that open when their boss falls, a boss's guard that stays
//! dead with it, Majordomo's arrival when Molten Core's runes are out. The
//! data is vmangos's (`map_template.script_name` names the script; boss
//! states 0 not started, 1 in progress, 2 failed, 3 done) and is saved with
//! the copy, so it outlives a restart. The adapters port Thistle Tea's;
//! the dungeons past the first five are in [`super::instance_adapters`].

// Places as Thistle Tea has them, to the digit.
#![allow(clippy::excessive_precision)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::GameData;
use crate::game::data::scripts::{ScriptStep, ScriptTable};
use crate::game::map::Map;

pub const NOT_STARTED: u32 = 0;
pub const IN_PROGRESS: u32 = 1;
pub const FAILED: u32 = 2;
pub const DONE: u32 = 3;

/// What a creature did, for the copy's script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Spawned,
    Aggro,
    Death,
    Evade,
}

/// The scripts ported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Deadmines,
    ShadowfangKeep,
    RazorfenKraul,
    ScarletMonastery,
    Maraudon,
    MoltenCore,
    WailingCaverns,
    BlackfathomDeeps,
    Gnomeregan,
    RazorfenDowns,
    ZulFarrak,
    Uldaman,
    SunkenTemple,
    Scholomance,
    Stratholme,
    BlackrockDepths,
    BlackrockSpire,
    DireMaul,
    OnyxiasLair,
    ZulGurub,
    BlackwingLair,
    RuinsOfAhnQiraj,
    /// A script with no port: its data is kept, and does nothing.
    Unported,
}

impl Script {
    pub fn by_name(name: &str) -> Option<Self> {
        Some(match name {
            "" => return None,
            "instance_deadmines" => Script::Deadmines,
            "instance_shadowfang_keep" => Script::ShadowfangKeep,
            "instance_razorfen_kraul" => Script::RazorfenKraul,
            "instance_scarlet_monastery" => Script::ScarletMonastery,
            "instance_maraudon" => Script::Maraudon,
            "instance_molten_core" => Script::MoltenCore,
            "instance_wailing_caverns" => Script::WailingCaverns,
            "instance_blackfathom_deeps" => Script::BlackfathomDeeps,
            "instance_gnomeregan" => Script::Gnomeregan,
            "instance_razorfen_downs" => Script::RazorfenDowns,
            "instance_zulfarrak" => Script::ZulFarrak,
            "instance_uldaman" => Script::Uldaman,
            "instance_sunken_temple" => Script::SunkenTemple,
            "instance_scholomance" => Script::Scholomance,
            "instance_stratholme" => Script::Stratholme,
            "instance_blackrock_depths" => Script::BlackrockDepths,
            "instance_blackrock_spire" => Script::BlackrockSpire,
            "instance_dire_maul" => Script::DireMaul,
            "instance_onyxia_lair" => Script::OnyxiasLair,
            "instance_zulgurub" => Script::ZulGurub,
            "instance_blackwing_lair" => Script::BlackwingLair,
            "instance_ruins_of_ahnqiraj" => Script::RuinsOfAhnQiraj,
            _ => Script::Unported,
        })
    }
}

/// When a door stands open.
#[derive(Debug, Clone, Copy)]
pub enum Open {
    /// Field done.
    Done(u32),
    /// Field at least this.
    AtLeast(u32, u32),
    /// As the rule says of the data.
    Rule(fn(&BTreeMap<u32, u32>) -> bool),
}

impl Open {
    fn holds(self, data: &BTreeMap<u32, u32>) -> bool {
        let v = |f: u32| data.get(&f).copied().unwrap_or(0);
        match self {
            Open::Done(f) => v(f) == DONE,
            Open::AtLeast(f, n) => v(f) >= n,
            Open::Rule(rule) => rule(data),
        }
    }
}

/// A copy's script and its data.
#[derive(Debug, Default)]
pub struct State {
    pub script: Option<Script>,
    pub data: BTreeMap<u32, u32>,
    /// Timed steps: (when, what).
    pub timers: Vec<(Instant, u32)>,
    /// Guids scripts stored (`SCRIPT_COMMAND_SET_INST_DATA64`).
    pub guids: ahash::AHashMap<u32, Guid>,
    /// What an adapter keeps for the copy's life, unsaved: counters and
    /// flags, the creatures it follows, and the spawns it has seen, by key.
    pub scratch: BTreeMap<u32, u32>,
    pub lists: ahash::AHashMap<u32, Vec<Guid>>,
    pub seen: ahash::AHashMap<u32, Vec<u32>>,
    /// Timed steps for one creature each: (when, what, whom).
    pub creature_timers: Vec<(Instant, u32, Guid)>,
    /// The spawns the script raised (chests, mostly), whose loot is saved
    /// with the copy.
    pub raised: ahash::AHashSet<u32>,
}

impl State {
    pub fn for_map(data: &GameData, map: u32, instance: u32) -> Self {
        if instance == 0 {
            return Self::default();
        }
        let script = data
            .scripts
            .map_templates
            .get(&map)
            .and_then(|t| Script::by_name(&t.script_name));
        Self {
            script,
            ..Default::default()
        }
    }
}

/// Boss deaths that settle an encounter field: (creature entry, field).
/// Others' scripts in the world database report their own.
fn bosses(s: Script) -> &'static [(u32, u32)] {
    match s {
        Script::Deadmines => &[(644, 10), (643, 11), (1763, 12)],
        Script::MoltenCore => &[
            (12098, 0),
            (12056, 1),
            (12264, 2),
            (11988, 3),
            (12057, 4),
            (11982, 5),
            (12259, 6),
            (12118, 7),
            (11502, 9),
        ],
        Script::Maraudon => &[(12225, 1)],
        _ => &[],
    }
}

/// The doors and buttons the script works, which no player may use in a
/// copy (Thistle Tea's `door_entries`).
fn door_entries(s: Script) -> &'static [u32] {
    match s {
        Script::ShadowfangKeep => &[18895, 18972, 18971],
        Script::RazorfenKraul => &[21099],
        Script::ScarletMonastery => &[104600],
        Script::BlackfathomDeeps => &[21117],
        Script::Gnomeregan => &[146086, 146085],
        Script::ZulFarrak => &[146084],
        Script::Uldaman => &[124372, 124367, 124369, 141869],
        Script::SunkenTemple => &[149431],
        Script::Scholomance => &[175570, 177371, 177372, 177373, 177375, 177376, 177377],
        Script::BlackrockDepths => &[
            170573, 170574, 170578, 170579, 170580, 170581, 170582, 170583, 170584, 161525, 161522,
            161524, 161523, 170575, 170577, 170576,
        ],
        Script::BlackrockSpire => &[
            175244, 175153, 164725, 175946, 175947, 175197, 175199, 175195, 175200, 175198, 175196,
            175194, 164726, 175185, 175186,
        ],
        Script::BlackwingLair => &super::bwl::DOOR_ENTRIES,
        Script::DireMaul => &[
            179503, 179506, 177220, 179502, 177259, 177257, 177258, 179504, 179505,
        ],
        _ => &[],
    }
}

/// Whether `object` is a door or button the copy's script works, which
/// players may not use.
pub fn scripted_door(map: &Map, object: Guid) -> bool {
    let Some(s) = map.instance.script else {
        return false;
    };
    map.key.instance != 0
        && map
            .gameobject(object)
            .is_some_and(|o| door_entries(s).contains(&o.entry))
}

/// Doors: (object entry, when it stands open).
fn doors(s: Script) -> &'static [(u32, Open)] {
    match s {
        Script::Deadmines => &[
            (13965, Open::Done(10)),
            (16400, Open::Done(11)),
            (16399, Open::Done(12)),
        ],
        Script::ShadowfangKeep => &[
            (18895, Open::Done(1)),
            (18972, Open::AtLeast(6, 4)),
            (18971, Open::Done(4)),
        ],
        Script::RazorfenKraul => &[(21099, Open::Done(1))],
        Script::ScarletMonastery => &[
            (
                104600,
                Open::Rule(|d| matches!(d.get(&SM_PAIR), Some(2..=4))),
            ),
            (101854, Open::Done(10)),
        ],
        _ => super::instance_adapters::doors(s),
    }
}

/// Guards that stay dead with their boss: (guard entry, boss field).
fn guards(s: Script) -> &'static [(u32, u32)] {
    match s {
        Script::MoltenCore => &[
            (12119, 7),
            (12057, 4),
            (11671, 5),
            (11673, 5),
            (11661, 6),
            (12099, 4),
            (12101, 4),
            (11672, 3),
            (11662, 0),
        ],
        _ => &[],
    }
}

/// Molten Core's runes: (rune object, its lieutenant's field, its own
/// field, its circle of fire's spawn).
const MC_RUNES: &[(u32, u32, u32, u32)] = &[
    (176951, 0, 16, 43157),
    (176952, 1, 17, 43158),
    (176953, 2, 18, 43159),
    (176954, 3, 19, 43160),
    (176955, 4, 20, 43165),
    (176956, 5, 21, 43161),
    (176957, 6, 22, 43163),
];
const MC_MAJORDOMO: u32 = 12018;
const RFK_KEEPERS_SLAIN: u32 = 100;
/// Scarlet Monastery's cathedral: its stage (field 1), its stages, and
/// the pair slain by bit (Mograine 1, Whitemane 2).
const SM_PAIR: u32 = 1;
const SM_DIED_ONCE: u32 = 2;
const SM_REVIVED: u32 = 3;
const SM_DONE: u32 = 4;
const SM_SLAIN: u32 = 100;
const SM_MOGRAINE: u32 = 3976;
const SM_WHITEMANE: u32 = 3977;
const MARAUDON_REDEEMED: u32 = 13716;
const MC_GOLEMAGG: u32 = 11988;
const MC_MAJORDOMO_FIELD: u32 = 8;
const MC_MAJORDOMO_SUMMONED: u32 = 23;
const MC_RAGNAROS: u32 = 11502;
const MC_RAGNAROS_FIELD: u32 = 9;
const MC_FIRELORD_CACHE: u32 = 362148;
// Timers.
const DM_IRON_CLAD_DOOR: u32 = 16397;
const DM_GUNPOWDER: u32 = 5000;
/// Deadmines' pirates who charge the door, by spawn.
const DM_PIRATES: [u32; 2] = [79289, 79290];
const T_DEADMINES_ALARM: u32 = 1;
const T_DEADMINES_ATTACK: u32 = 2;
const T_RAGNAROS_ROARS: u32 = 3;
const T_RAGNAROS_ENGAGES: u32 = 4;

/// A field's value before anything sets it, as Thistle Tea's
/// `initial_value` has it: Dire Maul's six tribute guards are all alive;
/// the rest start at 0.
pub fn initial_value(script: Option<Script>, field: u32) -> u32 {
    match (script, field) {
        (Some(Script::DireMaul), 15) => 6,
        _ => 0,
    }
}

/// A field of the copy's data: what it was set to, or its initial value.
/// Conditions read instance data through this.
pub fn get_data(map: &Map, field: u32) -> u32 {
    map.instance
        .data
        .get(&field)
        .copied()
        .unwrap_or_else(|| initial_value(map.instance.script, field))
}

/// The copy's script's data, as it was saved. What was under way when the
/// copy went (at a restart, or when it emptied and unloaded) starts over,
/// the vmangos way: its timers and summons went with it.
pub fn load(map: &mut Map, saved: &BTreeMap<u32, u32>) {
    map.instance.data = saved.clone();
    if let Some(s) = map.instance.script {
        demote(&mut map.instance.data, s);
    }
}

/// The fields an adapter's lost timers and summons left under way, back to
/// where they start.
fn demote(data: &mut BTreeMap<u32, u32>, s: Script) {
    let mut back = |field: u32, under_way: &dyn Fn(u32) -> bool| {
        if let Some(v) = data.get_mut(&field)
            && under_way(*v)
        {
            *v = NOT_STARTED;
        }
    };
    let in_progress = |v: u32| v == IN_PROGRESS;
    match s {
        // The pyramid's waves, and a Gahz'rilla called.
        Script::ZulFarrak => {
            back(1, &|v| (1..=7).contains(&v));
            back(2, &in_progress);
        }
        // The seal grinding open, the keepers and Archaedas awake.
        Script::Uldaman => {
            for f in [0, 1, 2, 11, 13, 14] {
                back(f, &in_progress);
            }
        }
        // Thermaplugg's fight and his bomb faces.
        Script::Gnomeregan => {
            back(1, &in_progress);
            for f in 10..16 {
                back(f, &|v| v != 0);
            }
        }
        // The Baron's run, Ramstein's abominations, the Baron's fight.
        Script::Stratholme => {
            back(0, &in_progress);
            back(4, &|v| matches!(v, IN_PROGRESS | 4));
            back(5, &in_progress);
        }
        // Kirtonos and Gandling called.
        Script::Scholomance => {
            back(0, &in_progress);
            back(7, &in_progress);
        }
        // The rookery's waves, and the stadium's.
        Script::BlackrockSpire => {
            back(6, &in_progress);
            back(super::ubrs_stadium::STADIUM, &in_progress);
        }
        // The Ring of Law, the Tomb of the Seven, the Lyceum, Magmus, the
        // dwarf runes.
        Script::BlackrockDepths => {
            back(0, &|v| matches!(v, IN_PROGRESS | FAILED));
            for f in [3, 4, 5, 43] {
                back(f, &in_progress);
            }
            back(46, &|v| v != 0);
        }
        // The cathedral's fight.
        Script::ScarletMonastery => back(SM_PAIR, &|v| (1..=3).contains(&v)),
        // Razorfen Downs' gong: the wave it called went with the copy, so
        // the gong is to be struck for it again, as it stood before (the
        // fiends from 0, the reavers from 9, Tuten'kash from 14, unless he
        // is dead).
        Script::RazorfenDowns => {
            let tutenkash_dead = data.get(&0) == Some(&DONE);
            if let Some(v) = data.get_mut(&1) {
                *v = match *v {
                    1..=8 => 0,
                    10..=13 => 9,
                    15.. if !tutenkash_dead => 14,
                    v => v,
                };
            }
        }
        Script::BlackwingLair => super::bwl::demote(data),
        // Majordomo called but not beaten: he is called again.
        Script::MoltenCore if data.get(&MC_MAJORDOMO_FIELD) != Some(&DONE) => {
            data.remove(&MC_MAJORDOMO_SUMMONED);
        }
        _ => {}
    }
}

/// After the spawns are in: doors, circles and guards as the data has
/// them.
pub fn settle_spawns(map: &mut Map) {
    let Some(s) = map.instance.script else {
        return;
    };
    doors_changed(map, s, &BTreeMap::new());
    // The Deadmines' iron-clad door stays blown.
    if s == Script::Deadmines && get_data(map, 1) > 0 {
        super::instance_adapters::operate(map, 16398, super::instance_adapters::Act::Open);
        super::instance_adapters::operate(
            map,
            DM_IRON_CLAD_DOOR,
            super::instance_adapters::Act::Destroy,
        );
        // Fired, but the copy went before its alarm: the alarm again, or
        // the pirates would never charge and the cannon not fire again.
        if get_data(map, 1) == 1 {
            schedule(map, T_DEADMINES_ALARM, Duration::from_secs(3));
        }
    }
    super::instance_adapters::settle_spawns(map, s);
    if s == Script::MoltenCore {
        for (_, _, rune, circle) in MC_RUNES {
            if get_data(map, *rune) == DONE
                && let Some(g) = crate::game::map::scripts::object_of_spawn(map, *circle)
            {
                map.remove_gameobject(g);
            }
        }
        summon_majordomo(map);
        if get_data(map, MC_MAJORDOMO_FIELD) == DONE {
            super::instance_adapters::raise_object(map, MC_FIRELORD_CACHE, 3_600_000);
        }
    }
}

/// An object come into the copy: a door of its script stands as its rule
/// has the copy's data now, as the copy's doors do at its start, so one
/// back after its rule flipped (a script took it away meanwhile) is as the
/// rule says (mangos's `OnObjectCreate`: a ziggurat's door made once its
/// boss is dead comes up open; Thistle Tea's reconciling of a spawned
/// object). Another keeps the state it spawned in.
pub fn object_added(map: &mut Map, object: Guid) {
    let Some(s) = map.instance.script else {
        return;
    };
    let Some(entry) = map.gameobject(object).map(|o| o.entry) else {
        return;
    };
    let Some((_, open)) = doors(s).iter().find(|(e, _)| *e == entry) else {
        return;
    };
    let now = open.holds(&map.instance.data);
    if open.holds(&BTreeMap::new()) != now {
        let act = if now {
            super::instance_adapters::Act::Open
        } else {
            super::instance_adapters::Act::Close
        };
        super::instance_adapters::operate_one(map, object, act);
    }
}

/// The doors whose rule the data `before` and the data now disagree on,
/// opened or shut to match it, as Thistle Tea's `Doors.changed` has it: a
/// door is only worked when its rule flips, and keeps its own state else.
fn doors_changed(map: &mut Map, s: Script, before: &BTreeMap<u32, u32>) {
    for (entry, open) in doors(s) {
        let now = open.holds(&map.instance.data);
        if open.holds(before) != now {
            let act = if now {
                super::instance_adapters::Act::Open
            } else {
                super::instance_adapters::Act::Close
            };
            super::instance_adapters::operate(map, *entry, act);
        }
    }
}

/// Set a field: `mode` 0 sets, 1 adds, 2 takes away. A done encounter
/// stays done.
pub fn set_data(map: &mut Map, field: u32, value: u32, mode: u32) {
    let Some(s) = map.instance.script else {
        return;
    };
    let old = get_data(map, field);
    // A set, or an add as the field's unsigned 32 bits wrap (mangos's
    // and Thistle Tea's increment), or a fall no lower than none; any
    // other mode is none (Thistle Tea's `ScriptStep` refuses it).
    let mut new = match mode {
        0 => value,
        1 => old.wrapping_add(value),
        2 => old.saturating_sub(value),
        _ => return,
    };
    if super::instance_adapters::set_data(map, s, field, new) {
        return;
    }
    if s == Script::ScarletMonastery && field == SM_PAIR {
        sm_set(map, new);
        return;
    }
    // Shadowfang's voidwalkers count up.
    if s == Script::ShadowfangKeep && field == 6 && mode == 0 {
        if value != DONE {
            return;
        }
        new = old + 1;
    }
    // Razorfen Kraul's ward keepers each report; the second drops the ward.
    if s == Script::RazorfenKraul && field == 1 && mode == 0 && old != DONE {
        let slain = get_data(map, RFK_KEEPERS_SLAIN) + 1;
        map.instance.data.insert(RFK_KEEPERS_SLAIN, slain);
        if slain < 2 {
            crate::game::instances::data_changed(&map.realm, map.key, &map.instance.data);
            return;
        }
    }
    if is_encounter(s, field) && old == DONE {
        new = DONE;
    }
    if new == old {
        return;
    }
    let before = map.instance.data.clone();
    map.instance.data.insert(field, new);
    doors_changed(map, s, &before);
    changed(map, s, field, old, new);
    crate::game::instances::data_changed(&map.realm, map.key, &map.instance.data);
}

/// A field set as it is, the doors it flips with it, and the copy saved.
pub fn store(map: &mut Map, field: u32, value: u32) {
    let before = map.instance.data.clone();
    map.instance.data.insert(field, value);
    if let Some(s) = map.instance.script {
        doors_changed(map, s, &before);
    }
    crate::game::instances::data_changed(&map.realm, map.key, &map.instance.data);
}

/// A timed step `key` of the copy's script, after `after`; one already
/// waiting under that key is called off, as Thistle Tea's `Schedule` has
/// it.
pub fn schedule(map: &mut Map, key: u32, after: Duration) {
    cancel(map, key);
    let at = map.now + after;
    map.instance.timers.push((at, key));
}

/// The timed steps `key` called off.
pub fn cancel(map: &mut Map, key: u32) {
    map.instance.timers.retain(|(_, k)| *k != key);
}

/// A timed step `key` for creature `g` alone, after `after`.
pub fn schedule_for(map: &mut Map, key: u32, g: Guid, after: Duration) {
    let at = map.now + after;
    map.instance.creature_timers.push((at, key, g));
}

/// Creature `g`'s timed steps called off.
pub fn cancel_for(map: &mut Map, g: Guid) {
    map.instance.creature_timers.retain(|(_, _, x)| *x != g);
}

fn is_encounter(s: Script, field: u32) -> bool {
    bosses(s).iter().any(|(_, f)| *f == field)
        || (s == Script::MoltenCore && field == MC_MAJORDOMO_FIELD)
        || (s == Script::ShadowfangKeep && matches!(field, 1..=5))
}

/// What a field's change brings.
fn changed(map: &mut Map, s: Script, field: u32, _old: u32, new: u32) {
    match (s, field, new) {
        // The Cache of the Firelord, which the world keeps back, for an
        // hour.
        (Script::MoltenCore, MC_MAJORDOMO_FIELD, DONE) => {
            super::instance_adapters::raise_object(map, MC_FIRELORD_CACHE, 3_600_000);
        }
        (Script::RazorfenKraul, 1, DONE) => release_agathelos(map),
        (Script::Maraudon, 1, DONE) => {
            for c in creatures_of(map, MARAUDON_REDEEMED) {
                set_concealed(map, c, false);
            }
        }
        _ => {}
    }
}

fn creatures_of(map: &Map, entry: u32) -> Vec<Guid> {
    map.creatures
        .iter()
        .filter(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
        .collect()
}

/// A pirate runs for the iron-clad door, off its patrol.
fn pirate_charges(map: &mut Map, g: Guid) {
    if let Some(c) = map.creature_mut(g) {
        c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
    }
    crate::game::map::scripts::move_to(
        map,
        g,
        Vector3d {
            x: -99.6611,
            y: -671.071655,
            z: 7.42241,
        },
        true,
    );
}

fn release_agathelos(map: &mut Map) {
    let now = map.now;
    for g in creatures_of(map, 4422) {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
            s.run = true;
        }
        if let Some(c) = map.creature_mut(g) {
            c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
        }
        crate::game::map::ai::rest(map, g, Some(now));
    }
}

/// A creature's event in the copy.
pub fn creature_event(map: &mut Map, g: Guid, event: Event) {
    let Some(s) = map.instance.script else {
        return;
    };
    let Some(c) = map.creature(g) else {
        return;
    };
    let entry = c.entry;
    let spawn = c.spawn_guid;
    // A guard of a boss that is dead goes when it spawns, for good.
    if event == Event::Spawned
        && let Some((_, field)) = guards(s).iter().find(|(e, _)| *e == entry)
        && get_data(map, *field) == DONE
    {
        despawn(map, g, true);
        return;
    }
    // A boss's death settles its encounter.
    if event == Event::Death
        && let Some((_, field)) = bosses(s).iter().find(|(e, _)| *e == entry)
    {
        set_data(map, *field, DONE, 0);
    }
    match (s, entry, event) {
        (Script::RazorfenKraul, 4422, Event::Spawned) if get_data(map, 1) == DONE => {
            release_agathelos(map)
        }
        // The pirates at the door charge it once the alarm has gone.
        (Script::Deadmines, 657, Event::Spawned)
            if spawn.is_some_and(|s| DM_PIRATES.contains(&s))
                && matches!(get_data(map, 1), 2 | 3) =>
        {
            pirate_charges(map, g)
        }
        // Scarlet Monastery: Herod's door, and the cathedral's pair: both
        // slain settles them; one back from the dead is slain no longer,
        // and once settled neither comes back.
        (Script::ScarletMonastery, 3975, Event::Death) => set_data(map, 10, DONE, 0),
        (Script::ScarletMonastery, SM_MOGRAINE | SM_WHITEMANE, Event::Death) => {
            let slain = get_data(map, SM_SLAIN) | sm_bit(entry);
            map.instance.data.insert(SM_SLAIN, slain);
            if slain == 3 {
                sm_store(map, SM_DONE);
            } else {
                crate::game::instances::data_changed(&map.realm, map.key, &map.instance.data);
            }
        }
        (Script::ScarletMonastery, SM_MOGRAINE | SM_WHITEMANE, Event::Spawned) => {
            if get_data(map, SM_PAIR) == SM_DONE {
                despawn(map, g, true);
            } else if get_data(map, SM_SLAIN) & sm_bit(entry) != 0 {
                let slain = get_data(map, SM_SLAIN) & !sm_bit(entry);
                store(map, SM_SLAIN, slain);
            }
        }
        // Maraudon: Celebras the Redeemed keeps hidden until the Cursed
        // falls.
        (Script::Maraudon, MARAUDON_REDEEMED, Event::Spawned) if get_data(map, 1) != DONE => {
            set_concealed(map, g, true)
        }
        // Molten Core: Majordomo's death has Ragnaros roar.
        (Script::MoltenCore, MC_MAJORDOMO, Event::Death) => {
            set_data(map, MC_MAJORDOMO_FIELD, DONE, 0);
            schedule(map, T_RAGNAROS_ROARS, Duration::from_secs(7));
            // A summon's corpse that goes ten seconds after the death.
            let at = map.now + Duration::from_secs(10);
            if spawn.is_none()
                && let Some(s) = crate::game::map::event_ai::scripted_mut(map, g)
            {
                s.despawn_at = Some(at);
                s.despawn_type = 0;
            }
        }
        (Script::MoltenCore, MC_GOLEMAGG, Event::Aggro) if get_data(map, 3) != DONE => {
            set_data(map, 3, IN_PROGRESS, 0)
        }
        // Golemagg's fight resets with both his ragers back as new.
        (Script::MoltenCore, MC_GOLEMAGG, Event::Spawned | Event::Evade)
            if get_data(map, 3) != DONE =>
        {
            set_data(map, 3, NOT_STARTED, 0);
            for spawn in [56685, 56699] {
                if let Some(r) = crate::game::map::scripts::creature_of_spawn(map, spawn) {
                    if map.creature(r).is_some_and(|c| c.alive()) {
                        crate::game::map::ai::quiet_death(map, r);
                    }
                    crate::game::map::ai::respawn_now(map, r);
                }
            }
        }
        // Golemagg's ragers go with him.
        (Script::MoltenCore, MC_GOLEMAGG, Event::Death) => {
            for r in creatures_of(map, 11672) {
                despawn(map, r, true);
            }
        }
        _ => {}
    }
    super::instance_adapters::creature_event(map, s, g, entry, spawn, event);
}

/// A creature the copy's script sends away: a summoned one for good; a
/// spawned one until its spawn's own respawn time comes round, as Thistle
/// Tea's `despawn` has it, or, `held`, for as long as the copy lasts (a
/// dead boss's guard).
pub(super) fn despawn(map: &mut Map, g: Guid, held: bool) {
    let Some((spawn, alive)) = map.creature(g).map(|c| (c.spawn_guid, c.alive())) else {
        return;
    };
    crate::game::map::scripts::despawn(map, g);
    let (Some(spawn), true) = (spawn, alive) else {
        return;
    };
    let wait = if held {
        Duration::from_secs(10 * 365 * 86_400)
    } else {
        spawn_respawn(map, spawn)
    };
    let now = map.now;
    if let Some(c) = map.creature_mut(g)
        && let crate::game::map::creature::Life::Dead { respawn_at, .. } = &mut c.life
    {
        *respawn_at = now + wait;
    }
}

/// How long a spawn waits to come back: its own respawn time.
fn spawn_respawn(map: &Map, spawn: u32) -> Duration {
    map.realm
        .data
        .creature_spawns
        .get(&map.key.map)
        .and_then(|v| v.iter().find(|s| s.guid == spawn))
        .map_or(Duration::from_secs(300), |s| {
            Duration::from_secs(u64::from(s.respawn_min.max(1)))
        })
}

/// A creature hidden from players, or shown again.
fn set_concealed(map: &mut Map, g: Guid, on: bool) {
    let step = ScriptStep {
        command: crate::game::map::scripts::C_SET_CONCEALED,
        datalong: [u32::from(on), 0, 0, 0],
        ..Default::default()
    };
    crate::game::map::scripts::execute(map, &step, g, g);
}

fn sm_bit(entry: u32) -> u32 {
    if entry == SM_MOGRAINE { 1 } else { 2 }
}

/// The cathedral's stage set, its door with it.
fn sm_store(map: &mut Map, value: u32) {
    store(map, SM_PAIR, value);
}

/// Scarlet Monastery's cathedral, as Thistle Tea stages Mograine and
/// Whitemane: settled stays settled; reset with one of them slain, it
/// settles and the other goes; else each stage brings what it brings.
fn sm_set(map: &mut Map, value: u32) {
    let current = get_data(map, SM_PAIR);
    if current == SM_DONE {
        return;
    }
    let slain = get_data(map, SM_SLAIN);
    if value == NOT_STARTED && slain != 0 {
        sm_store(map, SM_DONE);
        for (entry, bit) in [(SM_MOGRAINE, 1), (SM_WHITEMANE, 2)] {
            if slain & bit == 0 {
                for c in creatures_of(map, entry) {
                    despawn(map, c, true);
                }
            }
        }
        return;
    }
    sm_store(map, value);
    let whitemane_slain = slain & 2 != 0;
    match (current, value) {
        // The chapel's defenders near Mograine join his fight.
        (_, IN_PROGRESS) => {
            let post = Vector3d {
                x: 1153.87,
                y: 1398.39,
                z: 32.61,
            };
            for entry in [4299, 4300, 4301, 4302, 4303, 4540] {
                for c in crate::game::map::scripts::creatures_near(map, post, entry, 82.0, true) {
                    zone_pulse(map, c);
                }
            }
        }
        // Reset after his first fall: Whitemane is back.
        (SM_DIED_ONCE, NOT_STARTED) if !whitemane_slain => {
            for c in map
                .creatures
                .iter()
                .filter(|(_, c)| c.entry == SM_WHITEMANE)
                .map(|(_, c)| c.guid)
                .collect::<Vec<_>>()
            {
                crate::game::map::ai::respawn_now(map, c);
            }
        }
        // He falls: she cries out and runs to raise him.
        (_, SM_DIED_ONCE) if !whitemane_slain => {
            for w in creatures_of(map, SM_WHITEMANE) {
                crate::game::map::scripts::talk(map, w, 2973, Guid::zero(), None);
                let step = ScriptStep {
                    command: 3,
                    datalong: [2, 0, 69, 2],
                    dataint: [100, 0, 0, 0],
                    target_type: 12,
                    target_param1: 2,
                    x: 3.0,
                    ..Default::default()
                };
                crate::game::map::scripts::execute(map, &step, w, w);
            }
        }
        // Raised, both turn on everyone in the cathedral.
        (_, SM_REVIVED) => {
            for entry in [SM_MOGRAINE, SM_WHITEMANE] {
                for c in creatures_of(map, entry) {
                    zone_pulse(map, c);
                }
            }
        }
        _ => {}
    }
}

/// Everyone in the copy is its enemy.
fn zone_pulse(map: &mut Map, g: Guid) {
    let step = ScriptStep {
        command: 49,
        datalong: [1, 0, 0, 0],
        ..Default::default()
    };
    crate::game::map::scripts::execute(map, &step, g, g);
}

/// A game object used in the copy.
pub fn object_used(map: &mut Map, entry: u32) {
    let Some(s) = map.instance.script else {
        return;
    };
    match (s, entry) {
        // Molten Core: a rune doused, its lieutenant dead, puts out its
        // circle; all seven bring Majordomo.
        (Script::MoltenCore, _) => {
            let Some((_, boss, rune, circle)) = MC_RUNES.iter().find(|r| r.0 == entry).copied()
            else {
                return;
            };
            if get_data(map, boss) != DONE {
                return;
            }
            set_data(map, rune, DONE, 0);
            if let Some(g) = crate::game::map::scripts::object_of_spawn(map, circle) {
                map.remove_gameobject(g);
            }
            summon_majordomo(map);
        }
        // Deadmines: the cannon fired blows the iron-clad door and starts
        // the pirates' assault.
        (Script::Deadmines, 16398) if get_data(map, 1) == 0 => {
            set_data(map, 1, 1, 0);
            super::instance_adapters::operate(map, 16398, super::instance_adapters::Act::Open);
            super::instance_adapters::operate(
                map,
                DM_IRON_CLAD_DOOR,
                super::instance_adapters::Act::Destroy,
            );
            schedule(map, T_DEADMINES_ALARM, Duration::from_secs(3));
        }
        // The Defias Gunpowder brings a Defias Blackguard down the ramp.
        (Script::Deadmines, 17155) if get_data(map, DM_GUNPOWDER) == 0 => {
            set_data(map, DM_GUNPOWDER, 1, 0);
            if let Some(g) = crate::game::map::scripts::summon(
                map,
                Guid::zero(),
                634,
                Vector3d {
                    x: -131.290833,
                    y: -591.243103,
                    z: 18.077190,
                },
                4.792192,
                Some((Duration::from_millis(310_000), 3)),
            ) {
                if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
                    s.summoner = None;
                }
                crate::game::map::scripts::move_to(
                    map,
                    g,
                    Vector3d {
                        x: -115.263672,
                        y: -617.396118,
                        z: 13.579387,
                    },
                    false,
                );
            }
        }
        _ => super::instance_adapters::object_used(map, s, entry),
    }
}

/// With every rune out, Majordomo comes to his hall, unless he has come
/// already, or he or Ragnaros is beaten.
fn summon_majordomo(map: &mut Map) {
    let all_out = MC_RUNES.iter().all(|r| get_data(map, r.2) == DONE);
    if !all_out
        || get_data(map, MC_RAGNAROS_FIELD) == DONE
        || get_data(map, MC_MAJORDOMO_SUMMONED) == DONE
        || get_data(map, MC_MAJORDOMO_FIELD) == DONE
    {
        return;
    }
    set_data(map, MC_MAJORDOMO_SUMMONED, DONE, 0);
    // No one's summon; his corpse goes ten seconds after he falls.
    if let Some(g) = crate::game::map::scripts::summon(
        map,
        Guid::zero(),
        MC_MAJORDOMO,
        Vector3d {
            x: 758.089,
            y: -1176.71,
            z: -118.640,
        },
        3.12414,
        None,
    ) && let Some(s) = crate::game::map::event_ai::scripted_mut(map, g)
    {
        s.summoner = None;
    }
}

/// The copy's timed steps.
pub fn update(map: &mut Map) {
    if !map.instance.creature_timers.is_empty() {
        let now = map.now;
        let due: Vec<(u32, Guid)> = map
            .instance
            .creature_timers
            .iter()
            .filter(|(t, _, _)| *t <= now)
            .map(|(_, k, g)| (*k, *g))
            .collect();
        map.instance.creature_timers.retain(|(t, _, _)| *t > now);
        if let Some(s) = map.instance.script {
            for (k, g) in due {
                super::instance_adapters::creature_timer(map, s, k, g);
            }
        }
    }
    if map.instance.timers.is_empty() {
        return;
    }
    let now = map.now;
    let due: Vec<u32> = map
        .instance
        .timers
        .iter()
        .filter(|(t, _)| *t <= now)
        .map(|(_, k)| *k)
        .collect();
    map.instance.timers.retain(|(t, _)| *t > now);
    for k in due {
        timer(map, k);
    }
}

fn timer(map: &mut Map, key: u32) {
    if key >= super::instance_adapters::TIMERS
        && let Some(s) = map.instance.script
    {
        super::instance_adapters::timer(map, s, key);
        return;
    }
    match key {
        T_DEADMINES_ALARM if get_data(map, 1) == 1 => {
            set_data(map, 1, 2, 0);
            if let Some(g) = creatures_of(map, 646).into_iter().next() {
                crate::game::map::scripts::talk(map, g, 1148, Guid::zero(), None);
            }
            for spawn in DM_PIRATES {
                if let Some(g) = crate::game::map::scripts::creature_of_spawn(map, spawn) {
                    pirate_charges(map, g);
                }
            }
            schedule(map, T_DEADMINES_ATTACK, Duration::from_secs(15));
        }
        T_DEADMINES_ATTACK if get_data(map, 1) == 2 => {
            set_data(map, 1, 3, 0);
            if let Some(g) = creatures_of(map, 646).into_iter().next() {
                crate::game::map::scripts::talk(map, g, 1149, Guid::zero(), None);
            }
        }
        // His roar: the line, and the emote with it.
        T_RAGNAROS_ROARS => {
            for g in creatures_of(map, MC_RAGNAROS) {
                crate::game::map::scripts::talk(map, g, 7685, Guid::zero(), None);
                let step = ScriptStep {
                    command: 1,
                    datalong: [15, 0, 0, 0],
                    ..Default::default()
                };
                crate::game::map::scripts::execute(map, &step, g, g);
            }
            schedule(map, T_RAGNAROS_ENGAGES, Duration::from_secs(3));
        }
        // Players may strike him now, and everyone in the copy is his
        // enemy.
        T_RAGNAROS_ENGAGES => {
            for g in creatures_of(map, MC_RAGNAROS) {
                if let Some(c) = map.creature_mut(g) {
                    let flags = c.fields.get_u32(crate::game::fields::UNIT_FIELD_FLAGS);
                    c.fields
                        .set_u32(crate::game::fields::UNIT_FIELD_FLAGS, flags & !0x100);
                }
                zone_pulse(map, g);
            }
        }
        _ => {}
    }
}

/// `target_type` 12: a creature an instance script knows. Here, the
/// index names an entry.
pub fn guid_data(map: &Map, index: u32) -> Option<Guid> {
    if let Some(g) = map.instance.guids.get(&index) {
        return Some(*g);
    }
    let entry = match (map.instance.script, index) {
        (Some(Script::ScarletMonastery), 2) => 3976,
        (Some(Script::ScarletMonastery), 3) => 3977,
        (Some(Script::MoltenCore), 12) => 11988,
        (_, i) => i,
    };
    map.creatures
        .iter()
        .find(|(_, c)| c.entry == entry)
        .map(|(_, c)| c.guid)
}

/// A living player stepped into area trigger `id` here (mangos's
/// `AreaTrigger_at_*` scripts the copy's script has).
pub fn area_trigger(map: &mut Map, player: Guid, id: u32) {
    if map.key.instance == 0 {
        return;
    }
    match map.instance.script {
        Some(Script::BlackrockSpire) => super::ubrs_stadium::area_trigger(map, player, id),
        Some(Script::BlackwingLair) => super::bwl::area_trigger(map, player, id),
        _ => {}
    }
}

/// A script that runs on a map event from `event_scripts`: started by the
/// copy, from its first creature.
pub fn run_event(map: &mut Map, event: u32, target: Guid) {
    let source = map
        .creatures
        .iter()
        .next()
        .map(|(_, c)| c.guid)
        .unwrap_or(target);
    crate::game::map::scripts::start(map, ScriptTable::Event, event, source, target);
}

/// A chest the copy's script raised: what it holds now is saved with the
/// copy, so one raised again after a reload keeps what was left in it.
/// One whose loot is gone (an ordinary chest that fills again) is
/// forgotten, to fill anew.
pub fn chest_changed(map: &Map, object: Guid) {
    if map.key.instance == 0 {
        return;
    }
    let Some(o) = map.gameobject(object) else {
        return;
    };
    let Some(spawn) = o.spawn_guid.filter(|s| map.instance.raised.contains(s)) else {
        return;
    };
    let chest = o.loot.as_ref().map(|l| crate::game::instances::SavedChest {
        items: l
            .items
            .iter()
            .map(|s| crate::game::instances::SavedSlot {
                item: s.item,
                count: s.count,
                taken: s.taken,
                random_property: s.random_property,
                quest: s.quest,
                taken_by: s
                    .taken_by
                    .iter()
                    .map(|g| crate::game::guid::counter(*g))
                    .collect(),
                quality: s.quality,
                condition: s.condition,
            })
            .collect(),
        gold: l.gold,
        back_at: None,
    });
    crate::game::instances::chest_changed(&map.realm, map.key, spawn, chest);
}

/// A raised chest emptied and gone, back (filled anew) after `after`.
pub fn chest_gone(map: &Map, spawn: u32, after: Duration) {
    if map.key.instance == 0 || !map.instance.raised.contains(&spawn) {
        return;
    }
    let chest = crate::game::instances::SavedChest {
        back_at: Some(crate::game::social::mail::now() + after.as_secs() as i64),
        ..Default::default()
    };
    crate::game::instances::chest_changed(&map.realm, map.key, spawn, Some(chest));
}

/// How a raised chest was left before the copy went: its loot, or, emptied
/// and not yet back, how long until it is.
pub enum Left {
    Fresh,
    Loot(Box<crate::game::map::loot::Loot>),
    Gone(Duration),
}

/// What the save says of raised chest `spawn`.
pub fn chest_left(map: &Map, spawn: u32) -> Left {
    let Some(chest) = crate::game::instances::save_of(&map.realm, map.key.instance)
        .and_then(|s| s.chests.get(&spawn).cloned())
    else {
        return Left::Fresh;
    };
    if let Some(back) = chest.back_at {
        let left = back - crate::game::social::mail::now();
        return if left > 0 {
            Left::Gone(Duration::from_secs(left as u64))
        } else {
            Left::Fresh
        };
    }
    let player = |c: u32| crate::game::guid::player(c);
    Left::Loot(Box::new(crate::game::map::loot::Loot {
        items: chest
            .items
            .into_iter()
            .map(|s| crate::game::map::loot::LootSlot {
                item: s.item,
                count: s.count,
                taken: s.taken,
                random_property: s.random_property,
                quest: s.quest,
                taken_by: s.taken_by.into_iter().map(player).collect(),
                quality: s.quality,
                condition: s.condition,
                ..Default::default()
            })
            .collect(),
        gold: chest.gold,
        ..Default::default()
    }))
}
