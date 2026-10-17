//! Escort and follower quests, as Thistle Tea keeps vmangos's
//! `npc_escortAI` and `FollowerAI` scripts: data ([`catalog`]) run by an
//! engine here. Accepting an escort's quest sets its escortee walking its
//! `script_waypoint` path along the navmesh, acting at chosen points (by
//! their point ids) and crediting the quest at its credit point; it may
//! hold at a point until what it summoned there is gone, or await a
//! script's word (a tome read, say) and give up without it; accepting
//! a follower's sets it trailing the player to its goal. Either fails the
//! quest if the escortee dies or the player strays too far, and puts the
//! escortee back where it stood at once. One that gets there goes after
//! its last wait and comes back on its spawn's own time, or in a second
//! for one that respawns at once.
//!
//! Each runs under its quest's map event, as Thistle Tea's do: the
//! escortee its source and the player its target, for half an hour.
//! Scripts reach the player through it (Remulos's credit for The Nightmare
//! Manifests), join it to go when it fails (Eranikus and his helpers), and
//! end it: badly, and the escort fails; well, and nothing fails it after.
//! A script that stops the escortee where it stands holds the escort there
//! until one sets it walking again.

pub mod catalog;

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Language, PlayerChatTag, SMSG_MESSAGECHAT, SMSG_MESSAGECHAT_ChatType, SMSG_QUESTUPDATE_FAILED,
    Vector3d,
};

use crate::game::data::CreatureSpawn;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;

/// Whom a summoned creature attacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    None,
    Escort,
    Player,
}

/// What an escortee or follower does.
#[derive(Debug, Clone, Copy)]
pub enum Act {
    Say(u32),
    SayBy(u32, u32),
    Emote(u32),
    Faction(u32),
    Stand(u8),
    RemoveUnitFlags(u32),
    NpcFlags(u32),
    Pause(u32),
    Cast(u32),
    Run,
    Walk,
    After(u32, &'static [Act]),
    /// `despawn_ms` `u32::MAX`: until it dies.
    Summon {
        entry: u32,
        at: (f32, f32, f32, f32),
        aim: Aim,
        despawn_ms: u32,
    },
    /// `count` at once, each on a walkable point within `scatter` yards of
    /// the place, or of the player's feet.
    SummonMany {
        entry: u32,
        at: (f32, f32, f32, f32),
        at_player: bool,
        count: u32,
        scatter: f32,
        aim: Aim,
        despawn_ms: u32,
    },
    /// Unit flags set, until it respawns.
    UnitFlags(u32),
    /// Its EventAI phase.
    EventPhase(u32),
    /// 0 passive, 1 defensive, 2 aggressive.
    React(u32),
    /// A spell on itself, with its cast time.
    CastTimed(u32),
    /// Its cast or channel of the spell ended.
    Interrupt(u32),
    /// A field of the copy's instance data set.
    InstanceData(u32, u32),
    /// A game object anyone may use, at a place, for a while (ms).
    SummonObject(u32, (f32, f32, f32, f32), u32),
    /// The nearest creature of an entry hears a script event.
    Signal(u32, u32),
    /// It turns to the nearest creature of an entry.
    Face(u32),
    /// The quest credited to the player and their group near.
    Credit,
    /// The nearest creature of an entry plays an emote.
    EmoteBy(u32, u32),
    /// The escort stops at its point until every creature it summoned
    /// from this point on is gone, for at most 400 s, then these run
    /// (Thistle Tea's `{:hold, actions}`).
    Hold(&'static [Act]),
    /// The escort stops at its point until a script lets it go
    /// (`release_waypoints`), then the first run; if nothing does within
    /// the time (ms), the second (Thistle Tea's `{:await, ...}`).
    Await(u32, &'static [Act], &'static [Act]),
    /// The nearest game object of an entry, within 40 yards, set to a
    /// state.
    ObjectState(u32, u32),
    /// The escort ends well, and the escortee dies.
    Die,
    /// The escort ends badly: the quest fails and the escortee is put
    /// back (vmangos's `ResetEscort`).
    Abort,
    /// The quest fails for the player and their group; the escort goes on.
    Fail,
    /// It never falls below this share of its health (percent).
    Invincible(u32),
    /// An aura of its own, gone.
    RemoveAura(u32),
    /// It turns on the player.
    Attack,
}

#[derive(Debug, Clone, Copy)]
pub struct Escort {
    pub quest: u32,
    pub entry: u32,
    pub credit_point: Option<u32>,
    pub giver: Option<u32>,
    pub max_distance: f32,
    pub start_delay_ms: u32,
    pub credit_delay_ms: u32,
    pub instant_respawn: bool,
    pub accept: &'static [Act],
    pub points: &'static [(u32, &'static [Act])],
    /// Points of its own, where the script walks no `script_waypoint`s.
    pub path: Option<&'static [(f32, f32, f32, u32)]>,
}

