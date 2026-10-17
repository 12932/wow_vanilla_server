//! Death: a player who dies can release their spirit to the nearest
//! graveyard as a ghost, leaving a corpse; run back and reclaim it; or ask
//! a spirit healer. Ghosts see the living; the living don't see ghosts.

use std::time::{Duration, Instant};

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

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
        f.set_u32(CORPSE_FIELD_FLAGS, 0x04 | insignia);
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
    crate::game::map::corpses::died(p, crate::game::map::corpses::unix_ms());
    p.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    p.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    // The dead dance, sit or kneel no more (Thistle Tea's death clears
    // the emote state and the posture).
    p.fields.set_u32(UNIT_NPC_EMOTESTATE, 0);
    p.set_stand_state(0);
    p.logout_at = None;
    crate::game::map::misc::capture_self_res(map, g);
    crate::game::map::combat::leave_combat(map, g);
    crate::game::map::auras::clear_on_death(map, g);
    crate::game::map::intoxication::clear(map, g);
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
        move_event: 0,
        speed: p.speeds.run,
    });
    p.outbound.send(&SMSG_FORCE_SWIM_SPEED_CHANGE {
        guid: p.guid,
        move_event: 0,
        speed: p.speeds.swim,
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
    p.death = Death::Alive;
    // An offer from before is spent.
    p.resurrect_offer = None;
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
    if was_ghost {
        speeds_to_observers(map, g);
    }
    if corpse.is_some() {
        map.remove_corpse(guid::corpse(guid::counter(g)));
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
    if resurrect(map, g, 0.5) {
        // Everything carried wears a quarter (Thistle Tea's `SpiritHealer`).
        crate::game::map::vendor::wear_carried(map, g, 0.25);
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
        gy.map == map.key.map
            && (gy.faction == 0 || gy.faction == faction)
            && crate::game::map::outdoor_pvp::graveyard_open(map, gy.id, faction)
    };
    // From a dungeon, one outside is as far as it is from the entrance.
    let dist = |gy: &crate::game::data::Graveyard| {
        let from = crate::game::map::corpses::reckoned_from(data, map.key.map, gy.map, pos);
        (gy.x - from.x).powi(2) + (gy.y - from.y).powi(2)
    };
    let best = data
        .graveyards
        .iter()
        .filter(|gy| gy.zone == zone && (gy.faction == 0 || gy.faction == faction))
        .filter(|gy| crate::game::map::outdoor_pvp::graveyard_open(map, gy.id, faction))
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

/// How near its corpse a ghost sees the living (Thistle Tea's
/// `@corpse_sight_range`; mangos's `(20 + 25) * Rate.Creature.Aggro`).
pub const CORPSE_SIGHT_YARDS: f32 = 45.0;

/// Whether player `observer` sees creature `c` (Thistle Tea's
/// `Visibility.Filter`, mangos-classic's `IsVisibleInGridForPlayer` and
/// `isInvisibleForAlive`): the living see all but the spirit healers and
/// guides; a ghost sees those, the creatures its kind are shown to (type
/// flag 0x2), and the living within 45 yards of its corpse.
fn creature_seen(map: &Map, observer: Guid, c: &crate::game::map::creature::Creature) -> bool {
    let spirit = c.fields.get_u32(UNIT_NPC_FLAGS)
        & (crate::game::map::npc::NPC_FLAG_SPIRITHEALER
            | crate::game::map::npc::NPC_FLAG_SPIRITGUIDE)
        != 0;
    let ghost = map
        .players
        .get(&observer)
        .is_some_and(|p| matches!(p.death, Death::Ghost { .. }));
    if !ghost {
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
    let corpse = map.corpses.get(&guid::corpse(guid::counter(observer)));
    c.alive()
        && corpse.is_some_and(|k| {
            let (a, b) = (k.position, c.position);
            (a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)
                <= CORPSE_SIGHT_YARDS * CORPSE_SIGHT_YARDS
        })
}

/// Whether `observer` may see `object`: the living don't see ghosts.
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
    if let Some(c) = map.creature(object)
        && !gm(observer)
    {
        return creature_seen(map, observer, c);
    }
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
    ghost(observer) || gm(observer)
}
