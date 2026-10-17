//! Gifts through the protocol: wrapping paper and what a gift keeps of the
//! item inside through a relog and the mail; the quests' dependencies; and
//! Winter Veil's presents under the cities' trees, out with the event a GM
//! starts.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::npc::{GossipMenu, Quest};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::enchant::{self, Enchant};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

const PAPER: u32 = 5042;
const GIFT: u32 = 5043;
const SWORD: u32 = 2200;
const RING: u32 = 2201;
const POTION: u32 = 2202;
const BAG: u32 = 2203;
const UNIQUE: u32 = 2204;
const ENCHANTMENT: u32 = 1900;

/// Winter Veil: the gifts' event, two of the presents under Ironforge's
/// tree, their quests and what they give, as vmangos has them.
const GIFTS_EVENT: u32 = 21;
const CAREFULLY_WRAPPED: u32 = 180_743;
const GAILY_WRAPPED: u32 = 180_747;
const CAREFULLY_SPAWN: u32 = 151_767;
const GAILY_SPAWN: u32 = 151_761;
const PRESENT_QUEST: u32 = 8744;
const GAILY_QUEST: u32 = 8768;
const PRESENT: u32 = 21_191;
const COOKIE: u32 = 21_254;
const PRESENTS_MENU: u32 = 6797;
const PRESENTS_TEXT: u32 = 8109;
/// The Wonderform Operator, who sends Alliance players to the tree, and
/// her quest, offered only while the gifts are out.
const OPERATOR: u32 = 15_732;
const WINTERS_PRESENTS: u32 = 8827;
const IF_TREE: (f32, f32, f32) = (-4916.04, -981.171, 501.848);

