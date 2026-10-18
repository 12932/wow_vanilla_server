//! Phase 3's second loader stage, the summons (Thistle Tea's
//! `world/loader/summon_test.exs` and `summon_owner_test.exs`): a tamed
//! beast, a pet put back from its record, a demon's name, and what a
//! summon's owner gives it, over the real data when it is here and on a
//! world of the test's own.

use std::sync::Arc;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::pets::{self, Kind};
use crate::game::map::{Map, MapHandle, MapKey};

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

/// A dwarf (a gnome warlock) of `class` at `level` in the Eastern Kingdoms, and the map.
async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    class: Class,
    level: u8,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let race = if class == Class::Warlock {
        Race::Gnome
    } else {
        Race::Dwarf
    };
    let g = c.enter(name, race, class).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    let data = realm.data.clone();
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, level);
    })
    .await;
    (c, g, map)
}

/// A wild creature of `entry` at `level` beside `g`.
fn wild(m: &mut Map, g: Guid, entry: u32, level: u8) -> Guid {
    let t = m.realm.data.creature_template(entry).cloned().unwrap();
    let at = m.players[&g].position();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &t,
        level,
        Vector3d {
            x: at.x + 2.0,
            ..at
        },
        0.0,
    )
    .with_base();
    m.add_creature(c)
}

fn pet_of(m: &Map, g: Guid) -> Guid {
    m.players[&g].owned.pet.expect("a pet")
}

const PRAIRIE_STALKER: u32 = 2960;
const TAME_BEAST: u32 = 1515;
const IMP: u32 = 416;
const SUMMON_IMP: u32 = 688;

/// Thistle Tea's tamed beast and its record (`summon_test.exs`): a
/// tamed beast knows its list's spells at its rank, owes the points they
/// cost (its loyalty brings none yet), and may be renamed; put back from a
/// record it has the record's level, experience, spells, loyalty, points,
/// health and stance, and its name and the rename it has had.
#[tokio::test]
#[ignore = "extended"]
async fn real_tamed_beasts_owe_their_spells_and_come_back_as_recorded() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, hunter, map) = enter(&realm, "P3OTAME", "Tamea", Class::Hunter, 50).await;
    let out = on(&map, move |m| {
        let beast = wild(m, hunter, PRAIRIE_STALKER, 12);
        pets::testing::tame(m, hunter, beast, TAME_BEAST);
        let pet = pet_of(m, hunter);
        let s = pets::state(m, pet).unwrap();
        let fresh = (
            s.spells.clone(),
            s.training_points,
            m.creature(pet).unwrap().fields.get_u32(UNIT_FIELD_FLAGS)
                & pets::testing::UNIT_FLAG_PET_RENAME,
        );
        let mut r = pets::testing::record_of(m, pet).unwrap();
        pets::unsummon(m, pet, true);
        r.level = 50;
        r.xp = 123;
        r.spells = vec![2649];
        r.loyalty = 4;
        r.loyalty_points = 12_345;
        r.training_points = 45;
        r.react = pets::REACT_PASSIVE;
        r.health = 777;
        r.number = 707;
        r.name = "Fang".into();
        r.renamed = true;
        let back = pets::testing::from_record(m, hunter, &r).unwrap();
        let c = m.creature(back).unwrap();
        let s = c.pet.as_ref().unwrap();
        let f = &c.fields;
        let restored = (
            f.get_u32(UNIT_FIELD_LEVEL),
            f.get_u32(UNIT_FIELD_PETEXPERIENCE),
            f.get_u32(UNIT_FIELD_PETNEXTLEVELEXP),
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_PETNUMBER),
            f.get_byte(UNIT_FIELD_BYTES_1, 1),
            f.get_u32(UNIT_FIELD_FLAGS) & pets::testing::UNIT_FLAG_PET_RENAME,
            f.get_u32(UNIT_TRAINING_POINTS),
        );
        let state = (
            s.spells.clone(),
            s.react,
            s.loyalty_points,
            s.training_points,
            s.name.clone(),
        );
        (fresh, restored, state)
    })
    .await;
    let (fresh, restored, state) = out;
    let d = &realm.data;
    let cost = |s: u32| {
        d.pets
            .abilities
            .values()
            .flatten()
            .find(|a| a.spell == s)
            .map_or(0, |a| a.cost as i32)
    };
    let owed: i32 = fresh.0.iter().map(|s| cost(*s)).sum();
    assert!(owed > 0, "{fresh:?}");
    assert_eq!(fresh.1, -owed, "owed");
    assert_ne!(fresh.2, 0, "may be renamed");
    assert_eq!(
        restored,
        (
            50,
            123,
            pets::testing::pet_xp_to_level(d, 50),
            777,
            707,
            4,
            0,
            (-46i32) as u32
        ),
        "as recorded, renamed; 45 points shown as -46, as vmangos's GetDispTP"
    );
    assert_eq!(
        state,
        (
            vec![2649],
            pets::REACT_PASSIVE,
            12_345,
            45,
            "Fang".to_string()
        )
    );
}

