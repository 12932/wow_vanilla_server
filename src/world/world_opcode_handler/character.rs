use crate::world::database::WorldDatabase;
use crate::world::world_opcode_handler::inventory::Inventory;
use std::time::Instant;
use wow_world_base::movement::DEFAULT_RUNNING_SPEED;
use wow_world_base::stats::BaseStats;
use wow_world_base::stats::calculate_mana;
use wow_world_base::vanilla::{Level, Map, PlayerGender, RaceClass, Vector3d};
use wow_world_messages::vanilla::{Area, CreatureFamily, MovementInfo, Power};
use wow_world_messages::Guid;

#[derive(Debug, Clone)]
pub struct Character {
    pub guid: Guid,
    pub name: String,
    /// Account this character belongs to. Set by
    /// `WorldDatabase::create_character_in_account` at insertion time; left
    /// empty by `Character::new` since the caller hasn't yet declared which
    /// account is creating it. **Must** be set before reads, or
    /// account-scoped lookups will skip the character.
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
    pub info: MovementInfo,
    pub movement_speed: f32,
    pub target: Guid,
    pub attacking: bool,
    pub auto_attack_timer: f32,
    pub inventory: Inventory,
    /// Server-authoritative root expiry. While `Some(t) > Instant::now()`,
    /// the movement opcode handlers drop incoming `MSG_MOVE_*_Client`
    /// packets — neither updating this character's authoritative position
    /// nor broadcasting movement to observers. Headless clients (load-test
    /// bots) keep sending moves while rooted; the server simply ignores them.
    /// Real WoW clients also receive `SMSG_FORCE_MOVE_ROOT` which locks
    /// their input UI for cooperative clients.
    ///
    /// Not persisted in snapshots — runtime-only state.
    pub root_until: Option<Instant>,
    /// Current HP. Combat-tracked; not derived from the BaseStats table.
    /// Vanilla's real formula produces thousands of HP at max level — fine
    /// for PvE flavor but tedious for PvP testing. We keep PvP bounded by
    /// initializing to `PVP_MAX_HEALTH = 100` so fights resolve in seconds
    /// rather than minutes. Damage applied via [`apply_damage`]; reaches
    /// `0` → [`is_dead`] returns true and the per-tick respawn loop in
    /// `World::tick` kicks off resurrection after `RESPAWN_DELAY`.
    pub current_health: u32,
    /// HP cap. Reset target on respawn. Stored on the character (not
    /// derived) so a future GM command can buff a target without touching
    /// race/class stats.
    pub max_health: u32,
    /// Set on the tick the character's `current_health` first reaches `0`.
    /// The respawn loop uses this to decide when to bring the player back.
    /// Runtime-only (snapshot save resets to alive at full HP — players
    /// don't get to log back in still dead).
    pub time_of_death: Option<Instant>,
}

// HP every player starts with under our simplified combat rules lives in
// `[combat] pvp_max_health` (default 100). Picked so a ~10-damage swing
// kills in 7–13 hits at the unarmed swing speed — kill-times of 15–25 s,
// easy to observe during loadtest fights. There is no auto-respawn: a
// player who hits zero stays dead until the server restarts (snapshot
// load resets HP to `max_health`).

impl Character {
    fn default_stats(&self) -> BaseStats {
        if let Some(s) = self.race_class.base_stats_for(self.level.as_int()) {
            return s;
        }
        if let Some(s) = self.race_class.base_stats().first().copied() {
            return s;
        }
        // No data at all for this race/class. Use a sane all-zero default
        // rather than panic — fresh chars with broken data should still log in.
        tracing::warn!(
            "no base stats found for {:?} (level {}); using zeros",
            self.race_class,
            self.level.as_int(),
        );
        BaseStats::new(0, 0, 0, 0, 0, 1, 0)
    }

