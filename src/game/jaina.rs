//! Lady Jaina Proudmoore comes to her city (`[guards] jaina_city`,
//! Stormwind's by default) when its alarm reaches `[guards] jaina_from`
//! (`map/guards.rs`), if she is alive and at home in Theramore: she says
//! she is needed, teleports out (spell 12980's visual, as mangos's own
//! scenes of her bring her in and out), and comes in by the fight with the
//! same visual, to fight the city's attackers. Once no one is left for her
//! and she has been out of combat a while, she goes home the same way. Dead
//! in the city, her corpse lies there as any creature's, and she is back in
//! Theramore after `[guards] jaina_respawn_secs`.
//!
//! She is in one place at a time: the realm knows where ([`State`], behind
//! its lock in `Realm::social`), and the two maps hand her over by calls.
//! Her spawn at home stays dead (unseen, its respawn put off) while she is
//! away.
//!
//! Her fight is Thistle Tea's script of her, imported with the rest
//! (`data/tt_scripts.rs`); in the city her Teleport sends its victim to
//! the Stockade's door rather than to the top of her tower in Theramore,
//! and her 40-yard rule is about that door.

use std::time::{Duration, Instant};

use tracing::info;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapKey};

/// Her entry, and her spawn in Theramore.
pub const JAINA: u32 = 4968;
pub const SPAWN: u32 = 7895;
pub const HOME_MAP: u32 = 1;
/// mangos's `SPELL_TELEPORT_VISUAL`, "Simple Teleport".
pub const TELEPORT_VISUAL: u32 = 12980;
/// Her Teleport, to her tower's top (`spell_target_position`).
pub const TELEPORT: u32 = 20682;
/// Stormwind City, and its prison's door: where the Stockade's exit puts
/// a player out (area trigger 503's target, as the data has it, else as
/// vmangos's has it).
const STORMWIND: u32 = 1519;
const STOCKADE_EXIT: u32 = 503;
const STOCKADE_DOOR: (Vector3d, f32) = (
    Vector3d {
        x: -8766.11,
        y: 845.499,
        z: 87.9952,
    },
    3.83972,
);
/// How long the teleport's flash shows before she goes.
pub const VANISH: Duration = Duration::from_millis(1500);
/// How long her flash as she comes waits for those near to know her.
const FLASH_WAIT: Duration = Duration::from_secs(1);
/// How near the fight she comes.
const ARRIVE_WITHIN: f32 = 8.0;
/// How often a map looks after her.
const LOOK_EVERY: Duration = Duration::from_millis(500);
/// A spawn put off "for good".
const NEVER: Duration = Duration::from_secs(10 * 365 * 86_400);

const LEAVING: &[&str] = &[
    "I have urgent matters to tend to. Tervosh, Theramore is in your hands until I return.",
    "Stormwind calls for aid. I will not be long.",
    "Forgive me, I am needed elsewhere. Keep watch while I am gone.",
];
const ARRIVING: &[&str] = &[
    "You dare bring war to the heart of Stormwind? Not while I stand!",
    "Stand fast, Stormwind! Help has come.",
    "Enough of this! Leave this city, or I will see you leave it.",
];
const GOING: &[&str] = &[
    "The city is safe for now. Theramore needs me.",
    "Stormwind can look after itself again. I must return home.",
];

/// Where she is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// In Theramore, alive or dead.
    #[default]
    Home,
    /// Called: Theramore is seeing her off.
    Leaving,
    /// In her city, on `map`, as `copy`.
    Away { map: MapKey, copy: Guid },
    /// Saying farewell to her city.
    Returning,
}

/// What the realm knows of her.
#[derive(Debug, Default)]
pub struct State {
    pub place: Place,
    /// When she was called, while she is leaving: a call lost (a map's
    /// actor gone) gives way after [`STALE`].
    pub called_at: Option<Instant>,
    /// Her visits to her city.
    pub visits: u32,
}

/// How long Theramore may take to see her off before her call is
/// forgotten.
const STALE: Duration = Duration::from_secs(30);

