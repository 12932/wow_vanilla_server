//! Death: a player who dies can release their spirit to the nearest
//! graveyard as a ghost, leaving a corpse; run back and reclaim it; or ask
//! a spirit healer. A ghost walks on water, and sees only what is near its
//! corpse, its group and the ghosts of its side; the living see ghosts only
//! in their group.

use std::time::{Duration, Instant};

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::conn::{Order, SpeedKind};
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::grid::{CellKey, cell_of};
use crate::game::map::player::{PLAYER_FLAGS_GHOST, Player};
use crate::game::map::{Map, MapKey};
use crate::game::registry::Team;
use crate::game::store::CorpseState;
use crate::game::update::{Create, Placement};

/// The ghost's aura.
pub const SPELL_GHOST: u32 = 8326;
/// How far from their corpse a ghost may reclaim it.
pub const RECLAIM_YARDS: f32 = 39.0;
/// How long after release a ghost must wait to reclaim, after a death
/// alone (`corpses` lengthens it for deaths close together).
pub const RECLAIM_DELAY: Duration = Duration::from_secs(30);
/// How much faster a ghost runs: the ghost aura's 25%, a wisp's 50%.
const GHOST_SPEED: f32 = 1.25;
const WISP_SPEED: f32 = 1.5;
/// The night elf's wisp form.
pub const SPELL_WISP: u32 = 20584;
const RACE_NIGHT_ELF: u8 = 4;
/// Resurrection sickness, from a spirit healer at level 11 and up.
pub const SPELL_RESURRECTION_SICKNESS: u32 = 15007;
/// `UNIT_NPC_FLAGS`: a spirit healer.
pub const NPC_FLAG_SPIRIT_HEALER: u32 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Death {
    Alive,
    /// Dead where they fell, not yet released.
    Dead {
        since: Instant,
    },
    /// Released: a ghost, with a corpse.
    Ghost {
        since: Instant,
    },
}

/// What is left where a player died.
#[derive(Debug)]
pub struct Corpse {
    pub guid: Guid,
    pub owner: Guid,
    pub fields: Fields,
    pub position: Vector3d,
    pub orientation: f32,
    pub observers: AHashSet<Guid>,
    pub cell: CellKey,
}

impl Corpse {
    /// A corpse of `p` at `pos`, facing `orientation`.
    pub fn new(data: &GameData, p: &Player, pos: Vector3d, orientation: f32) -> Self {
        let g = guid::corpse(guid::counter(p.guid));
        let mut f = Fields::new(TypeId::Corpse, g, 0);
        f.set_guid(CORPSE_FIELD_OWNER, p.guid);
        f.set_f32(CORPSE_FIELD_FACING, orientation);
        f.set_f32(CORPSE_FIELD_POS_X, pos.x);
        f.set_f32(CORPSE_FIELD_POS_X + 1, pos.y);
        f.set_f32(CORPSE_FIELD_POS_X + 2, pos.z);
        f.set_u32(
            CORPSE_FIELD_DISPLAY_ID,
            p.fields.get_u32(UNIT_FIELD_NATIVEDISPLAYID),
        );
        // What is worn, as its look and its inventory type over it
        // (Thistle Tea's and mangos's corpse items).
        for slot in 0..19u16 {
            let entry = p
                .fields
                .get_u32(PLAYER_VISIBLE_ITEM_1_0 + slot * VISIBLE_ITEM_SIZE);
            if entry != 0 {
                let display = p.item_display_id(entry);
                let kind = crate::game::map::player::item_display(data, entry).map_or(0, |d| d.1);
                f.set_u32(CORPSE_FIELD_ITEM + slot, display | kind << 24);
            }
        }
        let r = &p.row;
        f.set_bytes(CORPSE_FIELD_BYTES_1, 0, r.race, r.gender, r.skin);
        f.set_bytes(
            CORPSE_FIELD_BYTES_2,
            r.face,
            r.hair_style,
            r.hair_color,
            r.facial_hair,
        );
        // A player's corpse; one fallen in a battleground has insignia to
        // take.
        let insignia = if p.fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::battleground::insignia::UNIT_FLAG_SKINNABLE,
        ) {
            crate::game::map::battleground::insignia::CORPSE_INSIGNIA
        } else {
            0
        };
        f.set_u32(
            CORPSE_FIELD_FLAGS,
            crate::game::map::bones::CORPSE_FLAG_UNK2 | insignia,
        );
        Self {
            guid: g,
            owner: p.guid,
            fields: f,
            position: pos,
            orientation,
            observers: AHashSet::new(),
            cell: cell_of(pos.x, pos.y),
        }
    }

    /// Whether these are bones, not a corpse to rise from.
    pub fn is_bones(&self) -> bool {
        self.fields.get_u32(CORPSE_FIELD_FLAGS) & crate::game::map::bones::CORPSE_FLAG_BONES != 0
    }

    pub fn create(&self) -> Create<'_> {
        Create {
            guid: self.guid,
            type_id: TypeId::Corpse,
            placement: Placement::Position {
                position: self.position,
                orientation: self.orientation,
            },
            fields: &self.fields,
            is_self: false,
            melee_target: None,
        }
    }
}

