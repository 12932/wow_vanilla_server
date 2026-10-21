//! vmangos's scripting data: EventAI events (`creature_ai_events`), the DB
//! script tables (creature AI, generic, event, spell, quest, gossip,
//! movement, object and area trigger scripts, all in one step format),
//! conditions, creature groups and links, spawn pools, and the maps'
//! instance scripts.

use ahash::AHashMap;

/// One step of a DB script.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ScriptStep {
    pub id: u32,
    /// After the script starts, in milliseconds.
    pub delay_ms: u32,
    pub priority: u32,
    pub command: u32,
    pub datalong: [u32; 4],
    pub dataint: [i32; 4],
    pub target_type: u32,
    pub target_param1: u32,
    pub target_param2: u32,
    pub data_flags: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub o: f32,
    pub condition: u32,
    /// What only a code-built step has (Thistle Tea's creature scripts).
    pub ext: Option<Box<StepExt>>,
}

/// A code-built step's extras, as Thistle Tea's `ScriptStep` has them.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct StepExt {
    /// Lines to say, one at random, in place of broadcast texts.
    pub texts: Vec<InlineText>,
    /// A summon's places: one no living creature stands on, at random.
    pub positions: Vec<[f32; 4]>,
    /// A summon lands on a random point this far around its place.
    pub scatter: f32,
    /// How many a summon brings.
    pub count: SummonCount,
    /// A summon or teleport at the target's feet.
    pub at_target: bool,
    /// A group join keeps the creature's distance and bearing from its
    /// leader.
    pub formation_from_position: bool,
    /// The target must be farther than this, flat, past both bounding
    /// radii.
    pub min_target_distance_2d: Option<f32>,
    /// A summon comes hidden from players, until a script shows it.
    pub concealed: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct InlineText {
    pub text: String,
    pub chat_type: u32,
    pub language: u32,
    pub emote: u32,
}

/// A summon's count: a number, or one that scales with the living
/// players on the summoner's threat list (ratio, at least, at most).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub enum SummonCount {
    Fixed(u32),
    ThreatPlayers(f32, u32, u32),
}

impl Default for SummonCount {
    fn default() -> Self {
        SummonCount::Fixed(1)
    }
}

/// `data_flags`.
pub const SF_SWAP_INITIAL_TARGETS: u32 = 0x1;
pub const SF_SWAP_FINAL_TARGETS: u32 = 0x2;
pub const SF_TARGET_SELF: u32 = 0x4;
pub const SF_ABORT_ON_FAILURE: u32 = 0x8;

/// One EventAI event of a creature.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct AiEvent {
    pub id: u32,
    pub creature: u32,
    pub condition: u32,
    pub kind: u32,
    pub inverse_phase_mask: u32,
    pub chance: u32,
    pub flags: u32,
    pub params: [i32; 4],
    /// `creature_ai_scripts` ids.
    pub actions: [u32; 3],
}

/// `event_flags`.
pub const EF_REPEATABLE: u32 = 0x1;
pub const EF_RANDOM_ACTION: u32 = 0x2;
pub const EF_NOT_CASTING: u32 = 0x4;
pub const EF_CHECK_RESULT: u32 = 0x8;
/// Thistle Tea's: fires only with its victim in melee reach, and its timer
/// waits while it is stunned.
pub const EF_VICTIM_IN_MELEE: u32 = 0x100;
pub const EF_PAUSE_WHILE_STUNNED: u32 = 0x200;

// Event types.
pub const EVENT_TIMER_IN_COMBAT: u32 = 0;
pub const EVENT_TIMER_OOC: u32 = 1;
pub const EVENT_HP: u32 = 2;
pub const EVENT_MANA: u32 = 3;
pub const EVENT_AGGRO: u32 = 4;
pub const EVENT_KILL: u32 = 5;
pub const EVENT_DEATH: u32 = 6;
pub const EVENT_EVADE: u32 = 7;
pub const EVENT_HIT_BY_SPELL: u32 = 8;
pub const EVENT_RANGE: u32 = 9;
pub const EVENT_OOC_LOS: u32 = 10;
pub const EVENT_SPAWNED: u32 = 11;
pub const EVENT_TARGET_HP: u32 = 12;
pub const EVENT_FRIENDLY_HP: u32 = 14;
pub const EVENT_FRIENDLY_IS_CC: u32 = 15;
pub const EVENT_FRIENDLY_MISSING_BUFF: u32 = 16;
pub const EVENT_SUMMONED_UNIT: u32 = 17;
pub const EVENT_TARGET_MANA: u32 = 18;
pub const EVENT_REACHED_HOME: u32 = 21;
pub const EVENT_RECEIVE_EMOTE: u32 = 22;
pub const EVENT_AURA: u32 = 23;
pub const EVENT_TARGET_AURA: u32 = 24;
pub const EVENT_SUMMONED_JUST_DIED: u32 = 25;
pub const EVENT_SUMMONED_JUST_DESPAWN: u32 = 26;
pub const EVENT_MISSING_AURA: u32 = 27;
pub const EVENT_TARGET_MISSING_AURA: u32 = 28;
pub const EVENT_MOVEMENT_INFORM: u32 = 29;
pub const EVENT_LEAVE_COMBAT: u32 = 30;
pub const EVENT_SCRIPT_EVENT: u32 = 31;
pub const EVENT_GROUP_MEMBER_DIED: u32 = 32;
pub const EVENT_VICTIM_ROOTED: u32 = 33;
pub const EVENT_SPELL_HIT_TARGET: u32 = 36;

