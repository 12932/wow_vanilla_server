//! Round 12's group loot, as mangos-classic's `Loot` has it: the current
//! looter (`m_currentLooterGuid`) for every method but free for all, the
//! items under the threshold theirs till they let the window go
//! (`isReleased`), round robin's corpse open to the rest for its gold
//! (R4-18), master loot with no master near as group loot, free-for-all
//! drops (`ITEM_FLAG_MULTI_DROP`) for each, loot conditions judged as the
//! loot is rolled (`PlayerOrGroupFulfilsCondition`), a corpse no one may
//! take anything from released for skinning, a rolled item shown to look
//! at, the gold shared by those it fell to, and a quest-starting drop
//! hidden from one who has its quest.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::scripts::Condition;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::loot::{self, DYNFLAG_LOOTABLE};
use crate::game::map::professions::UNIT_FLAG_SKINNABLE;
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::tests_loot::LOOT_ITEM;

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

/// Linen: white, under the threshold.
const LINEN: u32 = 2100;
/// A deed: free for all.
const DEED: u32 = 2101;
/// A note that starts a quest.
const NOTE: u32 = 2102;
const NOTE_QUEST: u32 = 9100;
/// A Horde token: its drop's condition met by no one here.
const TOKEN: u32 = 2103;
/// What its group falls to.
const TUSK: u32 = 2104;
/// A sigil whose condition only Bob meets.
const SIGIL: u32 = 2105;

/// An event that never runs: its condition is met by no one.
const NO_ONE: u32 = 9101;
/// A quest only Bob has done.
const BOBS: u32 = 9102;
const BOBS_QUEST: u32 = 9200;

const GNOLL: u32 = 3010;
const COURIER: u32 = 3011;
const GHOUL: u32 = 3012;
const WOLF: u32 = 3013;
const BANDIT: u32 = 3014;
const SEER: u32 = 3015;
/// The tests' kobold (`tests_loot`): the green blade and 30 copper.
const KOBOLD: u32 = 3001;

fn white(entry: u32, name: &str) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        quality: 1,
        stackable: 20,
        ..Default::default()
    }
}

fn sure(item: u32) -> LootEntry {
    LootEntry {
        item,
        chance: 100.0,
        min: 1,
        max: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = crate::game::tests_loot::data();
    let mut deed = white(DEED, "Westfall Deed");
    deed.flags = loot::ITEM_FLAG_MULTI_DROP;
    let mut note = white(NOTE, "A Torn Note");
    note.start_quest = NOTE_QUEST;
    for t in [
        white(LINEN, "Linen Cloth"),
        deed,
        note,
        white(TOKEN, "Horde Token"),
        white(TUSK, "Tusk"),
        white(SIGIL, "Sigil"),
    ] {
        d.items.insert(t.entry, t);
    }
    for (id, kind, value) in [(NO_ONE, 12, 999), (BOBS, 8, BOBS_QUEST as i32)] {
        d.scripts.conditions.insert(
            id,
            Condition {
                entry: id,
                kind,
                values: [value, 0, 0, 0],
                flags: 0,
            },
        );
    }
    let creature = |entry: u32, name: &str, skin: u32| CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 1,
        level_max: 1,
        faction: 32,
        unit_class: 1,
        health_max: 50,
        loot_id: entry,
        skinning_loot_id: skin,
        ..Default::default()
    };
    for (entry, name, skin) in [
        (GNOLL, "Gnoll", 0),
        (COURIER, "Courier", 0),
        (GHOUL, "Ghoul", 0),
        (WOLF, "Wolf", 3002),
        (BANDIT, "Bandit", 0),
        (SEER, "Seer", 0),
    ] {
        d.creature_templates
            .insert(entry, creature(entry, name, skin));
    }
    let token = LootEntry {
        condition: NO_ONE,
        ..sure(TOKEN)
    };
    d.loot.creature.insert(GNOLL, vec![sure(LINEN)]);
    d.loot.creature.insert(COURIER, vec![sure(DEED)]);
    // The token's group: its sure chance is no one's, so it falls to the
    // tusk.
    d.loot.creature.insert(
        GHOUL,
        vec![
            LootEntry { group: 1, ..token },
            LootEntry {
                chance: 0.0,
                group: 1,
                ..sure(TUSK)
            },
        ],
    );
    d.loot.creature.insert(WOLF, vec![token]);
    d.loot.creature.insert(BANDIT, vec![sure(NOTE)]);
    d.loot.creature.insert(
        SEER,
        vec![
            LootEntry {
                condition: BOBS,
                ..sure(SIGIL)
            },
            sure(LINEN),
        ],
    );
    let spawns = d.creature_spawns.entry(0).or_default();
    for (n, entry) in [
        (31, GNOLL),
        (32, COURIER),
        (33, GHOUL),
        (34, WOLF),
        (35, BANDIT),
        (36, SEER),
        (37, KOBOLD),
        (38, KOBOLD),
        (39, KOBOLD),
    ] {
        spawns.push(CreatureSpawn {
            guid: n,
            entries: vec![entry],
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + (n - 30) as f32 * 0.3,
            z: START.2,
            respawn_min: 300,
            respawn_max: 300,
            ..Default::default()
        });
    }
    d
}