impl Map {
    pub fn add_corpse(&mut self, c: Corpse) {
        let (g, cell, at) = (c.guid, c.cell, c.position);
        self.grid.insert(g, cell);
        self.corpses.insert(g, c);
        self.visibility_dirty.insert(g);
        self.hold_near(g, at);
    }

    pub fn remove_corpse(&mut self, g: Guid) {
        let Some(mut c) = self.corpses.remove(&g) else {
            return;
        };
        self.grid.remove(g, c.cell);
        for o in c.observers.drain() {
            if let Some(p) = self.players.get_mut(&o) {
                p.known.remove(&g);
                self.forget.entry(o).or_default().push(g);
            }
        }
    }
}

/// A player's health reached 0.
pub fn kill(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.death != Death::Alive {
        return;
    }
    p.death = Death::Dead { since: now };
    // The client counts down to the release it makes itself, out of a copy
    // (mangos's `KillPlayer`: `PLAYER_FIELD_BYTE_RELEASE_TIMER` unless the
    // map is instanceable).
    if crate::game::instances::instance_type(&map.realm, map.key.map) == 0 {
        let flags = p.fields.get_byte(PLAYER_FIELD_BYTES, 0);
        p.fields.set_byte(
            PLAYER_FIELD_BYTES,
            0,
            flags | PLAYER_FIELD_BYTE_RELEASE_TIMER,
        );
    }
    // The dead lie where they fell until they release or rise: rooted,
    // as mangos's `KillPlayer` roots them (`SendMoveRoot`).
    crate::game::map::movement::root_to(&p.outbound, g, true);
    // A run ends where it fell: the body steered no more, and those who
    // see it told it stopped, so their clients don't run it on along the
    // ground (mangos's `SetDeathState` and `StopMoving`, a stop spline to
    // the set; Thistle Tea's death's `MovementStopped`). One the server
    // drove is stopped as its control ends, below.
    let steered = crate::game::map::movement::steered(&p.movement.flags);
    p.movement.flags = crate::game::map::movement::unsteered(p.movement.flags);
    // (A passenger's stop would be on the deck, which this one isn't.)
    let stops = steered && p.driven.is_none() && p.movement.flags.get_on_transport().is_none();
    let at = p.position();
    crate::game::map::corpses::died(p, crate::game::map::corpses::unix_ms());
    p.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    p.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    // The dead dance, sit or kneel no more (Thistle Tea's death clears
    // the emote state and the posture).
    p.fields.set_u32(UNIT_NPC_EMOTESTATE, 0);
    p.set_stand_state(0);
    if stops {
        let spline_id = map.next_spline_id;
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        let frame = crate::game::map::ai::stop_packet(g, at, spline_id);
        map.send_to_observers(g, &frame, false);
    }
    crate::game::map::player_motion::control(map, g);
    // A logout's countdown goes on (mangos's session logs the dead out
    // too); cancelled, it leaves them lying rooted.
    crate::game::map::misc::capture_self_res(map, g);
    crate::game::map::pets::master_died(map, g);
    crate::game::map::combat::leave_combat(map, g);
    // Creatures let go of the dead: their threat on them goes, and they
    // turn to the next or go home (mangos's `deleteReferences` on death,
    // Thistle Tea's released threat), so none is held through a
    // resurrection.
    crate::game::map::ai::forget(map, g);
    crate::game::map::auras::clear_on_death(map, g);
    crate::game::map::pets::master_died(map, g);
    crate::game::map::intoxication::clear(map, g);
    crate::game::map::duel::left(map, g, false);
    // Out of an arena, free-for-all no more.
    crate::game::map::pvp::died(map, g);
    crate::game::map::trade::cancel(
        map,
        g,
        wow_world_messages::vanilla::SMSG_TRADE_STATUS_TradeStatus::TargetDead,
    );
}

/// `PLAYER_FIELD_BYTES`' first byte's flag: the client shows the time to
/// the release it makes by itself.
pub const PLAYER_FIELD_BYTE_RELEASE_TIMER: u8 = 0x08;

/// How long the dead lie before their spirit is released for them, out of
/// a copy (mangos's `KillPlayer`'s `m_deathTimer`).
pub const AUTO_RELEASE: std::time::Duration = std::time::Duration::from_secs(6 * 60);

/// The dead who have lain six minutes outside a dungeon, raid or
/// battleground released to their graveyard (mangos's `Player::Update`:
/// `BuildPlayerRepop`, `RepopAtGraveyard`; "not auto-free ghost from body
/// in instances").
pub fn auto_release(map: &mut Map) {
    if crate::game::instances::instance_type(&map.realm, map.key.map) != 0 {
        return;
    }
    let now = map.now;
    let due: Vec<Guid> = map
        .players
        .values()
        .filter(|p| {
            matches!(p.death, Death::Dead { since } if now.saturating_duration_since(since) >= AUTO_RELEASE)
        })
        .map(|p| p.guid)
        .collect();
    for g in due {
        release(map, g);
    }
}

