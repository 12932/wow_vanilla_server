//! A creature in a map: built from its spawn and template, and moving as
//! its spawn says (standing, wandering near home, or walking its
//! waypoints). Only a creature some player sees moves.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashSet;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    SMSG_MONSTER_MOVE, SMSG_MONSTER_MOVE_MonsterMoveType, SplineFlag, Vector3d,
};

use crate::game::conn::encode;
use crate::game::data::{CreatureSpawn, CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::grid::{CellKey, cell_of};
use crate::game::update::{Create, Placement, Speeds, SplineState};

/// Spawn movement types.
pub const MOVEMENT_IDLE: u8 = 0;
pub const MOVEMENT_RANDOM: u8 = 1;
pub const MOVEMENT_WAYPOINT: u8 = 2;

/// How long a wandering creature pauses between walks.
const WANDER_PAUSE_MS: (u64, u64) = (4_000, 10_000);

/// Spline flags: falling, run mode, flying (a Catmull-Rom spline).
pub const SPLINE_FALLING: u32 = 0x2;
pub const SPLINE_RUN_MODE: u32 = 0x100;

/// `UNIT_FIELD_BYTES_2`'s misc flags: a creature's auras shown as they are.
pub const UNIT_BYTE2_FLAG_AURAS: u8 = 0x10;
pub const SPLINE_FLYING: u32 = 0x200;

/// Where a creature is going.
#[derive(Debug, Clone)]
pub enum Motion {
    /// Standing until then (never, for `None`).
    Idle { until: Option<Instant> },
    /// Walking a path.
    Moving(Walk),
}

#[derive(Debug, Clone)]
pub struct Walk {
    pub points: Vec<Vector3d>,
    /// The distance from the start to each point.
    lengths: Vec<f32>,
    pub started: Instant,
    pub duration: Duration,
    pub speed: f32,
    pub spline_id: u32,
    pub run: bool,
    /// Through the air, not along the ground.
    pub fly: bool,
    /// Falling (a flyer's corpse).
    pub falling: bool,
    /// The facing it takes on arriving, the packet says.
    pub face: Option<f32>,
    /// The unit it turns to face as it goes (a chase's victim).
    pub face_target: Option<Guid>,
}

impl Walk {
    pub fn new(
        points: Vec<Vector3d>,
        speed: f32,
        started: Instant,
        spline_id: u32,
        run: bool,
    ) -> Self {
        let mut lengths = Vec::with_capacity(points.len());
        let mut total = 0.0;
        lengths.push(0.0);
        for w in points.windows(2) {
            total += dist3(w[0], w[1]);
            lengths.push(total);
        }
        let speed = speed.max(0.1);
        Self {
            // A walk of no finite length (to a place out of the world)
            // takes no time, rather than panic.
            duration: Duration::try_from_secs_f32(total / speed).unwrap_or(Duration::ZERO),
            points,
            lengths,
            started,
            speed,
            spline_id,
            run,
            fly: false,
            falling: false,
            face: None,
            face_target: None,
        }
    }

    /// The points it has passed by `now`, where it set off from first.
    pub fn passed(&self, now: Instant) -> impl Iterator<Item = Vector3d> + '_ {
        let travelled = now.duration_since(self.started).as_secs_f32() * self.speed;
        self.points
            .iter()
            .zip(&self.lengths)
            .filter(move |(_, l)| **l <= travelled)
            .map(|(p, _)| *p)
    }

    /// The distance from the start to each point.
    pub fn lengths(&self) -> &[f32] {
        &self.lengths
    }

    pub fn destination(&self) -> Vector3d {
        *self.points.last().expect("a walk has points")
    }

    /// The points still ahead at `now`.
    pub fn remaining(&self, now: Instant) -> Vec<Vector3d> {
        let travelled = now.duration_since(self.started).as_secs_f32() * self.speed;
        self.points
            .iter()
            .zip(&self.lengths)
            .filter(|(_, l)| **l > travelled)
            .map(|(p, _)| *p)
            .collect()
    }

    pub fn done(&self, now: Instant) -> bool {
        now.duration_since(self.started) >= self.duration
    }

    /// Where along the walk the creature is at `now`, and which way it
    /// faces.
    pub fn position_at(&self, now: Instant) -> (Vector3d, f32) {
        // A fall drops under gravity, straight down to where it lands
        // (mangos's falling spline, its `computeFallElevation`).
        if self.falling {
            let (from, to) = (self.points[0], self.destination());
            let t = now.duration_since(self.started).as_secs_f32();
            let z = from.z - crate::game::map::creature_motion::fall_distance(t);
            return (
                Vector3d {
                    z: z.max(to.z),
                    ..to
                },
                0.0,
            );
        }
        let travelled = now.duration_since(self.started).as_secs_f32() * self.speed;
        for i in 1..self.points.len() {
            if travelled <= self.lengths[i] {
                let (a, b) = (self.points[i - 1], self.points[i]);
                let seg = (self.lengths[i] - self.lengths[i - 1]).max(f32::EPSILON);
                let t = ((travelled - self.lengths[i - 1]) / seg).clamp(0.0, 1.0);
                let p = Vector3d {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                    z: a.z + (b.z - a.z) * t,
                };
                return (p, (b.y - a.y).atan2(b.x - a.x));
            }
        }
        let n = self.points.len();
        let (a, b) = (self.points[n.saturating_sub(2)], self.points[n - 1]);
        (b, (b.y - a.y).atan2(b.x - a.x))
    }

    /// Run mode (0x100) and flying (0x200); a fall's only falling (0x2).
    pub fn spline_flags(&self) -> u32 {
        if self.falling {
            return SPLINE_FALLING;
        }
        (if self.run { SPLINE_RUN_MODE } else { 0 }) | (if self.fly { SPLINE_FLYING } else { 0 })
    }

    /// The flags `SMSG_MONSTER_MOVE` carries: its own, and run mode always,
    /// as mangos-classic's `PacketBuilder` and Thistle Tea send them (the
    /// client misbehaves without it; it walks or runs by the duration).
    pub fn monster_move_flags(&self) -> u32 {
        self.spline_flags() | SPLINE_RUN_MODE
    }

    /// The walk's `SMSG_MONSTER_MOVE`, as mangos-classic writes it for the
    /// 1.12 client: a flyer's Catmull-Rom spline in whole points; else
    /// [`Self::packet`].
    pub fn frame(&self, guid: Guid, from: Vector3d) -> Option<Arc<[u8]>> {
        if self.monster_move_flags() & SPLINE_FLYING == 0 {
            return encode(&self.packet(guid, from));
        }
        // wow_messages packs every point after the first, so this one is
        // written by hand (`WriteCatmullRomPath`): each point after the
        // start, whole.
        let mut b = crate::game::data::items::Body::default();
        crate::game::update::write_packed_guid(&mut b.0, guid);
        b.f32(from.x).f32(from.y).f32(from.z).u32(self.spline_id);
        match (self.face_target, self.face) {
            (Some(target), _) => b.u8(3).u64(target.guid()),
            (None, Some(angle)) => b.u8(4).f32(angle),
            (None, None) => b.u8(0),
        };
        b.u32(self.monster_move_flags())
            .u32(self.duration.as_millis() as u32);
        let points = &self.points[1..];
        b.u32(points.len() as u32);
        for p in points {
            b.f32(p.x).f32(p.y).f32(p.z);
        }
        Some(b.frame(crate::game::map::ai::SMSG_MONSTER_MOVE))
    }

    /// A walk on the ground as `SMSG_MONSTER_MOVE` puts it
    /// (`WriteLinearPath`): the destination whole, then the points between
    /// as packed offsets from it; one too near it to pack (it would
    /// freeze the client), or too far, is left out.
    pub fn packet(&self, guid: Guid, from: Vector3d) -> SMSG_MONSTER_MOVE {
        let dest = self.destination();
        let mut splines = vec![dest];
        for p in &self.points[1..self.points.len() - 1] {
            let offset = Vector3d {
                x: dest.x - p.x,
                y: dest.y - p.y,
                z: dest.z - p.z,
            };
            let near = offset.x * offset.x + offset.y * offset.y + offset.z * offset.z < 0.5;
            // 11, 11 and 10 bits of quarter yards.
            let packs = offset.x.abs() < 255.0 && offset.y.abs() < 255.0 && offset.z.abs() < 127.0;
            if !near && packs {
                splines.push(offset);
            }
        }
        SMSG_MONSTER_MOVE {
            guid,
            spline_point: from,
            spline_id: self.spline_id,
            move_type: {
                let (spline_flags, duration) = (
                    SplineFlag::new(self.monster_move_flags()),
                    self.duration.as_millis() as u32,
                );
                match (self.face_target, self.face) {
                    (Some(target), _) => SMSG_MONSTER_MOVE_MonsterMoveType::FacingTarget {
                        duration,
                        spline_flags,
                        splines,
                        target,
                    },
                    (None, Some(angle)) => SMSG_MONSTER_MOVE_MonsterMoveType::FacingAngle {
                        angle,
                        duration,
                        spline_flags,
                        splines,
                    },
                    (None, None) => SMSG_MONSTER_MOVE_MonsterMoveType::Normal {
                        duration,
                        spline_flags,
                        splines,
                    },
                }
            },
        }
    }
}

