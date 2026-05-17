use crate::world::database::WorldDatabase;
use crate::world::world_opcode_handler::character::Character;
use wow_world_base::vanilla::{Map, PlayerGender, RaceClass, Vector3d};
use wow_world_messages::vanilla::{CMSG_CHAR_CREATE, MovementInfo};

/// Gurubashi Arena on Eastern Kingdoms — chosen as the universal spawn
/// for every new character regardless of race. Pinned here so creation
/// stays deterministic and avoids race-specific starter-data edge cases.
/// Loadtest bots use the same anchor (see
/// `src/loadtest/worker/movement.rs::ANCHOR`) so server-side AOI clusters
/// match between real and synthetic clients.
const SPAWN_POSITION: (f32, f32, f32, f32) = (-13206.0, 272.0, 21.857, 0.0);

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

    // Override spawn to Gurubashi Arena for every new character regardless of
    // race. Keeps creation deterministic + dodges race-specific starter-data
    // edge cases until each race's path has been validated end-to-end.
    let (x, y, z, o) = SPAWN_POSITION;
    character.map = Map::EasternKingdoms;
    character.info = MovementInfo {
        flags: Default::default(),
        timestamp: 0,
        position: Vector3d { x, y, z },
        orientation: o,
        fall_time: 0.0,
    };

    tracing::info!(
        "CHAR_CREATE: name={} race_class={:?} gender={:?} guid={:?} -> Gurubashi Arena",
        c.name,
        race_class,
        gender,
        character.guid,
    );

    Some(character)
}