/// How deep a player may fall: below it, the void (mangos's
/// `HandleMovementOpcodes`, z under -500).
pub const VOID_Z: f32 = -500.0;

/// A player fallen below the world: the living take their whole health
/// as a fall (`DAMAGE_FALL_TO_VOID`, logged as a fall) and are released,
/// and the dead, a ghost too, are put at their graveyard (mangos's
/// `EnvironmentalDamage`, `KillPlayer`, `BuildPlayerRepop`,
/// `RepopAtGraveyard`).
pub fn below_world(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if p.position().z >= VOID_Z
        || p.flight.is_some()
        || p.movement.flags.get_on_transport().is_some()
    {
        return;
    }
    if p.death == Death::Alive {
        let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        crate::game::map::environment::damage(
            map,
            g,
            wow_world_messages::vanilla::EnvironmentalDamageType::Fall,
            max,
        );
    }
    match map.players.get(&g).map(|p| p.death) {
        Some(Death::Dead { .. }) => release(map, g),
        Some(Death::Ghost { .. }) => repop_at_graveyard(map, g),
        _ => {}
    }
}

/// A ghost put at the graveyard it would be released to (mangos's
/// `RepopAtGraveyard`).
fn repop_at_graveyard(map: &mut Map, g: Guid) {
    let Some((pos, zone, team)) = map.players.get(&g).map(|p| (p.position(), p.zone, p.team))
    else {
        return;
    };
    if let Some((at, facing)) = crate::game::map::battleground::graveyard(map, g) {
        map.teleport(g, map.key, at, facing);
        return;
    }
    let zone = crate::game::map::corpses::graveyard_zone(map, zone);
    let (to, at, facing) = nearest_graveyard(map, zone, team, pos);
    map.teleport(g, to, at, facing);
}

/// `CMSG_REPOP_REQUEST`: release the spirit to the nearest graveyard.
pub fn release(map: &mut Map, g: Guid) {
    let Some((pos, zone, team)) = repop(map, g) else {
        return;
    };
    // In a battleground, the fallen wait at their side's start.
    if let Some((at, facing)) = crate::game::map::battleground::graveyard(map, g) {
        crate::game::map::battleground::released(map, g);
        map.teleport(g, map.key, at, facing);
        return;
    }
    // A fighter fallen in Gurubashi Arena watches from the stands.
    if crate::game::map::arena::release_to_stands(map, g) {
        return;
    }
    let zone = crate::game::map::corpses::graveyard_zone(map, zone);
    let (to, at, facing) = nearest_graveyard(map, zone, team, pos);
    map.teleport(g, to, at, facing);
}

/// The release itself, where the player lies (mangos's `BuildPlayerRepop`):
/// a corpse left, and they a ghost. Where they stood, their zone and side,
/// or `None` if they aren't dead and unreleased.
fn repop(map: &mut Map, g: Guid) -> Option<(Vector3d, u32, Team)> {
    let now = map.now;
    let p = map.players.get_mut(&g)?;
    if !matches!(p.death, Death::Dead { .. }) {
        return None;
    }
    // A ghost walks (mangos's `BuildPlayerRepop` unroots).
    crate::game::map::movement::root_to(&p.outbound, g, false);
    let corpse = Corpse::new(&map.realm.data, p, p.position(), p.movement.orientation);
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::battleground::insignia::UNIT_FLAG_SKINNABLE,
        false,
    );
    p.row.state.corpse = Some(CorpseState {
        map: map.key.map,
        x: corpse.position.x,
        y: corpse.position.y,
        z: corpse.position.z,
        orientation: corpse.orientation,
    });
    p.row.state.released_at = crate::game::map::corpses::unix_ms();
    become_ghost(p, now);
    let (pos, zone, team) = (p.position(), p.zone, p.team);
    map.add_corpse(corpse);
    speeds_to_observers(map, g);
    Some((pos, zone, team))
}

/// A player leaving the world (a logout, the end of a disconnect's
/// linger, a shutdown) dead and not released is released first: a ghost,
/// put at the graveyard their release would take them to, a corpse left
/// where they fell (mangos's `LogoutPlayer`: `BuildPlayerRepop` and
/// `RepopAtGraveyard` while the death timer runs). So they come back a
/// ghost, not risen where they fell. A spirit of redemption dies first, as
/// mangos's `LogoutPlayer` kills one. Before, they were saved dead and
/// unreleased, and rose at their next login at full health.
pub fn release_at_logout(map: &mut Map, g: Guid) {
    if crate::game::map::spells::redemption::spirit(map, g) {
        crate::game::map::spells::redemption::dies(map, g);
    }
    let Some((pos, zone, team)) = repop(map, g) else {
        return;
    };
    let (to, at, facing) = match crate::game::map::battleground::graveyard(map, g) {
        Some((at, facing)) => (map.key.map, at, facing),
        None => {
            let zone = crate::game::map::corpses::graveyard_zone(map, zone);
            let (to, at, facing) = nearest_graveyard(map, zone, team, pos);
            (to.map, at, facing)
        }
    };
    if let Some(p) = map.players.get_mut(&g) {
        p.movement.position = at;
        p.movement.orientation = facing;
        p.movement.flags = MovementInfo_MovementFlags::empty();
        p.flight = None;
        p.row.map = to;
    }
}

