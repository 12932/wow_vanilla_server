//! Alterac Valley's NPCs as Thistle Tea scripts them, after vmangos's
//! `battleground_alterac.cpp` and `alterac_valley.cpp`: the summoners who
//! march to their altar with their escorts and invoke their lord there
//! until ten of their side complete the ritual; Ivus and Lokholar, who rise
//! and march on the enemy; the wing commanders, freed and home again, who
//! mount, rise and send their war rider at the enemy's keep; the cavalry
//! and the ground assault, who assemble, rally at their commander's call
//! and march in formation; and the generals, captains and marshals, who
//! fight in their keeps, call their guards and won't be drawn out.
//!
//! The fighting (timed spells, health thresholds, aggro and evade) is
//! EventAI built here ([`install`]), in place of the world database's; the
//! rest happens here, as the routes ([`super::av_routes`]) reach their
//! points and the valley's state ([`super::av_quests`]) launches them.

use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::battleground::av_quests::{self, extras, extras_mut};
use crate::game::map::battleground::av_routes::{self, Flow};
use crate::game::map::combat::dist;
use crate::game::map::event_ai;

/// Ids for the events and scripts built here, clear of the world
/// database's and the boss scripts'.
const BASE: u32 = 91_000_000;

const INVOCATION: u32 = 11206;
/// A summoner's phases: marching, mounted, invoking, done.
const MARCHING: u32 = 1;
const MOUNTED: u32 = 2;
const INVOKING: u32 = 3;
const SUMMONED: u32 = 4;
/// Unit flags a departing NPC drops (spawning, immune to players) and the
/// PvP flag it takes.
const DEPART_CLEARS: u32 = 0x101;
const PVP: u32 = 0x1000;
const QUEST_GIVER: u32 = 0x2;
const EMOTE_SALUTE: u32 = 22;

/// Each side's summoner, as Thistle Tea's `AlteracValleyAssault` has
/// them.
struct Summoner {
    entry: u32,
    escort: u32,
    mount: u32,
    escort_mount: u32,
    /// The points where they mount, get down, and reach the altar.
    mounts_at: usize,
    walks_at: usize,
    altar_at: usize,
    altar_text: u32,
    altar: u32,
    altar_position: (f32, f32, f32, f32),
    lord: u32,
    /// How long after the ritual they stay.
    stays: Duration,
}

const SUMMONERS: [Summoner; 2] = [
    Summoner {
        entry: 13442,
        escort: 13443,
        mount: 9695,
        escort_mount: 9695,
        mounts_at: 3,
        walks_at: 48,
        altar_at: 49,
        altar_text: 8735,
        altar: 178670,
        altar_position: (-199.993, -343.217, 6.77662, 3.68265),
        lord: 13419,
        stays: Duration::from_secs(6),
    },
    Summoner {
        entry: 13236,
        escort: 13284,
        mount: 12242,
        escort_mount: 1166,
        mounts_at: 6,
        walks_at: 42,
        altar_at: 43,
        altar_text: 8632,
        altar: 178465,
        altar_position: (-360.139, -133.403, 26.4856, 4.41568),
        lord: 13256,
        stays: Duration::from_secs(600),
    },
];

const IVUS: u32 = 13419;
const LOKHOLAR: u32 = 13256;
/// Where the lords make their home on the field.
const LORD_HOME: (f32, f32, f32) = (-260.0, -290.0, 6.7);

/// The wing commanders: entry, the point of their route that's home, their
/// mount, and their war rider.
const WINGS: [(u32, usize, u32, u32); 6] = [
    (13179, 74, 11012, 14943),
    (13180, 84, 11012, 14944),
    (13181, 97, 11012, 14945),
    (13438, 66, 1148, 14946),
    (13439, 76, 1148, 14948),
    (13437, 92, 1148, 14947),
];
/// The war riders: the Horde's, sent at Dun Baldar, and the Alliance's,
/// at Frostwolf Keep.
const HORDE_RIDERS: [u32; 3] = [14943, 14944, 14945];
const ALLIANCE_RIDERS: [u32; 3] = [14946, 14947, 14948];
const RIDER_TARGETS: [(f32, f32, f32); 2] = [(-1311.53, -355.28, 130.93), (618.4, -87.97, 85.77)];

/// The cavalry: commander, rider, mount, rally point, last point,
/// speech, war cry, and how far the call carries.
struct Cavalry {
    commander: u32,
    rider: u32,
    mount: u32,
    rally: usize,
    last: usize,
    speech: u32,
    warcry: u32,
    radius: f32,
}

/// By side: the Stormpike rams, the Frostwolf wolves.
const CAVALRY: [Cavalry; 2] = [
    Cavalry {
        commander: 13577,
        rider: 13576,
        mount: 2786,
        rally: 5,
        last: 92,
        speech: 8906,
        warcry: 8908,
        radius: 20.0,
    },
    Cavalry {
        commander: 13441,
        rider: 13440,
        mount: 1166,
        rally: 2,
        last: 81,
        speech: 8890,
        warcry: 8891,
        radius: 50.0,
    },
];

/// The ground assault: commander, its first troop entry (then a tier
/// each), its last point, speech, war cry; where it assembles, and what
/// the quartermaster says.
struct Ground {
    commander: u32,
    troops: u32,
    last: usize,
    troop_last: usize,
    speech: u32,
    warcry: u32,
    at: (f32, f32, f32, f32),
    announcement: u32,
}

const GROUND: [Ground; 2] = [
    Ground {
        commander: 13446,
        troops: 13524,
        last: 48,
        troop_last: 49,
        speech: 8906,
        warcry: 8908,
        at: (-243.75, -431.32, 20.0, 2.59),
        announcement: 8907,
    },
    Ground {
        commander: 13449,
        troops: 13528,
        last: 53,
        troop_last: 54,
        speech: 8890,
        warcry: 8891,
        at: (-470.38, -51.84, 41.0, 5.93),
        announcement: 8913,
    },
];

/// The leaders who hold their keeps: entry, home, how far they'll be
/// drawn, and who comes when they call.
struct Leader {
    entry: u32,
    home: (f32, f32, f32),
    leash: f32,
    linked: &'static [u32],
    general: bool,
}

const VANNDAR: u32 = 11948;
const DREK_THAR: u32 = 11946;
const BALINDA: u32 = 11949;
const GALVANGAR: u32 = 11947;
const WOLVES: [u32; 2] = [12121, 12122];

const LEADERS: [Leader; 4] = [
    Leader {
        entry: VANNDAR,
        home: (722.4, -11.0, 50.7),
        leash: 35.0,
        linked: &[
            13333, 14762, 14763, 14764, 14765, 14766, 14767, 14768, 14769,
        ],
        general: true,
    },
    Leader {
        entry: DREK_THAR,
        home: (-1370.9, -219.8, 98.5),
        leash: 33.0,
        linked: &[
            12121, 12122, 14770, 14771, 14772, 14773, 14774, 14775, 14776, 14777,
        ],
        general: true,
    },
    Leader {
        entry: BALINDA,
        home: (-57.7, -286.6, 15.6),
        leash: 45.0,
        linked: &[],
        general: false,
    },
    Leader {
        entry: GALVANGAR,
        home: (-545.2, -165.3, 57.8),
        leash: 45.0,
        linked: &[],
        general: false,
    },
];
/// A general's phases: fighting, and drawn out.
const FIGHTING: u32 = 0;
const LEASHED: u32 = 1;
const LINKED_RADIUS: f32 = 100.0;

