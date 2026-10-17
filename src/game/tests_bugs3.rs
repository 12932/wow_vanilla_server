//! Bugs found while mapping wvs's tests against Thistle Tea's, batch 3:
//! the world, visibility, spawns and the wire layouts mangos-zero gives
//! the 1.12 client.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

const SMSG_LOG_XPGAIN: u16 = 0x1D0;

/// A kill's experience names its victim and goes on with the experience
/// before the rested bonus and the group's rate; a quest's or an
/// explorer's ends at its type (mangos-zero's `SendLogXPGain`).
#[tokio::test]
async fn experience_for_a_kill_carries_its_rested_split() {
    let realm = harness::realm();
    let (mut a, g, map) = enter(&realm, "A", "Killer", Race::Human, Class::Warrior).await;
    let victim = guid::unit(299, 7);
    on(&map, move |m| {
        crate::game::map::stats::give_xp(m, g, 40, Some(victim));
    })
    .await;
    let kill = a
        .wait_frame("the kill's experience", SMSG_LOG_XPGAIN, |_| true)
        .await;
    assert_eq!(kill.len(), 21, "guid, total, type, base and rate");
    assert_eq!(&kill[..8], &victim.guid().to_le_bytes());
    assert_eq!(u32_at(&kill, 8), 40);
    assert_eq!(kill[12], 0, "a kill");
    assert_eq!(u32_at(&kill, 13), 40, "none of it rested");
    assert_eq!(f32::from_le_bytes(kill[17..21].try_into().unwrap()), 1.0);
    on(&map, move |m| {
        crate::game::map::stats::give_xp(m, g, 25, None)
    })
    .await;
    let quest = a
        .wait_frame("the quest's experience", SMSG_LOG_XPGAIN, |_| true)
        .await;
    assert_eq!(quest.len(), 13, "guid, total and type");
    assert_eq!(u32_at(&quest, 8), 25);
    assert_eq!(quest[12], 1, "not a kill");
}

const RES: u32 = 900_300;
const REBIRTH: u32 = 900_301;
const SMSG_RESURRECT_REQUEST: u16 = 0x15B;

fn unit_spell(id: u32, effect: u32) -> spells::SpellTemplate {
    let mut s = spells::SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = spells::SpellEffect {
        effect,
        target_a: 25,
        base_points: 34,
        ..Default::default()
    };
    s
}

/// A resurrection's offer is laid out as mangos-zero's: no name for a
/// player caster, not a spirit healer, and whether the client holds the
/// acceptance back by its timer, which Rebirth's attribute skips.
#[tokio::test]
async fn a_resurrection_offer_says_whether_it_waits_on_the_timer() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    d.spells.spells.insert(RES, unit_spell(RES, 18));
    let mut rebirth = unit_spell(REBIRTH, 18);
    rebirth.attributes[3] = spells::ATTR_EX3_NO_RES_TIMER;
    d.spells.spells.insert(REBIRTH, rebirth);
    let realm = harness::realm_with(d);
    let (_a, priest, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, fallen, _) = enter(&realm, "B", "Fallen", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, priest, fallen, 1_000_000);
        crate::game::map::spells::cast_triggered(m, priest, RES, Some(fallen));
    })
    .await;
    let offer = b
        .wait_frame("the offer", SMSG_RESURRECT_REQUEST, |_| true)
        .await;
    let mut want = priest.guid().to_le_bytes().to_vec();
    want.extend([1, 0, 0, 0, 0]);
    want.extend([0, 1]);
    assert_eq!(offer, want, "no name, not a spirit healer, on the timer");
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, priest, REBIRTH, Some(fallen));
    })
    .await;
    let offer = b
        .wait_frame("Rebirth's offer", SMSG_RESURRECT_REQUEST, |_| true)
        .await;
    assert_eq!(offer[offer.len() - 2..], [0, 0], "at once");
}

const SHIELD_BASH: u32 = 900_310;
const SMSG_CAST_RESULT: u16 = 0x130;

/// A cast refused for the gear it wants names the gear: its class, its
/// subclasses and its inventory types, as mangos-zero sends them and
/// wow_messages reads them.
#[tokio::test]
async fn a_cast_refused_for_its_gear_names_the_gear() {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut bash = unit_spell(SHIELD_BASH, 2);
    bash.effects[0].target_a = 1;
    bash.equipped_item_class = 4;
    bash.equipped_item_subclass_mask = 1 << 6;
    bash.equipped_item_inventory_type_mask = 1 << 14;
    d.spells.spells.insert(SHIELD_BASH, bash);
    let realm = harness::realm_with(d);
    let (mut a, g, map) = enter(&realm, "A", "Bare", Race::Human, Class::Warrior).await;
    on(&map, move |m| {
        crate::game::map::spells::learn(m, g, SHIELD_BASH)
    })
    .await;
    a.send(CMSG_CAST_SPELL {
        spell: SHIELD_BASH,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
        },
    })
    .await;
    let refused = a
        .wait_frame("the refusal", SMSG_CAST_RESULT, |b| b.len() > 5)
        .await;
    let mut want = SHIELD_BASH.to_le_bytes().to_vec();
    want.extend([2, crate::game::map::spells::fail::EQUIPPED_ITEM_CLASS]);
    want.extend(4u32.to_le_bytes());
    want.extend((1u32 << 6).to_le_bytes());
    want.extend((1u32 << 14).to_le_bytes());
    assert_eq!(refused, want, "a shield, in the off hand");
    let parsed = a
        .wait_for("the refusal, read", |m| match m {
            ServerOpcodeMessage::SMSG_CAST_RESULT(r) => Some(r.clone()),
            _ => None,
        })
        .await;
    assert!(matches!(
        parsed.result,
        SMSG_CAST_RESULT_SimpleSpellCastResult::Failure { .. }
    ));
}

/// A spawn's looks in an event are its latest patch's, whatever order
/// the rows are stored in.
#[test]
fn an_event_creatures_looks_are_its_latest_patchs() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE game_event_creature_data (guid, patch, entry_id, display_id, \
         equipment_id, spell_start, spell_end, event);
         INSERT INTO game_event_creature_data VALUES (1, 7, 0, 200, 0, 0, 0, 5);
         INSERT INTO game_event_creature_data VALUES (1, 3, 0, 100, 0, 0, 0, 5);",
    )
    .unwrap();
    let mut d = crate::game::data::world::WorldData::default();
    crate::game::data::world::load(&db, None, &mut d);
    let looks = &d.event_creatures[&1];
    assert_eq!(looks.len(), 1);
    assert_eq!(looks[0].display, 200, "patch 7's, not patch 3's");
}