/// Thistle Tea's demon names (`summon_test.exs`): an imp draws its name
/// once and keeps it when summoned again; no imp may be renamed.
#[tokio::test]
#[ignore = "extended"]
async fn real_demons_keep_the_name_they_drew() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, warlock, map) = enter(&realm, "P3ODEMN", "Demona", Class::Warlock, 50).await;
    let out = on(&map, move |m| {
        let mut names = Vec::new();
        for _ in 0..2 {
            pets::testing::summon_demon(m, warlock, IMP, SUMMON_IMP);
            let imp = pet_of(m, warlock);
            names.push((
                pets::state(m, imp).unwrap().name.clone(),
                m.creature(imp).unwrap().fields.get_u32(UNIT_FIELD_FLAGS)
                    & pets::testing::UNIT_FLAG_PET_RENAME,
            ));
            pets::dismiss(m, imp);
        }
        names
    })
    .await;
    assert!(!out[0].0.is_empty() && out[0].0 != "Imp", "{out:?}");
    assert_eq!(out[0], out[1], "kept");
    assert_eq!(out[0].1, 0, "no rename");
}

/// Thistle Tea's owner stamps (`summon_owner_test.exs`): a player's
/// guardian is summoned and created by them, under a player's control,
/// with its template's flags beside; a creature's has its owner's guids
/// and its template's flags alone.
#[tokio::test]
async fn a_summons_owner_is_stamped_and_only_a_players_controls_it() {
    let mut d = GameData::empty();
    let entry = 90_100;
    d.creature_templates.insert(
        entry,
        CreatureTemplate {
            entry,
            name: "Helper".into(),
            display_ids: [262, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            health_max: 100,
            base_attack_time: 2000,
            unit_class: 1,
            unit_flags: 0x1000,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut c = Client::connect(&realm, "P3OOWNR");
    let g = c.enter("Ownera", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let at = m.players[&g].position();
        let stamp = |m: &Map, s: Guid| {
            let f = &m.creature(s).unwrap().fields;
            (
                f.get_guid(UNIT_FIELD_SUMMONEDBY),
                f.get_guid(UNIT_FIELD_CREATEDBY),
                f.get_u32(UNIT_FIELD_FLAGS),
            )
        };
        let mine = pets::testing::spawn(m, g, entry, Kind::Guardian, 1, at, 0.0);
        let creature = guid::unit(entry, 0x7F_0001);
        let t = m.realm.data.creature_template(entry).cloned().unwrap();
        let mut owner = Creature::new(creature, &t, 1, at, 0.0);
        owner.fields.set_u32(UNIT_FIELD_FLAGS, 0);
        m.add_creature(owner.with_base());
        let theirs = pets::testing::spawn(m, creature, entry, Kind::Guardian, 1, at, 0.0);
        (stamp(m, mine), stamp(m, theirs), creature)
    })
    .await;
    let (mine, theirs, creature) = out;
    let controlled = pets::UNIT_FLAG_PLAYER_CONTROLLED;
    assert_eq!((mine.0, mine.1), (g, g));
    assert_eq!(mine.2, controlled | 0x1000, "controlled, its own kept");
    assert_eq!((theirs.0, theirs.1, theirs.2), (creature, creature, 0x1000));
}
