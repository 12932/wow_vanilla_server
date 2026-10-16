//! Update fields (1.12.1, build 5875): their indices, and a store that
//! remembers which fields changed since the last update.
//!
//! Every object's fields are a flat array of `u32`s. A setter that changes
//! a value marks it dirty; the map's update encodes the dirty fields once
//! into a values block and clears them. Which fields an observer may see is
//! [`Visibility`]'s business.

use wow_world_messages::Guid;

// ── Object ──────────────────────────────────────────────────────────────
pub const OBJECT_FIELD_GUID: u16 = 0;
pub const OBJECT_FIELD_TYPE: u16 = 2;
pub const OBJECT_FIELD_ENTRY: u16 = 3;
pub const OBJECT_FIELD_SCALE_X: u16 = 4;
pub const OBJECT_END: u16 = 6;

// ── Item ────────────────────────────────────────────────────────────────
pub const ITEM_FIELD_OWNER: u16 = 6;
pub const ITEM_FIELD_CONTAINED: u16 = 8;
pub const ITEM_FIELD_CREATOR: u16 = 10;
pub const ITEM_FIELD_GIFTCREATOR: u16 = 12;
pub const ITEM_FIELD_STACK_COUNT: u16 = 14;
pub const ITEM_FIELD_DURATION: u16 = 15;
pub const ITEM_FIELD_SPELL_CHARGES: u16 = 16;
pub const ITEM_FIELD_FLAGS: u16 = 21;
pub const ITEM_FIELD_ENCHANTMENT: u16 = 22;
pub const ITEM_FIELD_PROPERTY_SEED: u16 = 43;
pub const ITEM_FIELD_RANDOM_PROPERTIES_ID: u16 = 44;
pub const ITEM_FIELD_ITEM_TEXT_ID: u16 = 45;
pub const ITEM_FIELD_DURABILITY: u16 = 46;
pub const ITEM_FIELD_MAXDURABILITY: u16 = 47;
pub const ITEM_END: u16 = 48;

// ── Container ───────────────────────────────────────────────────────────
pub const CONTAINER_FIELD_NUM_SLOTS: u16 = 48;
pub const CONTAINER_FIELD_SLOT_1: u16 = 50;
pub const CONTAINER_END: u16 = 122;

