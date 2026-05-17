use crate::world::database::WorldDatabase;
use crate::world::world_opcode_handler::character::Character;
use wow_world_base::vanilla::{PlayerGender, RaceClass, Vector3d};
use wow_world_messages::vanilla::{CMSG_CHAR_CREATE, MovementInfo};

// Universal spawn point lives in `[spawn]` of `config.toml` (default:
// Gurubashi Arena on Eastern Kingdoms). Loadtest bots use the same
// anchor (see `src/loadtest/worker/movement.rs::ANCHOR`) so AOI
// clusters match between real and synthetic clients — change both if
// you tune the spawn.

pub(crate) fn create_character(c: CMSG_CHAR_CREATE, db: &mut WorldDatabase) -> Option<Character> {
    let race_class = match RaceClass::try_from((c.race, c.class)) {
        Ok(rc) => rc,
        Err(e) => {
            tracing::warn!(
                "CHAR_CREATE rejected: invalid race/class {:?}/{:?}: {e:?}",
                c.race,
                c.class,
            );
            return None;
        }
    };
    let gender = match PlayerGender::try_from(c.gender) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!("CHAR_CREATE rejected: invalid gender {:?}: {e:?}", c.gender);
            return None;
        }
    };

    let mut character = Character::new(
        db,
        c.name.clone(),
        race_class,
        gender,
        c.skin_color,
        c.face,
        c.hair_style,
        c.hair_color,
        c.facial_hair,
    );

    // Override spawn for every new character regardless of race so creation
    // stays deterministic and dodges race-specific starter-data edge cases.
    // Coordinates come from `[spawn]` in `config.toml`.
    let spawn = &crate::config::config().spawn;
    character.map = spawn.map;
    character.info = MovementInfo {
        flags: Default::default(),
        timestamp: 0,
        position: Vector3d {
            x: spawn.x,
            y: spawn.y,
            z: spawn.z,
        },
        orientation: spawn.orientation,
        fall_time: 0.0,
    };

    tracing::info!(
        "CHAR_CREATE: name={} race_class={:?} gender={:?} guid={:?} -> {:?} ({}, {}, {})",
        c.name,
        race_class,
        gender,
        character.guid,
        spawn.map,
        spawn.x,
        spawn.y,
        spawn.z,
    );

    Some(character)
}
