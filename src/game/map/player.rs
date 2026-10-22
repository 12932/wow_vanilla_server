//! A player in a map: their stored character, their update fields, and
//! what their client knows.

use std::sync::Arc;
use std::time::Instant;

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::{MovementInfo, MovementInfo_MovementFlags, Vector3d};

use crate::game::conn::Outbound;
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::grid::{CellKey, cell_of};
use crate::game::registry::{Online, Team};
use crate::game::store::{BAG_PLAYER, CharacterRow, ItemRow};
use crate::game::update::{Create, Placement, Speeds};

/// Equipment slots: head through tabard.
pub const EQUIPMENT_SLOTS: u8 = 19;
/// Inventory slots in the player's own fields: equipment, bags, backpack,
/// bank, bank bags, buyback and keyring.
pub const INVENTORY_SLOTS: u8 = 113;

pub const UNIT_FLAG_PVP_ATTACKABLE: u32 = 0x0000_0008;
pub const UNIT_FLAG_DISABLE_MOVE: u32 = 0x0000_0004;
pub const UNIT_FLAG_PVP: u32 = 0x0000_1000;
pub const UNIT_FLAG_STUNNED: u32 = 0x0004_0000;
pub const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;

pub const PLAYER_FLAGS_GROUP_LEADER: u32 = 0x0000_0001;
pub const PLAYER_FLAGS_AFK: u32 = 0x0000_0002;
pub const PLAYER_FLAGS_DND: u32 = 0x0000_0004;
pub const PLAYER_FLAGS_GM: u32 = 0x0000_0008;
pub const PLAYER_FLAGS_GHOST: u32 = 0x0000_0010;
pub const PLAYER_FLAGS_RESTING: u32 = 0x0000_0020;
pub const PLAYER_FLAGS_FFA_PVP: u32 = 0x0000_0080;
pub const PLAYER_FLAGS_HIDE_HELM: u32 = 0x0000_0400;
pub const PLAYER_FLAGS_HIDE_CLOAK: u32 = 0x0000_0800;
pub const PLAYER_FLAGS_PVP_DESIRED: u32 = 0x0000_0200;

/// `UNIT_FIELD_BYTES_1`'s first byte.
pub const STAND_STATE_STAND: u8 = 0;
pub const STAND_STATE_SIT: u8 = 1;

/// An item the player has.
#[derive(Debug, Clone)]
pub struct Item {
    pub guid: Guid,
    pub entry: u32,
    pub bag: u8,
    pub slot: u8,
    pub fields: Fields,
    pub data: String,
    /// The item's look, for a corpse.
    pub display: u32,
}

impl Item {
    pub fn new(
        data: &GameData,
        guid: Guid,
        owner: Guid,
        entry: u32,
        bag: u8,
        slot: u8,
        count: u32,
    ) -> Self {
        let bag_slots = data
            .item(entry)
            .filter(|t| matches!(t.class, 1 | 11) && t.container_slots > 0)
            .map(|t| t.container_slots.min(36));
        let kind = if bag_slots.is_some() {
            TypeId::Container
        } else {
            TypeId::Item
        };
        let mut fields = Fields::new(kind, guid, entry);
        if let Some(n) = bag_slots {
            fields.set_u32(CONTAINER_FIELD_NUM_SLOTS, n);
        }
        fields.set_guid(ITEM_FIELD_OWNER, owner);
        fields.set_guid(ITEM_FIELD_CONTAINED, owner);
        fields.set_u32(ITEM_FIELD_STACK_COUNT, count.max(1));
        let durability = max_durability(data, entry);
        fields.set_u32(ITEM_FIELD_DURABILITY, durability);
        fields.set_u32(ITEM_FIELD_MAXDURABILITY, durability);
        if let Some(t) = data.item(entry) {
            for (i, s) in t.spells.iter().enumerate() {
                fields.set_i32(ITEM_FIELD_SPELL_CHARGES + i as u16, s.charges);
            }
            fields.set_u32(ITEM_FIELD_DURATION, t.duration);
        }
        let display = item_display(data, entry).map(|d| d.0).unwrap_or(0);
        let mut item = Self {
            guid,
            entry,
            bag,
            slot,
            fields,
            data: String::new(),
            display,
        };
        crate::game::map::items::on_create(data, &mut item);
        item
    }

