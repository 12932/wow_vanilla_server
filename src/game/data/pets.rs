//! What pets are made of: vmangos's `pet_levelstats`, `pet_name_generation`,
//! `petcreateinfo_spell`, `pet_spell_data` and `spell_pet_auras`, the
//! `CreatureFamily` and `StableSlotPrices` DBCs, and each family's skill
//! lines from `SkillLineAbility`.

use ahash::AHashMap;

/// A pet's stats at a level (`pet_levelstats`; entry 1 is every hunter
/// pet's).
#[derive(Debug, Clone, Copy, Default)]
pub struct PetLevel {
    pub health: u32,
    pub mana: u32,
    pub armor: u32,
    pub damage: (f32, f32),
    pub stats: [u32; 5],
}

/// A creature family (`CreatureFamily`).
#[derive(Debug, Clone, Default)]
pub struct Family {
    pub id: u32,
    pub name: String,
    /// The family's skill line, and the generic pet skill line.
    pub skills: [u32; 2],
    /// The foods it eats: meat 1, fish 2, cheese 4, bread 8, fungus 16,
    /// fruit 32, raw meat 64, raw fish 128.
    pub food_mask: u32,
    pub min_scale: f32,
    pub max_scale: f32,
}

/// A spell of a pet skill line (`SkillLineAbility`).
#[derive(Debug, Clone, Copy, Default)]
pub struct PetAbility {
    pub spell: u32,
    pub skill: u32,
    /// 2: a family passive, learned on taming.
    pub acquire: u32,
    pub superseded_by: u32,
    /// The training points it costs (vanilla's `num_skills_up` column).
    pub cost: u32,
}

#[derive(Debug, Default)]
pub struct PetData {
    pub levels: AHashMap<(u32, u8), PetLevel>,
    /// Name halves by creature entry: (word, half), half 0 the start.
    pub names: AHashMap<u32, Vec<(String, u8)>>,
    /// `petcreateinfo_spell`: the spells a summoned pet starts with.
    pub create_spells: AHashMap<u32, Vec<u32>>,
    /// `pet_spell_data`, by list: the spells a creature has as a pet.
    pub spell_lists: AHashMap<u32, Vec<u32>>,
    /// Creatures' `pet_spell_list_id`.
    pub creature_spell_list: AHashMap<u32, u32>,
    /// Totems' `totem_spell_id`: the spell each works.
    pub totem_spells: AHashMap<u32, u32>,
    /// `spell_pet_auras`: an owner's aura (by spell) gives its pet another.
    pub owner_auras: Vec<(u32, u32, u32)>,
    pub families: AHashMap<u32, Family>,
    /// The spells of each pet skill line.
    pub abilities: AHashMap<u32, Vec<PetAbility>>,
    /// The cost of each stable slot, from the first.
    pub stable_prices: Vec<u32>,
    /// What the Grimoires teach demons.
    pub grimoire_spells: Vec<u32>,
}

impl PetData {
    pub fn level(&self, entry: u32, level: u8) -> Option<PetLevel> {
        self.levels.get(&(entry, level)).copied()
    }
}