/// What a map keeps of her comings and goings through it.
#[derive(Debug, Default)]
pub struct Local {
    /// Theramore: her going, at its time, to `to` near `at`, against `foe`.
    pub departing: Option<Departure>,
    /// Her city: her going home, at its time.
    pub returning: Option<(Instant, Guid)>,
    /// Her city: since when she has been at peace.
    pub calm_since: Option<Instant>,
    pub look_at: Option<Instant>,
    /// Her flash as she comes, and her words, once those near know her
    /// (or at the time given): `send_to_observers` reaches only those
    /// who do, and she is seen at the update.
    pub flash: Option<(Guid, Instant, Option<&'static str>)>,
}

#[derive(Debug, Clone, Copy)]
pub struct Departure {
    pub at: Instant,
    pub jaina: Guid,
    pub to: MapKey,
    pub near: Vector3d,
    pub foe: Guid,
}

fn line(lines: &[&'static str]) -> &'static str {
    use rand::RngExt;
    lines[rand::rng().random_range(0..lines.len())]
}

fn say(map: &mut Map, g: Guid, text: &str) {
    crate::game::map::scripts::say(
        map,
        g,
        text,
        crate::game::map::scripts::CHAT_SAY,
        0,
        Guid::zero(),
    );
}

/// The copy of her in her city, if she is there now.
pub fn copy(map: &Map) -> Option<Guid> {
    match map.realm.social.jaina.lock().place {
        Place::Away { map: m, copy } if m == map.key => Some(copy),
        _ => None,
    }
}

/// Her city's alarm called her, near `near`, against `foe`: if she is at
/// home, Theramore sees her off.
pub fn call(map: &mut Map, city: u32, near: Vector3d, foe: Guid) {
    let realm = map.realm.clone();
    {
        let mut s = realm.social.jaina.lock();
        if s.place != Place::Home {
            return;
        }
        s.place = Place::Leaving;
        s.called_at = Some(Instant::now());
    }
    info!("{}: Jaina Proudmoore is called to area {city}", map.key);
    let to = map.key;
    realm
        .maps
        .get(&realm, MapKey::world(HOME_MAP))
        .call(move |m| depart(m, to, near, foe));
}

/// Theramore: she goes if she is alive, at home and at peace; she says so
/// and teleports out.
fn depart(map: &mut Map, to: MapKey, near: Vector3d, foe: Guid) {
    let at_peace = crate::game::map::scripts::creature_of_spawn(map, SPAWN).filter(|g| {
        map.creature(*g)
            .is_some_and(|c| c.alive() && !c.combat.attacking && !c.evading)
    });
    let Some(g) = at_peace else {
        map.realm.social.jaina.lock().place = Place::Home;
        info!(
            "{}: Jaina Proudmoore can't leave Theramore (dead, gone or fighting)",
            map.key
        );
        return;
    };
    say(map, g, line(LEAVING));
    crate::game::map::spells::cast_triggered(map, g, TELEPORT_VISUAL, None);
    map.guards.jaina.departing = Some(Departure {
        at: map.now + VANISH,
        jaina: g,
        to,
        near,
        foe,
    });
}

/// Her city: she comes near `near` with the teleport's flash, says so, and
/// fights `foe`.
fn arrive(map: &mut Map, near: Vector3d, foe: Guid) {
    let data = map.realm.data.clone();
    let spawn = data
        .creature_spawns
        .get(&HOME_MAP)
        .and_then(|v| v.iter().find(|s| s.guid == SPAWN))
        .cloned();
    let spot = crate::game::map::guards::reachable_point(map, near, ARRIVE_WITHIN).unwrap_or(near);
    // Facing the fight.
    let o = map
        .position_of(foe)
        .map_or(0.0, |p| (p.y - spot.y).atan2(p.x - spot.x));
    let made = spawn.and_then(|spawn| {
        let counter = map.next_creature_counter;
        map.next_creature_counter += 1;
        let s = crate::game::data::CreatureSpawn {
            guid: counter | 0x80_0000,
            map: map.key.map,
            x: spot.x,
            y: spot.y,
            z: spot.z,
            orientation: o,
            movement_type: 0,
            wander_distance: 0.0,
            spawn_flags: 0,
            ..spawn
        };
        let mut c = crate::game::map::creature::Creature::from_spawn(&data, &s, &mut rand::rng())?;
        c.spawn_guid = None;
        Some(map.add_creature(c))
    });
    let Some(g) = made else {
        // She can't come: home again.
        map.realm.social.jaina.lock().place = Place::Home;
        bring_home(map);
        return;
    };
    // Seen, then the flash, then her words.
    map.guards.jaina.flash = Some((g, map.now + FLASH_WAIT, Some(line(ARRIVING))));
    {
        let mut s = map.realm.social.jaina.lock();
        s.place = Place::Away {
            map: map.key,
            copy: g,
        };
        s.visits += 1;
    }
    map.guards.jaina.calm_since = None;
    map.guards.jaina.returning = None;
    info!(
        "{}: Jaina Proudmoore arrives at ({:.1}, {:.1}, {:.1})",
        map.key, spot.x, spot.y, spot.z
    );
    // At home her template takes no one on sight (`flags_extra`'s
    // no-aggro); come to defend the city, she does, and after each kill
    // takes the next. (The user saw her stand before 80 Horde: the one she
    // came for had died.)
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.reaction = Some(crate::game::map::assistance::Reaction::Aggressive);
    }
    let fights = map.unit_view(foe).is_some_and(|u| u.alive);
    if fights && crate::game::map::combat::can_attack(map, g, foe) {
        crate::game::map::ai::attacked(map, g, foe);
    }
}

/// Tell Theramore to bring her spawn back now.
fn bring_home(map: &Map) {
    let realm = map.realm.clone();
    realm
        .maps
        .get(&realm, MapKey::world(HOME_MAP))
        .call(come_home);
}

/// Theramore: she is back at her spawn, with the teleport's flash.
fn come_home(map: &mut Map) {
    if let Some(g) = crate::game::map::scripts::creature_of_spawn(map, SPAWN)
        && map.creature(g).is_some_and(|c| !c.alive())
    {
        crate::game::map::ai::respawn_now(map, g);
    }
    map.realm.social.jaina.lock().place = Place::Home;
    if let Some(g) = crate::game::map::scripts::creature_of_spawn(map, SPAWN)
        && map.creature(g).is_some_and(|c| c.alive())
    {
        map.guards.jaina.flash = Some((g, map.now + FLASH_WAIT, None));
    }
    info!("{}: Jaina Proudmoore is home in Theramore", map.key);
}

/// A creature died on `map`: if it was her in her city, she comes back to
/// Theramore after her respawn time. Her corpse lies as any's.
pub fn creature_died(map: &mut Map, g: Guid) {
    if map.creature(g).is_none_or(|c| c.entry != JAINA) {
        return;
    }
    {
        let mut s = map.realm.social.jaina.lock();
        match s.place {
            Place::Away { map: m, copy } if m == map.key && copy == g => s.place = Place::Home,
            _ => return,
        }
    }
    map.guards.jaina.returning = None;
    map.guards.jaina.calm_since = None;
    let secs = map.cfg.guards.jaina_respawn_secs;
    info!(
        "{}: Jaina Proudmoore died; back in Theramore in {secs} s",
        map.key
    );
    let realm = map.realm.clone();
    realm
        .maps
        .get(&realm, MapKey::world(HOME_MAP))
        .call(move |m| {
            let now = m.now;
            if let Some(g) = crate::game::map::scripts::creature_of_spawn(m, SPAWN)
                && let Some(c) = m.creature_mut(g)
                && let crate::game::map::creature::Life::Dead { respawn_at, .. } = &mut c.life
            {
                *respawn_at = now + Duration::from_secs(secs);
            }
        });
}

/// Where her Teleport sends its victim when she casts it in her city: the
/// Stockade's door in Stormwind, elsewhere where she came.
pub fn teleport_to(map: &Map, caster: Guid, spell: u32) -> Option<(u32, Vector3d, f32)> {
    if spell != TELEPORT || map.creature(caster).is_none_or(|c| c.entry != JAINA) {
        return None;
    }
    copy(map).filter(|c| *c == caster)?;
    Some(city_spot(map, caster))
}

/// Her city's stand-in for her tower: the Stockade's door in Stormwind,
/// elsewhere where she came.
fn city_spot(map: &Map, copy: Guid) -> (u32, Vector3d, f32) {
    if map.cfg.guards.jaina_city == STORMWIND && map.key.map == 0 {
        let (p, o) = stockade_door(map);
        return (0, p, o);
    }
    let (p, o) = map
        .creature(copy)
        .map_or((Vector3d::default(), 0.0), |c| (c.home, c.home_orientation));
    (map.key.map, p, o)
}

/// Just outside the Stockade's door, where its exit puts a player.
pub fn stockade_door(map: &Map) -> (Vector3d, f32) {
    map.realm
        .data
        .area_triggers
        .get(&STOCKADE_EXIT)
        .and_then(|t| t.teleport.as_ref())
        .filter(|t| t.map == 0)
        .map_or(STOCKADE_DOOR, |t| {
            (
                Vector3d {
                    x: t.x,
                    y: t.y,
                    z: t.z,
                },
                t.orientation,
            )
        })
}

/// A point a condition measures her from: her tower's top, in her city,
/// is the stand-in for it ([`teleport_to`]).
pub fn stand_in(map: &Map, who: Option<Guid>, point: [f32; 3]) -> [f32; 3] {
    let Some(who) = who.filter(|g| map.creature(*g).is_some_and(|c| c.entry == JAINA)) else {
        return point;
    };
    let tower = map.realm.data.spells.target_positions.get(&TELEPORT);
    let at_tower = tower.is_some_and(|&(_, x, y, z, _)| {
        (x - point[0]).abs() < 1.0 && (y - point[1]).abs() < 1.0 && (z - point[2]).abs() < 1.0
    });
    if !at_tower || copy(map) != Some(who) {
        return point;
    }
    let (_, p, _) = city_spot(map, who);
    [p.x, p.y, p.z]
}

/// Whether she is at peace: no one to fight, no one fighting her.
fn calm(map: &Map, g: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return false;
    };
    c.alive()
        && !c.evading
        && !c.combat.attacking
        && c.threat.is_empty()
        && !c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT)
        && !map
            .players
            .keys()
            .any(|p| map.combat(*p).is_some_and(|f| f.attacking && f.victim == g))
}

