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
            duration: Duration::from_secs_f32(total / speed),
            points,
            lengths,
            started,
            speed,
            spline_id,
            run,
            fly: false,
        }
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

    /// Run mode (0x100) and flying (0x200).
    pub fn spline_flags(&self) -> u32 {
        (if self.run { 0x100 } else { 0 }) | (if self.fly { 0x200 } else { 0 })
    }

    /// The walk as `SMSG_MONSTER_MOVE` puts it: the destination, then the
    /// points between as offsets from the path's middle.
    pub fn packet(&self, guid: Guid, from: Vector3d) -> SMSG_MONSTER_MOVE {
        let dest = self.destination();
        let mut splines = vec![dest];
        let between = &self.points[1..self.points.len() - 1];
        if !between.is_empty() {
            let middle = Vector3d {
                x: (self.points[0].x + dest.x) / 2.0,
                y: (self.points[0].y + dest.y) / 2.0,
                z: (self.points[0].z + dest.z) / 2.0,
            };
            for p in between {
                splines.push(Vector3d {
                    x: middle.x - p.x,
                    y: middle.y - p.y,
                    z: middle.z - p.z,
                });
            }
        }
        SMSG_MONSTER_MOVE {
            guid,
            spline_point: from,
            spline_id: self.spline_id,
            move_type: SMSG_MONSTER_MOVE_MonsterMoveType::Normal,
            spline_flags: SplineFlag::new(self.spline_flags()),
            duration: self.duration.as_millis() as u32,
            splines,
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
    pub waypoint: usize,
    pub motion: Motion,
    /// Players whose clients know this creature.
    pub observers: AHashSet<Guid>,
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
    pub taunted_by: Option<Guid>,
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
            motion: Motion::Idle { until: None },
            observers: AHashSet::new(),
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
            evading: false,
            chase_target_at: None,
            aggro_check_at: Instant::now(),
            hostile_to_players: false,
            loot: None,
            base: Default::default(),
            derived: Default::default(),
            taunted_by: None,
            owner: None,
            pet: None,
            scripted: None,
            event_looks: 0,
            summoned_at: None,
            engagement: Default::default(),
        }
        .with_base()
    }

    /// Its values as they now are become its base.
    pub fn with_base(mut self) -> Self {
        self.base = crate::game::map::derive::UnitBase::capture(&self.fields, self.speeds.run);
        crate::game::map::derive::creature(&mut self);
        self
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
        let display = c.fields.get_u32(UNIT_FIELD_DISPLAYID);
        if let Some(d) = data.display_info.get(&display) {
            let scale = c.fields.get_f32(OBJECT_FIELD_SCALE_X);
            if d.bounding_radius > 0.0 {
                c.fields
                    .set_f32(UNIT_FIELD_BOUNDINGRADIUS, d.bounding_radius * scale);
            }
            if d.combat_reach > 0.0 {
                c.fields
                    .set_f32(UNIT_FIELD_COMBATREACH, d.combat_reach * scale);
            }
        }
        let mut equipment = template.equipment_id;
        if let Some(addon) = data.creature_addons.get(&spawn.guid) {
            if addon.display_id != 0 {
                c.fields.set_u32(UNIT_FIELD_DISPLAYID, addon.display_id);
                c.fields
                    .set_u32(UNIT_FIELD_NATIVEDISPLAYID, addon.display_id);
            }
            if addon.mount_display_id != 0 {
                c.fields
                    .set_u32(UNIT_FIELD_MOUNTDISPLAYID, addon.mount_display_id);
            }
            if addon.equipment_id > 0 {
                equipment = addon.equipment_id as u32;
            } else if addon.equipment_id < 0 {
                equipment = 0;
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
        c.movement_type = spawn.movement_type;
        c.wander_distance = spawn.wander_distance;
        if spawn.movement_type == MOVEMENT_WAYPOINT && !data.waypoints.contains_key(&spawn.guid) {
            c.movement_type = MOVEMENT_IDLE;
        }
        if c.movement_type == MOVEMENT_RANDOM && c.wander_distance <= 0.0 {
            c.movement_type = MOVEMENT_IDLE;
        }
        if c.movement_type != MOVEMENT_IDLE {
            // Spread the first moves out, so creatures don't all start at
            // once.
            c.motion = Motion::Idle {
                until: Some(Instant::now() + Duration::from_millis(rng.random_range(0..8_000))),
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
            let slot = slot as u16;
            let Some(t) = data.item(*entry) else {
                continue;
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
    }

    pub fn create(&self, now: Instant) -> Create<'_> {
        let (position, orientation, spline) = match &self.motion {
            Motion::Moving(w) if !w.done(now) => {
                let (p, o) = w.position_at(now);
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
                fall_time: 0.0,
                speeds: self.speeds,
                spline,
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

/// How long a wandering creature waits before its next walk.
pub fn wander_pause(rng: &mut impl rand::Rng) -> Duration {
    Duration::from_millis(rng.random_range(WANDER_PAUSE_MS.0..WANDER_PAUSE_MS.1))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(m.splines[0].x, 10.0);
        assert_eq!(m.duration, 8000);
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
