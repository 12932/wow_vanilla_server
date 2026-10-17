//! The smaller player systems Thistle Tea has: far sight, the mount's
//! special animation, growth auras, the looking-for-group stub,
//! self-resurrection (soulstones and Reincarnation), summons, talent
//! resets, unlearning a skill, and the movement acknowledgements other
//! clients hear of (speed changes, knockbacks, skipped time).

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::conn::encode;
use crate::game::data::items::Body;
use crate::game::fields::*;
use crate::game::map::death::Death;
use crate::game::map::{Map, MapKey};

pub const MSG_MOVE_KNOCK_BACK: u16 = 0x0F1;
pub const MSG_MOVE_SET_RUN_SPEED: u16 = 0x0CD;
pub const MSG_MOVE_SET_RUN_BACK_SPEED: u16 = 0x0CF;
pub const MSG_MOVE_SET_WALK_SPEED: u16 = 0x0D1;
pub const MSG_MOVE_SET_SWIM_SPEED: u16 = 0x0D3;
pub const MSG_MOVE_SET_SWIM_BACK_SPEED: u16 = 0x0D5;
pub const MSG_MOVE_SET_TURN_RATE: u16 = 0x0D8;
pub const MSG_MOVE_TIME_SKIPPED: u16 = 0x319;

/// Soulstone Resurrection's ranks, and the self-resurrection each gives.
const SOULSTONES: [(u32, u32); 5] = [
    (20707, 3026),
    (20762, 20758),
    (20763, 20759),
    (20764, 20760),
    (20765, 20761),
];
const REINCARNATION: u32 = 21169;
const REINCARNATION_PASSIVE: u32 = 20608;
/// The Untalent spell's visual, cast on a reset.
const SPELL_UNTALENT_VISUAL: u32 = 14867;
/// How long a summons stands.
pub const SUMMON_FOR: Duration = Duration::from_secs(120);

/// What a player has going outside the stored character.
#[derive(Debug, Default)]
pub struct PlayerMisc {
    pub rest_since: Option<Instant>,
    pub rest_checked_at: Option<Instant>,
    pub breath: Option<crate::game::map::environment::Mirror>,
    pub fatigue: Option<crate::game::map::environment::Mirror>,
    pub env_damage_at: Option<Instant>,
    pub env_checked_at: Option<Instant>,
    /// A summons waiting: by whom, to where, until when.
    pub summon: Option<Summon>,
    /// A quest a group member shared, waiting to be taken.
    pub quest_shared: Option<(Guid, u32)>,
    /// When the self-resurrection last went off.
    pub self_res_at: Option<Instant>,
    /// An opened item's loot: the item, and what is in it.
    pub item_loot: Option<(Guid, crate::game::map::loot::Loot)>,
}

/// A summons: to the summoner's side.
#[derive(Debug, Clone, Copy)]
pub struct Summon {
    pub by: Guid,
    pub to: MapKey,
    pub at: Vector3d,
    pub orientation: f32,
    pub until: Instant,
}

/// The packed GUID and movement info of a movement packet, as
/// wow_messages lays them out, for the packets it doesn't have.
pub fn movement_bytes(guid: Guid, info: &MovementInfo) -> Vec<u8> {
    let frame = encode(&MSG_MOVE_HEARTBEAT_Server {
        guid,
        info: info.clone(),
    })
    .expect("a heartbeat encodes");
    frame[4..].to_vec()
}

fn relay(map: &mut Map, g: Guid, info: MovementInfo, opcode: u16, tail: &[u8]) {
    if let Some(p) = map.players.get_mut(&g) {
        p.movement = info.clone();
    }
    let mut b = Body(movement_bytes(g, &info));
    b.0.extend_from_slice(tail);
    let frame = b.frame(opcode);
    map.send_to_observers(g, &frame, false);
}

fn speed_ack(map: &mut Map, g: Guid, info: MovementInfo, speed: f32, opcode: u16) {
    relay(map, g, info, opcode, &speed.to_le_bytes());
}