/// Make a player a ghost: one health point, the ghost aura, and speed. A
/// ghost logging back in is one since their release, and told the wait
/// left to reclaim.
pub fn become_ghost(p: &mut Player, now: Instant) {
    let unix = crate::game::map::corpses::unix_ms();
    if p.row.state.released_at <= 0 {
        p.row.state.released_at = unix;
    }
    let since = now
        .checked_sub(crate::game::map::corpses::since_release(p, unix))
        .unwrap_or(now);
    p.death = Death::Ghost { since };
    p.row.state.dead = true;
    p.fields.set_u32(UNIT_FIELD_HEALTH, 1);
    p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_GHOST, true);
    p.fields.set_u32(PLAYER_SELF_RES_SPELL, 0);
    p.fields.set_u32(UNIT_FIELD_AURA, SPELL_GHOST);
    // A night elf's wisp runs faster (Thistle Tea's ghost spells).
    let speed = if p.row.race == RACE_NIGHT_ELF {
        p.fields.set_u32(UNIT_FIELD_AURA + 1, SPELL_WISP);
        WISP_SPEED
    } else {
        GHOST_SPEED
    };
    p.speeds.run = 7.0 * speed;
    p.speeds.swim = 4.722_222 * speed;
    p.outbound.send(&SMSG_FORCE_RUN_SPEED_CHANGE {
        guid: p.guid,
        move_event: p.outbound.order(Order::Speed {
            guid: p.guid,
            kind: SpeedKind::Run,
            speed: p.speeds.run,
        }),
        speed: p.speeds.run,
    });
    p.outbound.send(&SMSG_FORCE_SWIM_SPEED_CHANGE {
        guid: p.guid,
        move_event: p.outbound.order(Order::Speed {
            guid: p.guid,
            kind: SpeedKind::Swim,
            speed: p.speeds.swim,
        }),
        speed: p.speeds.swim,
    });
    // A ghost walks on water (mangos's `HandleAuraGhost`: `SetWaterWalk`).
    p.outbound.send(&SMSG_MOVE_WATER_WALK {
        guid: p.guid,
        counter: p.outbound.order(Order::WaterWalk {
            guid: p.guid,
            on: true,
        }),
    });
    let left = crate::game::map::corpses::remaining_ms(p, unix);
    if left > 0 {
        crate::game::map::corpses::send_delay(p, left as u32);
    }
}