/// A summoning ritual at an altar: who of the side channel into it.
#[derive(Debug, Clone)]
pub struct Ritual {
    pub object: Guid,
    pub side: usize,
    /// Each with where they stood to join.
    pub users: Vec<(Guid, Vector3d)>,
    pub needed: usize,
}

/// Something a script does later.
#[derive(Debug, Clone, Copy)]
enum Later {
    /// A wing commander mounts and rises.
    Rise(u32),
    /// And sends out their rider.
    SendRider(u32),
    /// A lord sets out.
    LordMarches(u32),
    Talk(u32),
    /// The summoner and their escorts leave.
    Leave(u32),
}

/// A war rider on its way, then over the enemy's keep.
#[derive(Debug, Clone, Copy)]
struct Rider {
    arrives: Instant,
    arrived: bool,
    next: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct State {
    later: Vec<(Instant, Guid, Later)>,
    pub rituals: Vec<Ritual>,
    riders: AHashMap<Guid, Rider>,
    /// The cavalry riders and troops, by the commander they wait on.
    followers: AHashMap<Guid, Guid>,
    /// The leaders fighting at the last look.
    fighting: AHashSet<Guid>,
    next_sweep: Option<Instant>,
}

fn state_mut(map: &mut Map) -> Option<&mut State> {
    extras_mut(map).map(|x| &mut x.scripts)
}

// What they do.

fn set_phase(map: &mut Map, g: Guid, phase: u32) {
    event_ai::set_phase(map, g, phase);
}

fn mount(map: &mut Map, g: Guid, display: u32) {
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_MOUNTDISPLAYID, display);
    }
}

fn set_run(map: &mut Map, g: Guid, on: bool) {
    if let Some(s) = event_ai::scripted_mut(map, g) {
        s.run = on;
    }
}

fn unit_flags(map: &mut Map, g: Guid, field: u16, add: u32, clear: u32) {
    if let Some(f) = map.unit_fields_mut(g) {
        let v = f.get_u32(field);
        f.set_u32(field, (v & !clear) | add);
    }
}

fn talk(map: &mut Map, g: Guid, text: u32) {
    crate::game::map::scripts::talk(map, g, text, Guid::zero(), None);
}

fn emote(map: &mut Map, g: Guid, e: u32) {
    if let Ok(emote) = Emote::try_from(e)
        && let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE { emote, guid: g })
    {
        map.send_to_observers(g, &frame, true);
    }
}

fn home_here(map: &mut Map, g: Guid) {
    crate::game::map::ai::advance_motion(map, g);
    if let Some(c) = map.creature_mut(g) {
        c.home = c.position;
        c.home_orientation = c.orientation;
    }
}

fn cast_self(map: &mut Map, g: Guid, spell: u32) {
    crate::game::map::spells::cast_triggered(map, g, spell, Some(g));
}

fn invoke(map: &mut Map, g: Guid) {
    cast_self(map, g, INVOCATION);
}

fn stop_invoking(map: &mut Map, g: Guid) {
    crate::game::map::spells::interrupt(map, g);
    crate::game::map::auras::remove_spell(map, g, INVOCATION, None);
}

/// The living creatures of `entry` within `radius` of `g`.
fn near(map: &Map, g: Guid, entry: u32, radius: f32) -> Vec<Guid> {
    let Some(at) = map.position_of(g) else {
        return Vec::new();
    };
    crate::game::map::scripts::creatures_near(map, at, entry, radius, true)
        .into_iter()
        .filter(|c| *c != g)
        .collect()
}

fn later(map: &mut Map, after: Duration, g: Guid, what: Later) {
    let at = map.now + after;
    if let Some(s) = state_mut(map) {
        s.later.push((at, g, what));
    }
}

fn despawn_in(map: &mut Map, g: Guid, after: Duration) {
    let at = map.now + after;
    if let Some(s) = event_ai::scripted_mut(map, g) {
        s.despawn_at = Some(at);
        s.despawn_type = 1;
    }
}

fn summon(map: &mut Map, entry: u32, at: (f32, f32, f32, f32)) -> Option<Guid> {
    crate::game::map::outdoor_pvp::summon(map, entry, at, 0)
}

