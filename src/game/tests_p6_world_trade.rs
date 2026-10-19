//! Round 6's world tests, ported from Thistle Tea's
//! `test/game/world/system/trade_test.exs` and `duel_test.exs`
//! (docs/parity/matrix.csv): a trade a partner leaves or walks away from,
//! and a duel's challenges, its forfeit and its pets.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::{GameData, ItemTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::duel::{self, Phase};
use crate::game::map::inventory::PACK_START;
use crate::game::map::{Map, MapKey};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_player2_ammo::handled;

const GEM: u32 = 9600;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.items.insert(
        GEM,
        ItemTemplate {
            entry: GEM,
            name: "Gem".into(),
            display_id: 1000,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d
}

/// `name` in the world at the start, with 1000 copper and a gem in the
/// pack's first slot (its GUID).
async fn trader(realm: &Arc<Realm>, name: &str, x: f32) -> (Client, Guid) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.heartbeat(START.0 + x, START.1, START.2).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    let gem = on(&map, move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let gem = crate::game::guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.set_money(1000);
        p.items.push(crate::game::map::player::Item::new(
            &d, gem, g, GEM, BAG_PLAYER, PACK_START, 1,
        ));
        p.refresh_inventory(&d);
        gem
    })
    .await;
    handled(&mut c).await;
    c.clear();
    (c, gem)
}

/// The next trade status `c` is told.
async fn trade_status(c: &mut Client) -> SMSG_TRADE_STATUS_TradeStatus {
    c.wait_for("SMSG_TRADE_STATUS", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => Some(s.status.clone()),
        _ => None,
    })
    .await
}

/// A trade open between `a` and `b`, `a` offering her gem and 100 copper
/// and accepting it.
async fn offered(a: &mut Client, b: &mut Client) {
    a.send(CMSG_INITIATE_TRADE { guid: b.guid }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    a.wait_for("the window open", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::OpenWindow).then_some(())
        }
        _ => None,
    })
    .await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag: BAG_PLAYER,
        slot: PACK_START,
    })
    .await;
    a.send(CMSG_SET_TRADE_GOLD {
        gold: Gold::new(100),
    })
    .await;
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(a).await;
    b.wait_for("her acceptance", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::TradeAccept).then_some(())
        }
        _ => None,
    })
    .await;
    a.clear();
    b.clear();
}

/// `g`'s money, whether they trade, and whether they have `gem`.
fn purse(m: &Map, g: Guid, gem: Guid) -> (u32, bool, bool) {
    let p = &m.players[&g];
    (p.money(), p.trade.is_some(), p.item_by_guid(gem).is_some())
}

/// Whether `c` is told the trade went through within [`NONE_WITHIN`].
async fn completes(c: &mut Client) -> bool {
    c.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::TradeComplete).then_some(())
        }
        _ => None,
    })
    .await
    .is_some()
}

/// Thistle Tea's trade "aborts when an owner disconnects before the second
/// preparation" and "disconnect before both snapshots commit preserves the
/// locked source and both balances": Alice has accepted when Bob's
/// connection drops; she is told the trade is off (mangos's logout,
/// `TradeCancel`), keeps her gem and her money, and her accepting again is
/// no trade.
#[tokio::test]
async fn a_partner_leaving_calls_the_trade_off_and_nothing_changes_hands() {
    let realm = harness::realm_with(data());
    let (mut a, gem) = trader(&realm, "Alice", 0.0).await;
    let (mut b, _) = trader(&realm, "Bob", 2.0).await;
    let (ga, gb) = (a.guid, b.guid);
    offered(&mut a, &mut b).await;
    b.disconnect();
    let why = trade_status(&mut a).await;
    assert!(
        matches!(
            why,
            SMSG_TRADE_STATUS_TradeStatus::TargetLogout
                | SMSG_TRADE_STATUS_TradeStatus::TradeCanceled
        ),
        "{why:?}"
    );
    let map = realm.maps.get(&realm, MapKey::world(0));
    assert_eq!(
        on(&map, move |m| purse(m, ga, gem)).await,
        (1000, false, true)
    );
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(&mut a).await;
    assert!(!completes(&mut a).await);
    assert_eq!(
        on(&map, move |m| purse(m, ga, gem)).await,
        (1000, false, true)
    );
    // Bob, back, has what he had.
    let mut b = Client::connect(&realm, "Bob");
    b.login(gb).await;
    let back = on(&map, move |m| m.players[&gb].money()).await;
    assert_eq!(back, 1000);
}

/// Thistle Tea's trade "moving away cancels and stale preparation cannot
/// revive the trade": a partner walking out of the trade's reach calls it
/// off at once (mangos's `Player::SetPosition`, `TradeCancel` with
/// `TARGET_TO_FAR`), not at the accept; accepting after is no trade.
#[tokio::test]
async fn walking_away_from_a_trade_calls_it_off_at_once() {
    let realm = harness::realm_with(data());
    let (mut a, gem) = trader(&realm, "Alice", 0.0).await;
    let (mut b, bobs) = trader(&realm, "Bob", 2.0).await;
    let (ga, gb) = (a.guid, b.guid);
    offered(&mut a, &mut b).await;
    b.heartbeat(START.0 + 6.0, START.1, START.2).await;
    handled(&mut b).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    harness::an_update(&map).await;
    assert!(
        on(&map, move |m| m.players[&ga].trade.is_some()).await,
        "six yards off is near enough"
    );
    b.heartbeat(START.0 + 20.0, START.1, START.2).await;
    let told = trade_status(&mut a).await;
    assert!(
        matches!(
            told,
            SMSG_TRADE_STATUS_TradeStatus::TargetToFar
                | SMSG_TRADE_STATUS_TradeStatus::TradeCanceled
        ),
        "{told:?}"
    );
    let _ = trade_status(&mut b).await;
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    handled(&mut b).await;
    assert!(!completes(&mut a).await);
    assert!(!completes(&mut b).await);
    assert_eq!(
        on(&map, move |m| purse(m, ga, gem)).await,
        (1000, false, true)
    );
    assert_eq!(
        on(&map, move |m| purse(m, gb, bobs)).await,
        (1000, false, true)
    );
}

