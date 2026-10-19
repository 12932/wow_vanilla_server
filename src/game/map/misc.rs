//! The smaller player systems Thistle Tea has: far sight, the mount's
//! special animation, growth auras, the looking-for-group stub,
//! self-resurrection (soulstones and Reincarnation), summons, talent
//! resets, unlearning a skill, and the movement acknowledgements other
//! clients hear of (knockbacks, skipped time). A speed change they hear of
//! at the change itself ([`speed_to_observers`]), not at its
//! acknowledgement.

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
pub const MSG_MOVE_SET_SWIM_SPEED: u16 = 0x0D3;
pub const MSG_MOVE_SET_SWIM_BACK_SPEED: u16 = 0x0D5;
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
    /// `[rates] rest_in_game`, as the rest check last read it (none: 1).
    pub rest_rate: Option<f32>,
    pub breath: Option<crate::game::map::environment::Mirror>,
    pub fatigue: Option<crate::game::map::environment::Mirror>,
    /// Lava's grace: a second touching it before it burns.
    pub lava: Option<crate::game::map::environment::Mirror>,
    /// The spell the liquid the player is in put on (Naxxramas's slime).
    pub liquid_spell: Option<u32>,
    /// In water deep enough to swim, by the terrain's liquid.
    pub in_water: bool,
    pub env_checked_at: Option<Instant>,
    /// A summons waiting: by whom, to where, until when.
    pub summon: Option<Summon>,
    /// A quest a group member shared, waiting to be taken.
    pub quest_shared: Option<(Guid, u32)>,
    /// A GM's god mode: no damage taken.
    pub god: bool,
    /// The intro cinematic's camera, and where it has the player see from.
    pub camera: Option<crate::game::map::cinematic::Camera>,
    pub view: Option<Vector3d>,
    /// An opened item's loot: the item, and what is in it.
    pub item_loot: Option<(Guid, crate::game::map::loot::Loot)>,
    /// When the items were last looked over for where they may be, whether
    /// the bank has been since login, and the timed items told of.
    pub items_checked_at: Option<Instant>,
    pub items_login_checked: bool,
    pub timed_told: Vec<Guid>,
    /// A logout under way sat and rooted the player.
    pub logout_rooted: bool,
    /// A fall under way: whether it went far, and the transport it is
    /// measured on.
    pub fall_far: bool,
    pub fall_transport: Guid,
    /// When it last cast a heal: a Stormwind Elite Guard's Charge goes for
    /// a healer who healed lately (`map/elites.rs`).
    pub healed_at: Option<Instant>,
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

/// The packed GUID and movement info of a movement packet, as 1.12 lays
/// them out, for the packets wow_messages doesn't have.
pub fn movement_bytes(guid: Guid, info: &MovementInfo) -> Vec<u8> {
    let frame = encode(&MSG_MOVE_HEARTBEAT_Server {
        guid,
        info: info.clone(),
    })
    .expect("a heartbeat encodes");
    crate::game::movement_wire::body_to_client(frame[4..].to_vec())
}

fn root_ack(map: &mut Map, g: Guid, info: MovementInfo, root: bool) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if p.movement.flags.get_root() == root {
        return;
    }
    p.movement = info.clone();
    let frame = if root {
        encode(&MSG_MOVE_ROOT_Server { player: g, info })
    } else {
        encode(&MSG_MOVE_UNROOT_Server { player: g, info })
    };
    if let Some(frame) = frame.map(crate::game::movement_wire::relay_to_client) {
        map.send_to_observers(g, &frame, false);
    }
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