fn dist3(a: Vector3d, b: Vector3d) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

#[derive(Debug)]
pub struct Creature {
    pub guid: Guid,
    pub entry: u32,
    /// The spawn it came from; `None` for one a GM spawned.
    pub spawn_guid: Option<u32>,
    pub fields: Fields,
    pub position: Vector3d,
    pub orientation: f32,
    pub home: Vector3d,
    pub home_orientation: f32,
    pub speeds: Speeds,
    pub move_flags: u32,
    pub movement_type: u8,
    pub wander_distance: f32,
    /// The waypoint it is bound for (an index into its path; past the end
    /// once a path that doesn't go round is done).
    pub waypoint: usize,
    /// The last waypoint it reached: where it goes back to after a fight.
    pub last_waypoint: Option<usize>,
    pub motion: Motion,
    /// Players whose clients know this creature.
    pub observers: AHashSet<Guid>,
    /// Awake though no one sees it and it isn't fighting, so that it moves
    /// (`map::awake`): Rexxar and his company running into a city from out
    /// of sight (`champions.rs`). Set through `Map::keep_awake`.
    pub keep_awake: bool,
    pub vis_anchor: Vector3d,
    pub cell: CellKey,
    pub life: Life,
    /// Dead and gone from view, waiting to respawn.
    pub despawned: bool,
    pub combat: crate::game::map::combat::Combat,
    pub combat_start: Option<Instant>,
    pub auras: crate::game::map::auras::Auras,
    pub caster: crate::game::map::spells::Caster,
    /// Threat, by unit.
    pub threat: Vec<(Guid, f32)>,
    /// The player whose kill this is.
    pub tapped_by: Option<Guid>,
    /// The group the tapper was in as they tapped it, whose kill it is
    /// while that group stands (`map::tap::recipients`).
    pub tap_group: Option<u32>,
    /// The tap as its observers were last told it (`map::tap`).
    pub tap_told: Option<Guid>,
    pub evading: bool,
    /// Where the victim was when the chase last pathed.
    pub chase_target_at: Option<Vector3d>,
    pub aggro_check_at: Instant,
    /// Its faction is hostile to players: it looks for them.
    pub hostile_to_players: bool,
    pub loot: Option<crate::game::map::loot::Loot>,
    /// Its values before auras.
    pub base: crate::game::map::derive::UnitBase,
    pub derived: crate::game::map::derive::Derived,
    /// The unit that summoned or owns it: a pet's master, a totem's shaman.
    pub owner: Option<Guid>,
    /// What it is as a pet, guardian, totem or charmed unit.
    pub pet: Option<Box<crate::game::map::pets::PetState>>,
    /// What scripts keep on it: EventAI's state, its phase, a summoner.
    pub scripted: Option<Box<crate::game::map::event_ai::Scripted>>,
    /// The event whose looks it wears; 0 for its own.
    pub event_looks: u32,
    /// When something summoned it, for a summons that lasts a while.
    pub summoned_at: Option<Instant>,
    /// Its fight's leash, reach, flight and regeneration.
    pub engagement: crate::game::map::creature_combat::Engagement,
    /// What its wounds leave of its run speed.
    pub wounded: f32,
    /// The zone and area it stands in, once worked out: as it comes into
    /// view and as it moves.
    pub zone_area: Option<(u32, u32)>,
    /// How it rests, as it came into the world.
    pub rest: Rest,
}