/// Bring a player back, with `fraction` of their health, mana and
/// energy, and no rage (Thistle Tea's `Death.resurrect`). Whether they
/// rose.
pub fn resurrect(map: &mut Map, g: Guid, fraction: f32) -> bool {
    // A fighter fallen in Gurubashi Arena's battle stays fallen until it
    // ends.
    if crate::game::map::arena::blocks_resurrection(map, g) {
        if let Some(p) = map.players.get(&g) {
            crate::game::map::chat::send_system(
                &p.outbound,
                "You have fallen in the arena's battle: you rise when it ends.",
            );
        }
        return false;
    }
    let Some(p) = map.players.get_mut(&g) else {
        return false;
    };
    if p.death == Death::Alive {
        return false;
    }
    let was_ghost = matches!(p.death, Death::Ghost { .. });
    // Risen where they fell, they may walk again (mangos's
    // `ResurrectPlayer` unroots).
    if !was_ghost {
        crate::game::map::movement::root_to(&p.outbound, g, false);
    }
    p.death = Death::Alive;
    // An offer from before is spent.
    p.resurrect_offer = None;
    p.resurrect_at = None;
    p.resurrect_map = None;
    p.resurrect_on_arrival = None;
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::battleground::insignia::UNIT_FLAG_SKINNABLE,
        false,
    );
    p.row.state.dead = false;
    p.row.state.released_at = 0;
    let corpse = p.row.state.corpse.take();
    // A share of each, but at least one of what there is any of.
    let share = |max: u32| {
        if max == 0 {
            0
        } else {
            ((max as f32 * fraction) as u32).max(1)
        }
    };
    let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    p.fields.set_u32(UNIT_FIELD_HEALTH, share(max).max(1));
    for power in [0u16, 3] {
        let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1 + power);
        p.fields.set_u32(UNIT_FIELD_POWER1 + power, share(max));
    }
    p.fields.set_u32(UNIT_FIELD_POWER1 + 1, 0);
    p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_GHOST, false);
    let flags = p.fields.get_byte(PLAYER_FIELD_BYTES, 0);
    p.fields.set_byte(
        PLAYER_FIELD_BYTES,
        0,
        flags & !PLAYER_FIELD_BYTE_RELEASE_TIMER,
    );
    if p.fields.get_u32(UNIT_FIELD_AURA) == SPELL_GHOST {
        p.fields.set_u32(UNIT_FIELD_AURA, 0);
    }
    if p.fields.get_u32(UNIT_FIELD_AURA + 1) == SPELL_WISP {
        p.fields.set_u32(UNIT_FIELD_AURA + 1, 0);
    }
    p.fields.set_u32(PLAYER_SELF_RES_SPELL, 0);
    if was_ghost {
        p.speeds.run = 7.0;
        p.speeds.swim = 4.722_222;
        p.outbound.send(&SMSG_FORCE_RUN_SPEED_CHANGE {
            guid: g,
            move_event: p.outbound.order(Order::Speed {
                guid: g,
                kind: SpeedKind::Run,
                speed: p.speeds.run,
            }),
            speed: p.speeds.run,
        });
        p.outbound.send(&SMSG_FORCE_SWIM_SPEED_CHANGE {
            guid: g,
            move_event: p.outbound.order(Order::Speed {
                guid: g,
                kind: SpeedKind::Swim,
                speed: p.speeds.swim,
            }),
            speed: p.speeds.swim,
        });
    }
    if was_ghost {
        speeds_to_observers(map, g);
        // Back on land, unless an aura has them walk on water (mangos's
        // `HandleAuraGhost` on removal).
        if !water_walk_aura(map, g)
            && let Some(p) = map.players.get(&g)
        {
            p.outbound.send(&SMSG_MOVE_LAND_WALK {
                guid: g,
                counter: p.outbound.order(Order::WaterWalk { guid: g, on: false }),
            });
        }
    }
    // Their corpse, if it lies here, is bones (one elsewhere, when they
    // come there: `corpses::arrived`).
    if corpse.is_some() {
        crate::game::map::bones::at_rising(map, g);
    }
    // Risen any way, no longer Waiting to Resurrect.
    crate::game::map::auras::remove_spell(
        map,
        g,
        crate::game::map::battleground::SPELL_WAITING_TO_RESURRECT,
        None,
    );
    // The living see each other again.
    map.visibility_dirty.insert(g);
    let observers: Vec<Guid> = map
        .players
        .get(&g)
        .map(|p| {
            p.known
                .iter()
                .copied()
                .filter(|k| guid::is_player(*k))
                .collect()
        })
        .unwrap_or_default();
    for o in observers {
        map.visibility_dirty.insert(o);
    }
    // The place's auras again (mangos's `ResurrectPlayer` forces
    // `UpdateZone` "to reapply auras").
    crate::game::map::spell_areas::reconcile(map, g);
    true
}

/// Bring a player back with `health` and `mana` (a spell's, or a
/// self-resurrection's), full energy and no rage (Thistle Tea's
/// `Death.resurrect_with`). Whether they rose.
pub fn resurrect_with(map: &mut Map, g: Guid, health: u32, mana: u32) -> bool {
    if !resurrect(map, g, 1.0) {
        return false;
    }
    if let Some(p) = map.players.get_mut(&g) {
        let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        p.fields
            .set_u32(UNIT_FIELD_HEALTH, health.clamp(1, max.max(1)));
        let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
        p.fields.set_u32(UNIT_FIELD_POWER1, mana.min(max));
    }
    true
}

/// Whether `g` is a released ghost.
pub fn is_ghost(map: &Map, g: Guid) -> bool {
    map.players
        .get(&g)
        .is_some_and(|p| matches!(p.death, Death::Ghost { .. }))
}

fn water_walk_aura(map: &Map, g: Guid) -> bool {
    crate::game::map::auras::auras(map, g)
        .is_some_and(|a| a.has(crate::game::map::auras::AURA_WATER_WALK))
}

/// Whether `g` walks on water: a ghost, or with a water-walking aura.
pub fn walks_on_water(map: &Map, g: Guid) -> bool {
    is_ghost(map, g) || water_walk_aura(map, g)
}

/// A client that has loaded a new map has lost its water walking: it is
/// told again (mangos's `SendInitialPacketsAfterAddToMap`).
pub fn resend_water_walk(map: &Map, g: Guid) {
    if walks_on_water(map, g)
        && let Some(p) = map.players.get(&g)
    {
        p.outbound.send(&SMSG_MOVE_WATER_WALK {
            guid: g,
            counter: p.outbound.order(Order::WaterWalk { guid: g, on: true }),
        });
    }
}

