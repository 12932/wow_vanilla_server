//! Server configuration: the code's defaults, then `config.toml`, then the
//! environment (`WVS__SECTION__KEY=value`).
//!
//! Every key is declared once, in a [`settings!`] block: its type, default,
//! range, whether it changes at a reload, the mangos key it is named after,
//! and its meaning. From that come the structs, their defaults, the checks
//! at load, `--print-config`, the generated `config.example.toml`, and the
//! hot reload's sorting of what changed. See `docs/CONFIG.md`.
//!
//! Deployment settings (`WOW_REALM_ADDRESS`, `WOW_TRACY`, `RUST_LOG`) stay
//! environment variables of their own.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Where the server reads its config: `config.toml` in its working
/// directory.
pub const PATH: &str = "config.toml";
/// The prefix of an environment variable that overrides a key.
pub const ENV_PREFIX: &str = "WVS__";

static CONFIG: OnceLock<ServerConfig> = OnceLock::new();

/// The config the server started with; the defaults if none was installed
/// (tests). What a reload changes is in the realm's
/// [`crate::game::RealmConfig`], not here: read here only what needs a
/// restart.
pub fn config() -> &'static ServerConfig {
    static FALLBACK: OnceLock<ServerConfig> = OnceLock::new();
    CONFIG
        .get()
        .unwrap_or_else(|| FALLBACK.get_or_init(ServerConfig::default))
}

/// Install `cfg` as the process's config. The first call wins.
pub fn install(cfg: ServerConfig) -> &'static ServerConfig {
    let _ = CONFIG.set(cfg);
    config()
}

/// When a key's new value takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reload {
    /// At a reload (`.reload config`, SIGHUP).
    Live,
    /// Only at a restart.
    Restart,
}

/// What a number is allowed to be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Check {
    /// Anything its type holds.
    Any,
    /// Within the range; outside, the nearest end, with a warning.
    Clamp(f64, f64),
    /// Within the range; outside, the config is refused.
    Refuse(f64, f64),
}

impl Check {
    /// "0 to 100, clamped", for the docs and the messages.
    pub fn describe(&self) -> Option<String> {
        match *self {
            Check::Any => None,
            Check::Clamp(lo, hi) => Some(format!("{} to {}, clamped", num(lo), num(hi))),
            Check::Refuse(lo, hi) => Some(format!("{} to {}, else refused", num(lo), num(hi))),
        }
    }
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// One key of a section.
#[derive(Debug)]
pub struct Key {
    pub name: &'static str,
    /// Its meaning: the field's doc comment.
    pub doc: &'static str,
    pub check: Check,
    pub reload: Reload,
    /// The mangos-classic key it is named after (`mangosd.conf`), if any.
    pub mangos: &'static str,
}

/// A `[section]` of the config.
#[derive(Debug)]
pub struct Section {
    pub name: &'static str,
    pub doc: &'static str,
    pub keys: &'static [Key],
}

impl Section {
    pub fn key(&self, name: &str) -> Option<&'static Key> {
        self.keys.iter().find(|k| k.name == name)
    }
}

