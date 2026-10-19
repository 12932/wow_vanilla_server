//! `.gear`: a character in random gear of their class and level, over the
//! real items.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// What a geared character wears, slot by slot: the item's entry, and
/// whether they may use it (`inventory::can_use`: proficiency, skill at
/// its rank, level).
type Worn = Vec<(u8, u32, bool)>;

/// A level-55 `race` `class`, given `.gear`: what they wear, the spells
/// they know, their ammunition, and the reply.
async fn geared(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Worn, Vec<u32>, u32, String) {
    geared_with(realm, (account, name), race, class, 55, ".gear").await
}

/// A `race` `class` of `level`, given `command` (`.gear`, `.gear raid`).
async fn geared_with(
    realm: &Arc<Realm>,
    (account, name): (&str, &str),
    race: Race,
    class: Class,
    level: u8,
    command: &str,
) -> (Worn, Vec<u32>, u32, String) {
    let mut c = Client::connect(realm, account);
    let g: Guid = c.enter(name, race, class).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    c.say(&format!(".character level {level}")).await;
    c.system_message(&format!("Level {level}")).await;
    c.say(command).await;
    let reply = c.system_message("Equipped").await;
    let (worn, spells, ammo) = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = &m.players[&g];
        let worn = p
            .items
            .iter()
            .filter(|i| i.bag == BAG_PLAYER && i.slot < 19)
            .map(|i| {
                let usable = data
                    .item(i.entry)
                    .is_some_and(|t| crate::game::map::inventory::can_use(p, &data, t).is_ok());
                (i.slot, i.entry, usable)
            })
            .collect::<Vec<_>>();
        let ammo = p.fields.get_u32(crate::game::fields::PLAYER_AMMO_ID);
        let ammo = if ammo != 0 && p.carried_count_of(ammo) > 0 {
            ammo
        } else {
            0
        };
        (worn, p.spells.clone(), ammo)
    })
    .await;
    (worn, spells, ammo, reply)
}

/// The armour subclass of what is worn in `slot`.
fn subclass(realm: &Realm, worn: &Worn, slot: u8) -> Option<(u32, u32, u32)> {
    let (_, entry, _) = worn.iter().find(|(s, _, _)| *s == slot)?;
    let t = realm.data.item(*entry)?;
    Some((t.class, t.subclass, t.inventory_type))
}

/// Every slot `.gear` fills but the hands: head, neck, shoulders, chest,
/// waist, legs, feet, wrists, hands, both rings, both trinkets, back.
const BODY: [u8; 14] = [0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14];

