//! Game data for the player systems around the world: factions and their
//! reputation, page texts, mail templates, exploration XP and meeting
//! stones.

use ahash::AHashMap;

/// A faction players have a standing with (`Faction.dbc` with a
/// reputation index).
#[derive(Debug, Clone, Default)]
pub struct FactionDef {
    pub id: u32,
    /// Its slot among the 64 a client knows.
    pub index: u32,
    pub parent: u32,
    pub name: String,
    /// Up to four (race mask, class mask, base standing, flags) choices;
    /// the first that fits a player is theirs.
    pub variants: [(u32, u32, i32, u32); 4],
}

impl FactionDef {
    /// The base standing and flags for a race and class.
    pub fn base(&self, race: u8, class: u8) -> (i32, u32) {
        let race_bit = if race > 0 { 1u32 << (race - 1) } else { 0 };
        let class_bit = if class > 0 { 1u32 << (class - 1) } else { 0 };
        self.variants
            .iter()
            .find(|(r, c, _, _)| (*r == 0 || r & race_bit != 0) && (*c == 0 || c & class_bit != 0))
            .filter(|(r, c, _, _)| *r != 0 || *c != 0)
            .map(|(_, _, b, f)| (*b, *f))
            .unwrap_or((0, 0))
    }
}

/// `reputation_spillover_template`: a gain with one faction spills to
/// another at a rate, up to a rank.
#[derive(Debug, Clone, Copy, Default)]
pub struct Spillover {
    pub faction: u32,
    pub rate: f32,
    pub max_rank: u32,
}

/// `creature_onkill_reputation`: what killing a creature is worth.
#[derive(Debug, Clone, Copy, Default)]
pub struct KillReputation {
    pub faction: u32,
    pub value: i32,
    pub max_rank: u32,
    /// 0 both teams, 1 the Alliance's, 2 the Horde's.
    pub team: u8,
    /// Half again to the faction's parent.
    pub team_award: bool,
}

/// A quest's mail when it's done (`RewMailTemplateId`).
#[derive(Debug, Clone, Copy, Default)]
pub struct QuestMail {
    pub template: u32,
    pub delay_secs: u32,
    pub money: u32,
}

/// A meeting stone: who may queue at it, and for where.
#[derive(Debug, Clone, Copy, Default)]
pub struct MeetingStone {
    pub min_level: u32,
    pub max_level: u32,
    pub area: u32,
}

#[derive(Debug, Default)]
pub struct MiscData {
    pub factions: AHashMap<u32, FactionDef>,
    /// Reputation index → faction.
    pub faction_by_index: AHashMap<u32, u32>,
    pub spillovers: AHashMap<u32, Vec<Spillover>>,
    pub kill_reputation: AHashMap<u32, Vec<KillReputation>>,
    /// `page_text`: entry → text and the next page.
    pub page_texts: AHashMap<u32, (String, u32)>,
    /// Mail templates: id → subject and body.
    pub mail_templates: AHashMap<u32, (String, String)>,
    /// `mail_loot_template`: template → items and counts it sends.
    pub mail_items: AHashMap<u32, Vec<(u32, u32)>>,
    pub quest_mail: AHashMap<u32, QuestMail>,
    /// `reputation_reward_rate`: faction → its quest, kill and spell rates.
    pub reward_rates: AHashMap<u32, (f32, f32, f32)>,
    /// `exploration_basexp`: level → XP for finding an area.
    pub exploration_xp: AHashMap<u32, u32>,
    /// `player_crit_per_agility`: (class, level) → agility a percent of
    /// melee crit, for the levels it lists.
    pub crit_per_agility: AHashMap<(u8, u8), f32>,
    /// Meeting stones by game object entry.
    pub meeting_stones: AHashMap<u32, MeetingStone>,
    /// `cinematic_waypoints`: where an intro's camera is at each point of
    /// its run, by cinematic sequence: (milliseconds, x, y, z), in order.
    pub cinematic_waypoints: AHashMap<u32, Vec<(u32, f32, f32, f32)>>,
}

impl MiscData {
    /// The agility a percent of melee crit takes for `class` at `level`:
    /// the rate of the highest level listed up to it (Thistle Tea's
    /// `PlayerCritPerAgility.rate`), if any is.
    pub fn crit_per_agility(&self, class: u8, level: u8) -> Option<f32> {
        (1..=level)
            .rev()
            .find_map(|l| self.crit_per_agility.get(&(class, l)).copied())
            .filter(|r| *r > 0.0)
    }

    pub fn faction(&self, id: u32) -> Option<&FactionDef> {
        self.factions.get(&id)
    }
}

/// Reputation ranks and where they start.
pub const RANKS: [i32; 8] = [-42_000, -6_000, -3_000, 0, 3_000, 9_000, 21_000, 42_000];
pub const RANK_HATED: u32 = 0;
pub const RANK_HOSTILE: u32 = 1;
pub const RANK_UNFRIENDLY: u32 = 2;
pub const RANK_NEUTRAL: u32 = 3;
pub const RANK_FRIENDLY: u32 = 4;
pub const RANK_HONORED: u32 = 5;
pub const RANK_REVERED: u32 = 6;
pub const RANK_EXALTED: u32 = 7;
pub const STANDING_MIN: i32 = -42_000;
pub const STANDING_MAX: i32 = 42_999;

/// The rank a total standing is at.
pub fn rank(standing: i32) -> u32 {
    RANKS.iter().rposition(|m| standing >= *m).unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_start_where_vanilla_puts_them() {
        assert_eq!(rank(-42_000), RANK_HATED);
        assert_eq!(rank(-6_000), RANK_HOSTILE);
        assert_eq!(rank(-1), RANK_UNFRIENDLY);
        assert_eq!(rank(0), RANK_NEUTRAL);
        assert_eq!(rank(2_999), RANK_NEUTRAL);
        assert_eq!(rank(9_000), RANK_HONORED);
        assert_eq!(rank(42_999), RANK_EXALTED);
    }

    #[test]
    fn the_first_variant_that_fits_gives_the_base() {
        let f = FactionDef {
            variants: [
                (1 | 4, 0, 3000, 0x11),
                (0, 1 << 3, -500, 0x1),
                (0, 0, 0, 0),
                (0, 0, 0, 0),
            ],
            ..Default::default()
        };
        assert_eq!(f.base(1, 1), (3000, 0x11));
        assert_eq!(f.base(2, 4), (-500, 0x1));
        assert_eq!(f.base(2, 1), (0, 0));
    }
}
