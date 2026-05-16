use crate::world::database::WorldDatabase;
use crate::world::world_opcode_handler::character::Character;
use wow_world_base::vanilla::{Map, PlayerGender, RaceClass, Vector3d};
use wow_world_messages::vanilla::{CMSG_CHAR_CREATE, MovementInfo};

/// Northshire Abbey on Eastern Kingdoms — the canonical Human starting
/// position. Pinned here so every new character lands somewhere we know
/// works, regardless of race. Avoids exercising race-specific starter data
/// paths until those are validated.
const NORTHSHIRE_ABBEY: (f32, f32, f32, f32) = (-8949.95, -132.493, 83.5312, 0.0);

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

    // Override spawn to Northshire Abbey for every new character regardless of
    // race. Keeps creation deterministic + dodges race-specific starter-data
    // edge cases until each race's path has been validated end-to-end.
    let (x, y, z, o) = NORTHSHIRE_ABBEY;
    character.map = Map::EasternKingdoms;
    character.info = MovementInfo {
        flags: Default::default(),
        timestamp: 0,
        position: Vector3d { x, y, z },
        orientation: o,
        fall_time: 0.0,
    };

    tracing::info!(
        "CHAR_CREATE: name={} race_class={:?} gender={:?} guid={:?} -> Northshire Abbey",
        c.name,
        race_class,
        gender,
        character.guid,
    );

    Some(character)
}