fn unit(entry: u32, n: u32) -> Guid {
    guid::unit(entry, n)
}

/// Alice, Bob and Cid at the start, in Alice's group under `method`, Alice
/// the master looter.
async fn group(method: GroupLootSetting) -> (std::sync::Arc<crate::game::Realm>, [Client; 3]) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Mage).await;
    for (name, who) in [("Bob", &mut b), ("Cid", &mut c)] {
        a.send(CMSG_GROUP_INVITE { name: name.into() }).await;
        who.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        who.send(CMSG_GROUP_ACCEPT {}).await;
        who.wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
    }
    let master = a.guid;
    a.send(CMSG_LOOT_METHOD {
        loot_setting: method,
        loot_master: master,
        loot_threshold: ItemQuality::Uncommon,
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while realm
        .social
        .groups
        .lock()
        .group_of(master)
        .map(|g| g.loot_method)
        != Some(method)
    {
        assert!(tokio::time::Instant::now() < deadline, "the loot method");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (realm, [a, b, c])
}

fn map_of(realm: &std::sync::Arc<crate::game::Realm>) -> MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

async fn kill(map: &MapHandle, by: Guid, creature: Guid) {
    on(map, move |m| {
        crate::game::map::combat::kill(m, by, creature)
    })
    .await;
}

fn count(m: &Map, g: Guid, entry: u32) -> u32 {
    m.players.get(&g).map_or(0, |p| p.count_of(entry))
}

/// `c` opens `corpse`: the window's items and their slot types, or `None`
/// if it shuts on them.
async fn open(c: &mut Client, corpse: Guid) -> Option<Vec<(u32, u8)>> {
    c.clear();
    c.send(CMSG_LOOT { guid: corpse }).await;
    c.wait_for("the loot window or its shutting", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RESPONSE(r) if r.guid == corpse => Some(Some(
            r.items.iter().map(|i| (i.item, i.ty.as_int())).collect(),
        )),
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) if r.guid == corpse => Some(None),
        _ => None,
    })
    .await
}