/// An object put down by a script, for no one.
fn place(map: &mut Map, entry: u32, at: (f32, f32, f32, f32), faction: u32) -> Option<Guid> {
    let data = map.realm.data.clone();
    let t = data.gameobject_templates.get(&entry)?;
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let mut go = crate::game::map::gameobject::GameObject::from_spawn(
        t,
        &crate::game::data::GameObjectSpawn {
            guid: counter | 0x80_0000,
            entry,
            map: map.key.map,
            x: at.0,
            y: at.1,
            z: at.2,
            orientation: at.3,
            rotation: [0.0, 0.0, (at.3 / 2.0).sin(), (at.3 / 2.0).cos()],
            state: 1,
            anim_progress: 100,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    if faction != 0 {
        go.fields.set_u32(GAMEOBJECT_FACTION, faction);
    }
    go.fields.set_u32(GAMEOBJECT_LEVEL, 0);
    Some(map.add_gameobject(go))
}

// The summoners.

fn summoner_side(entry: u32) -> Option<usize> {
    SUMMONERS.iter().position(|s| s.entry == entry)
}

/// The offerings are made: the summoner sets out for the altar.
pub fn summoner_departs(map: &mut Map, s: usize) {
    let Some(g) = creature_of(map, SUMMONERS[s].entry) else {
        return;
    };
    if !av_routes::start(map, g, SUMMONERS[s].entry) {
        // Nowhere to walk: the altar is here.
        at_altar(map, g, s);
    }
}

fn creature_of(map: &Map, entry: u32) -> Option<Guid> {
    map.creatures
        .iter()
        .find(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
}

/// The escorts of `g` within `radius`: what's done to them.
fn escorts(map: &Map, g: Guid, s: usize, radius: f32) -> Vec<Guid> {
    near(map, g, SUMMONERS[s].escort, radius)
}

fn depart(map: &mut Map, g: Guid) {
    set_phase(map, g, MARCHING);
    unit_flags(map, g, UNIT_FIELD_FLAGS, PVP, DEPART_CLEARS);
    set_run(map, g, false);
}

fn mounted(map: &mut Map, g: Guid, s: usize) {
    set_phase(map, g, MOUNTED);
    mount(map, g, SUMMONERS[s].mount);
    set_run(map, g, true);
    for e in escorts(map, g, s, 40.0) {
        set_phase(map, e, MOUNTED);
        mount(map, e, SUMMONERS[s].escort_mount);
        set_run(map, e, true);
    }
}

fn on_foot(map: &mut Map, g: Guid, s: usize) {
    set_phase(map, g, MARCHING);
    mount(map, g, 0);
    set_run(map, g, false);
    for e in escorts(map, g, s, 40.0) {
        set_phase(map, e, MARCHING);
        mount(map, e, 0);
        set_run(map, e, false);
    }
}

/// At the altar: the summoner puts it down and invokes; the escorts join
/// in. The ritual waits on the side.
fn at_altar(map: &mut Map, g: Guid, s: usize) {
    set_phase(map, g, INVOKING);
    home_here(map, g);
    talk(map, g, SUMMONERS[s].altar_text);
    let altar = place(map, SUMMONERS[s].altar, SUMMONERS[s].altar_position, 0);
    invoke(map, g);
    for e in escorts(map, g, s, 30.0) {
        set_phase(map, e, INVOKING);
        invoke(map, e);
    }
    let Some(object) = altar else {
        return;
    };
    let needed = map
        .realm
        .data
        .gameobject_templates
        .get(&SUMMONERS[s].altar)
        .map_or(1, |t| t.data[0].max(1) as usize);
    if let Some(st) = state_mut(map) {
        st.rituals.push(Ritual {
            object,
            side: s,
            users: Vec::new(),
            needed,
        });
    }
}

/// `CMSG_GAMEOBJ_USE` on an altar: one more of its side channels into the
/// ritual. Returns whether it was one.
pub fn use_ritual(map: &mut Map, g: Guid, object: Guid) -> bool {
    let Some(i) =
        extras(map).and_then(|x| x.scripts.rituals.iter().position(|r| r.object == object))
    else {
        return false;
    };
    let side = map.battleground.as_ref().and_then(|m| m.team_of(g));
    let Some(at) = map.players.get(&g).map(|p| p.position()) else {
        return true;
    };
    let joined = {
        let Some(r) = state_mut(map).map(|st| &mut st.rituals[i]) else {
            return true;
        };
        if side != Some(r.side) || r.users.iter().any(|(u, _)| *u == g) {
            false
        } else {
            r.users.push((g, at));
            true
        }
    };
    if !joined {
        return true;
    }
    let duration = {
        let data = &map.realm.data;
        data.spells
            .get(INVOCATION)
            .and_then(|s| data.spells.duration(s, 60))
            .unwrap_or(0)
    };
    if let Some(p) = map.players.get_mut(&g) {
        p.fields.set_guid(UNIT_FIELD_CHANNEL_OBJECT, object);
        p.fields.set_u32(UNIT_CHANNEL_SPELL, INVOCATION);
        p.outbound.send(&MSG_CHANNEL_START_Server {
            spell: INVOCATION,
            duration,
        });
    }
    let done = extras(map).is_some_and(|x| {
        let r = &x.scripts.rituals[i];
        r.users.len() >= r.needed
    });
    if done {
        complete(map, i);
    }
    true
}

fn end_channel(map: &mut Map, g: Guid) {
    if let Some(p) = map.players.get_mut(&g) {
        if p.fields.get_u32(UNIT_CHANNEL_SPELL) == INVOCATION {
            p.fields.set_guid(UNIT_FIELD_CHANNEL_OBJECT, Guid::zero());
            p.fields.set_u32(UNIT_CHANNEL_SPELL, 0);
        }
        p.outbound.send(&MSG_CHANNEL_UPDATE_Server { time: 0 });
    }
}

/// Those who left the ritual (moved, died, went) no longer count.
fn rituals(map: &mut Map) {
    let Some(rituals) = extras(map).map(|x| x.scripts.rituals.clone()) else {
        return;
    };
    for (i, r) in rituals.iter().enumerate() {
        let left: Vec<Guid> = r
            .users
            .iter()
            .filter(|(u, at)| {
                map.players.get(u).is_none_or(|p| {
                    p.death != crate::game::map::death::Death::Alive
                        || dist(p.position(), *at) > 1.5
                })
            })
            .map(|(u, _)| *u)
            .collect();
        for u in &left {
            end_channel(map, *u);
        }
        if let Some(st) = state_mut(map)
            && let Some(r) = st.rituals.get_mut(i)
        {
            r.users.retain(|(u, _)| !left.contains(u));
        }
    }
    // An altar gone takes its ritual with it.
    let gone: Vec<Guid> = rituals
        .iter()
        .filter(|r| map.gameobject(r.object).is_none())
        .map(|r| r.object)
        .collect();
    if let Some(st) = state_mut(map) {
        st.rituals.retain(|r| !gone.contains(&r.object));
    }
}

/// The ritual is complete: the lord rises; the summoner and escorts stop,
/// take up the altar, and leave a while later.
fn complete(map: &mut Map, i: usize) {
    let Some(r) = state_mut(map).map(|st| st.rituals.remove(i)) else {
        return;
    };
    for (u, _) in &r.users {
        end_channel(map, *u);
    }
    let s = r.side;
    let altar = map
        .gameobject(r.object)
        .map(|o| (o.position, o.orientation));
    map.remove_gameobject(r.object);
    if let Some((at, o)) = altar {
        raise_lord(map, s, (at.x, at.y, at.z, o));
    }
    let Some(g) = creature_of(map, SUMMONERS[s].entry) else {
        return;
    };
    set_phase(map, g, SUMMONED);
    stop_invoking(map, g);
    for e in escorts(map, g, s, 200.0) {
        set_phase(map, e, SUMMONED);
        stop_invoking(map, e);
    }
    later(map, SUMMONERS[s].stays, g, Later::Leave(SUMMONERS[s].entry));
    if SUMMONERS[s].lord == LOKHOLAR {
        talk(map, g, 8626);
    }
}

/// The lord rises at the altar: it speaks, makes the field its home, and
/// sets out on its route.
fn raise_lord(map: &mut Map, s: usize, at: (f32, f32, f32, f32)) {
    let lord = SUMMONERS[s].lord;
    let Some(g) = summon(map, lord, at) else {
        return;
    };
    let (text, second) = if lord == LOKHOLAR {
        (8616, Some(8617))
    } else {
        (8736, None)
    };
    talk(map, g, text);
    if let Some(c) = map.creature_mut(g) {
        c.home = Vector3d {
            x: LORD_HOME.0,
            y: LORD_HOME.1,
            z: LORD_HOME.2,
        };
    }
    set_run(map, g, false);
    later(map, Duration::from_secs(1), g, Later::LordMarches(lord));
    if let Some(t) = second {
        later(map, Duration::from_secs(3), g, Later::Talk(t));
    }
}

// The wing commanders.

fn wing(entry: u32) -> Option<(u32, usize, u32, u32)> {
    WINGS.iter().copied().find(|w| w.0 == entry)
}

/// A freed wing commander sets out for home. False if they have no road.
pub fn commander_departs(map: &mut Map, g: Guid) -> bool {
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
        return false;
    };
    set_phase(map, g, 1);
    unit_flags(map, g, UNIT_FIELD_FLAGS, PVP, DEPART_CLEARS);
    unit_flags(map, g, UNIT_NPC_FLAGS, 0, QUEST_GIVER);
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_u32(UNIT_DYNAMIC_FLAGS, 0);
        f.set_byte(UNIT_FIELD_BYTES_1, 0, 0);
    }
    set_run(map, g, true);
    if entry == 13439 {
        cast_self(map, g, 5759);
    }
    av_routes::start(map, g, entry)
}

/// A wing commander is home: they give their quests again.
fn commander_home(map: &mut Map, g: Guid, entry: u32) {
    set_phase(map, g, 2);
    unit_flags(map, g, UNIT_NPC_FLAGS, QUEST_GIVER, 0);
    home_here(map, g);
    av_quests::fleet_home(map, entry);
}

/// The air attack is ordered: the commander mounts and rises, then sends
/// out their rider.
pub fn commander_launches(map: &mut Map, g: Guid) {
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
        return;
    };
    set_phase(map, g, 3);
    later(map, Duration::from_secs(5), g, Later::Rise(entry));
    later(map, Duration::from_secs(10), g, Later::SendRider(entry));
}

