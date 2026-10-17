//! Phase 3's professions, Thistle Tea's skinning, pickpocketing and
//! disenchanting tests ported: what may be skinned and when, a skill's
//! bonuses and its gains, pockets picked once a life, and items
//! disenchanted.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapKey, loot, professions};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
/// A wolf to skin, a mark with pockets, one without.
const WOLF: u32 = 3400;
const MARK: u32 = 3401;
const BARE: u32 = 3402;
const LEATHER: u32 = 9400;
const COIN: u32 = 9401;
const LETTER: u32 = 9402;
const BRACER: u32 = 9403;
const DUST: u32 = 9404;
const SKIN: u32 = 900_700;
const PICK: u32 = 900_701;
const STEALTH: u32 = 900_702;
const KNACK: u32 = 900_703;
const DISENCHANT: u32 = 900_704;
const SKINNING: u32 = 393;
const ENCHANTING: u32 = 333;

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

fn spell(id: u32, effects: &[(u32, u32, i32, i32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    for (i, (effect, aura, misc, points)) in effects.iter().enumerate() {
        s.effects[i] = SpellEffect {
            effect: *effect,
            aura: *aura,
            misc_value: *misc,
            base_points: *points - 1,
            base_dice: 1,
            target_a: if *aura != 0 { 1 } else { 25 },
            ..Default::default()
        };
    }
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let creature = |entry, level: u8| CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: level,
        level_max: level,
        faction: 32,
        unit_class: 1,
        health_max: 100,
        ..Default::default()
    };
    let mut wolf = creature(WOLF, 5);
    wolf.skinning_loot_id = 10;
    wolf.loot_id = 11;
    wolf.gold_min = 7;
    wolf.gold_max = 7;
    d.creature_templates.insert(WOLF, wolf);
    let mut mark = creature(MARK, 10);
    mark.pickpocket_loot_id = 20;
    d.creature_templates.insert(MARK, mark);
    d.creature_templates.insert(BARE, creature(BARE, 10));
    let mut spawns = Vec::new();
    for (n, entry) in [
        (1, WOLF),
        (2, WOLF),
        (3, MARK),
        (4, MARK),
        (5, BARE),
        (6, WOLF),
    ] {
        spawns.push(CreatureSpawn {
            guid: n,
            entries: vec![entry],
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + (n as f32 - 3.0) * 0.4,
            z: START.2,
            ..Default::default()
        });
    }
    d.creature_spawns.insert(0, spawns);
    for e in [LEATHER, COIN, LETTER, DUST] {
        d.items.insert(e, item(e));
    }
    let mut bracer = item(BRACER);
    bracer.class = 4;
    bracer.quality = 2;
    bracer.item_level = 60;
    bracer.required_level = 55;
    bracer.stackable = 1;
    bracer.disenchant_id = 30;
    d.items.insert(BRACER, bracer);
    let drop = |item, chance: f32| LootEntry {
        item,
        chance,
        min: 1,
        max: 1,
        ..Default::default()
    };
    d.loot.skinning.insert(10, vec![drop(LEATHER, 100.0)]);
    d.loot
        .pickpocket
        .insert(20, vec![drop(COIN, 100.0), drop(LETTER, -100.0)]);
    d.loot.disenchant.insert(30, vec![drop(DUST, 100.0)]);
    d.spells.ranges.insert(1, (0.0, 30.0));
    for s in [
        spell(SKIN, &[(professions::EFFECT_SKINNING, 0, 0, 0)]),
        spell(PICK, &[(professions::EFFECT_PICKPOCKET, 0, 0, 0)]),
        spell(
            STEALTH,
            &[(6, crate::game::map::auras::AURA_MOD_STEALTH, 0, 1)],
        ),
        spell(KNACK, &[(6, 30, SKINNING as i32, 5), (6, 30, 356, 100)]),
        spell(DISENCHANT, &[(professions::EFFECT_DISENCHANT, 0, 0, 0)]),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    // A quest that wants the letter.
    d.npc.quests.insert(
        50,
        crate::game::data::npc::Quest {
            id: 50,
            title: "Letters".into(),
            min_level: 1,
            level: 1,
            required_items: [(LETTER, 1), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
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

fn unit(n: u32) -> Guid {
    let entry = match n {
        3 | 4 => MARK,
        5 => BARE,
        _ => WOLF,
    };
    guid::unit(entry, n)
}

fn at(target: Guid) -> Targets {
    Targets {
        unit: Some(target),
        ..Default::default()
    }
}

/// Why `caster` can't cast `spell` at `t`, as `professions::check` says.
fn why(m: &Map, caster: Guid, spell: u32, t: Targets) -> Option<u8> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    professions::check(m, caster, &s, &t, None)
}

fn run(m: &mut Map, caster: Guid, spell: u32, t: Targets) {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    professions::execute(m, caster, &s, &t, None);
}

fn learn_skill(m: &mut Map, g: Guid, skill: u32, value: u32) {
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::skills::learn_skill(p, skill, 1);
    let i = (0..128u16)
        .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == skill)
        .unwrap();
    p.fields
        .set_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 1, (75 << 16) | value);
}

fn skinnable(m: &Map, c: Guid) -> bool {
    m.creature(c)
        .is_some_and(|c| c.fields.get_u32(UNIT_FIELD_FLAGS) & professions::UNIT_FLAG_SKINNABLE != 0)
}

// `SpellCastResult`s.
const BAD_TARGETS: u8 = 0x0a;
const LOW_CASTLEVEL: u8 = 0x2c;
const ONLY_STEALTHED: u8 = 0x57;
const TARGETS_DEAD: u8 = 0x65;
const TARGET_NOT_DEAD: u8 = 0x6e;
const TARGET_NOT_LOOTED: u8 = 0x70;
const TARGET_NO_POCKETS: u8 = 0x72;
const TARGET_UNSKINNABLE: u8 = 0x74;
const CANT_BE_DISENCHANTED: u8 = 0x0c;

/// Thistle Tea's `Skinning.validate_target` and `sync/1`: not alive, not
/// owned, with a skin, its loot gone, not skinned already; skinnable shown
/// only once looted, and not after its respawn.
#[tokio::test]
async fn a_corpse_is_skinned_once_looted() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(unit(1)).await;
    let wolf = unit(1);
    let checks = on_map(&realm, move |m| {
        // At 300 an attempt never fails.
        learn_skill(m, g, SKINNING, 300);
        let alive = why(m, g, SKIN, at(wolf));
        crate::game::map::combat::kill(m, g, wolf);
        let unlooted = (why(m, g, SKIN, at(wolf)), skinnable(m, wolf));
        // Its gold taken, the window closed: skinnable.
        loot::show(m, g, wolf);
        loot::take_money(m, g);
        loot::release(m, g, wolf);
        let looted = (why(m, g, SKIN, at(wolf)), skinnable(m, wolf));
        run(m, g, SKIN, at(wolf));
        let skinned = (why(m, g, SKIN, at(wolf)), skinnable(m, wolf));
        crate::game::map::combat::kill(m, g, unit(5));
        let bare = why(m, g, SKIN, at(unit(5)));
        (alive, unlooted, looted, skinned, bare)
    })
    .await;
    assert_eq!(checks.0, Some(TARGET_NOT_DEAD));
    assert_eq!(checks.1, (Some(TARGET_NOT_LOOTED), false));
    assert_eq!(checks.2, (None, true));
    assert_eq!(checks.3, (Some(TARGET_UNSKINNABLE), false), "skinned once");
    assert_eq!(checks.4, Some(TARGET_UNSKINNABLE), "no skin");
    // A wolf of another's isn't a wild one; a respawned one isn't skinnable.
    let other = on_map(&realm, move |m| {
        let w = unit(2);
        crate::game::map::combat::kill(m, g, w);
        m.creature_mut(w).unwrap().owner = Some(g);
        let owned = why(m, g, SKIN, at(w));
        crate::game::map::ai::respawn_now(m, wolf);
        (owned, skinnable(m, wolf))
    })
    .await;
    assert_eq!(other, (Some(BAD_TARGETS), false));
    // Too low to skin a level 20 wolf with 99.
    let low = on_map(&realm, move |m| {
        let w = unit(6);
        m.creature_mut(w)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 20);
        crate::game::map::combat::kill(m, g, w);
        loot::show(m, g, w);
        loot::take_money(m, g);
        loot::release(m, g, w);
        learn_skill(m, g, SKINNING, 99);
        why(m, g, SKIN, at(w))
    })
    .await;
    assert_eq!(low, Some(LOW_CASTLEVEL));
}

/// Thistle Tea's `Skinning.skill/1`: a skill counts its own bonuses, not
/// another skill's.
#[tokio::test]
async fn skinning_counts_its_own_bonuses() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let skill = on_map(&realm, move |m| {
        learn_skill(m, g, SKINNING, 1);
        crate::game::map::spells::cast_triggered(m, g, KNACK, Some(g));
        professions::skill_value(&m.players[&g], SKINNING)
    })
    .await;
    assert_eq!(skill, 6);
}