/// A section's struct, its defaults and its description, from one list:
/// `name: Type = default, check, Live|Restart, "Mangos.Key";`, each with
/// its doc comment.
macro_rules! settings {
    (
        $(#[doc = $sdoc:literal])*
        $vis:vis struct $name:ident in $sect:literal {
            $(
                $(#[doc = $doc:literal])*
                $field:ident: $ty:ty = $default:expr, $check:expr, $reload:ident, $mangos:literal;
            )*
        }
    ) => {
        $(#[doc = $sdoc])*
        #[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
        #[serde(default, deny_unknown_fields)]
        $vis struct $name {
            $(
                $(#[doc = $doc])*
                pub $field: $ty,
            )*
        }

        impl Default for $name {
            fn default() -> Self {
                Self { $($field: $default,)* }
            }
        }

        impl $name {
            pub const SECTION: Section = Section {
                name: $sect,
                doc: concat!($($sdoc, "\n",)*),
                keys: &[$(Key {
                    name: stringify!($field),
                    doc: concat!($($doc, "\n",)*),
                    check: $check,
                    reload: Reload::$reload,
                    mangos: $mangos,
                },)*],
            };
        }
    };
}

use Check::{Any, Clamp};

/// Where the world's data is looked for when the config doesn't say: under
/// `data/` in the working directory, so a server run beside its data needs
/// no paths at all.
pub const DEFAULT_DATA_DIR: &str = "data/db";
pub const DEFAULT_NAV_DIR: &str = "data/nav";
pub const DEFAULT_TERRAIN_DIR: &str = "data/terrain";

settings! {
    /// Connections: queues, timeouts and limits.
    pub struct NetworkConfig in "network" {
        /// Bytes a client's outbound queue may hold; a client that falls
        /// further behind is disconnected, not sent a partial world.
        outbound_channel_bytes: usize = 4 * 1024 * 1024, Clamp(65_536.0, 1_073_741_824.0), Restart, "";
        /// Seconds every read of the login and the world handshake must
        /// arrive within.
        handshake_timeout_secs: u64 = 10, Clamp(1.0, 600.0), Restart, "";
        /// Seconds a client may send nothing before it is disconnected.
        /// Clients ping every 30.
        client_idle_timeout_secs: u64 = 120, Clamp(10.0, 3600.0), Restart, "";
        /// Connections still in the login or the world handshake, at most.
        max_pending_connections: usize = 1024, Clamp(1.0, 1_000_000.0), Restart, "";
        /// Packets a client may send a second, sustained (three seconds'
        /// worth at once); a client that floods is disconnected.
        max_packets_per_sec: u32 = 100, Clamp(10.0, 100_000.0), Restart, "";
        /// Read-only queries (names, creatures, objects, items, quests,
        /// texts, times) a client may send a second, sustained (three
        /// seconds' worth at once), apart from the packets: a client
        /// walking into a crowd asks after everyone in it at once. One that
        /// floods them is disconnected.
        max_queries_per_sec: u32 = 1000, Clamp(10.0, 1_000_000.0), Restart, "";
        /// Pings a client may send in a row each less than 27 seconds after
        /// the last (the client pings every 30) before it is disconnected;
        /// 0 never disconnects.
        max_overspeed_pings: u32 = 2, Clamp(0.0, 1000.0), Restart, "MaxOverspeedPings";
        /// Where the login server listens, `ip:port`. Every interface by
        /// default (decision 53); `127.0.0.1:3724` keeps it to this
        /// machine, as a load test should.
        auth_address: String = "0.0.0.0:3724".to_owned(), Any, Restart, "";
        /// Where the world server listens, `ip:port`. Without
        /// `WOW_REALM_ADDRESS`, the realm list sends clients to
        /// `localhost` on its port.
        world_address: String = "0.0.0.0:8085".to_owned(), Any, Restart, "";
        /// Milliseconds a map may keep players' movement before it sends
        /// it on, gathering what each client is sent into one write: a
        /// crowd's packets cost far less so, a little later. 0 sends each
        /// burst of movement as it is handled.
        broadcast_batch_ms: u64 = 10, Clamp(0.0, 50.0), Live, "";
    }
}

impl NetworkConfig {
    pub fn handshake_timeout(&self) -> Duration {
        Duration::from_secs(self.handshake_timeout_secs)
    }

    pub fn client_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.client_idle_timeout_secs)
    }
}

settings! {
    /// The world: saves, the message of the day, the character store and
    /// the data's paths.
    pub struct WorldConfig in "world" {
        /// Seconds between saves of each online character.
        save_interval_secs: u64 = 300, Clamp(10.0, 86_400.0), Live, "PlayerSave.Interval";
        /// The character store (with the accounts in it).
        characters_db: PathBuf = PathBuf::from("characters.sqlite"), Any, Restart, "";
        /// The directory with `vmangos.sqlite` and `dbc.sqlite`, as Thistle
        /// Tea exports them; `data/db` if unset and there. The server won't
        /// start on data that is missing or doesn't load; without any, the
        /// world is empty, and only for a store with no characters.
        data_dir: Option<PathBuf> = None, Any, Restart, "DataDir";
        /// A rustigator bake, the directory with `Azeroth.map`; `data/nav`
        /// if unset and there. Without it creatures walk straight lines and
        /// zones come from the client.
        nav_dir: Option<PathBuf> = None, Any, Restart, "";
        /// Thistle Tea's terrain tiles (`nix run .#terrain-data`); `data/terrain`
        /// if unset and there. Without them liquids have no kind outside
        /// WMOs: no deep-sea fatigue, no lava in the open.
        terrain_dir: Option<PathBuf> = None, Any, Restart, "";
        /// Seconds a navmesh or terrain tile is kept with no player or
        /// awake creature near it and nothing asking about it; then it is
        /// let go, and read again if it is needed. 0 keeps every tile read.
        tile_unload_secs: u64 = 300, Clamp(0.0, 86_400.0), Live, "GridCleanUpDelay";
        /// Shown to every player at login.
        motd: String = "Welcome to wow_vanilla_server.".to_owned(), Any, Live, "Motd";
        /// Names no character may take.
        reserved_names: Vec<String> = Vec::new(), Any, Restart, "";
        /// Signatures a guild charter needs.
        petition_signatures: u32 = 9, Clamp(0.0, 9.0), Live, "MinPetitionSigns";
        /// Seconds mail with money or an item takes between accounts.
        mail_delay_secs: u64 = 3600, Clamp(0.0, 2_592_000.0), Live, "MailDeliveryDelay";
        /// The weekday honor is ranked on, each week: 1 Monday to 7 Sunday
        /// (Tuesday, as Thistle Tea).
        honor_reset_weekday: u8 = crate::game::honor::DEFAULT_RESET_WEEKDAY, Clamp(1.0, 7.0), Restart, "";
        /// Honorable kills a week needs to be ranked at its end.
        min_honor_kills: u32 = crate::game::honor::MINIMUM_KILLS, Clamp(1.0, 10_000.0), Live, "MinHonorKills";
        /// The most players a `/who` lists (the client shows 50 at most).
        max_who_list_returns: usize = crate::game::map::social::WHO_LIMIT, Clamp(1.0, 49.0), Live, "MaxWhoListReturns";
        /// Zones' weather changes: rain, snow and storms, by the season.
        /// Off, the weather is fine everywhere but where a GM set it for
        /// good.
        weather: bool = true, Any, Live, "ActivateWeather";
        /// Seconds between a zone's rolls for a change of weather.
        weather_change_secs: u64 = crate::game::weather::CHANGE_EVERY.as_secs(), Clamp(10.0, 86_400.0), Live, "ChangeWeatherInterval";
        /// The least deposit an auction takes, in copper.
        auction_deposit_min: u32 = 0, Clamp(0.0, 2_147_483_647.0), Live, "Auction.Deposit.Min";
        /// The realm's name in the realm list.
        realm_name: String = crate::auth::REALM_NAME.to_owned(), Any, Restart, "";
    }
}

impl WorldConfig {
    /// The data directories the config leaves out: the defaults under
    /// `data/`, where they exist.
    pub fn default_dirs(&mut self) {
        for (dir, default) in [
            (&mut self.data_dir, DEFAULT_DATA_DIR),
            (&mut self.nav_dir, DEFAULT_NAV_DIR),
            (&mut self.terrain_dir, DEFAULT_TERRAIN_DIR),
        ] {
            if dir.is_none() && Path::new(default).is_dir() {
                tracing::info!("using {default}");
                *dir = Some(PathBuf::from(default));
            }
        }
    }
}

settings! {
    /// How far players see, in yards. Further costs the server more and
    /// the client more.
    pub struct VisibilityConfig in "visibility" {
        /// On the continents.
        continents_yards: f32 = 100.0, Clamp(20.0, 500.0), Live, "Visibility.Distance.Continents";
        /// In dungeons and raids.
        instances_yards: f32 = 170.0, Clamp(20.0, 500.0), Live, "Visibility.Distance.Instances";
        /// In battlegrounds: further, so a battle's far side is in view.
        battleground_yards: f32 = 400.0, Clamp(20.0, 500.0), Live, "Visibility.Distance.BGArenas";
        /// On a flight master's gryphon or wyvern: further, as the land
        /// goes by below. Back to the map's own on landing.
        flight_yards: f32 = 400.0, Clamp(20.0, 500.0), Live, "";
        /// Past these yards from a player who moves, those who see them
        /// get only every `distant_move_every`th of their heartbeats (a
        /// start, a stop, a jump, a turn or a landing always): fewer
        /// packets for a crowd's far side. 0 sends every heartbeat to
        /// everyone, as 1.12's servers do.
        distant_move_yards: f32 = 0.0, Clamp(0.0, 500.0), Live, "";
        /// Of a mover's heartbeats, the share an observer past
        /// `distant_move_yards` gets: one in this many. 1 is every one.
        distant_move_every: u32 = 1, Clamp(1.0, 100.0), Live, "";
    }
}

settings! {
    /// Multipliers on what players earn, regenerate and pay, and the
    /// skill-ups' sizes and chances. 1 is 1.12's own.
    pub struct RatesConfig in "rates" {
        /// XP from kills.
        xp_kill: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.XP.Kill";
        /// XP from quests.
        xp_quest: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.XP.Quest";
        /// XP from discovering an area.
        xp_explore: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.XP.Explore";
        /// The drop chance of poor (grey) items.
        drop_item_poor: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Poor";
        /// The drop chance of normal (white) items.
        drop_item_normal: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Normal";
        /// The drop chance of uncommon (green) items.
        drop_item_uncommon: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Uncommon";
        /// The drop chance of rare (blue) items.
        drop_item_rare: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Rare";
        /// The drop chance of epic (purple) items.
        drop_item_epic: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Epic";
        /// The drop chance of legendary (orange) items.
        drop_item_legendary: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Legendary";
        /// The drop chance of artifacts.
        drop_item_artifact: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Artifact";
        /// The chance a table's reference to another table is rolled.
        drop_item_referenced: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Referenced";
        /// The drop chance of quest items, for those on the quest.
        drop_item_quest: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Item.Quest";
        /// Money dropped by creatures and found in chests.
        drop_money: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Drop.Money";
        /// Honor from kills and battlegrounds (a dishonorable kill costs
        /// what it costs).
        honor: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Honor";
        /// Reputation gained from kills, quests and spells (losses are as
        /// they are).
        reputation_gain: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Reputation.Gain";
        /// Rested XP earned resting in game.
        rest_in_game: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Rest.InGame";
        /// Rested XP earned logged out in an inn or a city.
        rest_offline_in_tavern_or_city: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Rest.Offline.InTavernOrCity";
        /// Rested XP earned logged out elsewhere (a quarter of an inn's at 1).
        rest_offline_in_wilderness: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Rest.Offline.InWilderness";
        /// Creatures' respawn times (each at least five seconds).
        respawn_time: f32 = 1.0, Clamp(0.0, 100.0), Live, "";
        /// Points a crafting skill rises by at a skill-up.
        skill_gain_crafting: u32 = 1, Clamp(1.0, 100.0), Live, "SkillGain.Crafting";
        /// Points a gathering skill (herbalism, mining, skinning,
        /// lockpicking) rises by at a skill-up.
        skill_gain_gathering: u32 = 1, Clamp(1.0, 100.0), Live, "SkillGain.Gathering";
        /// Points a weapon skill rises by at a skill-up.
        skill_gain_weapon: u32 = 1, Clamp(1.0, 100.0), Live, "SkillGain.Weapon";
        /// Points defense rises by at a skill-up.
        skill_gain_defense: u32 = 1, Clamp(1.0, 100.0), Live, "SkillGain.Defense";
        /// The chance, in percent, an orange recipe raises its skill.
        skill_chance_orange: u32 = 100, Clamp(0.0, 100.0), Live, "SkillChance.Orange";
        /// The chance, in percent, a yellow recipe raises its skill.
        skill_chance_yellow: u32 = 75, Clamp(0.0, 100.0), Live, "SkillChance.Yellow";
        /// The chance, in percent, a green recipe raises its skill.
        skill_chance_green: u32 = 25, Clamp(0.0, 100.0), Live, "SkillChance.Green";
        /// The chance, in percent, a grey recipe raises its skill.
        skill_chance_grey: u32 = 0, Clamp(0.0, 100.0), Live, "SkillChance.Grey";
        /// Mining's chance to rise halves each this many points (0: it
        /// never does, mangos's default).
        skill_chance_mining_steps: u32 = 0, Clamp(0.0, 300.0), Live, "SkillChance.MiningSteps";
        /// Skinning's chance to rise halves each this many points (0: it
        /// never does, mangos's default).
        skill_chance_skinning_steps: u32 = 0, Clamp(0.0, 300.0), Live, "SkillChance.SkinningSteps";
        /// The least rate a grey creature's kill gives reputation at.
        reputation_low_level_kill: f32 = 0.2, Clamp(0.0, 1.0), Live, "Rate.Reputation.LowLevel.Kill";
        /// The least rate a quest well below the player gives reputation
        /// at: a fifth less for each level past five below, down to this.
        reputation_low_level_quest: f32 = 0.2, Clamp(0.0, 1.0), Live, "Rate.Reputation.LowLevel.Quest";
        /// Health players regenerate from spirit.
        health: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Health";
        /// Mana players regenerate from spirit.
        mana: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Mana";
        /// Rage from blows dealt and taken.
        rage_income: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Rage.Income";
        /// Rage lost out of combat.
        rage_loss: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Rage.Loss";
        /// Energy regenerated.
        energy: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Energy";
        /// Damage from falls.
        damage_fall: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Damage.Fall";
        /// How long auctions run (the deposit is the chosen time's).
        auction_time: f32 = 1.0, Clamp(0.01, 100.0), Live, "Rate.Auction.Time";
        /// Auctions' deposits.
        auction_deposit: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Auction.Deposit";
        /// The auction house's cut of a sale (never more than the sale).
        auction_cut: f32 = 1.0, Clamp(0.0, 100.0), Live, "Rate.Auction.Cut";
    }
}

impl RatesConfig {
    /// The drop rate of an item of `quality` (0 poor to 6 artifact).
    pub fn drop_item(&self, quality: u32) -> f32 {
        match quality {
            0 => self.drop_item_poor,
            1 => self.drop_item_normal,
            2 => self.drop_item_uncommon,
            3 => self.drop_item_rare,
            4 => self.drop_item_epic,
            5 => self.drop_item_legendary,
            _ => self.drop_item_artifact,
        }
    }

    /// Whether every drop rate is 1, so a roll needn't look its items up.
    pub fn drops_as_they_are(&self) -> bool {
        [
            self.drop_item_poor,
            self.drop_item_normal,
            self.drop_item_uncommon,
            self.drop_item_rare,
            self.drop_item_epic,
            self.drop_item_legendary,
            self.drop_item_artifact,
            self.drop_item_referenced,
            self.drop_item_quest,
        ]
        .iter()
        .all(|r| *r == 1.0)
    }
}

settings! {
    /// Creatures: how near they notice players, how far they chase, how
    /// they call for help, how long their corpses lie.
    pub struct CreaturesConfig in "creatures" {
        /// Multiplies the distance at which creatures notice players.
        aggro_rate: f32 = 1.0, Clamp(0.0, 10.0), Live, "Rate.Creature.Aggro";
        /// Yards from where a fight began that a creature chases before it
        /// may give up.
        leash_radius: f32 = 30.0, Clamp(5.0, 500.0), Live, "LeashRadius";
        /// Seconds beyond its leash without a hostile act before a creature
        /// goes home.
        leash_timeout_secs: u64 = 12, Clamp(1.0, 600.0), Live, "";
        /// Seconds a creature tries for a victim it can't reach before it
        /// goes home.
        unreachable_timeout_secs: u64 = 24, Clamp(1.0, 600.0), Live, "";
        /// Yards a creature drawn into a fight calls its kind from.
        assistance_radius: f32 = 5.0, Clamp(0.0, 100.0), Live, "CreatureFamilyAssistanceRadius";
        /// Milliseconds the called take to come.
        assistance_delay_ms: u64 = 1500, Clamp(0.0, 60_000.0), Live, "CreatureFamilyAssistanceDelay";
        /// Yards a fleeing creature looks for help within.
        flee_assistance_radius: f32 = 30.0, Clamp(0.0, 100.0), Live, "CreatureFamilyFleeAssistanceRadius";
        /// Milliseconds a creature flees for help, where its script doesn't
        /// say (mangos's default is 10000).
        flee_delay_ms: u64 = crate::game::map::creature_combat::FLEE_FOR.as_millis() as u64, Clamp(0.0, 60_000.0), Live, "CreatureFamilyFleeDelay";
        /// Seconds a wandering creature stands still once a player talks
        /// to it.
        interaction_pause_secs: u64 = crate::game::map::creature_motion::TALK_PAUSE.as_secs(), Clamp(0.0, 3600.0), Live, "InteractionPauseTimer";
        /// Seconds a normal creature's corpse lies (no longer than its
        /// respawn).
        corpse_decay_normal: u64 = 300, Clamp(1.0, 86_400.0), Live, "Corpse.Decay.NORMAL";
        /// Seconds a rare's corpse lies.
        corpse_decay_rare: u64 = 900, Clamp(1.0, 86_400.0), Live, "Corpse.Decay.RARE";
        /// Seconds an elite's corpse lies.
        corpse_decay_elite: u64 = 600, Clamp(1.0, 86_400.0), Live, "Corpse.Decay.ELITE";
        /// Seconds a rare elite's corpse lies.
        corpse_decay_rare_elite: u64 = 1200, Clamp(1.0, 86_400.0), Live, "Corpse.Decay.RAREELITE";
        /// Seconds a boss's corpse lies.
        corpse_decay_world_boss: u64 = 3600, Clamp(1.0, 86_400.0), Live, "Corpse.Decay.WORLDBOSS";
    }
}

impl CreaturesConfig {
    /// How long the corpse of a creature of `rank` lies: 0 normal, 1
    /// elite, 2 rare elite, 3 boss, 4 rare.
    pub fn corpse_decay(&self, rank: u32) -> Duration {
        Duration::from_secs(match rank {
            1 => self.corpse_decay_elite,
            2 => self.corpse_decay_rare_elite,
            3 => self.corpse_decay_world_boss,
            4 => self.corpse_decay_rare,
            _ => self.corpse_decay_normal,
        })
    }
}

settings! {
    /// Town guards, as mangos-classic calls them: a guard each time a
    /// creature that calls the guards takes a victim. A town's alarm
    /// counts the guards called there lately and cools when the calls
    /// stop; as it rises, the guards grow harder to fight, a capital sends
    /// its elite two at a time, then its Elite Guards, and at the top its
    /// champion comes with more of them: Jaina Proudmoore to Stormwind,
    /// Ironforge and Darnassus, Rexxar to Orgrimmar, the Undercity and
    /// Thunder Bluff.
    pub struct GuardsConfig in "guards" {
        /// Seconds a called guard stays once out of combat (mangos-classic's
        /// 30); it goes at once when it dies.
        despawn_secs: u64 = 300, Clamp(5.0, 86_400.0), Live, "";
        /// Yards a called guard wanders about where it came, out of combat,
        /// and again once home from a fight.
        wander_yards: f32 = 10.0, Clamp(0.0, 30.0), Live, "";
        /// Seconds without a call after which a town's alarm halves.
        alarm_half_life_secs: u64 = 300, Clamp(10.0, 86_400.0), Live, "";
        /// Guards in the alarm from which each new guard comes with
        /// Avoidance (spell 23198's aura: area spells miss it that many
        /// percent more often).
        avoidance_from: u32 = 20, Clamp(1.0, 100_000.0), Live, "";
        /// Avoidance, in percent, at the threshold and for each further
        /// `avoidance_step_guards` guards.
        avoidance_step_pct: u32 = 5, Clamp(0.0, 100.0), Live, "";
        /// Guards in the alarm per step of Avoidance.
        avoidance_step_guards: u32 = 10, Clamp(1.0, 100_000.0), Live, "";
        /// The most Avoidance a guard comes with, in percent.
        avoidance_max_pct: u32 = 50, Clamp(0.0, 100.0), Live, "";
        /// Guards in the alarm from which a call brings two of the town's
        /// elite (Stormwind's Royal Guard, Orgrimmar's Kor'kron, Undercity's
        /// Royal Dreadguard, Thunder Bluff's Honor Guard), or two of its
        /// guards where it has none.
        royal_from: u32 = 40, Clamp(1.0, 100_000.0), Live, "";
        /// Guards in the alarm of a defended city (one in `jaina_cities` or
        /// `rexxar_cities`) whose crossing brings `elite_count` of the
        /// city's Elite Guards (Stormwind's, Ironforge's, Darnassus's
        /// Sentinels, Orgrimmar's Grunts, the Undercity's Deathguards,
        /// Thunder Bluff's Bluffwatchers) near the call that crossed it,
        /// against its attacker: once each time the alarm rises through
        /// it, not at each call.
        elite_from: u32 = 50, Clamp(1.0, 100_000.0), Live, "";
        /// Elite Guards the crossing of `elite_from` brings.
        elite_count: u32 = 2, Clamp(0.0, 20.0), Live, "";
        /// The cities Jaina defends, by area: 1519 Stormwind City, 1537
        /// Ironforge, 1657 Darnassus. Their alarms bring their Elite Guards
        /// and her; empty, she never leaves Theramore. She is one: called
        /// by one while she is out, a city waits, and its next call after
        /// she is home brings her.
        jaina_cities: Vec<u32> = vec![1519, 1537, 1657], Any, Live, "";
        /// Guards in the alarm of one of Jaina's cities from which she
        /// comes, if she is alive and at home in Theramore.
        jaina_from: u32 = 60, Clamp(1.0, 100_000.0), Live, "";
        /// Seconds Jaina stays once she has no one left to fight and is
        /// out of combat, before she goes home.
        jaina_stays_secs: u64 = 60, Clamp(5.0, 86_400.0), Live, "";
        /// The city's Elite Guards who come with Jaina, around her, walk to
        /// her side fighting what they meet, guard her, and leave with her.
        jaina_elites: u32 = 4, Clamp(0.0, 20.0), Live, "";
        /// Yards from Jaina her Elite Guards come.
        jaina_elites_yards: f32 = 40.0, Clamp(5.0, 100.0), Live, "";
        /// Seconds Jaina takes to come back to Theramore once she has died
        /// in a city (her spawn's own time, a day).
        jaina_respawn_secs: u64 = 86_400, Clamp(5.0, 2_592_000.0), Live, "";
        /// The cities Rexxar defends, by area: 1637 Orgrimmar, 1497 the
        /// Undercity, 1638 Thunder Bluff. Their alarms bring their Elite
        /// Guards and him, with Misha; empty, he never leaves Desolace. One
        /// Rexxar, as one Jaina.
        rexxar_cities: Vec<u32> = vec![1637, 1497, 1638], Any, Live, "";
        /// Guards in the alarm of one of Rexxar's cities from which he
        /// comes, if he is alive and at home in Desolace.
        rexxar_from: u32 = 60, Clamp(1.0, 100_000.0), Live, "";
        /// Seconds Rexxar stays once he has no one left to fight and is out
        /// of combat, before he goes.
        rexxar_stays_secs: u64 = 60, Clamp(5.0, 86_400.0), Live, "";
        /// The city's Elite Guards who run in with Rexxar, behind him,
        /// guard him, and run off with him.
        rexxar_elites: u32 = 4, Clamp(0.0, 20.0), Live, "";
        /// Yards from the fight Rexxar comes into the city, out of sight
        /// (past the 100 yards a player sees and its 10 more), to run in;
        /// nearer where no walk from there reaches the fight.
        rexxar_arrive_yards: f32 = 110.0, Clamp(10.0, 300.0), Live, "";
        /// Yards Rexxar runs off before he is gone, as he leaves a city
        /// and as he leaves Desolace for one.
        rexxar_run_yards: f32 = 40.0, Clamp(5.0, 100.0), Live, "";
        /// Rexxar's run speed as he comes and goes, as a multiple of his
        /// own (his template's, 8 yards a second).
        rexxar_run_speed: f32 = 2.0, Clamp(1.0, 5.0), Live, "";
        /// Seconds Rexxar takes to come back to Desolace once he has died
        /// in a city (a day, as Jaina's).
        rexxar_respawn_secs: u64 = 86_400, Clamp(5.0, 2_592_000.0), Live, "";
    }
}

settings! {
    /// New characters, the level cap, the realm's type, and what is left
    /// of the dead.
    pub struct PlayersConfig in "players" {
        /// The level new characters start at (no higher than `max_level`).
        start_level: u8 = 1, Clamp(1.0, 60.0), Live, "StartPlayerLevel";
        /// The highest level a player can reach.
        max_level: u8 = 60, Clamp(1.0, 60.0), Restart, "MaxPlayerLevel";
        /// Copper new characters start with.
        start_money: u32 = 0, Clamp(0.0, 2_147_483_647.0), Live, "StartPlayerMoney";
        /// Characters an account may have on the realm.
        characters_per_realm: usize = 10, Clamp(1.0, 10.0), Live, "CharactersPerRealm";
        /// A PvP realm: an account's characters are all of one side, and
        /// everyone is flagged for PvP.
        pvp_realm: bool = false, Any, Restart, "GameType";
        /// Weapon skills and Defense at their caps, at login and each
        /// level, rather than raised by use from 1.
        always_max_skill_for_level: bool = false, Any, Live, "AlwaysMaxSkillForLevel";
        /// Hours the bones a corpse turns to lie where it was, once its
        /// player rises (reclaims it, is raised, or a spirit healer raises
        /// them), seen by everyone; 0 leaves none. Bones in the open world
        /// are kept across a restart, with the time they had left; a
        /// dungeon's or a battleground's go with its copy. mangos-classic
        /// has no time for them: its bones lie until their grid unloads,
        /// aren't saved, and `Death.Bones.World` and
        /// `Death.Bones.Battleground` turn them off.
        bones_hours: u64 = 18, Clamp(0.0, 8_760.0), Live, "";
        /// Yards from a kill a group's member must be for a share of it:
        /// its XP, loot, quest credit, reputation and honor (in a
        /// battleground, its honorable kill).
        group_reward_yards: f32 = crate::game::map::combat::GROUP_REWARD_YARDS, Clamp(5.0, 500.0), Live, "MaxGroupXPDistance";
        /// The level from which a spirit healer's resurrection brings
        /// sickness: a minute at this level, a minute more a level, ten
        /// at most. Above the level cap, never.
        death_sickness_level: u32 = 11, Clamp(1.0, 100.0), Live, "Death.SicknessLevel";
        /// Primary professions (alchemy, mining, tailoring, ...) a player
        /// may learn.
        max_primary_trade_skill: u32 = 2, Clamp(0.0, 10.0), Live, "MaxPrimaryTradeSkill";
        /// The chance, in percent, a blow dealt or taken wears a point off
        /// a random piece of a player's gear.
        durability_loss_chance_damage: f32 = crate::game::map::vendor::HIT_WEAR_CHANCE, Clamp(0.0, 100.0), Live, "DurabilityLossChance.Damage";
        /// Seconds a group's leader may be offline before the lead goes
        /// to a member online (a raid's assistant first); with no one
        /// online, it waits.
        group_offline_leader_secs: u64 = 300, Clamp(0.0, 86_400.0), Live, "Group.OfflineLeaderDelay";
    }
}

settings! {
    /// Quests: which givers show what.
    pub struct QuestsConfig in "quests" {
        /// Levels a player may be short of a quest's least level and still
        /// see its giver's grey "!"; further below, the giver shows nothing
        /// for it. -1 shows it at any gap.
        high_level_hide_diff: i32 = 7, Clamp(-1.0, 60.0), Live, "Quests.HighLevelHideDiff";
    }
}

settings! {
    /// Battlegrounds' queues and matches.
    pub struct BattlegroundConfig in "battleground" {
        /// Seconds from a match's first arrival to its start.
        start_delay_secs: u64 = 120, Clamp(0.0, 600.0), Live, "";
        /// Players a side needs for a match; 0 for each battleground's own.
        min_players: u32 = 0, Clamp(0.0, 40.0), Live, "";
        /// Seconds after a match ends before everyone is sent home.
        auto_leave_secs: u64 = 120, Clamp(10.0, 600.0), Live, "";
        /// Seconds an invitation to a match stands.
        invitation_secs: u64 = 120, Clamp(10.0, 600.0), Live, "";
        /// A player who leaves a match before its end is a Deserter, kept
        /// out of battlegrounds for a while.
        cast_deserter: bool = true, Any, Live, "Battleground.CastDeserter";
        /// Seconds a match goes on with a side short of players before it
        /// ends, the side ahead (or the larger) winning; 0 never ends it.
        premature_finish_secs: u64 = 300, Clamp(0.0, 3600.0), Live, "BattleGround.PrematureFinishTimer";
    }
}

settings! {
    /// World PvP on the continents.
    pub struct OutdoorPvpConfig in "outdoor_pvp" {
        /// The Eastern Plaguelands' four towers, their buffs (there and in
        /// Stratholme and Scholomance) and the Crown Guard graveyard.
        eastern_plaguelands: bool = true, Any, Restart, "OutdoorPvp.EPEnabled";
        /// Silithus's silithyst and the Cenarion Favor (there and in
        /// Ahn'Qiraj).
        silithus: bool = true, Any, Restart, "OutdoorPvp.SIEnabled";
    }
}

settings! {
    /// World events the server runs of itself.
    pub struct EventsConfig in "events" {
        /// The Elemental Invasion: rifts in Un'Goro, Silithus, Azshara and
        /// Winterspring, their stages and lords, then days of rest. Off,
        /// it stops where it is.
        elemental_invasion: bool = true, Any, Live, "";
        /// The Scourge Invasion's necropolises over the zones it attacks,
        /// as `.invasion on` (Thistle Tea's `scourge_invasion`, off). A
        /// GM's `.invasion on` or `off` holds until this changes.
        scourge_invasion: bool = false, Any, Live, "";
    }
}

settings! {
    /// Chat: how far it carries, and the flood control. GMs aren't
    /// muted.
    pub struct ChatConfig in "chat" {
        /// Yards a say carries, players' and creatures'.
        say_yards: f32 = crate::game::map::chat::SAY_YARDS, Clamp(1.0, 500.0), Live, "ListenRange.Say";
        /// Yards a yell carries, and a boss's emote.
        yell_yards: f32 = crate::game::map::chat::YELL_YARDS, Clamp(1.0, 1000.0), Live, "ListenRange.Yell";
        /// Yards an emote (`/wave`) carries.
        text_emote_yards: f32 = crate::game::map::chat::TEXT_EMOTE_YARDS, Clamp(1.0, 500.0), Live, "ListenRange.TextEmote";
        /// Lines in a row, each within `flood_message_delay_secs` of the
        /// last, that mute a player; 0 never mutes.
        flood_message_count: u32 = crate::game::map::chat::FLOOD_LINES, Clamp(0.0, 1000.0), Live, "ChatFlood.MessageCount";
        /// Seconds within which a line follows the last to count towards
        /// a flood.
        flood_message_delay_secs: u64 = crate::game::map::chat::FLOOD_GAP.as_secs(), Clamp(1.0, 60.0), Live, "ChatFlood.MessageDelay";
        /// Seconds a flooding player is muted.
        flood_mute_secs: u64 = crate::game::map::chat::FLOOD_MUTE.as_secs(), Clamp(1.0, 86_400.0), Live, "ChatFlood.MuteTime";
    }
}

settings! {
    /// Water and lava: how long a player has before they come to harm,
    /// and lava's burn.
    pub struct EnvironmentConfig in "environment" {
        /// Seconds of breath under water before drowning.
        breath_secs: u64 = crate::game::map::environment::BREATH.as_secs(), Clamp(1.0, 3600.0), Live, "MirrorTimer.Breath.Max";
        /// Seconds in the deep sea before fatigue.
        fatigue_secs: u64 = crate::game::map::environment::FATIGUE.as_secs(), Clamp(1.0, 3600.0), Live, "MirrorTimer.Fatigue.Max";
        /// Seconds touching lava before it burns.
        lava_secs: u64 = crate::game::map::environment::LAVA_GRACE.as_secs(), Clamp(0.0, 60.0), Live, "MirrorTimer.Environmental.Max";
        /// The least fire damage lava does every two seconds.
        lava_damage_min: u32 = 605, Clamp(0.0, 1_000_000.0), Live, "EnvironmentalDamage.Min";
        /// The most (no less than `lava_damage_min`).
        lava_damage_max: u32 = 610, Clamp(0.0, 1_000_000.0), Live, "EnvironmentalDamage.Max";
    }
}

settings! {
    /// Dungeon and raid copies.
    pub struct InstancesConfig in "instances" {
        /// Seconds an empty copy stays up. What it remembers (its dead,
        /// who went in) is kept at least thirty minutes after its last use,
        /// for its players to come back to.
        unload_delay_secs: u64 = crate::game::instances::UNLOAD_EMPTY_AFTER.as_secs(), Clamp(1.0, 86_400.0), Live, "Instance.UnloadDelay";
        /// The hour of the day, UTC, raids reset at, on their reset days.
        reset_hour: u32 = crate::game::instances::RESET_HOUR, Clamp(0.0, 23.0), Live, "Instance.ResetTimeHour";
        /// Copies an account may go into an hour; one it was in within the
        /// hour doesn't count again.
        entries_per_hour: usize = crate::game::instances::ENTRIES_AN_HOUR, Clamp(1.0, 1000.0), Live, "";
    }
}

settings! {
    /// The movement check: a player who moves faster than they can, over
    /// time, is pulled back. GMs are never checked.
    pub struct AnticheatConfig in "anticheat" {
        /// Check players' movement at all.
        speed_check: bool = true, Any, Live, "";
        /// How much faster than their speed a player may seem to go.
        speed_tolerance: f32 = 1.25, Clamp(1.0, 10.0), Live, "";
        /// Seconds of their speed a player may get ahead by: the lag their
        /// packets may bunch up by. Time earns it back; however many
        /// packets they send, they go no further.
        latency_allowance_secs: f32 = 2.0, Clamp(0.0, 5.0), Live, "";
        /// Yards a player may get ahead by besides, once, not per packet;
        /// and how far a client under a root may say it is before it is put
        /// back.
        distance_allowance_yards: f32 = 3.0, Clamp(0.0, 100.0), Live, "";
    }
}

settings! {
    /// Who may log in, and who is a GM.
    pub struct AccountsConfig in "accounts" {
        /// Every account is an administrator. For development and load
        /// tests only.
        everyone_gm: bool = false, Any, Restart, "";
        /// `auto`: an unknown name gets an account at its first login, with
        /// the name as its password (development and load tests).
        /// `existing`: only accounts made with `.account create`.
        creation: crate::accounts::Creation = crate::accounts::Creation::Auto, Any, Restart, "";
        /// GM levels by account: 1 moderator, 2 game master, 3
        /// administrator, as `{ ADMIN = 3 }`.
        gm: BTreeMap<String, u8> = BTreeMap::new(), Any, Restart, "";
    }
}

settings! {
    /// GM commands and GMs' characters.
    pub struct GmConfig in "gm" {
        /// The level a GM's new characters start at, if above
        /// `[players] start_level`.
        start_level: u8 = 1, Clamp(1.0, 60.0), Live, "GM.StartLevel";
        /// The GM level a command needs, by its name, over the server's own
        /// (0 player to 3 administrator), as `{ kick = 2, announce = 2 }`.
        command_levels: BTreeMap<String, u8> = BTreeMap::new(), Any, Live, "";
        /// Players' commands: `.help`, `.commands`, `.save`, `.start`,
        /// `.dismount`, `.server info|motd`, `.gm ingame`, `.account` and
        /// `.account password <old> <new> <new>`. Off, a player's "." line
        /// is chat.
        player_commands: bool = true, Any, Live, "PlayerCommands";
    }
}

settings! {
    /// Where new characters start, instead of their race's starting zone:
    /// peon's heartbeat placement assumes Gurubashi Arena's pit.
    pub struct SpawnConfig in "spawn" {
        /// Start new characters here.
        enabled: bool = false, Any, Restart, "";
        /// The map (0 Eastern Kingdoms, 1 Kalimdor).
        map: u32 = 0, Any, Restart, "";
        /// Where on the map, in yards.
        x: f32 = 0.0, Any, Restart, "";
        /// Where on the map, in yards.
        y: f32 = 0.0, Any, Restart, "";
        /// Where on the map, in yards.
        z: f32 = 0.0, Any, Restart, "";
        /// Facing, in radians.
        orientation: f32 = 0.0, Any, Restart, "";
    }
}

settings! {
    /// Gurubashi Arena's last man standing: `.arena start [countdown s]
    /// [max minutes]` starts one with everyone alive on the pit's floor.
    pub struct ArenaConfig in "arena" {
        /// Minutes between scheduled battles; 0 for only those a GM starts.
        every_minutes: u64 = 0, Clamp(0.0, crate::game::map::arena::MAX_MINUTES as f64), Live, "";
        /// Seconds of countdown before a battle.
        countdown_secs: u64 = 30, Clamp(0.0, crate::game::map::arena::MAX_SECS as f64), Live, "";
        /// Seconds into the fight the ring of fire starts closing.
        ring_after_secs: u64 = 120, Clamp(0.0, crate::game::map::arena::MAX_SECS as f64), Live, "";
        /// Seconds between the ring's steps.
        ring_every_secs: u64 = 20, Clamp(1.0, 3600.0), Live, "";
        /// Yards the ring closes by at each step.
        ring_step_yards: f32 = 5.0, Any, Live, "";
        /// The ring's smallest radius.
        ring_min_yards: f32 = 6.0, Any, Live, "";
        /// The longest a battle runs: then the ring is at its smallest and
        /// burns its hardest.
        max_minutes: u64 = 20, Clamp(0.0, crate::game::map::arena::MAX_MINUTES as f64), Live, "";
        /// The champion's prize in copper.
        reward_copper: u32 = 250_000, Any, Live, "";
        /// The champion's item (Arena Grand Master).
        reward_item: u32 = 19024, Any, Live, "";
        /// The champion's buff (Spirit of Zandalar).
        champion_spell: u32 = 24425, Any, Live, "";
        /// Fighters a battle needs to start.
        min_fighters: u32 = 2, Any, Live, "";
        /// Seconds a fighter may stand off the pit's floor before they
        /// have fled.
        off_floor_secs: u64 = 5, Clamp(0.0, 600.0), Live, "";
        /// Seconds the fallen lie before their ghosts go up to the stands.
        lie_secs: u64 = 5, Clamp(0.0, 600.0), Live, "";
        /// Seconds within which a fighter's kills are a double, triple or
        /// more.
        multi_kill_secs: u64 = 10, Clamp(0.0, 600.0), Live, "";
        /// Seconds between the announcer's counts of the fighters left,
        /// when no one falls.
        roll_call_secs: u64 = 60, Clamp(5.0, 3600.0), Live, "";
        /// Seconds a player keeps the free-for-all flag after walking out
        /// of an arena's area (Gurubashi's pit), so the edge can't be
        /// teased; back in within them, it carries on. A death or a
        /// teleport ends them. 0 drops the flag at once, as mangos does.
        ffa_grace_secs: u64 = 20, Clamp(0.0, 3600.0), Live, "";
    }
}

/// `v` at `rate`, rounded down (as mangos scales); at 1, `v` itself.
pub fn scaled(v: u32, rate: f32) -> u32 {
    if rate == 1.0 {
        v
    } else {
        (v as f64 * f64::from(rate)) as u32
    }
}

/// The whole config: a field per section, named as its section.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub network: NetworkConfig,
    pub world: WorldConfig,
    pub visibility: VisibilityConfig,
    pub chat: ChatConfig,
    pub rates: RatesConfig,
    pub creatures: CreaturesConfig,
    pub guards: GuardsConfig,
    pub players: PlayersConfig,
    pub environment: EnvironmentConfig,
    pub quests: QuestsConfig,
    pub instances: InstancesConfig,
    pub battleground: BattlegroundConfig,
    pub outdoor_pvp: OutdoorPvpConfig,
    pub events: EventsConfig,
    pub anticheat: AnticheatConfig,
    pub accounts: AccountsConfig,
    pub gm: GmConfig,
    pub spawn: SpawnConfig,
    pub arena: ArenaConfig,
}

/// Every section, in the order they are printed.
pub const SECTIONS: &[&Section] = &[
    &NetworkConfig::SECTION,
    &WorldConfig::SECTION,
    &VisibilityConfig::SECTION,
    &ChatConfig::SECTION,
    &RatesConfig::SECTION,
    &CreaturesConfig::SECTION,
    &GuardsConfig::SECTION,
    &PlayersConfig::SECTION,
    &EnvironmentConfig::SECTION,
    &QuestsConfig::SECTION,
    &InstancesConfig::SECTION,
    &BattlegroundConfig::SECTION,
    &OutdoorPvpConfig::SECTION,
    &EventsConfig::SECTION,
    &AnticheatConfig::SECTION,
    &AccountsConfig::SECTION,
    &GmConfig::SECTION,
    &SpawnConfig::SECTION,
    &ArenaConfig::SECTION,
];

pub fn section(name: &str) -> Option<&'static Section> {
    SECTIONS.iter().copied().find(|s| s.name == name)
}

