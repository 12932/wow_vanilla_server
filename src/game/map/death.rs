//! Death: a player who dies can release their spirit to the nearest
//! graveyard as a ghost, leaving a corpse; run back and reclaim it; or ask
//! a spirit healer. Ghosts see the living; the living don't see ghosts.

use std::time::{Duration, Instant};

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

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
/// How long after release a ghost must wait to reclaim.
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
    pub fn new(p: &Player, pos: Vector3d, orientation: f32) -> Self {
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
        for slot in 0..19u16 {
            let entry = p
                .fields
                .get_u32(PLAYER_VISIBLE_ITEM_1_0 + slot * VISIBLE_ITEM_SIZE);
            if entry != 0 {
                let display = p.item_display_id(entry);
                f.set_u32(CORPSE_FIELD_ITEM + slot, display);
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
        // A player's corpse.
        f.set_u32(CORPSE_FIELD_FLAGS, 0x04);
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
        let (g, cell) = (c.guid, c.cell);
        self.grid.insert(g, cell);
        self.corpses.insert(g, c);
        self.visibility_dirty.insert(g);
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
    p.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    p.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    p.logout_at = None;
    crate::game::map::misc::capture_self_res(map, g);
    crate::game::map::combat::leave_combat(map, g);
    crate::game::map::auras::clear_on_death(map, g);
    crate::game::map::duel::left(map, g, false);
    crate::game::map::trade::cancel(
        map,
        g,
        wow_world_messages::vanilla::SMSG_TRADE_STATUS_TradeStatus::TargetDead,
    );
}

/// `CMSG_REPOP_REQUEST`: release the spirit to the nearest graveyard.
pub fn release(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if !matches!(p.death, Death::Dead { .. }) {
        return;
    }
    let corpse = Corpse::new(p, p.position(), p.movement.orientation);
    p.row.state.corpse = Some(CorpseState {
        map: map.key.map,
        x: corpse.position.x,
        y: corpse.position.y,
        z: corpse.position.z,
        orientation: corpse.orientation,
    });
    become_ghost(p, now);
    let (pos, zone, team) = (p.position(), p.zone, p.team);
    map.add_corpse(corpse);
    // In a battleground, the fallen wait at their side's start.
    if let Some((at, facing)) = crate::game::map::battleground::graveyard(map, g) {
        map.teleport(g, map.key, at, facing);
        return;
    }
    // A fighter fallen in Gurubashi Arena watches from the stands.
    if crate::game::map::arena::release_to_stands(map, g) {
        return;
    }
    let (to, at, facing) = nearest_graveyard(map, zone, team, pos);
    map.teleport(g, to, at, facing);
}

/// Make a player a ghost: one health point, the ghost aura, and speed.
pub fn become_ghost(p: &mut Player, now: Instant) {
    p.death = Death::Ghost { since: now };
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
        move_event: 0,
        speed: p.speeds.run,
    });
    p.outbound.send(&SMSG_FORCE_SWIM_SPEED_CHANGE {
        guid: p.guid,
        move_event: 0,
        speed: p.speeds.swim,
    });
    p.outbound.send(&SMSG_CORPSE_RECLAIM_DELAY {
        delay: RECLAIM_DELAY,
    });
}

/// Bring a player back, with `fraction` of their health and mana.
pub fn resurrect(map: &mut Map, g: Guid, fraction: f32) {
    // A fighter fallen in Gurubashi Arena's battle stays fallen until it
    // ends.
    if crate::game::map::arena::blocks_resurrection(map, g) {
        if let Some(p) = map.players.get(&g) {
            crate::game::map::chat::send_system(
                &p.outbound,
                "You have fallen in the arena's battle: you rise when it ends.",
            );
        }
        return;
    }
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.death == Death::Alive {
        return;
    }
    let was_ghost = matches!(p.death, Death::Ghost { .. });
    p.death = Death::Alive;
    p.row.state.dead = false;
    let corpse = p.row.state.corpse.take();
    let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    p.fields
        .set_u32(UNIT_FIELD_HEALTH, ((max as f32 * fraction) as u32).max(1));
    let max_mana = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
    p.fields
        .set_u32(UNIT_FIELD_POWER1, (max_mana as f32 * fraction) as u32);
    p.fields.set_flag(PLAYER_FLAGS, PLAYER_FLAGS_GHOST, false);
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
            move_event: 0,
            speed: p.speeds.run,
        });
        p.outbound.send(&SMSG_FORCE_SWIM_SPEED_CHANGE {
            guid: g,
            move_event: 0,
            speed: p.speeds.swim,
        });
    }
    if corpse.is_some() {
        map.remove_corpse(guid::corpse(guid::counter(g)));
    }
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
}

