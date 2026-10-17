//! Phase 3's loot, Thistle Tea's loot tests ported: rolls and their votes,
//! who may open a corpse and roll for what is on it, the master looter's
//! kept items, and what doesn't fit staying in the window.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::Condition;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::loot;
use crate::game::map::{Map, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const KOBOLD: u32 = 3200;
/// A green blade; one for warriors; one at level 10; one with a
/// condition no one meets.
const BLADE: u32 = 9200;
const AXE: u32 = 9201;
const RING: u32 = 9202;
const IDOL: u32 = 9203;
const FILLER: u32 = 9204;
/// For paladins alone.
const MACE: u32 = 9205;
const FEAR: u32 = 900_500;
const LEVEL_TEN: u32 = 1;
const LEVEL_FIFTY: u32 = 2;

fn item(entry: u32, quality: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        quality,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        buy_count: 1,
        ..Default::default()
    }
}

/// Kobolds 21 to 30 in front of the start: those from 21 to 25 and 30 drop
/// the blade; 26 and 29 the axe, the blade and the ring; 27 the idol; 28
/// the mace.
fn data() -> GameData {
    let mut d = GameData::empty();
    for e in [BLADE, RING, IDOL] {
        d.items.insert(e, item(e, 2));
    }
    let mut axe = item(AXE, 2);
    axe.allowable_class = 1;
    d.items.insert(AXE, axe);
    d.items.insert(FILLER, item(FILLER, 0));
    let mut mace = item(MACE, 2);
    mace.allowable_class = 2;
    d.items.insert(MACE, mace);
    let entry = |item, condition| LootEntry {
        item,
        chance: 100.0,
        min: 1,
        max: 1,
        condition,
        ..Default::default()
    };
    d.loot.creature.insert(1, vec![entry(BLADE, 0)]);
    d.loot.creature.insert(
        2,
        vec![entry(AXE, 0), entry(BLADE, 0), entry(RING, LEVEL_TEN)],
    );
    d.loot.creature.insert(3, vec![entry(IDOL, LEVEL_FIFTY)]);
    d.loot.creature.insert(4, vec![entry(MACE, 0)]);
    for (n, table) in [(0, 1), (1, 2), (2, 3), (3, 4)] {
        let e = KOBOLD + n;
        d.creature_templates.insert(
            e,
            CreatureTemplate {
                entry: e,
                name: "Kobold".into(),
                display_ids: [903, 0, 0, 0],
                display_probabilities: [1, 0, 0, 0],
                display_scales: [1.0, 0.0, 0.0, 0.0],
                level_min: 1,
                level_max: 1,
                faction: 32,
                unit_class: 1,
                health_max: 200,
                loot_id: table,
                gold_min: 30,
                gold_max: 30,
                ..Default::default()
            },
        );
    }
    let mut spawns = Vec::new();
    for n in 21..=30u32 {
        let e = guid::entry(kobold(n));
        spawns.push(CreatureSpawn {
            guid: n,
            entries: vec![e],
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + (n as f32 - 25.0) * 0.3,
            z: START.2,
            ..Default::default()
        });
    }
    d.creature_spawns.insert(0, spawns);
    // A fear any unit may cast on any other.
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut fear = crate::game::data::spells::SpellTemplate {
        id: FEAR,
        name: "Fear".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    fear.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_MOD_FEAR,
        target_a: 25,
        ..Default::default()
    };
    d.spells.spells.insert(FEAR, fear);
    for (entry, level) in [(LEVEL_TEN, 10), (LEVEL_FIFTY, 50)] {
        d.scripts.conditions.insert(
            entry,
            Condition {
                entry,
                kind: 15,
                values: [level, 1, 0, 0],
                flags: 0,
            },
        );
    }
    d
}