/// Where a follower is going: near a creature, or a place.
#[derive(Debug, Clone, Copy)]
pub enum Goal {
    Creature(u32, f32),
    Point(f32, f32, f32, f32),
}

#[derive(Debug, Clone, Copy)]
pub struct Follower {
    pub quest: u32,
    pub entry: u32,
    pub goal: Goal,
    pub credit: bool,
    pub despawn_ms: u32,
    pub max_distance: f32,
    pub accept: &'static [Act],
    pub arrive: &'static [Act],
}

/// A point of an escort's path: where, its wait, and its point id.
#[derive(Debug, Clone, Copy)]
struct Point {
    at: Vector3d,
    wait_ms: u32,
    id: u32,
}

#[derive(Debug, Clone)]
enum Kind {
    Escort {
        def: Escort,
        path: Vec<Point>,
        /// The point it is walking to, or waiting at.
        next: usize,
        /// Where the walk under way ends, to tell its arrival from a walk
        /// a fight broke off.
        walking: Option<Vector3d>,
        /// At `next`, its acts run.
        arrived: bool,
        waiting_until: Option<Instant>,
        start_at: Instant,
        /// A hold or await at its point.
        hold: Option<Hold>,
    },
    Follower {
        def: Follower,
        arrived: bool,
    },
}

/// An escort held at its point, as Thistle Tea's `WaypointHold` has it.
#[derive(Debug, Clone, Copy)]
struct Hold {
    until: Instant,
    /// Not let go before this: the summons beside it are counted first.
    ready_at: Instant,
    /// An await, let go by a script; else a hold, by its summons' going.
    signal: bool,
    signaled: bool,
    /// The summons it waits on: the escort's, from this index.
    from: usize,
    release: &'static [Act],
    expired: Option<&'static [Act]>,
}

/// How long a hold waits on its summons at most.
const HOLD_MAX: Duration = Duration::from_secs(400);

/// An escort or follower under way.
#[derive(Debug, Clone)]
pub struct Active {
    pub quest: u32,
    pub player: Guid,
    pub creature: Guid,
    kind: Kind,
    run: bool,
    delayed: Vec<(Instant, &'static [Act])>,
    summons: Vec<(Guid, Instant)>,
    /// Every creature it summoned, for its holds.
    summoned: Vec<Guid>,
    /// When it ends (and the escortee goes home).
    ends_at: Option<Instant>,
    failed: bool,
    /// It got there: the escortee goes, and comes back on its spawn's
    /// time.
    done: bool,
    /// Its map event ended well: nothing fails it now.
    event_over: bool,
}

impl Active {
    /// Set out now, not after the start delay.
    pub fn skip_start(&mut self) {
        if let Kind::Escort { start_at, .. } = &mut self.kind {
            *start_at = Instant::now() - Duration::from_secs(1);
        }
    }

    /// Its clocks `by` on: what it waits for (its start, its waits, its
    /// delayed acts, a hold's time) comes sooner.
    pub fn hurry(&mut self, by: Duration) {
        let sooner = |t: &mut Instant| {
            if let Some(s) = t.checked_sub(by) {
                *t = s;
            }
        };
        for (t, _) in self.delayed.iter_mut() {
            sooner(t);
        }
        if let Some(t) = self.ends_at.as_mut() {
            sooner(t);
        }
        if let Kind::Escort {
            start_at,
            waiting_until,
            hold,
            ..
        } = &mut self.kind
        {
            sooner(start_at);
            if let Some(t) = waiting_until.as_mut() {
                sooner(t);
            }
            if let Some(h) = hold.as_mut() {
                sooner(&mut h.until);
                sooner(&mut h.ready_at);
            }
        }
    }

    /// Whether it is held at its point, by a hold or an await.
    pub fn held(&self) -> bool {
        matches!(&self.kind, Kind::Escort { hold: Some(_), .. })
    }

    /// The id of the point an escort is bound for, or waits at.
    pub fn point(&self) -> Option<u32> {
        match &self.kind {
            Kind::Escort { path, next, .. } => path.get(*next).map(|p| p.id),
            Kind::Follower { .. } => None,
        }
    }
}

/// How far from a player a giver's escortee may stand.
const GIVER_REACH: f32 = 30.0;
const SAY_YARDS: f32 = 25.0;

pub fn escort(quest: u32) -> Option<&'static Escort> {
    catalog::ESCORTS.iter().find(|e| e.quest == quest)
}

pub fn follower(quest: u32) -> Option<&'static Follower> {
    catalog::FOLLOWERS.iter().find(|f| f.quest == quest)
}