fn full_set(realm: &Realm, worn: &Worn, armour: u32) {
    for slot in BODY.into_iter().chain([15, 17]) {
        assert!(
            worn.iter().any(|(s, _, _)| *s == slot),
            "slot {slot} is filled: {worn:?}"
        );
    }
    for (slot, entry, usable) in worn {
        assert!(usable, "the item {entry} in slot {slot} may be used");
        // The shirt and tabard are left as they were.
        if matches!(slot, 3 | 18) {
            continue;
        }
        let t = realm.data.item(*entry).unwrap();
        assert!(
            (2..=3).contains(&t.quality) && (45..=55).contains(&t.required_level),
            "the item {entry} in slot {slot} is uncommon or rare, of level 45 to 55"
        );
    }
    for slot in [0, 2, 4, 5, 6, 7, 8, 9] {
        let (class, sub, _) = subclass(realm, worn, slot).unwrap();
        assert_eq!((class, sub), (4, armour), "the armour in slot {slot}");
    }
    // A two-hander, or something in the off hand too.
    let (_, _, main) = subclass(realm, worn, 15).unwrap();
    assert!(
        main == 17 || worn.iter().any(|(s, _, _)| *s == 16),
        "a two-hander or an off hand: {worn:?}"
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn gear_dresses_a_level_55_warrior_mage_and_hunter_in_what_they_may_wear() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let (worn, _, _, reply) =
        geared(&realm, "GEARW", "Gearwar", Race::Undead, Class::Warrior).await;
    full_set(&realm, &worn, 4);
    assert!(
        reply.len() > 20,
        "the reply names what was equipped: {reply}"
    );

    let (worn, spells, _, _) = geared(&realm, "GEARM", "Gearmage", Race::Undead, Class::Mage).await;
    full_set(&realm, &worn, 1);
    assert_eq!(subclass(&realm, &worn, 17).map(|s| s.1), Some(19), "a wand");
    assert!(spells.contains(&5019), "Shoot");

    let (worn, spells, ammo, _) =
        geared(&realm, "GEARH", "Gearhunt", Race::Dwarf, Class::Hunter).await;
    full_set(&realm, &worn, 3);
    let (_, ranged, _) = subclass(&realm, &worn, 17).unwrap();
    assert!([2, 3, 18].contains(&ranged), "a bow, gun or crossbow");
    assert!(spells.contains(&75), "Auto Shot");
    let ammo = realm.data.item(ammo).expect("ammunition chosen");
    let weapon = worn.iter().find(|(s, _, _)| *s == 17).unwrap().1;
    assert_eq!(ammo.subclass, realm.data.item(weapon).unwrap().ammo_type);
}

/// `.gear raid`: a level-60 warrior in tier 2 (Battlegear of Wrath) from
/// head to hands, and epics in every other slot, all of it usable; a
/// level-60 priest in tier 1 or 2; a level-50 mage in the best there is
/// for them at 50, none of it a developer's.
#[tokio::test]
#[ignore = "extended"]
async fn gear_raid_dresses_a_raider_in_their_tier_set_and_epics() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let set_slots = [0, 2, 4, 5, 6, 7, 8, 9];
    let (worn, _, _, _) = geared_with(
        &realm,
        ("RAIDW", "Raidwar"),
        Race::Undead,
        Class::Warrior,
        60,
        ".gear raid 2",
    )
    .await;
    for slot in BODY.into_iter().chain([15, 17]) {
        assert!(worn.iter().any(|(s, _, _)| *s == slot), "slot {slot}");
    }
    for (slot, entry, usable) in &worn {
        if matches!(slot, 3 | 18) {
            continue;
        }
        let t = realm.data.item(*entry).unwrap();
        assert!(usable, "{} in slot {slot} may be used", t.name);
        if set_slots.contains(slot) {
            assert_eq!(t.set_id, 218, "{} in slot {slot} is tier 2's", t.name);
        } else {
            assert_eq!(t.quality, 4, "{} in slot {slot} is epic", t.name);
        }
    }

    let (worn, _, _, _) = geared_with(
        &realm,
        ("RAIDP", "Raidpriest"),
        Race::Undead,
        Class::Priest,
        60,
        ".gear raid",
    )
    .await;
    let sets: Vec<u32> = set_slots
        .iter()
        .map(|slot| {
            let (_, entry, _) = worn.iter().find(|(s, _, _)| s == slot).expect("worn");
            realm.data.item(*entry).unwrap().set_id
        })
        .collect();
    assert!(
        sets.iter().all(|s| *s == 202) || sets.iter().all(|s| *s == 211),
        "Prophecy or Transcendence: {sets:?}"
    );

    let (worn, _, _, reply) = geared_with(
        &realm,
        ("RAIDM", "Raidmage"),
        Race::Undead,
        Class::Mage,
        50,
        ".gear raid",
    )
    .await;
    assert!(reply.starts_with("Equipped"), "{reply}");
    let mut rare = 0;
    for (slot, entry, usable) in &worn {
        if matches!(slot, 3 | 18) {
            continue;
        }
        let t = realm.data.item(*entry).unwrap();
        assert!(
            *usable && t.required_level <= 50,
            "{} in slot {slot}",
            t.name
        );
        assert!(!crate::game::map::gm_dev::unreal(&t.name), "{}", t.name);
        rare += usize::from(t.quality >= 3);
    }
    assert!(rare >= 12, "rare or better nearly everywhere: {worn:?}");
}

/// What was worn is replaced: the starting weapon is gone, not put in
/// the bags, and a second `.gear` leaves one item a slot.
#[tokio::test]
#[ignore = "extended"]
async fn gear_replaces_what_was_worn() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "GEARR");
    let g: Guid = c.enter("Gearre", Race::Undead, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let start = on(&map, move |m| {
        let p = &m.players[&g];
        p.item_at(BAG_PLAYER, 15).map(|i| p.items[i].entry)
    })
    .await
    .expect("a starting weapon");
    c.say(".character level 55").await;
    c.system_message("Level 55").await;
    for _ in 0..2 {
        c.say(".gear").await;
        c.system_message("Equipped").await;
    }
    let (left, slots) = on(&map, move |m| {
        let p = &m.players[&g];
        let mut slots: Vec<u8> = p
            .items
            .iter()
            .filter(|i| i.bag == BAG_PLAYER && i.slot < 19)
            .map(|i| i.slot)
            .collect();
        slots.sort_unstable();
        (p.count_of(start), slots)
    })
    .await;
    assert_eq!(left, 0, "the starting weapon is gone");
    let mut unique = slots.clone();
    unique.dedup();
    assert_eq!(slots, unique, "one item a slot");
    assert!(
        slots.contains(&15) && slots.contains(&16),
        "two weapons: {slots:?}"
    );
}
