//! Entering the world, and leaving it.

use std::time::Duration;

use tracing::debug;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Area, DateTime, InitialSpell, LogoutResult, LogoutSpeed, SMSG_ACTION_BUTTONS,
    SMSG_BINDPOINTUPDATE, SMSG_INIT_WORLD_STATES, SMSG_INITIAL_SPELLS, SMSG_LOGIN_SETTIMESPEED,
    SMSG_LOGIN_VERIFY_WORLD, SMSG_LOGOUT_CANCEL_ACK, SMSG_LOGOUT_RESPONSE, SMSG_TUTORIAL_FLAGS,
    Vector3d,
};

use crate::game::fields::*;
use crate::game::map::movement::map_enum;
use crate::game::map::player::{Player, STAND_STATE_SIT, STAND_STATE_STAND, UNIT_FLAG_STUNNED};
use crate::game::map::{Map, chat};
use crate::game::session::Control;

/// How long a logout takes outside an inn.
pub const LOGOUT_DELAY: Duration = Duration::from_secs(20);

/// The packets a client gets on entering the world, up to its own create
/// block. Everything around it follows at the next update.
pub fn send_login(map: &mut Map, p: &mut Player) {
    // The helm and cloak as they were left.
    use crate::game::map::player::{PLAYER_FLAGS_HIDE_CLOAK, PLAYER_FLAGS_HIDE_HELM};
    for flag in [PLAYER_FLAGS_HIDE_HELM, PLAYER_FLAGS_HIDE_CLOAK] {
        let on = p.row.state.flags & flag != 0;
        p.fields.set_flag(PLAYER_FLAGS, flag, on);
    }
    let out = &p.outbound;
    out.send(&SMSG_LOGIN_VERIFY_WORLD {
        map: map_enum(map.key.map),
        position: p.position(),
        orientation: p.movement.orientation,
    });
    out.send_frame(crate::game::map::account_data::digests_frame(
        &p.row.state.caches,
    ));
    let bind = p.row.state.bind.unwrap_or(crate::game::store::Bind {
        map: p.row.map,
        zone: p.row.zone,
        x: p.row.x,
        y: p.row.y,
        z: p.row.z,
    });
    // mangos sends it, and Thistle Tea after it.
    out.send(&wow_world_messages::vanilla::SMSG_SET_REST_START { unknown1: 0 });
    out.send(&SMSG_BINDPOINTUPDATE {
        position: Vector3d {
            x: bind.x,
            y: bind.y,
            z: bind.z,
        },
        map: map_enum(bind.map),
        area: Area::try_from(bind.zone).unwrap_or_default(),
    });
    out.send(&SMSG_TUTORIAL_FLAGS {
        tutorial_data: p.row.state.tutorials,
    });
    out.send(&SMSG_INITIAL_SPELLS {
        unknown1: 0,
        initial_spells: p
            .spells
            .iter()
            .filter_map(|s| u16::try_from(*s).ok())
            .map(|spell_id| InitialSpell {
                spell_id,
                unknown1: 0,
            })
            .collect(),
        cooldowns: Vec::new(),
    });
    let mut buttons = [0u32; 120];
    for a in &p.row.actions {
        if let Some(b) = buttons.get_mut(usize::from(a.button)) {
            *b = u32::from(a.action) | (u32::from(a.kind) << 24);
        }
    }
    out.send(&SMSG_ACTION_BUTTONS { data: buttons });
    crate::game::map::reputation::send_login(p, &map.realm.data);
    crate::game::map::rest::restore(p, map.now, &map.cfg.rates);
    crate::game::map::intoxication::restore(p);
    crate::game::map::exploration::init(p);
    let out = &p.outbound;
    out.send(&SMSG_LOGIN_SETTIMESPEED {
        datetime: game_time(),
        timescale: 1.0 / 60.0,
    });
    out.send(&SMSG_INIT_WORLD_STATES {
        map: map_enum(map.key.map),
        area: Area::try_from(p.zone).unwrap_or_default(),
        states: crate::game::map::world_states::states_for(map, p.zone),
    });
    let guid = p.guid;
    // The player's own create block, and their items'.
    {
        let mut batch = crate::game::update::Batch::new();
        for item in &p.items {
            batch.push(&crate::game::update::create_block(
                &item.create(),
                Visibility::Owner,
            ));
        }
        batch.push(&crate::game::update::create_block(
            &p.create(true),
            Visibility::Owner,
        ));
        for frame in batch.finish() {
            p.outbound.send_frame(frame);
        }
        p.fields.clear_dirty();
        for item in &mut p.items {
            item.fields.clear_dirty();
        }
    }
    if p.row.at_login & crate::game::store::AT_LOGIN_FIRST != 0
        && let Some(c) = map.realm.data.races.get(&p.row.race)
        && let Ok(id) = wow_world_messages::vanilla::CinematicSequenceId::try_from(c.cinematic)
    {
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_TRIGGER_CINEMATIC {
                cinematic_sequence_id: id,
            });
        let sequence = c.cinematic;
        crate::game::map::cinematic::begin(map, p, sequence);
    }
    // Temporary enchantments' time left (Thistle Tea's
    // `send_active_timers`).
    for item in &p.items {
        let e = crate::game::map::enchant::get(item, crate::game::map::enchant::TEMP_SLOT);
        if e.id != 0 && e.duration_ms > 0 {
            p.outbound.send(
                &wow_world_messages::vanilla::SMSG_ITEM_ENCHANT_TIME_UPDATE {
                    item: item.guid,
                    slot: crate::game::map::enchant::TEMP_SLOT as u32,
                    duration: e.duration_ms / 1000,
                    player: guid,
                },
            );
        }
    }
    let motd = &map.cfg.motd;
    if !motd.is_empty() {
        chat::send_system(&p.outbound, motd);
    }
    p.row.at_login &= !crate::game::store::AT_LOGIN_FIRST;
    debug!("{} ({guid}) entered {}", p.name, map.key);
}

