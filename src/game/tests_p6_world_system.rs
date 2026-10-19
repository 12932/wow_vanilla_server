//! Round 6's world tests, ported from Thistle Tea's
//! `test/game/world/system/` (docs/parity/matrix.csv): GM tickets, the
//! auction house's races and restarts, honor's calendar, a GM's weather.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::honor::Ledger;
use crate::game::map::MapKey;
use crate::game::tests_p3_core_auction::{self as auctions, GOODS, auctioneer, goods, ok, sell};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

/// `c` writes a ticket: whether it was taken.
async fn ticket(c: &mut Client, text: &str) -> GmTicketResponse {
    c.send(CMSG_GMTICKET_CREATE {
        category: CMSG_GMTICKET_CREATE_GmTicketType::Stuck,
        map: Map::EasternKingdoms,
        position: Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        message: text.into(),
        reserved_for_future_use: String::new(),
    })
    .await;
    c.wait_for("SMSG_GMTICKET_CREATE", |m| match m {
        ServerOpcodeMessage::SMSG_GMTICKET_CREATE(r) => Some(r.response),
        _ => None,
    })
    .await
}

/// What `c` is shown of their ticket: its text and its three ages in
/// days (its own, the oldest open one's, the queue's last change), or
/// none.
async fn shown(c: &mut Client) -> Option<(String, f32, f32, f32)> {
    c.clear();
    c.send(CMSG_GMTICKET_GETTICKET {}).await;
    c.wait_for("SMSG_GMTICKET_GETTICKET", |m| match m {
        ServerOpcodeMessage::SMSG_GMTICKET_GETTICKET(t) => Some(match &t.status {
            SMSG_GMTICKET_GETTICKET_GmTicketStatus::HasText {
                text,
                days_since_ticket_creation,
                days_since_oldest_ticket_creation,
                days_since_last_updated,
                ..
            } => Some((
                text.clone(),
                *days_since_ticket_creation,
                *days_since_oldest_ticket_creation,
                *days_since_last_updated,
            )),
            _ => None,
        }),
        _ => None,
    })
    .await
}

/// Thistle Tea's `GmTickets.create` "keeps one open ticket per character
/// until it is answered" and `view` "shows the oldest open ticket and the
/// last change": a second ticket while one is open is refused; once a GM
/// has answered it, a new one replaces it. A writer is shown their own
/// ticket, the age of the oldest open one and of the queue's last change;
/// one with none, none.
#[tokio::test]
async fn one_open_ticket_each_and_its_view_tells_the_queues_ages() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    assert_eq!(
        ticket(&mut a, "Help").await,
        GmTicketResponse::CreateSuccess
    );
    assert_eq!(
        ticket(&mut a, "Help again").await,
        GmTicketResponse::CreateError
    );
    let id = realm
        .social
        .tickets
        .lock()
        .of(guid::counter(ga))
        .map(|t| t.id)
        .unwrap();
    a.say(&format!(".ticket respond {id} Done")).await;
    a.system_message("answered").await;
    assert_eq!(ticket(&mut a, "New").await, GmTicketResponse::CreateSuccess);
    let mine: Vec<(u32, String)> = realm
        .social
        .tickets
        .lock()
        .tickets
        .values()
        .filter(|t| t.player == guid::counter(ga))
        .map(|t| (t.id, t.text.clone()))
        .collect();
    assert_eq!(
        mine,
        [(id + 1, "New".to_owned())],
        "the answered one replaced"
    );

    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    assert_eq!(ticket(&mut b, "B").await, GmTicketResponse::CreateSuccess);
    // Alice's written ten days ago, Bob's two, the queue changed with his.
    {
        let now = crate::game::social::mail::now();
        let mut tickets = realm.social.tickets.lock();
        for t in tickets.tickets.values_mut() {
            let ago = if t.player == guid::counter(ga) { 10 } else { 2 };
            t.created = now - ago * 86_400;
            t.modified = t.created;
        }
        tickets.changed = now - 2 * 86_400;
    }
    let (text, own, oldest, changed) = shown(&mut b).await.expect("Bob's ticket");
    assert_eq!(text, "B");
    for (days, want) in [(own, 2.0), (oldest, 10.0), (changed, 2.0)] {
        assert!((days - want).abs() < 0.01, "{days} for {want}");
    }
    let mut c = Client::connect(&realm, "C");
    c.enter("Cid", Race::Human, Class::Warrior).await;
    assert_eq!(shown(&mut c).await, None);
    let _ = gb;
}

