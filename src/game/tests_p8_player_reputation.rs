//! Round 8's player: reputation's rewards and their auras.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::misc::{FactionDef, KillReputation, RANK_EXALTED, Spillover};
use crate::game::data::npc::Quest;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::reputation::{creature_killed, quest_rewarded, spell_reward, standing};

const ARGENT: u32 = 529;
const CENARION: u32 = 609;
const DARNASSUS: u32 = 69;
const GHOUL: u32 = 20_300;
/// +10% to every gain, and +20% to kills' with the Argent Dawn.
const DIPLOMAT: u32 = 94_400;

fn data() -> GameData {
    let mut d = GameData::empty();
    for (id, index) in [(ARGENT, 13), (CENARION, 14), (DARNASSUS, 15)] {
        d.misc.factions.insert(
            id,
            FactionDef {
                id,
                index,
                name: format!("Faction {id}"),
                variants: [(0xFF, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
                ..Default::default()
            },
        );
        d.misc.faction_by_index.insert(index, id);
    }
    // The Cenarion Circle's gains spill a half to Darnassus.
    d.misc.spillovers.insert(
        CENARION,
        vec![Spillover {
            faction: DARNASSUS,
            rate: 0.5,
            max_rank: RANK_EXALTED,
        }],
    );
    d.misc.kill_reputation.insert(
        GHOUL,
        vec![KillReputation {
            faction: ARGENT,
            value: 100,
            max_rank: RANK_EXALTED,
            team: 0,
            team_award: false,
        }],
    );
    let mut diplomat = SpellTemplate {
        id: DIPLOMAT,
        name: "Diplomat".into(),
        range_index: 1,
        duration_index: 21,
        equipped_item_class: -1,
        ..Default::default()
    };
    for (i, (aura, misc, amount)) in [(156, 0, 10), (190, ARGENT as i32, 20)]
        .into_iter()
        .enumerate()
    {
        diplomat.effects[i] = SpellEffect {
            effect: 6,
            aura,
            misc_value: misc,
            base_points: amount - 1,
            base_dice: 1,
            die_sides: 1,
            target_a: 1,
            ..Default::default()
        };
    }
    d.spells.spells.insert(DIPLOMAT, diplomat);
    d
}

/// A creature of `entry` at the player's level beside them.
fn ghoul(m: &mut Map, g: Guid) -> Guid {
    let level = m.players[&g].level();
    let at = m.players[&g].position();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let t = CreatureTemplate {
        entry: GHOUL,
        name: "Ghoul".into(),
        display_ids: [903, 0, 0, 0],
        health_max: 100,
        ..Default::default()
    };
    let c = crate::game::map::creature::Creature::new(
        crate::game::guid::unit(GHOUL, counter | 0x80_0000),
        &t,
        level,
        Vector3d {
            x: at.x + 2.0,
            ..at
        },
        0.0,
    );
    m.add_creature(c)
}

/// Thistle Tea's reputation_test "applies general and kill-only faction
/// aura bonuses": a kill's reputation takes both the auras that raise all
/// gains (156) and those that raise one faction's (190), a spell's only
/// the first: 100 a kill is 130, 100 a spell 110. As mangos's
/// `CalculateReputationGain`, which wvs lacked the second of.
#[tokio::test]
async fn a_kill_takes_its_factions_aura_bonus_and_a_spell_only_the_general_one() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let got = map
        .query(move |m| {
            let data = m.realm.data.clone();
            crate::game::map::spells::cast_triggered(m, g, DIPLOMAT, Some(g));
            let victim = ghoul(m, g);
            creature_killed(m, g, victim);
            let kill = standing(&m.players[&g], &data, ARGENT);
            spell_reward(m, g, ARGENT, 100);
            (kill, standing(&m.players[&g], &data, ARGENT))
        })
        .await
        .unwrap();
    assert_eq!(got, (130, 240));
}

/// Thistle Tea's reputation_test "applies every translated quest reward":
/// each of a quest's rewards is given, a loss as a gain, and one its
/// spillover mask marks spills nowhere.
#[tokio::test]
async fn a_quest_gives_each_reward_and_spills_only_where_its_mask_lets() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let quest = |no_spill: u32| Quest {
        id: 1,
        level: 1,
        reputation_rewards: [(CENARION, 250), (ARGENT, -25), (0, 0), (0, 0), (0, 0)],
        reputation_no_spillover: no_spill,
        ..Default::default()
    };
    let got = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let of = |m: &Map, f| standing(&m.players[&g], &data, f);
            quest_rewarded(m, g, &quest(0));
            let spilled = (of(m, CENARION), of(m, ARGENT), of(m, DARNASSUS));
            quest_rewarded(m, g, &quest(1));
            (spilled, (of(m, CENARION), of(m, ARGENT), of(m, DARNASSUS)))
        })
        .await
        .unwrap();
    assert_eq!(got.0, (250, -25, 125), "a half to Darnassus");
    assert_eq!(got.1, (500, -50, 125), "masked, none");
}
