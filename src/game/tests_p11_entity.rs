//! Round 11's entity rows (docs/parity/matrix.csv): what a creature's
//! guardian is worth, and the rest of the world's entities.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellDuration, SpellEffect};
use crate::game::fields::{PLAYER_XP, UNIT_DYNAMIC_FLAGS};
use crate::game::map::Map;
use crate::game::tests_p3_combat::{BRUTE, PACK, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{spell, take_quest};
use crate::game::tests_p4_world::on;

/// A creature's guardian, its summoning, and a quest to kill one.
const IMP: u32 = 98_400;
const SUMMON_IMP: u32 = 98_401;
const IMP_QUEST: u32 = 98_402;

fn guardian_world() -> GameData {
    let mut d = data();
    d.spells.durations.insert(
        4,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    let mut imp = template(IMP, 3);
    imp.faction = PACK;
    imp.gold_min = 100;
    imp.gold_max = 100;
    d.creature_templates.insert(IMP, imp);
    let mut s = spell(
        SUMMON_IMP,
        &[SpellEffect {
            effect: crate::game::map::pets::EFFECT_SUMMON_GUARDIAN,
            misc_value: IMP as i32,
            base_points: 0,
            base_dice: 1,
            target_a: 32,
            ..Default::default()
        }],
    );
    s.duration_index = 4;
    d.spells.spells.insert(SUMMON_IMP, s);
    d.npc.quests.insert(
        IMP_QUEST,
        crate::game::data::npc::Quest {
            id: IMP_QUEST,
            required_kills: [(IMP as i32, 5), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d
}

fn xp(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(PLAYER_XP)
}

fn kills(m: &Map, g: Guid) -> u32 {
    m.players[&g]
        .journal
        .log
        .iter()
        .find(|l| l.quest == IMP_QUEST)
        .map_or(0, |l| l.kills[0])
}

/// Thistle Tea's "NPC pets grant one reward and leave an unlootable corpse
/// until removal" (summon_lifecycle_test 36) and "NPC pets award reduced
/// XP alongside quest and reputation credit" (player_group_reward_test
/// 38), as the user settled them (#51, mangos-classic's): a creature's
/// guardian killed gives no experience (`XP::Gain`, nothing for `IsPet`,
/// where wvs gave its template's whole and Thistle Tea three quarters),
/// its kill's credit still counts (`RewardSinglePlayerAtKill`'s
/// `KilledMonster`), once, and its corpse has nothing to loot; the same
/// creature spawned on its own gives its experience and its gold.
#[tokio::test]
async fn a_creatures_guardian_gives_no_experience_but_its_credit_once_and_no_loot() {
    let (_realm, _a, ga, map) = setup_with(guardian_world(), Class::Warrior).await;
    let got = on(&map, move |m| {
        take_quest(m, ga, IMP_QUEST);
        let brute = spawn(m, ga, BRUTE, 20, 10.0);
        crate::game::map::spells::cast(
            m,
            brute,
            SUMMON_IMP,
            crate::game::map::spells::Targets::default(),
            None,
        );
        let imp = m
            .creatures
            .iter()
            .find(|(_, c)| c.entry == IMP && c.alive())
            .map(|(_, c)| c.guid)
            .expect("the brute's imp");
        let guardian = m
            .creature(imp)
            .and_then(|c| c.scripted.as_deref())
            .is_some_and(|s| s.guardian);
        let before = xp(m, ga);
        crate::game::map::combat::kill(m, ga, imp);
        let gained = xp(m, ga) - before;
        let credit = kills(m, ga);
        let corpse = m.creature(imp).map(|c| {
            (
                c.alive(),
                c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & 0x1 != 0,
                c.loot.as_ref().map_or(0, |l| l.gold),
            )
        });
        // Killed again, it is dead already: nothing more.
        crate::game::map::combat::kill(m, ga, imp);
        let credit_again = kills(m, ga);
        // The same creature of its own: its experience and its gold.
        let own = spawn(m, ga, IMP, 20, 5.0);
        let before = xp(m, ga);
        crate::game::map::combat::kill(m, ga, own);
        let own_gained = xp(m, ga) - before;
        let own_lootable = m
            .creature(own)
            .is_some_and(|c| c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & 0x1 != 0);
        (
            guardian,
            gained,
            credit,
            corpse,
            credit_again,
            own_gained,
            own_lootable,
        )
    })
    .await;
    let (guardian, gained, credit, corpse, credit_again, own_gained, own_lootable) = got;
    assert!(guardian, "the brute's imp is its guardian");
    assert_eq!(gained, 0, "a creature's guardian gives no experience");
    assert_eq!(credit, 1, "its kill counts for the quest");
    assert_eq!(
        corpse,
        Some((false, false, 0)),
        "a corpse with nothing on it"
    );
    assert_eq!(credit_again, 1, "once");
    assert!(own_gained > 0, "one of its own gives its experience");
    assert!(own_lootable, "and its gold");
}