/// Every opcode the player systems here and beside them handle, before
/// movement's own: reputation, quest sharing, tickets, items, meeting
/// stones, express flights, and those of this module.
pub fn handle_all(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    let msg = handle(map, g, msg)?;
    let msg = crate::game::map::reputation::handle(map, g, msg)?;
    let msg = crate::game::map::questshare::handle(map, g, msg)?;
    let msg = crate::game::map::tickets::handle(map, g, msg)?;
    let msg = crate::game::map::items_misc::handle(map, g, msg)?;
    let msg = crate::game::map::meeting_stones::handle(map, g, msg)?;
    match msg {
        ClientOpcodeMessage::CMSG_ACTIVATETAXIEXPRESS(m) => {
            crate::game::map::npc::activate_express(map, g, m.guid, &m.nodes);
            None
        }
        other => Some(other),
    }
}

impl Map {
    /// A packet wow_messages couldn't read, by its opcode.
    pub fn handle_raw(&mut self, guid: Guid, opcode: u16, body: &[u8]) {
        if !self.players.contains_key(&guid) {
            return;
        }
        if crate::game::map::reputation::handle_raw(self, guid, opcode, body) {
            return;
        }
        tracing::trace!("raw opcode 0x{opcode:03X} not handled");
    }
}

/// The opcodes of this module.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        C::CMSG_FORCE_RUN_SPEED_CHANGE_ACK(m) if m.guid == g => {
            speed_ack(map, g, m.info, m.new_speed, MSG_MOVE_SET_RUN_SPEED)
        }
        C::CMSG_FORCE_RUN_BACK_SPEED_CHANGE_ACK(m) if m.guid == g => {
            speed_ack(map, g, m.info, m.new_speed, MSG_MOVE_SET_RUN_BACK_SPEED)
        }
        C::CMSG_FORCE_SWIM_SPEED_CHANGE_ACK(m) if m.guid == g => {
            speed_ack(map, g, m.info, m.new_speed, MSG_MOVE_SET_SWIM_SPEED)
        }
        C::CMSG_FORCE_SWIM_BACK_SPEED_CHANGE_ACK(m) if m.guid == g => {
            speed_ack(map, g, m.info, m.new_speed, MSG_MOVE_SET_SWIM_BACK_SPEED)
        }
        C::CMSG_FORCE_WALK_SPEED_CHANGE_ACK(m) if m.guid == g => {
            speed_ack(map, g, m.info, m.new_speed, MSG_MOVE_SET_WALK_SPEED)
        }
        C::CMSG_FORCE_TURN_RATE_CHANGE_ACK(m) if m.guid == g => {
            relay(map, g, m.info, MSG_MOVE_SET_TURN_RATE, &std::f32::consts::PI.to_le_bytes())
        }
        C::CMSG_MOVE_KNOCK_BACK_ACK(m) if m.guid == g => {
            // The jump's angle and speeds, as the client fell.
            let tail: Vec<u8> = match m.info.flags.get_jumping() {
                Some(j) => [j.cos_angle, j.sin_angle, j.xy_speed, j.z_speed]
                    .iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect(),
                None => vec![0; 16],
            };
            relay(map, g, m.info, MSG_MOVE_KNOCK_BACK, &tail)
        }
        C::CMSG_MOVE_TIME_SKIPPED(m) if m.guid == g => {
            if let Some(p) = map.players.get_mut(&g) {
                p.movement.timestamp = p.movement.timestamp.wrapping_add(m.lag);
            }
            let mut b = Body::default();
            crate::game::update::write_packed_guid(&mut b.0, g);
            b.u32(m.lag);
            let frame = b.frame(MSG_MOVE_TIME_SKIPPED);
            map.send_to_observers(g, &frame, false);
        }
        C::CMSG_FAR_SIGHT(m) => far_sight(map, g, m.operation == FarSightOperation::Add),
        C::CMSG_MOUNTSPECIAL_ANIM => {
            if let Some(frame) = encode(&SMSG_MOUNTSPECIAL_ANIM { guid: g }) {
                map.send_to_observers(g, &frame, true);
            }
        }
        C::CMSG_CANCEL_GROWTH_AURA => cancel_growth(map, g),
        C::MSG_LOOKING_FOR_GROUP => {
            if let Some(p) = map.players.get(&g) {
                p.outbound
                    .send(&MSG_LOOKING_FOR_GROUP_Server { unknown1: 0 });
            }
        }
        C::CMSG_SELF_RES => self_resurrect(map, g),
        C::CMSG_SUMMON_RESPONSE(m) => accept_summon(map, g, m.summoner),
        C::MSG_TALENT_WIPE_CONFIRM(m) => talent_wipe(map, g, m.wiping_npc),
        C::CMSG_UNLEARN_SKILL(m) => unlearn_skill(map, g, m.skill.as_int() as u32),
        other => return Some(other),
    }
    None
}