fn item(entry: u32, name: &str) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: name.into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 1,
        buy_count: 1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let mut paper = item(PAPER, "Red Ribboned Wrapping Paper");
    paper.class = 15;
    paper.flags = 0x200;
    paper.stackable = 10;
    let mut gift = item(GIFT, "Red Ribboned Gift");
    gift.flags = 0x200;
    let mut sword = item(SWORD, "Short Sword");
    sword.class = 2;
    sword.inventory_type = 13;
    sword.max_durability = 50;
    let mut ring = item(RING, "Ring of Binding");
    ring.class = 4;
    ring.inventory_type = 11;
    ring.bonding = 1;
    let mut potion = item(POTION, "Potion");
    potion.stackable = 5;
    let mut bag = item(BAG, "Small Pouch");
    bag.class = 1;
    bag.inventory_type = 18;
    bag.container_slots = 6;
    let mut unique = item(UNIQUE, "One of a Kind");
    unique.max_count = 1;
    let mut present = item(PRESENT, "Carefully Wrapped Present");
    present.class = 15;
    present.flags = 0x4;
    present.bonding = 1;
    let mut cookie = item(COOKIE, "Winter Veil Cookie");
    cookie.stackable = 20;
    for t in [
        paper, gift, sword, ring, potion, bag, unique, present, cookie,
    ] {
        d.items.insert(t.entry, t);
    }
    d.loot.item.insert(
        PRESENT,
        vec![LootEntry {
            item: COOKIE,
            chance: 100.0,
            min: 5,
            max: 5,
            ..Default::default()
        }],
    );
    // The gifts' event, never on by the calendar here: only a GM starts it.
    d.events.insert(
        GIFTS_EVENT,
        GameEvent {
            id: GIFTS_EVENT,
            name: "Feast of Winter Veil: Gifts".into(),
            start: 0,
            end: 0,
            occurence: 525_600,
            length: 12_960,
            hardcoded: false,
        },
    );
    let present_object = |entry, name: &str, menu| {
        let mut t = GameObjectTemplate {
            entry,
            kind: 2,
            display_id: 1,
            name: name.into(),
            flags: 4,
            size: 1.0,
            ..Default::default()
        };
        t.data[0] = 43;
        t.data[3] = menu;
        t
    };
    d.gameobject_templates.insert(
        CAREFULLY_WRAPPED,
        present_object(CAREFULLY_WRAPPED, "Carefully Wrapped Present", 0),
    );
    d.gameobject_templates.insert(
        GAILY_WRAPPED,
        present_object(GAILY_WRAPPED, "Gaily Wrapped Present", PRESENTS_MENU),
    );
    let under_the_tree = |guid, entry, x: f32, y: f32, z: f32| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x,
        y,
        z,
        respawn_min: 300,
        respawn_max: 300,
        state: 1,
        ..Default::default()
    };
    d.gameobject_spawns.insert(
        0,
        vec![
            under_the_tree(
                CAREFULLY_SPAWN,
                CAREFULLY_WRAPPED,
                IF_TREE.0,
                IF_TREE.1,
                IF_TREE.2,
            ),
            under_the_tree(GAILY_SPAWN, GAILY_WRAPPED, -4919.12, -980.604, 501.456),
        ],
    );
    d.event_objects.insert(CAREFULLY_SPAWN, GIFTS_EVENT as i32);
    d.event_objects.insert(GAILY_SPAWN, GIFTS_EVENT as i32);
    d.npc.gossip_menus.insert(
        PRESENTS_MENU,
        GossipMenu {
            texts: vec![(PRESENTS_TEXT, 0)],
            options: Vec::new(),
        },
    );
    let present_quest = |id, title: &str, min_level, reward| Quest {
        id,
        title: title.into(),
        min_level,
        level: i32::from(min_level),
        flags: 8,
        zone_or_sort: -22,
        offer_reward_text: "To a very special $r $c.".into(),
        rewards: [(reward, 1), (0, 0), (0, 0), (0, 0)],
        ..Default::default()
    };
    d.npc.quests.insert(
        PRESENT_QUEST,
        present_quest(PRESENT_QUEST, "A Carefully Wrapped Present", 1, PRESENT),
    );
    d.npc.quests.insert(
        GAILY_QUEST,
        present_quest(GAILY_QUEST, "A Gaily Wrapped Present", 20, 21_310),
    );
    for (object, quest) in [
        (CAREFULLY_WRAPPED, PRESENT_QUEST),
        (GAILY_WRAPPED, GAILY_QUEST),
    ] {
        d.npc.object_starts.insert(object, vec![quest]);
        d.npc.object_ends.insert(object, vec![quest]);
    }
    let mut winters = present_quest(WINTERS_PRESENTS, "Winter's Presents", 1, 0);
    winters.required_races = 77;
    winters.rewards = [(0, 0); 4];
    d.npc.quests.insert(WINTERS_PRESENTS, winters);
    d.npc
        .creature_starts
        .insert(OPERATOR, vec![WINTERS_PRESENTS]);
    d.world.event_quests.insert(WINTERS_PRESENTS, GIFTS_EVENT);
    d.creature_templates.insert(
        OPERATOR,
        CreatureTemplate {
            entry: OPERATOR,
            name: "Wonderform Operator".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 30,
            level_max: 30,
            faction: 35,
            npc_flags: 0x2,
            unit_class: 1,
            health_max: 500,
            speed_walk: 1.0,
            speed_run: 1.0,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 90,
            entries: vec![OPERATOR],
            map: 0,
            x: IF_TREE.0 + 4.0,
            y: IF_TREE.1,
            z: IF_TREE.2,
            respawn_min: 300,
            respawn_max: 300,
            ..Default::default()
        }],
    );
    d
}

fn map0(realm: &Arc<Realm>) -> MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map0(realm).query(f).await.expect("the map answers")
}

/// Give `g` `count` of `entry`; where it went.
async fn give(realm: &Arc<Realm>, g: Guid, entry: u32, count: u32) -> (u8, u8) {
    on_map(realm, move |m| {
        crate::game::map::inventory::add_item(m, g, entry, count, false).expect("room");
        let p = &m.players[&g];
        let i = p.items.iter().rposition(|i| i.entry == entry).unwrap();
        (p.items[i].bag, p.items[i].slot)
    })
    .await
}

/// What is at a player's item: entry, flags, gift wrapper, owner,
/// durability and max, its first enchantment, and where it is.
#[derive(Debug, PartialEq)]
struct Look {
    entry: u32,
    field_entry: u32,
    flags: u32,
    wrapper: Guid,
    owner: Guid,
    durability: (u32, u32),
    enchant: u32,
    at: (u8, u8),
}

