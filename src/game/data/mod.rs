//! The game data: templates and spawns from vmangos's database, and tables
//! from the client's DBCs, loaded once and shared, read-only, by every
//! map.
//!
//! Without the databases (in tests, or a bare checkout) the server runs on
//! what `wow_world_base` and `wow_items` carry: races, classes, starting
//! positions and gear, and items.

pub mod items;
pub mod load;
pub mod npc;
pub mod pets;
pub mod spells;

use ahash::AHashMap;
use wow_world_base::vanilla::{Class, Gender, PlayerGender, PlayerRace, Race, RaceClass};

pub use items::ItemTemplate;

/// The patch the server runs: 1.12, vmangos's patch 10.
pub const PATCH: i64 = 10;

/// A creature's template.
#[derive(Debug, Clone, Default)]
pub struct CreatureTemplate {
    pub entry: u32,
    pub name: String,
    pub sub_name: String,
    pub display_ids: [u32; 4],
    pub display_scales: [f32; 4],
    pub display_probabilities: [u32; 4],
    pub mount_display_id: u32,
    pub level_min: u8,
    pub level_max: u8,
    pub faction: u32,
    pub npc_flags: u32,
    pub gossip_menu_id: u32,
    pub unit_flags: u32,
    pub dynamic_flags: u32,
    pub creature_type: u32,
    pub family: u32,
    pub rank: u32,
    pub type_flags: u32,
    pub unit_class: u8,
    pub speed_walk: f32,
    pub speed_run: f32,
    pub detection_range: f32,
    pub call_for_help_range: f32,
    pub leash_range: f32,
    pub health_multiplier: f32,
    pub mana_multiplier: f32,
    pub armor_multiplier: f32,
    pub damage_multiplier: f32,
    pub damage_variance: f32,
    pub damage_school: u8,
    pub base_attack_time: u32,
    pub ranged_attack_time: u32,
    pub resistances: [i32; 6],
    pub xp_multiplier: f32,
    pub loot_id: u32,
    pub pickpocket_loot_id: u32,
    pub skinning_loot_id: u32,
    pub gold_min: u32,
    pub gold_max: u32,
    pub spell_list_id: u32,
    pub movement_type: u8,
    pub inhabit_type: u8,
    pub civilian: bool,
    pub racial_leader: bool,
    pub equipment_id: u32,
    pub trainer_id: u32,
    pub vendor_id: u32,
    pub trainer_type: u8,
    pub trainer_spell: u32,
    pub trainer_class: u8,
    pub trainer_race: u8,
    pub static_flags1: u32,
    pub static_flags2: u32,
    pub flags_extra: u32,
    pub auras: Vec<u32>,
    pub ai_name: String,
    pub script_name: String,
    /// Health at the bottom and top of the level range, from the class
    /// level stats and the multiplier; 0 if there are no stats.
    pub health_min: u32,
    pub health_max: u32,
}

/// A creature spawn.
#[derive(Debug, Clone, Default)]
pub struct CreatureSpawn {
    pub guid: u32,
    /// The entries it may be, one picked at each spawn.
    pub entries: Vec<u32>,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    pub respawn_min: u32,
    pub respawn_max: u32,
    pub wander_distance: f32,
    pub health_percent: f32,
    pub movement_type: u8,
    pub spawn_flags: u32,
}