    pub fn from_row(data: &GameData, row: &ItemRow, owner: Guid) -> Self {
        // A gift is made as what it wraps, then wrapped again.
        let gift = crate::game::map::items_misc::wrapped(&row.data);
        let mut item = Self::new(
            data,
            guid::item(row.guid),
            owner,
            gift.map_or(row.entry, |w| w.entry),
            row.bag,
            row.slot,
            row.count,
        );
        item.fields.set_u32(ITEM_FIELD_DURABILITY, row.durability);
        item.fields.set_u32(ITEM_FIELD_FLAGS, row.flags);
        // The charges left as saved (mangos's `LoadFromDB`); an item kept
        // before they were has its template's, from `new`.
        if let Some(charges) = row.charges {
            for (i, c) in charges.iter().enumerate() {
                item.fields.set_i32(ITEM_FIELD_SPELL_CHARGES + i as u16, *c);
            }
        }
        item.data = row.data.clone();
        if let Some(text) = row
            .data
            .split(';')
            .find_map(|t| t.strip_prefix("text:"))
            .and_then(|t| t.parse().ok())
        {
            item.fields.set_u32(ITEM_FIELD_ITEM_TEXT_ID, text);
        }
        crate::game::map::enchant::decode(data, &mut item);
        if let Some(w) = gift {
            crate::game::map::items_misc::rewrap_loaded(&mut item, row.entry, w);
        }
        item
    }

    pub fn to_row(&self) -> ItemRow {
        ItemRow {
            guid: guid::counter(self.guid),
            entry: self.entry,
            bag: self.bag,
            slot: self.slot,
            count: self.fields.get_u32(ITEM_FIELD_STACK_COUNT),
            durability: self.fields.get_u32(ITEM_FIELD_DURABILITY),
            flags: self.fields.get_u32(ITEM_FIELD_FLAGS),
            data: crate::game::map::enchant::encode(self),
            charges: Some(std::array::from_fn(|i| {
                self.fields.get_u32(ITEM_FIELD_SPELL_CHARGES + i as u16) as i32
            })),
        }
    }

    /// What the auction house and the mailbox show of an item kept as a
    /// row: its permanent enchantment, random property, suffix factor and
    /// its first spell's charges (mangos-zero's `BuildAuctionInfo` and mail
    /// list).
    pub fn shown(data: &GameData, row: &ItemRow) -> Shown {
        let f = Self::from_row(data, row, Guid::zero()).fields;
        Shown {
            enchantment: f.get_u32(ITEM_FIELD_ENCHANTMENT),
            random_property: f.get_u32(ITEM_FIELD_RANDOM_PROPERTIES_ID),
            suffix_factor: f.get_u32(ITEM_FIELD_PROPERTY_SEED),
            charges: f.get_u32(ITEM_FIELD_SPELL_CHARGES),
        }
    }

    pub fn create(&self) -> Create<'_> {
        Create {
            guid: self.guid,
            type_id: self.fields.type_id(),
            placement: Placement::None,
            fields: &self.fields,
            is_self: false,
            melee_target: None,
        }
    }
}

/// What a list shows of an item: [`Item::shown`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shown {
    pub enchantment: u32,
    pub random_property: u32,
    pub suffix_factor: u32,
    pub charges: u32,
}

/// A teleport the client has been told of and hasn't acknowledged.
#[derive(Debug, Clone, Copy)]
pub struct PendingTeleport {
    pub since: Instant,
}