/// A dwarf in the world at the start of Dun Morogh, beside the others.
async fn dwarf(realm: &Arc<Realm>, account: &str, name: &str) -> (Client, Guid) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Dwarf, Class::Warrior).await;
    (c, g)
}

/// `a` challenges `b`, who accepts, and the fight begins.
async fn fighting(map: &crate::game::map::MapHandle, a: Guid, b: &mut Client) {
    let gb = b.guid;
    on(map, move |m| duel::challenge(m, a, gb)).await;
    let flag = b
        .wait_for("SMSG_DUEL_REQUESTED", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_REQUESTED(r) => Some(r.initiator),
            _ => None,
        })
        .await;
    b.send(CMSG_DUEL_ACCEPTED { guid: flag }).await;
    handled(b).await;
    on(map, move |m| {
        for g in [a, gb] {
            if let Some(d) = m.players.get_mut(&g).and_then(|p| p.duel.as_mut()) {
                d.phase = Phase::Countdown {
                    until: Instant::now(),
                };
            }
        }
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while !on(map, move |m| duel::dueling(m, a, gb)).await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the duel never began"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Why `from` can't challenge `to` now, as the Duel spell is checked.
fn refusal(m: &Map, from: Guid, to: Guid) -> Option<u8> {
    let mut s = crate::game::data::spells::SpellTemplate {
        id: 7266,
        ..Default::default()
    };
    s.effects[0].effect = crate::game::map::spells::EFFECT_DUEL;
    duel::check(m, from, &s, Some(to))
}

/// Thistle Tea's duel "provides the admission snapshot used by spell
/// validation" and "a disconnect forfeits an active duel": while Alice and
/// Bob duel, neither may be challenged nor challenge another (mangos's
/// `SPELL_FAILED_TARGET_DUELING`); Alice's connection dropping mid-fight
/// is her flight, Bob the winner, the flag gone.
#[tokio::test]
async fn duellists_are_busy_and_one_who_leaves_flees_the_duel() {
    let realm = harness::realm_with(data());
    let (a, ga) = dwarf(&realm, "A", "Alice").await;
    let (mut b, gb) = dwarf(&realm, "B", "Bob").await;
    let (_c, gc) = dwarf(&realm, "C", "Cid").await;
    let map = realm.registry.online(ga).unwrap().map();
    assert_eq!(on(&map, move |m| refusal(m, gc, gb)).await, None);
    on(&map, move |m| duel::challenge(m, ga, gb)).await;
    for (from, to) in [(gc, gb), (gc, ga), (ga, gc), (gb, gc)] {
        assert_eq!(
            on(&map, move |m| refusal(m, from, to)).await,
            Some(0x68),
            "a challenge asked is a duel"
        );
    }
    on(&map, move |m| duel::cancel(m, gb)).await;
    assert_eq!(
        on(&map, move |m| refusal(m, gc, gb)).await,
        None,
        "called off"
    );
    fighting(&map, ga, &mut b).await;
    assert_eq!(on(&map, move |m| refusal(m, gc, ga)).await, Some(0x68));
    let flag = on(&map, move |m| m.players[&gb].duel.map(|d| d.flag)).await;
    b.clear();
    a.disconnect();
    let (reason, loser, winner) = b
        .wait_for("SMSG_DUEL_WINNER", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_WINNER(w) => {
                Some((w.reason, w.opponent_name.clone(), w.initiator_name.clone()))
            }
            _ => None,
        })
        .await;
    assert_eq!(
        (reason, loser.as_str(), winner.as_str()),
        (DuelWinnerReason::Fled, "Alice", "Bob")
    );
    let after = on(&map, move |m| {
        (
            m.players[&gb].duel.is_none(),
            flag.is_some_and(|f| m.gameobject(f).is_none()),
        )
    })
    .await;
    assert_eq!(after, (true, true));
}

/// Thistle Tea's duel "still stops a creature pet's attacks": the duel won,
/// the hunter's pet stops fighting the loser (mangos's
/// `CombatStopWithPets`).
#[tokio::test]
async fn a_duel_won_stops_the_pets_fighting_the_other_side() {
    let (realm, _h, gh, map, pet, _wolf) = crate::game::tests_pets::tamed().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Dwarf, Class::Warrior).await;
    fighting(&map, gh, &mut b).await;
    on(&map, move |m| {
        crate::game::map::combat::start_attack(m, pet, gb)
    })
    .await;
    let attacking = move |m: &mut Map| {
        m.creature(pet)
            .is_some_and(|c| c.combat.attacking && c.combat.victim == gb)
    };
    assert!(on(&map, attacking).await, "the pet at Bob");
    b.send(CMSG_DUEL_CANCELLED { guid: Guid::zero() }).await;
    handled(&mut b).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(&map, attacking).await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the pet fights on after the duel"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(on(&map, move |m| m.players[&gb].duel.is_none()).await);
}