/// `c` takes slot `slot` from their window, and has it when `entry` is
/// in their bags, waiting a moment for it.
async fn take(c: &mut Client, map: &MapHandle, slot: u8, entry: u32) -> u32 {
    let g = c.guid;
    c.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: slot }).await;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        let n = on(map, move |m| count(m, g, entry)).await;
        if n > 0 || tokio::time::Instant::now() > deadline {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn release(c: &mut Client, map: &MapHandle, corpse: Guid) {
    let g = c.guid;
    c.send(CMSG_LOOT_RELEASE { guid: corpse }).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(map, move |m| m.players.get(&g).and_then(|p| p.looting)).await == Some(corpse) {
        assert!(tokio::time::Instant::now() < deadline, "the window shuts");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Group loot: the gnoll's linen, under the threshold, is Alice's, the
/// current looter's, alone; the window shuts on Bob, who has nothing in
/// it. Once Alice lets it go, Bob takes it (mangos's `Loot::Release`).
#[tokio::test]
async fn the_items_under_the_threshold_are_the_current_looters_till_they_let_go() {
    let (realm, [mut a, mut b, _c]) = group(GroupLootSetting::GroupLoot).await;
    let ga = a.guid;
    let map = map_of(&realm);
    let gnoll = unit(GNOLL, 31);
    kill(&map, ga, gnoll).await;
    let looter = on(&map, move |m| {
        loot::loot_ref(m, gnoll).and_then(|l| l.assigned)
    })
    .await;
    assert_eq!(
        looter,
        Some(ga),
        "the first kill's looter is the first member"
    );
    assert_eq!(open(&mut b, gnoll).await, None, "nothing of Bob's yet");
    assert_eq!(take(&mut b, &map, 0, LINEN).await, 0);
    assert_eq!(open(&mut a, gnoll).await, Some(vec![(LINEN, 0)]));
    release(&mut a, &map, gnoll).await;
    assert_eq!(open(&mut b, gnoll).await, Some(vec![(LINEN, 0)]), "let go");
    assert_eq!(take(&mut b, &map, 0, LINEN).await, 1);
    assert_eq!(on(&map, move |m| count(m, ga, LINEN)).await, 0);
}

/// Round robin (R4-18): the kobold is Alice's turn, but Bob, one it fell
/// to, opens it for the gold, shared three ways (mangos's `CanLoot`); its
/// blade stays Alice's.
#[tokio::test]
async fn round_robin_lets_the_rest_take_the_gold() {
    let (realm, [a, mut b, _c]) = group(GroupLootSetting::RoundRobin).await;
    let (ga, gb) = (a.guid, b.guid);
    let map = map_of(&realm);
    let kobold = unit(KOBOLD, 37);
    kill(&map, ga, kobold).await;
    assert_eq!(open(&mut b, kobold).await, Some(vec![]), "the gold alone");
    assert_eq!(take(&mut b, &map, 0, LOOT_ITEM).await, 0, "Alice's blade");
    b.send(CMSG_LOOT_MONEY {}).await;
    b.wait_for("SMSG_LOOT_MONEY_NOTIFY", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_MONEY_NOTIFY(n) => Some(n.amount),
        _ => None,
    })
    .await;
    let money = on(&map, move |m| {
        [ga, gb].map(|g| m.players.get(&g).map_or(0, |p| p.money()))
    })
    .await;
    assert_eq!(money, [10, 10]);
}

/// Master loot with the master away is group loot (mangos's
/// `SetGroupLootRight`): the blade is rolled for.
#[tokio::test]
async fn master_loot_with_the_master_away_is_group_loot() {
    let (realm, [a, b, _c]) = group(GroupLootSetting::MasterLoot).await;
    let (ga, bob) = (a.guid, b.guid);
    let map = map_of(&realm);
    // Alice, the master, kills from afar: her pet's, say.
    let kobold = unit(KOBOLD, 38);
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().movement.position.x += 300.0;
    })
    .await;
    on(&map, move |m| {
        crate::game::map::combat::kill(m, bob, kobold)
    })
    .await;
    let method = on(&map, move |m| {
        loot::loot_ref(m, kobold).map(|l| (l.method, l.master, l.rolls.len()))
    })
    .await;
    assert_eq!(method, Some((Some(GroupLootSetting::GroupLoot), None, 1)));
}

/// A free-for-all drop, the Westfall Deed, drops for each it fell to:
/// all three take one (mangos's `freeForAll`).
#[tokio::test]
async fn a_free_for_all_drop_is_each_ones() {
    let (realm, [mut a, mut b, mut c]) = group(GroupLootSetting::GroupLoot).await;
    let map = map_of(&realm);
    let courier = unit(COURIER, 32);
    kill(&map, a.guid, courier).await;
    for who in [&mut a, &mut b, &mut c] {
        assert_eq!(
            open(who, courier).await,
            Some(vec![(DEED, 0)]),
            "{:?}",
            who.guid
        );
        assert_eq!(take(who, &map, 0, DEED).await, 1, "{:?}", who.guid);
    }
    // Taken by all, the corpse is looted.
    for who in [&mut a, &mut b, &mut c] {
        release(who, &map, courier).await;
    }
    let lootable = on(&map, move |m| {
        m.creature(courier)
            .unwrap()
            .fields
            .get_u32(UNIT_DYNAMIC_FLAGS)
            & DYNFLAG_LOOTABLE
            != 0
    })
    .await;
    assert!(!lootable);
}

/// The ghoul's group: its sure token has a condition no one here meets,
/// so the group falls to the tusk (mangos's `LootGroup::Roll`).
#[tokio::test]
async fn a_drop_no_one_may_have_falls_to_another_of_its_group() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = map_of(&realm);
    let ghoul = unit(GHOUL, 33);
    kill(&map, ga, ghoul).await;
    assert_eq!(open(&mut a, ghoul).await, Some(vec![(TUSK, 0)]));
}

/// The wolf drops only a token no one may have: nothing drops, and the
/// corpse may be skinned at once (mangos's `IsLootedForAll`).
#[tokio::test]
async fn a_corpse_with_nothing_anyone_may_take_may_be_skinned() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = map_of(&realm);
    let wolf = unit(WOLF, 34);
    kill(&map, ga, wolf).await;
    let state = on(&map, move |m| {
        let c = m.creature(wolf).unwrap();
        (
            c.loot.as_ref().map(|l| l.items.len()),
            c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & DYNFLAG_LOOTABLE != 0,
            c.fields.get_u32(UNIT_FIELD_FLAGS) & UNIT_FLAG_SKINNABLE != 0,
        )
    })
    .await;
    assert_eq!(state, (Some(0), false, true));
}

