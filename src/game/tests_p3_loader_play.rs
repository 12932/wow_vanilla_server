//! Phase 3's loader tests played out (Thistle Tea's `world/loader` tests
//! of the real spells): what the real spells do once cast, on the real
//! data when it is here.

use std::sync::Arc;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapHandle, MapKey, auras, objects};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// A character of `race` and `class` in the world, and their map.
async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let continent = if matches!(
        race,
        Race::Orc | Race::Tauren | Race::Troll | Race::NightElf
    ) {
        1
    } else {
        0
    };
    let map = realm.maps.get(realm, MapKey::world(continent));
    (c, g, map)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// `g` at `level`, knowing `learn`, its powers full.
fn ready(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
    }
    let p = m.players.get_mut(&g).unwrap();
    for i in 0..5 {
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
        p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
    }
}

/// A creature of `entry` at `level`, `yards` in front of `g` and facing
/// it, hardy.
fn dummy(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the template");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &template,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
    let c = c.with_base();
    m.add_creature(c)
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

const DETECT_TRAPS: u32 = 2836;
const FREEZING_TRAP: u32 = 1499;
const FREEZING_TRAP_OBJECT: u32 = 2561;
const SEARING_TOTEM: u32 = 3599;
const WARD_OF_ZANZIL: u32 = 8832;
const BATTLE_STANDARD: u32 = 23_034;
const CHICKEN_EGG: u32 = 13_563;
const SAND_TRAP: u32 = 25_648;

/// Thistle Tea's owned and wild objects and its independent totems
/// (`spell_vmangos_test.exs`): a hunter's stealthed trap hides from her
/// enemy until Detect Traps shows it; a ward and a battleground standard
/// stand beside a shaman's fire totem, taking no element's slot, with the
/// health their spells give; a quest's egg and a sand trap are summoned
/// wild for their spells' two minutes and one.
#[tokio::test]
async fn real_traps_totems_and_wild_objects_keep_their_kinds() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, hunter, map) = enter(&realm, "P3JTRAP1", "Trappa", Race::Dwarf, Class::Hunter).await;
    let (_r, rogue, _) = enter(&realm, "P3JTRAP2", "Seeka", Race::Dwarf, Class::Rogue).await;
    let seen = on(&map, move |m| {
        ready(m, hunter, 60, &[FREEZING_TRAP]);
        ready(m, rogue, 60, &[]);
        let at = m.players[&hunter].position();
        m.players.get_mut(&rogue).unwrap().movement.position = at;
        // Free for all, they are each other's enemies.
        for g in [hunter, rogue] {
            m.players.get_mut(&g).unwrap().fields.set_flag(
                PLAYER_FLAGS,
                crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
                true,
            );
        }
        spells::cast_triggered(m, hunter, FREEZING_TRAP, None);
        let trap = m
            .gameobjects
            .iter()
            .find(|(_, o)| o.entry == FREEZING_TRAP_OBJECT)
            .map(|(_, o)| o.guid)
            .expect("the trap");
        let before = objects::seen_by(m, rogue, trap);
        spells::cast_triggered(m, rogue, DETECT_TRAPS, None);
        (
            objects::seen_by(m, hunter, trap),
            before,
            objects::seen_by(m, rogue, trap),
        )
    })
    .await;
    assert_eq!(seen, (true, false, true), "hers, hidden, then detected");

    let (_s, shaman, kalimdor) =
        enter(&realm, "P3JTRAP3", "Totema", Race::Orc, Class::Shaman).await;
    let totems = on(&kalimdor, move |m| {
        ready(m, shaman, 60, &[SEARING_TOTEM]);
        spells::cast_triggered(m, shaman, SEARING_TOTEM, None);
        let fire = m.players[&shaman].owned.totems[0].expect("the fire totem");
        spells::cast_triggered(m, shaman, WARD_OF_ZANZIL, None);
        spells::cast_triggered(m, shaman, BATTLE_STANDARD, None);
        let owned = &m.players[&shaman].owned;
        let loose = owned.loose_totems.clone();
        let health: Vec<u32> = loose
            .iter()
            .map(|g| {
                m.unit_fields(*g)
                    .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXHEALTH))
            })
            .collect();
        (
            owned.totems[0] == Some(fire) && m.creature(fire).is_some(),
            health,
        )
    })
    .await;
    assert_eq!(
        totems,
        (true, vec![5, 1500]),
        "the fire totem stays; the ward's five and the standard's 1500"
    );

    let wild = on(&map, move |m| {
        let now = m.now;
        spells::cast_triggered(m, hunter, CHICKEN_EGG, None);
        spells::cast_triggered(m, hunter, SAND_TRAP, Some(hunter));
        [161_513u32, 180_647]
            .map(|entry| {
                m.objects
                    .summoned
                    .iter()
                    .find(|s| m.gameobject(s.guid).is_some_and(|o| o.entry == entry))
                    .map(|s| (s.owner, s.expires.map(|e| (e - now).as_secs())))
            })
            .to_vec()
    })
    .await;
    assert_eq!(
        wild,
        vec![Some((None, Some(120))), Some((None, Some(60)))],
        "nobody's, for two minutes and one"
    );
}