/// A creature's resting posture as it came into the world (its spawn's
/// addon, Thistle Tea's `Posture`): a fight stands it up and stops its
/// emote, and it takes them back home.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rest {
    pub stand: u8,
    pub sheath: u8,
    pub emote: u32,
    pub mount: u32,
}

impl Rest {
    pub fn of(f: &Fields) -> Self {
        Rest {
            stand: f.get_byte(UNIT_FIELD_BYTES_1, 0),
            sheath: f.get_byte(UNIT_FIELD_BYTES_2, 0),
            emote: f.get_u32(UNIT_NPC_EMOTESTATE),
            mount: f.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
        }
    }
}

/// Alive, or dead and waiting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Life {
    Alive,
    Dead {
        corpse_until: Instant,
        respawn_at: Instant,
    },
}

impl Creature {
    pub fn alive(&self) -> bool {
        self.life == Life::Alive
    }

    pub fn new(
        guid: Guid,
        template: &CreatureTemplate,
        level: u8,
        position: Vector3d,
        orientation: f32,
    ) -> Self {
        let mut fields = Fields::new(TypeId::Unit, guid, template.entry);
        let (display, scale) = pick_display(template);
        fields.set_f32(OBJECT_FIELD_SCALE_X, scale);
        let health = template.health_max.max(1);
        fields.set_u32(UNIT_FIELD_HEALTH, health);
        fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
        fields.set_u32(UNIT_FIELD_LEVEL, u32::from(level.max(1)));
        fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, template.faction);
        fields.set_u32(UNIT_FIELD_FLAGS, template.unit_flags);
        fields.set_u32(UNIT_DYNAMIC_FLAGS, template.dynamic_flags);
        fields.set_u32(UNIT_NPC_FLAGS, template.npc_flags);
        fields.set_bytes(UNIT_FIELD_BYTES_0, 0, template.unit_class, 0, 0);
        fields.set_u32(UNIT_FIELD_DISPLAYID, display);
        fields.set_u32(UNIT_FIELD_NATIVEDISPLAYID, display);
        fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, template.mount_display_id);
        // Weapons out, unless its addon says otherwise (mangos's
        // `InitEntry`, Thistle Tea's sheath of 1).
        fields.set_byte(UNIT_FIELD_BYTES_2, 0, 1);
        // Its buffs shown as buffs, and dispellable as such (mangos's
        // `UNIT_BYTE2_FLAG_AURAS`, Thistle Tea's misc flags 0x10).
        fields.set_byte(UNIT_FIELD_BYTES_2, 1, UNIT_BYTE2_FLAG_AURAS);
        let attack = if template.base_attack_time > 0 {
            template.base_attack_time
        } else {
            2000
        };
        fields.set_u32(UNIT_FIELD_BASEATTACKTIME, attack);
        fields.set_u32(UNIT_FIELD_BASEATTACKTIME + 1, attack);
        fields.set_u32(UNIT_FIELD_RANGEDATTACKTIME, template.ranged_attack_time);
        fields.set_f32(UNIT_FIELD_BOUNDINGRADIUS, 0.5 * scale);
        fields.set_f32(UNIT_FIELD_COMBATREACH, 1.5 * scale);
        fields.set_f32(UNIT_MOD_CAST_SPEED, 1.0);
        let mut speeds = Speeds::default();
        if template.speed_walk > 0.0 {
            speeds.walk = 2.5 * template.speed_walk;
        }
        if template.speed_run > 0.0 {
            speeds.run = 7.0 * template.speed_run;
        }
        Self {
            guid,
            entry: template.entry,
            spawn_guid: None,
            fields,
            position,
            orientation,
            home: position,
            home_orientation: orientation,
            speeds,
            move_flags: 0,
            movement_type: MOVEMENT_IDLE,
            wander_distance: 0.0,
            waypoint: 0,
            last_waypoint: None,
            motion: Motion::Idle { until: None },
            observers: AHashSet::new(),
            keep_awake: false,
            vis_anchor: position,
            cell: cell_of(position.x, position.y),
            life: Life::Alive,
            despawned: false,
            combat: Default::default(),
            auras: Default::default(),
            caster: Default::default(),
            combat_start: None,
            threat: Vec::new(),
            tapped_by: None,
            tap_group: None,
            tap_told: None,
            evading: false,
            chase_target_at: None,
            aggro_check_at: Instant::now(),
            hostile_to_players: false,
            loot: None,
            base: Default::default(),
            derived: Default::default(),
            owner: None,
            pet: None,
            scripted: None,
            event_looks: 0,
            summoned_at: None,
            engagement: Default::default(),
            wounded: 1.0,
            zone_area: None,
            rest: Rest::default(),
        }
        .with_base()
    }

    /// Its values as they now are become its base, and its posture its
    /// rest.
    pub fn with_base(mut self) -> Self {
        self.base = crate::game::map::derive::UnitBase::capture(&self.fields, self.speeds.run);
        self.rest = Rest::of(&self.fields);
        crate::game::map::derive::creature(&mut self);
        self
    }

    /// Its size, reach and bounding radius for the model it shows, as
    /// Thistle Tea's mob builder sets them. The size is the template's for
    /// that display (`template`, where the display is the template's own),
    /// else the display's own: CreatureDisplayInfo's scale by its model's.
    /// The model's reach and radius are measured at the display's own
    /// size, and grow and shrink from it (Onyxia's reach is 26 yards, not
    /// 47 by her size twice over).
    pub fn fit_model(&mut self, data: &GameData, template: Option<&CreatureTemplate>) {
        let display = self.fields.get_u32(UNIT_FIELD_DISPLAYID);
        let Some(d) = data.display_info.get(&display) else {
            return;
        };
        // Its gender, the model's (mangos's `InitEntry` from the model's
        // info, Thistle Tea's from the display's addon).
        if let Some(gender) = d.gender {
            self.fields.set_byte(UNIT_FIELD_BYTES_0, 2, gender);
        }
        let native = if d.scale > 0.0 { d.scale } else { 1.0 };
        let scale = template
            .and_then(|t| {
                let i = t.display_ids.iter().position(|x| *x == display)?;
                Some(t.display_scales[i])
            })
            .filter(|s| *s > 0.0)
            .unwrap_or(native);
        self.fields.set_f32(OBJECT_FIELD_SCALE_X, scale);
        let grow = scale / native;
        let (radius, reach) = (
            if d.bounding_radius > 0.0 {
                d.bounding_radius
            } else {
                0.5
            },
            if d.combat_reach > 0.0 {
                d.combat_reach
            } else {
                1.5
            },
        );
        self.fields
            .set_f32(UNIT_FIELD_BOUNDINGRADIUS, radius * grow);
        self.fields.set_f32(UNIT_FIELD_COMBATREACH, reach * grow);
    }

    /// A creature for a spawn: its entry picked among the spawn's, its
    /// level in the template's range, and the spawn's look and gear.
    pub fn from_spawn(
        data: &GameData,
        spawn: &CreatureSpawn,
        rng: &mut impl rand::Rng,
    ) -> Option<Self> {
        let entry = spawn.entries[rng.random_range(0..spawn.entries.len())];
        let template = data.creature_template(entry)?;
        let level = if template.level_max > template.level_min {
            rng.random_range(template.level_min..=template.level_max)
        } else {
            template.level_min
        }
        .max(1);
        let position = Vector3d {
            x: spawn.x,
            y: spawn.y,
            z: spawn.z,
        };
        let mut c = Self::new(
            guid::unit(entry, spawn.guid),
            template,
            level,
            position,
            spawn.orientation,
        );
        c.spawn_guid = Some(spawn.guid);
        // Out, but unseen until a script shows it.
        if spawn.spawn_flags & crate::game::data::SPAWN_FLAG_NOT_VISIBLE != 0 {
            c.scripted.get_or_insert_with(Default::default).concealed = true;
        }
        if let Some(stats) = data.class_level_stats.get(&(template.unit_class, level)) {
            let health =
                crate::game::data::load::creature_health(stats.health, template.health_multiplier);
            let health_now = if spawn.health_percent > 0.0 && spawn.health_percent < 100.0 {
                ((health as f32 * spawn.health_percent / 100.0) as u32).max(1)
            } else {
                health
            };
            c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
            c.fields.set_u32(UNIT_FIELD_HEALTH, health_now);
            let mana = (stats.mana as f32 * template.mana_multiplier.max(0.0)).round() as u32;
            if mana > 0 {
                c.fields.set_u32(UNIT_FIELD_MAXPOWER1, mana);
                c.fields.set_u32(UNIT_FIELD_POWER1, mana);
            }
            // vmangos's creature damage: the class's at the level, by the
            // template's multiplier, spread by its variance.
            let mult = if template.damage_multiplier > 0.0 {
                template.damage_multiplier
            } else {
                1.0
            };
            let variance = if template.damage_variance > 0.0 {
                template.damage_variance
            } else {
                0.14
            };
            let base = stats.melee_damage * mult;
            c.fields
                .set_f32(UNIT_FIELD_MINDAMAGE, (base * (1.0 - variance)).max(1.0));
            c.fields
                .set_f32(UNIT_FIELD_MAXDAMAGE, (base * (1.0 + variance)).max(1.0));
            c.fields
                .set_u32(UNIT_FIELD_ATTACK_POWER, stats.attack_power);
            let armor_mult = if template.armor_multiplier > 0.0 {
                template.armor_multiplier
            } else {
                1.0
            };
            c.fields.set_u32(
                UNIT_FIELD_RESISTANCES,
                (stats.armor as f32 * armor_mult) as u32,
            );
        }
        c.hostile_to_players =
            crate::game::map::ai::hostile_to_players(data, template.faction, template.civilian);
        let mut equipment = template.equipment_id;
        let mut own_display = true;
        if let Some(addon) = data.creature_addons.get(&spawn.guid) {
            if addon.display_id != 0 {
                c.fields.set_u32(UNIT_FIELD_DISPLAYID, addon.display_id);
                c.fields
                    .set_u32(UNIT_FIELD_NATIVEDISPLAYID, addon.display_id);
                own_display = false;
            }
            // Its own mount, none, or (-1) its template's.
            if addon.mount_display_id >= 0 {
                c.fields
                    .set_u32(UNIT_FIELD_MOUNTDISPLAYID, addon.mount_display_id as u32);
            }
            // Its own weapons, none, or (-1) its template's.
            if addon.equipment_id >= 0 {
                equipment = addon.equipment_id as u32;
            }
            c.fields.set_byte(UNIT_FIELD_BYTES_1, 0, addon.stand_state);
            c.fields.set_byte(UNIT_FIELD_BYTES_2, 0, addon.sheath_state);
            c.fields.set_u32(UNIT_NPC_EMOTESTATE, addon.emote_state);
        }
        if equipment != 0
            && let Some(items) = data.creature_equipment.get(&equipment)
        {
            c.set_equipment(data, *items);
        }
        c.fit_model(data, own_display.then_some(template));
        c.movement_type = spawn.movement_type;
        c.wander_distance = spawn.wander_distance;
        // A flyer is in the air from the start.
        if crate::game::map::creature_motion::template_flies(template) {
            c.move_flags |= crate::game::map::creature_motion::FLIGHT_FLAGS;
        }
        // A spawn with a path of its own walks it, whatever its movement
        // type says (bar going round in the air), from the point nearest
        // its spawn, as Thistle Tea builds its route. One set to walk a
        // path with none of its own walks its entry's
        // (`creature_movement_template`, mangos's `GetDefaultPath`).
        match data.waypoints.get(&spawn.guid).filter(|p| !p.is_empty()) {
            Some(path) if c.movement_type != crate::game::map::creature_motion::MOVEMENT_CYCLIC => {
                c.movement_type = MOVEMENT_WAYPOINT;
                c.waypoint = nearest_waypoint(path, position);
            }
            None if c.movement_type == MOVEMENT_WAYPOINT => {
                match data
                    .waypoints_by_entry
                    .get(&c.entry)
                    .filter(|p| !p.is_empty())
                {
                    Some(path) => c.waypoint = nearest_waypoint(path, position),
                    None => c.movement_type = MOVEMENT_IDLE,
                }
            }
            _ => {}
        }
        if c.movement_type == MOVEMENT_RANDOM && c.wander_distance <= 0.0 {
            c.movement_type = MOVEMENT_IDLE;
        }
        if c.movement_type != MOVEMENT_IDLE {
            c.motion = Motion::Idle {
                until: Some(Instant::now()),
            };
        }
        for (i, r) in template.resistances.iter().enumerate() {
            c.fields
                .set_u32(UNIT_FIELD_RESISTANCES + 1 + i as u16, (*r).max(0) as u32);
        }
        Some(c.with_base())
    }

    /// The weapons a creature holds.
    pub fn set_equipment(&mut self, data: &GameData, items: [u32; 3]) {
        for (slot, entry) in items.iter().enumerate() {
            if data.item(*entry).is_some() {
                self.put_weapon(data, slot as u16, *entry);
            }
        }
    }

    /// The weapons a script hands it, slot by slot: `None` leaves a slot
    /// as it is; an entry no item has (0 among them) empties it, as
    /// Thistle Tea's `ScriptEquipment` has it.
    pub fn set_equipment_slots(&mut self, data: &GameData, slots: [Option<u32>; 3]) {
        for (slot, entry) in slots.iter().enumerate() {
            if let Some(entry) = *entry {
                self.put_weapon(data, slot as u16, entry);
            }
        }
    }

    /// Item `entry` in weapon slot `slot`; none if no item has it.
    fn put_weapon(&mut self, data: &GameData, slot: u16, entry: u32) {
        let Some(t) = data.item(entry) else {
            self.fields
                .set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + slot, 0);
            self.fields.set_u32(UNIT_VIRTUAL_ITEM_INFO + slot * 2, 0);
            self.fields
                .set_u32(UNIT_VIRTUAL_ITEM_INFO + slot * 2 + 1, 0);
            return;
        };
        self.fields
            .set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + slot, t.display_id);
        self.fields.set_bytes(
            UNIT_VIRTUAL_ITEM_INFO + slot * 2,
            t.class as u8,
            t.subclass as u8,
            t.material as u8,
            t.inventory_type as u8,
        );
        self.fields.set_bytes(
            UNIT_VIRTUAL_ITEM_INFO + slot * 2 + 1,
            t.sheath as u8,
            0,
            0,
            0,
        );
    }

    pub fn create(&self, now: Instant) -> Create<'_> {
        let (position, orientation, spline) = match &self.motion {
            Motion::Moving(w) if !w.done(now) => {
                let (p, o) = w.position_at(now);
                // A fall keeps the way it faced.
                let o = if w.falling { self.orientation } else { o };
                let elapsed = now.duration_since(w.started).as_millis() as u32;
                (
                    p,
                    o,
                    Some(SplineState {
                        flags: w.spline_flags(),
                        time_passed: elapsed,
                        duration: w.duration.as_millis() as u32,
                        id: w.spline_id,
                        nodes: w.points.clone(),
                        final_node: w.destination(),
                    }),
                )
            }
            _ => (self.position, self.orientation, None),
        };
        Create {
            guid: self.guid,
            type_id: TypeId::Unit,
            placement: Placement::Living {
                flags: self.move_flags,
                timestamp: 0,
                position,
                orientation,
                pitch: 0.0,
                fall_time: 0,
                speeds: self.speeds,
                spline,
                transport: None,
            },
            fields: &self.fields,
            is_self: false,
            melee_target: None,
        }
    }

    /// Start walking `points` (the first where it is now), telling
    /// `send` the packet for observers.
    pub fn walk(
        &mut self,
        points: Vec<Vector3d>,
        run: bool,
        now: Instant,
        spline_id: u32,
    ) -> Option<Arc<[u8]>> {
        self.walk_at(points, run, false, None, now, spline_id)
    }

    /// [`Self::walk`], flying or not, and taking `time` if it is given.
    pub fn walk_at(
        &mut self,
        points: Vec<Vector3d>,
        run: bool,
        fly: bool,
        time: Option<Duration>,
        now: Instant,
        spline_id: u32,
    ) -> Option<Arc<[u8]>> {
        if points.len() < 2 {
            return None;
        }
        let mut speed = if run {
            self.speeds.run
        } else {
            self.speeds.walk
        };
        if let Some(t) = time.filter(|t| !t.is_zero()) {
            let length: f32 = points.windows(2).map(|w| dist3(w[0], w[1])).sum();
            speed = length / t.as_secs_f32();
        }
        let mut walk = Walk::new(points, speed, now, spline_id, run);
        walk.fly = fly;
        let frame = walk.frame(self.guid, self.position);
        self.motion = Motion::Moving(walk);
        frame
    }

    /// Its run going on at its new run speed, from where it is, if it was
    /// running at `was` (a timed move keeps its time): the packet.
    pub fn retime(&mut self, was: f32, now: Instant, spline_id: u32) -> Option<Arc<[u8]>> {
        let Motion::Moving(w) = &self.motion else {
            return None;
        };
        if !w.run || w.falling || w.done(now) || (w.speed - was).abs() > 0.01 {
            return None;
        }
        let (at, _) = w.position_at(now);
        let mut points = vec![at];
        points.extend(w.remaining(now));
        if points.len() < 2 {
            return None;
        }
        let mut walk = Walk::new(points, self.speeds.run, now, spline_id, true);
        walk.fly = w.fly;
        walk.face = w.face;
        walk.face_target = w.face_target;
        // The run is still what it was: a script's move, a flight for help.
        let old = w.spline_id;
        if let Some(s) = self.scripted.as_deref_mut()
            && let Some(m) = s.scripted_move.as_mut()
            && m.spline_id == old
        {
            m.spline_id = spline_id;
        }
        if self.engagement.control_spline == Some(old) {
            self.engagement.control_spline = Some(spline_id);
        }
        self.position = at;
        let frame = encode(&walk.packet(self.guid, at));
        self.motion = Motion::Moving(walk);
        frame
    }

    /// A move in a fight: [`Self::walk_at`], facing `victim` as it goes
    /// and at the end, as Thistle Tea's `Navigation.chase` and mangos's
    /// chase (`SetFacing`) send it: a chase at a run, a step about its
    /// victim at a walk.
    pub fn walk_facing(
        &mut self,
        points: Vec<Vector3d>,
        run: bool,
        fly: bool,
        victim: Guid,
        now: Instant,
        spline_id: u32,
    ) -> Option<Arc<[u8]>> {
        if points.len() < 2 {
            return None;
        }
        let speed = if run {
            self.speeds.run
        } else {
            self.speeds.walk
        };
        let mut walk = Walk::new(points, speed, now, spline_id, run);
        walk.fly = fly;
        walk.face_target = Some(victim);
        let frame = encode(&walk.packet(self.guid, self.position));
        self.motion = Motion::Moving(walk);
        frame
    }
}