/// `CMSG_RECLAIM_CORPSE`: a ghost near their corpse, after the delay.
pub fn reclaim(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let Death::Ghost { since } = p.death else {
        return;
    };
    let Some(c) = p.row.state.corpse else {
        return;
    };
    if c.map != map.key.map || now.duration_since(since) < RECLAIM_DELAY {
        return;
    }
    let pos = p.position();
    let d2 = (pos.x - c.x).powi(2) + (pos.y - c.y).powi(2) + (pos.z - c.z).powi(2);
    if d2 > RECLAIM_YARDS * RECLAIM_YARDS {
        return;
    }
    resurrect(map, g, 0.5);
}

/// `MSG_CORPSE_QUERY`: where a ghost's corpse is, for their map.
pub fn corpse_query(map: &Map, g: Guid) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let result = match p.row.state.corpse {
        Some(c) => MSG_CORPSE_QUERY_Server_CorpseQueryResult::Found {
            map: crate::game::map::movement::map_enum(c.map),
            position: Vector3d {
                x: c.x,
                y: c.y,
                z: c.z,
            },
            corpse_map: crate::game::map::movement::map_enum(c.map),
        },
        None => MSG_CORPSE_QUERY_Server_CorpseQueryResult::NotFound,
    };
    p.outbound.send(&MSG_CORPSE_QUERY_Server { result });
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

/// `CMSG_SPIRIT_HEALER_ACTIVATE`: resurrected where the ghost stands.
pub fn spirit_healer_activate(map: &mut Map, g: Guid, healer: Guid) {
    let near = match (map.players.get(&g), map.creature(healer)) {
        (Some(p), Some(c)) => {
            let (a, b) = (p.position(), c.position);
            (a.x - b.x).powi(2) + (a.y - b.y).powi(2) <= 15.0 * 15.0
                && c.fields.get_u32(UNIT_NPC_FLAGS) & NPC_FLAG_SPIRIT_HEALER != 0
        }
        _ => false,
    };
    if near {
        resurrect(map, g, 0.5);
        crate::game::map::vendor::wear(map, g, 0.25);
        resurrection_sickness(map, g);
    }
}

/// How long resurrection sickness lasts at `level`: a minute a level from
/// eleven, ten at most.
pub fn resurrection_sickness_ms(level: u32) -> Option<u32> {
    (level >= 11).then(|| ((level - 10) * 60_000).min(600_000))
}

/// A spirit healer's price: resurrection sickness.
fn resurrection_sickness(map: &mut Map, g: Guid) {
    let level = map.players.get(&g).map_or(1, |p| u32::from(p.level()));
    let Some(ms) = resurrection_sickness_ms(level) else {
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
    let data = &map.realm.data;
    let on_map = |gy: &&crate::game::data::Graveyard| {
        gy.map == map.key.map && (gy.faction == 0 || gy.faction == faction)
    };
    let dist = |gy: &crate::game::data::Graveyard| (gy.x - pos.x).powi(2) + (gy.y - pos.y).powi(2);
    let best = data
        .graveyards
        .iter()
        .filter(|gy| gy.zone == zone && (gy.faction == 0 || gy.faction == faction))
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .or_else(|| {
            data.graveyards
                .iter()
                .filter(on_map)
                .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        });
    match best {
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

/// Whether `observer` may see `object`: the living don't see ghosts.
pub fn can_see(map: &Map, observer: Guid, object: Guid) -> bool {
    if !guid::is_player(object) || observer == object {
        return true;
    }
    let ghost = |g: Guid| {
        map.players
            .get(&g)
            .is_some_and(|p| matches!(p.death, Death::Ghost { .. }))
    };
    if !ghost(object) {
        return true;
    }
    ghost(observer)
        || map.players.get(&observer).is_some_and(|p| {
            p.fields
                .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM)
        })
}
