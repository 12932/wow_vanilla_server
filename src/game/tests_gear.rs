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

/// Over the real items, `.gear` at 28 (where a battleground's rings are of
/// one name at 18 and 28, and the unique rings and trinkets about the
/// level are many) never puts two of one unique item, nor of one unique
/// name, on; and `.gear enchant` puts Crusader on the weapon worn.
#[tokio::test]
#[ignore = "extended"]
async fn gear_puts_on_one_of_each_unique_item_and_enchant_puts_crusader_on() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "GEARU");
    let g: Guid = c.enter("Gearuniq", Race::Human, Class::Priest).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    c.say(".character level 28").await;
    c.system_message("Level 28").await;
    for _ in 0..10 {
        c.say(".gear").await;
        c.system_message("Equipped").await;
        let worn: Vec<(String, u32)> = on(&map, move |m| {
            let data = m.realm.data.clone();
            let p = &m.players[&g];
            p.items
                .iter()
                .filter(|i| i.bag == BAG_PLAYER && i.slot < 19)
                .filter_map(|i| data.item(i.entry))
                .filter(|t| t.max_count > 0)
                .map(|t| (t.name.clone(), t.entry))
                .collect()
        })
        .await;
        let mut names: Vec<&String> = worn.iter().map(|(n, _)| n).collect();
        names.sort();
        let all = names.len();
        names.dedup();
        assert_eq!(names.len(), all, "a unique one twice: {worn:?}");
    }
    c.say(".gear enchant mainhand 1900").await;
    let reply = c.system_message("Enchanted").await;
    assert!(reply.ends_with("with Crusader."), "{reply}");
    let on_it = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_at(BAG_PLAYER, 15).expect("a weapon");
        crate::game::map::enchant::get(&p.items[i], crate::game::map::enchant::PERM_SLOT).id
    })
    .await;
    assert_eq!(on_it, 1900);
}

/// `.pet random`: a level 30 hunter's pets drawn from many beast families,
/// not a cat or a wolf each time, every one tameable and of their level.
#[test]
#[ignore = "extended"]
fn pet_random_draws_many_families_of_the_hunters_level() {
    if !have_data() {
        return;
    }
    let data = harness::load_real(DB.as_path());
    let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(7);
    let mut families = std::collections::BTreeSet::new();
    for _ in 0..200 {
        let entry = crate::game::map::pets::random_beast_for(&data, 30, &mut rng).expect("a beast");
        let t = data.creature_template(entry).unwrap();
        assert!(
            t.level_min <= 30 && 30 <= t.level_max,
            "{} of level 30",
            t.name
        );
        families.insert(t.family);
    }
    assert!(families.len() >= 8, "families: {families:?}");
}

/// `.gear green` and `.gear blue` at 60, over the real items: a warrior
/// and a priest in uncommon items, and in rare ones, in every body slot,
/// all of it usable and of level 50 to 60; few slots short of the quality
/// (filled from both).
#[tokio::test]
#[ignore = "extended"]
async fn gear_green_and_blue_dress_a_level_60_in_one_quality() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    for (k, (race, class)) in [(Race::Undead, Class::Warrior), (Race::Human, Class::Priest)]
        .into_iter()
        .enumerate()
    {
        for (q, command) in [(2, ".gear green"), (3, ".gear blue")] {
            let account = format!("QUAL{k}{q}");
            let name = format!("Qual{}{}", ["a", "b"][k], ["x", "y", "z", "w"][q as usize]);
            let (worn, ..) = geared_with(&realm, (&account, &name), race, class, 60, command).await;
            for slot in BODY.into_iter().chain([15]) {
                assert!(
                    worn.iter().any(|(s, _, _)| *s == slot),
                    "{command}: slot {slot} is filled: {worn:?}"
                );
            }
            let mut other = 0;
            for (slot, entry, usable) in &worn {
                if matches!(slot, 3 | 18) {
                    continue;
                }
                assert!(
                    usable,
                    "{command}: the item {entry} in slot {slot} may be used"
                );
                let t = realm.data.item(*entry).unwrap();
                assert!(
                    (2..=3).contains(&t.quality) && (50..=60).contains(&t.required_level),
                    "{command}: the item {entry} in slot {slot}"
                );
                other += u32::from(t.quality != q);
            }
            assert!(
                other <= 2,
                "{command}: {other} slots of the other quality: {worn:?}"
            );
        }
    }
}