/// Thistle Tea's `Skinning.gain_chance`: by how far past the need, twice
/// for an elite, halved each 75 points.
#[test]
fn a_skinning_skill_rises_by_its_steps() {
    let chance = |value, level, elite: bool| {
        professions::gather_gain_chance(
            SKINNING,
            value,
            professions::skinning_required(level, value.min(99)),
            if elite { 2 } else { 1 },
        )
    };
    assert_eq!(chance(24, 5, false), 100.0);
    assert_eq!(chance(25, 5, false), 75.0);
    assert_eq!(chance(50, 5, false), 25.0);
    assert_eq!(chance(75, 17, false), 50.0);
    assert_eq!(chance(75, 17, true), 100.0);
    let late =
        |value: u32, level: u32| professions::gather_gain_chance(SKINNING, value, level * 5, 1);
    assert_eq!(late(150, 30), 25.0);
    assert_eq!(late(225, 45), 12.5);
}

/// The loot window `c` is shown: its gold and items.
async fn window(c: &mut Client) -> (u32, Vec<u32>) {
    let b = c
        .wait_frame("SMSG_LOOT_RESPONSE", loot::SMSG_LOOT_RESPONSE, |_| true)
        .await;
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let items = (0..usize::from(b[13]))
        .map(|n| u32_at(14 + n * 22 + 1))
        .collect();
    (u32_at(9), items)
}