/// The seer's sigil only Bob may have: Alice, the current looter, doesn't
/// see it, so it's Bob's at once, without her letting it go (mangos's
/// `currentLooterPass`); the linen stays hers.
#[tokio::test]
async fn what_the_current_looter_may_not_have_is_the_others_at_once() {
    let (realm, [mut a, mut b, _c]) = group(GroupLootSetting::GroupLoot).await;
    let (ga, gb) = (a.guid, b.guid);
    let map = map_of(&realm);
    on(&map, move |m| {
        m.players
            .get_mut(&gb)
            .unwrap()
            .journal
            .rewarded
            .insert(BOBS_QUEST);
    })
    .await;
    let seer = unit(SEER, 36);
    kill(&map, ga, seer).await;
    assert_eq!(open(&mut a, seer).await, Some(vec![(LINEN, 0)]));
    let bobs = open(&mut b, seer).await.expect("Bob's window");
    assert_eq!(bobs, vec![(SIGIL, 0)]);
    let slot = on(&map, move |m| {
        loot::loot_ref(m, seer)
            .unwrap()
            .items
            .iter()
            .position(|s| s.item == SIGIL)
            .unwrap() as u8
    })
    .await;
    assert_eq!(take(&mut b, &map, slot, SIGIL).await, 1);
}

/// The bandit's note starts a quest Alice has taken: it isn't hers to
/// loot (mangos's `AllowedForPlayer`), so the corpse has nothing for her.
#[tokio::test]
async fn a_quest_starting_drop_hides_from_one_on_its_quest() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = map_of(&realm);
    on(&map, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .journal
            .add(crate::game::store::Logged {
                quest: NOTE_QUEST,
                ..Default::default()
            });
    })
    .await;
    let bandit = unit(BANDIT, 35);
    kill(&map, ga, bandit).await;
    assert_eq!(open(&mut a, bandit).await, None);
    assert_eq!(take(&mut a, &map, 0, NOTE).await, 0);
    let lootable = on(&map, move |m| {
        m.creature(bandit)
            .unwrap()
            .fields
            .get_u32(UNIT_DYNAMIC_FLAGS)
            & DYNFLAG_LOOTABLE
            != 0
    })
    .await;
    assert!(!lootable);
}

/// Group loot: while the blade is rolled for, it shows in the window to
/// look at (slot type 1, mangos's `LOOT_SLOT_VIEW`); the gold is shared
/// by those it fell to, Bob too though he has gone off since (mangos's
/// `SendGold`, by `m_ownerSet`).
#[tokio::test]
async fn a_rolled_item_shows_and_the_gold_is_theirs_it_fell_to() {
    let (realm, [mut a, b, c]) = group(GroupLootSetting::GroupLoot).await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    let map = map_of(&realm);
    let kobold = unit(KOBOLD, 39);
    kill(&map, ga, kobold).await;
    on(&map, move |m| {
        m.players.get_mut(&gb).unwrap().movement.position.x += 200.0;
    })
    .await;
    assert_eq!(open(&mut a, kobold).await, Some(vec![(LOOT_ITEM, 1)]));
    a.send(CMSG_LOOT_MONEY {}).await;
    a.wait_for("SMSG_LOOT_CLEAR_MONEY", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_CLEAR_MONEY).then_some(())
    })
    .await;
    let money = on(&map, move |m| {
        [ga, gb, gc].map(|g| m.players.get(&g).map_or(0, |p| p.money()))
    })
    .await;
    assert_eq!(money, [10, 10, 10]);
}

/// A group's equally chanced pick passes over an item already in the loot
/// half the time (mangos's `IsItemAlreadyIn`): with the first sure, the
/// second of two is picked three times in four, not half.
#[test]
fn a_group_picks_again_half_the_time_over_a_duplicate() {
    use rand::SeedableRng;
    let d = data();
    let table = vec![
        sure(LINEN),
        LootEntry {
            chance: 0.0,
            group: 1,
            ..sure(LINEN)
        },
        LootEntry {
            chance: 0.0,
            group: 1,
            ..sure(TUSK)
        },
    ];
    let mut rng = rand::rngs::StdRng::seed_from_u64(12);
    let mut tusks = 0;
    for _ in 0..2000 {
        let mut out = Vec::new();
        loot::roll(&d, &table, &[], &mut rng, &mut out, 0);
        tusks += out.iter().filter(|s| s.item == TUSK).count();
    }
    assert!((1350..1650).contains(&tusks), "{tusks} of 2000");
}