/// Each update, on the open world's maps: Theramore sees her off at her
/// time; her city watches over her stay, keeps her in it, and sees her
/// home.
pub fn update(map: &mut Map) {
    if map.key.instance != 0 {
        return;
    }
    let now = map.now;
    if let Some(d) = map.guards.jaina.departing
        && d.at <= now
    {
        map.guards.jaina.departing = None;
        gone_from_home(map, d);
    }
    if let Some((at, g)) = map.guards.jaina.returning
        && at <= now
    {
        map.guards.jaina.returning = None;
        crate::game::map::scripts::remove_for_good(map, g);
        bring_home(map);
    }
    if let Some((g, until, words)) = map.guards.jaina.flash
        && (until <= now || map.creature(g).is_none_or(|c| !c.observers.is_empty()))
    {
        map.guards.jaina.flash = None;
        if map.creature(g).is_some_and(|c| c.alive()) {
            crate::game::map::spells::cast_triggered(map, g, TELEPORT_VISUAL, None);
            if let Some(words) = words {
                say(map, g, words);
            }
        }
    }
    if map.guards.jaina.look_at.is_some_and(|t| t > now) {
        return;
    }
    map.guards.jaina.look_at = Some(now + LOOK_EVERY);
    let place = map.realm.social.jaina.lock().place;
    match place {
        Place::Away { map: key, copy } if key == map.key => watch(map, copy),
        // Theramore, her call lost: she stays.
        Place::Leaving
            if map.key == MapKey::world(HOME_MAP)
                && map.guards.jaina.departing.is_none()
                && map
                    .realm
                    .social
                    .jaina
                    .lock()
                    .called_at
                    .is_some_and(|t| t.elapsed() > STALE) =>
        {
            map.realm.social.jaina.lock().place = Place::Home;
        }
        // Theramore, after its actor started again while she was away: her
        // spawn back alive goes again.
        Place::Away { .. } | Place::Returning
            if map.key == MapKey::world(HOME_MAP) && map.guards.jaina.departing.is_none() =>
        {
            if let Some(g) = crate::game::map::scripts::creature_of_spawn(map, SPAWN)
                && map.creature(g).is_some_and(|c| c.alive())
            {
                crate::game::map::scripts::despawn_with(map, g, Some(NEVER));
            }
        }
        _ => {}
    }
}