// ── Unit ────────────────────────────────────────────────────────────────
pub const UNIT_FIELD_CHARM: u16 = 6;
pub const UNIT_FIELD_SUMMON: u16 = 8;
pub const UNIT_FIELD_CHARMEDBY: u16 = 10;
pub const UNIT_FIELD_SUMMONEDBY: u16 = 12;
pub const UNIT_FIELD_CREATEDBY: u16 = 14;
pub const UNIT_FIELD_TARGET: u16 = 16;
pub const UNIT_FIELD_PERSUADED: u16 = 18;
pub const UNIT_FIELD_CHANNEL_OBJECT: u16 = 20;
pub const UNIT_FIELD_HEALTH: u16 = 22;
pub const UNIT_FIELD_POWER1: u16 = 23;
pub const UNIT_FIELD_MAXHEALTH: u16 = 28;
pub const UNIT_FIELD_MAXPOWER1: u16 = 29;
pub const UNIT_FIELD_LEVEL: u16 = 34;
pub const UNIT_FIELD_FACTIONTEMPLATE: u16 = 35;
pub const UNIT_FIELD_BYTES_0: u16 = 36;
pub const UNIT_VIRTUAL_ITEM_SLOT_DISPLAY: u16 = 37;
pub const UNIT_VIRTUAL_ITEM_INFO: u16 = 40;
pub const UNIT_FIELD_FLAGS: u16 = 46;
pub const UNIT_FIELD_AURA: u16 = 47;
pub const UNIT_FIELD_AURAFLAGS: u16 = 95;
pub const UNIT_FIELD_AURALEVELS: u16 = 101;
pub const UNIT_FIELD_AURAAPPLICATIONS: u16 = 113;
pub const UNIT_FIELD_AURASTATE: u16 = 125;
pub const UNIT_FIELD_BASEATTACKTIME: u16 = 126;
pub const UNIT_FIELD_RANGEDATTACKTIME: u16 = 128;
pub const UNIT_FIELD_BOUNDINGRADIUS: u16 = 129;
pub const UNIT_FIELD_COMBATREACH: u16 = 130;
pub const UNIT_FIELD_DISPLAYID: u16 = 131;
pub const UNIT_FIELD_NATIVEDISPLAYID: u16 = 132;
pub const UNIT_FIELD_MOUNTDISPLAYID: u16 = 133;
pub const UNIT_FIELD_MINDAMAGE: u16 = 134;
pub const UNIT_FIELD_MAXDAMAGE: u16 = 135;
pub const UNIT_FIELD_MINOFFHANDDAMAGE: u16 = 136;
pub const UNIT_FIELD_MAXOFFHANDDAMAGE: u16 = 137;
pub const UNIT_FIELD_BYTES_1: u16 = 138;
pub const UNIT_FIELD_PETNUMBER: u16 = 139;
pub const UNIT_FIELD_PET_NAME_TIMESTAMP: u16 = 140;
pub const UNIT_FIELD_PETEXPERIENCE: u16 = 141;
pub const UNIT_FIELD_PETNEXTLEVELEXP: u16 = 142;
pub const UNIT_DYNAMIC_FLAGS: u16 = 143;
pub const UNIT_CHANNEL_SPELL: u16 = 144;
pub const UNIT_MOD_CAST_SPEED: u16 = 145;
pub const UNIT_CREATED_BY_SPELL: u16 = 146;
pub const UNIT_NPC_FLAGS: u16 = 147;
pub const UNIT_NPC_EMOTESTATE: u16 = 148;
pub const UNIT_TRAINING_POINTS: u16 = 149;
pub const UNIT_FIELD_STAT0: u16 = 150;
pub const UNIT_FIELD_RESISTANCES: u16 = 155;
pub const UNIT_FIELD_BASE_MANA: u16 = 162;
pub const UNIT_FIELD_BASE_HEALTH: u16 = 163;
pub const UNIT_FIELD_BYTES_2: u16 = 164;
pub const UNIT_FIELD_ATTACK_POWER: u16 = 165;
pub const UNIT_FIELD_ATTACK_POWER_MODS: u16 = 166;
pub const UNIT_FIELD_ATTACK_POWER_MULTIPLIER: u16 = 167;
pub const UNIT_FIELD_RANGED_ATTACK_POWER: u16 = 168;
pub const UNIT_FIELD_RANGED_ATTACK_POWER_MODS: u16 = 169;
pub const UNIT_FIELD_RANGED_ATTACK_POWER_MULTIPLIER: u16 = 170;
pub const UNIT_FIELD_MINRANGEDDAMAGE: u16 = 171;
pub const UNIT_FIELD_MAXRANGEDDAMAGE: u16 = 172;
pub const UNIT_FIELD_POWER_COST_MODIFIER: u16 = 173;
pub const UNIT_FIELD_POWER_COST_MULTIPLIER: u16 = 180;
pub const UNIT_END: u16 = 188;