/// A player's new speed, to those who see them: `MSG_MOVE_SET_*_SPEED`
/// with where they are (Thistle Tea's observer speed packets). Sent with
/// the player's own `SMSG_FORCE_*_SPEED_CHANGE`, once: the client's
/// acknowledgement isn't passed on, as Thistle Tea's isn't.
pub fn speed_to_observers(map: &Map, g: Guid, opcode: u16, speed: f32) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let mut b = Body(movement_bytes(g, &p.movement));
    b.0.extend_from_slice(&speed.to_le_bytes());
    let frame = b.frame(opcode);
    map.send_to_observers(g, &frame, false);
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
    let msg = crate::game::map::account_data::handle(map, g, msg)?;
    let msg = crate::game::map::trade_spells::handle(map, g, msg)?;
    let msg = crate::game::map::item_rules::handle(map, g, msg)?;
    match msg {
        ClientOpcodeMessage::CMSG_COMPLETE_CINEMATIC => {
            crate::game::map::cinematic::finish(map, g);
            None
        }
        ClientOpcodeMessage::CMSG_ZONEUPDATE(m) => {
            // Without a navmesh or terrain, the client's zone is the one explored.
            if map.nav.is_none() && map.terrain.is_none() {
                crate::game::map::exploration::entered(map, g, m.area.as_int());
            }
            Some(ClientOpcodeMessage::CMSG_ZONEUPDATE(m))
        }
        ClientOpcodeMessage::CMSG_SET_SELECTION(m) => {
            // Looked at here, then handled as before.
            crate::game::map::reputation::reveal_target(map, g, m.target);
            Some(ClientOpcodeMessage::CMSG_SET_SELECTION(m))
        }
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
        if opcode == crate::game::map::account_data::CMSG_UPDATE_ACCOUNT_DATA {
            crate::game::map::account_data::update_raw(self, guid, body);
            return;
        }
        if opcode == crate::game::map::tickets::CMSG_GMSURVEY_SUBMIT {
            crate::game::map::tickets::survey(self, guid, body);
            return;
        }
        tracing::trace!("raw opcode 0x{opcode:03X} not handled");
    }
}

/// The opcodes of this module.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        // A unit the player's client moves (Mind Control, Eyes of the
        // Beast): those who see it hear of its new speed once that client
        // has it, as mangos-classic tells them. Its own speed it hears at
        // the change.
        C::CMSG_FORCE_RUN_SPEED_CHANGE_ACK(m)
            if m.guid != g
                && map
                    .players
                    .get(&g)
                    .is_some_and(|p| p.owned.mover == Some(m.guid)) =>
        {
            relay(
                map,
                m.guid,
                m.info,
                MSG_MOVE_SET_RUN_SPEED,
                &m.new_speed.to_le_bytes(),
            )
        }
        // A root or an unroot taken: the client is where it says, and
        // those who see it are told, as mangos's HandleMoveRootAck tells
        // them; one for the state the client is already in is stale.
        C::CMSG_FORCE_MOVE_ROOT_ACK(m) if m.guid == g => root_ack(map, g, m.info, true),
        C::CMSG_FORCE_MOVE_UNROOT_ACK(m) if m.guid == g => root_ack(map, g, m.info, false),
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
        // Nothing to do, as mangos's HandleCancelGrowthAuraOpcode and
        // Thistle Tea's CmsgCancelGrowthAura have it: the player's scale
        // stays.
        C::CMSG_CANCEL_GROWTH_AURA => {}
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
    crate::game::map::cinematic::update(map);
    crate::game::map::environment::update(map);
    crate::game::map::item_rules::update(map);
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
    // Reincarnation off its cooldown (its own or its category's), with
    // its Ankh carried, and a self-resurrection of its own to offer
    // (Thistle Tea's "malformed" spell offers nothing).
    let reincarnation = p.spells.contains(&REINCARNATION_PASSIVE)
        && data.spells.get(REINCARNATION).is_some_and(|s| {
            s.has_effect(94) && {
                let cooling = p.caster.cooldowns.get(&s.id).is_some_and(|t| *t > now)
                    || (s.category != 0
                        && p.caster
                            .category_cooldowns
                            .get(&s.category)
                            .is_some_and(|t| *t > now));
                !cooling && has_reagents(p, s)
            }
        });
    let spell = stone.unwrap_or(if reincarnation { REINCARNATION } else { 0 });
    p.fields.set_u32(PLAYER_SELF_RES_SPELL, spell);
}

