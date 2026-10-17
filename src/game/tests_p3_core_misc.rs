//! Phase 3's odds and ends of Thistle Tea's core: dithering a fraction to
//! a whole, the transports' GUIDs, a ticket's age in days, a loot row's
//! count, and mail's delivery delay and cash on delivery's three days.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::*;
use crate::game::guid::{self, HighGuid};
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::map::reputation::dither;

/// Thistle Tea's `Math.dither/2`: a whole value whatever the roll; a
/// fraction its floor or its ceiling, on average what it was; a loss the
/// same way down.
#[test]
fn dithering_keeps_whole_values_and_fractions_on_average() {
    for v in [0.0, 10.0, -3.0] {
        for roll in [0.0, 0.5, 0.999] {
            assert_eq!(dither(v, roll), v as i32);
        }
    }
    for v in [0.25f32, 7.5, 19.75, -0.25] {
        let samples: Vec<i32> = (0..100)
            .map(|n| dither(v, (n as f32 + 0.5) / 100.0))
            .collect();
        assert!(
            samples
                .iter()
                .all(|s| *s == v.floor() as i32 || *s == v.ceil() as i32),
            "{v}: {samples:?}"
        );
        let mean = samples.iter().sum::<i32>() as f32 / 100.0;
        assert!((mean - v).abs() < 1e-4, "{v}: {mean}");
    }
}

/// Thistle Tea's `Guid.transport?/1`: an animated transport's GUID and a
/// map object transport's are transports, a game object's of the same
/// entry not.
#[test]
fn transports_have_guids_of_their_own() {
    let animated = Guid::new((0xF120u64 << 48) | (176_080u64 << 24) | 18_802);
    let ship = Guid::new((0x1FC0u64 << 48) | 164_871);
    let object = guid::game_object(176_080, 18_802);
    assert_eq!(guid::high(animated), Some(HighGuid::Transport));
    assert_eq!(guid::high(ship), Some(HighGuid::MoTransport));
    assert_eq!(guid::high(object), Some(HighGuid::GameObject));
}

/// Thistle Tea's loot `roll/3`: a row's count between its minimum and
/// maximum, each of them dropped.
#[test]
fn a_loot_rows_count_is_between_its_minimum_and_maximum() {
    use rand::SeedableRng;
    let data = GameData::empty();
    let rows = [LootEntry {
        item: 10,
        chance: 100.0,
        min: 2,
        max: 4,
        ..Default::default()
    }];
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..200 {
        let mut out = Vec::new();
        crate::game::map::loot::roll(&data, &rows, &[], &mut rng, &mut out, 0);
        assert_eq!(out.len(), 1);
        assert!((2..=4).contains(&out[0].count), "{}", out[0].count);
        seen.insert(out[0].count);
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), vec![2, 3, 4]);
}