/// A template's look: a display picked by the probabilities, and its scale.
fn pick_display(t: &CreatureTemplate) -> (u32, f32) {
    let total: u32 = t
        .display_ids
        .iter()
        .zip(t.display_probabilities)
        .filter(|(d, _)| **d != 0)
        .map(|(_, p)| p)
        .sum();
    let mut chosen = None;
    if total > 0 {
        let mut roll = rand::rng().random_range(0..total);
        for (i, (d, p)) in t
            .display_ids
            .iter()
            .zip(t.display_probabilities)
            .enumerate()
        {
            if *d == 0 {
                continue;
            }
            if roll < p {
                chosen = Some(i);
                break;
            }
            roll -= p;
        }
    }
    let i = chosen
        .or_else(|| t.display_ids.iter().position(|d| *d != 0))
        .unwrap_or(0);
    let scale = t.display_scales[i];
    (t.display_ids[i], if scale > 0.0 { scale } else { 1.0 })
}

/// The index of the point of `path` nearest `at`.
pub fn nearest_waypoint(path: &[crate::game::data::Waypoint], at: Vector3d) -> usize {
    let d = |w: &crate::game::data::Waypoint| {
        (w.x - at.x).powi(2) + (w.y - at.y).powi(2) + (w.z - at.z).powi(2)
    };
    (0..path.len())
        .min_by(|a, b| d(&path[*a]).total_cmp(&d(&path[*b])))
        .unwrap_or(0)
}