/// A hunter's quiver or pouch and what is in it, after a `.gear`: the
/// ranged weapon's ammo type, the bag's slot and entry, how many of the
/// chosen ammunition are in it and carried in all, and its slots × stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Quiver {
    ammo_type: u32,
    slot: u8,
    bag: u32,
    ammo: u32,
    inside: u32,
    carried: u32,
    full: u32,
}

fn quiver_of(m: &Map, g: Guid) -> Option<Quiver> {
    let data = m.realm.data.clone();
    let p = &m.players[&g];
    let weapon = p.item_at(BAG_PLAYER, 17).map(|i| p.items[i].entry)?;
    let ammo_type = data.item(weapon)?.ammo_type;
    let (slot, bag) = (19..23).find_map(|s| {
        let i = p.item_at(BAG_PLAYER, s)?;
        let t = data.item(p.items[i].entry)?;
        (t.class == 11).then_some((s, t.entry))
    })?;
    let ammo = p.fields.get_u32(crate::game::fields::PLAYER_AMMO_ID);
    let inside = p
        .items
        .iter()
        .filter(|i| i.bag == slot && i.entry == ammo)
        .map(|i| i.count())
        .sum();
    let stack = data.item(ammo).map_or(0, |t| t.stackable.max(1));
    Some(Quiver {
        ammo_type,
        slot,
        bag,
        ammo,
        inside,
        carried: p.carried_count_of(ammo),
        full: data.item(bag)?.container_slots * stack,
    })
}

/// A hunter of `level`, `.gear`ed `times` times (`command`): after each,
/// its quiver and the reply.
async fn geared_hunter(
    realm: &Arc<Realm>,
    (account, name): (&str, &str),
    level: u8,
    command: &str,
    times: usize,
) -> (Guid, Vec<(Option<Quiver>, String)>) {
    let mut c = Client::connect(realm, account);
    let g: Guid = c.enter(name, Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    c.say(&format!(".character level {level}")).await;
    c.system_message(&format!("Level {level}")).await;
    let mut out = Vec::new();
    for _ in 0..times {
        c.say(command).await;
        let reply = c.system_message("Equipped").await;
        out.push((on(&map, move |m| quiver_of(m, g)).await, reply));
    }
    (g, out)
}

/// `.gear` gives a hunter the best quiver (a bow or crossbow) or ammo
/// pouch (a gun) of its level on the bag bar, full of its best
/// ammunition, slots × stack, the ammunition chosen; at 40 the Quickdraw
/// Quiver or Thick Leather Ammo Pouch, 3,200 Explosive Arrows (an
/// engineer's) or Accurate Slugs. The bag's kind follows the weapon's from one `.gear` to the
/// next, and a `.gear` with the weapon's kind unchanged gives nothing more.
#[tokio::test]
#[ignore = "extended"]
async fn gear_fills_a_level_40_hunters_quiver_or_pouch() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let (_, runs) = geared_hunter(&realm, ("QUIV40", "Quivforty"), 40, ".gear", 12).await;
    let mut kinds = std::collections::BTreeSet::new();
    let mut last: Option<Quiver> = None;
    for (q, reply) in &runs {
        let q = q.expect("a quiver or pouch");
        let bag = realm.data.item(q.bag).unwrap();
        let ammo = realm.data.item(q.ammo).unwrap();
        let (want_bag, want_sub, want_ammo) = if q.ammo_type == 2 {
            ("Quickdraw Quiver", 2, "Explosive Arrow")
        } else {
            ("Thick Leather Ammo Pouch", 3, "Accurate Slugs")
        };
        assert_eq!(
            (bag.name.as_str(), bag.subclass),
            (want_bag, want_sub),
            "{reply}"
        );
        assert_eq!(ammo.name, want_ammo);
        assert_eq!(ammo.subclass, q.ammo_type);
        assert_eq!(
            (q.full, q.inside, q.carried),
            (3200, 3200, 3200),
            "full, none loose: {q:?}"
        );
        match last {
            Some(l) if l.ammo_type == q.ammo_type => {
                assert_eq!((l.slot, l.bag), (q.slot, q.bag), "the same bag kept");
                assert!(
                    !reply.contains(want_bag) && !reply.contains(want_ammo),
                    "nothing more given: {reply}"
                );
            }
            _ => assert!(
                reply.contains(&format!("{want_bag}, 3,200 {want_ammo}")),
                "the reply names the bag and its ammunition: {reply}"
            ),
        }
        kinds.insert(q.ammo_type);
        last = Some(q);
    }
    eprintln!("ammo types seen: {kinds:?}");
}