fn rise(map: &mut Map, g: Guid, entry: u32) {
    let Some((_, _, display, _)) = wing(entry) else {
        return;
    };
    unit_flags(map, g, UNIT_NPC_FLAGS, 0, QUEST_GIVER);
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_DISPLAYID, display);
    }
    av_routes::set_fly(map, g, true);
    set_run(map, g, true);
    if let Some(s) = event_ai::scripted_mut(map, g) {
        s.no_combat_movement = true;
    }
    cast_self(map, g, 24085);
    if let Some(at) = map.position_of(g) {
        av_routes::go(
            map,
            g,
            Vector3d {
                z: at.z + 30.0,
                ..at
            },
        );
    }
}

fn send_rider(map: &mut Map, g: Guid, entry: u32) {
    let Some((_, _, _, rider)) = wing(entry) else {
        return;
    };
    cast_self(map, g, 24699);
    crate::game::map::ai::advance_motion(map, g);
    let Some((at, o)) = map.creature(g).map(|c| (c.position, c.orientation)) else {
        return;
    };
    if let Some(r) = summon(map, rider, (at.x, at.y, at.z, o)) {
        rider_departs(map, r);
    }
}

// The war riders.

/// A war rider flies at the enemy's keep, and won't fight until it's
/// there.
fn rider_departs(map: &mut Map, g: Guid) {
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
        return;
    };
    let side = if HORDE_RIDERS.contains(&entry) { 1 } else { 0 };
    let (x, y, z) = RIDER_TARGETS[side];
    let target = Vector3d { x, y, z };
    av_routes::set_fly(map, g, true);
    set_run(map, g, true);
    if let Some(c) = map.creature_mut(g) {
        c.home = target;
        c.movement_type = 0;
    }
    if let Some(s) = event_ai::scripted_mut(map, g) {
        s.passive = true;
    }
    let arrives = av_routes::go(map, g, target).unwrap_or(map.now);
    if let Some(st) = state_mut(map) {
        st.riders.insert(
            g,
            Rider {
                arrives,
                arrived: false,
                next: arrives,
            },
        );
    }
}

/// The war riders: there, they fight; out of a fight, they circle their
/// target and go for the nearest enemy within fifty yards.
fn riders(map: &mut Map) {
    let now = map.now;
    let Some(all) = extras(map).map(|x| x.scripts.riders.clone()) else {
        return;
    };
    for (g, r) in all {
        let Some(c) = map.creature(g).filter(|c| c.alive()) else {
            if let Some(st) = state_mut(map) {
                st.riders.remove(&g);
            }
            continue;
        };
        if c.combat.attacking || c.evading {
            continue;
        }
        let home = c.home;
        if !r.arrived {
            if r.arrives > now {
                continue;
            }
            crate::game::map::ai::advance_motion(map, g);
            set_phase(map, g, 1);
            if let Some(s) = event_ai::scripted_mut(map, g) {
                s.passive = false;
            }
            if let Some(st) = state_mut(map)
                && let Some(r) = st.riders.get_mut(&g)
            {
                r.arrived = true;
                r.next = now;
            }
            continue;
        }
        if r.next > now {
            continue;
        }
        let enemy = map.position_of(g).and_then(|at| {
            map.players_near(at, 50.0)
                .into_iter()
                .filter(|p| {
                    map.players.get(p).is_some_and(|pl| {
                        pl.death == crate::game::map::death::Death::Alive
                            && dist(pl.position(), at) <= 50.0
                    }) && crate::game::map::combat::hostile(map, g, *p)
                })
                .min_by(|a, b| {
                    let da = map.position_of(*a).map_or(f32::MAX, |p| dist(p, at));
                    let db = map.position_of(*b).map_or(f32::MAX, |p| dist(p, at));
                    da.total_cmp(&db)
                })
        });
        if let Some(e) = enemy {
            crate::game::map::ai::attacked(map, g, e);
        } else if map
            .creature(g)
            .is_some_and(|c| !matches!(c.motion, crate::game::map::creature::Motion::Moving(_)))
        {
            // Around its target, at its height.
            use rand::RngExt;
            let mut rng = rand::rng();
            let angle = rng.random_range(0.0..std::f32::consts::TAU);
            let reach = rng.random_range(0.0f32..55.0);
            let to = Vector3d {
                x: home.x + reach * angle.cos(),
                y: home.y + reach * angle.sin(),
                z: home.z,
            };
            av_routes::go(map, g, to);
        }
        if let Some(st) = state_mut(map)
            && let Some(r) = st.riders.get_mut(&g)
        {
            r.next = now + Duration::from_secs(1);
        }
    }
}

// The cavalry.

/// The charge is ordered: the commander rides out with eight riders.
pub fn cavalry_charges(map: &mut Map, g: Guid, s: usize) {
    set_phase(map, g, 1);
    unit_flags(map, g, UNIT_FIELD_FLAGS, PVP, DEPART_CLEARS);
    set_run(map, g, true);
    for at in cavalry_positions(s) {
        if let Some(r) = summon(map, CAVALRY[s].rider, at) {
            rider_ready(map, r, s);
        }
    }
    av_routes::start(map, g, CAVALRY[s].commander);
}

fn cavalry_positions(s: usize) -> Vec<(f32, f32, f32, f32)> {
    (0..8)
        .map(|i| {
            let k = (i % 5) as f32;
            if s == 1 {
                let (x, y, o) = if i < 4 {
                    (-1230.0, -611.0, 5.4)
                } else {
                    (-1223.0, -619.0, 2.22)
                };
                (x - 4.0 * k, y - 3.0 * k, 54.0, o)
            } else {
                let (x, y) = if i < 4 {
                    (610.0, -35.0)
                } else {
                    (607.0, -37.0)
                };
                (x + 2.0 * k, y - 3.0 * k, 45.0, 0.6)
            }
        })
        .collect()
}

/// A rider mounts and waits by its commander; with none near, it rides
/// out alone.
fn rider_ready(map: &mut Map, g: Guid, s: usize) {
    mount(map, g, CAVALRY[s].mount);
    set_run(map, g, true);
    let commander = near(map, g, CAVALRY[s].commander, 100.0).into_iter().next();
    match commander {
        Some(c) => follow(map, g, c),
        None => orphaned(map, g, CAVALRY[s].rider, true),
    }
}

/// `g` waits on its commander `c`.
fn follow(map: &mut Map, g: Guid, c: Guid) {
    if let Some(st) = state_mut(map) {
        st.followers.insert(g, c);
    }
}