// ── Player ──────────────────────────────────────────────────────────────
pub const PLAYER_DUEL_ARBITER: u16 = 188;
pub const PLAYER_FLAGS: u16 = 190;
pub const PLAYER_GUILDID: u16 = 191;
pub const PLAYER_GUILDRANK: u16 = 192;
pub const PLAYER_BYTES: u16 = 193;
pub const PLAYER_BYTES_2: u16 = 194;
pub const PLAYER_BYTES_3: u16 = 195;
pub const PLAYER_DUEL_TEAM: u16 = 196;
pub const PLAYER_GUILD_TIMESTAMP: u16 = 197;
pub const PLAYER_QUEST_LOG_1_1: u16 = 198;
/// Fields per quest log entry: quest id, counters, timer.
pub const QUEST_LOG_ENTRY_SIZE: u16 = 3;
pub const QUEST_LOG_ENTRIES: u16 = 20;
pub const PLAYER_VISIBLE_ITEM_1_CREATOR: u16 = 258;
pub const PLAYER_VISIBLE_ITEM_1_0: u16 = 260;
pub const PLAYER_VISIBLE_ITEM_1_PROPERTIES: u16 = 268;
/// Fields per visible item: creator (2), entry and 7 enchants (8),
/// properties (1), padding (1).
pub const VISIBLE_ITEM_SIZE: u16 = 12;
pub const PLAYER_FIELD_INV_SLOT_HEAD: u16 = 486;
pub const PLAYER_FIELD_PACK_SLOT_1: u16 = 532;
pub const PLAYER_FIELD_BANK_SLOT_1: u16 = 564;
pub const PLAYER_FIELD_BANKBAG_SLOT_1: u16 = 612;
pub const PLAYER_FIELD_VENDORBUYBACK_SLOT_1: u16 = 624;
pub const PLAYER_FIELD_KEYRING_SLOT_1: u16 = 648;
pub const PLAYER_FARSIGHT: u16 = 712;
pub const PLAYER_FIELD_COMBO_TARGET: u16 = 714;
pub const PLAYER_XP: u16 = 716;
pub const PLAYER_NEXT_LEVEL_XP: u16 = 717;
pub const PLAYER_SKILL_INFO_1_1: u16 = 718;
pub const PLAYER_CHARACTER_POINTS1: u16 = 1102;
pub const PLAYER_CHARACTER_POINTS2: u16 = 1103;
pub const PLAYER_TRACK_CREATURES: u16 = 1104;
pub const PLAYER_TRACK_RESOURCES: u16 = 1105;
pub const PLAYER_BLOCK_PERCENTAGE: u16 = 1106;
pub const PLAYER_DODGE_PERCENTAGE: u16 = 1107;
pub const PLAYER_PARRY_PERCENTAGE: u16 = 1108;
pub const PLAYER_CRIT_PERCENTAGE: u16 = 1109;
pub const PLAYER_RANGED_CRIT_PERCENTAGE: u16 = 1110;
pub const PLAYER_EXPLORED_ZONES_1: u16 = 1111;
pub const PLAYER_REST_STATE_EXPERIENCE: u16 = 1175;
pub const PLAYER_FIELD_COINAGE: u16 = 1176;
pub const PLAYER_FIELD_POSSTAT0: u16 = 1177;
pub const PLAYER_FIELD_NEGSTAT0: u16 = 1182;
pub const PLAYER_FIELD_RESISTANCEBUFFMODSPOSITIVE: u16 = 1187;
pub const PLAYER_FIELD_RESISTANCEBUFFMODSNEGATIVE: u16 = 1194;
pub const PLAYER_FIELD_MOD_DAMAGE_DONE_POS: u16 = 1201;
pub const PLAYER_FIELD_MOD_DAMAGE_DONE_NEG: u16 = 1208;
pub const PLAYER_FIELD_MOD_DAMAGE_DONE_PCT: u16 = 1215;
pub const PLAYER_FIELD_BYTES: u16 = 1222;
pub const PLAYER_AMMO_ID: u16 = 1223;
pub const PLAYER_SELF_RES_SPELL: u16 = 1224;
pub const PLAYER_FIELD_PVP_MEDALS: u16 = 1225;
pub const PLAYER_FIELD_BUYBACK_PRICE_1: u16 = 1226;
pub const PLAYER_FIELD_BUYBACK_TIMESTAMP_1: u16 = 1238;
pub const PLAYER_FIELD_SESSION_KILLS: u16 = 1250;
pub const PLAYER_FIELD_YESTERDAY_KILLS: u16 = 1251;
pub const PLAYER_FIELD_LAST_WEEK_KILLS: u16 = 1252;
pub const PLAYER_FIELD_THIS_WEEK_KILLS: u16 = 1253;
pub const PLAYER_FIELD_THIS_WEEK_CONTRIBUTION: u16 = 1254;
pub const PLAYER_FIELD_LIFETIME_HONORABLE_KILLS: u16 = 1255;
pub const PLAYER_FIELD_LIFETIME_DISHONORABLE_KILLS: u16 = 1256;
pub const PLAYER_FIELD_YESTERDAY_CONTRIBUTION: u16 = 1257;
pub const PLAYER_FIELD_LAST_WEEK_CONTRIBUTION: u16 = 1258;
pub const PLAYER_FIELD_LAST_WEEK_RANK: u16 = 1259;
pub const PLAYER_FIELD_BYTES2: u16 = 1260;
pub const PLAYER_FIELD_WATCHED_FACTION_INDEX: u16 = 1261;
pub const PLAYER_FIELD_COMBAT_RATING_1: u16 = 1262;
pub const PLAYER_END: u16 = 1282;