fn looting(m: &Map, g: Guid) -> Option<Guid> {
    m.players[&g].looting
}

/// Thistle Tea's `Pickpocket.validate` and `Pockets`: only from stealth, a
/// living creature with pockets; picked once a life, their gold taken once;
/// a later rogue finds only what their quests want.
#[tokio::test]
async fn pockets_are_picked_once_a_life() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Rogue).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Rogue).await;
    a.sees(unit(3)).await;
    let mark = unit(3);
    let checks = on_map(&realm, move |m| {
        let open = why(m, ga, PICK, at(mark));
        crate::game::map::spells::cast_triggered(m, ga, STEALTH, Some(ga));
        let bare = why(m, ga, PICK, at(unit(5)));
        crate::game::map::combat::kill(m, ga, unit(4));
        let dead = why(m, ga, PICK, at(unit(4)));
        (open, bare, dead, why(m, ga, PICK, at(mark)))
    })
    .await;
    assert_eq!(
        checks,
        (
            Some(ONLY_STEALTHED),
            Some(TARGET_NO_POCKETS),
            Some(TARGETS_DEAD),
            None
        )
    );
    a.clear();
    on_map(&realm, move |m| run(m, ga, PICK, at(mark))).await;
    let (gold, items) = window(&mut a).await;
    assert_eq!(gold % 10, 0, "in tens of copper");
    assert_eq!(items, vec![COIN], "no letter for Alice");
    let money = on_map(&realm, move |m| {
        let before = m.players[&ga].money();
        loot::take_money(m, ga);
        loot::take_money(m, ga);
        loot::take_item(m, ga, 0);
        loot::release(m, ga, mark);
        m.players[&ga].money() - before
    })
    .await;
    assert_eq!(money, gold, "taken once");
    // Picked again: nothing more.
    let again = on_map(&realm, move |m| {
        run(m, ga, PICK, at(mark));
        looting(m, ga)
    })
    .await;
    assert_eq!(again, None, "empty pockets stay empty");
    // Bob's quest wants the letter: his pockets have it alone.
    b.clear();
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&gb).unwrap();
        p.journal.add(crate::game::store::Logged {
            quest: 50,
            ..Default::default()
        });
        crate::game::map::spells::cast_triggered(m, gb, STEALTH, Some(gb));
        run(m, gb, PICK, at(mark));
    })
    .await;
    assert_eq!(window(&mut b).await, (0, vec![LETTER]));
}