/// Everything here time drives: rest, breath and fatigue, summonses
/// lapsing.
pub fn update(map: &mut Map) {
    crate::game::map::rest::update(map);
    crate::game::map::environment::update(map);
    let now = map.now;
    for p in map.players.values_mut() {
        if p.misc.summon.is_some_and(|s| s.until <= now) {
            p.misc.summon = None;
        }
    }
}

/// `CMSG_FAR_SIGHT`: the client's view moves to (or back from) a far sight
/// object; the server only keeps the field in step.
fn far_sight(map: &mut Map, g: Guid, add: bool) {
    if let Some(p) = map.players.get_mut(&g)
        && !add
    {
        p.fields.set_guid(PLAYER_FARSIGHT, Guid::zero());
    }
}

/// `CMSG_CANCEL_GROWTH_AURA`: auras that make the player bigger come off.
fn cancel_growth(map: &mut Map, g: Guid) {
    use crate::game::map::auras::{self, AURA_MOD_SCALE};
    auras::remove_where(map, g, |h| {
        h.positive
            && h
                .effects
                .iter()
                .any(|a| a.kind == AURA_MOD_SCALE && a.amount > 0)
    });
}

/// At death: the self-resurrection a soulstone or Reincarnation offers,
/// before the auras go.
pub fn capture_self_res(map: &mut Map, g: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let stone = p.auras.holders.iter().find_map(|h| {
        SOULSTONES
            .iter()
            .find(|(aura, _)| *aura == h.spell)
            .map(|(_, res)| *res)
    });
    let reincarnation = p.spells.contains(&REINCARNATION_PASSIVE)
        && p
            .misc
            .self_res_at
            .is_none_or(|t| now.duration_since(t) >= reincarnation_cooldown(&data))
        && data
            .spells
            .get(REINCARNATION)
            .is_some_and(|s| s.reagents.iter().all(|(r, n)| *r == 0 || p.count_of(*r) >= *n));
    let spell = stone.unwrap_or(if reincarnation { REINCARNATION } else { 0 });
    p.fields.set_u32(PLAYER_SELF_RES_SPELL, spell);
}

fn reincarnation_cooldown(data: &crate::game::data::GameData) -> Duration {
    Duration::from_millis(u64::from(
        data.spells
            .get(REINCARNATION)
            .map_or(3_600_000, |s| s.recovery_time.max(s.category_recovery_time)),
    ))
}