/// Theramore: the flash over, she is gone, and comes to her city.
fn gone_from_home(map: &mut Map, d: Departure) {
    if map.creature(d.jaina).is_none_or(|c| !c.alive()) {
        map.realm.social.jaina.lock().place = Place::Home;
        return;
    }
    crate::game::map::scripts::despawn_with(map, d.jaina, Some(NEVER));
    info!("{}: Jaina Proudmoore leaves Theramore", map.key);
    let realm = map.realm.clone();
    let (near, foe) = (d.near, d.foe);
    realm
        .maps
        .get(&realm, d.to)
        .call(move |m| arrive(m, near, foe));
}

/// Her city: she is kept in it, and at peace long enough, she goes home.
fn watch(map: &mut Map, g: Guid) {
    let now = map.now;
    if map.creature(g).is_none() {
        // Gone some other way: home.
        map.realm.social.jaina.lock().place = Place::Returning;
        bring_home(map);
        return;
    }
    if map.creature(g).is_some_and(|c| !c.alive()) {
        return;
    }
    // Fighting beyond her city, she goes back to where she came.
    let city = map.cfg.guards.jaina_city;
    let fighting = map.creature(g).is_some_and(|c| c.combat.attacking);
    if fighting {
        let at = map.creature(g).map(|c| c.position);
        let place = at.and_then(|at| crate::game::map::world::zone_and_area(map, at));
        if place.is_some_and(|(zone, area)| zone != city && area != city) {
            crate::game::map::ai::enter_evade(map, g);
        }
    }
    if !calm(map, g) {
        map.guards.jaina.calm_since = None;
        return;
    }
    let since = *map.guards.jaina.calm_since.get_or_insert(now);
    let stays = Duration::from_secs(map.cfg.guards.jaina_stays_secs);
    if now.duration_since(since) < stays || map.guards.jaina.returning.is_some() {
        return;
    }
    say(map, g, line(GOING));
    crate::game::map::spells::cast_triggered(map, g, TELEPORT_VISUAL, None);
    map.guards.jaina.returning = Some((now + VANISH, g));
    map.realm.social.jaina.lock().place = Place::Returning;
    info!("{}: Jaina Proudmoore goes home to Theramore", map.key);
}