/// The commander's call at the rally point: its men take their places
/// behind it.
fn rally(map: &mut Map, g: Guid, entries: &[u32], radius: f32, warcry: u32) {
    for &entry in entries {
        for m in near(map, g, entry, radius) {
            talk(map, m, warcry);
            set_run(map, m, true);
            set_phase(map, m, 2);
            crate::game::map::creature_groups::leave(map, m);
            // In formation, fighting and evading together.
            crate::game::map::creature_groups::join_with(
                map,
                m,
                g,
                0.0,
                0.0,
                crate::game::data::scripts::GROUP_FORMATION
                    | crate::game::data::scripts::GROUP_AGGRO_TOGETHER
                    | crate::game::data::scripts::GROUP_EVADE_TOGETHER,
            );
            av_routes::join(map, m, g);
            follow(map, m, g);
        }
    }
}

/// A rider or troop whose commander fell before the rally goes on alone
/// along its own route; the cavalry don't stay long.
fn orphaned(map: &mut Map, g: Guid, entry: u32, cavalry: bool) {
    set_phase(map, g, 1);
    set_run(map, g, true);
    av_routes::start(map, g, entry);
    if cavalry {
        despawn_in(map, g, Duration::from_secs(600));
    }
}

// The ground assault.

/// The orders are taken: the commander and ten troops of the side's tier
/// assemble by the quartermaster.
pub fn ground_assembles(map: &mut Map, quartermaster: Guid, s: usize, tier: u8) {
    let gr = &GROUND[s];
    talk(map, quartermaster, gr.announcement);
    emote(map, quartermaster, EMOTE_SALUTE);
    let Some(commander) = summon(map, gr.commander, gr.at) else {
        return;
    };
    home_here(map, commander);
    av_quests::ground_commander(map, s, commander);
    for at in ground_positions(s) {
        if let Some(t) = summon(map, gr.troops + u32::from(tier.min(3)), at) {
            set_run(map, t, true);
            follow(map, t, commander);
        }
    }
}

fn ground_positions(s: usize) -> Vec<(f32, f32, f32, f32)> {
    (0..10)
        .map(|i| {
            let k = (i % 5) as f32;
            if s == 0 {
                let (x, y) = if i < 5 {
                    (-240.9, -431.11)
                } else {
                    (-238.35, -432.42)
                };
                (x - k, y - k, 20.2, 2.59)
            } else {
                let (x, y) = if i < 5 {
                    (-472.2, -48.4)
                } else {
                    (-474.8, -48.7)
                };
                (x - 0.4 * k, y - 1.6 * k, 41.3, 5.93)
            }
        })
        .collect()
}

/// The orders are turned in: the commander salutes and marches.
pub fn ground_marches(map: &mut Map, g: Guid, s: usize) {
    set_phase(map, g, 1);
    emote(map, g, EMOTE_SALUTE);
    unit_flags(map, g, UNIT_FIELD_FLAGS, PVP, DEPART_CLEARS);
    set_run(map, g, false);
    av_routes::start(map, g, GROUND[s].commander);
}

fn is_troop(entry: u32) -> Option<usize> {
    GROUND
        .iter()
        .position(|gr| (gr.troops..gr.troops + 4).contains(&entry))
}

// The routes' points.

/// `g`, walking `entry`'s route, reached point `point`.
pub fn arrived(map: &mut Map, g: Guid, entry: u32, point: usize) -> Flow {
    if let Some(s) = summoner_side(entry) {
        let sm = &SUMMONERS[s];
        return match point {
            0 => {
                depart(map, g);
                Flow::Next
            }
            p if p == sm.mounts_at => {
                mounted(map, g, s);
                Flow::Next
            }
            p if p == sm.walks_at => {
                on_foot(map, g, s);
                Flow::Next
            }
            p if p == sm.altar_at => {
                at_altar(map, g, s);
                Flow::Stop
            }
            _ => Flow::Next,
        };
    }
    match (entry, point) {
        (LOKHOLAR, 30) => {
            talk(map, g, 8740);
            home_here(map, g);
            Flow::Next
        }
        (LOKHOLAR, 31) | (IVUS, 21) => {
            if entry == IVUS {
                talk(map, g, 8739);
            }
            home_here(map, g);
            Flow::Stop
        }
        (IVUS, 1) => {
            talk(map, g, 8737);
            Flow::Next
        }
        _ => {
            if let Some((_, home, _, _)) = wing(entry)
                && home == point
            {
                commander_home(map, g, entry);
                return Flow::Stop;
            }
            if let Some(s) = CAVALRY.iter().position(|c| c.commander == entry) {
                if point == CAVALRY[s].rally {
                    talk(map, g, CAVALRY[s].speech);
                    return Flow::Hold(Duration::from_secs(6));
                }
                if point == CAVALRY[s].last {
                    home_here(map, g);
                    return Flow::Stop;
                }
            }
            if let Some(s) = GROUND.iter().position(|gr| gr.commander == entry) {
                if point == 2 {
                    talk(map, g, GROUND[s].speech);
                    return Flow::Hold(Duration::from_secs(6));
                }
                if point == GROUND[s].last {
                    home_here(map, g);
                    return Flow::Stop;
                }
            }
            if let Some(s) = is_troop(entry)
                && point == GROUND[s].troop_last
            {
                home_here(map, g);
                return Flow::Stop;
            }
            Flow::Next
        }
    }
}

/// A hold at `point` is over.
pub fn released(map: &mut Map, g: Guid, entry: u32, point: usize) -> Flow {
    if let Some(s) = CAVALRY.iter().position(|c| c.commander == entry)
        && point == CAVALRY[s].rally
    {
        rally(
            map,
            g,
            &[CAVALRY[s].rider],
            CAVALRY[s].radius,
            CAVALRY[s].warcry,
        );
    }
    if let Some(s) = GROUND.iter().position(|gr| gr.commander == entry)
        && point == 2
    {
        set_run(map, g, true);
        let troops: Vec<u32> = (GROUND[s].troops..GROUND[s].troops + 4).collect();
        rally(map, g, &troops, 40.0, GROUND[s].warcry);
    }
    Flow::Next
}

/// One of the valley's creatures died: those who follow it go on alone.
pub fn died(map: &mut Map, g: Guid, entry: u32) {
    let commander_of = CAVALRY
        .iter()
        .position(|c| c.commander == entry)
        .map(|s| (CAVALRY[s].rider, true))
        .or_else(|| {
            GROUND
                .iter()
                .position(|gr| gr.commander == entry)
                .map(|s| (GROUND[s].troops, false))
        });
    if let Some((first, cavalry)) = commander_of {
        let entries: Vec<u32> = if cavalry {
            vec![first]
        } else {
            (first..first + 4).collect()
        };
        let followers: Vec<Guid> = extras(map)
            .map(|x| {
                x.scripts
                    .followers
                    .iter()
                    .filter(|(_, c)| **c == g)
                    .map(|(f, _)| *f)
                    .collect()
            })
            .unwrap_or_default();
        let followers: Vec<Guid> = followers
            .into_iter()
            .filter(|f| {
                map.creature(*f)
                    .is_some_and(|c| c.alive() && entries.contains(&c.entry))
            })
            .collect();
        for f in followers {
            let rallied = event_ai::phase(map, f) == 2;
            let own = map.creature(f).map_or(0, |c| c.entry);
            crate::game::map::creature_groups::leave(map, f);
            av_routes::leave(map, f);
            if rallied {
                set_phase(map, f, 1);
                set_run(map, f, true);
                if cavalry {
                    despawn_in(map, f, Duration::from_secs(600));
                }
            } else {
                orphaned(map, f, own, cavalry);
            }
        }
    }
    if let Some(st) = state_mut(map) {
        st.riders.remove(&g);
        st.later.retain(|(_, w, _)| *w != g);
        st.fighting.remove(&g);
        st.followers.retain(|f, c| *f != g && *c != g);
    }
}