#[derive(Debug)]
pub struct Player {
    pub guid: Guid,
    pub account: Arc<str>,
    pub name: Arc<str>,
    pub online: Arc<Online>,
    pub outbound: Outbound,
    /// The character as loaded; [`Player::to_row`] brings it up to date.
    pub row: CharacterRow,
    pub fields: Fields,
    pub movement: MovementInfo,
    pub speeds: Speeds,
    /// A GM's `.speed`: the run's base, as a multiple of a run, kept
    /// through what changes speeds (Thistle Tea's `base_run_speed`).
    pub speed_rate: f32,
    pub team: Team,
    pub zone: u32,
    pub area: u32,
    pub items: Vec<Item>,
    pub spells: Vec<u32>,
    /// Objects the client has been sent and not told to forget.
    pub known: AHashSet<Guid>,
    /// Players whose clients know this one.
    pub observers: crate::game::map::observers::Observers,
    /// Whose marks were on them as their observers were last told
    /// (`map::views`).
    pub views_told: crate::game::map::views::Viewers,
    /// Where visibility was last worked out.
    pub vis_anchor: Vector3d,
    pub cell: CellKey,
    pub logout_at: Option<Instant>,
    /// Their client gone, they stay in the world until then, for it to
    /// come back and take them over (`Map::handle_disconnect`).
    pub linger_until: Option<Instant>,
    pub teleport: Option<PendingTeleport>,
    /// Arrived from another map and waiting for the client's worldport
    /// acknowledgement before anything is sent about the world.
    pub awaiting_worldport: bool,
    /// When the played-time counters were last brought up to date.
    pub played_at: Instant,
    pub save_at: Instant,
    pub gm: bool,
    pub gm_level: u8,
    pub death: crate::game::map::death::Death,
    /// When health and power next regenerate.
    pub regen_at: Instant,
    /// The part of a point of health regenerated and not yet given, for
    /// the next tick (Thistle Tea's `health_regen_carry`).
    pub health_regen_carry: f32,
    /// When mana was last spent, for the five-second rule.
    pub mana_spent_at: Option<Instant>,
    pub combat: crate::game::map::combat::Combat,
    /// The corpse or object whose loot window is open.
    pub looting: Option<Guid>,
    /// The next buyback slot to fill.
    pub buyback_next: u8,
    /// When buyback's timestamps count from: the login.
    pub buyback_started: Instant,
    /// The NPC and gossip menu the player last had open.
    pub gossip_menu: Option<(Guid, u32)>,
    /// The quest objects this client knows shown as usable for a quest.
    pub quest_objects: AHashSet<Guid>,
    /// Factions at war with the player only for the fight they are in.
    pub temporary_war: Vec<u32>,
    pub flight: Option<crate::game::map::npc::Flight>,
    pub journal: crate::game::map::quests::Journal,
    pub auras: crate::game::map::auras::Auras,
    pub caster: crate::game::map::spells::Caster,
    /// A resurrection offered: by whom, and with what health and mana.
    pub resurrect_offer: Option<(Guid, u32, u32)>,
    /// Where the caster stood as they offered it: where it brings one
    /// (mangos's `setResurrectRequestData`).
    pub resurrect_at: Option<Vector3d>,
    /// The map the caster was in as they offered it (the corpse's, for one
    /// cast at a corpse from another map): where it brings one.
    pub resurrect_map: Option<crate::game::map::MapKey>,
    /// A resurrection taken, its health and mana, waiting for the teleport
    /// to its caster to land (mangos's `DELAYED_RESURRECT_PLAYER`).
    pub resurrect_on_arrival: Option<(u32, u32)>,
    pub trade: Option<crate::game::map::trade::Trade>,
    pub duel: Option<crate::game::map::duel::Duel>,
    /// Where a player in a battleground came from.
    pub bg_return: Option<(crate::game::map::MapKey, Vector3d, f32)>,
    /// When a player in a copy no longer theirs goes home.
    pub eviction: Option<Instant>,
    /// The copy's map just left, to tell the client once it's arrived.
    pub last_instance: Option<u32>,
    /// The last position movement was checked from, and when.
    pub move_check: Option<(Vector3d, Instant)>,
    /// The top of a fall under way.
    pub fall_from: Option<f32>,
    /// Chat flood control: lines close together, and a mute.
    pub chat_last: Option<Instant>,
    pub chat_burst: u32,
    pub muted_until: Option<Instant>,
    pub derived: crate::game::map::derive::Derived,
    /// Moved by the server: feared, confused or charmed.
    pub driven: Option<crate::game::map::player_motion::Driven>,
    /// A charge it is on.
    pub charge: Option<crate::game::map::charge::Charge>,
    /// The last place it stood on the ground, for Stuck: (map, position,
    /// facing).
    pub safe_position: Option<(u32, Vector3d, f32)>,
    pub drunk: crate::game::map::intoxication::Drunk,
    pub gadgets: crate::game::map::item_scripts::Gadgets,
    /// The pet, totems, guardians and charmed unit the player controls.
    pub owned: crate::game::map::pets::Owned,
    pub misc: crate::game::map::misc::PlayerMisc,
    pub pvp: crate::game::map::pvp::PvpState,
    /// Damage from other players lately, to share a kill's honor.
    pub honor_damage: crate::game::map::pvp::DamageHistory,
    /// The gear changed: its auras want bringing in line.
    pub gear_dirty: bool,
    /// Gear's outdoor-only spells are held off indoors: outside again,
    /// they want putting back.
    pub gear_held_indoors: bool,
    /// `[players] always_max_skill_for_level`, as it was at login: the
    /// weapon and defense skills are put at their cap at login and each
    /// level.
    pub skills_maxed: bool,
}

