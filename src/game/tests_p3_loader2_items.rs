//! Phase 3's second loader stage, the items' spells (Thistle Tea's
//! `world/loader/equipment_spells_dbc_test.exs` and the item tests beside
//! it): what real gear gives while worn, over the real data when it is
//! here.

use std::sync::Arc;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::player::Item;
use crate::game::map::{Map, MapHandle, MapKey, auras, derive};
use crate::game::store::BAG_PLAYER;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    let data = realm.data.clone();
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        derive::unit(m, g);
    })
    .await;
    (c, g, map)
}

/// Put real item `entry` in `slot`, or take what is there off (`entry`
/// 0), and work the gear out again.
fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = m.players.get_mut(&g).expect("the player");
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    if entry != 0 {
        p.items.push(Item::new(
            &data,
            guid::item(counter),
            g,
            entry,
            BAG_PLAYER,
            slot,
            1,
        ));
    }
    p.refresh_inventory(&data);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
}

fn spells_on(m: &Map, g: Guid) -> Vec<u32> {
    let mut v: Vec<u32> =
        auras::auras(m, g).map_or(Vec::new(), |a| a.holders.iter().map(|h| h.spell).collect());
    v.sort();
    v
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

const NECK: u8 = 1;
const CHEST: u8 = 4;
const TRINKET1: u8 = 12;
const RANGED: u8 = 17;
const BAG1: u8 = 19;
const LAMINA: u32 = 18_714;
const MALGENS_LONG_BOW: u32 = 22_318;
const CHARGED_LIGHTNING_ROD: u32 = 11_860;
const SAVAGE_GLADIATOR_CHAIN: u32 = 11_726;
const BLACKHANDS_BREADTH: u32 = 13_965;
const VEREKS_COLLAR: u32 = 11_755;

/// Thistle Tea's quiver and its haste (`equipment_spells_dbc_test.exs`):
/// a quiver's 15 per cent shortens the bow's 2.9 seconds, not a wand's,
/// once a time however often the gear is worked out; it comes back with
/// the bow, and goes with the quiver.
#[tokio::test]
#[ignore = "extended"]
async fn real_quiver_haste_follows_the_ranged_weapon() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, hunter, map) = enter(&realm, "P3OQUIV", "Quiva", Race::Dwarf, Class::Hunter).await;
    let times = on(&map, move |m| {
        let time = |m: &Map| {
            m.players[&hunter]
                .fields
                .get_u32(UNIT_FIELD_RANGEDATTACKTIME)
        };
        equip(m, hunter, LAMINA, BAG1);
        equip(m, hunter, MALGENS_LONG_BOW, RANGED);
        let bow = time(m);
        derive::unit(m, hunter);
        let again = time(m);
        equip(m, hunter, CHARGED_LIGHTNING_ROD, RANGED);
        let wand = time(m);
        equip(m, hunter, MALGENS_LONG_BOW, RANGED);
        let back = time(m);
        equip(m, hunter, 0, BAG1);
        let bare = time(m);
        [bow, again, wand, back, bare]
    })
    .await;
    // Cut to the millisecond, as Thistle Tea's 3000 to 2608.
    let hasted = (2900.0 / 1.15f32) as u32;
    assert_eq!(times, [hasted, hasted, 1500, hasted, 2900]);
}

