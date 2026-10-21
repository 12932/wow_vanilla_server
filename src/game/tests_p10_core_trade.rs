//! Round 10, against Thistle Tea's `trade/opening_test.exs`: a rogue picks
//! the lock of a box their partner keeps in the trade's seventh slot; as
//! the trade goes through the box is unlocked, Bob keeps it, and Alice's
//! Lockpicking rises (mangos-classic's `EffectOpenLock`, its
//! `UpdateGatherSkill` for a lock not opened by an item).

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::professions::SKILL_LOCKPICKING;
use crate::game::map::{Map, MapKey, skills};

const JUNKBOX: u32 = 9_252;
const PICK_LOCK: u32 = 1804;
const LOCK: u32 = 55;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.items.insert(
        JUNKBOX,
        ItemTemplate {
            entry: JUNKBOX,
            name: "Battered Junkbox".into(),
            allowable_class: -1,
            allowable_race: -1,
            stackable: 1,
            buy_count: 1,
            lock_id: LOCK,
            flags: 0x4,
            ..Default::default()
        },
    );
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 33,
        misc_value: 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        PICK_LOCK,
        SpellTemplate {
            id: PICK_LOCK,
            name: "Pick Lock".into(),
            effects,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    // A lockpicker's lock of no skill.
    d.crafts.locks.insert(
        LOCK,
        vec![crate::game::data::crafts::LockKey {
            kind: 2,
            index: 1,
            skill: 0,
        }],
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

#[tokio::test]
async fn a_lock_picked_in_a_trade_opens_the_partners_box_and_raises_the_rogues_skill() {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, data(), None);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Rogue).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    b.sees(ga).await;
    let (junkbox, at) = on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.spells.push(PICK_LOCK);
        skills::learn_skill(p, SKILL_LOCKPICKING, 1);
        skills::set_value(p, SKILL_LOCKPICKING, 1);
        crate::game::map::inventory::add_item(m, gb, JUNKBOX, 1, false).expect("room");
        let p = &m.players[&gb];
        let i = p.items.iter().rposition(|i| i.entry == JUNKBOX).unwrap();
        (p.items[i].guid, (p.items[i].bag, p.items[i].slot))
    })
    .await;
    a.send(CMSG_INITIATE_TRADE { guid: gb }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    b.send(CMSG_SET_TRADE_ITEM {
        trade_slot: 6,
        bag: at.0,
        slot: at.1,
    })
    .await;
    a.wait_for("Bob's box in the seventh slot", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS_EXTENDED(t) => {
            (t.trade_slots[6].item != 0).then_some(())
        }
        _ => None,
    })
    .await;
    a.send(CMSG_CAST_SPELL {
        spell: PICK_LOCK,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty().set_item(
                SpellCastTargets_SpellCastTargetFlags_Item::TradeItem {
                    trade_item: Guid::new(6),
                },
            ),
        },
    })
    .await;
    let before = on_map(&realm, move |m| {
        skills::value(&m.players[&ga], SKILL_LOCKPICKING)
    })
    .await;
    a.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    b.send(CMSG_ACCEPT_TRADE { unknown1: 0 }).await;
    a.wait_for("TRADE_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::TradeComplete).then_some(())
        }
        _ => None,
    })
    .await;
    let after = on_map(&realm, move |m| {
        let p = &m.players[&gb];
        (
            p.item_by_guid(junkbox)
                .map(|i| p.items[i].fields.get_u32(ITEM_FIELD_FLAGS) & 0x4),
            skills::value(&m.players[&ga], SKILL_LOCKPICKING),
        )
    })
    .await;
    // Nothing until the trade goes through; then the box is Bob's still,
    // open, and her skill a point up.
    assert_eq!(before, Some(1));
    assert_eq!(after, (Some(0x4), Some(2)));
}