async fn look(realm: &Arc<Realm>, g: Guid, item: Guid) -> Option<Look> {
    on_map(realm, move |m| {
        let p = m.players.get(&g)?;
        let it = &p.items[p.item_by_guid(item)?];
        Some(Look {
            entry: it.entry,
            field_entry: it.fields.get_u32(OBJECT_FIELD_ENTRY),
            flags: it.fields.get_u32(ITEM_FIELD_FLAGS),
            wrapper: it.fields.get_guid(ITEM_FIELD_GIFTCREATOR),
            owner: it.fields.get_guid(ITEM_FIELD_OWNER),
            durability: (
                it.fields.get_u32(ITEM_FIELD_DURABILITY),
                it.fields.get_u32(ITEM_FIELD_MAXDURABILITY),
            ),
            enchant: enchant::get(it, 0).id,
            at: (it.bag, it.slot),
        })
    })
    .await
}

async fn relog(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    let g = c.guid;
    c.login(g).await;
}

/// `SMSG_INVENTORY_CHANGE_FAILURE` with `result`.
async fn refused(c: &mut Client, result: InventoryResult) {
    let want = result.as_int();
    c.wait_frame(&format!("{result:?}"), 0x112, |b| b.first() == Some(&want))
        .await;
}

fn wrap(paper: (u8, u8), what: (u8, u8)) -> CMSG_WRAP_ITEM {
    CMSG_WRAP_ITEM {
        gift_bag_index: paper.0,
        gift_slot: paper.1,
        item_bag_index: what.0,
        item_slot: what.1,
    }
}

#[tokio::test]
async fn a_gift_keeps_its_item_through_a_relog_and_the_mail() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    // A worn, enchanted sword, and paper to wrap it in.
    let at = give(&realm, ga, SWORD, 1).await;
    let paper = give(&realm, ga, PAPER, 2).await;
    let sword = on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_at(at.0, at.1).unwrap();
        let it = &mut p.items[i];
        it.fields.set_u32(ITEM_FIELD_DURABILITY, 30);
        enchant::set(
            it,
            0,
            Enchant {
                id: ENCHANTMENT,
                duration_ms: 0,
                charges: 0,
            },
        );
        it.guid
    })
    .await;
    a.send(wrap(paper, at)).await;
    let wrapped = Look {
        entry: GIFT,
        field_entry: GIFT,
        flags: crate::game::map::items_misc::ITEM_DYNFLAG_WRAPPED,
        wrapper: ga,
        owner: ga,
        durability: (30, 50),
        enchant: ENCHANTMENT,
        at,
    };
    let mut seen = None;
    for _ in 0..100 {
        seen = look(&realm, ga, sword).await;
        if seen.as_ref().is_some_and(|l| l.entry == GIFT) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(seen.as_ref(), Some(&wrapped), "a gift, the sword inside");
    let papers = on_map(&realm, move |m| m.players[&ga].count_of(PAPER)).await;
    assert_eq!(papers, 1, "a sheet of paper used");
    // A gift isn't wrapped again.
    a.send(wrap(paper, at)).await;
    refused(&mut a, InventoryResult::WrappedCantBeWrapped).await;
    // Kept across a relog.
    relog(&mut a).await;
    assert_eq!(
        look(&realm, ga, sword).await.as_ref(),
        Some(&wrapped),
        "after a relog"
    );
    // Mailed to Bob, cash on delivery refused.
    let mail = |cod| CMSG_SEND_MAIL {
        mailbox: Guid::zero(),
        receiver: "Bob".into(),
        subject: "For you".into(),
        body: String::new(),
        unknown1: 0,
        unknown2: 0,
        item: sword,
        money: Gold::new(0),
        cash_on_delivery_amount: cod,
        unknown3: 0,
        unknown4: 0,
    };
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().set_money(1000)
    })
    .await;
    a.send(mail(10)).await;
    let cod = a
        .wait_for("SMSG_SEND_MAIL_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => Some(r.action),
            _ => None,
        })
        .await;
    assert!(
        matches!(
            cod,
            SMSG_SEND_MAIL_RESULT_MailAction::Send {
                result2: SMSG_SEND_MAIL_RESULT_MailResultTwo::ErrCantSendWrappedCod
            }
        ),
        "{cod:?}"
    );
    a.send(mail(0)).await;
    a.wait_for("the mail sent", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => matches!(
            r.action,
            SMSG_SEND_MAIL_RESULT_MailAction::Send {
                result2: SMSG_SEND_MAIL_RESULT_MailResultTwo::Ok
            }
        )
        .then_some(()),
        _ => None,
    })
    .await;
    assert!(look(&realm, ga, sword).await.is_none(), "gone from Alice");
    b.wait_for("SMSG_RECEIVED_MAIL", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_RECEIVED_MAIL(_)).then_some(())
    })
    .await;
    b.send(CMSG_GET_MAIL_LIST {
        mailbox: Guid::zero(),
    })
    .await;
    let (id, listed) = b
        .wait_for("SMSG_MAIL_LIST_RESULT", |m| match m {
            ServerOpcodeMessage::SMSG_MAIL_LIST_RESULT(l) => {
                l.mails.first().map(|m| (m.message_id, m.item))
            }
            _ => None,
        })
        .await;
    assert_eq!(listed, GIFT, "the mail carries the gift");
    b.send(CMSG_MAIL_TAKE_ITEM {
        mailbox: Guid::zero(),
        mail_id: id,
    })
    .await;
    b.wait_for("the gift taken", |m| match m {
        ServerOpcodeMessage::SMSG_SEND_MAIL_RESULT(r) => matches!(
            r.action,
            SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
                result: SMSG_SEND_MAIL_RESULT_MailResult::Ok { .. }
            }
        )
        .then_some(()),
        _ => None,
    })
    .await;
    relog(&mut b).await;
    let bobs = look(&realm, gb, sword)
        .await
        .expect("Bob has the gift, by its GUID");
    assert_eq!(
        (bobs.entry, bobs.flags, bobs.wrapper, bobs.owner),
        (GIFT, wrapped.flags, ga, gb),
        "still Alice's gift, now Bob's"
    );
    // Opened: the sword, as it was, and Bob's.
    b.send(CMSG_OPEN_ITEM {
        bag_index: bobs.at.0,
        slot: bobs.at.1,
    })
    .await;
    let mut opened = None;
    for _ in 0..100 {
        opened = look(&realm, gb, sword).await;
        if opened.as_ref().is_some_and(|l| l.entry == SWORD) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        opened,
        Some(Look {
            entry: SWORD,
            field_entry: SWORD,
            flags: 0,
            wrapper: Guid::zero(),
            owner: gb,
            durability: (30, 50),
            enchant: ENCHANTMENT,
            at: bobs.at,
        }),
        "the sword back, worn and enchanted as Alice left it"
    );
    // And it stays a sword when saved.
    relog(&mut b).await;
    assert_eq!(look(&realm, gb, sword).await.map(|l| l.entry), Some(SWORD));
}