/// The event types the engine runs (all but 13, 19, 20, 34 and 35, as
/// Thistle Tea).
pub fn event_supported(kind: u32) -> bool {
    matches!(kind, 0..=12 | 14..=18 | 21..=33 | 36)
}

/// A condition (`conditions`).
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Condition {
    pub entry: u32,
    pub kind: i32,
    pub values: [i32; 4],
    pub flags: u32,
}

pub const CF_REVERSE_RESULT: u32 = 0x1;
pub const CF_SWAP_TARGETS: u32 = 0x2;

/// A creature group's member (`creature_groups`).
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct GroupMember {
    pub guid: u32,
    pub distance: f32,
    pub angle: f32,
    pub flags: u32,
}

/// Group flags.
pub const GROUP_FORMATION: u32 = 0x1;
pub const GROUP_AGGRO_TOGETHER: u32 = 0x2;
pub const GROUP_EVADE_TOGETHER: u32 = 0x4;
pub const GROUP_RESPAWN_TOGETHER: u32 = 0x8;
/// Its dead respawn when its leader evades (or one does while the leader
/// lives and they evade together); when any of it evades.
pub const GROUP_RESPAWN_ON_LEADER_EVADE: u32 = 0x10;
pub const GROUP_RESPAWN_ON_EVADE: u32 = 0x20;
/// Its leader hears of a member's death; its members hear of any death.
pub const GROUP_INFORM_LEADER_ON_MEMBER_DIED: u32 = 0x40;
pub const GROUP_INFORM_MEMBERS_ON_ANY_DIED: u32 = 0x80;

/// `creature_linking` flags.
pub const LINK_AGGRO_ON_AGGRO: u32 = 0x0001;
pub const LINK_TO_AGGRO_ON_AGGRO: u32 = 0x0002;
pub const LINK_RESPAWN_ON_EVADE: u32 = 0x0004;
pub const LINK_TO_RESPAWN_ON_EVADE: u32 = 0x0008;
pub const LINK_RESPAWN_ON_RESPAWN: u32 = 0x0080;
pub const LINK_DESPAWN_ON_RESPAWN: u32 = 0x0100;
pub const LINK_CANT_SPAWN_IF_BOSS_DEAD: u32 = 0x0400;
pub const LINK_CANT_SPAWN_IF_BOSS_ALIVE: u32 = 0x0800;
pub const LINK_DESPAWN_ON_EVADE: u32 = 0x1000;
pub const LINK_DESPAWN_ON_DESPAWN: u32 = 0x2000;
pub const LINK_EVADE_ON_EVADE: u32 = 0x4000;

/// A spawn pool (`pool_template`): at most `max_limit` of its members up
/// at once.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Pool {
    pub entry: u32,
    pub max_limit: u32,
    /// Its creature and object spawns, and the pools in it: (guid or pool,
    /// chance).
    pub creatures: Vec<(u32, f32)>,
    pub objects: Vec<(u32, f32)>,
    pub pools: Vec<(u32, f32)>,
    pub mother: u32,
}

/// A `spell_area` row: `spell` belongs in `area` for players who match.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct SpellArea {
    pub spell: u32,
    pub area: u32,
    /// A quest that must be taken (and, with `quest_start_active`, not yet
    /// complete) or rewarded.
    pub quest_start: u32,
    pub quest_start_active: bool,
    /// A quest that must not be rewarded yet.
    pub quest_end: u32,
    /// An aura the player must (positive) or mustn't (negative) have.
    pub aura_spell: i32,
    pub race_mask: u32,
    /// 0 male, 1 female, 2 either.
    pub gender: u32,
    /// Cast on the player there; otherwise only allowed there.
    pub autocast: bool,
}

/// `map_template`: a map's instance script and reset, and for a dungeon
/// the dungeon it is in and where its entrance is outside.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct MapTemplate {
    pub map: u32,
    pub map_type: u32,
    pub player_limit: u32,
    pub reset_delay: u32,
    pub script_name: String,
    pub name: String,
    pub parent: u32,
    /// The zone its graveyards are linked to.
    pub linked_zone: u32,
    /// The entrance a ghost is shown: map, x, y.
    pub ghost_entrance: Option<(u32, f32, f32)>,
}