/// Extra state a spawn starts with.
#[derive(Debug, Clone, Default)]
pub struct CreatureAddon {
    pub display_id: u32,
    pub mount_display_id: u32,
    pub equipment_id: i32,
    pub stand_state: u8,
    pub sheath_state: u8,
    pub emote_state: u32,
    pub auras: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Waypoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    pub wait_ms: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ClassLevelStats {
    pub health: u32,
    pub mana: u32,
    pub melee_damage: f32,
    pub ranged_damage: f32,
    pub attack_power: u32,
    pub ranged_attack_power: u32,
    pub armor: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DisplayInfo {
    pub scale: f32,
    pub bounding_radius: f32,
    pub combat_reach: f32,
}

#[derive(Debug, Clone, Default)]
pub struct GameObjectTemplate {
    pub entry: u32,
    pub kind: u8,
    pub display_id: u32,
    pub name: String,
    pub icon: String,
    pub faction: u32,
    pub flags: u32,
    pub size: f32,
    pub data: [u32; 24],
    pub gold_min: u32,
    pub gold_max: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GameObjectSpawn {
    pub guid: u32,
    pub entry: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    pub rotation: [f32; 4],
    pub respawn_min: u32,
    pub respawn_max: u32,
    pub anim_progress: u8,
    pub state: u8,
    pub spawn_flags: u32,
}

/// An area or zone, from `AreaTable`.
#[derive(Debug, Clone, Default)]
pub struct AreaInfo {
    pub id: u32,
    pub map: u32,
    /// The zone it is in; 0 for a zone.
    pub zone: u32,
    pub explore_bit: u32,
    pub flags: u32,
    pub level: u32,
    pub name: String,
    pub team: u32,
}

/// An area's flags.
pub const AREA_FLAG_ARENA: u32 = 0x80;
pub const AREA_FLAG_CAPITAL: u32 = 0x100;
pub const AREA_FLAG_CITY: u32 = 0x200;
pub const AREA_FLAG_SANCTUARY: u32 = 0x800;

/// An area trigger: where it is and, if it teleports, where to.
#[derive(Debug, Clone, Default)]
pub struct AreaTrigger {
    pub id: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub radius: f32,
    pub box_length: f32,
    pub box_width: f32,
    pub box_height: f32,
    pub box_yaw: f32,
    pub teleport: Option<Teleport>,
}

#[derive(Debug, Clone, Default)]
pub struct Teleport {
    pub required_level: u8,
    pub message: String,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
}

#[derive(Debug, Clone, Default)]
pub struct MapInfo {
    pub id: u32,
    pub internal_name: String,
    pub name: String,
    pub instance_type: u32,
    pub player_limit: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Tele {
    pub name: String,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
}

/// What a race and class start with.
#[derive(Debug, Clone, Default)]
pub struct StartInfo {
    pub map: u32,
    pub zone: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    /// `(item entry, slot, count)`; slot `u8::MAX` for the item's
    /// equipment slot, or the first free bag slot.
    pub items: Vec<(u32, u8, u32)>,
    pub spells: Vec<u32>,
    /// `(button, action, type)`.
    pub actions: Vec<(u8, u16, u8)>,
}

/// A graveyard, and the zone whose dead it takes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Graveyard {
    pub id: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    pub zone: u32,
    /// 469 Alliance, 67 Horde, 0 both.
    pub faction: u32,
}

/// A faction template, from `FactionTemplate`: who a unit of it is friends
/// and enemies with.
#[derive(Debug, Clone, Copy, Default)]
pub struct FactionTemplate {
    pub id: u32,
    pub faction: u32,
    pub flags: u32,
    /// The groups it is in, it is friends with, and it is enemies with.
    pub ours: u32,
    pub friend_mask: u32,
    pub enemy_mask: u32,
    pub enemies: [u32; 4],
    pub friends: [u32; 4],
}

impl FactionTemplate {
    /// Whether a unit of this template is hostile to one of `other`.
    pub fn hostile_to(&self, other: &FactionTemplate) -> bool {
        if self.id == other.id {
            return false;
        }
        if other.faction != 0 {
            if self.enemies.contains(&other.faction) {
                return true;
            }
            if self.friends.contains(&other.faction) {
                return false;
            }
        }
        self.enemy_mask & other.ours != 0
    }

    pub fn friendly_to(&self, other: &FactionTemplate) -> bool {
        if self.id == other.id {
            return true;
        }
        if other.faction != 0 {
            if self.enemies.contains(&other.faction) {
                return false;
            }
            if self.friends.contains(&other.faction) {
                return true;
            }
        }
        self.friend_mask & other.ours != 0 || self.ours & other.friend_mask != 0
    }
}

/// A loot table's row.
#[derive(Debug, Clone, Copy, Default)]
pub struct LootEntry {
    pub item: u32,
    /// Percent; negative for a quest item, dropped only for its quest.
    pub chance: f32,
    pub group: u8,
    /// Negative: a reference to another table, rolled `max` times.
    pub min: i32,
    pub max: u32,
    pub condition: u32,
}

#[derive(Debug, Default)]
pub struct LootTables {
    pub creature: AHashMap<u32, Vec<LootEntry>>,
    pub gameobject: AHashMap<u32, Vec<LootEntry>>,
    pub reference: AHashMap<u32, Vec<LootEntry>>,
    pub pickpocket: AHashMap<u32, Vec<LootEntry>>,
    pub skinning: AHashMap<u32, Vec<LootEntry>>,
    pub fishing: AHashMap<u32, Vec<LootEntry>>,
    pub item: AHashMap<u32, Vec<LootEntry>>,
    pub disenchant: AHashMap<u32, Vec<LootEntry>>,
}

/// An item a vendor sells.
#[derive(Debug, Clone, Copy, Default)]
pub struct VendorItem {
    pub slot: u32,
    pub item: u32,
    /// 0 for unlimited.
    pub max_count: u32,
    pub restock_secs: u32,
    pub condition: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RaceInfo {
    pub male_display: u32,
    pub female_display: u32,
    pub faction: u32,
    pub cinematic: u32,
}

#[derive(Debug, Default)]
pub struct GameData {
    pub creature_templates: AHashMap<u32, CreatureTemplate>,
    pub creature_spawns: AHashMap<u32, Vec<CreatureSpawn>>,
    pub creature_addons: AHashMap<u32, CreatureAddon>,
    pub creature_equipment: AHashMap<u32, [u32; 3]>,
    pub waypoints: AHashMap<u32, Vec<Waypoint>>,
    pub class_level_stats: AHashMap<(u8, u8), ClassLevelStats>,
    pub display_info: AHashMap<u32, DisplayInfo>,
    pub gameobject_templates: AHashMap<u32, GameObjectTemplate>,
    pub gameobject_spawns: AHashMap<u32, Vec<GameObjectSpawn>>,
    pub items: AHashMap<u32, ItemTemplate>,
    pub areas: AHashMap<u32, AreaInfo>,
    pub area_triggers: AHashMap<u32, AreaTrigger>,
    pub maps: AHashMap<u32, MapInfo>,
    pub tele: Vec<Tele>,
    pub start: AHashMap<(u8, u8), StartInfo>,
    /// `(race, class, level)` → strength, agility, stamina, intellect,
    /// spirit.
    pub player_level_stats: AHashMap<(u8, u8, u8), [u32; 5]>,
    /// `(class, level)` → base health and mana.
    pub player_class_stats: AHashMap<(u8, u8), (u32, u32)>,
    /// XP to the next level, by level.
    pub xp_for_level: AHashMap<u8, u32>,
    pub races: AHashMap<u8, RaceInfo>,
    pub graveyards: Vec<Graveyard>,
    pub faction_templates: AHashMap<u32, FactionTemplate>,
    pub loot: LootTables,
    /// By creature entry, and by vendor template.
    pub vendors: AHashMap<u32, Vec<VendorItem>>,
    pub vendor_templates: AHashMap<u32, Vec<VendorItem>>,
    /// `DurabilityCosts`: item level → 21 weapon then 8 armor subclass costs.
    pub durability_costs: AHashMap<u32, Vec<u32>>,
    /// `DurabilityQuality`: id → multiplier.
    pub durability_quality: AHashMap<u32, f32>,
    pub npc: npc::NpcData,
    pub spells: spells::SpellData,
    /// Battlemaster entry → battleground template.
    pub battlemasters: AHashMap<u32, u32>,
    /// `battleground_template`: 1 Alterac Valley, 2 Warsong Gulch, 3 Arathi Basin.
    pub battlegrounds: AHashMap<u32, BgTemplate>,
    /// `transports`: boats and zeppelins, with their cycles in milliseconds.
    pub transports: Vec<TransportRoute>,
    /// `game_event`: seasonal events, by id.
    pub events: AHashMap<u32, GameEvent>,
    /// Creature and object spawns tied to an event: during it (a positive
    /// id) or outside it (negative).
    pub event_creatures: AHashMap<u32, i32>,
    pub event_objects: AHashMap<u32, i32>,
    /// `game_weather`: a zone's rain, snow and storm chances, by season.
    pub weather: AHashMap<u32, [[u32; 3]; 4]>,
    /// `WorldSafeLocs`: id → map, x, y, z.
    pub safe_locs: AHashMap<u32, (u32, f32, f32, f32)>,
    /// Text emote → the animation it plays.
    pub text_emote_anims: AHashMap<u32, u32>,
    pub pets: pets::PetData,
}

impl GameData {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether this is vmangos's world, not the empty fallback.
    pub fn is_loaded(&self) -> bool {
        !self.creature_templates.is_empty()
    }

    pub fn creature_template(&self, entry: u32) -> Option<&CreatureTemplate> {
        self.creature_templates.get(&entry)
    }

    pub fn item(&self, entry: u32) -> Option<&ItemTemplate> {
        self.items.get(&entry)
    }

    pub fn area(&self, id: u32) -> Option<&AreaInfo> {
        self.areas.get(&id)
    }

    /// The zone an area is in (itself, for a zone).
    pub fn zone_of(&self, area: u32) -> u32 {
        match self.areas.get(&area) {
            Some(a) if a.zone != 0 => a.zone,
            _ => area,
        }
    }

    /// What a race and class start with: vmangos's tables, or
    /// `wow_world_base`'s.
    pub fn start_info(&self, race: u8, class: u8) -> Option<StartInfo> {
        if let Some(s) = self.start.get(&(race, class)) {
            return Some(s.clone());
        }
        if !self.start.is_empty() {
            return None;
        }
        start_info(race, class)
    }

    /// The base stats for a race, class and level.
    pub fn level_stats(&self, race: u8, class: u8, level: u8) -> Option<BaseStats> {
        if let (Some(s), Some((health, mana))) = (
            self.player_level_stats.get(&(race, class, level)),
            self.player_class_stats.get(&(class, level)),
        ) {
            return Some(BaseStats {
                stats: *s,
                health: *health,
                mana: *mana,
            });
        }
        let rc = race_class(race, class)?;
        let s = rc
            .base_stats_for(level)
            .or_else(|| rc.base_stats().first().copied())?;
        Some(BaseStats {
            stats: [
                u32::from(s.strength),
                u32::from(s.agility),
                u32::from(s.stamina),
                u32::from(s.intellect),
                u32::from(s.spirit),
            ],
            health: u32::from(s.health),
            mana: u32::from(s.mana),
        })
    }

    /// XP needed to go from `level` to the next.
    pub fn xp_to_level(&self, level: u8) -> u32 {
        if let Some(x) = self.xp_for_level.get(&level) {
            return *x;
        }
        wow_world_base::vanilla::exp::exp_required_to_level_up(level)
            .and_then(|x| u32::try_from(x).ok())
            .unwrap_or(0)
    }

    pub fn race_display(&self, race: u8, gender: u8) -> (u32, f32) {
        let (fallback, scale) = race_display(race, gender);
        let display = self
            .races
            .get(&race)
            .map(|r| {
                if gender == 0 {
                    r.male_display
                } else {
                    r.female_display
                }
            })
            .filter(|d| *d != 0)
            .unwrap_or(fallback);
        (display, scale)
    }

    pub fn race_faction(&self, race: u8) -> u32 {
        self.races
            .get(&race)
            .map(|r| r.faction)
            .filter(|f| *f != 0)
            .unwrap_or_else(|| race_faction(race))
    }
}

/// A race, class and level's base stats.
#[derive(Debug, Clone, Copy)]
pub struct BaseStats {
    pub stats: [u32; 5],
    pub health: u32,
    pub mana: u32,
}

/// The race and class pair, if the combination exists.
pub fn race_class(race: u8, class: u8) -> Option<RaceClass> {
    let race = Race::try_from(race).ok()?;
    let class = Class::try_from(class).ok()?;
    RaceClass::try_from((race, class)).ok()
}

/// What a race and class start with, from `wow_world_base`.
pub fn start_info(race: u8, class: u8) -> Option<StartInfo> {
    let rc = race_class(race, class)?;
    let start = rc.starting_position();
    Some(StartInfo {
        map: start.map.as_int(),
        zone: 0,
        x: start.x,
        y: start.y,
        z: start.z,
        orientation: start.orientation,
        items: rc
            .starter_items()
            .iter()
            .map(|i| (i.item, i.ty.as_int(), u32::from(i.amount)))
            .collect(),
        spells: rc.starter_spells().to_vec(),
        actions: Vec::new(),
    })
}

/// The look a race and gender start with: display id, and scale.
pub fn race_display(race: u8, gender: u8) -> (u32, f32) {
    let race = Race::try_from(race)
        .ok()
        .and_then(|r| PlayerRace::try_from(r).ok());
    let gender = Gender::try_from(gender)
        .ok()
        .and_then(|g| PlayerGender::try_from(g).ok());
    let (Some(race), Some(gender)) = (race, gender) else {
        return (49, 1.0);
    };
    (
        u32::try_from(race.display_id(gender)).unwrap_or(49),
        race.race_scale(gender),
    )
}

/// The faction template a race's players have.
pub fn race_faction(race: u8) -> u32 {
    Race::try_from(race)
        .ok()
        .and_then(|r| PlayerRace::try_from(r).ok())
        .map(|r| u32::from(r.faction_id().as_int()))
        .unwrap_or(1)
}

pub fn gender(g: u8) -> Gender {
    Gender::try_from(g).unwrap_or(Gender::Male)
}

impl GameData {
    /// What a vendor sells: its entry's list, then its template's.
    pub fn vendor_items(&self, entry: u32) -> Vec<VendorItem> {
        let mut items = self.vendors.get(&entry).cloned().unwrap_or_default();
        if let Some(t) = self.creature_template(entry)
            && t.vendor_id != 0
            && let Some(more) = self.vendor_templates.get(&t.vendor_id)
        {
            items.extend(more.iter().copied());
        }
        items
    }
}

/// A battleground's rules from `battleground_template`.
#[derive(Debug, Clone, Default)]
pub struct BgTemplate {
    pub id: u32,
    pub min_players: u32,
    pub max_players: u32,
    pub min_level: u32,
    pub max_level: u32,
    /// Alliance's and Horde's spells on a win and a loss.
    pub win_spells: [u32; 2],
    pub lose_spells: [u32; 2],
    /// `WorldSafeLocs` where each team starts.
    pub starts: [u32; 2],
}

/// A boat or zeppelin: its object, and how long its round takes.
#[derive(Debug, Clone, Default)]
pub struct TransportRoute {
    pub entry: u32,
    pub name: String,
    pub period: u32,
}

/// A seasonal event: from `start` to `end` (Unix seconds), `length`
/// minutes in every `occurence`.
#[derive(Debug, Clone, Default)]
pub struct GameEvent {
    pub id: u32,
    pub name: String,
    pub start: i64,
    pub end: i64,
    pub occurence: i64,
    pub length: i64,
}

impl GameEvent {
    pub fn active(&self, now: i64) -> bool {
        if self.occurence <= 0 || now < self.start || now >= self.end {
            return false;
        }
        (now - self.start) % (self.occurence * 60) < self.length * 60
    }
}