/// A ghost's run and swim speeds, or the risen's, to those who see the
/// player, at the change (Thistle Tea's ghost auras change them so).
fn speeds_to_observers(map: &Map, g: Guid) {
    use crate::game::map::misc::{MSG_MOVE_SET_RUN_SPEED, MSG_MOVE_SET_SWIM_SPEED};
    let Some(speeds) = map.players.get(&g).map(|p| p.speeds) else {
        return;
    };
    crate::game::map::misc::speed_to_observers(map, g, MSG_MOVE_SET_RUN_SPEED, speeds.run);
    crate::game::map::misc::speed_to_observers(map, g, MSG_MOVE_SET_SWIM_SPEED, speeds.swim);
}

/// `CMSG_RECLAIM_CORPSE`: a ghost near their corpse, after the delay.
pub fn reclaim(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if !matches!(p.death, Death::Ghost { .. }) {
        return;
    }
    let Some(c) = p.row.state.corpse else {
        return;
    };
    if c.map != map.key.map || !crate::game::map::corpses::ready(p, now) {
        return;
    }
    let pos = p.position();
    let d2 = (pos.x - c.x).powi(2) + (pos.y - c.y).powi(2) + (pos.z - c.z).powi(2);
    if d2 > RECLAIM_YARDS * RECLAIM_YARDS {
        return;
    }
    // In a battleground, whole, and only while the battle is on (Thistle
    // Tea's `restore_percent`; mangos's whole too).
    let fraction = match &map.battleground {
        Some(m) if m.phase == crate::game::map::battleground::Phase::Playing => 1.0,
        Some(_) => return,
        None => 0.5,
    };
    resurrect(map, g, fraction);
}

/// `MSG_CORPSE_QUERY`: where a ghost's corpse is, for their map: in a
/// dungeon they are outside, at its entrance.
pub fn corpse_query(map: &mut Map, g: Guid) {
    let Some(corpse) = map.players.get(&g).map(|p| p.row.state.corpse) else {
        return;
    };
    let result = match corpse {
        Some(c) => {
            let at = Vector3d {
                x: c.x,
                y: c.y,
                z: c.z,
            };
            let (shown, position) = crate::game::map::corpses::marker(map, c.map, at);
            MSG_CORPSE_QUERY_Server_CorpseQueryResult::Found {
                map: crate::game::map::movement::map_enum(shown),
                position,
                corpse_map: crate::game::map::movement::map_enum(c.map),
            }
        }
        None => MSG_CORPSE_QUERY_Server_CorpseQueryResult::NotFound,
    };
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&MSG_CORPSE_QUERY_Server { result });
    }
}

/// A ghost talking to a spirit healer: it offers a resurrection.
pub fn spirit_healer_hello(map: &Map, g: Guid, healer: Guid) -> bool {
    let Some(p) = map.players.get(&g) else {
        return false;
    };
    let Some(c) = map.creature(healer) else {
        return false;
    };
    if c.fields.get_u32(UNIT_NPC_FLAGS) & NPC_FLAG_SPIRIT_HEALER == 0 {
        return false;
    }
    if matches!(p.death, Death::Ghost { .. }) {
        p.outbound
            .send(&SMSG_SPIRIT_HEALER_CONFIRM { guid: healer });
    }
    true
}

/// `CMSG_SPIRIT_HEALER_ACTIVATE`: a ghost resurrected where they stand, by
/// a living spirit healer within reach (Thistle Tea's `valid_healer?`).
pub fn spirit_healer_activate(map: &mut Map, g: Guid, healer: Guid) {
    let ghost = map
        .players
        .get(&g)
        .is_some_and(|p| matches!(p.death, Death::Ghost { .. }));
    if !ghost || crate::game::map::npc::npc_entry(map, g, healer, NPC_FLAG_SPIRIT_HEALER).is_none()
    {
        return;
    }
    // The graveyard nearest the corpse, if it isn't the one nearest the
    // ghost: the risen are put there (mangos's `SendSpiritResurrect`), so a
    // ghost's run is no journey. A corpse on another map is left be.
    let corpse_grave = map.players.get(&g).and_then(|p| {
        let c = p
            .row
            .state
            .corpse
            .as_ref()
            .filter(|c| c.map == map.key.map)?;
        Some((
            Vector3d {
                x: c.x,
                y: c.y,
                z: c.z,
            },
            p.position(),
            p.zone,
            p.team,
        ))
    });
    if resurrect(map, g, 0.5) {
        // Everything carried wears a quarter (Thistle Tea's `SpiritHealer`).
        crate::game::map::vendor::wear_carried(map, g, 0.25);
        resurrection_sickness(map, g);
        if let Some((corpse, ghost, zone, team)) = corpse_grave {
            let corpse_zone =
                crate::game::map::world::zone_and_area(map, corpse).map_or(zone, |(z, _)| z);
            let corpse_zone = crate::game::map::corpses::graveyard_zone(map, corpse_zone);
            let ghost_zone = crate::game::map::corpses::graveyard_zone(map, zone);
            let (to, at, facing) = nearest_graveyard(map, corpse_zone, team, corpse);
            let (_, ghost_at, _) = nearest_graveyard(map, ghost_zone, team, ghost);
            // (No graveyard for the corpse leaves them where they stand.)
            if at != corpse && (to != map.key || at != ghost_at) {
                map.teleport(g, to, at, facing);
            }
        }
    }
}

