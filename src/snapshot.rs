use crate::world::database::WorldDatabase;
use crate::world::world_opcode_handler::character::Character;
use crate::world::world_opcode_handler::creature::Creature;
use crate::world::world_opcode_handler::inventory::Inventory;
use crate::world::world_opcode_handler::item::Item;
use serde::{Deserialize, Serialize};
use slab::Slab;
use std::path::Path;
use wow_items::vanilla::lookup_item;
use wow_world_base::movement::DEFAULT_RUNNING_SPEED;
use wow_world_base::vanilla::{Area, Level, Map, PlayerGender, RaceClass, Vector3d};
use wow_world_messages::vanilla::MovementInfo;
use wow_world_messages::Guid;

pub const SNAPSHOT_PATH: &str = "snapshot.bin";

#[derive(Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub next_guid: u64,
    pub characters: Vec<CharacterSnapshot>,
    pub creatures: Vec<CreatureSnapshot>,
}

#[derive(Serialize, Deserialize)]
pub struct CharacterSnapshot {
    pub guid: u64,
    pub name: String,
    /// Account that owns this character. Older snapshots predate the field
    /// (everything was effectively global), so missing values default to
    /// `DEV` — that's where `Dev`/`HumOne`/`HumTwo` originally lived.
    #[serde(default = "default_legacy_account")]
    pub account: String,
    pub race_class: RaceClass,
    pub gender: PlayerGender,
    pub skin: u8,
    pub face: u8,
    pub hairstyle: u8,
    pub haircolor: u8,
    pub facialhair: u8,
    pub level: Level,
    pub area: Area,
    pub map: Map,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    pub movement_speed: f32,
    pub inventory: Vec<InventoryEntry>,
}

fn default_legacy_account() -> String {
    crate::world::database::DEV_ACCOUNT.to_string()
}

#[derive(Serialize, Deserialize)]
pub struct InventoryEntry {
    pub slot: u8,
    pub item_entry: u32,
    pub item_guid: u64,
    pub amount: u8,
    pub creator_guid: u64,
}

#[derive(Serialize, Deserialize)]
pub struct CreatureSnapshot {
    pub guid: u64,
    pub name: String,
    pub map: Map,
    pub level: u8,
    pub display_id: u16,
    pub entry: u32,
    pub faction_template: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    #[serde(default)]
    pub health: u32,
    #[serde(default)]
    pub max_health: u32,
}

impl WorldSnapshot {
    pub fn capture(db: &WorldDatabase, creatures: &Slab<Creature>) -> Self {
        Self {
            next_guid: db.next_guid(),
            characters: db.all_characters().iter().map(CharacterSnapshot::from).collect(),
            creatures: creatures.iter().map(|(_, c)| CreatureSnapshot::from(c)).collect(),
        }
    }