impl Player {
    /// The player for a stored character.
    pub fn from_row(data: &GameData, row: CharacterRow, online: Arc<Online>, now: Instant) -> Self {
        let guid = guid::player(row.guid);
        let position = Vector3d {
            x: row.x,
            y: row.y,
            z: row.z,
        };
        let movement = MovementInfo {
            flags: MovementInfo_MovementFlags::empty(),
            timestamp: 0,
            position,
            orientation: row.orientation,
            fall_time: 0,
        };
        let items = row
            .items
            .iter()
            .map(|r| Item::from_row(data, r, guid))
            .collect();
        let mut p = Self {
            guid,
            account: online.account.clone(),
            name: online.name.clone(),
            gm: online.gm,
            gm_level: online.gm_level,
            outbound: online.outbound.clone(),
            online,
            fields: Fields::new(TypeId::Player, guid, 0),
            movement,
            speeds: Speeds::default(),
            speed_rate: 1.0,
            team: Team::of_race(row.race),
            zone: row.zone,
            area: 0,
            items,
            spells: row.spells.clone(),
            known: AHashSet::new(),
            observers: Default::default(),
            views_told: Default::default(),
            vis_anchor: position,
            cell: cell_of(position.x, position.y),
            logout_at: None,
            linger_until: None,
            teleport: None,
            awaiting_worldport: false,
            played_at: now,
            save_at: now,
            death: crate::game::map::death::Death::Alive,
            regen_at: now + crate::game::map::stats::REGEN_INTERVAL,
            health_regen_carry: 0.0,
            mana_spent_at: None,
            combat: Default::default(),
            looting: None,
            buyback_next: 0,
            buyback_started: now,
            gossip_menu: None,
            quest_objects: AHashSet::new(),
            temporary_war: Vec::new(),
            flight: None,
            journal: row.state.quests.clone(),
            auras: Default::default(),
            caster: Default::default(),
            resurrect_offer: None,
            resurrect_at: None,
            resurrect_map: None,
            resurrect_on_arrival: None,
            trade: None,
            duel: None,
            bg_return: None,
            eviction: None,
            last_instance: None,
            move_check: None,
            fall_from: None,
            chat_last: None,
            chat_burst: 0,
            muted_until: None,
            derived: Default::default(),
            driven: None,
            charge: None,
            safe_position: None,
            drunk: Default::default(),
            gadgets: Default::default(),
            owned: Default::default(),
            misc: Default::default(),
            pvp: Default::default(),
            honor_damage: Default::default(),
            gear_dirty: true,
            gear_held_indoors: false,
            skills_maxed: false,
            row,
        };
        p.init_fields(data);
        crate::game::map::skills::init(&mut p, data);
        crate::game::map::spells::refresh_talent_points(&mut p, data);
        p
    }

    pub fn position(&self) -> Vector3d {
        self.movement.position
    }