/// `CMSG_SELF_RES`: dead and not released, back with the spell's share of
/// health and mana.
fn self_resurrect(map: &mut Map, g: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if !matches!(p.death, Death::Dead { .. }) {
        return;
    }
    let spell = p.fields.get_u32(PLAYER_SELF_RES_SPELL);
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    // `SELF_RESURRECT`: points are a percentage, or (below zero) health
    // with the misc value's mana.
    let Some(e) = s.effects.iter().find(|e| e.effect == 94) else {
        return;
    };
    let points = e.base_points + e.base_dice.max(1);
    let (max_health, max_mana) = (
        p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
        p.fields.get_u32(UNIT_FIELD_MAXPOWER1),
    );
    let (health, mana) = if points < 0 {
        ((-points) as u32, e.misc_value.max(0) as u32)
    } else {
        (max_health * points as u32 / 100, max_mana * points as u32 / 100)
    };
    // Reincarnation takes its Ankh.
    for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0) {
        let mut left = *count;
        while left > 0 {
            let Some(i) = map
                .players
                .get(&g)
                .and_then(|p| p.items.iter().position(|it| it.entry == *reagent))
            else {
                break;
            };
            let n = map.players[&g].items[i].count().min(left);
            crate::game::map::inventory::take_item(map, g, i, n);
            left -= n;
        }
    }
    crate::game::map::death::resurrect(map, g, 1.0);
    if let Some(p) = map.players.get_mut(&g) {
        p.fields
            .set_u32(UNIT_FIELD_HEALTH, health.clamp(1, max_health.max(1)));
        p.fields.set_u32(UNIT_FIELD_POWER1, mana.min(max_mana));
        p.fields.set_u32(PLAYER_SELF_RES_SPELL, 0);
        if spell == REINCARNATION {
            p.misc.self_res_at = Some(now);
        }
    }
}

/// Offer `target` (anywhere in the realm) a summons to where `summoner`
/// stands.
pub fn offer_summon(map: &Map, summoner: Guid, target: Guid) {
    let Some(s) = map.players.get(&summoner) else {
        return;
    };
    let Some(online) = map.realm.registry.online(target) else {
        return;
    };
    let summon = Summon {
        by: summoner,
        to: map.key,
        at: s.position(),
        orientation: s.movement.orientation,
        until: Instant::now() + SUMMON_FOR,
    };
    let zone = s.zone;
    online.map().call(move |m| {
        let Some(p) = m.players.get_mut(&target) else {
            return;
        };
        if p.death != Death::Alive {
            return;
        }
        p.misc.summon = Some(summon);
        p.outbound.send(&SMSG_SUMMON_REQUEST {
            summoner,
            area: Area::try_from(zone).unwrap_or_default(),
            auto_decline_time: SUMMON_FOR,
        });
    });
}

/// `CMSG_SUMMON_RESPONSE`: the summons is taken.
fn accept_summon(map: &mut Map, g: Guid, summoner: Guid) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let Some(s) = p.misc.summon.take() else {
        return;
    };
    if s.by != summoner || s.until <= now || p.death != Death::Alive {
        return;
    }
    if p.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    ) {
        return;
    }
    map.teleport(g, s.to, s.at, s.orientation);
}

/// The cost of a player's next talent reset, in copper: 1, 5 and 10 gold,
/// then 5 more each time to 50, less 5 a month since the last.
pub fn talent_wipe_cost(last: u32, last_at: i64, now: i64) -> u32 {
    const GOLD: u32 = 10_000;
    if last < GOLD {
        return GOLD;
    }
    if last < 5 * GOLD {
        return 5 * GOLD;
    }
    if last < 10 * GOLD {
        return 10 * GOLD;
    }
    let months = ((now - last_at).max(0) / (30 * 24 * 3600)) as u32;
    if months > 0 {
        return last.saturating_sub(5 * GOLD * months).max(10 * GOLD);
    }
    (last + 5 * GOLD).min(50 * GOLD)
}

/// The trainer's offer to reset the player's talents (gossip option 16).
pub fn talent_wipe_offer(map: &mut Map, g: Guid, npc: Guid) {
    let now = chrono::Utc::now().timestamp();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let cost = talent_wipe_cost(p.row.state.talent_wipes, p.row.state.last_talent_wipe, now);
    p.outbound.send(&SMSG_GOSSIP_COMPLETE {});
    p.outbound.send(&MSG_TALENT_WIPE_CONFIRM_Server {
        wiping_npc: npc,
        cost_in_copper: cost,
    });
}

/// The talent spells a player knows, and those they taught.
fn talent_spells(p: &crate::game::map::player::Player, data: &crate::game::data::GameData) -> Vec<u32> {
    let mut out = Vec::new();
    for t in data.spells.talents.values() {
        for r in t.ranks.iter().filter(|r| **r != 0 && p.spells.contains(r)) {
            out.push(*r);
            if let Some(s) = data.spells.get(*r) {
                for e in s.effects.iter().filter(|e| e.effect == 36 && e.trigger_spell != 0) {
                    if p.spells.contains(&e.trigger_spell) {
                        out.push(e.trigger_spell);
                    }
                }
            }
        }
    }
    out
}