/// A key's place: its section and its name.
pub type Place = (&'static str, &'static str);

/// Keys that moved, by their old place: still read there, into their new
/// ones.
pub const MOVED: &[(Place, &[Place])] = &[
    (
        ("world", "visibility_yards"),
        &[
            ("visibility", "continents_yards"),
            ("visibility", "instances_yards"),
        ],
    ),
    (
        ("world", "battleground_visibility_yards"),
        &[("visibility", "battleground_yards")],
    ),
    (
        ("world", "max_characters_per_account"),
        &[("players", "characters_per_realm")],
    ),
    (("world", "pvp_realm"), &[("players", "pvp_realm")]),
];

fn moved(section: &str, key: &str) -> Option<&'static [Place]> {
    MOVED
        .iter()
        .find(|((s, k), _)| *s == section && *k == key)
        .map(|(_, to)| *to)
}

/// A config as loaded, with what was clamped.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: ServerConfig,
    /// Values out of range, and what was used instead.
    pub warnings: Vec<String>,
}

/// Read `path` as TOML, with the environment's `WVS__` variables over it,
/// and log what was clamped. A missing file gives the defaults; one that
/// doesn't parse, an unknown key or variable, or a value refused, is an
/// error, so a typo isn't silently ignored.
pub fn load(path: &Path) -> Result<ServerConfig, String> {
    let loaded = load_full(path)?;
    for w in &loaded.warnings {
        tracing::warn!("{w}");
    }
    Ok(loaded.config)
}