    pub fn map_id(&self) -> u32 {
        self.row.map
    }

    pub fn level(&self) -> u8 {
        self.fields.get_u32(UNIT_FIELD_LEVEL) as u8
    }

    /// The power the player uses now: rage, energy or mana by class, and
    /// for a druid by form.
    pub fn power_type(&self) -> u8 {
        match self.fields.get_byte(UNIT_FIELD_BYTES_0, 3) {
            0 if self.row.class == 1 => 1,
            0 if self.row.class == 4 => 3,
            p => p,
        }
    }

    fn init_fields(&mut self, data: &GameData) {
        let row = &self.row;
        let (display, scale) = data.race_display(row.race, row.gender);
        let faction = data.race_faction(row.race);
        let next_level = data.xp_to_level(row.level.max(1));
        let f = &mut self.fields;
        f.set_f32(OBJECT_FIELD_SCALE_X, scale);
        let power = match row.class {
            1 => 1u8,
            4 => 3,
            _ => 0,
        };
        f.set_bytes(UNIT_FIELD_BYTES_0, row.race, row.class, row.gender, power);
        f.set_u32(UNIT_FIELD_LEVEL, u32::from(row.level.max(1)));
        f.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
        f.set_u32(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP_ATTACKABLE);
        f.set_u32(UNIT_FIELD_BASEATTACKTIME, 2000);
        f.set_u32(UNIT_FIELD_BASEATTACKTIME + 1, 2000);
        f.set_u32(UNIT_FIELD_RANGEDATTACKTIME, 2000);
        f.set_f32(UNIT_FIELD_BOUNDINGRADIUS, 0.389);
        f.set_f32(UNIT_FIELD_COMBATREACH, 1.5);
        f.set_u32(UNIT_FIELD_DISPLAYID, display);
        f.set_u32(UNIT_FIELD_NATIVEDISPLAYID, display);
        f.set_f32(UNIT_MOD_CAST_SPEED, 1.0);
        f.set_f32(UNIT_FIELD_MINDAMAGE, 1.0);
        f.set_f32(UNIT_FIELD_MAXDAMAGE, 2.0);
        f.set_f32(UNIT_FIELD_ATTACK_POWER_MULTIPLIER, 0.0);
        f.set_f32(UNIT_FIELD_RANGED_ATTACK_POWER_MULTIPLIER, 0.0);
        // Nothing off or on by school until auras say (Thistle Tea's
        // `PowerCostSync`: a share, not a factor).
        for i in 0..7 {
            f.set_f32(UNIT_FIELD_POWER_COST_MULTIPLIER + i, 0.0);
        }
        f.set_bytes(UNIT_FIELD_BYTES_2, 0, 0, 0, 0);
        f.set_bytes(
            PLAYER_BYTES,
            row.skin,
            row.face,
            row.hair_style,
            row.hair_color,
        );
        // Facial hair, -, bank bag slots, rest state (normal).
        f.set_bytes(
            PLAYER_BYTES_2,
            row.facial_hair,
            0,
            row.state
                .bank_bag_slots
                .min(crate::game::map::npc::MAX_BANK_BAG_SLOTS),
            2,
        );
        f.set_bytes(PLAYER_BYTES_3, row.gender, 0, 0, 0);
        f.set_u32(PLAYER_XP, row.xp);
        f.set_u32(
            PLAYER_NEXT_LEVEL_XP,
            if next_level == 0 { 400 } else { next_level },
        );
        f.set_u32(PLAYER_FIELD_COINAGE, row.money);
        f.set_u32(PLAYER_FIELD_WATCHED_FACTION_INDEX, u32::MAX);
        f.set_bytes(PLAYER_FIELD_BYTES, 0, 0, row.state.action_bar_toggles, 0);
        f.set_f32(PLAYER_FIELD_MOD_DAMAGE_DONE_PCT, 1.0);
        for i in 0..7 {
            f.set_f32(PLAYER_FIELD_MOD_DAMAGE_DONE_PCT + i, 1.0);
        }
        f.set_u32(PLAYER_AMMO_ID, row.state.ammo);
        self.apply_stats(data);
        let health = if self.row.state.health == 0 && !self.row.state.dead {
            self.fields.get_u32(UNIT_FIELD_MAXHEALTH)
        } else {
            self.row
                .state
                .health
                .min(self.fields.get_u32(UNIT_FIELD_MAXHEALTH))
        };
        self.fields.set_u32(UNIT_FIELD_HEALTH, health);
        let p = u16::from(self.power_type());
        let max_power = self.fields.get_u32(UNIT_FIELD_MAXPOWER1 + p);
        let power = match self.power_type() {
            0 => {
                let stored = self.row.state.powers[0];
                if stored == 0 {
                    max_power
                } else {
                    stored.min(max_power)
                }
            }
            3 => max_power,
            _ => 0,
        };
        self.fields.set_u32(UNIT_FIELD_POWER1 + p, power);
        self.equip_fields();
        crate::game::map::quests::refresh_log(self, data);
    }