// ── Game object ─────────────────────────────────────────────────────────
pub const OBJECT_FIELD_CREATED_BY: u16 = 6;
pub const GAMEOBJECT_DISPLAYID: u16 = 8;
pub const GAMEOBJECT_FLAGS: u16 = 9;
pub const GAMEOBJECT_ROTATION: u16 = 10;
pub const GAMEOBJECT_STATE: u16 = 14;
pub const GAMEOBJECT_POS_X: u16 = 15;
pub const GAMEOBJECT_POS_Y: u16 = 16;
pub const GAMEOBJECT_POS_Z: u16 = 17;
pub const GAMEOBJECT_FACING: u16 = 18;
pub const GAMEOBJECT_DYN_FLAGS: u16 = 19;
pub const GAMEOBJECT_FACTION: u16 = 20;
pub const GAMEOBJECT_TYPE_ID: u16 = 21;
pub const GAMEOBJECT_LEVEL: u16 = 22;
pub const GAMEOBJECT_ARTKIT: u16 = 23;
pub const GAMEOBJECT_ANIMPROGRESS: u16 = 24;
pub const GAMEOBJECT_END: u16 = 26;

// ── Dynamic object ──────────────────────────────────────────────────────
pub const DYNAMICOBJECT_CASTER: u16 = 6;
pub const DYNAMICOBJECT_BYTES: u16 = 8;
pub const DYNAMICOBJECT_SPELLID: u16 = 9;
pub const DYNAMICOBJECT_RADIUS: u16 = 10;
pub const DYNAMICOBJECT_POS_X: u16 = 11;
pub const DYNAMICOBJECT_FACING: u16 = 14;
pub const DYNAMICOBJECT_END: u16 = 16;

// ── Corpse ──────────────────────────────────────────────────────────────
pub const CORPSE_FIELD_OWNER: u16 = 6;
pub const CORPSE_FIELD_FACING: u16 = 8;
pub const CORPSE_FIELD_POS_X: u16 = 9;
pub const CORPSE_FIELD_DISPLAY_ID: u16 = 12;
pub const CORPSE_FIELD_ITEM: u16 = 13;
pub const CORPSE_FIELD_BYTES_1: u16 = 32;
pub const CORPSE_FIELD_BYTES_2: u16 = 33;
pub const CORPSE_FIELD_GUILD: u16 = 34;
pub const CORPSE_FIELD_FLAGS: u16 = 35;
pub const CORPSE_FIELD_DYNAMIC_FLAGS: u16 = 36;
pub const CORPSE_END: u16 = 38;

// ── OBJECT_FIELD_TYPE bits ──────────────────────────────────────────────
pub const TYPEMASK_OBJECT: u32 = 0x0001;
pub const TYPEMASK_ITEM: u32 = 0x0002;
pub const TYPEMASK_CONTAINER: u32 = 0x0004;
pub const TYPEMASK_UNIT: u32 = 0x0008;
pub const TYPEMASK_PLAYER: u32 = 0x0010;
pub const TYPEMASK_GAMEOBJECT: u32 = 0x0020;
pub const TYPEMASK_DYNAMICOBJECT: u32 = 0x0040;
pub const TYPEMASK_CORPSE: u32 = 0x0080;