/// [`load`], the warnings returned rather than logged.
pub fn load_full(path: &Path) -> Result<Loaded, String> {
    load_noting_blank(path).map(|(loaded, _)| loaded)
}

/// [`load_full`], and whether the file was missing or empty: at a reload
/// that is more likely a file gone, or one an editor has truncated to
/// write, than one meant to be the defaults.
pub fn load_noting_blank(path: &Path) -> Result<(Loaded, bool), String> {
    let origin = path.display().to_string();
    let text = match std::fs::read_to_string(path) {
        Ok(s) => {
            tracing::info!("loaded config from {origin}");
            s
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("no config file at {origin}; using defaults");
            String::new()
        }
        // There, but unreadable: an error, not the defaults (another
        // character store, everyone's accounts made at first login).
        Err(e) => return Err(format!("{origin}: {e}")),
    };
    let mut loaded = resolve(&text, &origin, &env_overrides()?)?;
    loaded.config.world.default_dirs();
    Ok((loaded, text.trim().is_empty()))
}

/// The process's `WVS__` variables.
pub fn env_overrides() -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (k, v) in std::env::vars_os() {
        let Some(k) = k.to_str().filter(|k| k.starts_with(ENV_PREFIX)) else {
            continue;
        };
        let v = v
            .into_string()
            .map_err(|_| format!("{k}: its value isn't UTF-8"))?;
        out.push((k.to_owned(), v));
    }
    out.sort();
    Ok(out)
}