#[derive(Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ScriptData {
    /// EventAI events, by creature entry.
    pub ai_events: AHashMap<u32, Vec<AiEvent>>,
    pub ai_scripts: AHashMap<u32, Vec<ScriptStep>>,
    pub generic: AHashMap<u32, Vec<ScriptStep>>,
    pub event: AHashMap<u32, Vec<ScriptStep>>,
    pub spell: AHashMap<u32, Vec<ScriptStep>>,
    pub quest_start: AHashMap<u32, Vec<ScriptStep>>,
    pub quest_end: AHashMap<u32, Vec<ScriptStep>>,
    pub gossip: AHashMap<u32, Vec<ScriptStep>>,
    pub movement: AHashMap<u32, Vec<ScriptStep>>,
    pub gameobject: AHashMap<u32, Vec<ScriptStep>>,
    pub areatrigger: AHashMap<u32, Vec<ScriptStep>>,
    pub creature_spells: AHashMap<u32, Vec<ScriptStep>>,
    pub conditions: AHashMap<u32, Condition>,
    /// The values of a code-built condition that has fractions (a point and
    /// a radius), as they are; `values` has them cut to whole numbers.
    pub condition_floats: AHashMap<u32, [f32; 4]>,
    /// Quests' `StartScript` and `CompleteScript`.
    pub quest_start_script: AHashMap<u32, u32>,
    pub quest_complete_script: AHashMap<u32, u32>,
    /// Gossip menus' scripts, each its greeting's ((menu, text) → script,
    /// as mangos's `gossip_menu` rows have them), and their options':
    /// (menu, option) → script.
    pub gossip_menu_script: AHashMap<(u32, u32), u32>,
    pub gossip_option_script: AHashMap<(u32, u32), u32>,
    /// Creature groups: leader → members, and member → leader.
    pub groups: AHashMap<u32, Vec<GroupMember>>,
    pub group_of: AHashMap<u32, u32>,
    /// `creature_linking`: slave → (master, flags).
    pub links: AHashMap<u32, (u32, u32)>,
    /// `creature_linking_template`: (slave entry, map) → (master entry,
    /// flags, search range); resolved into `links` at load
    /// (`scripts_load::link_templates`).
    pub link_templates: AHashMap<(u32, u32), (u32, u32, f32)>,
    pub pools: AHashMap<u32, Pool>,
    /// A spawn's pool.
    pub creature_pool: AHashMap<u32, u32>,
    pub object_pool: AHashMap<u32, u32>,
    pub map_templates: AHashMap<u32, MapTemplate>,
    /// `areatrigger_template`'s scripts: trigger → generic script.
    pub areatrigger_script: AHashMap<u32, u32>,
    /// `areatrigger_template.cooldown`: seconds a scripted trigger rests
    /// after it runs, in each copy of its map.
    pub areatrigger_cooldown: AHashMap<u32, u32>,
    /// `game_event.holiday`: event → holiday.
    pub event_holidays: AHashMap<u32, u32>,
    /// `quest_template.RequiredCondition`.
    pub quest_condition: AHashMap<u32, u32>,
    /// `areatrigger_teleport.required_condition`.
    pub teleport_condition: AHashMap<u32, u32>,
    /// `spell_area`: auras a place (and a quest, race or gender) brings.
    pub spell_areas: Vec<SpellArea>,
    /// Paths creature scripts walk (`start_waypoints` source 5): by entry
    /// and variant.
    pub routes: super::tt_scripts::Routes,
    /// Code-built object scripts (Thistle Tea's `GOHello` ports): run on
    /// the player who uses the object, at it. By spawn, and by entry for an
    /// object of no spawn.
    pub object_hello: AHashMap<u32, u32>,
    pub object_hello_entry: AHashMap<u32, u32>,
}

impl ScriptData {
    /// The steps of a script table's script, sorted by delay then
    /// priority.
    pub fn table(&self, table: ScriptTable) -> &AHashMap<u32, Vec<ScriptStep>> {
        match table {
            ScriptTable::CreatureAi => &self.ai_scripts,
            ScriptTable::Generic => &self.generic,
            ScriptTable::Event => &self.event,
            ScriptTable::Spell => &self.spell,
            ScriptTable::QuestStart => &self.quest_start,
            ScriptTable::QuestEnd => &self.quest_end,
            ScriptTable::Gossip => &self.gossip,
            ScriptTable::Movement => &self.movement,
            ScriptTable::GameObject => &self.gameobject,
            ScriptTable::AreaTrigger => &self.areatrigger,
            ScriptTable::CreatureSpells => &self.creature_spells,
        }
    }
}

/// Which script table a script comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub enum ScriptTable {
    CreatureAi,
    Generic,
    Event,
    Spell,
    QuestStart,
    QuestEnd,
    Gossip,
    Movement,
    GameObject,
    AreaTrigger,
    CreatureSpells,
}