    /// Stats, health and power for the level, gear and auras.
    pub fn apply_stats(&mut self, data: &GameData) {
        crate::game::map::derive::player(self, data);
    }

    /// The inventory slot fields and visible items, from `items`.
    pub fn equip_fields(&mut self) {
        for slot in 0..INVENTORY_SLOTS {
            self.fields.set_guid(
                PLAYER_FIELD_INV_SLOT_HEAD + u16::from(slot) * 2,
                Guid::zero(),
            );
        }
        // What others see of each worn item (mangos's `SetVisibleItemSlot`):
        // its maker, its entry, its permanent and temporary enchantments
        // (the glow), its random property and suffix factor (its name).
        let visible = |f: &mut Fields, slot: u8, item: Option<&Item>| {
            let at = u16::from(slot) * VISIBLE_ITEM_SIZE;
            let field = |i: Option<&Item>, field: u16| i.map_or(0, |i| i.fields.get_u32(field));
            f.set_guid(
                PLAYER_VISIBLE_ITEM_1_CREATOR + at,
                item.map_or(Guid::zero(), |i| i.fields.get_guid(ITEM_FIELD_CREATOR)),
            );
            f.set_u32(PLAYER_VISIBLE_ITEM_1_0 + at, item.map_or(0, |i| i.entry));
            f.set_u32(
                PLAYER_VISIBLE_ITEM_1_0 + at + 1,
                field(item, ITEM_FIELD_ENCHANTMENT),
            );
            f.set_u32(
                PLAYER_VISIBLE_ITEM_1_0 + at + 2,
                field(item, ITEM_FIELD_ENCHANTMENT + 3),
            );
            f.set_u32(
                PLAYER_VISIBLE_ITEM_1_PROPERTIES + at,
                field(item, ITEM_FIELD_RANDOM_PROPERTIES_ID) & 0xFFFF,
            );
            f.set_u32(
                PLAYER_VISIBLE_ITEM_1_PROPERTIES + at + 1,
                field(item, ITEM_FIELD_PROPERTY_SEED),
            );
        };
        for slot in 0..EQUIPMENT_SLOTS {
            let item = self
                .items
                .iter()
                .find(|i| i.bag == BAG_PLAYER && i.slot == slot);
            visible(&mut self.fields, slot, item);
        }
        for item in &self.items {
            if item.bag != BAG_PLAYER || item.slot >= INVENTORY_SLOTS {
                continue;
            }
            self.fields.set_guid(
                PLAYER_FIELD_INV_SLOT_HEAD + u16::from(item.slot) * 2,
                item.guid,
            );
        }
    }