/// The config from a file's `text` (from `origin`, for messages) and the
/// `WVS__` variables `env`: the defaults, the file over them, the
/// environment over that; then every value checked.
pub fn resolve(text: &str, origin: &str, env: &[(String, String)]) -> Result<Loaded, String> {
    let mut table: toml::Table = text.parse().map_err(|e| format!("{origin}: {e}"))?;
    move_old_keys(&mut table, origin)?;
    known(&table, origin)?;
    let defaults = defaults_table();
    for (name, raw) in env {
        for (path, value) in env_value(name, raw, &defaults)? {
            set(&mut table, &path, value);
        }
    }
    let mut warnings = Vec::new();
    check(&mut table, &mut warnings)?;
    let mut config: ServerConfig = toml::Value::Table(table)
        .try_into()
        .map_err(|e| format!("{origin}: {e}"))?;
    cross_check(&mut config, &mut warnings)?;
    Ok(Loaded { config, warnings })
}

/// Old keys into their new places; both set is an error.
fn move_old_keys(table: &mut toml::Table, origin: &str) -> Result<(), String> {
    for ((s, k), to) in MOVED {
        let Some(v) = table
            .get_mut(*s)
            .and_then(|t| t.as_table_mut())
            .and_then(|t| t.remove(*k))
        else {
            continue;
        };
        for (ns, nk) in *to {
            let target = table
                .entry(*ns)
                .or_insert_with(|| toml::Value::Table(Default::default()));
            let Some(target) = target.as_table_mut() else {
                return Err(format!("{origin}: [{ns}] must be a table"));
            };
            if target.contains_key(*nk) {
                return Err(format!(
                    "{origin}: [{s}] {k} is the old name of [{ns}] {nk}, and both are set: keep [{ns}] {nk}"
                ));
            }
            target.insert((*nk).to_owned(), v.clone());
        }
        tracing::info!(
            "{origin}: [{s}] {k} is now {}; read there",
            to.iter()
                .map(|(ns, nk)| format!("[{ns}] {nk}"))
                .collect::<Vec<_>>()
                .join(" and ")
        );
    }
    Ok(())
}