#[tokio::test]
async fn a_gift_is_traded_and_opened_by_its_new_owner() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    b.sees(ga).await;
    let at = give(&realm, ga, SWORD, 1).await;
    let paper = give(&realm, ga, PAPER, 1).await;
    let sword = on_map(&realm, move |m| {
        let p = &m.players[&ga];
        p.items[p.item_at(at.0, at.1).unwrap()].guid
    })
    .await;
    a.send(wrap(paper, at)).await;
    for _ in 0..100 {
        if look(&realm, ga, sword)
            .await
            .is_some_and(|l| l.entry == GIFT)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    a.send(CMSG_INITIATE_TRADE { guid: gb }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    a.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 0,
        bag: at.0,
        slot: at.1,
    })
    .await;
    // Bob sees a gift from Alice, not what is in it.
    let offered = b
        .wait_for("Alice's offer", |m| match m {
            ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) if t.trade_slots[0].item != 0 => {
                let s = &t.trade_slots[0];
                Some((s.item, s.wrapped, s.gift_wrapper))
            }
            _ => None,
        })
        .await;
    assert_eq!(offered, (GIFT, true, ga));
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    b.wait_for("TRADE_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::TradeComplete).then_some(())
        }
        _ => None,
    })
    .await;
    let bobs = look(&realm, gb, sword).await.expect("Bob has the gift");
    assert_eq!((bobs.entry, bobs.wrapper, bobs.owner), (GIFT, ga, gb));
    b.send(CMSG_OPEN_ITEM {
        bag_index: bobs.at.0,
        slot: bobs.at.1,
    })
    .await;
    let mut opened = None;
    for _ in 0..100 {
        opened = look(&realm, gb, sword).await;
        if opened.as_ref().is_some_and(|l| l.entry == SWORD) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let opened = opened.unwrap();
    assert_eq!(
        (
            opened.entry,
            opened.flags,
            opened.wrapper,
            opened.owner,
            opened.durability
        ),
        (SWORD, 0, Guid::zero(), gb, (50, 50))
    );
}