    pub fn test_character(
        db: &mut WorldDatabase,
        name: impl Into<String>,
        race_class: RaceClass,
        gender: PlayerGender,
    ) -> Self {
        let mut c = Self::new(db, name, race_class, gender, 0, 0, 0, 0, 0);
        c.level = Level::new_vanilla_max_level_player();
        c
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: &mut WorldDatabase,
        name: impl Into<String>,
        race_class: RaceClass,
        gender: PlayerGender,
        skin: u8,
        face: u8,
        hair_style: u8,
        hair_color: u8,
        facial_hair: u8,
    ) -> Self {
        let start = race_class.starting_position();

        let inventory = Inventory::new(race_class.starter_items(), db);

        Self {
            guid: db.new_guid().into(),
            name: name.into(),
            account: String::new(),
            race_class,
            gender,
            skin,
            face,
            hairstyle: hair_style,
            haircolor: hair_color,
            facialhair: facial_hair,
            level: Level::new_player(),
            area: Default::default(),
            map: start.map,
            info: MovementInfo {
                flags: Default::default(),
                timestamp: 0,
                position: Vector3d {
                    x: start.x,
                    y: start.y,
                    z: start.z,
                },
                orientation: start.orientation,
                fall_time: 0.0,
            },
            movement_speed: DEFAULT_RUNNING_SPEED,
            target: Default::default(),
            attacking: false,
            auto_attack_timer: 0.0,
            inventory,
            root_until: None,
            current_health: crate::config::config().combat.pvp_max_health,
            max_health: crate::config::config().combat.pvp_max_health,
            time_of_death: None,
        }
    }

    pub fn is_rooted(&self) -> bool {
        self.root_until
            .map(|t| t > Instant::now())
            .unwrap_or(false)
    }

    pub fn is_dead(&self) -> bool {
        self.current_health == 0
    }

    /// Apply combat damage. Saturating — doesn't underflow when overkilled.
    /// Returns the new health value so the caller can decide whether this
    /// hit is the killing blow without re-reading the field.
    pub fn apply_damage(&mut self, amount: u32) -> u32 {
        self.current_health = self.current_health.saturating_sub(amount);
        self.current_health
    }

    /// Reset to alive at full HP. Called by the per-tick respawn loop in
    /// `World::tick` after `RESPAWN_DELAY` has elapsed since
    /// `time_of_death`. Does not move the character — caller decides
    /// whether to teleport to a graveyard or rez in place.
    pub fn respawn_full_health(&mut self) {
        self.current_health = self.max_health;
        self.time_of_death = None;
        self.attacking = false;
        self.auto_attack_timer = 0.0;
    }

    pub fn update_auto_attack_timer(&mut self, dt: f32) {
        if self.auto_attack_timer > 0.0 {
            self.auto_attack_timer -= dt;
        }
    }

    pub fn strength(&self) -> i32 {
        self.default_stats().strength.into()
    }

    pub fn base_mana(&self) -> i32 {
        self.default_stats().mana.into()
    }

    pub fn max_mana(&self) -> i32 {
        if self.race_class.class().power_type() == Power::Mana {
            calculate_mana(self.default_stats().mana, self.default_stats().intellect).into()
        } else {
            0
        }
    }

    pub fn agility(&self) -> i32 {
        self.default_stats().agility.into()
    }

    pub fn stamina(&self) -> i32 {
        self.default_stats().stamina.into()
    }

    pub fn intellect(&self) -> i32 {
        self.default_stats().intellect.into()
    }

    pub fn spirit(&self) -> i32 {
        self.default_stats().spirit.into()
    }
}

impl From<Character> for wow_world_messages::vanilla::Character {
    fn from(e: Character) -> Self {
        wow_world_messages::vanilla::Character {
            guid: e.guid,
            name: e.name,
            race: e.race_class.race().into(),
            class: e.race_class.class(),
            gender: e.gender.into(),
            skin: e.skin,
            face: e.face,
            hair_style: e.hairstyle,
            hair_color: e.haircolor,
            facial_hair: e.facialhair,
            level: e.level,
            area: e.area,
            map: e.map,
            position: e.info.position,
            guild_id: 0,
            flags: Default::default(),
            first_login: false,
            pet_display_id: 0,
            pet_level: Level::zero(),
            pet_family: CreatureFamily::None,
            equipment: e.inventory.to_character_gear(),
        }
    }
}

impl Eq for Character {}

impl PartialEq for Character {
    fn eq(&self, other: &Self) -> bool {
        self.guid == other.guid
    }
}