/// At 60, the best a hunter may have of its level: the Harpy Hide Quiver
/// or Gnoll Skin Bandolier (16 slots, 15% haste: rare); with `.gear raid`
/// a bow's Ancient Sinew Wrapped Lamina (18 slots, epic), 3,600 arrows.
/// And the quiver's haste is on the hunter: its ranged attack is its
/// weapon's delay less 15 per cent.
#[tokio::test]
#[ignore = "extended"]
async fn gear_gives_a_level_60_hunter_the_best_quiver_and_its_haste() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let (g, runs) = geared_hunter(&realm, ("QUIV60", "Quivsixty"), 60, ".gear", 1).await;
    let q = runs[0].0.expect("a quiver or pouch");
    let name = &realm.data.item(q.bag).unwrap().name;
    let want = if q.ammo_type == 2 {
        "Harpy Hide Quiver"
    } else {
        "Gnoll Skin Bandolier"
    };
    assert_eq!(name, want);
    assert_eq!((q.inside, q.full), (3200, 3200));
    let ammo = &realm.data.item(q.ammo).unwrap().name;
    let want = if q.ammo_type == 2 {
        "Explosive Arrow"
    } else {
        "Thorium Shells"
    };
    assert_eq!(ammo, want, "a vendor's or a craft's, not a drop");
    // The haste, at the map's next gear step.
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (mut time, mut delay) = (0, 0);
    for _ in 0..100 {
        (time, delay) = on(&map, move |m| {
            let p = &m.players[&g];
            let w = p.item_at(BAG_PLAYER, 17).map(|i| p.items[i].entry).unwrap();
            (
                p.fields
                    .get_u32(crate::game::fields::UNIT_FIELD_RANGEDATTACKTIME),
                m.realm.data.item(w).unwrap().delay,
            )
        })
        .await;
        if time < delay {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // To the millisecond (the haste a multiplier in f32: 2300 to 1999).
    let hasted = (delay as f32 / 1.15) as u32;
    assert!(
        time.abs_diff(hasted) <= 1,
        "15% ranged haste on {delay}: {time}, not {hasted}"
    );

    // A raider with a bow: the Lamina. The weapon's kind is the random
    // pick's, so a few tries.
    let (_, runs) = geared_hunter(&realm, ("QUIVR", "Quivraid"), 60, ".gear raid", 8).await;
    let bows: Vec<Quiver> = runs
        .iter()
        .filter_map(|(q, _)| *q)
        .filter(|q| q.ammo_type == 2)
        .collect();
    for q in &bows {
        assert_eq!(
            realm.data.item(q.bag).unwrap().name,
            "Ancient Sinew Wrapped Lamina"
        );
        assert_eq!(realm.data.item(q.ammo).unwrap().name, "Doomshot");
        assert_eq!((q.inside, q.full), (3600, 3600));
    }
    for (q, _) in &runs {
        let q = q.expect("a quiver or pouch");
        if q.ammo_type == 3 {
            assert_eq!(realm.data.item(q.bag).unwrap().name, "Gnoll Skin Bandolier");
            assert_eq!(
                realm.data.item(q.ammo).unwrap().name,
                "Miniature Cannon Balls"
            );
        }
    }
}

/// With all four bag slots holding 16-slot bags, the first is replaced by
/// the quiver and its items moved to the backpack; with no room for
/// them, the bags are kept and the reply says so, the ammunition loose.
#[tokio::test]
#[ignore = "extended"]
async fn gear_puts_a_quiver_in_place_of_a_bag_and_moves_its_items() {
    if !have_data() {
        return;
    }
    const TRAVELERS_BACKPACK: u32 = 4500;
    const LINEN: u32 = 2589;
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    assert_eq!(
        realm.data.item(TRAVELERS_BACKPACK).unwrap().container_slots,
        16
    );
    for (k, crowded) in [false, true].into_iter().enumerate() {
        let mut c = Client::connect(&realm, &format!("QUIVB{k}"));
        let g: Guid = c
            .enter(
                &format!("Quivbag{}", ["a", "b"][k]),
                Race::Dwarf,
                Class::Hunter,
            )
            .await;
        let map = realm.maps.get(&realm, MapKey::world(0));
        c.say(".character level 40").await;
        c.system_message("Level 40").await;
        let linen = on(&map, move |m| {
            use crate::game::map::inventory::{self, Destination};
            let p = &m.players[&g];
            // The bag bar cleared of the starting pouch and what is in it.
            let gone: Vec<Guid> = p
                .items
                .iter()
                .filter(|i| {
                    (19..23).contains(&i.bag) || (i.bag == BAG_PLAYER && (19..23).contains(&i.slot))
                })
                .map(|i| i.guid)
                .collect();
            for item in gone {
                let i = m.players[&g].item_by_guid(item).unwrap();
                let n = m.players[&g].items[i].count();
                inventory::take_item(m, g, i, n);
            }
            for s in 19..23 {
                inventory::add_item_to(m, g, TRAVELERS_BACKPACK, 1, Destination::Worn(s)).unwrap();
            }
            // Linen in the first bag; with `crowded`, everywhere.
            inventory::add_item_to(m, g, LINEN, 60, Destination::Bag(19, None)).unwrap();
            if crowded {
                let p = &m.players[&g];
                let n = inventory::fitting(p, &m.realm.data, LINEN, 100_000).unwrap();
                inventory::add_item(m, g, LINEN, n, false).unwrap();
            }
            m.players[&g].count_of(LINEN)
        })
        .await;
        c.say(".gear").await;
        let reply = c.system_message("Equipped").await;
        let (q, bags, linen_after, in_pack) = on(&map, move |m| {
            let p = &m.players[&g];
            let bags: Vec<u32> = (19..23)
                .filter_map(|s| p.item_at(BAG_PLAYER, s).map(|i| p.items[i].entry))
                .collect();
            let in_pack = p
                .items
                .iter()
                .filter(|i| i.entry == LINEN && i.bag == BAG_PLAYER)
                .count();
            (quiver_of(m, g), bags, p.count_of(LINEN), in_pack)
        })
        .await;
        assert_eq!(linen_after, linen, "no linen lost: {reply}");
        if !crowded {
            let q = q.expect("a quiver in a bag's place");
            assert_eq!(q.slot, 19, "the first of the smallest");
            assert_eq!(bags.iter().filter(|b| **b == TRAVELERS_BACKPACK).count(), 3);
            assert_eq!(in_pack, 3, "the bag's three stacks in the backpack");
            assert_eq!((q.inside, q.full), (3200, 3200));
            assert!(!reply.contains("No room"), "{reply}");
        } else {
            assert!(q.is_none(), "no quiver: {bags:?}");
            assert_eq!(bags, [TRAVELERS_BACKPACK; 4]);
            assert!(
                reply.contains("No room for the Traveler's Backpack's items, so no"),
                "{reply}"
            );
        }
    }
}

/// The ammunition `.gear` picks, over the real items: by the damage it
/// adds, among what anyone may buy or make (`.gear`) or get any way but a
/// standing (`.gear raid`: a boss's drop, a quest's reward too). At 60
/// nothing of those beats the pick; `.gear`'s at 60 is a craft's
/// (Explosive Arrows, Thorium Shells), `.gear raid`'s the dungeon bosses'
/// Doomshot and Miniature Cannon Balls; the Alterac Valley
/// quartermasters' Ice Threaded ammunition, behind Honored, neither's.
#[test]
#[ignore = "extended"]
fn gear_picks_ammunition_by_its_damage_and_where_it_comes_from() {
    use crate::game::data::ammo_source::*;
    use crate::game::map::gm_dev::best_ammo;
    if !have_data() {
        return;
    }
    let data = harness::load_real(DB.as_path());
    let name =
        |t: Option<&crate::game::data::ItemTemplate>| t.map_or("-".into(), |t| t.name.clone());
    let dps = |t: &crate::game::data::ItemTemplate| t.damages[0].0 + t.damages[0].1;
    for level in [1, 10, 25, 40, 51, 60] {
        for ammo in [2, 3] {
            eprintln!(
                "{level} {}: .gear {}, .gear raid {}",
                ["", "", "arrows", "bullets"][ammo as usize],
                name(best_ammo(&data, ammo, level, false)),
                name(best_ammo(&data, ammo, level, true)),
            );
        }
    }
    let at = |ammo, raid| best_ammo(&data, ammo, 60, raid).unwrap().name.clone();
    assert_eq!(at(2, false), "Explosive Arrow");
    assert_eq!(at(3, false), "Thorium Shells");
    assert_eq!(at(2, true), "Doomshot");
    assert_eq!(at(3, true), "Miniature Cannon Balls");
    assert_eq!(best_ammo(&data, 2, 1, false).unwrap().name, "Rough Arrow");
    assert_eq!(best_ammo(&data, 3, 10, false).unwrap().name, "Heavy Shot");
    assert_eq!(best_ammo(&data, 2, 25, false).unwrap().name, "Razor Arrow");
    assert_eq!(
        best_ammo(&data, 3, 40, false).unwrap().name,
        "Accurate Slugs"
    );
    assert_eq!(
        best_ammo(&data, 3, 51, true).unwrap().name,
        "Rockshard Pellets"
    );
    for raid in [false, true] {
        let from = if raid {
            VENDOR | CRAFTED | DROP | QUEST
        } else {
            VENDOR | CRAFTED
        };
        for ammo in [2, 3] {
            let pick = best_ammo(&data, ammo, 60, raid).unwrap();
            assert_ne!(data.ammo_source(pick.entry) & from, 0);
            let better: Vec<&str> = data
                .items
                .values()
                .filter(|t| t.class == 6 && t.subclass == ammo && t.quality <= 3)
                .filter(|t| t.required_level <= 60 && t.required_reputation_faction == 0)
                .filter(|t| t.flags & 0x10 == 0 && !crate::game::map::gm_dev::unreal(&t.name))
                .filter(|t| data.ammo_source(t.entry) & from != 0)
                .filter(|t| dps(t) > dps(pick))
                .map(|t| t.name.as_str())
                .collect();
            assert!(
                better.is_empty(),
                "better than {} (raid {raid}): {better:?}",
                pick.name
            );
        }
    }
    for entry in [19_316, 19_317] {
        assert_eq!(
            data.ammo_source(entry),
            GATED_VENDOR,
            "Ice Threaded: Honored only"
        );
    }
}

/// What a hunter's client is sent as `.gear` puts a quiver or pouch on and
/// fills it (its create, its ammunition's, a quiver swapped for a pouch
/// and a bag replaced, the stacks moved), through the 1.12 client's own
/// parsers.
#[tokio::test]
#[ignore = "extended"]
async fn a_hunters_quiver_and_its_ammunition_pass_the_client() {
    if !have_data() {
        return;
    }
    let (check_py, wow_exe) =
        crate::game::tests_client_emu::checker().expect("tools/client_check/WoW.exe and `uv`");
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "QUIVC");
    let g: Guid = c.enter("Quivcheck", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    c.say(".character level 40").await;
    c.system_message("Level 40").await;
    for _ in 0..4 {
        c.say(".gear").await;
        c.system_message("Equipped").await;
    }
    // Four bags, linen in the first: the quiver in its place.
    on(&map, move |m| {
        use crate::game::map::inventory::{self, Destination};
        while let Some(i) = m.players[&g].items.iter().position(|i| {
            (19..23).contains(&i.bag) || (i.bag == BAG_PLAYER && (19..23).contains(&i.slot))
        }) {
            let n = m.players[&g].items[i].count();
            inventory::take_item(m, g, i, n);
        }
        for s in 19..23 {
            inventory::add_item_to(m, g, 4500, 1, Destination::Worn(s)).unwrap();
        }
        inventory::add_item_to(m, g, 2589, 60, Destination::Bag(19, None)).unwrap();
    })
    .await;
    c.say(".gear").await;
    c.system_message("Equipped").await;
    // The changes flushed: an update or two of the map.
    let start = on(&map, |m| m.stats.updates).await;
    while on(&map, |m| m.stats.updates).await < start + 3 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    c.say(".gear").await;
    c.system_message("Equipped").await;
    let (capture, counts) = crate::game::tests_client_emu::capture(vec![&mut c]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "updates captured"
    );
    crate::game::tests_client_emu::run_checker(&check_py, &wow_exe, &capture);
}