#[tokio::test]
async fn wrapping_refuses_what_mangos_refuses() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let paper = give(&realm, g, PAPER, 3).await;
    let ring = give(&realm, g, RING, 1).await;
    let potion = give(&realm, g, POTION, 1).await;
    let bag = give(&realm, g, BAG, 1).await;
    let unique = give(&realm, g, UNIQUE, 1).await;
    let sword = give(&realm, g, SWORD, 1).await;
    for (what, why) in [
        (paper, InventoryResult::WrappedCantBeWrapped),
        (ring, InventoryResult::BoundCantBeWrapped),
        (potion, InventoryResult::StackableCantBeWrapped),
        (bag, InventoryResult::BagsCantBeWrapped),
        (unique, InventoryResult::UniqueCantBeWrapped),
        ((BAG_PLAYER, 38), InventoryResult::ItemNotFound),
    ] {
        a.send(wrap(paper, what)).await;
        refused(&mut a, why).await;
    }
    // Only paper wraps.
    a.send(wrap(sword, potion)).await;
    refused(&mut a, InventoryResult::ItemNotFound).await;
    // Nothing worn: not the sword in hand, nor a bag on the bar.
    on_map(&realm, move |m| {
        crate::game::map::inventory::move_item(m, g, sword, (BAG_PLAYER, 15));
        crate::game::map::inventory::move_item(m, g, bag, (BAG_PLAYER, 19));
    })
    .await;
    a.send(wrap(paper, (BAG_PLAYER, 15))).await;
    refused(&mut a, InventoryResult::EquippedCantBeWrapped).await;
    a.send(wrap(paper, (BAG_PLAYER, 19))).await;
    refused(&mut a, InventoryResult::EquippedCantBeWrapped).await;
    // Nor by the dead.
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Dead {
            since: std::time::Instant::now(),
        };
    })
    .await;
    a.send(wrap(paper, unique)).await;
    refused(&mut a, InventoryResult::YouAreDead).await;
    let left = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.count_of(PAPER), p.count_of(GIFT))
    })
    .await;
    assert_eq!(left, (3, 0), "no paper spent, no gift made");
}

/// What a GO use or quest packet brings back about quests.
#[derive(Debug, PartialEq)]
enum Said {
    Details(u32),
    Reward(u32),
    Done(u32),
    Menu(u32, Vec<u32>),
    List(Vec<u32>),
    Invalid,
}

fn said(m: &ServerOpcodeMessage) -> Option<Said> {
    Some(match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_DETAILS(d) => Said::Details(d.quest_id),
        ServerOpcodeMessage::SMSG_QUESTGIVER_OFFER_REWARD(r) => Said::Reward(r.quest_id),
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_COMPLETE(c) => Said::Done(c.quest_id),
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => Said::Menu(
            g.title_text_id,
            g.quests.iter().map(|q| q.quest_id).collect(),
        ),
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_LIST(l) => {
            Said::List(l.quest_items.iter().map(|q| q.quest_id).collect())
        }
        ServerOpcodeMessage::SMSG_QUESTGIVER_QUEST_INVALID(_) => Said::Invalid,
        _ => return None,
    })
}

async fn spawned(realm: &Arc<Realm>, spawn: u32) -> Option<Guid> {
    on_map(realm, move |m| {
        m.gameobjects
            .iter()
            .find(|(_, o)| o.spawn_guid == Some(spawn))
            .map(|(_, o)| o.guid)
    })
    .await
}

/// The next `GAMEOBJECT_DYN_FLAGS` this client is told for `object`.
async fn shown(c: &mut Client, object: Guid) -> i32 {
    c.wait_for("the object's dynamic flags", |m| {
        harness::values(m)
            .into_iter()
            .find_map(|(g, mask)| match mask {
                UpdateMask::GameObject(o) if g == object => o.gameobject_dyn_flags(),
                _ => None,
            })
    })
    .await
}