    pub fn restore_db_only(self) -> WorldDatabase {
        let characters = self.characters.into_iter().map(|s| s.into_character()).collect();
        WorldDatabase::from_parts(characters, self.next_guid)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let tmp = path.as_ref().with_extension("bin.tmp");
        let bytes = postcard::to_allocvec(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let snap: WorldSnapshot = postcard::from_bytes(&bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Some(snap))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl From<&Character> for CharacterSnapshot {
    fn from(c: &Character) -> Self {
        let inventory = c
            .inventory
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                slot.as_ref().map(|item| InventoryEntry {
                    slot: i as u8,
                    item_entry: item.item.entry(),
                    item_guid: item.guid.guid(),
                    amount: item.amount,
                    creator_guid: item.creator.guid(),
                })
            })
            .collect();

        Self {
            guid: c.guid.guid(),
            name: c.name.clone(),
            account: c.account.clone(),
            race_class: c.race_class,
            gender: c.gender,
            skin: c.skin,
            face: c.face,
            hairstyle: c.hairstyle,
            haircolor: c.haircolor,
            facialhair: c.facialhair,
            level: c.level,
            area: c.area,
            map: c.map,
            x: c.info.position.x,
            y: c.info.position.y,
            z: c.info.position.z,
            orientation: c.info.orientation,
            movement_speed: c.movement_speed,
            inventory,
        }
    }
}

impl CharacterSnapshot {
    pub fn into_character(self) -> Character {
        let mut slots: [Option<Item>; Inventory::SLOT_COUNT] =
            [(); Inventory::SLOT_COUNT].map(|()| None);
        for entry in self.inventory {
            if let Some(item_static) = lookup_item(entry.item_entry) {
                slots[entry.slot as usize] = Some(Item {
                    item: item_static,
                    guid: Guid::new(entry.item_guid),
                    amount: entry.amount,
                    creator: Guid::new(entry.creator_guid),
                });
            }
        }

        Character {
            guid: Guid::new(self.guid),
            name: self.name,
            account: self.account,
            race_class: self.race_class,
            gender: self.gender,
            skin: self.skin,
            face: self.face,
            hairstyle: self.hairstyle,
            haircolor: self.haircolor,
            facialhair: self.facialhair,
            level: self.level,
            area: self.area,
            map: self.map,
            info: MovementInfo {
                flags: Default::default(),
                timestamp: 0,
                position: Vector3d {
                    x: self.x,
                    y: self.y,
                    z: self.z,
                },
                orientation: self.orientation,
                fall_time: 0.0,
            },
            movement_speed: if self.movement_speed > 0.0 {
                self.movement_speed
            } else {
                DEFAULT_RUNNING_SPEED
            },
            target: Guid::zero(),
            attacking: false,
            auto_attack_timer: 0.0,
            inventory: Inventory { slots },
            root_until: None,
        }
    }
}

impl From<&Creature> for CreatureSnapshot {
    fn from(c: &Creature) -> Self {
        Self {
            guid: c.guid.guid(),
            name: c.name.clone(),
            map: c.map,
            level: c.level,
            display_id: c.display_id,
            entry: c.entry,
            faction_template: c.faction_template,
            x: c.info.position.x,
            y: c.info.position.y,
            z: c.info.position.z,
            orientation: c.info.orientation,
            health: c.health,
            max_health: c.max_health,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snapshot deserialization should accept payloads that omit fields we've
    /// since added (via `#[serde(default)]`). This guards against the
    /// inevitable "I added a field and now nobody can load their save"
    /// incident — anyone adding fields to a *Snapshot must keep the existing
    /// shape decodable. Increment the test as fields are added.
    #[test]
    fn creature_snapshot_round_trip_preserves_fields() {
        let original = CreatureSnapshot {
            guid: 0x1000_0042,
            name: "Stormwind Guard".to_string(),
            map: Map::EasternKingdoms,
            level: 60,
            display_id: 1742,
            entry: 68,
            faction_template: 12,
            x: -8927.76,
            y: 481.33,
            z: 93.9432,
            orientation: 1.71042,
            health: 4500,
            max_health: 5000,
        };
        let bytes = postcard::to_allocvec(&original).expect("serialize");
        let decoded: CreatureSnapshot = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded.guid, original.guid);
        assert_eq!(decoded.name, original.name);
        assert_eq!(decoded.level, original.level);
        assert_eq!(decoded.display_id, original.display_id);
        assert_eq!(decoded.entry, original.entry);
        assert_eq!(decoded.faction_template, original.faction_template);
        assert_eq!(decoded.health, original.health);
        assert_eq!(decoded.max_health, original.max_health);
        assert!((decoded.x - original.x).abs() < 1e-3);
        assert!((decoded.y - original.y).abs() < 1e-3);
        assert!((decoded.z - original.z).abs() < 1e-3);
    }

    #[test]
    fn world_snapshot_round_trip_empty_state() {
        let snap = WorldSnapshot {
            next_guid: 42,
            characters: vec![],
            creatures: vec![],
        };
        let bytes = postcard::to_allocvec(&snap).expect("serialize");
        let decoded: WorldSnapshot = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded.next_guid, 42);
        assert_eq!(decoded.characters.len(), 0);
        assert_eq!(decoded.creatures.len(), 0);
    }

    #[test]
    fn world_snapshot_save_load_round_trip() {
        let snap = WorldSnapshot {
            next_guid: 123,
            characters: vec![],
            creatures: vec![],
        };
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "wow_vanilla_server_snapshot_test_{}.bin",
            std::process::id()
        ));
        snap.save(&path).expect("save");
        let loaded = WorldSnapshot::load(&path).expect("load").expect("Some");
        assert_eq!(loaded.next_guid, 123);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn world_snapshot_load_missing_returns_none() {
        let path = std::env::temp_dir().join("wow_vanilla_server_definitely_missing.bin");
        let _ = std::fs::remove_file(&path);
        let result = WorldSnapshot::load(&path).expect("load");
        assert!(result.is_none());
    }
}