/// Whether `p` carries what `s` takes.
fn has_reagents(
    p: &crate::game::map::player::Player,
    s: &crate::game::data::spells::SpellTemplate,
) -> bool {
    s.reagents
        .iter()
        .all(|(r, n)| *r == 0 || p.carried_count_of(*r) >= *n)
}

/// `CMSG_SELF_RES`: dead and not released, back with the spell's share of
/// health and mana (with the caster's modifiers: Improved Reincarnation's),
/// full energy and no rage; the reagent taken and the cooldown started.
/// Without the reagent still carried, nothing (Thistle Tea's
/// `SelfResurrection`).
fn self_resurrect(map: &mut Map, g: Guid) {
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
    let Some(index) = s.effects.iter().position(|e| e.effect == 94) else {
        return;
    };
    if !has_reagents(p, s) {
        return;
    }
    let e = s.effects[index];
    let m = crate::game::map::spells::mods::for_spell(map, g, s);
    let points = m.points(s, index, e.base_points + e.base_dice.max(1));
    let (max_health, max_mana) = (
        p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
        p.fields.get_u32(UNIT_FIELD_MAXPOWER1),
    );
    let (health, mana) = if points < 0 {
        ((-points) as u32, e.misc_value.max(0) as u32)
    } else {
        (
            max_health * points as u32 / 100,
            max_mana * points as u32 / 100,
        )
    };
    // Reincarnation takes its Ankh.
    for (reagent, count) in s.reagents.iter().filter(|(r, _)| *r != 0) {
        let mut left = *count;
        while left > 0 {
            let Some(i) = map.players.get(&g).and_then(|p| {
                p.items
                    .iter()
                    .position(|it| it.entry == *reagent && it.carried())
            }) else {
                break;
            };
            let n = map.players[&g].items[i].count().min(left);
            crate::game::map::inventory::take_item(map, g, i, n);
            left -= n;
        }
    }
    crate::game::map::spells::cooldowns::start(map, g, s, &m, None);
    crate::game::map::death::resurrect_with(map, g, health, mana);
}