async fn status(c: &mut Client, giver: Guid) -> QuestGiverStatus {
    c.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: giver }).await;
    c.wait_for("SMSG_QUESTGIVER_STATUS", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(s) if s.guid == giver => Some(s.status),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn winter_veils_presents_come_with_the_event_and_give_once() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    // To the tree in Ironforge.
    let map = map0(&realm);
    map.query(move |m| {
        m.teleport(
            g,
            m.key,
            Vector3d {
                x: IF_TREE.0,
                y: IF_TREE.1,
                z: IF_TREE.2,
            },
            0.0,
        )
    })
    .await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    let operator = guid_of_creature(&realm, OPERATOR).await;
    a.sees(operator).await;
    // Before the gifts: nothing under the tree, nothing to send her there.
    assert_eq!(spawned(&realm, CAREFULLY_SPAWN).await, None);
    assert_eq!(status(&mut a, operator).await, QuestGiverStatus::None);
    a.say(".event list").await;
    a.system_message("Events: none").await;
    a.say(".event start 21").await;
    a.system_message("Event 21 Feast of Winter Veil: Gifts started.")
        .await;
    a.say(".event start 21").await;
    a.system_message("already active").await;
    let present = spawned(&realm, CAREFULLY_SPAWN)
        .await
        .expect("a present under the tree");
    let gaily = spawned(&realm, GAILY_SPAWN).await.expect("and another");
    a.sees(present).await;
    a.sees(gaily).await;
    // The present has a quest for her: usable. The gaily wrapped one has
    // none until level 20.
    assert_eq!(shown(&mut a, present).await, 1);
    // It lasts past the calendar's next look.
    crate::game::events::sweep(&realm);
    assert!(crate::game::events::current(&realm).contains(&GIFTS_EVENT));
    assert_eq!(status(&mut a, operator).await, QuestGiverStatus::Available);
    // The present offers its quest; taken once, not twice.
    let used = |o| CMSG_GAMEOBJ_USE { guid: o };
    a.send(used(present)).await;
    assert_eq!(
        a.wait_for("the present's quest", said).await,
        Said::Details(PRESENT_QUEST)
    );
    let accept = CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: present,
        quest_id: PRESENT_QUEST,
    };
    a.send(accept).await;
    a.send(accept).await;
    assert_eq!(
        a.wait_for("a second take refused", said).await,
        Said::Invalid
    );
    let logged = on_map(&realm, move |m| {
        m.players[&g]
            .journal
            .log
            .iter()
            .filter(|l| l.quest == PRESENT_QUEST)
            .count()
    })
    .await;
    assert_eq!(logged, 1);
    // Nothing to do but turn it in, at the present.
    a.send(used(present)).await;
    assert_eq!(
        a.wait_for("the reward", said).await,
        Said::Reward(PRESENT_QUEST)
    );
    a.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: present,
        quest_id: PRESENT_QUEST,
        reward: 0,
    })
    .await;
    assert_eq!(
        a.wait_for("the quest done", said).await,
        Said::Done(PRESENT_QUEST)
    );
    assert_eq!(
        shown(&mut a, present).await,
        0,
        "nothing more for her there"
    );
    // Done with it for good: the present has nothing more for her.
    a.send(used(present)).await;
    assert!(
        matches!(a.wait_for("the present again", said).await, Said::Menu(_, q) if q.is_empty())
    );
    a.send(accept).await;
    assert_eq!(a.wait_for("no second time", said).await, Said::Invalid);
    let (presents, on_log) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.count_of(PRESENT), p.journal.has(PRESENT_QUEST))
    })
    .await;
    assert_eq!((presents, on_log), (1, false));
    // The gaily wrapped present has its pile of gifts, and its quest only
    // from level 20.
    a.send(used(gaily)).await;
    assert_eq!(
        a.wait_for("the pile of gifts", said).await,
        Said::Menu(PRESENTS_TEXT, Vec::new())
    );
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        crate::game::map::stats::set_level(m.players.get_mut(&g).unwrap(), &d, 20);
    })
    .await;
    assert_eq!(shown(&mut a, gaily).await, 1, "at 20, its quest is hers");
    a.send(used(gaily)).await;
    assert_eq!(
        a.wait_for("the pile, with its quest", said).await,
        Said::Menu(PRESENTS_TEXT, vec![GAILY_QUEST])
    );
    // Opened, the present gives its cookies and is used up.
    let (bag, slot, item) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        let it = p.items.iter().find(|i| i.entry == PRESENT).unwrap();
        (it.bag, it.slot, it.guid)
    })
    .await;
    a.send(CMSG_OPEN_ITEM {
        bag_index: bag,
        slot,
    })
    .await;
    let want = item.guid().to_le_bytes();
    let b = a
        .wait_frame("the present's loot", 0x160, |b| b.starts_with(&want))
        .await;
    // The GUID, the kind, no money, one item: its slot, entry and count.
    let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    assert_eq!((u32_at(9), b[13]), (0, 1));
    assert_eq!((b[14], u32_at(15), u32_at(19)), (0, COOKIE, 5));
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_LOOT_REMOVED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_REMOVED(_)).then_some(())
    })
    .await;
    a.send(CMSG_LOOT_RELEASE { guid: item }).await;
    a.wait_for("SMSG_LOOT_RELEASE_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(_)).then_some(())
    })
    .await;
    let (cookies, presents) = on_map(&realm, move |m| {
        let p = &m.players[&g];
        (p.count_of(COOKIE), p.count_of(PRESENT))
    })
    .await;
    assert_eq!((cookies, presents), (5, 0));
    // Stopped, the presents go and the operator says no more of them.
    a.say(".event stop 21").await;
    a.system_message("stopped").await;
    a.forgets(present).await;
    assert_eq!(spawned(&realm, CAREFULLY_SPAWN).await, None);
    crate::game::events::sweep(&realm);
    assert!(!crate::game::events::current(&realm).contains(&GIFTS_EVENT));
    assert_eq!(status(&mut a, operator).await, QuestGiverStatus::None);
}