    /// What a create block says about where the player is.
    pub fn placement(&self) -> Placement {
        let flags = movement_flags(&self.movement.flags);
        let jump = self
            .movement
            .flags
            .get_jumping()
            .map(|j| crate::game::update::Jump {
                z_speed: j.z_speed,
                cos_angle: j.cos_angle,
                sin_angle: j.sin_angle,
                xy_speed: j.xy_speed,
            });
        Placement::Living {
            flags: flags
                & (crate::game::update::MOVEFLAG_FORWARD
                    | crate::game::update::MOVEFLAG_WALK_MODE
                    | crate::game::update::MOVEFLAG_SWIMMING),
            timestamp: self.movement.timestamp,
            position: self.movement.position,
            orientation: self.movement.orientation,
            pitch: self
                .movement
                .flags
                .get_swimming()
                .map(|s| s.pitch)
                .unwrap_or(0.0),
            // Mid-jump, the jump, from the fall's time into it, as mangos's
            // `MovementInfo::Write` puts it: one who comes into view of a
            // jumper sees them in the air, not standing on it.
            fall_time: jump.map_or(0, |_| self.movement.fall_time),
            jump,
            spline_elevation: self
                .movement
                .flags
                .get_spline_elevation()
                .map(|e| e.spline_elevation),
            speeds: self.speeds,
            spline: None,
            transport: self.movement.flags.get_on_transport().map(|t| {
                crate::game::update::Aboard {
                    transport: t.transport.guid,
                    position: t.transport.position,
                    orientation: t.transport.orientation,
                }
            }),
        }
    }

    pub fn create(&self, is_self: bool) -> Create<'_> {
        Create {
            guid: self.guid,
            type_id: TypeId::Player,
            placement: self.placement(),
            fields: &self.fields,
            is_self,
            melee_target: None,
        }
    }

    /// The create another's client is told of the player by, at `now`. On
    /// a flight, where it has taken them and its path as a flying spline,
    /// its time passed, as mangos's create carries a taxi flight's (and a
    /// creature's walk, `Creature::create`): the flight's
    /// `SMSG_MONSTER_MOVE` went only to those who saw it take off, and a
    /// newcomer to the flier's view saw a mounted player standing still.
    pub fn create_for_others(&self, now: Instant) -> Create<'_> {
        let mut create = self.create(false);
        if let Some(f) = self.flight.as_ref().filter(|f| !f.walk.done(now))
            && let Placement::Living {
                position,
                orientation,
                spline,
                ..
            } = &mut create.placement
        {
            (*position, *orientation) = f.walk.position_at(now);
            *spline = Some(crate::game::update::SplineState {
                flags: crate::game::map::npc::FLIGHT_SPLINE,
                time_passed: now.saturating_duration_since(f.walk.started).as_millis() as u32,
                duration: f.walk.duration.as_millis() as u32,
                id: f.walk.spline_id,
                nodes: f.walk.points.clone(),
                final_node: f.walk.destination(),
                final_angle: None,
                final_target: None,
            });
        }
        create
    }

    pub fn stand_state(&self) -> u8 {
        self.fields.get_byte(UNIT_FIELD_BYTES_1, 0)
    }

    pub fn set_stand_state(&mut self, state: u8) {
        self.fields.set_byte(UNIT_FIELD_BYTES_1, 0, state);
    }

    /// Bring the played-time counters up to `now`.
    pub fn count_played(&mut self, now: Instant) {
        let secs = now.duration_since(self.played_at).as_secs();
        if secs > 0 {
            let secs = u32::try_from(secs).unwrap_or(u32::MAX);
            self.row.played_total = self.row.played_total.saturating_add(secs);
            self.row.played_level = self.row.played_level.saturating_add(secs);
            self.played_at += std::time::Duration::from_secs(u64::from(secs));
        }
    }

    /// The character as it now is, to store.
    pub fn to_row(&mut self, now: Instant) -> CharacterRow {
        self.count_played(now);
        crate::game::map::rest::accrue(self, now);
        // A flight already flown is a landing at its end (Thistle Tea's
        // `Taxi.pause` of an elapsed flight), not a flight to take up.
        let landed = self
            .flight
            .as_ref()
            .filter(|f| f.walk.done(now))
            .map(|f| f.walk.destination());
        let pos = landed.unwrap_or_else(|| self.position());
        let f = &self.fields;
        let mut row = self.row.clone();
        row.level = self.level();
        row.xp = f.get_u32(PLAYER_XP);
        row.money = f.get_u32(PLAYER_FIELD_COINAGE);
        row.x = pos.x;
        row.y = pos.y;
        row.z = pos.z;
        row.orientation = self.movement.orientation;
        row.zone = self.zone;
        row.logout_time = chrono::Utc::now().timestamp();
        row.state.health = f.get_u32(UNIT_FIELD_HEALTH);
        for i in 0..5 {
            row.state.powers[i] = f.get_u32(UNIT_FIELD_POWER1 + i as u16);
        }
        row.state.action_bar_toggles = f.get_byte(PLAYER_FIELD_BYTES, 2);
        row.state.bank_bag_slots = f.get_byte(PLAYER_BYTES_2, 2);
        row.state.drunk = self.drunk.value;
        row.state.ammo = f.get_u32(PLAYER_AMMO_ID);
        // What's sold stays to be bought back this session only (Thistle
        // Tea's `Buyback.logout`).
        row.items = self
            .items
            .iter()
            .filter(|i| !crate::game::map::vendor::in_buyback(i.bag, i.slot))
            .map(Item::to_row)
            .collect();
        row.state.transport = self.movement.flags.get_on_transport().map(|t| {
            let deck = t.transport.position;
            (
                crate::game::transports::entry_of(t.transport.guid),
                deck.x,
                deck.y,
                deck.z,
                t.transport.orientation,
            )
        });
        row.state.flight = self.flight.as_ref().filter(|_| landed.is_none()).map(|f| {
            crate::game::store::SavedFlight {
                map: self.row.map,
                points: f
                    .walk
                    .remaining(now)
                    .into_iter()
                    .map(|p| (p.x, p.y, p.z))
                    .collect(),
                destination: f.destination,
                mount: f.mount,
            }
        });
        row.state.reputation = crate::game::map::reputation::kept(self);
        row.state.quests = self.journal.clone();
        row.state.auras = crate::game::map::spells::cooldowns::saved_auras(self, now);
        row.state.cooldowns = crate::game::map::spells::cooldowns::saved_cooldowns(self, now);
        row.state.category_cooldowns =
            crate::game::map::spells::cooldowns::saved_category_cooldowns(self, now);
        row.state.cooldown_sources = crate::game::map::spells::cooldowns::saved_sources(self, now);
        row.state.saved_at_ms = chrono::Utc::now().timestamp_millis();
        row.spells = self.spells.clone();
        row.state.skills = Some(crate::game::map::skills::saved(self));
        row
    }
}