async fn money(realm: &Arc<Realm>, g: Guid) -> u32 {
    let map = realm.maps.get(realm, MapKey::world(0));
    on(&map, move |m| m.players[&g].money()).await
}

/// Thistle Tea's `Auction.transact` "serializes competing buyouts and
/// charges only the winning buyer": two buy one auction out at once; one
/// has it, by mail, and pays; the other is told it's gone and keeps every
/// copper; the seller is paid once.
#[tokio::test]
async fn of_two_buying_one_auction_out_at_once_only_the_winner_pays() {
    let realm = harness::realm_with(auctions::data());
    let mut a = auctions::player(&realm, "A", "Alice").await;
    let mut b = auctions::player(&realm, "B", "Bob").await;
    let mut c = auctions::player(&realm, "C", "Cid").await;
    let item = goods(&realm, a.guid).await;
    let (id, _) = sell(&mut a, item, 100, 1_000, 120).await;
    b.clear();
    c.clear();
    for x in [&b, &c] {
        x.send(CMSG_AUCTION_PLACE_BID {
            auctioneer: auctioneer(),
            auction_id: id,
            price: Gold::new(1_000),
        })
        .await;
    }
    let result = async |x: &mut Client| {
        x.wait_for("SMSG_AUCTION_COMMAND_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_AUCTION_COMMAND_RESULT(r) => Some(r.action),
            _ => None,
        })
        .await
    };
    let (rb, rc) = (result(&mut b).await, result(&mut c).await);
    let gone = SMSG_AUCTION_COMMAND_RESULT_AuctionCommandAction::BidPlaced {
        result: SMSG_AUCTION_COMMAND_RESULT_AuctionCommandResult::ErrItemNotFound,
    };
    let (winner, loser) = match (ok(&rb), ok(&rc)) {
        (true, false) => {
            assert_eq!(rc, gone);
            (b.guid, c.guid)
        }
        (false, true) => {
            assert_eq!(rb, gone);
            (c.guid, b.guid)
        }
        other => panic!("one winner: {other:?} ({rb:?}, {rc:?})"),
    };
    assert_eq!(money(&realm, winner).await, 10_000 - 1_000);
    assert_eq!(money(&realm, loser).await, 10_000, "the loser pays nothing");
    let mail = realm.social.mail.lock();
    let to = |g: Guid| {
        mail.mails
            .values()
            .filter(|m| m.receiver == guid::counter(g))
            .map(|m| (m.money, m.item.as_ref().map(|i| i.guid)))
            .collect::<Vec<_>>()
    };
    assert_eq!(to(winner), [(0, Some(guid::counter(item)))]);
    assert_eq!(to(loser), []);
    assert_eq!(to(a.guid).len(), 1, "the seller paid once");
    assert!(realm.social.auctions.lock().auctions.is_empty());
}