async fn guid_of_creature(realm: &Arc<Realm>, entry: u32) -> Guid {
    on_map(realm, move |m| {
        m.creatures
            .iter()
            .find(|(_, c)| c.entry == entry)
            .map(|(_, c)| c.guid)
    })
    .await
    .expect("spawned")
}

/// Quests that hang on others: a quest whose `NextQuestId` points on, a
/// group all of which must be done, a breadcrumb, a chain, timed quests.
fn chain_data() -> GameData {
    let mut d = GameData::empty();
    let q = |id| Quest {
        id,
        title: format!("Quest {id}"),
        min_level: 1,
        level: 1,
        ..Default::default()
    };
    // 1 leads on to 2 by its NextQuestId alone.
    let mut first = q(1);
    first.next_quest = 2;
    d.npc.quests.insert(1, first);
    d.npc.quests.insert(2, q(2));
    // 3 and 4, a group all of which comes before 5.
    for id in [3, 4] {
        let mut x = q(id);
        x.exclusive_group = -10;
        x.next_quest = 5;
        d.npc.quests.insert(id, x);
    }
    d.npc.quests.insert(5, q(5));
    // 6 and 7: one or the other.
    for id in [6, 7] {
        let mut x = q(id);
        x.exclusive_group = 20;
        d.npc.quests.insert(id, x);
    }
    // 8, a breadcrumb to 9.
    let mut crumb = q(8);
    crumb.breadcrumb_for = 9;
    d.npc.quests.insert(8, crumb);
    d.npc.quests.insert(9, q(9));
    // 10 then 11 in a chain.
    let mut ten = q(10);
    ten.next_in_chain = 11;
    d.npc.quests.insert(10, ten);
    d.npc.quests.insert(11, q(11));
    // Two timed quests.
    for id in [12, 13] {
        let mut x = q(id);
        x.limit_time = 600;
        d.npc.quests.insert(id, x);
    }
    d
}

