//! Round 12's stage h, skills as mangos-classic raises them: a rogue's
//! Poisons by their recipes (`UpdateCraftSkill`), Mining and Skinning with
//! no halving every 75 points (`SkillChance.MiningSteps` 0), and one roll
//! a node, whatever it comes to (`EffectOpenLock`'s skill-up list).

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::crafts::LockKey;
use crate::game::data::spells::{SkillAbility, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapKey, loot, professions, skills};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const POISONS: u32 = 40;
const MINING: u32 = 186;
/// Instant Poison's recipe as the client's data has it: Poisons, yellow
/// at 125 and grey at 175.
const INSTANT_POISON: u32 = 900_800;
const POISON: u32 = 9500;
const MINE: u32 = 900_801;
const VEIN: u32 = 180_950;
const ORE: u32 = 9501;
/// A vein's lock: Mining (3) from 150.
const VEIN_LOCK: u32 = 60;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 20,
        allowable_class: -1,
        allowable_race: -1,
        buy_count: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 24,
        item_type: u64::from(POISON),
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        INSTANT_POISON,
        SpellTemplate {
            id: INSTANT_POISON,
            name: "Instant Poison".into(),
            effects,
            equipped_item_class: -1,
            range_index: 1,
            ..Default::default()
        },
    );
    d.spells.skill_abilities.insert(
        INSTANT_POISON,
        vec![SkillAbility {
            skill: POISONS,
            spell: INSTANT_POISON,
            trivial_low: 125,
            trivial_high: 175,
            ..Default::default()
        }],
    );
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: professions::EFFECT_OPEN_LOCK,
        misc_value: 3,
        base_dice: 1,
        target_a: 25,
        ..Default::default()
    };
    d.spells.spells.insert(
        MINE,
        SpellTemplate {
            id: MINE,
            name: "Mining".into(),
            effects,
            equipped_item_class: -1,
            range_index: 1,
            ..Default::default()
        },
    );
    d.crafts.locks.insert(
        VEIN_LOCK,
        vec![LockKey {
            kind: crate::game::data::crafts::LOCK_KEY_SKILL,
            index: 3,
            skill: 150,
        }],
    );
    let mut vein = [0u32; 24];
    (vein[0], vein[1]) = (VEIN_LOCK, 61);
    d.gameobject_templates.insert(
        VEIN,
        GameObjectTemplate {
            entry: VEIN,
            kind: professions::GO_CHEST,
            display_id: 1,
            name: "Vein".into(),
            size: 1.0,
            data: vein,
            ..Default::default()
        },
    );
    d.gameobject_spawns.insert(
        0,
        (0..5)
            .map(|n| GameObjectSpawn {
                guid: 800 + n,
                entry: VEIN,
                map: 0,
                x: START.0 + 1.0,
                y: START.1 + n as f32 * 0.5,
                z: START.2,
                state: 1,
                respawn_min: 60,
                ..Default::default()
            })
            .collect(),
    );
    for e in [POISON, ORE] {
        d.items.insert(e, item(e));
    }
    d.loot.gameobject.insert(
        61,
        vec![
            LootEntry {
                item: ORE,
                chance: 100.0,
                min: 1,
                max: 1,
                ..Default::default()
            },
            LootEntry {
                item: ORE,
                chance: 100.0,
                min: 1,
                max: 1,
                ..Default::default()
            },
        ],
    );
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// `skill` learned at `step`, at `value`.
fn learn_at(m: &mut Map, g: Guid, skill: u32, step: u32, value: u32) {
    let p = m.players.get_mut(&g).unwrap();
    skills::learn_skill(p, skill, step);
    skills::set_value(p, skill, value);
}

fn value(m: &Map, g: Guid, skill: u32) -> u32 {
    skills::value(&m.players[&g], skill).unwrap_or(0)
}

fn vein(n: u32) -> Guid {
    guid::game_object(VEIN, 800 + n)
}

/// `g` mines `node`, the attempt taking.
fn mine(m: &mut Map, g: Guid, node: Guid) {
    let s = m.realm.data.spells.get(MINE).cloned().unwrap();
    m.spells.openings_rolled.insert(g);
    professions::execute(
        m,
        g,
        &s,
        &Targets {
            object: Some(node),
            ..Default::default()
        },
        None,
    );
    m.spells.openings_rolled.remove(&g);
}

/// mangos's `UpdateCraftSkill`: a poison made raises Poisons by its
/// recipe's colour, as any recipe its skill, so a rogue's Poisons climbs
/// to the trainers' ranks (it stayed at 1, and no rank past the first could
/// be trained).
#[tokio::test]
async fn poisons_rise_as_poisons_are_made() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Rogue).await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(UNIT_FIELD_LEVEL, 30);
        learn_at(m, g, POISONS, 0, 1);
        crate::game::map::spells::learn(m, g, INSTANT_POISON);
    })
    .await;
    let before = on_map(&realm, move |m| value(m, g, POISONS)).await;
    assert_eq!(before, 1);
    let made = move |m: &mut Map| {
        m.players[&g]
            .items
            .iter()
            .filter(|i| i.entry == POISON)
            .map(|i| i.fields.get_u32(ITEM_FIELD_STACK_COUNT).max(1))
            .sum::<u32>()
    };
    for n in 1..=3 {
        a.send(CMSG_CAST_SPELL {
            spell: INSTANT_POISON,
            targets: crate::game::map::spells::wire_targets(&Targets {
                unit: Some(g),
                ..Default::default()
            }),
        })
        .await;
        let mut tries = 0;
        while on_map(&realm, made).await < n {
            tries += 1;
            assert!(tries < 100, "poison {n} made");
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    }
    let after = on_map(&realm, move |m| value(m, g, POISONS)).await;
    assert_eq!(after, 4, "a point each, orange");
}