/// Thistle Tea's auctions' "restart recovery restores the book and
/// receipts and expires a bid exactly once": an auction with Bob's bid on
/// it is there after a restart, his bid and all; at its deadline he is
/// mailed the goods and Alice her money, once however often it's swept.
#[tokio::test]
async fn an_auction_with_a_bid_outlasts_a_restart_and_settles_once() {
    let dir = std::env::temp_dir().join(format!("wvs-p6g-auction-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("characters.sqlite");
    let (store, startup) = crate::game::store::Store::open(&db).unwrap();
    let realm = Realm::new(
        harness::test_config(),
        store,
        startup,
        auctions::data(),
        None,
    );
    let mut a = auctions::player(&realm, "A", "Alice").await;
    let mut b = auctions::player(&realm, "B", "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    let item = goods(&realm, ga).await;
    let (id, _) = sell(&mut a, item, 100, 0, 120).await;
    assert!(ok(&auctions::bid(&mut b, id, 100).await));
    handled(&mut b).await;
    realm.shutdown().await;
    drop((a, b));
    drop(realm);

    let (store, startup) = crate::game::store::Store::open(&db).unwrap();
    let realm = Realm::new(
        harness::test_config(),
        store,
        startup,
        auctions::data(),
        None,
    );
    let kept = realm
        .social
        .auctions
        .lock()
        .auctions
        .get(&id)
        .map(|x| (x.owner, x.bidder, x.bid, x.item.guid, x.item.entry));
    assert_eq!(
        kept,
        Some((
            guid::counter(ga),
            guid::counter(gb),
            100,
            guid::counter(item),
            GOODS
        ))
    );
    realm
        .social
        .auctions
        .lock()
        .auctions
        .get_mut(&id)
        .unwrap()
        .expires_at = crate::game::social::mail::now() - 1;
    for _ in 0..3 {
        crate::game::social::auction::sweep(&realm);
    }
    assert!(realm.social.auctions.lock().auctions.is_empty());
    let mail = realm.social.mail.lock();
    let to = |g: Guid| {
        mail.mails
            .values()
            .filter(|m| m.receiver == guid::counter(g))
            .map(|m| (m.money, m.item.as_ref().map(|i| i.guid)))
            .collect::<Vec<_>>()
    };
    assert_eq!(to(gb), [(0, Some(guid::counter(item)))], "won, once");
    // 100 less the goblins' 15, and the 75 deposit back.
    assert_eq!(to(ga), [(160, None)], "paid, once");
    drop(mail);
    drop(realm);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Thistle Tea's honor "settles before the first award after midnight":
/// the first kill of a new day goes on that day, the calendar moved on
/// first (wvs's sweep moves it every five seconds; an award doesn't wait
/// for it), and a week it settles is told at the next sweep.
#[test]
fn the_first_kill_after_midnight_counts_for_the_new_day() {
    let mut l = Ledger::new(5, 4);
    l.register(1, 0, 60);
    l.register(2, 1, 60);
    let first = l.on(5).player_kill(1, 2, 1.0).expect("honorable").points;
    assert_eq!(first, 188.0);
    let again = l.on(6).player_kill(1, 2, 1.0).expect("honorable").points;
    assert_eq!(l.day, 6);
    let days = &l.entries[&1].days;
    assert_eq!(days[&5].contribution, first);
    assert_eq!(days[&6].contribution, first, "a new day, no diminishing");
    assert_eq!(again, first);
    assert!(!l.week_turned);
    l.on(l.week_start + 7);
    assert!(l.week_turned, "the sweep tells the week's ranks");
}

/// Thistle Tea's honor "refreshing a profile preserves its ledger": a
/// character registered again (a new level) keeps the day's kills, and the
/// next kill of the same victim is worth a tenth less.
#[test]
fn registering_again_keeps_the_ledger() {
    let mut l = Ledger::new(5, 4);
    l.register(1, 0, 60);
    l.register(2, 1, 60);
    let first = l.on(5).player_kill(1, 2, 1.0).unwrap().points;
    let before = l.entries[&1].clone();
    l.register(1, 0, 59);
    let after = &l.entries[&1];
    assert_eq!(after.days, before.days);
    assert_eq!(after.level, 59);
    let second = l.on(5).player_kill(1, 2, 1.0).unwrap().points;
    assert_eq!(
        second,
        crate::game::honor::kill_points(59, 60, 0, 1).trunc()
    );
    assert_eq!(l.entries[&1].days[&5].contribution, first + second);
    assert_eq!(l.entries[&1].days[&5].honorable_kills, 2);
}

/// Thistle Tea's weather "permanent controls survive rolls until resumed":
/// a GM's weather held for good isn't rolled over by the season, however
/// often it's swept; let go, the next sweep rolls it again.
#[tokio::test]
async fn a_gms_weather_held_for_good_lasts_until_let_go() {
    let mut d = crate::game::data::GameData::empty();
    d.weather.insert(12, [[100, 0, 0]; 4]);
    let realm = harness::realm_with(d);
    crate::game::weather::set(&realm, 12, 2, 0.8, true);
    let held = realm.social.weather.lock()[&12];
    assert_eq!((held.kind, held.grade), (2, 0.8));
    for _ in 0..20 {
        crate::game::weather::sweep(&realm);
        assert_eq!(realm.social.weather.lock()[&12], held, "held");
    }
    crate::game::weather::release(&realm, 12);
    crate::game::weather::sweep(&realm);
    let now = crate::game::social::mail::now();
    let rolled = realm.social.weather.lock()[&12];
    assert!(
        rolled.next > now && rolled.next < i64::MAX,
        "rolled again, the next roll in ten minutes: {rolled:?}"
    );
}

const GROUNDING: u32 = 96_400;
const CHAIN_BOLT: u32 = 96_401;
const TOTEM: u32 = 3_830;

fn magnets() -> crate::game::data::GameData {
    use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, SpellDuration, SpellEffect};
    use crate::game::tests_p3_entity::{aura_effect, spell};
    let mut d = crate::game::tests_p3_combat::data();
    let mut totem = crate::game::tests_p3_combat::template(TOTEM, 11);
    totem.faction = crate::game::tests_p3_combat::HUMANS;
    d.creature_templates.insert(TOTEM, totem);
    let mut grounding = spell(
        GROUNDING,
        &[aura_effect(
            crate::game::map::auras::AURA_SPELL_MAGNET,
            0,
            0,
            1,
        )],
    );
    grounding.duration_index = 9;
    // A magic bolt that jumps to two more.
    let mut bolt = spell(
        CHAIN_BOLT,
        &[SpellEffect {
            effect: 2,
            base_points: 99,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            chain_target: 3,
            ..Default::default()
        }],
    );
    bolt.dmg_class = 1;
    bolt.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    for s in [grounding, bolt] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.durations.insert(
        9,
        SpellDuration {
            base: 60_000,
            ..Default::default()
        },
    );
    d
}

/// Thistle Tea's spell magnets "an intercepted chain stops at the magnet",
/// "launch packets and delivery use the redirected target" and "source
/// process exit retires protection": a chain bolt at Alice under a
/// grounding totem goes to the totem and jumps on to no one, Alice beside
/// it whole (mangos's `CheckAndAddMagnetTarget`); the spell's go names the
/// totem alone. A totem gone from the world grounds nothing: the next
/// lands on her.
#[tokio::test]
async fn a_grounded_chain_stops_at_its_magnet_and_a_gone_magnet_grounds_nothing() {
    use crate::game::fields::{UNIT_FIELD_HEALTH, UNIT_FIELD_MAXHEALTH};
    use crate::game::map::Map;
    use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
    let (_realm, mut a, ga, map) = setup_with(magnets(), Class::Warrior).await;
    let hardy = |m: &mut Map, g: Guid| {
        let f = &mut m.players.get_mut(&g).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000_000);
        f.set_u32(UNIT_FIELD_HEALTH, 100_000_000);
    };
    let whole = |m: &Map, g: Guid| {
        let f = m.unit_fields(g).unwrap();
        f.get_u32(UNIT_FIELD_HEALTH) == f.get_u32(UNIT_FIELD_MAXHEALTH)
    };
    let ground = move |m: &mut Map, totem: Guid| {
        let s = m.realm.data.spells.get(GROUNDING).cloned().unwrap();
        for on in [ga, totem] {
            crate::game::map::auras::apply(m, totem, on, &s, 0, 0);
        }
        hardy(m, ga);
    };
    let (brute, totem) = on(&map, move |m| {
        hardy(m, ga);
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        let totem = spawn(m, ga, TOTEM, 20, 1.0);
        ground(m, totem);
        (brute, totem)
    })
    .await;
    a.sees(totem).await;
    a.clear();
    let r = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, brute, CHAIN_BOLT, Some(ga));
        (whole(m, ga), whole(m, totem))
    })
    .await;
    assert_eq!(
        r,
        (true, false),
        "the totem took it, and it went no further"
    );
    let hits = a
        .wait_for("the chain's go", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == CHAIN_BOLT => Some(s.hits.clone()),
            _ => None,
        })
        .await;
    assert_eq!(hits, [totem]);
    let landed = on(&map, move |m| {
        let gone = spawn(m, ga, TOTEM, 20, 1.5);
        ground(m, gone);
        m.remove_creature(gone);
        crate::game::map::spells::cast_triggered(m, brute, CHAIN_BOLT, Some(ga));
        !whole(m, ga)
    })
    .await;
    assert!(landed, "a gone totem grounds nothing");
}