/// How long resurrection sickness lasts at `level`: a minute a level from
/// eleven, ten at most.
pub fn resurrection_sickness_ms(level: u32) -> Option<u32> {
    resurrection_sickness_from(level, 11)
}

/// [`resurrection_sickness_ms`], from level `start` (`[players]
/// death_sickness_level`, mangos's `Death.SicknessLevel`) rather than
/// eleven.
pub fn resurrection_sickness_from(level: u32, start: u32) -> Option<u32> {
    let start = start.max(1);
    (level >= start).then(|| ((level - start + 1).saturating_mul(60_000)).min(600_000))
}

/// A spirit healer's price: resurrection sickness.
fn resurrection_sickness(map: &mut Map, g: Guid) {
    let level = map.players.get(&g).map_or(1, |p| u32::from(p.level()));
    let Some(ms) = resurrection_sickness_from(level, map.cfg.players.death_sickness_level) else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(SPELL_RESURRECTION_SICKNESS) else {
        return;
    };
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, crate::game::map::spells::points(s, i, level)))
        .collect();
    let _ = crate::game::map::auras::apply_spell(
        map,
        g,
        g,
        s,
        &effects,
        crate::game::map::auras::Context {
            duration: Some(Some(ms)),
            ..Default::default()
        },
    );
}

/// The graveyard a player at `pos` in `zone` goes to: one for their zone and
/// faction, the nearest; else the nearest on the map; else where they are.
pub fn nearest_graveyard(
    map: &Map,
    zone: u32,
    team: Team,
    pos: Vector3d,
) -> (MapKey, Vector3d, f32) {
    let faction = match team {
        Team::Alliance => 469,
        Team::Horde => 67,
    };
    let open = |id| crate::game::map::outdoor_pvp::graveyard_open(map, id, faction);
    match graveyard_for(&map.realm.data, map.key.map, zone, faction, pos, open) {
        Some(gy) => (
            MapKey::world(gy.map),
            Vector3d {
                x: gy.x,
                y: gy.y,
                z: gy.z,
            },
            gy.orientation,
        ),
        None => (map.key, pos, 0.0),
    }
}

/// [`nearest_graveyard`]'s pick for `faction` (469 or 67) at `pos` on map
/// `here`, of those `open` lets them use, as mangos's
/// `GetClosestGraveyardHelper` and Thistle Tea's `Graveyard.closest_of`
/// have it: of the zone's own, the nearest on `here`; else, from a
/// dungeon, the nearest outside by horizontal distance from its entrance;
/// else one on any map. With none of the zone's, the nearest on `here`.
pub fn graveyard_for(
    data: &crate::game::data::GameData,
    here: u32,
    zone: u32,
    faction: u32,
    pos: Vector3d,
    open: impl Fn(u32) -> bool,
) -> Option<&crate::game::data::Graveyard> {
    use crate::game::data::Graveyard;
    let allowed = |gy: &Graveyard| (gy.faction == 0 || gy.faction == faction) && open(gy.id);
    let near =
        |gy: &Graveyard| (gy.x - pos.x).powi(2) + (gy.y - pos.y).powi(2) + (gy.z - pos.z).powi(2);
    let entrance = crate::game::map::corpses::dungeon(data, here)
        .and_then(|d| d.ghost_entrance)
        .filter(|(_, x, y)| *x != 0.0 || *y != 0.0);
    let pick = |of: &dyn Fn(&Graveyard) -> bool| {
        let linked: Vec<&Graveyard> = data
            .graveyards
            .iter()
            .filter(|gy| of(gy) && allowed(gy))
            .collect();
        let here_best = linked
            .iter()
            .filter(|gy| gy.map == here)
            .min_by(|a, b| near(a).total_cmp(&near(b)));
        let outside = entrance.and_then(|(m, x, y)| {
            linked
                .iter()
                .filter(|gy| gy.map == m && m != here)
                .min_by(|a, b| {
                    let d = |gy: &Graveyard| (gy.x - x).powi(2) + (gy.y - y).powi(2);
                    d(a).total_cmp(&d(b))
                })
        });
        here_best
            .or(outside)
            .or_else(|| linked.iter().find(|gy| gy.map != here))
            .copied()
    };
    pick(&|gy| gy.zone == zone).or_else(|| pick(&|gy| gy.map == here))
}

/// How near its corpse a ghost sees (mangos's `(20 + 25) *
/// Rate.Creature.Aggro`, Thistle Tea's `@corpse_sight_range`).
pub const CORPSE_SIGHT_YARDS: f32 = 45.0;

