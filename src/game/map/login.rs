//! Entering the world, and leaving it.

use std::time::Duration;

use tracing::debug;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    Area, DateTime, FactionFlag, FactionInitializer, InitialSpell, LogoutResult, LogoutSpeed,
    SMSG_ACCOUNT_DATA_TIMES, SMSG_ACTION_BUTTONS, SMSG_BINDPOINTUPDATE, SMSG_FORCE_MOVE_ROOT,
    SMSG_FORCE_MOVE_UNROOT, SMSG_INIT_WORLD_STATES, SMSG_INITIAL_SPELLS, SMSG_INITIALIZE_FACTIONS,
    SMSG_LOGIN_SETTIMESPEED, SMSG_LOGIN_VERIFY_WORLD, SMSG_LOGOUT_CANCEL_ACK, SMSG_LOGOUT_RESPONSE,
    SMSG_TUTORIAL_FLAGS, Vector3d,
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
    let out = &p.outbound;
    out.send(&SMSG_LOGIN_VERIFY_WORLD {
        map: map_enum(map.key.map),
        position: p.position(),
        orientation: p.movement.orientation,
    });
    out.send(&SMSG_ACCOUNT_DATA_TIMES { data: [0; 32] });
    let bind = p.row.state.bind.unwrap_or(crate::game::store::Bind {
        map: p.row.map,
        zone: p.row.zone,
        x: p.row.x,
        y: p.row.y,
        z: p.row.z,
    });
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
    out.send(&SMSG_INITIALIZE_FACTIONS {
        factions: (0..64)
            .map(|_| FactionInitializer {
                flag: FactionFlag::empty(),
                standing: 0,
            })
            .collect(),
    });
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
    }
    let motd = &map.realm.config.motd;
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

/// `CMSG_LOGOUT_REQUEST`: sit, root, and leave after [`LOGOUT_DELAY`];
/// at once for a GM.
pub fn request_logout(map: &mut Map, guid: Guid) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    if p.logout_at.is_some() {
        return;
    }
    if p.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    ) {
        p.outbound.send(&SMSG_LOGOUT_RESPONSE {
            result: LogoutResult::FailureInCombat,
            speed: LogoutSpeed::Delayed,
        });
        return;
    }
    let instant = p.gm
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
    p.logout_at = Some(now + LOGOUT_DELAY);
    p.set_stand_state(STAND_STATE_SIT);
    p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_STUNNED, true);
    p.outbound.send(&SMSG_FORCE_MOVE_ROOT { guid, counter: 0 });
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
    p.set_stand_state(STAND_STATE_STAND);
    p.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_STUNNED, false);
    p.outbound
        .send(&SMSG_FORCE_MOVE_UNROOT { guid, counter: 0 });
}

/// The logout is due: save the character and hand the session back to the
/// character screen, which tells the client.
pub fn complete_logout(map: &mut Map, guid: Guid) {
    let Some(mut p) = map.take_player(guid) else {
        return;
    };
    p.logout_at = None;
    p.set_stand_state(STAND_STATE_STAND);
    p.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_STUNNED, false);
    let session = p.online.session.clone();
    map.save_and_go_offline(p);
    let _ = session.send(Control::LoggedOut);
}