/// The bits of a client's movement flags a create block keeps.
pub fn movement_flags(flags: &MovementInfo_MovementFlags) -> u32 {
    use crate::game::update::{MOVEFLAG_FORWARD, MOVEFLAG_SWIMMING, MOVEFLAG_WALK_MODE};
    let mut bits = 0;
    if flags.get_forward() {
        bits |= MOVEFLAG_FORWARD;
    }
    if flags.get_walk_mode() {
        bits |= MOVEFLAG_WALK_MODE;
    }
    if flags.get_swimming().is_some() {
        bits |= MOVEFLAG_SWIMMING;
    }
    bits
}

/// An item's durability when new: vmangos's template, or `wow_items`'s.
pub fn max_durability(data: &GameData, entry: u32) -> u32 {
    match data.item(entry) {
        Some(t) => t.max_durability,
        None => wow_items::vanilla::lookup_item(entry)
            .map(|i| u32::try_from(i.max_durability()).unwrap_or(0))
            .unwrap_or(0),
    }
}

/// An item's look on the character screen and on a creature: its display
/// id and inventory type.
pub fn item_display(data: &GameData, entry: u32) -> Option<(u32, u32)> {
    match data.item(entry) {
        Some(t) => Some((t.display_id, t.inventory_type)),
        None => wow_items::vanilla::lookup_item(entry)
            .map(|i| (i.display_id(), u32::from(i.inventory_type().as_int()))),
    }
}

impl Player {
    /// The display id of an item, for a corpse or a creature.
    pub fn item_display_id(&self, entry: u32) -> u32 {
        self.items
            .iter()
            .find(|i| i.entry == entry)
            .map(|i| i.display)
            .unwrap_or(0)
    }
}