const FROSTBOLT: u32 = 116;
const FIRST_REFLECTION: u32 = 30_003;
const DETECT_MAGIC: u32 = 2855;
const LESSER_INVISIBILITY: u32 = 3680;
const INVISIBILITY: u32 = 11_392;
const DETECT_INVISIBILITY: [u32; 3] = [132, 2970, 11_743];
const KOBOLD_VERMIN: u32 = 6;

/// `a` and `b` side by side, free for all.
fn enemies(m: &mut Map, a: Guid, b: Guid) {
    let at = m.players[&a].position();
    m.players.get_mut(&b).unwrap().movement.position = Vector3d {
        x: at.x + 5.0,
        ..at
    };
    for g in [a, b] {
        m.players.get_mut(&g).unwrap().fields.set_flag(
            PLAYER_FLAGS,
            crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
            true,
        );
    }
}

/// Thistle Tea's reflection, Detect Magic and invisibility
/// (`spell_vmangos_test.exs`): Sheen of Zanza's sure first reflection
/// turns a Frostbolt back on the mage and is spent; Detect Magic shows a
/// creature's auras for two minutes; a potion's invisibility hides a
/// warrior from an enemy, whom a rank of Detect Invisibility as strong as
/// it lets see him.
#[tokio::test]
async fn real_reflection_detect_magic_and_invisibility() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_m, mage, map) = enter(&realm, "P3JREFL1", "Refla", Race::Human, Class::Mage).await;
    let (_w, warrior, _) = enter(&realm, "P3JREFL2", "Reflb", Race::Undead, Class::Warrior).await;
    let reflected = on(&map, move |m| {
        ready(m, mage, 60, &[]);
        ready(m, warrior, 60, &[]);
        enemies(m, mage, warrior);
        spells::cast_triggered(m, warrior, FIRST_REFLECTION, None);
        let charged = has(m, warrior, FIRST_REFLECTION);
        let (mage_before, warrior_before) = (health(m, mage), health(m, warrior));
        spells::cast_triggered(m, mage, FROSTBOLT, Some(warrior));
        (
            charged,
            health(m, mage) < mage_before,
            health(m, warrior) == warrior_before,
            has(m, warrior, FIRST_REFLECTION),
        )
    })
    .await;
    assert_eq!(
        reflected,
        (true, true, true, false),
        "the Frostbolt hits its mage, and the reflection is spent"
    );

    let shown = on(&map, move |m| {
        let kobold = dummy(m, mage, KOBOLD_VERMIN, 5, 10.0);
        let flags = |m: &Map| {
            m.unit_fields(kobold)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS))
        };
        let before = flags(m) & 0x0800_0000;
        spells::cast_triggered(m, mage, DETECT_MAGIC, Some(kobold));
        let now = m.now;
        let lasts = auras::auras(m, kobold)
            .and_then(|a| a.holders.iter().find(|h| h.spell == DETECT_MAGIC))
            .and_then(|h| h.expires)
            .map(|e| (e - now).as_secs());
        (before, flags(m) & 0x0800_0000, lasts)
    })
    .await;
    assert_eq!(shown, (0, 0x0800_0000, Some(120)), "its auras shown");

    let seen = on(&map, move |m| {
        auras::remove_spell(m, warrior, FIRST_REFLECTION, None);
        [LESSER_INVISIBILITY, INVISIBILITY].map(|potion| {
            spells::cast_triggered(m, warrior, potion, None);
            let mut seen = vec![spells::stealth::sees(m, mage, warrior)];
            for rank in DETECT_INVISIBILITY {
                spells::cast_triggered(m, mage, rank, None);
                seen.push(spells::stealth::sees(m, mage, warrior));
                auras::remove_spell(m, mage, rank, None);
            }
            auras::remove_spell(m, warrior, potion, None);
            seen
        })
    })
    .await;
    assert_eq!(
        seen,
        [
            vec![false, true, true, true],
            vec![false, false, true, true]
        ],
        "hidden, then seen by a detection as strong"
    );
}