/// Thistle Tea's `GmTicket.age_days/2`: a ticket half a day old is half a
/// day old, as the ticket's status tells its writer.
#[tokio::test]
async fn a_tickets_age_is_told_in_days() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_GMTICKET_CREATE {
        category: CMSG_GMTICKET_CREATE_GmTicketType::Stuck,
        map: wow_world_messages::vanilla::Map::EasternKingdoms,
        position: Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        message: "I am stuck".into(),
        reserved_for_future_use: String::new(),
    })
    .await;
    a.wait_for("SMSG_GMTICKET_CREATE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GMTICKET_CREATE(_)).then_some(())
    })
    .await;
    {
        let mut tickets = realm.social.tickets.lock();
        let half_a_day_ago = crate::game::social::mail::now() - 43_200;
        for t in tickets.tickets.values_mut() {
            t.created = half_a_day_ago;
            t.modified = half_a_day_ago;
        }
    }
    a.clear();
    a.send(CMSG_GMTICKET_GETTICKET {}).await;
    let days = a
        .wait_for("SMSG_GMTICKET_GETTICKET", |m| match m {
            ServerOpcodeMessage::SMSG_GMTICKET_GETTICKET(t) => match &t.status {
                SMSG_GMTICKET_GETTICKET_GmTicketStatus::HasText {
                    days_since_ticket_creation,
                    ..
                } => Some(*days_since_ticket_creation),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert!((days - 0.5).abs() < 0.01, "{days}");
}

fn mail_realm() -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.mail_delay = Duration::from_secs(3600);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let mut d = GameData::empty();
    d.items.insert(
        117,
        ItemTemplate {
            entry: 117,
            name: "Jerky".into(),
            display_id: 1000,
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    Realm::new(cfg, store, startup, d, None)
}

/// Thistle Tea's `Mail.new/2`: a letter is there at once; an item or
/// money sent to another account comes an hour later; cash on delivery's
/// mail goes back after three days.
#[tokio::test]
async fn mail_with_things_waits_an_hour_and_cash_on_delivery_three_days() {
    let realm = mail_realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let items = map
        .query(move |m| {
            m.players.get_mut(&ga).unwrap().set_money(10_000);
            for _ in 0..2 {
                let counter = m
                    .realm
                    .next_item
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let d = m.realm.data.clone();
                let p = m.players.get_mut(&ga).unwrap();
                let slot = p.free_carry_slot(&d).unwrap();
                p.items.push(crate::game::map::player::Item::new(
                    &d,
                    guid::item(counter),
                    ga,
                    117,
                    slot.0,
                    slot.1,
                    1,
                ));
                p.refresh_inventory(&d);
            }
            m.players[&ga]
                .items
                .iter()
                .filter(|i| i.entry == 117)
                .map(|i| i.guid)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    let send = |subject: &str, item: Guid, money: u32, cod: u32| CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: "Bob".into(),
        subject: subject.into(),
        body: String::new(),
        unknown1: 0,
        unknown2: 0,
        item,
        money: Gold::new(money),
        cash_on_delivery_amount: cod,
        unknown3: 0,
        unknown4: 0,
    };
    for m in [
        send("letter", Guid::zero(), 0, 0),
        send("item", items[0], 0, 0),
        send("money", Guid::zero(), 100, 0),
        send("cod", items[1], 0, 50),
    ] {
        a.send(m).await;
        a.wait_for("SMSG_SEND_MAIL_RESULT", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(_)).then_some(())
        })
        .await;
    }
    let now = crate::game::social::mail::now();
    let mails = realm.social.mail.lock();
    let of = |subject: &str| {
        mails
            .mails
            .values()
            .find(|m| m.subject == subject && m.receiver == guid::counter(gb))
            .map(|m| (m.deliver_at - now, m.expires_at - m.deliver_at))
            .unwrap()
    };
    let day = crate::game::social::mail::DAY;
    let (letter, item, money, cod) = (of("letter"), of("item"), of("money"), of("cod"));
    assert!(letter.0 <= 0, "at once: {letter:?}");
    for (what, (delay, _)) in [("item", item), ("money", money), ("cod", cod)] {
        assert!((3598..=3600).contains(&delay), "{what}: {delay}");
    }
    assert_eq!((item.1, cod.1), (30 * day, 3 * day));
}

/// Thistle Tea's `Weather.set/2`, a GM's `.weather`: rain at its fullest
/// told short of 1, fine weather with no grade at all; a grade past 0 to
/// 1, or weather of no kind, refused and the zone's weather kept.
#[tokio::test]
async fn a_gm_sets_the_weather_the_client_can_show() {
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.send(CMSG_ZONEUPDATE {
        area: Area::ElwynnForest,
    })
    .await;
    let weather = || {
        realm
            .social
            .weather
            .lock()
            .get(&12)
            .map(|w| (w.kind, w.grade))
    };
    a.say(".weather rain 1").await;
    let told = a
        .wait_for("the rain", |m| match m {
            ServerOpcodeMessage::SMSG_WEATHER(w) if w.weather_type == WeatherType::Rain => {
                Some(w.grade)
            }
            _ => None,
        })
        .await;
    assert_eq!(told, 0.9999);
    a.clear();
    a.say(".weather fine 0.8").await;
    a.wait_for("fine", |m| match m {
        ServerOpcodeMessage::SMSG_WEATHER(w) if w.weather_type == WeatherType::Fine => {
            Some(w.grade)
        }
        _ => None,
    })
    .await;
    assert_eq!(weather(), Some((0, 0.0)));
    for line in [
        ".weather rain -0.1",
        ".weather snow 1.1",
        ".weather lava 0.2",
    ] {
        a.say(line).await;
        a.system_message("").await;
        assert_eq!(weather(), Some((0, 0.0)), "{line}");
    }
}