/// The in-game clock: the host's local time.
pub fn game_time() -> DateTime {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    DateTime::new(
        u8::try_from(now.year() - 2000).unwrap_or(0),
        u8::try_from(now.month0())
            .ok()
            .and_then(|m| m.try_into().ok())
            .unwrap_or_default(),
        u8::try_from(now.day0()).unwrap_or(0),
        u8::try_from(now.weekday().num_days_from_sunday())
            .ok()
            .and_then(|d| d.try_into().ok())
            .unwrap_or_default(),
        u8::try_from(now.hour()).unwrap_or(0),
        u8::try_from(now.minute()).unwrap_or(0),
    )
}

/// GM Freeze: no logout under it.
const SPELL_GM_FREEZE: u32 = 9454;
/// `PLAYER_FIELD_BYTES`' first byte: logging out.
const FIELD_BYTES_LOGGING_OUT: u8 = 0x04;

/// `CMSG_LOGOUT_REQUEST`, as Thistle Tea's `Logout`: refused in combat,
/// in the air or frozen by a GM; at once resting or on a flight; else
/// after [`LOGOUT_DELAY`] (a GM's too), sat down and rooted if free to
/// move. A trade and a loot window close first.
pub fn request_logout(map: &mut Map, guid: Guid) {
    let now = map.now;
    let Some(p) = map.players.get(&guid) else {
        return;
    };
    // A request during a countdown starts it again, or, refused, ends it
    // (mangos's `HandleLogoutRequestOpcode`, Thistle Tea's `Logout.request`).
    let pending = p.logout_at.is_some();
    if let Some(target) = p.looting {
        crate::game::map::loot::release(map, guid, target);
    }
    crate::game::map::trade::cancel(
        map,
        guid,
        wow_world_messages::vanilla::SMSG_TRADE_STATUS_TradeStatus::TradeCanceled,
    );
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    let airborne = p.movement.flags.get_jumping().is_some() || p.movement.flags.get_fallingfar();
    let refused = if p.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    ) {
        Some(LogoutResult::FailureInCombat)
    } else if airborne {
        Some(LogoutResult::FailureJumpingOrFalling)
    } else if p.auras.has_spell(SPELL_GM_FREEZE) {
        Some(LogoutResult::FailureFrozenByGm)
    } else {
        None
    };
    if let Some(result) = refused {
        if p.logout_at.take().is_some() {
            undo_logout(p);
        }
        p.outbound.send(&SMSG_LOGOUT_RESPONSE {
            result,
            speed: LogoutSpeed::Delayed,
        });
        return;
    }
    let delay = map.cfg.logout_delay;
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    let instant = p.flight.is_some()
        || p.fields
            .has_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_RESTING);
    p.outbound.send(&SMSG_LOGOUT_RESPONSE {
        result: LogoutResult::Success,
        speed: if instant {
            LogoutSpeed::Instant
        } else {
            LogoutSpeed::Delayed
        },
    });
    if instant {
        p.logout_at = Some(now);
        return;
    }
    p.logout_at = Some(now + delay);
    if pending {
        return;
    }
    use crate::game::map::auras::{AURA_MOD_CONFUSE, AURA_MOD_FEAR, AURA_MOD_ROOT, AURA_MOD_STUN};
    let held = [
        AURA_MOD_ROOT,
        AURA_MOD_STUN,
        AURA_MOD_FEAR,
        AURA_MOD_CONFUSE,
    ]
    .iter()
    .any(|a| p.auras.has(*a));
    let free = !held && p.death == crate::game::map::death::Death::Alive;
    let flags = p.fields.get_byte(PLAYER_FIELD_BYTES, 0);
    p.fields
        .set_byte(PLAYER_FIELD_BYTES, 0, flags | FIELD_BYTES_LOGGING_OUT);
    p.misc.logout_rooted = free;
    if !free {
        return;
    }
    let mounted = p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0;
    let swimming = p.movement.flags.get_swimming().is_some();
    if p.stand_state() == STAND_STATE_STAND && !mounted && !swimming {
        p.set_stand_state(STAND_STATE_SIT);
    }
    p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_STUNNED, true);
    p.outbound
        .send_frame(crate::game::map::movement::root_frame(guid, true));
}