/// Evil Twin (the Shadowy Figure's curse): no summons reaches one under
/// it.
pub const EVIL_TWIN: u32 = 23_445;

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
        // Evil Twin's dummy turns summonses away unheard (mangos's
        // `EffectSummonPlayer`).
        if p.death != Death::Alive || p.auras.has_spell(EVIL_TWIN) {
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

/// Whether `npc` may reset `g`'s talents: a living trainer of their class
/// within reach, for a player of level ten or more (mangos's
/// `CanTrainAndResetTalentsOf`, Thistle Tea's `TalentReset.confirm`).
fn resets_talents(map: &Map, g: Guid, npc: Guid) -> bool {
    let Some(entry) =
        crate::game::map::npc::npc_entry(map, g, npc, crate::game::map::npc::NPC_FLAG_TRAINER)
    else {
        return false;
    };
    let (Some(p), Some(t)) = (map.players.get(&g), map.realm.data.creature_template(entry)) else {
        return false;
    };
    p.level() >= 10 && t.trainer_type == 0 && t.trainer_class == p.row.class
}

/// The trainer's offer to reset the player's talents (gossip option 16).
pub fn talent_wipe_offer(map: &mut Map, g: Guid, npc: Guid) {
    let now = chrono::Utc::now().timestamp();
    let resets = resets_talents(map, g, npc);
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if !resets {
        p.outbound.send(&SMSG_GOSSIP_COMPLETE {});
        return;
    }
    let cost = talent_wipe_cost(p.row.state.talent_wipes, p.row.state.last_talent_wipe, now);
    p.outbound.send(&SMSG_GOSSIP_COMPLETE {});
    p.outbound.send(&MSG_TALENT_WIPE_CONFIRM_Server {
        wiping_npc: npc,
        cost_in_copper: cost,
    });
}

/// The talent spells a player knows, and those they taught.
pub fn talent_spells(
    p: &crate::game::map::player::Player,
    data: &crate::game::data::GameData,
) -> Vec<u32> {
    // A talent's ranks, and those a trainer taught after them (Mortal
    // Strike's), as mangos's `resetTalents` takes every rank of a talent's
    // chain.
    let mut out: Vec<u32> = p
        .spells
        .iter()
        .copied()
        .filter(|s| {
            data.spells.talent_of(*s).is_some()
                && !data.spells.talents.values().any(|t| t.ranks.contains(s))
        })
        .collect();
    for t in data.spells.talents.values() {
        for r in t.ranks.iter().filter(|r| **r != 0 && p.spells.contains(r)) {
            out.push(*r);
            if let Some(s) = data.spells.get(*r) {
                for e in s
                    .effects
                    .iter()
                    .filter(|e| e.effect == 36 && e.trigger_spell != 0)
                {
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

/// `MSG_TALENT_WIPE_CONFIRM`: paid for, the talents are unlearned. Asked
/// of one who may not reset them, nothing; too little money is told as a
/// failed purchase, then as nothing reset, as mangos's
/// `HandleTalentWipeConfirmOpcode` answers, and an empty allocation as
/// nothing reset.
fn talent_wipe(map: &mut Map, g: Guid, npc: Guid) {
    let data = map.realm.data.clone();
    let now = chrono::Utc::now().timestamp();
    if !resets_talents(map, g, npc) {
        return;
    }
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let cost = talent_wipe_cost(p.row.state.talent_wipes, p.row.state.last_talent_wipe, now);
    let spells = talent_spells(p, &data);
    let poor = p.money() < cost;
    if spells.is_empty() || poor {
        if poor && !spells.is_empty() {
            p.outbound.send(&SMSG_BUY_FAILED {
                guid: Guid::zero(),
                item: 0,
                result: BuyResult::NotEnoughtMoney,
            });
        }
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

/// `SkillRaceClassInfo`'s flag: the skill may be unlearned.
const SKILL_FLAG_CAN_UNLEARN: u32 = 0x20;
/// The primary professions, unlearnable without the client's data.
const PRIMARY_PROFESSIONS: [u32; 9] = [164, 165, 171, 182, 186, 197, 202, 333, 393];

/// Whether a player of `race` and `class` may unlearn `skill`: its
/// `SkillRaceClassInfo` says so (mangos's `SKILL_FLAG_CAN_UNLEARN`,
/// Thistle Tea's `unlearnable?`), the primary professions alone.
pub fn unlearnable(data: &crate::game::data::GameData, skill: u32, race: u8, class: u8) -> bool {
    if data.spells.skill_race_class.is_empty() {
        return PRIMARY_PROFESSIONS.contains(&skill);
    }
    let (race_bit, class_bit) = (1u32 << (race.max(1) - 1), 1u32 << (class.max(1) - 1));
    data.spells
        .skill_race_class
        .get(&skill)
        .and_then(|rows| {
            rows.iter().find(|(races, classes, _)| {
                (*races == 0 || races & race_bit != 0)
                    && (*classes == 0 || classes & class_bit != 0)
            })
        })
        .is_some_and(|(_, _, flags)| flags & SKILL_FLAG_CAN_UNLEARN != 0)
}

/// `CMSG_UNLEARN_SKILL`: a profession (or other skill the player may drop)
/// and its spells are forgotten.
fn unlearn_skill(map: &mut Map, g: Guid, skill: u32) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if crate::game::map::skills::value(p, skill).is_none()
        || !unlearnable(&data, skill, p.row.race, p.row.class)
    {
        return;
    }
    let spells: Vec<u32> = p
        .spells
        .iter()
        .copied()
        .filter(|s| data.spells.spell_skills.get(s) == Some(&skill))
        .chain(p.spells.iter().copied().filter(|s| {
            data.spells.get(*s).is_some_and(|t| {
                t.effects.iter().any(|e| {
                    e.effect == crate::game::map::spells::EFFECT_SKILL
                        && e.misc_value as u32 == skill
                })
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