fn kobold(n: u32) -> Guid {
    let e = match n {
        26 | 29 => KOBOLD + 1,
        27 => KOBOLD + 2,
        28 => KOBOLD + 3,
        _ => KOBOLD,
    };
    guid::unit(e, n)
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

/// Alice and Bob (warriors) and Cid (a mage) at the start, in Alice's
/// group with `method`.
async fn group(realm: &Arc<Realm>, method: GroupLootSetting) -> [Client; 3] {
    let mut a = Client::connect(realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(realm, "C");
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
    a.wait_for("the loot method", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => l
            .group_not_empty
            .as_ref()
            .filter(|g| g.loot_setting == method)
            .map(|_| ()),
        _ => None,
    })
    .await;
    for who in [&mut a, &mut b, &mut c] {
        who.sees(kobold(26)).await;
    }
    [a, b, c]
}

fn count(m: &Map, g: Guid, entry: u32) -> u32 {
    m.players.get(&g).map_or(0, |p| p.count_of(entry))
}

async fn kill(realm: &Arc<Realm>, killer: Guid, n: u32) {
    let k = kobold(n);
    on_map(realm, move |m| crate::game::map::combat::kill(m, killer, k)).await;
}

/// The votes echoed to `c` for the roll on `object`'s first slot: who, and
/// the kind (0 need, 2 greed, 128 pass).
fn echoes(c: &mut Client) -> Vec<(Guid, u8)> {
    let mut out = Vec::new();
    for m in std::mem::take(&mut c.backlog) {
        if let ServerOpcodeMessage::SMSG_LOOT_ROLL(r) = &m {
            if r.roll_number == 0 || r.roll_number == 128 {
                out.push((r.player, r.vote.as_int()));
            }
        } else {
            c.backlog.push(m);
        }
    }
    out
}

/// Wait until `who`'s earlier packets have been handled.
async fn handled(who: &mut Client) {
    who.send(CMSG_NAME_QUERY { guid: who.guid }).await;
    who.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's `LootRoll` `vote/3` and `resolve/2`: one vote each, from
/// those the roll is for; greed wins when nobody needs; nobody rolling,
/// or nobody voting by the end, is all passed, and the item is free.
#[tokio::test]
async fn a_roll_takes_one_vote_each_from_its_rollers() {
    let realm = harness::realm_with(data());
    let [mut a, mut b, mut c] = group(&realm, GroupLootSetting::GroupLoot).await;
    let (ga, gb) = (a.guid, b.guid);
    let mut d = Client::connect(&realm, "D");
    d.enter("Dan", Race::Human, Class::Warrior).await;
    kill(&realm, ga, 21).await;
    let corpse = kobold(21);
    for who in [&mut a, &mut b, &mut c] {
        who.wait_for("SMSG_LOOT_START_ROLL", |m| match m {
            ServerOpcodeMessage::SMSG_LOOT_START_ROLL(r) if r.item == BLADE => Some(()),
            _ => None,
        })
        .await;
    }
    let vote = |vote| CMSG_LOOT_ROLL {
        item: corpse,
        item_slot: 0,
        vote,
    };
    // Bob greeds, then needs; Dan, outside the group, needs.
    b.send(vote(RollVote::Greed)).await;
    b.send(vote(RollVote::Need)).await;
    d.send(vote(RollVote::Need)).await;
    handled(&mut b).await;
    handled(&mut d).await;
    a.send(vote(RollVote::Pass)).await;
    c.send(vote(RollVote::Pass)).await;
    let won = c
        .wait_for("SMSG_LOOT_ROLL_WON", |m| match m {
            ServerOpcodeMessage::SMSG_LOOT_ROLL_WON(w) => Some((w.winning_player, w.vote.as_int())),
            _ => None,
        })
        .await;
    assert_eq!(won, (gb, 2), "Bob, on greed");
    let seen = echoes(&mut c);
    assert_eq!(seen.iter().filter(|(g, _)| *g == gb).count(), 1, "one vote");
    assert!(!seen.iter().any(|(g, _)| *g == d.guid), "Dan's isn't");
    // Everyone passes: all passed, and the blade is free on a corpse still
    // lootable; till then, its gold taken, the roll keeps it lootable.
    kill(&realm, ga, 22).await;
    let corpse = kobold(22);
    for who in [&mut a, &mut b, &mut c] {
        who.wait_for("SMSG_LOOT_START_ROLL", |m| match m {
            ServerOpcodeMessage::SMSG_LOOT_START_ROLL(r) if r.item == BLADE => Some(()),
            _ => None,
        })
        .await;
    }
    assert!(opened(&mut a, corpse).await);
    a.send(CMSG_LOOT_MONEY {}).await;
    a.send(CMSG_LOOT_RELEASE { guid: corpse }).await;
    handled(&mut a).await;
    let pending = on_map(&realm, move |m| {
        let l = loot::loot_ref(m, corpse).unwrap();
        let lootable = m
            .creature(corpse)
            .unwrap()
            .fields
            .get_u32(UNIT_DYNAMIC_FLAGS)
            & loot::DYNFLAG_LOOTABLE;
        (l.gold, l.rolls.len(), lootable)
    })
    .await;
    assert_eq!(pending, (0, 1, loot::DYNFLAG_LOOTABLE));
    for who in [&mut a, &mut b, &mut c] {
        who.send(CMSG_LOOT_ROLL {
            item: corpse,
            item_slot: 0,
            vote: RollVote::Pass,
        })
        .await;
    }
    a.wait_for("SMSG_LOOT_ALL_PASSED", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_ALL_PASSED(p) => (p.looted_target == corpse).then_some(()),
        _ => None,
    })
    .await;
    let free = on_map(&realm, move |m| {
        let l = loot::loot_ref(m, corpse).unwrap();
        let lootable = m
            .creature(corpse)
            .unwrap()
            .fields
            .get_u32(UNIT_DYNAMIC_FLAGS)
            & loot::DYNFLAG_LOOTABLE;
        (l.items[0].blocked, l.rolls.len(), lootable)
    })
    .await;
    assert_eq!(free, (false, 0, loot::DYNFLAG_LOOTABLE));
    // Nobody votes before the end: all passed.
    kill(&realm, ga, 23).await;
    let corpse = kobold(23);
    a.wait_for("SMSG_LOOT_START_ROLL", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_START_ROLL(r) if r.item == BLADE => Some(()),
        _ => None,
    })
    .await;
    on_map(&realm, move |m| {
        let now = m.now;
        for r in loot::loot_mut(m, corpse).unwrap().rolls.iter_mut() {
            r.ends = now;
        }
        loot::update(m);
    })
    .await;
    a.wait_for("SMSG_LOOT_ALL_PASSED", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_ALL_PASSED(p) => (p.looted_target == corpse).then_some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's `resolve/2`: the highest roll wins, and of a tie the
/// first in guid order.
#[test]
fn the_highest_roll_wins_and_a_tie_goes_by_guid() {
    let (one, two, three) = (guid::player(1), guid::player(2), guid::player(3));
    let (rolled, won) = loot::pick_winner(vec![(two, 42), (one, 42)]).unwrap();
    assert_eq!(rolled, vec![(one, 42), (two, 42)]);
    assert_eq!(won, (one, 42), "a tie, to the first guid");
    let (_, won) = loot::pick_winner(vec![(one, 10), (three, 90), (two, 55)]).unwrap();
    assert_eq!(won, (three, 90));
    assert!(loot::pick_winner(Vec::new()).is_none());
}

/// Whether `CMSG_LOOT` on `corpse` opens a window for `c`.
async fn opened(c: &mut Client, corpse: Guid) -> bool {
    c.clear();
    c.raw.clear();
    c.send(CMSG_LOOT { guid: corpse }).await;
    handled(c).await;
    c.backlog
        .iter()
        .any(|m| matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)))
        || c.raw
            .iter()
            .any(|f| u16::from_le_bytes([f[2], f[3]]) == loot::SMSG_LOOT_RESPONSE)
}