/// Thistle Tea's item store "allocates unique item guids" and "stores the
/// created item": two items made for Alice have GUIDs of their own, of
/// the item kind, and each is hers, owned and held by her, as its fields
/// say.
#[tokio::test]
async fn items_made_have_guids_of_their_own_and_are_their_owners() {
    use crate::game::fields::{ITEM_FIELD_CONTAINED, ITEM_FIELD_OWNER};
    use crate::game::tests_p3_combat::{SWORD, setup_with};
    let (_realm, _a, ga, map) =
        setup_with(crate::game::tests_p3_combat::data(), Class::Warrior).await;
    let made = on(&map, move |m| {
        for _ in 0..2 {
            crate::game::map::inventory::add_item(m, ga, SWORD, 1, false).expect("room");
        }
        m.players[&ga]
            .items
            .iter()
            .filter(|i| i.entry == SWORD)
            .map(|i| {
                (
                    i.guid,
                    i.fields.get_guid(ITEM_FIELD_OWNER),
                    i.fields.get_guid(ITEM_FIELD_CONTAINED),
                )
            })
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(made.len(), 2);
    assert_ne!(made[0].0, made[1].0, "a GUID each");
    for (g, owner, contained) in made {
        assert_eq!(g.guid() >> 48, 0x4000, "of the item kind: {g}");
        assert_eq!((owner, contained), (ga, ga));
    }
}