/// The nearest living creature of `entry` to `at`, within `reach`.
fn nearest(map: &Map, entry: u32, at: Vector3d, reach: f32) -> Option<Guid> {
    map.grid
        .around(at.x, at.y, reach)
        .filter(|g| crate::game::guid::entry(*g) == entry && crate::game::guid::is_unit(*g))
        .filter_map(|g| map.creature(g))
        .filter(|c| c.alive() && dist(c.position, at) <= reach)
        .min_by(|a, b| dist(a.position, at).total_cmp(&dist(b.position, at)))
        .map(|c| c.guid)
}

/// A quest was accepted: its escort or follower sets out.
pub fn accepted(map: &mut Map, player: Guid, quest: u32) {
    let Some(at) = map.players.get(&player).map(|p| p.position()) else {
        return;
    };
    let now = map.now;
    let (entry, kind, accept) = if let Some(def) = escort(quest) {
        // Its own points are numbered from 0; `script_waypoint`'s by their
        // point ids.
        let mut path: Vec<Point> = match def.path {
            Some(points) => points
                .iter()
                .zip(0..)
                .map(|((x, y, z, w), id)| Point {
                    at: Vector3d {
                        x: *x,
                        y: *y,
                        z: *z,
                    },
                    wait_ms: *w,
                    id,
                })
                .collect(),
            None => map
                .realm
                .data
                .world
                .script_waypoints
                .get(&def.entry)
                .map(|ps| {
                    ps.iter()
                        .map(|p| Point {
                            at: Vector3d {
                                x: p.x,
                                y: p.y,
                                z: p.z,
                            },
                            wait_ms: p.wait_ms,
                            id: p.point,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        path.sort_by_key(|p| p.id);
        if path.is_empty() {
            return;
        }
        path.shrink_to_fit();
        (
            def.entry,
            Kind::Escort {
                def: *def,
                path,
                next: 0,
                walking: None,
                arrived: false,
                waiting_until: None,
                start_at: now + Duration::from_millis(u64::from(def.start_delay_ms)),
                hold: None,
            },
            def.accept,
        )
    } else if let Some(def) = follower(quest) {
        (
            def.entry,
            Kind::Follower {
                def: *def,
                arrived: false,
            },
            def.accept,
        )
    } else {
        return;
    };
    let Some(creature) = nearest(map, entry, at, GIVER_REACH) else {
        return;
    };
    if map.escorts.iter().any(|a| a.creature == creature) {
        return;
    }
    if let Some(c) = map.creature_mut(creature) {
        c.fields.set_u32(UNIT_NPC_FLAGS, 0);
        c.movement_type = crate::game::map::creature::MOVEMENT_IDLE;
        c.motion = Motion::Idle { until: None };
    }
    map.escorts.push(Active {
        quest,
        player,
        creature,
        kind,
        run: false,
        delayed: Vec::new(),
        summons: Vec::new(),
        summoned: Vec::new(),
        ends_at: None,
        failed: false,
        done: false,
        event_over: false,
    });
    // The quest's map event, as Thistle Tea starts one for each escort,
    // for half an hour: scripts reach the player through it, and join it
    // to go when it fails.
    crate::game::map::map_events::start(map, quest, creature, player, EVENT_LASTS);
    let i = map.escorts.len() - 1;
    run_acts(map, i, accept);
}

/// How long an escort's map event lasts: past it, the escort fails.
const EVENT_LASTS: Duration = Duration::from_secs(1800);

/// The map event `id` of an escort of `source` ended, by a script or its
/// time, as Thistle Tea's escort event has it: well, and nothing fails the
/// escort from then on (a script that ends it so has credited the quest,
/// and sends the escortee off itself); badly, and the quest fails and the
/// escortee is put back.
pub fn event_ended(map: &mut Map, id: u32, source: Guid, success: bool) {
    let now = map.now;
    let Some(i) = map
        .escorts
        .iter()
        .position(|a| a.quest == id && a.creature == source && a.ends_at.is_none())
    else {
        return;
    };
    if success {
        map.escorts[i].event_over = true;
        return;
    }
    if !map.escorts[i].failed {
        let player = map.escorts[i].player;
        fail(map, player, id);
        map.escorts[i].failed = true;
    }
    map.escorts[i].ends_at = Some(now);
}

/// A creature says a broadcast text.
fn say(map: &Map, who: Guid, text: u32) {
    let Some(c) = map.creature(who) else {
        return;
    };
    let data = &map.realm.data;
    let Some(line) = data.npc.broadcast_line(text, false) else {
        return;
    };
    let name = data
        .creature_template(c.entry)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let Some(frame) = crate::game::conn::encode(&SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::MonsterSay {
            sender1: who,
            sender_name: name,
            target: Guid::zero(),
        },
        language: Language::Universal,
        message: line.to_owned(),
        tag: PlayerChatTag::None,
    }) else {
        return;
    };
    for g in map.players_near(c.position, SAY_YARDS) {
        if let Some(p) = map.players.get(&g) {
            p.outbound.send_frame(frame.clone());
        }
    }
}

fn run_acts(map: &mut Map, i: usize, acts: &'static [Act]) {
    let now = map.now;
    let (creature, player) = (map.escorts[i].creature, map.escorts[i].player);
    // A hold waits on what these acts summon, those before it too.
    let batch = map.escorts[i].summoned.len();
    for act in acts {
        match *act {
            Act::Say(text) => say(map, creature, text),
            Act::SayBy(entry, text) => {
                if let Some(at) = map.creature(creature).map(|c| c.position)
                    && let Some(by) = nearest(map, entry, at, 60.0)
                {
                    say(map, by, text);
                }
            }
            Act::Emote(emote) => crate::game::map::scripts::emote(map, creature, emote),
            // As a script sets it, so whom it is hostile to follows.
            Act::Faction(f) => script_step(map, creature, creature, 22, [f, 0, 0, 0]),
            Act::Stand(s) => {
                if let Some(c) = map.creature_mut(creature) {
                    c.fields.set_byte(UNIT_FIELD_BYTES_1, 0, s);
                }
            }
            Act::RemoveUnitFlags(m) => {
                if let Some(c) = map.creature_mut(creature) {
                    let f = c.fields.get_u32(UNIT_FIELD_FLAGS);
                    c.fields.set_u32(UNIT_FIELD_FLAGS, f & !m);
                }
            }
            Act::NpcFlags(m) => {
                if let Some(c) = map.creature_mut(creature) {
                    c.fields.set_u32(UNIT_NPC_FLAGS, m);
                }
            }
            Act::Pause(ms) => {
                if let Kind::Escort { waiting_until, .. } = &mut map.escorts[i].kind {
                    let base = waiting_until.unwrap_or(now).max(now);
                    *waiting_until = Some(base + Duration::from_millis(u64::from(ms)));
                }
            }
            Act::Cast(spell) => {
                crate::game::map::spells::cast_triggered(map, creature, spell, Some(creature));
            }
            Act::Run => map.escorts[i].run = true,
            Act::Walk => map.escorts[i].run = false,
            Act::After(ms, later) => map.escorts[i]
                .delayed
                .push((now + Duration::from_millis(u64::from(ms)), later)),
            Act::Summon {
                entry,
                at,
                aim,
                despawn_ms,
            } => {
                let near = map.creature(creature).map(|c| c.position);
                let pos = if at.0 == 0.0 && at.1 == 0.0 && at.2 == 0.0 {
                    near
                } else {
                    Some(Vector3d {
                        x: at.0,
                        y: at.1,
                        z: at.2,
                    })
                };
                let Some(pos) = pos else {
                    continue;
                };
                summon_for(map, i, entry, pos, at.3, aim, despawn_ms);
            }
            Act::SummonMany {
                entry,
                at,
                at_player,
                count,
                scatter,
                aim,
                despawn_ms,
            } => {
                let place = if at_player {
                    map.players.get(&player).map(|p| p.position())
                } else {
                    Some(Vector3d {
                        x: at.0,
                        y: at.1,
                        z: at.2,
                    })
                };
                let Some(place) = place else {
                    continue;
                };
                for _ in 0..count {
                    let pos = (scatter > 0.0)
                        .then(|| {
                            map.nav
                                .as_mut()
                                .and_then(|n| n.random_point(place, scatter))
                        })
                        .flatten()
                        .unwrap_or(place);
                    summon_for(map, i, entry, pos, at.3, aim, despawn_ms);
                }
            }
            Act::UnitFlags(m) => {
                if let Some(c) = map.creature_mut(creature) {
                    let f = c.fields.get_u32(UNIT_FIELD_FLAGS);
                    c.fields.set_u32(UNIT_FIELD_FLAGS, f | m);
                }
            }
            Act::EventPhase(phase) => script_step(map, creature, creature, 44, [phase, 0, 0, 0]),
            Act::React(state) => script_step(map, creature, creature, 59, [state, 0, 0, 0]),
            Act::CastTimed(spell) => {
                crate::game::map::scripts::cast(map, creature, creature, spell, 0);
            }
            Act::Interrupt(spell) => {
                let casting = map
                    .caster(creature)
                    .and_then(|c| c.casting.as_ref())
                    .is_some_and(|c| c.spell == spell);
                if casting {
                    crate::game::map::spells::interrupt(map, creature);
                }
            }
            Act::InstanceData(field, value) => {
                crate::game::map::instance_script::set_data(map, field, value, 0);
            }
            Act::SummonObject(entry, at, ms) => {
                crate::game::map::objects::summon(
                    map,
                    None,
                    entry,
                    Vector3d {
                        x: at.0,
                        y: at.1,
                        z: at.2,
                    },
                    at.3,
                    ms,
                    0,
                    0,
                    None,
                );
            }
            Act::Signal(entry, event) => {
                if let Some(at) = map.creature(creature).map(|c| c.position)
                    && let Some(by) = nearest(map, entry, at, 150.0)
                {
                    script_step(map, by, creature, 85, [event, 0, 0, 0]);
                }
            }
            Act::Face(entry) => {
                if let Some(at) = map.creature(creature).map(|c| c.position)
                    && let Some(to) = nearest(map, entry, at, 150.0)
                {
                    script_step(map, creature, to, 35, [0; 4]);
                }
            }
            Act::Credit => {
                let (quest, reach) = (map.escorts[i].quest, max_distance(&map.escorts[i].kind));
                if let Some(at) = map.creature(creature).map(|c| c.position) {
                    credit(map, player, quest, reach, at);
                }
            }
            Act::EmoteBy(entry, emote) => {
                if let Some(at) = map.creature(creature).map(|c| c.position)
                    && let Some(by) = nearest(map, entry, at, 150.0)
                {
                    crate::game::map::scripts::emote(map, by, emote);
                }
            }
            Act::Hold(release) => hold(map, i, now, HOLD_MAX, false, batch, release, None),
            Act::Await(ms, release, expired) => {
                let wait = Duration::from_millis(u64::from(ms));
                hold(map, i, now, wait, true, batch, release, Some(expired));
            }
            Act::ObjectState(entry, state) => {
                if let Some(at) = map.creature(creature).map(|c| c.position)
                    && let Some(o) = map
                        .gameobjects
                        .iter()
                        .filter(|(_, o)| o.entry == entry && dist(o.position, at) <= 40.0)
                        .min_by(|(_, a), (_, b)| {
                            dist(a.position, at).total_cmp(&dist(b.position, at))
                        })
                        .map(|(_, o)| o.guid)
                {
                    crate::game::map::scripts::set_object_state(map, o, state);
                }
            }
            Act::Die => {
                map.escorts[i].done = true;
                map.escorts[i].ends_at = Some(now);
                if let Some(hp) = map
                    .creature(creature)
                    .filter(|c| c.alive())
                    .map(|c| c.fields.get_u32(UNIT_FIELD_HEALTH))
                {
                    crate::game::map::combat::deal_damage(map, creature, creature, hp);
                }
                return;
            }
            Act::Abort => {
                if !map.escorts[i].failed {
                    fail(map, player, map.escorts[i].quest);
                }
                map.escorts[i].failed = true;
                map.escorts[i].ends_at = Some(now);
                return;
            }
            Act::Fail => {
                let quest = map.escorts[i].quest;
                script_step(map, creature, player, 70, [quest, 0, 0, 0]);
                map.escorts[i].failed = true;
            }
            Act::Invincible(pct) => script_step(map, creature, creature, 52, [pct, 1, 0, 0]),
            Act::RemoveAura(spell) => {
                crate::game::map::auras::remove_spell(map, creature, spell, None);
            }
            Act::Attack => script_step(map, creature, player, 26, [0; 4]),
        }
    }
}

/// Escort `i` held at its point, as Thistle Tea's `WaypointHold.start`
/// has it: until `wait` is up, or sooner, by a script for an await, or for
/// a hold once the summons from `from` on are gone.
#[allow(clippy::too_many_arguments)]
fn hold(
    map: &mut Map,
    i: usize,
    now: Instant,
    wait: Duration,
    signal: bool,
    from: usize,
    release: &'static [Act],
    expired: Option<&'static [Act]>,
) {
    if let Kind::Escort { hold, .. } = &mut map.escorts[i].kind {
        *hold = Some(Hold {
            until: now + wait,
            ready_at: now + Duration::from_millis(1),
            signal,
            signaled: false,
            from,
            release,
            expired,
        });
    }
}

/// A script stops `creature` where it stands (movement idle), as Thistle
/// Tea's idle motion takes an escortee off its path: its escort waits
/// there until a script sets it walking again ([`resume`]), or its map
/// event ends.
pub fn halt(map: &mut Map, creature: Guid) {
    let now = map.now;
    if let Some(i) = map.escorts.iter().position(|a| a.creature == creature) {
        hold(map, i, now, EVENT_LASTS, true, 0, &[], None);
    }
}

/// A script sets `creature` walking its path again: a halted escort goes
/// on.
pub fn resume(map: &mut Map, creature: Guid) {
    if let Some(a) = map.escorts.iter_mut().find(|a| a.creature == creature)
        && let Kind::Escort { hold, .. } = &mut a.kind
        && hold.is_some_and(|h| h.signal && h.release.is_empty() && h.expired.is_none())
    {
        *hold = None;
    }
}

/// A script lets `creature`'s escort go from its await or hold
/// (`release_waypoints`): whether it had one.
pub fn release(map: &mut Map, creature: Guid) -> bool {
    let now = map.now;
    let Some(a) = map.escorts.iter_mut().find(|a| a.creature == creature) else {
        return false;
    };
    let Kind::Escort { hold: Some(h), .. } = &mut a.kind else {
        return false;
    };
    h.signaled |= now < h.until;
    h.until = h.until.min(now);
    true
}

/// Whether escort `i`'s hold lets it go now: the acts to run if so.
fn held(map: &Map, i: usize, now: Instant) -> Option<Option<&'static [Act]>> {
    let Kind::Escort { hold: Some(h), .. } = &map.escorts[i].kind else {
        return Some(None);
    };
    if now >= h.until {
        return Some(Some(match h.expired {
            Some(expired) if h.signal && !h.signaled => expired,
            _ => h.release,
        }));
    }
    if h.signal || now < h.ready_at {
        return None;
    }
    let gone = map.escorts[i].summoned[h.from.min(map.escorts[i].summoned.len())..]
        .iter()
        .all(|g| map.creature(*g).is_none_or(|c| !c.alive()));
    gone.then_some(Some(h.release))
}

/// One script command from `source` on `target`.
fn script_step(map: &mut Map, source: Guid, target: Guid, command: u32, datalong: [u32; 4]) {
    let step = crate::game::data::scripts::ScriptStep {
        command,
        datalong,
        ..Default::default()
    };
    crate::game::map::scripts::execute(map, &step, source, target);
}

/// A summon of escort `i`'s, gone after `despawn_ms` (`u32::MAX`: when it
/// dies), and whom it attacks.
#[allow(clippy::too_many_arguments)]
fn summon_for(
    map: &mut Map,
    i: usize,
    entry: u32,
    pos: Vector3d,
    o: f32,
    aim: Aim,
    despawn_ms: u32,
) {
    let now = map.now;
    let (creature, player) = (map.escorts[i].creature, map.escorts[i].player);
    let Some(g) = summon(map, entry, pos, o) else {
        return;
    };
    map.escorts[i].summoned.push(g);
    if despawn_ms != u32::MAX {
        map.escorts[i]
            .summons
            .push((g, now + Duration::from_millis(u64::from(despawn_ms))));
    }
    match aim {
        Aim::Escort => crate::game::map::ai::attacked(map, g, creature),
        Aim::Player => crate::game::map::ai::attacked(map, g, player),
        Aim::None => {}
    }
}

fn max_distance(kind: &Kind) -> f32 {
    match kind {
        Kind::Escort { def, .. } => def.max_distance,
        Kind::Follower { def, .. } => def.max_distance,
    }
}

/// A creature of `entry` at `pos`, for a while: it doesn't respawn.
fn summon(map: &mut Map, entry: u32, pos: Vector3d, o: f32) -> Option<Guid> {
    let data = map.realm.data.clone();
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let spawn = CreatureSpawn {
        guid: counter | 0x80_0000,
        entries: vec![entry],
        map: map.key.map,
        x: pos.x,
        y: pos.y,
        z: pos.z,
        orientation: o,
        ..Default::default()
    };
    let mut c = crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())?;
    c.spawn_guid = None;
    Some(map.add_creature(c))
}

/// The quest is credited to the player and their group near.
fn credit(map: &mut Map, player: Guid, quest: u32, reach: f32, near: Vector3d) {
    let mut who = vec![player];
    {
        let groups = map.realm.social.groups.lock();
        if let Some(g) = groups.group_of(player) {
            who.extend(g.members.iter().map(|m| m.guid).filter(|g| *g != player));
        }
    }
    let data = map.realm.data.clone();
    for g in who {
        let Some(p) = map.players.get_mut(&g) else {
            continue;
        };
        if g != player && dist(p.position(), near) > reach {
            continue;
        }
        let Some(slot) = p.journal.slot_of(quest) else {
            continue;
        };
        p.journal.log[slot].explored = true;
        crate::game::map::quests::refresh_log(p, &data);
        crate::game::map::quests::check(map, g, quest);
    }
}

fn fail(map: &mut Map, player: Guid, quest: u32) {
    let data = map.realm.data.clone();
    if let Some(p) = map.players.get_mut(&player)
        && let Some(slot) = p.journal.slot_of(quest)
    {
        p.journal.log[slot].failed = true;
        crate::game::map::quests::refresh_log(p, &data);
        p.outbound
            .send(&SMSG_QUESTUPDATE_FAILED { quest_id: quest });
    }
}

/// The escortee goes back where it stood, as it was, at once.
fn reset(map: &mut Map, creature: Guid) {
    let data = map.realm.data.clone();
    let spawn = map
        .creature(creature)
        .and_then(|c| c.spawn_guid)
        .and_then(|s| {
            data.creature_spawns
                .get(&map.key.map)
                .and_then(|l| l.iter().find(|x| x.guid == s))
                .cloned()
        });
    map.remove_creature(creature);
    if let Some(s) = spawn {
        let on = crate::game::events::current(&map.realm);
        if let Some(c) = crate::game::events::spawn_creature(&data, &s, &on, &mut rand::rng()) {
            map.add_creature(c);
        }
    }
}

/// An escortee whose escort got there goes, as Thistle Tea's `despawn`
/// has it: back in a second if it respawns at once, else on its spawn's
/// own respawn time.
fn depart(map: &mut Map, creature: Guid, instant: bool) {
    let data = map.realm.data.clone();
    let Some(spawn) = map.creature(creature).map(|c| c.spawn_guid) else {
        return;
    };
    let Some(spawn) = spawn.and_then(|s| {
        data.creature_spawns
            .get(&map.key.map)
            .and_then(|l| l.iter().find(|x| x.guid == s))
    }) else {
        map.remove_creature(creature);
        return;
    };
    let wait = if instant {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(u64::from(spawn.respawn_min.max(1)))
    };
    crate::game::map::scripts::despawn(map, creature);
    let now = map.now;
    if let Some(c) = map.creature_mut(creature)
        && let crate::game::map::creature::Life::Dead { respawn_at, .. } = &mut c.life
    {
        *respawn_at = now + wait;
    }
}

/// It walks (or runs) to `to`, along the navmesh: where the walk ends, if
/// it set out.
fn walk_to(map: &mut Map, creature: Guid, to: Vector3d, run: bool) -> Option<Vector3d> {
    crate::game::map::scripts::move_with(
        map,
        creature,
        to,
        crate::game::map::scripts::Move {
            run,
            pathfind: true,
            ..Default::default()
        },
    );
    match &map.creature(creature)?.motion {
        Motion::Moving(w) => w.points.last().copied(),
        _ => Some(to),
    }
}

/// The flat distance between two places.
fn dist2(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Escorts and followers move along, act, credit and fail.
pub fn update(map: &mut Map) {
    if map.escorts.is_empty() {
        return;
    }
    let now = map.now;
    let mut i = 0;
    while i < map.escorts.len() {
        if step(map, i, now) {
            i += 1;
        } else {
            let a = map.escorts.remove(i);
            // Its map event ends with it, and those who joined it hear.
            if crate::game::map::map_events::target_of(map, 22, a.quest, 0) == Some(a.creature) {
                crate::game::map::map_events::end(map, a.quest, a.done);
            }
            for (g, _) in &a.summons {
                map.remove_creature(*g);
            }
            if a.done {
                let instant = match &a.kind {
                    Kind::Escort { def, .. } => def.instant_respawn,
                    Kind::Follower { .. } => false,
                };
                depart(map, a.creature, instant);
            } else {
                reset(map, a.creature);
            }
        }
    }
}

/// One escort's step; false when it is over.
fn step(map: &mut Map, i: usize, now: Instant) -> bool {
    let (creature, player, quest) = (
        map.escorts[i].creature,
        map.escorts[i].player,
        map.escorts[i].quest,
    );
    // Summons whose time is up go.
    let gone: Vec<Guid> = map.escorts[i]
        .summons
        .iter()
        .filter(|(_, t)| *t <= now)
        .map(|(g, _)| *g)
        .collect();
    map.escorts[i].summons.retain(|(_, t)| *t > now);
    for g in gone {
        map.remove_creature(g);
    }
    if let Some(end) = map.escorts[i].ends_at {
        return end > now;
    }
    // Delayed acts come due.
    let due: Vec<&'static [Act]> = map.escorts[i]
        .delayed
        .iter()
        .filter(|(t, _)| *t <= now)
        .map(|(_, a)| *a)
        .collect();
    map.escorts[i].delayed.retain(|(t, _)| *t > now);
    for acts in due {
        run_acts(map, i, acts);
    }
    let Some((cpos, alive, fighting, moving)) = map.creature(creature).map(|c| {
        (
            c.position,
            c.alive(),
            c.combat.attacking,
            matches!(c.motion, Motion::Moving(_)),
        )
    }) else {
        return false;
    };
    // No limit to how far the player may be, for one with none.
    let max = max_distance(&map.escorts[i].kind);
    let ppos = map.players.get(&player).map(|p| p.position());
    let strayed = ppos.is_none_or(|p| max > 0.0 && dist(p, cpos) > max);
    let failable = !map.escorts[i].failed && !map.escorts[i].event_over;
    if !alive || (strayed && failable) {
        if failable {
            fail(map, player, quest);
        }
        return false;
    }
    if fighting || moving {
        return true;
    }
    let run = map.escorts[i].run;
    match map.escorts[i].kind.clone() {
        Kind::Escort {
            def,
            path,
            next,
            walking,
            arrived,
            waiting_until,
            start_at,
            hold: _,
        } => {
            // Held at its point: until it is let go, then what that runs.
            match held(map, i, now) {
                None => return true,
                Some(Some(acts)) => {
                    if let Kind::Escort { hold, .. } = &mut map.escorts[i].kind {
                        *hold = None;
                    }
                    run_acts(map, i, acts);
                    return true;
                }
                Some(None) => {}
            }
            if now < start_at || waiting_until.is_some_and(|t| t > now) {
                return true;
            }
            let Some(point) = path.get(next).copied() else {
                return false;
            };
            if !arrived {
                // There: the point's acts, its credit, its wait; and a
                // fight from here on comes back here.
                let there = walking.is_some_and(|end| dist2(cpos, end) <= 2.0)
                    || dist(cpos, point.at) < 1.5;
                if there {
                    if let Kind::Escort {
                        walking, arrived, ..
                    } = &mut map.escorts[i].kind
                    {
                        *walking = None;
                        *arrived = true;
                    }
                    if let Some(c) = map.creature_mut(creature) {
                        c.home = cpos;
                    }
                    if let Some((_, acts)) = def.points.iter().find(|(p, _)| *p == point.id) {
                        run_acts(map, i, acts);
                    }
                    if def.credit_point == Some(point.id) {
                        const CREDIT: &[Act] = &[Act::Credit];
                        let delay = Duration::from_millis(u64::from(def.credit_delay_ms));
                        if delay.is_zero() {
                            run_acts(map, i, CREDIT);
                        } else {
                            map.escorts[i].delayed.push((now + delay, CREDIT));
                        }
                    }
                    if let Kind::Escort { waiting_until, .. } = &mut map.escorts[i].kind {
                        let base = waiting_until.unwrap_or(now).max(now);
                        *waiting_until =
                            Some(base + Duration::from_millis(u64::from(point.wait_ms)));
                    }
                    return true;
                }
                // On its way, or back on it after a fight.
                let end = walk_to(map, creature, point.at, run);
                if let Kind::Escort { walking, .. } = &mut map.escorts[i].kind {
                    *walking = end;
                }
                return true;
            }
            // Its wait over: on to the next point, or, past the last, done.
            if next + 1 >= path.len() {
                map.escorts[i].done = true;
                map.escorts[i].ends_at = Some(now);
                return true;
            }
            if let Kind::Escort {
                next,
                arrived,
                waiting_until,
                walking,
                ..
            } = &mut map.escorts[i].kind
            {
                *next += 1;
                *arrived = false;
                *waiting_until = None;
                *walking = None;
            }
            let to = path[next + 1].at;
            let end = walk_to(map, creature, to, run);
            if let Kind::Escort { walking, .. } = &mut map.escorts[i].kind {
                *walking = end;
            }
            true
        }
        Kind::Follower { def, arrived } => {
            if arrived {
                return true;
            }
            let reached = match def.goal {
                Goal::Creature(entry, radius) => nearest(map, entry, cpos, radius).is_some(),
                Goal::Point(x, y, z, radius) => dist(cpos, Vector3d { x, y, z }) <= radius,
            };
            if reached {
                if def.credit {
                    credit(map, player, quest, def.max_distance, cpos);
                }
                if let Kind::Follower { arrived, .. } = &mut map.escorts[i].kind {
                    *arrived = true;
                }
                run_acts(map, i, def.arrive);
                map.escorts[i].done = true;
                map.escorts[i].ends_at =
                    Some(now + Duration::from_millis(u64::from(def.despawn_ms.max(1000))));
                return true;
            }
            // Trailing the player, two yards to their side.
            if let Some(p) = ppos
                && dist(p, cpos) > 3.0
            {
                let o = map
                    .players
                    .get(&player)
                    .map_or(0.0, |p| p.movement.orientation)
                    + std::f32::consts::FRAC_PI_2;
                let to = Vector3d {
                    x: p.x + 2.0 * o.cos(),
                    y: p.y + 2.0 * o.sin(),
                    z: p.z,
                };
                walk_to(map, creature, to, dist(p, cpos) > 10.0);
            }
            true
        }
    }
}