/// Thistle Tea's `Disenchant`: an enchanter of any skill disenchants high
/// equipment; not a stack, an item missing or another's, one not to be
/// disenchanted; the one item goes, its dust the enchanter's own.
#[tokio::test]
async fn an_item_is_disenchanted_by_any_enchanter() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let item_at = |target: Guid| Targets {
        item: Some(target),
        ..Default::default()
    };
    let checks = on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, BRACER, 2, false).unwrap();
        crate::game::map::inventory::add_item(m, g, LEATHER, 1, false).unwrap();
        let p = &m.players[&g];
        let bracers: Vec<Guid> = p
            .items
            .iter()
            .filter(|i| i.entry == BRACER)
            .map(|i| i.guid)
            .collect();
        let leather = p.items.iter().find(|i| i.entry == LEATHER).unwrap().guid;
        let unskilled = why(m, g, DISENCHANT, item_at(bracers[0]));
        learn_skill(m, g, ENCHANTING, 1);
        (
            unskilled,
            why(m, g, DISENCHANT, item_at(bracers[0])),
            why(m, g, DISENCHANT, item_at(leather)),
            why(m, g, DISENCHANT, item_at(guid::item(999_999))),
            bracers,
        )
    })
    .await;
    assert_eq!(checks.0, Some(LOW_CASTLEVEL), "no enchanter");
    assert_eq!(checks.1, None, "level one does a level 60 bracer");
    assert_eq!(checks.2, Some(CANT_BE_DISENCHANTED));
    assert_eq!(checks.3, Some(CANT_BE_DISENCHANTED), "not hers");
    let bracers = checks.4;
    a.clear();
    let gone = on_map(&realm, move |m| {
        run(m, g, DISENCHANT, item_at(bracers[1]));
        let p = &m.players[&g];
        (
            p.item_by_guid(bracers[0]).is_some(),
            p.item_by_guid(bracers[1]).is_some(),
        )
    })
    .await;
    assert_eq!(gone, (true, false), "that one bracer alone");
    assert_eq!(window(&mut a).await.1, vec![DUST]);
    let dust = on_map(&realm, move |m| {
        loot::take_item(m, g, 0);
        m.players[&g].count_of(DUST)
    })
    .await;
    assert_eq!(dust, 1);
}

const DRAIN: u32 = 900_710;
const CAPTURE: u32 = 900_711;
const SHARD: u32 = 6265;
const CAUGHT: u32 = 9405;

/// Drain Soul's shard, and a capture's item, on the creature's death: an
/// aura of the caster's on it.
fn drain_data() -> GameData {
    let mut d = data();
    for (id, item, family) in [(DRAIN, SHARD, 5), (CAPTURE, CAUGHT, 0)] {
        let mut s = spell(
            id,
            &[(6, crate::game::map::auras::AURA_CHANNEL_DEATH_ITEM, 0, 1)],
        );
        s.effects[0].item_type = u64::from(item);
        s.effects[0].target_a = 25;
        s.family_name = family;
        s.duration_index = 0;
        d.spells.spells.insert(id, s);
    }
    for e in [SHARD, CAUGHT] {
        d.items.insert(e, item(e));
    }
    d
}

fn drain(m: &mut Map, caster: Guid, target: Guid, spell: u32, amount: i32) {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    crate::game::map::auras::apply_spell(m, caster, target, &s, &[(0, amount)], Default::default())
        .unwrap();
}

/// Thistle Tea's `DeathItem`: a shard for a warlock whose tap the
/// creature is, theirs or their group's; not for another's tap, a grey
/// creature, nor a pet; one a warlock however many drains; a capture's
/// item whatever the tap or level; none of a nonpositive amount.
#[tokio::test]
async fn a_soul_shard_is_for_the_tap_and_its_group() {
    use crate::game::map::spells::outcome::death_items;
    let realm = harness::realm_with(drain_data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warlock).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GROUP_INVITE(_)
        )
        .then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while realm.social.groups.lock().group_of(ga).is_none() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let wolf = unit(1);
    let got = on_map(&realm, move |m| {
        let tap = |m: &mut Map, by: Option<Guid>| {
            m.creature_mut(wolf).unwrap().tapped_by = by;
        };
        let shards = |m: &Map| death_items(m, wolf, 1_000_000);
        drain(m, ga, wolf, DRAIN, 1);
        drain(m, ga, wolf, DRAIN, 1);
        tap(m, Some(gb));
        let group = shards(m);
        tap(m, Some(gc));
        let other = shards(m);
        tap(m, Some(ga));
        let own = shards(m);
        m.creature_mut(wolf)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 1);
        let p = m.players.get_mut(&ga).unwrap();
        crate::game::map::stats::set_level(p, &m.realm.data.clone(), 60);
        let grey = shards(m);
        drain(m, ga, wolf, CAPTURE, 2);
        tap(m, Some(gc));
        let caught = shards(m);
        (group, other, own, grey, caught)
    })
    .await;
    assert_eq!(got.0, vec![(ga, SHARD, 1)], "Bob's tap, Alice's group");
    assert_eq!(got.1, vec![], "Cid's tap");
    assert_eq!(got.2, vec![(ga, SHARD, 1)], "one, of two drains");
    assert_eq!(got.3, vec![], "grey");
    assert_eq!(got.4, vec![(ga, CAUGHT, 2)], "the capture whatever");
    let none = on_map(&realm, move |m| {
        let wolf = unit(2);
        drain(m, ga, wolf, DRAIN, 0);
        death_items(m, wolf, 1_000_000)
    })
    .await;
    assert_eq!(none, vec![], "none of nothing");
}