/// The object type byte of a create block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TypeId {
    Item = 1,
    Container = 2,
    Unit = 3,
    Player = 4,
    GameObject = 5,
    DynamicObject = 6,
    Corpse = 7,
}

impl TypeId {
    pub const fn field_count(self) -> u16 {
        match self {
            TypeId::Item => ITEM_END,
            TypeId::Container => CONTAINER_END,
            TypeId::Unit => UNIT_END,
            TypeId::Player => PLAYER_END,
            TypeId::GameObject => GAMEOBJECT_END,
            TypeId::DynamicObject => DYNAMICOBJECT_END,
            TypeId::Corpse => CORPSE_END,
        }
    }

    pub const fn type_mask(self) -> u32 {
        TYPEMASK_OBJECT
            | match self {
                TypeId::Item => TYPEMASK_ITEM,
                TypeId::Container => TYPEMASK_ITEM | TYPEMASK_CONTAINER,
                TypeId::Unit => TYPEMASK_UNIT,
                TypeId::Player => TYPEMASK_UNIT | TYPEMASK_PLAYER,
                TypeId::GameObject => TYPEMASK_GAMEOBJECT,
                TypeId::DynamicObject => TYPEMASK_DYNAMICOBJECT,
                TypeId::Corpse => TYPEMASK_CORPSE,
            }
    }
}

/// A bit set over field indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    words: Vec<u32>,
}

impl Mask {
    pub fn new(field_count: u16) -> Self {
        Self {
            words: vec![0; usize::from(field_count).div_ceil(32)],
        }
    }

    #[inline]
    pub fn set(&mut self, index: u16) {
        let i = usize::from(index);
        if let Some(w) = self.words.get_mut(i / 32) {
            *w |= 1 << (i % 32);
        }
    }

    #[inline]
    pub fn get(&self, index: u16) -> bool {
        let i = usize::from(index);
        self.words
            .get(i / 32)
            .is_some_and(|w| w & (1 << (i % 32)) != 0)
    }

    pub fn clear(&mut self) {
        self.words.iter_mut().for_each(|w| *w = 0);
    }

    pub fn any(&self) -> bool {
        self.words.iter().any(|w| *w != 0)
    }

    pub fn words(&self) -> &[u32] {
        &self.words
    }

    /// Every set index, in order.
    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let bit = w.trailing_zeros();
                w &= w - 1;
                Some((wi * 32 + bit as usize) as u16)
            })
        })
    }
}

/// An object's update fields, with the ones changed since the last update
/// marked dirty.
#[derive(Debug, Clone)]
pub struct Fields {
    type_id: TypeId,
    values: Vec<u32>,
    dirty: Mask,
}