// Each update.

pub fn update(map: &mut Map) {
    av_routes::update(map);
    run_later(map);
    rituals(map);
    riders(map);
    sweep(map);
}

fn run_later(map: &mut Map) {
    let now = map.now;
    let due: Vec<(Guid, Later)> = {
        let Some(st) = state_mut(map) else {
            return;
        };
        if st.later.iter().all(|(t, _, _)| *t > now) {
            return;
        }
        let (due, rest): (Vec<_>, Vec<_>) = st.later.drain(..).partition(|(t, _, _)| *t <= now);
        st.later = rest;
        due.into_iter().map(|(_, g, w)| (g, w)).collect()
    };
    for (g, what) in due {
        if map.creature(g).is_none_or(|c| !c.alive()) {
            continue;
        }
        match what {
            Later::Rise(entry) => rise(map, g, entry),
            Later::SendRider(entry) => send_rider(map, g, entry),
            Later::LordMarches(entry) => {
                av_routes::start_at(map, g, entry, 0, true);
            }
            Later::Talk(text) => talk(map, g, text),
            Later::Leave(entry) => {
                if let Some(s) = summoner_side(entry) {
                    for e in escorts(map, g, s, 100.0) {
                        mount(map, e, 0);
                        crate::game::map::scripts::despawn(map, e);
                    }
                }
                crate::game::map::scripts::despawn(map, g);
            }
        }
    }
}

/// Once a second: the leaders in their keeps (drawn too far, they go
/// home; fighting, they call their guards; Balinda holds her ground in
/// range), and the cavalry and quartermasters back from the dead.
fn sweep(map: &mut Map) {
    let now = map.now;
    if extras(map).is_some_and(|x| x.scripts.next_sweep.is_some_and(|t| t > now)) {
        return;
    }
    if let Some(st) = state_mut(map) {
        st.next_sweep = Some(now + Duration::from_secs(1));
    }
    for l in &LEADERS {
        let Some(g) = creature_of(map, l.entry) else {
            continue;
        };
        let Some(c) = map.creature(g) else {
            continue;
        };
        let (fighting, at, victim) = (c.combat.attacking, c.position, c.combat.victim);
        let was = extras(map).is_some_and(|x| x.scripts.fighting.contains(&g));
        if let Some(st) = state_mut(map) {
            if fighting {
                st.fighting.insert(g);
            } else {
                st.fighting.remove(&g);
            }
        }
        if was && !fighting && l.entry == DREK_THAR {
            // He raises his fallen wolves.
            for w in WOLVES {
                let fallen: Vec<Guid> = map
                    .creatures
                    .iter()
                    .filter(|(_, c)| {
                        c.entry == w && !c.alive() && dist(c.position, at) <= LINKED_RADIUS
                    })
                    .map(|(_, c)| c.guid)
                    .collect();
                for f in fallen {
                    crate::game::map::ai::respawn_now(map, f);
                }
            }
        }
        if !fighting {
            continue;
        }
        let home = Vector3d {
            x: l.home.0,
            y: l.home.1,
            z: l.home.2,
        };
        if dist(at, home) > l.leash {
            if l.general {
                set_phase(map, g, LEASHED);
            }
            crate::game::map::ai::enter_evade(map, g);
            continue;
        }
        if !victim.is_zero() {
            for &entry in l.linked {
                for m in near(map, g, entry, LINKED_RADIUS) {
                    let idle = map
                        .creature(m)
                        .is_some_and(|c| c.alive() && !c.combat.attacking && !c.evading);
                    if idle && crate::game::map::combat::can_attack(map, m, victim) {
                        crate::game::map::ai::attacked(map, m, victim);
                    }
                }
            }
        }
        if l.entry == BALINDA {
            let spot = map
                .position_of(victim)
                .is_some_and(|v| (5.0..=25.0).contains(&dist(v, at)));
            if let Some(s) = event_ai::scripted_mut(map, g) {
                s.no_combat_movement = spot;
            }
        }
    }
    av_quests::back_from_the_dead(map);
}

/// Every route walked, and every wait over, now: as far as the
/// scripts go.
#[cfg(test)]
pub fn hurry(map: &mut Map) {
    for _ in 0..10 {
        av_routes::hurry(map);
        let now = map.now;
        let Some(st) = state_mut(map) else {
            return;
        };
        for r in st.riders.values_mut() {
            r.arrives = r.arrives.min(now);
            r.next = r.next.min(now);
        }
        if st.later.is_empty() {
            riders(map);
            return;
        }
        for l in st.later.iter_mut() {
            l.0 = now;
        }
        run_later(map);
    }
}

// The fighting, as EventAI.

struct Builder<'a> {
    data: &'a mut ScriptData,
    next: u32,
}

const T_PROVIDED: u32 = 0;
const T_VICTIM: u32 = 1;
const T_HOSTILE_SECOND: u32 = 2;
const TRIGGERED: u32 = 0x02;
const AURA_NOT_PRESENT: u32 = 0x20;

fn step(command: u32, datalong: [u32; 4]) -> ScriptStep {
    ScriptStep {
        command,
        datalong,
        ..Default::default()
    }
}

fn cast(spell: u32) -> ScriptStep {
    ScriptStep {
        target_type: T_VICTIM,
        ..step(15, [spell, 0, 0, 0])
    }
}

fn cast_at(spell: u32, target_type: u32) -> ScriptStep {
    ScriptStep {
        target_type,
        ..step(15, [spell, 0, 0, 0])
    }
}

fn self_cast(spell: u32, flags: u32) -> ScriptStep {
    ScriptStep {
        data_flags: SF_TARGET_SELF,
        target_type: T_PROVIDED,
        ..step(15, [spell, flags, 0, 0])
    }
}

fn say(texts: &[u32]) -> ScriptStep {
    let mut dataint = [0i32; 4];
    for (i, t) in texts.iter().take(4).enumerate() {
        dataint[i] = *t as i32;
    }
    ScriptStep {
        command: 0,
        dataint,
        ..Default::default()
    }
}

fn mount_step(display: u32) -> ScriptStep {
    step(24, [display, 1, 0, 0])
}

fn run_step(on: bool) -> ScriptStep {
    step(25, [u32::from(on), 0, 0, 0])
}

fn phase_step(phase: u32) -> ScriptStep {
    step(44, [phase, 0, 0, 0])
}

fn home_step() -> ScriptStep {
    step(34, [1, 0, 0, 0])
}

fn stand_step(state: u32) -> ScriptStep {
    step(28, [state, 0, 0, 0])
}