/// Thistle Tea's `OpenLock.resolve/4`: a skill's bonuses count only if it
/// is known; an item cast has its own strength alone and no gain; an item
/// key opens its lock, else the first skill way decides; a key spell wants
/// its key; an ordinary opening wants no skill and gains none.
#[tokio::test]
async fn a_lock_opens_by_its_ways() {
    use crate::game::data::crafts::LockKey;
    use professions::{Opening, resolve_lock};
    const MINING: u32 = 186;
    let mut d = data();
    let key = |kind, index, skill| LockKey { kind, index, skill };
    // A vein (3) wanting 65; a chest (1) wanting 125, a charge's (16) 150;
    // an item key or lockpicking 100; a key alone; an ordinary lock (5).
    d.crafts.locks.insert(39, vec![key(2, 3, 65)]);
    d.crafts.locks.insert(40, vec![key(2, 1, 125)]);
    d.crafts.locks.insert(41, vec![key(2, 16, 150)]);
    d.crafts
        .locks
        .insert(42, vec![key(1, 123, 0), key(2, 1, 100)]);
    d.crafts.locks.insert(83, vec![key(1, 5397, 0)]);
    d.crafts.locks.insert(44, vec![key(2, 5, 100)]);
    d.crafts.locks.insert(45, vec![key(2, 3, 1)]);
    let open = |id: u32, kind: u32, points: i32| {
        spell(
            id,
            &[(professions::EFFECT_OPEN_LOCK, 0, kind as i32, points)],
        )
    };
    let fire = spell(900_740, &[(professions::EFFECT_OPEN_LOCK_ITEM, 0, 0, 0)]);
    for s in [
        spell(900_750, &[(6, 30, MINING as i32, 5)]),
        open(900_723, 3, 0),
        open(900_721, 1, 125),
        open(900_736, 16, 150),
        open(900_731, 1, 0),
        open(900_725, 5, 0),
        fire,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Rogue).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let s = |id: u32| data.spells.get(id).cloned().unwrap();
        let resolve = |m: &Map, spell: u32, lock: u32, item: Option<u32>| {
            resolve_lock(&data, &m.players[&g], &s(spell), 0, lock, item)
        };
        let unknown = resolve(m, 900_723, 45, None);
        learn_skill(m, g, MINING, 60);
        crate::game::map::spells::cast_triggered(m, g, 900_750, Some(g));
        let known = resolve(m, 900_723, 39, None);
        (
            unknown,
            known,
            resolve(m, 900_721, 40, Some(15_870)),
            resolve(m, 900_721, 40, Some(15_870)).map(|o| o.gain),
            resolve(m, 900_736, 41, Some(4367)),
            resolve(m, 900_731, 42, Some(123)),
            resolve(m, 900_731, 42, Some(456)),
            resolve(m, 900_740, 83, Some(5397)),
            resolve(m, 900_740, 83, Some(1234)),
            resolve(m, 900_740, 83, None),
            resolve(m, 900_725, 44, None),
        )
    })
    .await;
    assert_eq!(got.0, Err(LOW_CASTLEVEL), "no bonus for a skill not known");
    assert_eq!(
        got.1,
        Ok(Opening {
            skill: Some(MINING),
            value: 65,
            required: 65,
            gain: true
        }),
        "60 and 5"
    );
    assert_eq!(got.2.map(|o| o.value), Ok(125), "the key's own 125");
    assert_eq!(got.3, Ok(false), "no gain from an item");
    assert_eq!(got.4.map(|o| (o.value, o.gain)), Ok((150, false)));
    assert_eq!(got.5, Ok(Opening::default()), "its item key");
    assert_eq!(got.6, Err(LOW_CASTLEVEL), "then lockpicking, short");
    assert_eq!(got.7, Ok(Opening::default()));
    assert_eq!(got.8, Err(BAD_TARGETS), "another key");
    assert_eq!(got.9, Err(BAD_TARGETS), "no key");
    assert_eq!(
        got.10.map(|o| (o.skill, o.required, o.gain)),
        Ok((None, 0, false)),
        "ordinary"
    );
}