/// Thistle Tea's `LootSession.view/2`: an untapped corpse is anyone's, a
/// tapped one its tapper's; not from afar, nor by the dead, the feared or
/// the confused (their window shuts).
#[tokio::test]
async fn a_corpse_is_its_tappers_to_loot_at_hand() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(kobold(24)).await;
    kill(&realm, ga, 24).await;
    kill(&realm, ga, 25).await;
    let may = on_map(&realm, move |m| {
        let tapped = loot::loot_ref(m, kobold(24)).unwrap();
        let (mine, theirs) = (tapped.may_loot(ga), tapped.may_loot(gb));
        // 25 as if no one tapped it.
        loot::loot_mut(m, kobold(25)).unwrap().owner = None;
        let free = loot::loot_ref(m, kobold(25)).unwrap();
        (mine, theirs, free.may_loot(gb))
    })
    .await;
    assert_eq!(may, (true, false, true));
    assert!(!opened(&mut b, kobold(24)).await, "Alice's");
    assert!(opened(&mut b, kobold(25)).await, "anyone's");
    // Ten yards off, Alice can't.
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position.x += 10.0;
    })
    .await;
    assert!(!opened(&mut a, kobold(24)).await, "out of reach");
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position.x -= 10.0;
    })
    .await;
    assert!(opened(&mut a, kobold(24)).await, "at hand");
    // The dead and the feared are told the window shuts.
    for state in 0..2 {
        on_map(&realm, move |m| {
            if state == 0 {
                m.players.get_mut(&ga).unwrap().death = crate::game::map::death::Death::Dead {
                    since: std::time::Instant::now(),
                };
            } else {
                m.players.get_mut(&ga).unwrap().death = crate::game::map::death::Death::Alive;
                crate::game::map::spells::cast_triggered(m, kobold(30), FEAR, Some(ga));
                assert!(
                    m.players[&ga]
                        .auras
                        .has(crate::game::map::auras::AURA_MOD_FEAR)
                );
            }
        })
        .await;
        a.clear();
        a.send(CMSG_LOOT { guid: kobold(24) }).await;
        a.wait_for("SMSG_LOOT_RELEASE_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) => {
                (r.guid == kobold(24)).then_some(())
            }
            _ => None,
        })
        .await;
    }
}