/// mangos's `UpdateGatherSkill` at `SkillChance.MiningSteps` 0 (its
/// default): an orange vein at 150 raises Mining every time, where wvs
/// halved the chance each 75 points (25% at 150).
#[tokio::test]
async fn mining_isnt_halved_every_75_points() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(vein(4)).await;
    let gained = on_map(&realm, move |m| {
        learn_at(m, g, MINING, 3, 150);
        for n in 0..4 {
            mine(m, g, vein(n));
            loot::release(m, g, vein(n));
        }
        value(m, g, MINING)
    })
    .await;
    assert_eq!(gained, 154, "a point a vein");
    // The steps are the server's to set: at 75, halved as before.
    let rates = crate::config::RatesConfig {
        skill_chance_mining_steps: 75,
        skill_chance_skinning_steps: 75,
        ..Default::default()
    };
    let none = crate::config::RatesConfig::default();
    let chance = professions::gather_gain_chance;
    assert_eq!(chance(MINING, 150, 150, 1, &none), 100.0);
    assert_eq!(chance(MINING, 150, 150, 1, &rates), 25.0);
    assert_eq!(
        chance(professions::SKILL_SKINNING, 225, 225, 1, &none),
        100.0
    );
    assert_eq!(
        chance(professions::SKILL_SKINNING, 225, 225, 1, &rates),
        12.5
    );
    // A need of 1 counts as 5: Copper stops at 105, not 101.
    assert_eq!(chance(MINING, 103, 1, 1, &none), 25.0);
    assert_eq!(chance(MINING, 105, 1, 1, &none), 0.0);
}

/// mangos's `EffectOpenLock`: a node is rolled once a spawn for each
/// player, whatever the roll, and not once it has its loot; opened, closed
/// and opened again, it isn't rolled again (wvs rolled until a point
/// landed).
#[tokio::test]
async fn a_node_is_rolled_once() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(vein(0)).await;
    b.sees(vein(0)).await;
    let got = on_map(&realm, move |m| {
        // Grey: the first roll can't land.
        learn_at(m, g, MINING, 4, 250);
        mine(m, g, vein(0));
        let grey = value(m, g, MINING);
        loot::release(m, g, vein(0));
        // Orange now: a second roll would.
        skills::set_value(m.players.get_mut(&g).unwrap(), MINING, 150);
        mine(m, g, vein(0));
        let again = value(m, g, MINING);
        loot::release(m, g, vein(0));
        // Another opening a node with its loot made isn't rolled either.
        learn_at(m, gb, MINING, 3, 150);
        mine(m, gb, vein(0));
        (grey, again, value(m, gb, MINING))
    })
    .await;
    assert_eq!(got, (250, 150, 150));
}

const FISHING: u32 = 356;

/// `g`'s bobber in `area`, a fish on it, used: what the hook comes to.
fn hook(m: &mut Map, g: Guid, area: u32) {
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let bobber = guid::game_object(35_591, 0x70_0000 | counter);
    let now = m.now;
    m.loot_state.bobbers.push(professions::Bobber {
        guid: bobber,
        owner: g,
        spell: 0,
        bite_at: now,
        gone_at: now + std::time::Duration::from_secs(20),
        bitten: true,
        zone: 0,
        area,
    });
    professions::use_object(m, g, bobber);
    loot::release(m, g, bobber);
}

/// mangos's `UpdateFishingSkill`, called by `GameObject::Use` on a hooked
/// bobber before the catch is told: a fish that escapes counts as one
/// landed, and the skill rises by steps (`value / 44` hooks a point up to
/// 300, two steps at a chance), where wvs rose only on a catch, by 2500 /
/// (value - 50) percent.
#[tokio::test]
async fn fishing_rises_by_steps_on_every_hooked_fish() {
    let mut d = data();
    // Area 1's fish always get away; area 2's are always landed.
    d.crafts.fishing_skill.insert(1, 500);
    d.crafts.fishing_skill.insert(2, -100);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        learn_at(m, g, FISHING, 1, 50);
        for _ in 0..3 {
            hook(m, g, 1);
        }
        let escaped = value(m, g, FISHING);
        learn_at(m, g, FISHING, 4, 150);
        m.players.get_mut(&g).unwrap().row.state.fishing_steps = 0;
        for _ in 0..9 {
            hook(m, g, 2);
        }
        (escaped, value(m, g, FISHING))
    })
    .await;
    assert_eq!(got.0, 53, "a point an escape below 75");
    // Three steps a point at 150 to 153, one or two a hook.
    assert!((153..=156).contains(&got.1), "{}", got.1);
}