impl Fields {
    pub fn new(type_id: TypeId, guid: Guid, entry: u32) -> Self {
        let count = type_id.field_count();
        let mut f = Self {
            type_id,
            values: vec![0; usize::from(count)],
            dirty: Mask::new(count),
        };
        f.set_guid(OBJECT_FIELD_GUID, guid);
        f.set_u32(OBJECT_FIELD_TYPE, type_id.type_mask());
        if entry != 0 {
            f.set_u32(OBJECT_FIELD_ENTRY, entry);
        }
        f.set_f32(OBJECT_FIELD_SCALE_X, 1.0);
        f
    }

    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    pub fn len(&self) -> u16 {
        self.values.len() as u16
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[inline]
    pub fn get_u32(&self, index: u16) -> u32 {
        self.values.get(usize::from(index)).copied().unwrap_or(0)
    }

    #[inline]
    pub fn get_f32(&self, index: u16) -> f32 {
        f32::from_bits(self.get_u32(index))
    }

    pub fn get_guid(&self, index: u16) -> Guid {
        Guid::from_u32s(self.get_u32(index), self.get_u32(index + 1))
    }

    pub fn get_byte(&self, index: u16, byte: u8) -> u8 {
        (self.get_u32(index) >> (u32::from(byte) * 8)) as u8
    }

    /// Set a field; marks it dirty if the value changed.
    #[inline]
    pub fn set_u32(&mut self, index: u16, value: u32) {
        let Some(slot) = self.values.get_mut(usize::from(index)) else {
            return;
        };
        if *slot != value {
            *slot = value;
            self.dirty.set(index);
        }
    }

    pub fn set_i32(&mut self, index: u16, value: i32) {
        self.set_u32(index, value as u32);
    }

    pub fn set_f32(&mut self, index: u16, value: f32) {
        self.set_u32(index, value.to_bits());
    }

    /// Set a GUID field. Both halves are marked dirty if either changed,
    /// as mangos does: a reader that finds one half of a GUID without the
    /// other can't make sense of it.
    pub fn set_guid(&mut self, index: u16, guid: Guid) {
        let (lower, upper) = guid.to_u32s();
        if self.get_u32(index) == lower && self.get_u32(index + 1) == upper {
            return;
        }
        for (i, v) in [(index, lower), (index + 1, upper)] {
            if let Some(slot) = self.values.get_mut(usize::from(i)) {
                *slot = v;
                self.dirty.set(i);
            }
        }
    }

    pub fn set_bytes(&mut self, index: u16, a: u8, b: u8, c: u8, d: u8) {
        self.set_u32(index, u32::from_le_bytes([a, b, c, d]));
    }

    pub fn set_byte(&mut self, index: u16, byte: u8, value: u8) {
        let shift = u32::from(byte) * 8;
        let old = self.get_u32(index);
        self.set_u32(
            index,
            (old & !(0xFF << shift)) | (u32::from(value) << shift),
        );
    }

    pub fn set_shorts(&mut self, index: u16, a: u16, b: u16) {
        self.set_u32(index, u32::from(a) | (u32::from(b) << 16));
    }

    pub fn set_flag(&mut self, index: u16, flag: u32, on: bool) {
        let old = self.get_u32(index);
        self.set_u32(index, if on { old | flag } else { old & !flag });
    }

    pub fn has_flag(&self, index: u16, flag: u32) -> bool {
        self.get_u32(index) & flag != 0
    }

    pub fn dirty(&self) -> &Mask {
        &self.dirty
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.any()
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    /// The fields an observer of `visibility` gets, among `candidates`
    /// (every non-zero field for a create, the dirty ones for an update),
    /// as `(mask words, values)`.
    pub fn select(&self, visibility: Visibility, dirty_only: bool) -> (Vec<u32>, Vec<u32>) {
        let mut mask = Mask::new(self.len());
        let mut values = Vec::new();
        let allowed = visibility.mask(self.type_id);
        let mut take = |index: u16, value: u32| {
            if allowed.is_none_or(|m| m.get(index)) {
                mask.set(index);
                values.push(value);
            }
        };
        if dirty_only {
            // The type goes in every block, changed or not: wow_messages
            // (and so peon) can't read a block without it. The client
            // doesn't mind.
            let mut typed = false;
            for index in self.dirty.iter() {
                if !typed && index > OBJECT_FIELD_TYPE {
                    take(
                        OBJECT_FIELD_TYPE,
                        self.values[usize::from(OBJECT_FIELD_TYPE)],
                    );
                }
                typed |= index >= OBJECT_FIELD_TYPE;
                take(index, self.values[usize::from(index)]);
            }
        } else {
            for (index, &value) in self.values.iter().enumerate() {
                if value != 0 {
                    take(index as u16, value);
                }
            }
        }
        // Trailing empty mask words are left off.
        let mut words = mask.words;
        while words.last() == Some(&0) {
            words.pop();
        }
        (words, values)
    }
}

/// Who is looking at an object, for the fields they may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// The object's owner: every field.
    Owner,
    /// Anyone else.
    Public,
}

impl Visibility {
    /// The fields this observer may see of an object of `type_id`; `None`
    /// for all of them.
    fn mask(self, type_id: TypeId) -> Option<&'static Mask> {
        match (self, type_id) {
            (Visibility::Public, TypeId::Player) => Some(player_public_mask()),
            _ => None,
        }
    }
}