/// Take spells off a player: their auras go, and the client forgets them.
pub fn forget_spells(map: &mut Map, g: Guid, spells: &[u32]) {
    for spell in spells {
        crate::game::map::auras::remove_spell(map, g, *spell, Some(g));
        if let Some(p) = map.players.get_mut(&g) {
            p.spells.retain(|s| s != spell);
            if let Ok(id) = u16::try_from(*spell) {
                p.outbound.send(&SMSG_REMOVED_SPELL { spell: id });
            }
        }
    }
}

/// `MSG_TALENT_WIPE_CONFIRM`: paid for, the talents are unlearned.
fn talent_wipe(map: &mut Map, g: Guid, npc: Guid) {
    let data = map.realm.data.clone();
    let now = chrono::Utc::now().timestamp();
    let near = match (map.players.get(&g), map.creature(npc)) {
        (Some(p), Some(c)) => {
            crate::game::map::combat::dist(p.position(), c.position) <= 10.0
                && c.fields.get_u32(UNIT_NPC_FLAGS) & crate::game::map::npc::NPC_FLAG_TRAINER != 0
        }
        _ => false,
    };
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let cost = talent_wipe_cost(p.row.state.talent_wipes, p.row.state.last_talent_wipe, now);
    let spells = talent_spells(p, &data);
    if !near || spells.is_empty() || p.money() < cost {
        p.outbound.send(&MSG_TALENT_WIPE_CONFIRM_Server {
            wiping_npc: Guid::zero(),
            cost_in_copper: 0,
        });
        return;
    }
    forget_spells(map, g, &spells);
    if let Some(p) = map.players.get_mut(&g) {
        let money = p.money() - cost;
        p.set_money(money);
        p.row.state.talent_wipes = cost;
        p.row.state.last_talent_wipe = now;
        crate::game::map::spells::refresh_talent_points(p, &data);
    }
    crate::game::map::spells::cast_triggered(map, g, SPELL_UNTALENT_VISUAL, Some(g));
}

/// `CMSG_UNLEARN_SKILL`: a profession (or other skill the player may drop)
/// and its spells are forgotten.
fn unlearn_skill(map: &mut Map, g: Guid, skill: u32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if crate::game::map::skills::value(p, skill).is_none() {
        return;
    }
    let spells: Vec<u32> = p
        .spells
        .iter()
        .copied()
        .filter(|s| data.spells.spell_skills.get(s) == Some(&skill))
        .chain(p.spells.iter().copied().filter(|s| {
            data.spells.get(*s).is_some_and(|t| {
                t.effects
                    .iter()
                    .any(|e| e.effect == crate::game::map::spells::EFFECT_SKILL && e.misc_value as u32 == skill)
            })
        }))
        .collect();
    forget_spells(map, g, &spells);
    if let Some(p) = map.players.get_mut(&g) {
        crate::game::map::skills::forget_skill(p, skill);
    }
}

/// Encode a frame by hand, for the tests.
pub fn frame(opcode: u16, body: &[u8]) -> Arc<[u8]> {
    Body(body.to_vec()).frame(opcode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talent_resets_cost_more_each_time_and_less_with_months() {
        const G: u32 = 10_000;
        assert_eq!(talent_wipe_cost(0, 0, 0), G);
        assert_eq!(talent_wipe_cost(G, 0, 0), 5 * G);
        assert_eq!(talent_wipe_cost(5 * G, 0, 0), 10 * G);
        assert_eq!(talent_wipe_cost(10 * G, 0, 0), 15 * G);
        assert_eq!(talent_wipe_cost(50 * G, 0, 0), 50 * G);
        // Two months on: ten gold off.
        assert_eq!(talent_wipe_cost(30 * G, 0, 61 * 24 * 3600), 20 * G);
    }
}