/// The logout's posture and root undone, as far as it made them.
fn undo_logout(p: &mut crate::game::map::player::Player) {
    let flags = p.fields.get_byte(PLAYER_FIELD_BYTES, 0);
    p.fields
        .set_byte(PLAYER_FIELD_BYTES, 0, flags & !FIELD_BYTES_LOGGING_OUT);
    if !std::mem::take(&mut p.misc.logout_rooted) {
        return;
    }
    if p.stand_state() == STAND_STATE_SIT && p.death == crate::game::map::death::Death::Alive {
        p.set_stand_state(STAND_STATE_STAND);
    }
    // A stun or root come since stays (Thistle Tea's `MovementSync`, after
    // the logout's own goes).
    use crate::game::map::auras::{AURA_MOD_ROOT, AURA_MOD_STUN};
    let stunned = p.auras.has(AURA_MOD_STUN);
    if !stunned {
        p.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_STUNNED, false);
    }
    if !stunned && !p.auras.has(AURA_MOD_ROOT) {
        p.outbound
            .send_frame(crate::game::map::movement::root_frame(p.guid, false));
    }
}

/// `CMSG_LOGOUT_CANCEL`.
pub fn cancel_logout(map: &mut Map, guid: Guid) {
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    if p.logout_at.take().is_none() {
        return;
    }
    p.outbound.send(&SMSG_LOGOUT_CANCEL_ACK {});
    undo_logout(p);
}

/// The logout is due: save the character and hand the session back to the
/// character screen, which tells the client.
pub fn complete_logout(map: &mut Map, guid: Guid) {
    let Some(mut p) = map.take_player(guid) else {
        return;
    };
    p.logout_at = None;
    undo_logout(&mut p);
    let session = p.online.session.clone();
    map.save_and_go_offline(p);
    let _ = session.send(Control::LoggedOut);
}