fn combat_movement_step(on: bool) -> ScriptStep {
    step(43, [u32::from(on), 0, 0, 0])
}

fn interrupt_step() -> ScriptStep {
    ScriptStep {
        data_flags: SF_TARGET_SELF,
        ..step(5, [0; 4])
    }
}

fn unaura_step(spell: u32) -> ScriptStep {
    ScriptStep {
        data_flags: SF_TARGET_SELF,
        ..step(14, [spell, 0, 0, 0])
    }
}

/// Only in these phases, as an inverse phase mask.
fn only_in(phases: &[u32]) -> u32 {
    let mut mask = u32::MAX;
    for p in phases {
        mask &= !(1 << p);
    }
    mask
}

impl Builder<'_> {
    fn script(&mut self, steps: Vec<ScriptStep>) -> u32 {
        self.next += 1;
        let id = self.next;
        let steps = steps.into_iter().map(|s| ScriptStep { id, ..s }).collect();
        self.data.ai_scripts.insert(id, steps);
        id
    }

    fn event(
        &mut self,
        entry: u32,
        kind: u32,
        params: [i32; 4],
        flags: u32,
        phases: u32,
        actions: Vec<Vec<ScriptStep>>,
    ) {
        let mut ids = [0u32; 3];
        for (i, a) in actions.into_iter().take(3).enumerate() {
            ids[i] = self.script(a);
        }
        self.next += 1;
        self.data.ai_events.entry(entry).or_default().push(AiEvent {
            id: self.next,
            creature: entry,
            condition: 0,
            kind,
            inverse_phase_mask: phases,
            chance: 100,
            flags,
            params,
            actions: ids,
        });
    }

    fn on(&mut self, entry: u32, kind: u32, phases: u32, steps: Vec<ScriptStep>) {
        self.event(entry, kind, [0; 4], 0, phases, vec![steps]);
    }

    /// In combat: first after `first`, then every `repeat` (ms).
    fn every(&mut self, entry: u32, first: (i32, i32), repeat: (i32, i32), steps: Vec<ScriptStep>) {
        self.event(
            entry,
            EVENT_TIMER_IN_COMBAT,
            [first.0, first.1, repeat.0, repeat.1],
            EF_REPEATABLE,
            0,
            vec![steps],
        );
    }

    /// With the victim between `min` and `max` yards, every `repeat`.
    fn within(
        &mut self,
        entry: u32,
        min: i32,
        max: i32,
        repeat: (i32, i32),
        steps: Vec<ScriptStep>,
    ) {
        self.event(
            entry,
            EVENT_RANGE,
            [min, max, repeat.0, repeat.1],
            EF_REPEATABLE,
            0,
            vec![steps],
        );
    }

    /// Below `pct` health: once a fight, or every `repeat` ms.
    fn below(&mut self, entry: u32, pct: i32, repeat: Option<i32>, steps: Vec<ScriptStep>) {
        let (flags, r) = match repeat {
            Some(r) => (EF_REPEATABLE, r),
            None => (0, 0),
        };
        self.event(entry, EVENT_HP, [pct, 0, r, r], flags, 0, vec![steps]);
    }
}

/// Every entry whose events are built here.
fn scripted_entries() -> Vec<u32> {
    let mut out = vec![VANNDAR, DREK_THAR, BALINDA, GALVANGAR];
    out.extend(14762..=14777);
    for s in &SUMMONERS {
        out.extend([s.entry, s.escort, s.lord]);
    }
    out.extend(WINGS.iter().map(|w| w.0));
    out.extend(HORDE_RIDERS);
    out.extend(ALLIANCE_RIDERS);
    for c in &CAVALRY {
        out.extend([c.commander, c.rider]);
    }
    for gr in &GROUND {
        out.extend(gr.troops..gr.troops + 4);
    }
    out
}

/// The valley's creatures' fighting, in place of the world database's
/// events (and the simple ones ported for some of them).
pub fn install(data: &mut GameData) {
    if !data.is_loaded() {
        return;
    }
    let s = &mut data.scripts;
    for e in scripted_entries() {
        s.ai_events.remove(&e);
    }
    let mut b = Builder {
        data: s,
        next: BASE,
    };
    leaders(&mut b);
    summoners(&mut b);
    // The wing commanders wait as prisoners: some sit.
    for (entry, ..) in WINGS {
        let sits = matches!(entry, 13439 | 13437);
        b.on(
            entry,
            EVENT_SPAWNED,
            0,
            vec![home_step(), stand_step(u32::from(sits))],
        );
    }
    // The war riders.
    for entry in HORDE_RIDERS.into_iter().chain(ALLIANCE_RIDERS) {
        b.every(entry, (0, 0), (5000, 5000), vec![cast(22088)]);
        b.every(entry, (8000, 8000), (7500, 7500), vec![cast(15285)]);
        b.every(entry, (13000, 13000), (9000, 9000), vec![cast(21188)]);
    }
    // The cavalry get down to fight, and mount again after.
    for c in &CAVALRY {
        b.on(
            c.commander,
            EVENT_SPAWNED,
            0,
            vec![mount_step(c.mount), home_step()],
        );
        for entry in [c.commander, c.rider] {
            b.on(entry, EVENT_AGGRO, 0, vec![mount_step(0)]);
            b.on(
                entry,
                EVENT_EVADE,
                0,
                vec![mount_step(c.mount), run_step(true)],
            );
        }
    }
    for gr in &GROUND {
        for entry in gr.troops..gr.troops + 4 {
            b.on(entry, EVENT_EVADE, 0, vec![run_step(true)]);
        }
    }
}

fn barks(b: &mut Builder, entry: u32, texts: [u32; 6]) {
    for pct in [80, 60, 40, 20, 5] {
        b.event(
            entry,
            EVENT_HP,
            [pct, 0, 0, 0],
            EF_RANDOM_ACTION,
            0,
            vec![
                vec![say(&texts[0..2])],
                vec![say(&texts[2..4])],
                vec![say(&texts[4..6])],
            ],
        );
    }
}

