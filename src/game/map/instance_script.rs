//! Instance scripts: a dungeon or raid copy's encounter data, and what it
//! does: doors that open when their boss falls, a boss's guard that stays
//! dead with it, Majordomo's arrival when Molten Core's runes are out. The
//! data is vmangos's (`map_template.script_name` names the script; boss
//! states 0 not started, 1 in progress, 2 failed, 3 done) and is saved with
//! the copy, so it outlives a restart. The adapters port Thistle Tea's;
//! the dungeons past the first five are in [`super::instance_adapters`].

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptTable;
use crate::game::fields::GAMEOBJECT_STATE;
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
        Script::ScarletMonastery => &[(104600, Open::AtLeast(1, 2)), (101854, Open::Done(10))],
        _ => super::instance_adapters::doors(s),
    }
}

/// Guards that stay dead with their boss: (guard entry, boss field).
fn guards(s: Script) -> &'static [(u32, u32)] {
    match s {
        Script::MoltenCore => &[
            (12119, 7),
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
const SM_SLAIN: u32 = 100;
const MC_MAJORDOMO_FIELD: u32 = 8;
const MC_MAJORDOMO_SUMMONED: u32 = 23;
const MC_RAGNAROS: u32 = 11502;
const MC_FIRELORD_CACHE: u32 = 362148;
// Timers.
const T_DEADMINES_ALARM: u32 = 1;
const T_DEADMINES_ATTACK: u32 = 2;
const T_RAGNAROS_ROARS: u32 = 3;
const T_RAGNAROS_ENGAGES: u32 = 4;

pub fn get_data(map: &Map, field: u32) -> u32 {
    map.instance.data.get(&field).copied().unwrap_or(0)
}

/// The copy's script's data, as it was saved, and its doors to match.
pub fn load(map: &mut Map, saved: &BTreeMap<u32, u32>) {
    map.instance.data = saved.clone();
}

/// After the spawns are in: doors, circles and guards as the data has
/// them.
pub fn settle_spawns(map: &mut Map) {
    let Some(s) = map.instance.script else {
        return;
    };
    apply_doors(map, s);
    super::instance_adapters::settle_spawns(map, s);
    if s == Script::MoltenCore {
        for (_, _, rune, circle) in MC_RUNES {
            if get_data(map, *rune) == DONE
                && let Some(g) = crate::game::map::scripts::object_of_spawn(map, *circle)
            {
                map.remove_gameobject(g);
            }
        }
    }
}

fn apply_doors(map: &mut Map, s: Script) {
    for (entry, open) in doors(s) {
        let state = if open.holds(&map.instance.data) { 0 } else { 1 };
        let objects: Vec<Guid> = map
            .gameobjects
            .iter()
            .filter(|(_, o)| o.entry == *entry)
            .map(|(_, o)| o.guid)
            .collect();
        for g in objects {
            if let Some(o) = map.gameobject_mut(g)
                && o.fields.get_u32(GAMEOBJECT_STATE) != state
            {
                o.fields.set_u32(GAMEOBJECT_STATE, state);
            }
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
    let mut new = match mode {
        1 => old.saturating_add(value),
        2 => old.saturating_sub(value),
        _ => value,
    };
    if super::instance_adapters::set_data(map, s, field, new) {
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
    map.instance.data.insert(field, new);
    changed(map, s, field, old, new);
    apply_doors(map, s);
    crate::game::instances::data_changed(&map.realm, map.key, &map.instance.data);
}

/// A field set as it is, its doors with it, and the copy saved.
pub fn store(map: &mut Map, field: u32, value: u32) {
    map.instance.data.insert(field, value);
    if let Some(s) = map.instance.script {
        apply_doors(map, s);
    }
    crate::game::instances::data_changed(&map.realm, map.key, &map.instance.data);
}

/// A timed step `key` of the copy's script, after `after`.
pub fn schedule(map: &mut Map, key: u32, after: Duration) {
    let at = map.now + after;
    map.instance.timers.push((at, key));
}

/// The timed steps `key` called off.
pub fn cancel(map: &mut Map, key: u32) {
    map.instance.timers.retain(|(_, k)| *k != key);
}

fn is_encounter(s: Script, field: u32) -> bool {
    bosses(s).iter().any(|(_, f)| *f == field)
        || (s == Script::MoltenCore && field == MC_MAJORDOMO_FIELD)
        || (s == Script::ShadowfangKeep && matches!(field, 1..=5))
}

/// What a field's change brings.
fn changed(map: &mut Map, s: Script, field: u32, _old: u32, new: u32) {
    match (s, field, new) {
        (Script::MoltenCore, MC_MAJORDOMO_FIELD, DONE) => {
            if let Some(g) = crate::game::map::scripts::object_of_spawn(map, MC_FIRELORD_CACHE) {
                crate::game::map::scripts::set_object_state(map, g, 1);
                map.visibility_dirty.insert(g);
            }
        }
        (Script::RazorfenKraul, 1, DONE) => release_agathelos(map),
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

fn release_agathelos(map: &mut Map) {
    let now = map.now;
    for g in creatures_of(map, 4422) {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
            s.run = true;
        }
        if let Some(c) = map.creature_mut(g) {
            c.movement_type = crate::game::map::creature::MOVEMENT_WAYPOINT;
            c.motion = crate::game::map::creature::Motion::Idle { until: Some(now) };
        }
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
    // A guard of a boss that is dead goes when it spawns.
    if event == Event::Spawned
        && let Some((_, field)) = guards(s).iter().find(|(e, _)| *e == entry)
        && get_data(map, *field) == DONE
    {
        crate::game::map::scripts::despawn(map, g);
        return;
    }
    if let Some((_, field)) = bosses(s).iter().find(|(e, _)| *e == entry) {
        let field = *field;
        match event {
            Event::Death => set_data(map, field, DONE, 0),
            Event::Aggro if get_data(map, field) != DONE => set_data(map, field, IN_PROGRESS, 0),
            Event::Evade if get_data(map, field) != DONE => set_data(map, field, FAILED, 0),
            _ => {}
        }
    }
    match (s, entry, event) {
        (Script::RazorfenKraul, 4422, Event::Spawned) if get_data(map, 1) == DONE => {
            release_agathelos(map)
        }
        // Scarlet Monastery: Herod's door, and the cathedral's pair.
        (Script::ScarletMonastery, 3975, Event::Death) => set_data(map, 10, DONE, 0),
        (Script::ScarletMonastery, 3976 | 3977, Event::Death) => {
            let bit = if entry == 3976 { 1 } else { 2 };
            let slain = get_data(map, SM_SLAIN) | bit;
            map.instance.data.insert(SM_SLAIN, slain);
            if slain == 3 {
                set_data(map, 1, 4, 0);
            } else {
                crate::game::instances::data_changed(&map.realm, map.key, &map.instance.data);
            }
        }
        // Molten Core: Majordomo's death has Ragnaros roar.
        (Script::MoltenCore, MC_MAJORDOMO, Event::Death) => {
            set_data(map, MC_MAJORDOMO_FIELD, DONE, 0);
            map.instance
                .timers
                .push((map.now + Duration::from_secs(7), T_RAGNAROS_ROARS));
        }
        // Golemagg's ragers go with him.
        (Script::MoltenCore, 11988, Event::Death) => {
            for r in creatures_of(map, 11672) {
                crate::game::map::scripts::despawn(map, r);
            }
        }
        _ => {}
    }
    super::instance_adapters::creature_event(map, s, g, entry, spawn, event);
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
            let all_out = MC_RUNES.iter().all(|r| get_data(map, r.2) == DONE);
            if all_out
                && get_data(map, MC_MAJORDOMO_SUMMONED) != DONE
                && get_data(map, MC_MAJORDOMO_FIELD) != DONE
            {
                set_data(map, MC_MAJORDOMO_SUMMONED, DONE, 0);
                let source = map
                    .creatures
                    .iter()
                    .next()
                    .map(|(_, c)| c.guid)
                    .unwrap_or_default();
                crate::game::map::scripts::summon(
                    map,
                    source,
                    MC_MAJORDOMO,
                    Vector3d {
                        x: 758.089,
                        y: -1176.71,
                        z: -118.640,
                    },
                    3.12414,
                    None,
                );
            }
        }
        // Deadmines: the iron-clad door, opened, starts the pirates'
        // assault.
        (Script::Deadmines, 16398) if get_data(map, 1) == 0 => {
            set_data(map, 1, 1, 0);
            if let Some(g) = map
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == 16397)
                .map(|(_, o)| o.guid)
            {
                map.remove_gameobject(g);
            }
            map.instance
                .timers
                .push((map.now + Duration::from_secs(3), T_DEADMINES_ALARM));
        }
        _ => super::instance_adapters::object_used(map, s, entry),
    }
}

/// The copy's timed steps.
pub fn update(map: &mut Map) {
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
            map.instance
                .timers
                .push((map.now + Duration::from_secs(15), T_DEADMINES_ATTACK));
        }
        T_DEADMINES_ATTACK if get_data(map, 1) == 2 => {
            set_data(map, 1, 3, 0);
            if let Some(g) = creatures_of(map, 646).into_iter().next() {
                crate::game::map::scripts::talk(map, g, 1149, Guid::zero(), None);
            }
        }
        T_RAGNAROS_ROARS => {
            if let Some(g) = creatures_of(map, MC_RAGNAROS).into_iter().next() {
                crate::game::map::scripts::talk(map, g, 7685, Guid::zero(), None);
            }
            map.instance
                .timers
                .push((map.now + Duration::from_secs(3), T_RAGNAROS_ENGAGES));
        }
        T_RAGNAROS_ENGAGES => {
            if let Some(g) = creatures_of(map, MC_RAGNAROS).into_iter().next() {
                if let Some(c) = map.creature_mut(g) {
                    let flags = c.fields.get_u32(crate::game::fields::UNIT_FIELD_FLAGS);
                    c.fields
                        .set_u32(crate::game::fields::UNIT_FIELD_FLAGS, flags & !0x100);
                }
                let players: Vec<Guid> = map.players.keys().copied().collect();
                for p in players {
                    crate::game::map::ai::attacked(map, g, p);
                }
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