/// Whether `at` is within [`CORPSE_SIGHT_YARDS`] of `observer`'s corpse
/// in this map. A ghost whose corpse is elsewhere sees nothing by it
/// (mangos's `IsWithinDistInMap`).
fn near_corpse(map: &Map, observer: Guid, at: Vector3d) -> bool {
    map.corpses
        .get(&guid::corpse(guid::counter(observer)))
        .is_some_and(|k| {
            let a = k.position;
            (a.x - at.x).powi(2) + (a.y - at.y).powi(2) + (a.z - at.z).powi(2)
                <= CORPSE_SIGHT_YARDS * CORPSE_SIGHT_YARDS
        })
}

/// Whether player `observer` sees creature `c` (mangos-classic's
/// `Creature::IsVisibleInGridForPlayer` and `isInvisibleForAlive`,
/// Thistle Tea's `Visibility.Filter`): the living see all but the spirit
/// healers and guides; a ghost sees those, the creatures shown to ghosts
/// (type flag 0x2), wherever they are, and the living within 45 yards of
/// its corpse.
fn creature_seen(map: &Map, observer: Guid, c: &crate::game::map::creature::Creature) -> bool {
    let spirit = c.fields.get_u32(UNIT_NPC_FLAGS)
        & (crate::game::map::npc::NPC_FLAG_SPIRITHEALER
            | crate::game::map::npc::NPC_FLAG_SPIRITGUIDE)
        != 0;
    if !is_ghost(map, observer) {
        return !spirit;
    }
    let shown_to_ghosts = map
        .realm
        .data
        .creature_template(c.entry)
        .is_some_and(|t| t.type_flags & 0x2 != 0);
    if spirit || shown_to_ghosts {
        return true;
    }
    c.alive() && near_corpse(map, observer, c.position)
}

/// Whether player `observer` sees player `object`, as mangos-classic's
/// `Player::IsVisibleInGridForPlayer`: the living (and the dead not yet
/// released) see all but ghosts; a group or raid sees its own, ghosts or
/// not; a ghost sees the ghosts of its side, and the living within 45
/// yards of its corpse.
fn player_seen(map: &Map, observer: Guid, object: Guid) -> bool {
    let (seer, seen) = (is_ghost(map, observer), is_ghost(map, object));
    if !seer && !seen {
        return true;
    }
    if crate::game::map::objects::same_group(map, observer, object) {
        return true;
    }
    if !seer {
        return false;
    }
    let (Some(o), Some(p)) = (map.players.get(&observer), map.players.get(&object)) else {
        return false;
    };
    if seen {
        return o.team == p.team;
    }
    p.death == Death::Alive && near_corpse(map, observer, p.position())
}

/// Whether a ghost `observer` sees an object other than a unit (a game
/// object, a corpse, a spell's area): only within 45 yards of its corpse,
/// but its own corpse and the transports anywhere.
fn object_seen_by_ghost(map: &Map, observer: Guid, object: Guid) -> bool {
    if map.globals.contains(&object) {
        return true;
    }
    if guid::is_corpse(object) {
        if let Some(k) = map.corpses.get(&object) {
            return (k.owner == observer && !k.is_bones())
                || near_corpse(map, observer, k.position);
        }
        return false;
    }
    if let Some(o) = map.gameobject(object)
        && matches!(o.kind, GO_TYPE_TRANSPORT | GO_TYPE_MO_TRANSPORT)
    {
        return true;
    }
    map.position_of(object)
        .is_some_and(|at| near_corpse(map, observer, at))
}

/// Game object types a ghost may ride wherever it is: an elevator, a boat.
const GO_TYPE_TRANSPORT: u8 = 11;
const GO_TYPE_MO_TRANSPORT: u8 = 15;

/// Whether `observer` may see `object`. A ghost sees only near its corpse:
/// see [`creature_seen`], [`player_seen`] and [`object_seen_by_ghost`]. A
/// game master sees everything.
pub fn can_see(map: &Map, observer: Guid, object: Guid) -> bool {
    let gm = |g: Guid| {
        map.players.get(&g).is_some_and(|p| {
            p.fields
                .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM)
        })
    };
    // A creature a script conceals.
    if guid::is_unit(object)
        && map
            .creature(object)
            .and_then(|c| c.scripted.as_deref())
            .is_some_and(|s| s.concealed)
    {
        return gm(observer);
    }
    if !crate::game::map::spells::stealth::sees(map, observer, object) {
        return false;
    }
    // A stealthed trap, from its owner's enemies.
    if guid::is_game_object(object) && !crate::game::map::objects::seen_by(map, observer, object) {
        return false;
    }
    if observer == object || gm(observer) {
        return true;
    }
    if let Some(c) = map.creature(object) {
        return creature_seen(map, observer, c);
    }
    if guid::is_player(object) {
        return player_seen(map, observer, object);
    }
    !is_ghost(map, observer) || object_seen_by_ghost(map, observer, object)
}