fn leaders(b: &mut Builder) {
    let whirlwind = 13736;
    // Vanndar Stormpike.
    b.on(VANNDAR, EVENT_AGGRO, 0, vec![say(&[10243])]);
    b.every(
        VANNDAR,
        (12000, 20000),
        (20000, 30000),
        vec![self_cast(19135, 0)],
    );
    b.every(VANNDAR, (8000, 8000), (8000, 10000), vec![cast(19136)]);
    b.every(
        VANNDAR,
        (5000, 5000),
        (5000, 8000),
        vec![self_cast(15588, 0)],
    );
    barks(b, VANNDAR, [8838, 8839, 8840, 8841, 8842, 8843]);
    // Drek'Thar.
    b.on(DREK_THAR, EVENT_AGGRO, 0, vec![say(&[10245])]);
    b.every(
        DREK_THAR,
        (8000, 12000),
        (12000, 16000),
        vec![self_cast(whirlwind, 0)],
    );
    b.every(DREK_THAR, (18000, 18000), (16000, 24000), vec![cast(19128)]);
    b.below(DREK_THAR, 30, Some(120000), vec![self_cast(28747, 0)]);
    barks(b, DREK_THAR, [8844, 8845, 8846, 8847, 8848, 8849]);
    // A general drawn out goes home and says so; one whose attackers fell
    // taunts them.
    for (entry, leashed, wiped) in [(VANNDAR, 10373, 10374), (DREK_THAR, 10377, 10376)] {
        b.on(
            entry,
            EVENT_EVADE,
            only_in(&[FIGHTING]),
            vec![say(&[wiped])],
        );
        b.on(
            entry,
            EVENT_EVADE,
            only_in(&[LEASHED]),
            vec![say(&[leashed]), phase_step(FIGHTING)],
        );
    }
    // Captain Balinda Stonehearth.
    b.on(BALINDA, EVENT_AGGRO, 0, vec![say(&[10054])]);
    b.below(BALINDA, 50, None, vec![say(&[10056])]);
    b.every(BALINDA, (0, 0), (2500, 3000), vec![cast(20420)]);
    b.within(BALINDA, 0, 10, (12000, 16000), vec![cast(22746)]);
    b.within(BALINDA, 0, 6, (6000, 9000), vec![cast(19712)]);
    b.every(
        BALINDA,
        (1750, 1750),
        (16000, 20000),
        vec![cast_at(15534, T_HOSTILE_SECOND)],
    );
    b.on(
        BALINDA,
        EVENT_EVADE,
        0,
        vec![say(&[10375]), combat_movement_step(true)],
    );
    // Captain Galvangar.
    b.on(GALVANGAR, EVENT_AGGRO, 0, vec![say(&[10055])]);
    b.below(GALVANGAR, 50, None, vec![say(&[10057])]);
    b.within(
        GALVANGAR,
        0,
        6,
        (10000, 16000),
        vec![self_cast(whirlwind, 0)],
    );
    b.every(GALVANGAR, (9000, 17000), (24000, 32000), vec![cast(19134)]);
    b.every(GALVANGAR, (4000, 4000), (7000, 9000), vec![cast(15284)]);
    b.every(GALVANGAR, (7000, 7000), (9000, 13000), vec![cast(16856)]);
    b.on(GALVANGAR, EVENT_EVADE, 0, vec![say(&[10378])]);
    // The marshals and warmasters.
    for entry in 14762..=14777 {
        b.within(entry, 8, 25, (12000, 18000), vec![cast(22911)]);
        b.every(entry, (8000, 8000), (8000, 10000), vec![cast(20684)]);
        b.every(
            entry,
            (4000, 4000),
            (14000, 20000),
            vec![self_cast(23511, 0)],
        );
        b.every(
            entry,
            (12000, 12000),
            (13000, 15000),
            vec![
                self_cast(whirlwind, 0),
                ScriptStep {
                    delay_ms: 2000,
                    ..self_cast(whirlwind, 0)
                },
            ],
        );
        b.below(entry, 30, Some(120000), vec![self_cast(8599, 0)]);
    }
}

fn summoners(b: &mut Builder) {
    // Primalist Thurloga.
    let thurloga = SUMMONERS[1].entry;
    b.on(
        thurloga,
        EVENT_AGGRO,
        0,
        vec![mount_step(0), self_cast(15786, 0)],
    );
    b.every(thurloga, (10000, 10000), (15000, 18000), vec![cast(16006)]);
    b.every(thurloga, (5000, 5000), (8000, 9000), vec![cast(15616)]);
    b.every(thurloga, (7500, 7500), (11000, 12000), vec![cast(15234)]);
    b.below(thurloga, 70, Some(7000), vec![self_cast(12492, 0)]);
    b.every(
        thurloga,
        (0, 0),
        (14000, 14000),
        vec![self_cast(15786, AURA_NOT_PRESENT)],
    );
    // Arch Druid Renferal.
    let renferal = SUMMONERS[0].entry;
    b.on(renferal, EVENT_AGGRO, 0, vec![mount_step(0)]);
    b.every(renferal, (3000, 3000), (9000, 13000), vec![cast(22127)]);
    b.every(
        renferal,
        (10000, 10000),
        (17000, 17000),
        vec![self_cast(15981, TRIGGERED)],
    );
    b.every(renferal, (0, 0), (7000, 7000), vec![cast(21668)]);
    // Their escorts: shield or thorns, and down from their mounts and out
    // of the invocation to fight.
    let shaman = SUMMONERS[1].escort;
    b.on(shaman, EVENT_SPAWNED, 0, vec![self_cast(12550, TRIGGERED)]);
    b.every(shaman, (5000, 5000), (5000, 6000), vec![cast(21401)]);
    b.below(shaman, 70, Some(7000), vec![self_cast(12492, 0)]);
    b.every(
        shaman,
        (9000, 9000),
        (9000, 11000),
        vec![self_cast(12550, AURA_NOT_PRESENT)],
    );
    let druid = SUMMONERS[0].escort;
    b.on(druid, EVENT_SPAWNED, 0, vec![self_cast(22128, TRIGGERED)]);
    b.every(druid, (3000, 3000), (9000, 13000), vec![cast(22127)]);
    b.every(druid, (0, 0), (7000, 9500), vec![cast(21668)]);
    for e in [shaman, druid] {
        b.on(
            e,
            EVENT_AGGRO,
            0,
            vec![mount_step(0), interrupt_step(), unaura_step(INVOCATION)],
        );
    }
    // Back from a fight: on their mount again, or invoking again.
    for s in &SUMMONERS {
        for (entry, display) in [(s.entry, s.mount), (s.escort, s.escort_mount)] {
            b.on(
                entry,
                EVENT_EVADE,
                only_in(&[MOUNTED]),
                vec![mount_step(display)],
            );
            b.on(
                entry,
                EVENT_EVADE,
                only_in(&[INVOKING]),
                vec![self_cast(INVOCATION, TRIGGERED)],
            );
        }
    }
    // Lokholar the Ice Lord.
    b.every(LOKHOLAR, (10000, 10000), (10000, 12000), vec![cast(21367)]);
    b.every(LOKHOLAR, (1000, 1000), (4500, 6500), vec![cast(21369)]);
    b.every(LOKHOLAR, (8500, 8500), (8000, 12000), vec![cast(14907)]);
    b.every(LOKHOLAR, (4000, 4000), (3000, 5000), vec![cast(19133)]);
    b.every(LOKHOLAR, (9000, 9000), (10000, 12000), vec![cast(15878)]);
    b.every(LOKHOLAR, (5500, 5500), (8000, 12000), vec![cast(16869)]);
    b.event(
        LOKHOLAR,
        EVENT_KILL,
        [0, 0, 1, 0],
        EF_REPEATABLE,
        0,
        vec![vec![say(&[8618]), self_cast(21307, TRIGGERED)]],
    );
    // Ivus the Forest Lord.
    b.every(IVUS, (8500, 8500), (6000, 8000), vec![self_cast(20654, 0)]);
    b.every(IVUS, (5500, 5500), (3000, 7000), vec![cast(21670)]);
    b.every(IVUS, (7000, 7000), (8000, 12000), vec![cast(21669)]);
    b.every(IVUS, (10000, 10000), (3000, 7000), vec![cast(21668)]);
    b.every(IVUS, (1000, 1000), (1000, 3000), vec![cast(21667)]);
}