/// How long a wandering creature waits before its next walk.
pub fn wander_pause(rng: &mut impl rand::Rng) -> Duration {
    Duration::from_millis(rng.random_range(WANDER_PAUSE_MS.0..WANDER_PAUSE_MS.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A creature is the size its template gives its display, else the
    /// display's own, and its reach grows from the display's own: Onyxia
    /// (2.0 of a 1.8 display, 23.4 yards at 1.8) reaches 26 yards, not 47;
    /// Hogger and a Kobold Miner are their displays' 1.15.
    #[test]
    #[ignore = "extended"]
    fn a_creature_is_its_displays_size_and_reaches_from_it() {
        let Some(data) = crate::game::data::load::real_data() else {
            return;
        };
        let fitted = |entry: u32| {
            let spawn = crate::game::data::CreatureSpawn {
                guid: 0xF0_0000,
                entries: vec![entry],
                ..Default::default()
            };
            let c = Creature::from_spawn(data, &spawn, &mut rand::rng()).unwrap();
            (
                c.fields.get_f32(OBJECT_FIELD_SCALE_X),
                c.fields.get_f32(UNIT_FIELD_COMBATREACH),
            )
        };
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        let (scale, reach) = fitted(10184);
        assert!(
            near(scale, 2.0) && near(reach, 26.0),
            "Onyxia {scale} {reach}"
        );
        let (scale, reach) = fitted(448);
        assert!(
            near(scale, 1.15) && near(reach, 1.725),
            "Hogger {scale} {reach}"
        );
        let (scale, reach) = fitted(40);
        assert!(
            near(scale, 1.15) && near(reach, 1.4375),
            "Kobold Miner {scale} {reach}"
        );
    }
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;

    #[test]
    fn a_walk_is_where_it_should_be_and_its_packet_parses() {
        let now = Instant::now();
        let points = vec![
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Vector3d {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            Vector3d {
                x: 10.0,
                y: 10.0,
                z: 0.0,
            },
        ];
        let w = Walk::new(points, 2.5, now, 1, false);
        assert_eq!(w.duration.as_millis(), 8000);
        let (p, _) = w.position_at(now + Duration::from_secs(2));
        assert!((p.x - 5.0).abs() < 0.01 && p.y.abs() < 0.01);
        let (p, _) = w.position_at(now + Duration::from_secs(6));
        assert!((p.x - 10.0).abs() < 0.01 && (p.y - 5.0).abs() < 0.01);
        let frame = encode(&w.packet(
            guid::unit(1, 1),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        ))
        .unwrap();
        let ServerOpcodeMessage::SMSG_MONSTER_MOVE(m) =
            ServerOpcodeMessage::read_unencrypted(&frame[..]).unwrap()
        else {
            panic!("not a monster move");
        };
        let SMSG_MONSTER_MOVE_MonsterMoveType::Normal {
            duration, splines, ..
        } = m.move_type
        else {
            panic!("not a walk");
        };
        assert_eq!(splines[0].x, 10.0);
        assert_eq!(duration, 8000);
    }

    #[test]
    fn a_moving_creatures_create_carries_its_spline_and_parses() {
        let t = CreatureTemplate {
            entry: 5,
            display_ids: [903, 0, 0, 0],
            health_max: 10,
            ..Default::default()
        };
        let now = Instant::now();
        let mut c = Creature::new(
            guid::unit(5, 1),
            &t,
            1,
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        );
        c.walk(
            vec![
                Vector3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Vector3d {
                    x: 5.0,
                    y: 0.0,
                    z: 0.0,
                },
                Vector3d {
                    x: 5.0,
                    y: 5.0,
                    z: 0.0,
                },
            ],
            false,
            now,
            3,
        );
        let block = crate::game::update::create_block(
            &c.create(now + Duration::from_secs(1)),
            crate::game::fields::Visibility::Public,
        );
        let mut b = crate::game::update::Batch::new();
        b.push(&block);
        let frame = &b.finish()[0];
        let m = ServerOpcodeMessage::read_unencrypted(&frame[..]);
        assert!(m.is_ok(), "{m:?}");
    }
}