/// Fill `g`'s bags.
async fn fill(realm: &Arc<Realm>, g: Guid) {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        let free = p
            .carry_slots(&d)
            .iter()
            .filter(|(b, s)| p.item_at(*b, *s).is_none())
            .count() as u32;
        crate::game::map::inventory::add_item(m, g, FILLER, free, false).unwrap();
    })
    .await;
}

/// Free one of `g`'s slots.
async fn unfill(realm: &Arc<Realm>, g: Guid) {
    on_map(realm, move |m| {
        let i = m.players[&g]
            .items
            .iter()
            .position(|i| i.entry == FILLER)
            .unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
    })
    .await;
}

fn set_level(m: &mut Map, g: Guid, level: u8) {
    let d = m.realm.data.clone();
    crate::game::map::stats::set_level(m.players.get_mut(&g).unwrap(), &d, level);
}

/// Who each roll on `corpse` is for, by item.
fn rollers(m: &Map, corpse: Guid) -> Vec<(u32, Vec<Guid>)> {
    let l = loot::loot_ref(m, corpse).unwrap();
    l.rolls
        .iter()
        .map(|r| (l.items[r.slot].item, r.eligible.clone()))
        .collect()
}

/// Thistle Tea's `start_rolls`: need before greed rolls an item among
/// those who could use it, group loot among all; only those who meet an
/// item's condition roll for it, and one alone has it at once; an item
/// nobody may roll for is left for the taking.
#[tokio::test]
async fn a_roll_is_for_those_who_may_have_the_item() {
    let realm = harness::realm_with(data());
    let [a, b, mut c] = group(&realm, GroupLootSetting::NeedBeforeGreed).await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    on_map(&realm, move |m| set_level(m, gc, 10)).await;
    kill(&realm, ga, 26).await;
    let rolls = on_map(&realm, move |m| rollers(m, kobold(26))).await;
    assert_eq!(
        rolls,
        vec![(AXE, vec![ga, gb]), (BLADE, vec![ga, gb, gc])],
        "the axe for the warriors"
    );
    c.wait_for("the ring, Cid's at once", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) => (r.item == RING).then_some(()),
        _ => None,
    })
    .await;
    // The mace is for paladins: nobody rolls, it's there to take.
    kill(&realm, ga, 28).await;
    let mace = on_map(&realm, move |m| {
        let l = loot::loot_ref(m, kobold(28)).unwrap();
        (l.rolls.len(), l.items[0].blocked)
    })
    .await;
    assert_eq!(mace, (0, false));
    // Group loot: the axe for all.
    a.send(CMSG_LOOT_METHOD {
        loot_setting: GroupLootSetting::GroupLoot,
        loot_master: Guid::zero(),
        loot_threshold: ItemQuality::Uncommon,
    })
    .await;
    let realm2 = realm.clone();
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let method = realm2
            .social
            .groups
            .lock()
            .group_of(ga)
            .map(|g| g.loot_method);
        if method == Some(GroupLootSetting::GroupLoot) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    kill(&realm, ga, 29).await;
    let rolls = on_map(&realm, move |m| rollers(m, kobold(29))).await;
    assert_eq!(rolls[0], (AXE, vec![ga, gb, gc]));
}