/// Thistle Tea's gear crit and dodge (`equipment_spells_dbc_test.exs`):
/// the Savage Gladiator Chain's and Blackhand's Breadth's 2 per cent of
/// crit, Verek's Collar's 1 of dodge, an aura apiece, and back after a
/// login; taken off, gone.
#[tokio::test]
#[ignore = "extended"]
async fn real_gear_crit_and_dodge_come_back_after_a_login() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, g, map) = enter(&realm, "P3OCRIT", "Crita", Race::Human, Class::Warrior).await;
    let read = move |m: &mut Map| {
        let f = &m.players[&g].fields;
        (
            f.get_f32(PLAYER_CRIT_PERCENTAGE),
            f.get_f32(PLAYER_DODGE_PERCENTAGE),
        )
    };
    let (base, worn, auras_worn) = on(&map, move |m| {
        let base = read(m);
        equip(m, g, SAVAGE_GLADIATOR_CHAIN, CHEST);
        equip(m, g, BLACKHANDS_BREADTH, TRINKET1);
        equip(m, g, VEREKS_COLLAR, NECK);
        let a = auras::auras(m, g).unwrap();
        let from_auras = (
            a.sum(auras::AURA_MOD_CRIT_PERCENT),
            a.sum(auras::AURA_MOD_DODGE_PERCENT),
        );
        (base, (read(m), from_auras), spells_on(m, g))
    })
    .await;
    let (worn, from_auras) = worn;
    let close = |a: f32, b: f32| (a - b).abs() < 0.01;
    // Their own agility too.
    assert_eq!(from_auras, (4, 1));
    assert!(
        worn.0 - base.0 >= 4.0 && worn.1 - base.1 >= 1.0,
        "{base:?} {worn:?}"
    );
    assert_eq!(auras_worn.iter().filter(|s| **s == 7598).count(), 2);
    assert!(auras_worn.contains(&13_669));
    relog(&mut c).await;
    let map = realm.registry.online(g).unwrap().map();
    // The gear's auras come back at the map's next gear step after the
    // login (`inventory::sync_gear_all`), not with it.
    for _ in 0..200 {
        if on(&map, move |m| spells_on(m, g).contains(&13_669)).await {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (restored, auras_back, removed) = on(&map, move |m| {
        derive::unit(m, g);
        let restored = read(m);
        let back = spells_on(m, g);
        for slot in [CHEST, TRINKET1, NECK] {
            equip(m, g, 0, slot);
        }
        (restored, back, read(m))
    })
    .await;
    assert!(
        close(restored.0, worn.0) && close(restored.1, worn.1),
        "{restored:?} after a login, {worn:?} worn"
    );
    assert_eq!(auras_back, auras_worn);
    assert!(
        close(removed.0, base.0) && close(removed.1, base.1),
        "{removed:?}"
    );
}

const TOOTH_OF_GNARR: u32 = 13_141;
const HAND_OF_JUSTICE: u32 = 11_815;
const BRIARWOOD_REED: u32 = 12_930;
const HEART_OF_WYRMTHALAK: u32 = 22_321;

/// Thistle Tea's other gear spells (`equipment_spells_dbc_test.exs`):
/// Tooth of Gnarr's mana comes in the five seconds after a cast, a point
/// a tick, and not without it; Hand of Justice's 20 attack power and its
/// extra attack's proc; Briarwood Reed's 29 spell damage and healing,
/// once; Heart of Wyrmthalak's proc of its flame at a swing a minute.
#[tokio::test]
#[ignore = "extended"]
async fn real_gear_spells_give_mana_power_and_procs() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "P3OGEAR", "Geara", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        let tick = |m: &mut Map| {
            let now = m.now;
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_u32(UNIT_FIELD_POWER1, 0);
            p.mana_spent_at = Some(now);
            p.regen_at = now;
            crate::game::map::stats::regen(m);
            m.players[&g].fields.get_u32(UNIT_FIELD_POWER1)
        };
        let bare = tick(m);
        equip(m, g, TOOTH_OF_GNARR, NECK);
        let tooth = tick(m);
        equip(m, g, 0, NECK);
        let ap = |m: &Map| {
            let f = &m.players[&g].fields;
            (
                f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS),
                f.get_u32(UNIT_FIELD_RANGED_ATTACK_POWER_MODS),
            )
        };
        let ap_bare = ap(m);
        equip(m, g, HAND_OF_JUSTICE, TRINKET1);
        let ap_hand = ap(m);
        let hand = spells_on(m, g);
        equip(m, g, BRIARWOOD_REED, TRINKET1);
        let data = m.realm.data.clone();
        let power = move |m: &Map| {
            let of = |id: u32, heal| {
                let s = data.spells.get(id).unwrap();
                crate::game::map::spells::outcome::power(m, g, g, s, heal).round() as u32
            };
            // Fireball's fire, and Flash Heal's healing.
            (
                m.players[&g]
                    .fields
                    .get_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_POS + 2),
                of(133, false),
                of(2061, true),
            )
        };
        let reed = power(m);
        derive::unit(m, g);
        let reed_again = power(m);
        equip(m, g, HEART_OF_WYRMTHALAK, TRINKET1);
        let heart = spells_on(m, g);
        (bare, tooth, ap_bare, ap_hand, hand, reed, reed_again, heart)
    })
    .await;
    let (bare, tooth, ap_bare, ap_hand, hand, reed, reed_again, heart) = out;
    assert_eq!((bare, tooth), (0, 1), "a point a tick, within the rule");
    assert_eq!(
        (ap_hand.0 - ap_bare.0, ap_hand.1 - ap_bare.1),
        (20, 20),
        "melee and ranged"
    );
    assert!(hand.contains(&15_600) && hand.contains(&9331), "{hand:?}");
    let d = &realm.data;
    let proc = d.spells.get(15_600).unwrap();
    assert_eq!(proc.effects[0].trigger_spell, 15_601);
    assert_eq!(reed, (29, 29, 29));
    assert_eq!(reed_again, reed, "once");
    assert!(heart.contains(&27_656), "{heart:?}");
    let pe = d.spells.proc_events[&27_656];
    assert_eq!((pe.school_mask, pe.ppm), (1, 1.0));
    // A minute's one swing: at 3 seconds, a 5 per cent chance.
    let chance = crate::game::map::procs::ppm_chance(pe.ppm, 3000);
    assert!((chance - 5.0).abs() < 0.01, "{chance}");
}