/// The tests' boar (`tests_loot`): a skin, and nothing else.
const BOAR: u32 = 3002;
const SKINNING: u32 = 8613;
const LEATHER: u32 = 2318;

/// The corpse's time left to lie, if it's a corpse.
fn corpse_left(m: &Map, creature: Guid) -> Option<Duration> {
    match m.creature(creature)?.life {
        crate::game::map::creature::Life::Dead { corpse_until, .. } => {
            Some(corpse_until.saturating_duration_since(m.now))
        }
        crate::game::map::creature::Life::Alive => None,
    }
}

/// Looted, a corpse with no skin goes within two minutes, not its whole
/// decay (mangos's `ReduceCorpseDecayTimer`).
#[tokio::test]
async fn a_looted_corpse_goes_sooner() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = map_of(&realm);
    let gnoll = unit(GNOLL, 31);
    kill(&map, ga, gnoll).await;
    let left = on(&map, move |m| corpse_left(m, gnoll)).await.unwrap();
    assert!(left > loot::LOOTING_TIME, "its decay: {left:?}");
    assert_eq!(open(&mut a, gnoll).await, Some(vec![(LINEN, 0)]));
    assert_eq!(take(&mut a, &map, 0, LINEN).await, 1);
    release(&mut a, &map, gnoll).await;
    let left = on(&map, move |m| corpse_left(m, gnoll)).await.unwrap();
    assert!(left <= loot::LOOTING_TIME, "looted: {left:?}");
}

/// Alice skins the boar and leaves the leather: it's her group's, free
/// for all (mangos's `SKIN_AVAILABLE`); Bob takes it, and the skinned
/// corpse goes at once (`SKINNED`).
#[tokio::test]
async fn a_skin_left_is_the_groups_and_a_skinned_corpse_goes() {
    let (realm, [mut a, mut b, _c]) = group(GroupLootSetting::GroupLoot).await;
    let ga = a.guid;
    let map = map_of(&realm);
    let boar = unit(BOAR, 24);
    on(&map, move |m| {
        crate::game::tests_loot::learn(m, ga, SKINNING, 393);
        let _ = crate::game::map::inventory::add_item(m, ga, 7005, 1, false);
        crate::game::map::combat::kill(m, ga, boar);
    })
    .await;
    let mut skinned = false;
    for _ in 0..40 {
        skinned = on(&map, move |m| {
            if let Some(c) = m.caster_mut(ga) {
                c.gcd_until = None;
                c.cooldowns.clear();
            }
            crate::game::map::spells::cast(
                m,
                ga,
                SKINNING,
                crate::game::map::spells::Targets {
                    unit: Some(boar),
                    ..Default::default()
                },
                None,
            );
            m.creature(boar)
                .and_then(|c| c.loot.as_ref())
                .is_some_and(|l| l.kind == loot::LootKind::Skinning)
        })
        .await;
        if skinned {
            break;
        }
    }
    assert!(skinned, "skinned");
    release(&mut a, &map, boar).await;
    assert_eq!(
        open(&mut b, boar).await,
        Some(vec![(LEATHER, 0)]),
        "the group's"
    );
    assert_eq!(take(&mut b, &map, 0, LEATHER).await, 1);
    release(&mut b, &map, boar).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while !on(&map, move |m| m.creature(boar).is_none_or(|c| c.despawned)).await {
        assert!(tokio::time::Instant::now() < deadline, "the corpse goes");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The master looter gives the blade to Bob, who has gone 100 yards off
/// since the kill: he is of the group and on the map, which is all
/// mangos's `HandleLootMasterGiveOpcode` asks.
#[tokio::test]
async fn the_master_gives_to_one_of_the_group_anywhere_on_the_map() {
    let (realm, [mut a, mut b, _c]) = group(GroupLootSetting::MasterLoot).await;
    let (ga, gb) = (a.guid, b.guid);
    let map = map_of(&realm);
    let kobold = unit(KOBOLD, 37);
    kill(&map, ga, kobold).await;
    on(&map, move |m| {
        m.players.get_mut(&gb).unwrap().movement.position.x += 100.0;
    })
    .await;
    assert_eq!(open(&mut a, kobold).await, Some(vec![(LOOT_ITEM, 2)]));
    a.send(CMSG_LOOT_MASTER_GIVE {
        loot: kobold,
        slot_id: 0,
        player: gb,
    })
    .await;
    b.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == LOOT_ITEM => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(on(&map, move |m| count(m, gb, LOOT_ITEM)).await, 1);
}