/// Thistle Tea's master reservations: the master gives an item only to
/// one who meets its condition, and what doesn't fit stays the master's.
#[tokio::test]
async fn the_master_keeps_what_does_not_fit() {
    let realm = harness::realm_with(data());
    let [mut a, mut b, mut c] = group(&realm, GroupLootSetting::MasterLoot).await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    kill(&realm, ga, 26).await;
    let corpse = kobold(26);
    assert!(opened(&mut a, corpse).await);
    let give = |to| CMSG_LOOT_MASTER_GIVE {
        loot: corpse,
        slot_id: 2,
        player: to,
    };
    let ring = move |m: &mut Map| {
        let l = loot::loot_ref(m, corpse).unwrap();
        (
            l.items[2].item,
            l.items[2].blocked,
            l.items[2].taken,
            count(m, gc, RING),
        )
    };
    // Cid is level 1, the ring wants 10.
    a.send(give(gc)).await;
    handled(&mut a).await;
    assert_eq!(on_map(&realm, ring).await, (RING, true, false, 0));
    // Level 10, but his bags are full: still the master's, not Bob's.
    on_map(&realm, move |m| set_level(m, gc, 10)).await;
    fill(&realm, gc).await;
    a.send(give(gc)).await;
    c.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(on_map(&realm, ring).await, (RING, true, false, 0));
    assert!(opened(&mut b, corpse).await);
    b.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 2 }).await;
    handled(&mut b).await;
    assert_eq!(on_map(&realm, move |m| count(m, gb, RING)).await, 0);
    // Room: it's his.
    unfill(&realm, gc).await;
    a.send(give(gc)).await;
    c.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) => (r.item == RING).then_some(()),
        _ => None,
    })
    .await;
    assert_eq!(on_map(&realm, ring).await, (RING, false, true, 1));
}

/// Thistle Tea's reservations and `finished?/1`: an item that doesn't fit
/// stays in the window to take later; a corpse with nothing left is no
/// longer lootable, and one with a roll still under way is.
#[tokio::test]
async fn what_does_not_fit_stays_in_the_window() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(kobold(30)).await;
    kill(&realm, ga, 30).await;
    let corpse = kobold(30);
    fill(&realm, ga).await;
    assert!(opened(&mut a, corpse).await);
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_LOOT_MONEY {}).await;
    a.send(CMSG_LOOT_RELEASE { guid: corpse }).await;
    handled(&mut a).await;
    let state = move |m: &mut Map| {
        let lootable = m
            .creature(corpse)
            .unwrap()
            .fields
            .get_u32(UNIT_DYNAMIC_FLAGS)
            & loot::DYNFLAG_LOOTABLE
            != 0;
        let l = loot::loot_ref(m, corpse).unwrap();
        (l.items[0].taken, l.gold, lootable, count(m, ga, BLADE))
    };
    assert_eq!(
        on_map(&realm, state).await,
        (false, 0, true, 0),
        "the blade left"
    );
    unfill(&realm, ga).await;
    assert!(opened(&mut a, corpse).await);
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: corpse }).await;
    handled(&mut a).await;
    assert_eq!(on_map(&realm, state).await, (true, 0, false, 1), "emptied");
}

/// Thistle Tea's need ownership: a roll's winner who can't carry the item
/// keeps it to take when they can, and no one else may.
#[tokio::test]
async fn a_winner_keeps_what_does_not_fit() {
    let realm = harness::realm_with(data());
    let [mut a, mut b, c] = group(&realm, GroupLootSetting::GroupLoot).await;
    let (ga, gb) = (a.guid, b.guid);
    fill(&realm, gb).await;
    kill(&realm, ga, 25).await;
    let corpse = kobold(25);
    b.wait_for("SMSG_LOOT_START_ROLL", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_START_ROLL(r) if r.item == BLADE => Some(()),
        _ => None,
    })
    .await;
    for (who, vote) in [
        (&a, RollVote::Pass),
        (&b, RollVote::Need),
        (&c, RollVote::Pass),
    ] {
        who.send(CMSG_LOOT_ROLL {
            item: corpse,
            item_slot: 0,
            vote,
        })
        .await;
    }
    b.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
        _ => None,
    })
    .await;
    assert!(opened(&mut a, corpse).await);
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    handled(&mut a).await;
    assert_eq!(
        on_map(&realm, move |m| count(m, ga, BLADE)).await,
        0,
        "Bob's"
    );
    unfill(&realm, gb).await;
    assert!(opened(&mut b, corpse).await);
    b.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    b.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) => (r.item == BLADE).then_some(()),
        _ => None,
    })
    .await;
}