/// The fields of a player other players see (mangos's `updateVisualBits`).
fn player_public_mask() -> &'static Mask {
    static MASK: std::sync::OnceLock<Mask> = std::sync::OnceLock::new();
    MASK.get_or_init(|| {
        let mut m = Mask::new(PLAYER_END);
        let mut set_range = |from: u16, to: u16| {
            for i in from..to {
                m.set(i);
            }
        };
        set_range(OBJECT_FIELD_GUID, OBJECT_FIELD_GUID + 2);
        set_range(OBJECT_FIELD_TYPE, OBJECT_FIELD_TYPE + 1);
        set_range(OBJECT_FIELD_SCALE_X, OBJECT_FIELD_SCALE_X + 1);
        set_range(UNIT_FIELD_CHARM, UNIT_FIELD_HEALTH);
        set_range(UNIT_FIELD_HEALTH, UNIT_FIELD_BYTES_0 + 1);
        set_range(UNIT_FIELD_FLAGS, UNIT_FIELD_AURASTATE);
        set_range(UNIT_FIELD_BASEATTACKTIME, UNIT_FIELD_BASEATTACKTIME + 2);
        set_range(UNIT_FIELD_BOUNDINGRADIUS, UNIT_FIELD_MOUNTDISPLAYID + 1);
        set_range(UNIT_FIELD_BYTES_1, UNIT_FIELD_PET_NAME_TIMESTAMP + 1);
        set_range(UNIT_DYNAMIC_FLAGS, UNIT_MOD_CAST_SPEED + 1);
        set_range(UNIT_FIELD_BYTES_2, UNIT_FIELD_BYTES_2 + 1);
        set_range(PLAYER_DUEL_ARBITER, PLAYER_GUILD_TIMESTAMP + 1);
        for entry in 0..QUEST_LOG_ENTRIES {
            let i = PLAYER_QUEST_LOG_1_1 + entry * QUEST_LOG_ENTRY_SIZE;
            set_range(i, i + 1);
        }
        for slot in 0..19 {
            let base = PLAYER_VISIBLE_ITEM_1_CREATOR + slot * VISIBLE_ITEM_SIZE;
            // Creator (2), entry and enchants (8), properties (1).
            set_range(base, base + 11);
        }
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_marks_a_field_dirty_and_the_same_value_does_not() {
        let mut f = Fields::new(TypeId::Unit, Guid::new(5), 299);
        f.clear_dirty();
        f.set_u32(UNIT_FIELD_HEALTH, 10);
        assert!(f.dirty().get(UNIT_FIELD_HEALTH));
        f.clear_dirty();
        f.set_u32(UNIT_FIELD_HEALTH, 10);
        assert!(!f.is_dirty());
    }

    #[test]
    fn others_see_a_players_health_but_not_their_money() {
        let mut f = Fields::new(TypeId::Player, Guid::new(1), 0);
        f.set_u32(UNIT_FIELD_HEALTH, 50);
        f.set_u32(PLAYER_FIELD_COINAGE, 99);
        let (mask, values) = f.select(Visibility::Public, false);
        let mask = Mask { words: mask };
        assert!(mask.get(UNIT_FIELD_HEALTH));
        assert!(!mask.get(PLAYER_FIELD_COINAGE));
        assert!(values.contains(&50) && !values.contains(&99));
        let (mask, _) = f.select(Visibility::Owner, false);
        assert!(Mask { words: mask }.get(PLAYER_FIELD_COINAGE));
    }

    #[test]
    fn a_mask_iterates_its_bits_in_order() {
        let mut m = Mask::new(100);
        for i in [3, 31, 32, 99] {
            m.set(i);
        }
        assert_eq!(m.iter().collect::<Vec<_>>(), vec![3, 31, 32, 99]);
    }
}