/// Every section and key in `table` is one there is.
fn known(table: &toml::Table, origin: &str) -> Result<(), String> {
    for (s, v) in table {
        let Some(sec) = section(s) else {
            return Err(format!(
                "{origin}: there is no section [{s}] (the sections: {})",
                SECTIONS
                    .iter()
                    .map(|s| s.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let Some(t) = v.as_table() else {
            return Err(format!("{origin}: {s} must be a section, [{s}]"));
        };
        for k in t.keys() {
            if sec.key(k).is_none() {
                return Err(format!("{origin}: there is no key [{s}] {k}"));
            }
        }
    }
    Ok(())
}

/// The defaults, as TOML.
pub fn defaults_table() -> toml::Table {
    toml::Table::try_from(ServerConfig::default()).expect("the defaults serialize")
}

fn set(table: &mut toml::Table, path: &[String], value: toml::Value) {
    let (last, parents) = path.split_last().expect("a path has a key");
    let mut t = table;
    for p in parents {
        let entry = t
            .entry(p.clone())
            .or_insert_with(|| toml::Value::Table(Default::default()));
        if !entry.is_table() {
            *entry = toml::Value::Table(Default::default());
        }
        t = entry.as_table_mut().expect("a table");
    }
    t.insert(last.clone(), value);
}

/// What `WVS__SECTION__KEY[__MORE]=raw` sets: the paths (a moved key sets
/// all its new places) and the value, read as the key's type.
fn env_value(
    name: &str,
    raw: &str,
    defaults: &toml::Table,
) -> Result<Vec<(Vec<String>, toml::Value)>, String> {
    let rest = name.strip_prefix(ENV_PREFIX).unwrap_or(name);
    let parts: Vec<&str> = rest.split("__").collect();
    if parts.len() < 2 || parts.iter().any(|p| p.is_empty()) {
        return Err(format!(
            "{name}: an override is {ENV_PREFIX}SECTION__KEY, as WVS__RATES__XP_KILL"
        ));
    }
    let (s, k) = (parts[0].to_ascii_lowercase(), parts[1].to_ascii_lowercase());
    let deeper: Vec<String> = parts[2..].iter().map(|p| (*p).to_owned()).collect();
    let places: Vec<(String, String)> = match moved(&s, &k) {
        Some(to) => to
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect(),
        None => {
            if section(&s).and_then(|sec| sec.key(&k)).is_none() {
                return Err(format!("{name}: there is no key [{s}] {k}"));
            }
            vec![(s, k)]
        }
    };
    let mut out = Vec::new();
    for (s, k) in places {
        let default = defaults.get(&s).and_then(|t| t.get(&k));
        let value = if deeper.is_empty() {
            typed(name, raw, default)?
        } else {
            if default.is_some_and(|d| !d.is_table()) {
                return Err(format!("{name}: [{s}] {k} has no keys within it"));
            }
            inline(raw).unwrap_or_else(|| toml::Value::String(raw.to_owned()))
        };
        let mut path = vec![s, k];
        path.extend(deeper.iter().cloned());
        out.push((path, value));
    }
    Ok(out)
}

/// `raw` as TOML's own value, if it is one (`[1, 2]`, `{ a = 1 }`, `"x"`).
fn inline(raw: &str) -> Option<toml::Value> {
    format!("v = {raw}")
        .parse::<toml::Table>()
        .ok()
        .and_then(|mut t| t.remove("v"))
}

/// `raw` as the type of the key's `default` (a string if it has none).
fn typed(name: &str, raw: &str, default: Option<&toml::Value>) -> Result<toml::Value, String> {
    use toml::Value as V;
    let t = raw.trim();
    Ok(match default {
        None | Some(V::String(_)) => V::String(raw.to_owned()),
        Some(V::Integer(_)) => V::Integer(
            t.parse()
                .map_err(|_| format!("{name}: `{raw}` isn't a whole number"))?,
        ),
        Some(V::Float(_)) => V::Float(
            t.parse()
                .map_err(|_| format!("{name}: `{raw}` isn't a number"))?,
        ),
        Some(V::Boolean(_)) => V::Boolean(match t.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" => false,
            _ => return Err(format!("{name}: `{raw}` isn't true or false")),
        }),
        Some(d @ (V::Array(_) | V::Table(_))) => match inline(t) {
            Some(v) if std::mem::discriminant(&v) == std::mem::discriminant(d) => v,
            _ => {
                let what = if d.is_array() {
                    "a list, as [\"a\", \"b\"]"
                } else {
                    "a table, as { a = 1 }"
                };
                return Err(format!("{name}: `{raw}` isn't {what}"));
            }
        },
        Some(V::Datetime(_)) => inline(t).ok_or_else(|| format!("{name}: `{raw}` isn't a date"))?,
    })
}

/// Each number within its key's range: clamped, with a warning, or
/// refused.
fn check(table: &mut toml::Table, warnings: &mut Vec<String>) -> Result<(), String> {
    for sec in SECTIONS {
        let Some(t) = table.get_mut(sec.name).and_then(|t| t.as_table_mut()) else {
            continue;
        };
        for key in sec.keys {
            let Some(value) = t.get_mut(key.name) else {
                continue;
            };
            let v = match value {
                toml::Value::Integer(i) => *i as f64,
                toml::Value::Float(f) => *f,
                _ => continue,
            };
            let at = format!("[{}] {}", sec.name, key.name);
            if v.is_nan() {
                return Err(format!("{at} isn't a number"));
            }
            let (lo, hi, clamp) = match key.check {
                Check::Any => continue,
                Check::Clamp(lo, hi) => (lo, hi, true),
                Check::Refuse(lo, hi) => (lo, hi, false),
            };
            if (lo..=hi).contains(&v) {
                continue;
            }
            if !clamp {
                return Err(format!(
                    "{at} = {} is outside its range, {} to {}",
                    num(v),
                    num(lo),
                    num(hi)
                ));
            }
            let c = v.clamp(lo, hi);
            warnings.push(format!(
                "{at} = {} is outside its range, {} to {}: using {}",
                num(v),
                num(lo),
                num(hi),
                num(c)
            ));
            *value = match value {
                toml::Value::Integer(_) => toml::Value::Integer(c as i64),
                _ => toml::Value::Float(c),
            };
        }
    }
    Ok(())
}

/// What one key's range can't say.
fn cross_check(c: &mut ServerConfig, warnings: &mut Vec<String>) -> Result<(), String> {
    let max = c.players.max_level;
    for (at, level) in [
        ("[players] start_level", &mut c.players.start_level),
        ("[gm] start_level", &mut c.gm.start_level),
    ] {
        if *level > max {
            warnings.push(format!(
                "{at} = {level} is above [players] max_level, {max}: using {max}"
            ));
            *level = max;
        }
    }
    let env = &mut c.environment;
    if env.lava_damage_max < env.lava_damage_min {
        warnings.push(format!(
            "[environment] lava_damage_max = {} is below lava_damage_min, {}: using {}",
            env.lava_damage_max, env.lava_damage_min, env.lava_damage_min
        ));
        env.lava_damage_max = env.lava_damage_min;
    }
    for (command, level) in &c.gm.command_levels {
        if *level > crate::accounts::ADMINISTRATOR {
            return Err(format!(
                "[gm] command_levels: {command} = {level} is outside 0 to 3"
            ));
        }
    }
    Ok(())
}

/// `cfg` as commented TOML: every key, its value, its default, its range,
/// whether a reload changes it, its mangos name, and its meaning. It
/// parses back to `cfg`.
pub fn render(cfg: &ServerConfig, title: &str) -> String {
    let values = toml::Table::try_from(cfg).expect("the config serializes");
    let defaults = defaults_table();
    let mut out = String::new();
    for line in title.lines() {
        out.push_str(format!("# {line}").trim_end());
        out.push('\n');
    }
    for sec in SECTIONS {
        out.push('\n');
        comment(&mut out, sec.doc);
        out.push_str(&format!("[{}]\n", sec.name));
        let vals = values.get(sec.name).and_then(|v| v.as_table());
        let defs = defaults.get(sec.name).and_then(|v| v.as_table());
        for key in sec.keys {
            out.push('\n');
            comment(&mut out, key.doc);
            let default = defs
                .and_then(|t| t.get(key.name))
                .map_or("unset".to_owned(), value_text);
            let mut meta = format!("Default {default}.");
            if let Some(range) = key.check.describe() {
                meta.push_str(&format!(" Range {range}."));
            }
            meta.push_str(match key.reload {
                Reload::Live => " Changes at a reload.",
                Reload::Restart => " Needs a restart.",
            });
            if !key.mangos.is_empty() {
                meta.push_str(&format!(" mangos: {}.", key.mangos));
            }
            out.push_str(&format!("# {meta}\n"));
            match vals.and_then(|t| t.get(key.name)) {
                Some(v) => out.push_str(&format!("{} = {}\n", key.name, value_text(v))),
                None => out.push_str(&format!("#{} =\n", key.name)),
            }
        }
    }
    out
}

/// The reference: every key at its default (`config.example.toml`).
pub fn reference() -> String {
    render(
        &ServerConfig::default(),
        "wow_vanilla_server's settings: every key, at its default.\n\
         \n\
         Generated by `wow_vanilla_server --print-config --defaults`; a test fails when it drifts.\n\
         Copy what you change into config.toml. Any key may be set from the environment as\n\
         WVS__SECTION__KEY=value (WVS__RATES__XP_KILL=2), over the file. See docs/CONFIG.md.",
    )
}

fn comment(out: &mut String, doc: &str) {
    for line in doc.lines() {
        let line = line.strip_prefix(' ').unwrap_or(line);
        out.push_str(format!("# {line}").trim_end());
        out.push('\n');
    }
}

/// A value as TOML writes it; a float at the shortest that reads back to
/// the same `f32` (every float in the config is one).
pub fn value_text(v: &toml::Value) -> String {
    use toml::Value as V;
    match v {
        V::Float(f) => format!("{:?}", *f as f32),
        V::Array(a) => format!(
            "[{}]",
            a.iter().map(value_text).collect::<Vec<_>>().join(", ")
        ),
        V::Table(t) if t.is_empty() => "{}".to_owned(),
        V::Table(t) => format!(
            "{{ {} }}",
            t.iter()
                .map(|(k, v)| format!("{} = {}", key_text(k), value_text(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

fn key_text(k: &str) -> String {
    if !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        k.to_owned()
    } else {
        toml::Value::String(k.to_owned()).to_string()
    }
}

/// Each key's value, as `section.key`.
fn flatten(cfg: &ServerConfig) -> BTreeMap<(&'static str, &'static str), Option<toml::Value>> {
    let t = toml::Table::try_from(cfg).expect("the config serializes");
    let mut out = BTreeMap::new();
    for sec in SECTIONS {
        let st = t.get(sec.name).and_then(|v| v.as_table());
        for key in sec.keys {
            out.insert(
                (sec.name, key.name),
                st.and_then(|s| s.get(key.name)).cloned(),
            );
        }
    }
    out
}

/// The keys whose values differ between `old` and `new`, as `[section] key`,
/// with when they take effect.
pub fn changes(old: &ServerConfig, new: &ServerConfig) -> Vec<(String, Reload)> {
    let (a, b) = (flatten(old), flatten(new));
    let mut out = Vec::new();
    for sec in SECTIONS {
        for key in sec.keys {
            let at = (sec.name, key.name);
            if a.get(&at) != b.get(&at) {
                out.push((format!("[{}] {}", sec.name, key.name), key.reload));
            }
        }
    }
    out
}

/// `new`, but with every key that needs a restart as `running` has it.
pub fn live_part(running: &ServerConfig, new: &ServerConfig) -> ServerConfig {
    let mut t = toml::Table::try_from(new).expect("the config serializes");
    let old = toml::Table::try_from(running).expect("the config serializes");
    for sec in SECTIONS {
        for key in sec.keys.iter().filter(|k| k.reload == Reload::Restart) {
            let was = old.get(sec.name).and_then(|s| s.get(key.name)).cloned();
            let Some(s) = t.get_mut(sec.name).and_then(|s| s.as_table_mut()) else {
                continue;
            };
            match was {
                Some(v) => s.insert(key.name.to_owned(), v),
                None => s.remove(key.name),
            };
        }
    }
    toml::Value::Table(t)
        .try_into()
        .expect("a mix of two configs is one")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn parse(text: &str) -> Result<ServerConfig, String> {
        resolve(text, "test.toml", &[]).map(|l| l.config)
    }

    #[test]
    fn the_checked_in_config_parses() {
        let text =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("config.toml"))
                .unwrap();
        parse(&text).unwrap();
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        let e = parse("[world]\nvisibilty_yards = 3\n").unwrap_err();
        assert!(e.contains("[world] visibilty_yards"), "{e}");
        assert!(parse("[wrold]\nmotd = \"x\"\n").is_err());
        assert!(parse("motd = \"x\"\n").is_err(), "a key outside a section");
    }

    #[test]
    fn an_unknown_variable_is_an_error() {
        let e = resolve("", "t", &env(&[("WVS__RATES__XP_KIL", "2")])).unwrap_err();
        assert!(e.contains("WVS__RATES__XP_KIL"), "{e}");
        assert!(resolve("", "t", &env(&[("WVS__NOPE__X", "2")])).is_err());
        assert!(resolve("", "t", &env(&[("WVS__RATES", "2")])).is_err());
        let e = resolve("", "t", &env(&[("WVS__RATES__XP_KILL", "lots")])).unwrap_err();
        assert!(e.contains("WVS__RATES__XP_KILL"), "{e}");
    }

    /// The defaults, the file over them, the environment over the file.
    #[test]
    fn the_environment_wins_over_the_file_over_the_defaults() {
        let file = "[rates]\nxp_kill = 2\nxp_quest = 3\n[world]\nmotd = \"file\"\n";
        let c = resolve(
            file,
            "t",
            &env(&[
                ("WVS__RATES__XP_QUEST", "4.5"),
                ("WVS__VISIBILITY__BATTLEGROUND_YARDS", "300"),
                ("WVS__WORLD__MOTD", "from the environment"),
                ("WVS__ANTICHEAT__SPEED_CHECK", "false"),
                ("WVS__ACCOUNTS__GM__ADMIN", "3"),
                ("WVS__WORLD__RESERVED_NAMES", "[\"Thrall\"]"),
            ]),
        )
        .unwrap()
        .config;
        assert_eq!(c.rates.xp_kill, 2.0, "the file's");
        assert_eq!(c.rates.xp_quest, 4.5, "the environment's, over the file's");
        assert_eq!(c.rates.xp_explore, 1.0, "the default");
        assert_eq!(c.visibility.battleground_yards, 300.0);
        assert_eq!(c.world.motd, "from the environment");
        assert!(!c.anticheat.speed_check);
        assert_eq!(c.accounts.gm.get("ADMIN"), Some(&3));
        assert_eq!(c.world.reserved_names, vec!["Thrall".to_owned()]);
    }

    #[test]
    fn a_value_out_of_range_is_clamped_with_a_warning() {
        let l = resolve(
            "[visibility]\ncontinents_yards = 5000\n[rates]\nxp_kill = -1\n",
            "t",
            &env(&[("WVS__WORLD__HONOR_RESET_WEEKDAY", "9")]),
        )
        .unwrap();
        assert_eq!(l.config.visibility.continents_yards, 500.0);
        assert_eq!(l.config.rates.xp_kill, 0.0);
        assert_eq!(l.config.world.honor_reset_weekday, 7);
        assert_eq!(l.warnings.len(), 3, "{:?}", l.warnings);
        assert!(
            l.warnings
                .iter()
                .any(|w| w.contains("[visibility] continents_yards") && w.contains("20 to 500")),
            "{:?}",
            l.warnings
        );
        // A start above the cap is the cap.
        let l = resolve("[players]\nmax_level = 20\nstart_level = 30\n", "t", &[]).unwrap();
        assert_eq!(l.config.players.start_level, 20);
        assert!(l.warnings[0].contains("max_level"), "{:?}", l.warnings);
    }

    #[test]
    fn a_value_refused_is_an_error() {
        let e = parse("[gm]\ncommand_levels = { kick = 7 }\n").unwrap_err();
        assert!(e.contains("kick") && e.contains("0 to 3"), "{e}");
        assert!(parse("[rates]\nxp_kill = nan\n").is_err());
        assert!(
            parse("[rates]\nxp_kill = \"two\"\n").is_err(),
            "a type wrong"
        );
        // Every refusing range names the key, as a clamp does.
        for sec in SECTIONS {
            for key in sec.keys {
                if let Check::Refuse(_, hi) = key.check {
                    let e = parse(&format!("[{}]\n{} = {}\n", sec.name, key.name, hi + 1.0))
                        .unwrap_err();
                    assert!(e.contains(key.name), "{e}");
                }
            }
        }
    }

    /// The `config.toml` that shipped before the sections: its keys still
    /// load, into their new places.
    #[test]
    fn the_old_keys_still_load() {
        let old = "[world]\nvisibility_yards = 150.0\nbattleground_visibility_yards = 300.0\n\
                   pvp_realm = true\nmax_characters_per_account = 4\nsave_interval_secs = 60\n\
                   motd = \"old\"\npetition_signatures = 9\nmail_delay_secs = 3600\n\
                   honor_reset_weekday = 2\ncharacters_db = \"characters.sqlite\"\n";
        let c = parse(old).unwrap();
        assert_eq!(c.visibility.continents_yards, 150.0);
        assert_eq!(c.visibility.instances_yards, 150.0);
        assert_eq!(c.visibility.battleground_yards, 300.0);
        assert!(c.players.pvp_realm);
        assert_eq!(c.players.characters_per_realm, 4);
        assert_eq!(c.world.save_interval_secs, 60);
        // Through the environment, too.
        let c = resolve(
            "",
            "t",
            &env(&[("WVS__WORLD__BATTLEGROUND_VISIBILITY_YARDS", "250")]),
        )
        .unwrap()
        .config;
        assert_eq!(c.visibility.battleground_yards, 250.0);
        // Old and new both set: which is meant isn't clear.
        let e = parse("[world]\nvisibility_yards = 1.0\n[visibility]\ncontinents_yards = 2.0\n")
            .unwrap_err();
        assert!(e.contains("[world] visibility_yards"), "{e}");
    }

    /// `--print-config`'s output reads back to the same config, whatever
    /// is in it.
    #[test]
    fn the_printed_config_parses_back_to_the_same() {
        let mut c = ServerConfig::default();
        c.rates.xp_kill = 2.5;
        c.spawn.z = 21.857;
        c.world.data_dir = Some(PathBuf::from("some/where"));
        c.world.motd = "a \"quoted\" line\nand another".into();
        c.world.reserved_names = vec!["Thrall".into(), "Jaina".into()];
        c.accounts.gm.insert("ADMIN".into(), 3);
        c.accounts.gm.insert("odd name".into(), 1);
        c.accounts.creation = crate::accounts::Creation::Existing;
        c.gm.command_levels.insert("kick".into(), 2);
        let text = render(&c, "test");
        assert_eq!(parse(&text).unwrap(), c, "{text}");
        assert_eq!(parse(&reference()).unwrap(), ServerConfig::default());
    }

    /// `config.example.toml` is what the code generates.
    #[test]
    fn the_reference_matches_the_code() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
        let shipped = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            shipped == reference(),
            "config.example.toml is out of date: run \
             `cargo run --bin wow_vanilla_server -- --print-config --defaults > config.example.toml`"
        );
    }

    /// docs/CONFIG.md's tables have every section and key.
    #[test]
    fn the_docs_list_every_key() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/CONFIG.md");
        let docs = std::fs::read_to_string(path).unwrap();
        for sec in SECTIONS {
            let at = docs
                .find(&format!("### `[{}]`", sec.name))
                .unwrap_or_else(|| panic!("docs/CONFIG.md has no [{}]", sec.name));
            let table = &docs[at..];
            let table = &table[..table[4..].find("\n### ").map_or(table.len(), |e| e + 4)];
            for key in sec.keys {
                assert!(
                    table.contains(&format!("| `{}` |", key.name)),
                    "docs/CONFIG.md's [{}] has no {}",
                    sec.name,
                    key.name
                );
            }
        }
    }

    /// Every section is described, and every key a section serializes.
    #[test]
    fn every_key_is_described() {
        let t = defaults_table();
        assert_eq!(
            t.keys().collect::<Vec<_>>().len(),
            SECTIONS.len(),
            "a section without its description"
        );
        for (s, v) in &t {
            let sec = section(s).unwrap_or_else(|| panic!("[{s}] undescribed"));
            for k in v.as_table().unwrap().keys() {
                assert!(sec.key(k).is_some(), "[{s}] {k} undescribed");
            }
            for key in sec.keys {
                assert!(!key.doc.trim().is_empty(), "[{s}] {}", key.name);
            }
        }
        for ((s, k), to) in MOVED {
            assert!(
                section(s).and_then(|x| x.key(k)).is_none(),
                "{s}.{k} is both"
            );
            for (ns, nk) in *to {
                assert!(section(ns).and_then(|x| x.key(nk)).is_some(), "{ns}.{nk}");
            }
        }
    }

    #[test]
    fn a_reload_keeps_what_needs_a_restart() {
        let running = ServerConfig::default();
        let mut new = running.clone();
        new.rates.xp_kill = 2.0;
        new.world.characters_db = PathBuf::from("elsewhere.sqlite");
        new.network.max_packets_per_sec = 5000;
        let changed = changes(&running, &new);
        assert_eq!(
            changed,
            vec![
                ("[network] max_packets_per_sec".to_owned(), Reload::Restart),
                ("[world] characters_db".to_owned(), Reload::Restart),
                ("[rates] xp_kill".to_owned(), Reload::Live),
            ]
        );
        let applied = live_part(&running, &new);
        assert_eq!(applied.rates.xp_kill, 2.0);
        assert_eq!(applied.world.characters_db, running.world.characters_db);
        assert_eq!(applied.network.max_packets_per_sec, 100);
    }

    /// No file is the defaults; a file there but unreadable (here a
    /// directory, as a permission or a bad byte would be) is an error, not
    /// the defaults.
    #[test]
    fn a_config_that_cant_be_read_is_an_error() {
        let dir = std::env::temp_dir().join(format!("wvs-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load(&dir.join("absent.toml")).is_ok(), "none: the defaults");
        assert!(load(&dir).is_err(), "unreadable: an error");
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, [0xFF, 0xFE, b'\n']).unwrap();
        assert!(load(&bad).is_err(), "not UTF-8: an error");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