#[tokio::test]
async fn quests_wait_on_the_quests_they_hang_on() {
    let realm = harness::realm_with(chain_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let takes = |ids: &'static [u32]| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| {
                let d = m.realm.data.clone();
                let p = &m.players[&g];
                ids.iter()
                    .map(|id| crate::game::map::quests::can_take(p, &d.npc.quests[id], &d))
                    .collect::<Vec<bool>>()
            })
            .await
        }
    };
    let set = |logged: &'static [u32], rewarded: &'static [u32]| {
        let realm = realm.clone();
        async move {
            on_map(&realm, move |m| {
                let p = m.players.get_mut(&g).unwrap();
                p.journal.log = logged
                    .iter()
                    .map(|q| crate::game::store::Logged {
                        quest: *q,
                        expires: if *q >= 12 { i64::MAX } else { 0 },
                        ..Default::default()
                    })
                    .collect();
                p.journal.rewarded = rewarded.iter().copied().collect();
            })
            .await
        }
    };
    // Nothing done: 2 waits on 1, 5 on all of 3 and 4.
    assert_eq!(takes(&[1, 2, 3, 5]).await, vec![true, false, true, false]);
    set(&[], &[1, 3]).await;
    assert_eq!(takes(&[2, 5]).await, vec![true, false], "4 is still to do");
    set(&[], &[1, 3, 4]).await;
    assert_eq!(takes(&[5]).await, vec![true]);
    // One of a group shuts out the other, taken or done.
    set(&[6], &[]).await;
    assert_eq!(takes(&[7]).await, vec![false]);
    set(&[], &[7]).await;
    assert_eq!(takes(&[6]).await, vec![false]);
    // A breadcrumb isn't for one on its quest, nor the quest for one on
    // the breadcrumb.
    set(&[9], &[]).await;
    assert_eq!(takes(&[8]).await, vec![false]);
    set(&[8], &[]).await;
    assert_eq!(takes(&[9]).await, vec![false]);
    set(&[], &[9]).await;
    assert_eq!(
        takes(&[8]).await,
        vec![false],
        "its quest done, the breadcrumb is moot"
    );
    // A chain's next shuts out its previous; the previous on, its next.
    set(&[11], &[]).await;
    assert_eq!(takes(&[10]).await, vec![false]);
    set(&[10], &[]).await;
    assert_eq!(takes(&[11]).await, vec![false]);
    // One timed quest at a time.
    set(&[12], &[]).await;
    assert_eq!(takes(&[13, 1]).await, vec![false, true]);
}

/// The real Winter Veil: the presents under both trees, their quests and
/// loot, the operator's quest tied to the gifts, the groups of the
/// Greatfather's quests.
#[test]
fn the_real_winter_veil_loads() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    for (object, quest, reward) in [
        (180_743, 8744, 21_191),
        (180_746, 8767, 21_270),
        (180_747, 8768, 21_310),
        (180_748, 8769, 21_327),
        (180_793, 8803, 21_363),
    ] {
        let t = &d.gameobject_templates[&object];
        assert_eq!(t.kind, 2, "{} is a quest giver", t.name);
        assert!(d.npc.object_starts[&object].contains(&quest));
        assert!(d.npc.object_ends[&object].contains(&quest));
        let q = &d.npc.quests[&quest];
        assert_eq!(q.rewards[0].0, reward);
        let present = d.item(reward).unwrap();
        assert_ne!(present.flags & 0x4, 0, "{} opens", present.name);
        assert!(
            d.loot.item.contains_key(&reward),
            "{} has loot",
            present.name
        );
    }
    // Under Ironforge's tree and Orgrimmar's, with the gifts.
    let under: Vec<u32> = d
        .gameobject_spawns
        .values()
        .flatten()
        .filter(|s| s.entry == 180_743)
        .map(|s| s.guid)
        .collect();
    assert_eq!(under.len(), 2);
    assert!(
        under
            .iter()
            .all(|s| d.event_objects.get(s) == Some(&(GIFTS_EVENT as i32)))
    );
    assert!(d.events.contains_key(&2) && d.events.contains_key(&GIFTS_EVENT));
    assert_eq!(
        d.world.event_quests.get(&WINTERS_PRESENTS),
        Some(&GIFTS_EVENT)
    );
    assert!(d.npc.gossip_menus.contains_key(&PRESENTS_MENU));
    // Greatfather Winter is Here!: one of the group.
    let g = d.npc.quest_graph();
    assert!(g.exclusive.get(&7022).is_some_and(|o| o.contains(&7023)));
    // The holiday's mail: Smokywood Pastures' thanks.
    let mail = d.misc.quest_mail.get(&7045).expect("a mail for the thanks");
    assert!(
        d.misc
            .mail_items
            .get(&mail.template)
            .is_some_and(|i| i.iter().any(|(e, _)| *e == 17_712))
    );
}
