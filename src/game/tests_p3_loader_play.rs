//! Phase 3's loader tests played out (Thistle Tea's `world/loader` tests
//! of the real spells): what the real spells do once cast, on the real
//! data when it is here.

use std::sync::Arc;
use std::time::{Duration, Instant};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::{self, Hand};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, outcome};
use crate::game::map::{Map, MapHandle, MapKey, auras, objects};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(crate::game::harness::load_real(DB.as_path()))
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
#[ignore = "extended"]
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
#[ignore = "extended"]
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

const BATTLE_STANCE: u32 = 2457;
const DEFENSIVE_STANCE: u32 = 71;
const BATTLE_STANCE_PASSIVE: u32 = 21_156;
const DEFENSIVE_STANCE_PASSIVE: u32 = 7376;
const BLOODTHIRST: u32 = 23_881;
const BLOODTHIRST_HEAL_BUFF: u32 = 23_885;
const HEROIC_STRIKE: u32 = 78;
const REVENGE: u32 = 6572;
const SUNDER_ARMOR: u32 = 7386;

/// What `g` has on `c`'s threat list.
fn threat(m: &Map, c: Guid, g: Guid) -> f32 {
    m.creature(c)
        .and_then(|c| c.threat.iter().find(|(t, _)| *t == g).map(|(_, v)| *v))
        .unwrap_or(0.0)
}

/// Thistle Tea's warrior spells (`spell_vmangos_test.exs`): Heroic
/// Strike's flat 20 threat over its damage, Revenge's 2.25 times and 63,
/// Fireball's damage alone with no `spell_threat` row; Sunder Armor's five
/// stacks; Bloodthirst's heal buff on the warrior (vmangos's
/// `spell_effect_mod`); and each stance's passives, the last's gone with
/// it.
#[tokio::test]
#[ignore = "extended"]
async fn real_warrior_threat_stacks_and_stances() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, w, map) = enter(&realm, "P3JWAR1", "Threata", Race::Human, Class::Warrior).await;
    let (_m, mage, _) = enter(&realm, "P3JWAR2", "Threatb", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, w, 60, &[]);
        ready(m, mage, 60, &[]);
        // Out of the Battle Stance a warrior logs in in, and its threat.
        auras::remove_spell(m, w, BATTLE_STANCE, None);
        let mut threats = Vec::new();
        for (who, spell) in [(w, HEROIC_STRIKE), (w, REVENGE), (mage, 133)] {
            let kobold = dummy(m, w, KOBOLD_VERMIN, 60, 2.0);
            let before = health(m, kobold);
            spells::cast_triggered(m, who, spell, Some(kobold));
            let dealt = (before - health(m, kobold)) as f32;
            threats.push((dealt > 0.0, threat(m, kobold, who) - dealt));
        }
        let kobold = dummy(m, w, KOBOLD_VERMIN, 60, 2.0);
        for _ in 0..6 {
            spells::cast_triggered(m, w, SUNDER_ARMOR, Some(kobold));
        }
        let sunder = auras::auras(m, kobold)
            .and_then(|a| a.holders.iter().find(|h| h.spell == SUNDER_ARMOR))
            .map(|h| h.stacks);
        spells::cast_triggered(m, w, BLOODTHIRST, Some(kobold));
        let buffed = has(m, w, BLOODTHIRST_HEAL_BUFF);
        spells::cast_triggered(m, w, BATTLE_STANCE, None);
        let battle = (
            has(m, w, BATTLE_STANCE_PASSIVE),
            has(m, w, DEFENSIVE_STANCE_PASSIVE),
        );
        spells::cast_triggered(m, w, DEFENSIVE_STANCE, None);
        let defensive = (
            has(m, w, BATTLE_STANCE_PASSIVE),
            has(m, w, DEFENSIVE_STANCE_PASSIVE),
        );
        (threats, sunder, buffed, battle, defensive)
    })
    .await;
    let (threats, sunder, buffed, battle, defensive) = out;
    assert!(threats.iter().all(|(hit, _)| *hit), "{threats:?}");
    let extra: Vec<f32> = threats.iter().map(|(_, x)| x.round()).collect();
    assert_eq!(extra[0], 20.0, "Heroic Strike's 20");
    assert!(extra[1] > 63.0, "Revenge's 2.25 times and 63: {}", extra[1]);
    assert_eq!(extra[2], 0.0, "Fireball's damage alone");
    assert_eq!(sunder, Some(5), "Sunder Armor stacks to five");
    assert!(buffed, "Bloodthirst's heal buff");
    assert_eq!(battle, (true, false), "Battle Stance's passive");
    assert_eq!(
        defensive,
        (false, true),
        "Defensive Stance's, and not Battle's"
    );
}

const SUMMON_FRONT: u32 = 15_734;
const SUMMON_PLAYER: u32 = 21_150;
const BANISH: u32 = 24_466;

/// Thistle Tea's caster-relative teleports (`spell_vmangos_test.exs`), as
/// mangos's `EffectTeleUnitsFaceCaster`: a summons to the caster's front
/// lands its radius (five yards) ahead of it, a summons to the caster on
/// its spot, and a banishment to its `spell_target_position`; each turned
/// to the caster's facing negated.
#[tokio::test]
#[ignore = "extended"]
async fn real_teleports_to_a_caster_land_where_their_targets_say() {
    if !have_data() {
        return;
    }
    let mut d = crate::game::harness::load_real(DB.as_path());
    let spot = Vector3d {
        x: -8_949.95 + 30.0,
        y: -132.493,
        z: 83.5312,
    };
    d.spells
        .target_positions
        .insert(BANISH, (0, spot.x, spot.y, spot.z, 1.0));
    let realm = harness::realm_with(d);
    let (_w, w, map) = enter(&realm, "P3JTELE1", "Telea", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        let kobold = dummy(m, w, KOBOLD_VERMIN, 20, 20.0);
        let (from, o) = {
            let c = m.creature(kobold).unwrap();
            (c.position, c.orientation)
        };
        let where_now = |m: &Map| {
            let p = &m.players[&w];
            (p.position(), p.movement.orientation)
        };
        let mut landed = Vec::new();
        for spell in [SUMMON_FRONT, SUMMON_PLAYER, BANISH] {
            spells::cast_triggered(m, kobold, spell, Some(w));
            landed.push(where_now(m));
        }
        (from, o, landed)
    })
    .await;
    let (from, o, landed) = out;
    let near = |a: Vector3d, b: Vector3d| (a.x - b.x).hypot(a.y - b.y) < 0.1;
    let front = Vector3d {
        x: from.x + 5.0 * o.cos(),
        y: from.y + 5.0 * o.sin(),
        z: from.z,
    };
    assert!(near(landed[0].0, front), "{:?} {front:?}", landed[0].0);
    assert!(near(landed[1].0, from), "{:?} {from:?}", landed[1].0);
    assert!(near(landed[2].0, spot), "{:?} {spot:?}", landed[2].0);
    let facing = (-o).rem_euclid(std::f32::consts::TAU);
    for (_, turned) in landed {
        assert!((turned - facing).abs() < 0.01, "{turned} {facing}");
    }
}

/// Thistle Tea's debug grants (`spell_vmangos_test.exs`'s talent grants,
/// rogue, paladin, warlock, hunter, shaman and druid rows): `.debug spells`
/// teaches a sixtieth-level character their trainers' spells and their
/// class quests' (the warlock's demons, the hunter's pet spells, the
/// shaman's totems, the druid's forms), but no talent nor a talent's
/// trained rank (Mortal Strike's) to one without the talent, and those
/// ranks to one with it.
#[tokio::test]
#[ignore = "extended"]
async fn debug_spells_teach_trainers_and_class_quests_but_no_talents() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let classes: [(Race, Class, &[u32], &[u32]); 7] = [
        (
            Race::Human,
            Class::Warrior,
            &[71, 2458, 78],
            &[
                12_294, 21_551, 21_553, 23_881, 23_892, 23_894, 12_975, 12_320,
            ],
        ),
        (
            Race::Human,
            Class::Rogue,
            &[53],
            &[13_750, 13_877, 14_177, 14_185, 14_056],
        ),
        (
            Race::Human,
            Class::Paladin,
            &[635],
            &[20_066, 20_473, 20_375],
        ),
        (
            Race::Human,
            Class::Warlock,
            &[688, 697, 712, 691, 1122, 18_540],
            &[18_288],
        ),
        (Race::Dwarf, Class::Hunter, &[1515, 883, 982, 136], &[]),
        (Race::Orc, Class::Shaman, &[8071, 3599, 5394, 8512], &[]),
        (Race::Tauren, Class::Druid, &[5487, 1066, 768], &[]),
    ];
    for (i, (race, class, has, hasnt)) in classes.into_iter().enumerate() {
        let (mut c, g, map) = enter(
            &realm,
            &format!("P3JDEBUG{i}"),
            &format!("Debuga{}", (b'a' + i as u8) as char),
            race,
            class,
        )
        .await;
        on(&map, move |m| ready(m, g, 60, &[])).await;
        c.say(".debug spells").await;
        c.system_message("Learned").await;
        let known = on(&map, move |m| m.players[&g].spells.clone()).await;
        for s in has {
            assert!(known.contains(s), "{class:?} learns {s}");
        }
        for s in hasnt {
            assert!(!known.contains(s), "{class:?} learns no talent's {s}");
        }
    }
    // With the talent, its trained ranks come too.
    let (mut c, g, map) = enter(
        &realm,
        "P3JDEBUGT",
        "Debugmortal",
        Race::Human,
        Class::Warrior,
    )
    .await;
    on(&map, move |m| ready(m, g, 60, &[12_294])).await;
    c.say(".debug spells").await;
    c.system_message("Learned").await;
    let known = on(&map, move |m| m.players[&g].spells.clone()).await;
    for s in [21_551, 21_552, 21_553] {
        assert!(known.contains(&s), "Mortal Strike's {s} with the talent");
    }
}

/// `caster` puts real spell `id`'s auras, at their sixtieth-level points,
/// on `target`.
fn apply_real(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let data = m.realm.data.clone();
    let s = data
        .spells
        .get(id)
        .unwrap_or_else(|| panic!("spell {id}"))
        .clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default());
}

/// `g` hits for exactly `damage` with each hand, of `school`.
fn hits_for(m: &mut Map, g: Guid, damage: f32) {
    let f = m.unit_fields_mut(g).unwrap();
    for (lo, hi) in [
        (UNIT_FIELD_MINDAMAGE, UNIT_FIELD_MAXDAMAGE),
        (UNIT_FIELD_MINRANGEDDAMAGE, UNIT_FIELD_MAXRANGEDDAMAGE),
    ] {
        f.set_f32(lo, damage);
        f.set_f32(hi, damage);
    }
}

const STONESKIN: [(u32, u32); 6] = [
    (8072, 4),
    (8156, 7),
    (8157, 11),
    (10_403, 16),
    (10_404, 22),
    (10_405, 30),
];
const CREATURE_SHADOWFORM: [(u32, u32); 2] = [(16_592, 20), (22_917, 40)];
const DAMPEN_MAGIC: [u32; 5] = [604, 8450, 8451, 10_173, 10_174];
const AMPLIFY_MAGIC: [u32; 4] = [1008, 8455, 10_169, 10_170];
const SMITE_1: u32 = 585;
const SMITE_8: u32 = 10_934;
const DECIMATE: u32 = 28_375;
const FIRE_WARD: u32 = 543;
const SACRIFICE: [(u32, u32); 2] = [(6940, 45), (20_729, 55)];
const SOUL_LINK_SPLIT: u32 = 25_228;
const FIRE_ELEMENTAL: u32 = 575;

/// Thistle Tea's damage taken auras on the real spells
/// (`attack_damage_taken_dbc_test.exs`, `damage_received_dbc_test.exs`,
/// `damage_sharing_dbc_test.exs`, `elemental_combat_dbc_test.exs`): every
/// Stoneskin rank takes its points off a melee blow and none off a ranged
/// one, a creature's Shadowform its share of a melee blow; Dampen and
/// Amplify Magic move a Smite by their points times its coefficient (more
/// for a later rank's), and a fire elemental's swing by their whole
/// points, until they go; Decimate takes no notice of them; Fire Ward
/// soaks a fire swing; Blessing of Sacrifice and Soul Link send their
/// share to their caster once.
#[tokio::test]
#[ignore = "extended"]
async fn real_damage_taken_auras_shares_and_wards() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_p, priest, map) = enter(&realm, "P3JTAKE1", "Takea", Race::Human, Class::Priest).await;
    let (_w, warrior, _) = enter(&realm, "P3JTAKE2", "Takeb", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, priest, 60, &[]);
        ready(m, warrior, 60, &[]);
        let brute = dummy(m, warrior, KOBOLD_VERMIN, 60, 2.0);
        hits_for(m, brute, 100.0);
        let blow =
            |m: &Map, hand| combat::weapon_damage(m, brute, warrior, hand, &mut rand::rng()).0;
        let mut melee = Vec::new();
        for (id, cut) in STONESKIN {
            apply_real(m, warrior, warrior, id);
            melee.push((
                id,
                100 - blow(m, Hand::Main),
                100 - blow(m, Hand::Ranged),
                cut,
            ));
            auras::remove_spell(m, warrior, id, None);
        }
        for (id, cut) in CREATURE_SHADOWFORM {
            apply_real(m, warrior, warrior, id);
            melee.push((
                id,
                100 - blow(m, Hand::Main),
                100 - blow(m, Hand::Ranged),
                cut,
            ));
            auras::remove_spell(m, warrior, id, None);
        }
        // Smite into the warrior, at a thousand before any modifier.
        let data = m.realm.data.clone();
        let smite = |m: &Map, id: u32| {
            let s = data.spells.get(id).unwrap();
            outcome::damage(
                m,
                priest,
                warrior,
                s,
                0,
                1000.0,
                &outcome::Landing::default(),
            )
            .amount
        };
        let decimate_alone = smite(m, DECIMATE);
        let mut magic = Vec::new();
        for id in DAMPEN_MAGIC.iter().chain(AMPLIFY_MAGIC.iter()).copied() {
            apply_real(m, priest, warrior, id);
            let flat =
                auras::auras(m, warrior).map_or(0, |a| a.sum_mask(auras::AURA_MOD_DAMAGE_TAKEN, 2));
            let coefficient = |id| {
                outcome::taken_coefficient(
                    m,
                    priest,
                    data.spells.get(id).unwrap(),
                    0,
                    outcome::Kind::Direct,
                )
            };
            let (low, high) = (coefficient(SMITE_1), coefficient(SMITE_8));
            magic.push((
                id,
                flat,
                smite(m, SMITE_1) as f32 - 1000.0,
                low,
                smite(m, SMITE_8) as f32 - 1000.0,
                high,
                smite(m, DECIMATE),
            ));
            auras::remove_spell(m, warrior, id, None);
        }
        let after = (smite(m, SMITE_8), smite(m, DECIMATE));
        // A fire elemental's swing: Dampen and Amplify Magic's whole
        // points; Fire Ward soaks it.
        let elemental = dummy(m, warrior, FIRE_ELEMENTAL, 60, 2.0);
        hits_for(m, elemental, 100.0);
        let fire =
            |m: &Map| combat::weapon_damage(m, elemental, warrior, Hand::Main, &mut rand::rng());
        let mut swings = vec![fire(m)];
        for id in [604, 1008] {
            apply_real(m, priest, warrior, id);
            swings.push(fire(m));
            auras::remove_spell(m, warrior, id, None);
        }
        let mage_level = |m: &mut Map, level| {
            let data = m.realm.data.clone();
            crate::game::map::stats::set_level(m.players.get_mut(&warrior).unwrap(), &data, level);
        };
        mage_level(m, 50);
        apply_real(m, warrior, warrior, FIRE_WARD);
        let ward = auras::auras(m, warrior).and_then(|a| {
            a.effects()
                .find(|(h, _)| h.spell == FIRE_WARD)
                .map(|(_, x)| x.left)
        });
        let soaked = auras::absorb(m, warrior, fire(m).0, 4);
        let ward_left = auras::auras(m, warrior).and_then(|a| {
            a.effects()
                .find(|(h, _)| h.spell == FIRE_WARD)
                .map(|(_, x)| x.left)
        });
        // Blessing of Sacrifice and Soul Link: the share to their caster.
        let mut shared = Vec::new();
        for (id, share) in SACRIFICE.into_iter().chain([(SOUL_LINK_SPLIT, 30)]) {
            apply_real(m, priest, warrior, id);
            let before = health(m, priest);
            let s = data.spells.get(SMITE_8).unwrap();
            let left = auras::share(m, brute, warrior, 100, s.school_mask());
            shared.push((id, left, before - health(m, priest), share));
            auras::remove_spell(m, warrior, id, None);
        }
        (
            melee,
            (magic, decimate_alone),
            after,
            swings,
            (ward, soaked, ward_left),
            shared,
        )
    })
    .await;
    let (melee, (magic, decimate_alone), after, swings, ward, shared) = out;
    assert_eq!(
        swings,
        vec![(100, 4), (90, 4), (115, 4)],
        "a fire swing, Dampened and Amplified"
    );
    assert_eq!(ward, (Some(165), (0, 100), Some(65)), "Fire Ward soaks it");
    for (id, left, taken, share) in shared {
        assert_eq!((left, taken), (100 - share, share), "{id}: its share, once");
    }
    for (id, main, ranged, cut) in melee {
        assert_eq!((main, ranged), (cut, 0), "{id}: off a melee blow only");
    }
    for (id, flat, low, c_low, high, c_high, decimate) in magic {
        assert!(flat != 0, "{id}");
        assert!(
            0.0 < c_low && c_low < c_high && c_high < 1.0,
            "{id}: {c_low} {c_high}"
        );
        assert!(
            (low - flat as f32 * c_low).abs() <= 1.0,
            "{id}: {low} {flat} {c_low}"
        );
        assert!(
            (high - flat as f32 * c_high).abs() <= 1.0,
            "{id}: {high} {flat} {c_high}"
        );
        assert_eq!(decimate, decimate_alone, "{id}: Decimate ignores it");
    }
    assert_eq!(after, (1000, decimate_alone), "gone, Smite is itself again");
}

const BLESSING_OF_LIGHT: [u32; 2] = [19_979, 25_890];
const HEALING_WAY: u32 = 29_203;
const HEALING_WAVE: u32 = 331;
const LESSER_HEALING_WAVE: u32 = 8004;
const CHAIN_HEAL: u32 = 1064;
const SUPPRESSIONS: [(u32, u32); 4] = [(9035, 160), (19_716, 50), (23_230, 100), (25_646, 180)];
const MORTAL_WOUND: u32 = 25_646;
const MECHANICAL_PATCH_KIT: u32 = 15_057;
const HARVEST_GOLEM: u32 = 36;
const FLASH_OF_LIGHT: u32 = 19_750;

/// Thistle Tea's healing on the real spells (`healing_bonus_dbc_test.exs`,
/// `healing_bonus_vmangos_test.exs`, `healing_received_dbc_test.exs`,
/// `mechanical_healing_dbc_test.exs`): one Dampen or Amplify Magic on a
/// unit, of any rank or caster, the last; Blessing of Light's points by
/// the heal's coefficient (Holy Light's 0.714, Flash of Light's 0.429 of
/// its triggered heal); Healing Way's stacks for Healing Wave alone; the
/// real healing cuts, and Mortal Wound's ten stacks to nothing; a Flash
/// of Light that heals; and the Mechanical Patch Kit's 700 on a machine.
#[tokio::test]
#[ignore = "extended"]
async fn real_healing_bonuses_cuts_and_machines() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_p, paladin, map) = enter(&realm, "P3JHEAL1", "Heala", Race::Human, Class::Paladin).await;
    let (_m, mage, _) = enter(&realm, "P3JHEAL2", "Healb", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, paladin, 60, &[]);
        ready(m, mage, 60, &[]);
        let data = m.realm.data.clone();
        // One magic of the mage's kind on a unit, whoever's.
        let mut magic = Vec::new();
        for ids in [[10_170, 10_174, 1008], [10_174, 10_170, 604]] {
            for (i, id) in ids.into_iter().enumerate() {
                apply_real(m, if i % 2 == 0 { paladin } else { mage }, paladin, id);
            }
            let held: Vec<u32> = auras::auras(m, paladin)
                .map(|a| {
                    a.holders
                        .iter()
                        .filter(|h| [ids[0], ids[1], ids[2]].contains(&h.spell))
                        .map(|h| h.spell)
                        .collect()
                })
                .unwrap_or_default();
            magic.push((held, ids[2]));
            for id in ids {
                auras::remove_spell(m, paladin, id, None);
            }
        }
        let mut light = Vec::new();
        for id in BLESSING_OF_LIGHT {
            apply_real(m, paladin, mage, id);
            light.push((
                spells::scripts::blessing_of_light_bonus(m, mage, true),
                spells::scripts::blessing_of_light_bonus(m, mage, false),
            ));
            auras::remove_spell(m, mage, id, None);
        }
        // Healing Way: three stacks.
        for _ in 0..3 {
            apply_real(m, mage, paladin, HEALING_WAY);
        }
        let taken = |m: &Map, id: u32| {
            outcome::spell_healing_taken(
                m,
                paladin,
                200.0,
                data.spells.get(id).unwrap(),
                0,
                outcome::Kind::Direct,
                1,
                1.0,
            )
        };
        let way = [HEALING_WAVE, LESSER_HEALING_WAVE, CHAIN_HEAL].map(|id| taken(m, id) as u32);
        auras::remove_spell(m, paladin, HEALING_WAY, None);
        let way_gone = taken(m, HEALING_WAVE) as u32;
        let mut cuts = Vec::new();
        for (id, left) in SUPPRESSIONS {
            apply_real(m, mage, paladin, id);
            cuts.push((id, outcome::healing_taken(m, paladin, 200.0) as u32, left));
            auras::remove_spell(m, paladin, id, None);
        }
        for _ in 0..10 {
            apply_real(m, mage, paladin, MORTAL_WOUND);
        }
        let wounded = outcome::healing_taken(m, paladin, 200.0) as u32;
        auras::remove_spell(m, paladin, MORTAL_WOUND, None);
        // Flash of Light heals; a Patch Kit mends a machine.
        m.players
            .get_mut(&paladin)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 100);
        spells::cast_triggered(m, paladin, FLASH_OF_LIGHT, Some(paladin));
        let flash = health(m, paladin) > 100;
        let golem = dummy(m, paladin, HARVEST_GOLEM, 60, 3.0);
        m.unit_fields_mut(golem)
            .unwrap()
            .set_u32(UNIT_FIELD_HEALTH, 100);
        spells::cast_triggered(m, paladin, MECHANICAL_PATCH_KIT, Some(golem));
        let mended = health(m, golem);
        (magic, light, way, way_gone, cuts, wounded, flash, mended)
    })
    .await;
    let (magic, light, way, way_gone, cuts, wounded, flash, mended) = out;
    for (held, last) in magic {
        assert_eq!(held, vec![last], "the last magic only");
    }
    assert_eq!(
        light,
        vec![(285, 49), (285, 49)],
        "400 and 115 by 0.714 and 0.429"
    );
    // 18% more, as mangos's float takes it (235.99...).
    assert!(matches!(way[0], 235 | 236), "Healing Way's 18%: {way:?}");
    assert_eq!(&way[1..], [200, 200], "for Healing Wave alone");
    assert_eq!(way_gone, 200);
    for (id, got, left) in cuts {
        assert_eq!(got, left, "{id}");
    }
    assert_eq!(wounded, 0, "ten Mortal Wounds heal nothing");
    assert!(flash, "Flash of Light heals");
    assert_eq!(mended, 800, "the Patch Kit's 700");
}

/// `caster` puts real spell `id`'s auras, at `level`'s points, on `target`.
fn apply_real_at(m: &mut Map, caster: Guid, target: Guid, id: u32, level: u32) {
    let data = m.realm.data.clone();
    let s = data
        .spells
        .get(id)
        .unwrap_or_else(|| panic!("spell {id}"))
        .clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, level)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default());
}

/// A unit's armor and resistances, and its intellect.
fn defenses(m: &Map, g: Guid) -> ([u32; 7], u32) {
    let f = m.unit_fields(g).unwrap();
    (
        std::array::from_fn(|i| f.get_u32(UNIT_FIELD_RESISTANCES + i as u16)),
        f.get_u32(UNIT_FIELD_STAT0 + 3),
    )
}

/// What `g`'s auras take off a target's resistances, by school mask.
fn penetration(m: &Map, g: Guid, mask: u32) -> i32 {
    auras::auras(m, g).map_or(0, |a| a.sum_mask(auras::AURA_MOD_TARGET_RESISTANCE, mask))
}

const MARK_OF_THE_WILD: u32 = 9885;
const FIRE_RESISTANCE_AURA: u32 = 19_891;
const SERRATED_BLADES: [(u32, u32, i32); 4] = [
    (14_171, 60, -100),
    (14_172, 60, -200),
    (14_173, 60, -300),
    (14_173, 40, -200),
];
const BONEREAVER: u32 = 21_153;
const SCHOOL_PENETRATION: [(u32, u32, i32); 5] = [
    (11_210, 126, -5),
    (12_592, 126, -10),
    (25_717, 4, -300),
    (25_718, 16, -300),
    (25_975, 124, -10),
];
const BASE_RESISTANCE: [(u32, u32, bool); 3] =
    [(819, 15, true), (21_740, 100, false), (21_925, 40, false)];
const SLAYING: [(u32, &[u32], u32, u32); 4] = [
    (11_406, &[3], 265, 0),
    (23_930, &[6], 81, 81),
    (29_112, &[3, 6], 150, 150),
    (17_352, &[6], 200, 0),
];

/// Thistle Tea's resistances and penetration on the real spells
/// (`resistance_dbc_test.exs`, `resistance_penetration_dbc_test.exs`,
/// `spell_base_resistance_dbc_test.exs`, `target_attack_power_dbc_test.exs`):
/// Mark of the Wild beside a stronger Fire Resistance Aura, fire's the
/// stronger and the Mark's back when it goes; Serrated Blades by the
/// rogue's level, Bonereaver's three stacks, the talents' and gear's
/// school masks; base armor and resistances; and slaying's attack power
/// against its creature types alone.
#[tokio::test]
#[ignore = "extended"]
async fn real_resistances_penetration_and_slaying() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_d, druid, map) = enter(&realm, "P3JRES1", "Resa", Race::NightElf, Class::Druid).await;
    let out = on(&map, move |m| {
        ready(m, druid, 60, &[]);
        let bare = defenses(m, druid);
        let mut marked = Vec::new();
        for order in [
            [MARK_OF_THE_WILD, FIRE_RESISTANCE_AURA],
            [FIRE_RESISTANCE_AURA, MARK_OF_THE_WILD],
        ] {
            for id in order {
                apply_real(m, druid, druid, id);
            }
            let both = defenses(m, druid);
            auras::remove_spell(m, druid, FIRE_RESISTANCE_AURA, None);
            let mark = defenses(m, druid);
            auras::remove_spell(m, druid, MARK_OF_THE_WILD, None);
            let none = defenses(m, druid);
            marked.push((both, mark, none));
        }
        let mut serrated = Vec::new();
        for (id, level, _) in SERRATED_BLADES {
            apply_real_at(m, druid, druid, id, level);
            serrated.push(penetration(m, druid, 1));
            auras::remove_spell(m, druid, id, None);
        }
        for _ in 0..4 {
            apply_real(m, druid, druid, BONEREAVER);
        }
        let bonereaver = penetration(m, druid, 1);
        auras::remove_spell(m, druid, BONEREAVER, None);
        let mut schools = Vec::new();
        for (id, mask, _) in SCHOOL_PENETRATION {
            apply_real(m, druid, druid, id);
            schools.push((
                penetration(m, druid, mask),
                penetration(m, druid, 1 | (!mask & 0x7e)),
            ));
            auras::remove_spell(m, druid, id, None);
        }
        let kobold = dummy(m, druid, KOBOLD_VERMIN, 60, 3.0);
        let mut base = Vec::new();
        for (id, _, _) in BASE_RESISTANCE {
            let before = defenses(m, kobold).0;
            apply_real(m, druid, kobold, id);
            let after = defenses(m, kobold).0;
            base.push(std::array::from_fn::<i32, 7, _>(|i| {
                after[i] as i32 - before[i] as i32
            }));
            auras::remove_spell(m, kobold, id, None);
        }
        let mut slaying = Vec::new();
        for (id, _, _, _) in SLAYING {
            apply_real(m, druid, druid, id);
            let by_type: Vec<(i32, i32)> = (1..=9)
                .map(|t| {
                    let bit = 1 << (t - 1);
                    let a = auras::auras(m, druid).unwrap();
                    (
                        a.sum_mask(auras::AURA_MOD_MELEE_ATTACK_POWER_VERSUS, bit),
                        a.sum_mask(auras::AURA_MOD_RANGED_ATTACK_POWER_VERSUS, bit),
                    )
                })
                .collect();
            slaying.push(by_type);
            auras::remove_spell(m, druid, id, None);
        }
        (bare, marked, serrated, bonereaver, schools, base, slaying)
    })
    .await;
    let (bare, marked, serrated, bonereaver, schools, base, slaying) = out;
    let up = |d: ([u32; 7], u32)| -> ([i32; 7], i32) {
        (
            std::array::from_fn(|i| d.0[i] as i32 - bare.0[i] as i32),
            d.1 as i32 - bare.1 as i32,
        )
    };
    // The Mark's 285 armor, and twice its 12 agility's.
    for (both, mark, none) in marked {
        assert_eq!(
            up(both),
            ([309, 20, 30, 20, 20, 20, 20], 12),
            "both, fire's the stronger"
        );
        assert_eq!(
            up(mark),
            ([309, 20, 20, 20, 20, 20, 20], 12),
            "the Mark's own fire"
        );
        assert_eq!(up(none), ([0; 7], 0), "and nothing");
    }
    assert_eq!(
        serrated,
        SERRATED_BLADES.map(|(_, _, a)| a).to_vec(),
        "by level"
    );
    assert_eq!(bonereaver, -2100, "three stacks of 700");
    for ((on, off), (id, _, amount)) in schools.into_iter().zip(SCHOOL_PENETRATION) {
        assert_eq!((on, off), (amount, 0), "{id}: its schools alone");
    }
    assert_eq!(base[0], [15, 0, 0, 0, 0, 0, 0], "Toughness: armor");
    assert_eq!(
        base[1],
        [0, 100, 100, 100, 100, 100, 100],
        "every magic school"
    );
    assert_eq!(base[2], [0, 40, 40, 40, 40, 40, 40]);
    for (by_type, (id, types, melee, ranged)) in slaying.into_iter().zip(SLAYING) {
        for (t, (got_melee, got_ranged)) in (1..=9).zip(by_type) {
            let on = types.contains(&t);
            assert_eq!(
                (got_melee, got_ranged),
                if on {
                    (melee as i32, ranged as i32)
                } else {
                    (0, 0)
                },
                "{id} against type {t}"
            );
        }
    }
}

/// A real creature of each type, 1 (beast) to 9 (mechanical).
const OF_TYPE: [u32; 9] = [2, 29, 49, 92, 90, 3, 6, 1, 36];
const SLAYER_DAMAGE: [(u32, u32, u32); 5] = [
    (7784, 1, 2),
    (13_650, 1, 6),
    (13_651, 4, 6),
    (4719, 6, 13),
    (4646, 3, 8),
];
const BEAST_DAMAGE_10: u32 = 7594;
const SLAYER_SPELL_POWER: [(u32, &[u32], u32); 8] = [
    (21_010, &[6], 14),
    (21_011, &[3], 14),
    (22_849, &[6], 35),
    (24_197, &[6], 48),
    (24_198, &[6], 48),
    (28_876, &[6], 26),
    (28_890, &[6], 60),
    (29_113, &[3, 6], 85),
];
const SHADOW_BOLT: u32 = 686;

/// Thistle Tea's slaying on the real spells (`target_damage_dbc_test.exs`,
/// `target_spell_power_dbc_test.exs`): the slayers' damage against their
/// creature type alone, an equipped one's apart from the damage shown and
/// the spell power; and the slaying gear's and potions' spell power (aura
/// 180) against theirs alone, none of it shown as spell power.
#[tokio::test]
#[ignore = "extended"]
async fn real_slaying_damage_and_spell_power_by_creature_type() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, warlock, map) = enter(&realm, "P3JSLAY1", "Slaya", Race::Human, Class::Warlock).await;
    let out = on(&map, move |m| {
        ready(m, warlock, 60, &[]);
        let foes: Vec<Guid> = OF_TYPE
            .iter()
            .map(|e| dummy(m, warlock, *e, 60, 2.0))
            .collect();
        // Each blow of 100 (an aura's coming or going works the weapon's
        // damage out again).
        let blow = |m: &mut Map, foe: Guid| {
            hits_for(m, warlock, 100.0);
            combat::weapon_damage(m, warlock, foe, Hand::Main, &mut rand::rng()).0 as i32
        };
        let base: Vec<i32> = foes.iter().map(|f| blow(m, *f)).collect();
        let mut damage = Vec::new();
        for (id, _, _) in SLAYER_DAMAGE {
            apply_real(m, warlock, warlock, id);
            let by_type: Vec<i32> = foes
                .iter()
                .zip(&base)
                .map(|(f, b)| blow(m, *f) - b)
                .collect();
            damage.push(by_type);
            auras::remove_spell(m, warlock, id, None);
        }
        let shown = |m: &mut Map| {
            auras::recalc(m, warlock);
            let f = m.unit_fields(warlock).unwrap();
            (
                f.get_f32(UNIT_FIELD_MINDAMAGE) as i32,
                combat::derived(m, warlock).spell_power,
            )
        };
        let before = shown(m);
        apply_real(m, warlock, warlock, BEAST_DAMAGE_10);
        let unshown = shown(m) == before;
        let beast = (blow(m, foes[0]) - base[0], unshown);
        auras::remove_spell(m, warlock, BEAST_DAMAGE_10, None);
        let data = m.realm.data.clone();
        let bolt = data.spells.get(SHADOW_BOLT).unwrap();
        let power = |m: &Map, foe: Guid| outcome::power(m, warlock, foe, bolt, false) as i32;
        let bare: Vec<i32> = foes.iter().map(|f| power(m, *f)).collect();
        let mut spell_power = Vec::new();
        for (id, _, _) in SLAYER_SPELL_POWER {
            apply_real(m, warlock, warlock, id);
            let by_type: Vec<i32> = foes
                .iter()
                .zip(&bare)
                .map(|(f, b)| power(m, *f) - b)
                .collect();
            spell_power.push((by_type, shown(m) == before));
            auras::remove_spell(m, warlock, id, None);
        }
        (damage, beast, spell_power)
    })
    .await;
    let (damage, beast, spell_power) = out;
    for (by_type, (id, kind, amount)) in damage.into_iter().zip(SLAYER_DAMAGE) {
        let want: Vec<i32> = (1..=9)
            .map(|t| if t == kind { amount as i32 } else { 0 })
            .collect();
        assert_eq!(by_type, want, "{id}");
    }
    assert_eq!(beast, (10, true), "on a beast, and not shown");
    for ((by_type, unshown), (id, kinds, amount)) in spell_power.into_iter().zip(SLAYER_SPELL_POWER)
    {
        let want: Vec<i32> = (1..=9)
            .map(|t| if kinds.contains(&t) { amount as i32 } else { 0 })
            .collect();
        assert_eq!(by_type, want, "{id}");
        assert!(unshown, "{id}: no spell power shown");
    }
}

const MANA_SHIELD: u32 = 1463;
const IMPROVED_MANA_SHIELD: [(u32, u32); 2] = [(11_252, 146), (12_605, 152)];
const FOOD_AND_DRINK: [(u32, u32, u32); 5] = [
    (24_355, 0, 2),
    (24_707, 3, 3),
    (25_990, 5, 5),
    (26_263, 4, 3),
    (29_055, 4, 3),
];
const SLOW_MANA: [(u32, u64, u32); 2] = [(18_792, 4, 2), (23_779, 3, 1)];
const RENEW: u32 = 139;
const CORRUPTION: u32 = 172;

/// `g`'s periodic effects of `spell`: (period, next tick, ticks done).
fn ticking(m: &Map, g: Guid, spell: u32) -> Vec<(Option<Duration>, Option<Instant>, u32)> {
    auras::auras(m, g)
        .map(|a| {
            a.effects()
                .filter(|(h, x)| h.spell == spell && x.period.is_some())
                .map(|(_, x)| (x.period, x.next_tick, x.ticks))
                .collect()
        })
        .unwrap_or_default()
}

/// Make `g`'s periodic effects of `spell` due now, as if their time had
/// come, `ticks` of them done.
fn come_due(m: &mut Map, g: Guid, spell: u32, ticks: u32) {
    let now = m.now;
    if let Some(a) = auras::auras_mut(m, g) {
        for h in a.holders.iter_mut().filter(|h| h.spell == spell) {
            for x in h.effects.iter_mut().filter(|x| x.period.is_some()) {
                x.next_tick = Some(now);
                x.ticks = ticks;
            }
        }
    }
}

/// Thistle Tea's shields, food and periodic cadence on the real spells
/// (`spell_mana_shield_dbc_test.exs`, `percent_mana_recovery_dbc_test.exs`,
/// `periodic_damage_ramps_dbc_test.exs`): Improved Mana Shield's ranks on
/// a live shield's mana a point; food and drink's shares of health and
/// mana a second; Fel Energy's and Resurgence's slower ticks; Renew and
/// Corruption cast again starting their interval over.
#[tokio::test]
#[ignore = "extended"]
async fn real_shields_food_and_periodic_cadence() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_m, mage, map) = enter(&realm, "P3JTICK1", "Ticka", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, mage, 50, &[]);
        let mana = |m: &Map| m.unit_fields(mage).unwrap().get_u32(UNIT_FIELD_POWER1);
        let set = |m: &mut Map, health: u32, mana: u32| {
            let f = m.unit_fields_mut(mage).unwrap();
            f.set_u32(UNIT_FIELD_HEALTH, health);
            f.set_u32(UNIT_FIELD_POWER1, mana);
        };
        let mut shields = Vec::new();
        for (talent, _) in IMPROVED_MANA_SHIELD {
            apply_real_at(m, mage, mage, talent, 50);
            set(m, 1000, 200);
            apply_real_at(m, mage, mage, MANA_SHIELD, 50);
            let soaked = auras::absorb(m, mage, 30, 1);
            shields.push((soaked, mana(m)));
            auras::remove_spell(m, mage, MANA_SHIELD, None);
            auras::remove_spell(m, mage, talent, None);
        }
        let max = |m: &Map| {
            let f = m.unit_fields(mage).unwrap();
            (
                f.get_u32(UNIT_FIELD_MAXHEALTH),
                f.get_u32(UNIT_FIELD_MAXPOWER1),
            )
        };
        let mut food = Vec::new();
        for (id, _, _) in FOOD_AND_DRINK {
            set(m, 100, 100);
            apply_real_at(m, mage, mage, id, 50);
            let period = ticking(m, mage, id).first().and_then(|t| t.0);
            come_due(m, mage, id, 0);
            auras::tick_due(m, mage, id, mage);
            let (mh, mm) = max(m);
            food.push((id, period, health(m, mage) - 100, mana(m) - 100, mh, mm));
            auras::remove_spell(m, mage, id, None);
        }
        let mut slow = Vec::new();
        for (id, _, _) in SLOW_MANA {
            set(m, 100, 100);
            apply_real_at(m, mage, mage, id, 50);
            let period = ticking(m, mage, id).first().and_then(|t| t.0);
            come_due(m, mage, id, 0);
            auras::tick_due(m, mage, id, mage);
            slow.push((id, period, mana(m) - 100, max(m).1));
            auras::remove_spell(m, mage, id, None);
        }
        let kobold = dummy(m, mage, KOBOLD_VERMIN, 50, 5.0);
        let mut again = Vec::new();
        for (id, on) in [(RENEW, mage), (CORRUPTION, kobold)] {
            spells::cast_triggered(m, mage, id, Some(on));
            // Two and a half seconds on, a tick done: cast again.
            let soon = m.now + Duration::from_millis(500);
            if let Some(a) = auras::auras_mut(m, on) {
                for h in a.holders.iter_mut().filter(|h| h.spell == id) {
                    for x in h.effects.iter_mut().filter(|x| x.period.is_some()) {
                        x.next_tick = Some(soon);
                        x.ticks = 1;
                    }
                }
            }
            spells::cast_triggered(m, mage, id, Some(on));
            let now = m.now;
            again.push((
                id,
                ticking(m, on, id)
                    .into_iter()
                    .map(|(p, next, ticks)| (p, next.map(|n| n - now), ticks))
                    .collect::<Vec<_>>(),
            ));
        }
        (shields, food, slow, again)
    })
    .await;
    let (shields, food, slow, again) = out;
    for ((soaked, mana), (talent, left)) in shields.into_iter().zip(IMPROVED_MANA_SHIELD) {
        assert_eq!(
            (soaked, mana),
            ((0, 30), left),
            "{talent}: less mana a point"
        );
    }
    for ((id, period, health, mana, mh, mm), (_, hp, mp)) in food.into_iter().zip(FOOD_AND_DRINK) {
        assert_eq!(period, Some(Duration::from_secs(1)), "{id}");
        assert_eq!((health, mana), (mh * hp / 100, mm * mp / 100), "{id}");
    }
    for ((id, period, mana, mm), (_, secs, pct)) in slow.into_iter().zip(SLOW_MANA) {
        assert_eq!(
            (period, mana),
            (Some(Duration::from_secs(secs)), mm * pct / 100),
            "{id}"
        );
    }
    let three = Some(Duration::from_secs(3));
    for (id, ticks) in again {
        assert_eq!(ticks, vec![(three, three, 0)], "{id} starts over");
    }
}

/// How a spell put on after another stands with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    Replaces,
    Blocked,
    Both,
}

/// Thistle Tea's spell groups on the real data (`spell_group_test.exs`,
/// `spell_group_dbc_test.exs`): food replacing food; a greater elixir, a
/// scroll and a stronger demoralizing shout or roar replacing a lesser, and
/// a lesser blocked by the greater, by the group's order rather than the
/// spells' ids (Demoralizing Shout's fifth rank and the Roar's sixth); Power
/// Infusion and Arcane Power; Blessing of Might's ranks beside Greater
/// Blessing of Might by its first rank's group; and an ordinary elixir
/// beside a greater one.
#[tokio::test]
#[ignore = "extended"]
async fn real_spell_groups_replace_and_block_by_their_order() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_a, a, map) = enter(&realm, "P3JGRP1", "Groupa", Race::Human, Class::Warrior).await;
    let (_b, b, _) = enter(&realm, "P3JGRP2", "Groupb", Race::Human, Class::Paladin).await;
    let pairs: [(u32, u32, Then); 14] = [
        (18_192, 19_710, Then::Replaces),
        (19_710, 18_192, Then::Replaces),
        (11_390, 17_539, Then::Replaces),
        (17_539, 11_390, Then::Blocked),
        (8118, 12_179, Then::Replaces),
        (12_179, 8118, Then::Blocked),
        (99, 1160, Then::Replaces),
        (11_556, 9898, Then::Blocked),
        (11_554, 9747, Then::Replaces),
        (10_060, 12_042, Then::Replaces),
        (12_042, 10_060, Then::Blocked),
        (25_291, 25_782, Then::Replaces),
        (19_836, 25_782, Then::Replaces),
        (25_782, 19_836, Then::Replaces),
    ];
    let out = on(&map, move |m| {
        ready(m, a, 60, &[]);
        ready(m, b, 60, &[]);
        let mut got = Vec::new();
        for (first, then, _) in pairs.iter().chain([(2374, 17_539, Then::Both)].iter()) {
            apply_real(m, b, a, *first);
            apply_real(m, a, a, *then);
            got.push(match (has(m, a, *first), has(m, a, *then)) {
                (false, true) => Then::Replaces,
                (true, false) => Then::Blocked,
                _ => Then::Both,
            });
            auras::remove_spell(m, a, *first, None);
            auras::remove_spell(m, a, *then, None);
        }
        got
    })
    .await;
    for ((first, then, want), got) in pairs
        .into_iter()
        .chain([(2374, 17_539, Then::Both)])
        .zip(out)
    {
        assert_eq!(got, want, "{then} after {first}");
    }
}

const MINING: u32 = 186;
const APPRENTICE_MINING: u32 = 2575;
const MINING_REWARDS: [u32; 3] = [2580, 2656, 2657];
const SMELT_SILVER: u32 = 2658;
const PICK_LOCK: u32 = 1804;
const LOCKPICKING: u32 = 633;
const POISONS_SPELL: u32 = 2842;
const POISONS: u32 = 40;
const INSTANT_POISON: u32 = 2835;
const MORTAL_STRIKE: u32 = 12_294;
const MORTAL_STRIKE_TRAINED: [u32; 2] = [21_551, 21_552];
const ARMS_TIER_ONE: u32 = 124;
const ARCANE_TIER_ONE: u32 = 74;

/// A skill's value and cap, if `g` has it.
fn skill(m: &Map, g: Guid, id: u32) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    Some((
        crate::game::map::skills::value(p, id)?,
        crate::game::map::skills::max_value(p, id)?,
    ))
}

/// Thistle Tea's skills and talents on the real data (`skill_test.exs`,
/// `rogue_skills_dbc_test.exs`, `talent_test.exs`): Mining brings Find
/// Minerals, Smelting and Smelt Copper with it, and unlearned takes them
/// and its recipes away, as only a skill flagged so may be; a rogue's Pick
/// Lock and Poisons bring Lockpicking and Poisons from 1, a recipe or a
/// warrior's none; a trained rank of a talent (Mortal Strike's) is the
/// talent's, to wipe with it; and a class learns only its own tabs'
/// talents.
#[tokio::test]
#[ignore = "extended"]
async fn real_skills_bring_their_spells_and_talents_their_ranks() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (w_client, w, map) =
        enter(&realm, "P3JSKILL1", "Skilla", Race::Human, Class::Warrior).await;
    let (_r, r, _) = enter(&realm, "P3JSKILL2", "Skillb", Race::Human, Class::Rogue).await;
    let mined = on(&map, move |m| {
        ready(m, w, 60, &[APPRENTICE_MINING, SMELT_SILVER]);
        let known = |m: &Map, s: &u32| m.players[&w].spells.contains(s);
        let data = m.realm.data.clone();
        (
            skill(m, w, MINING).is_some(),
            MINING_REWARDS.iter().all(|s| known(m, s)),
            [
                crate::game::map::misc::unlearnable(&data, MINING, 1, 1),
                crate::game::map::misc::unlearnable(&data, MINING, 2, 1),
                crate::game::map::misc::unlearnable(&data, 43, 1, 1),
                crate::game::map::misc::unlearnable(&data, 999, 1, 1),
            ],
        )
    })
    .await;
    assert_eq!(
        mined,
        (true, true, [true, true, false, false]),
        "mining and its rewards"
    );
    w_client
        .send(CMSG_UNLEARN_SKILL {
            skill: Skill::Mining,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let gone = on(&map, move |m| {
        let p = &m.players[&w];
        (
            skill(m, w, MINING),
            [APPRENTICE_MINING, SMELT_SILVER]
                .iter()
                .chain(MINING_REWARDS.iter())
                .any(|s| p.spells.contains(s)),
        )
    })
    .await;
    assert_eq!(
        gone,
        (None, false),
        "the skill, its rewards and recipes gone"
    );
    let rogue = on(&map, move |m| {
        ready(m, r, 20, &[INSTANT_POISON]);
        let recipe_alone = skill(m, r, POISONS);
        ready(m, r, 20, &[PICK_LOCK, POISONS_SPELL]);
        ready(m, w, 60, &[PICK_LOCK]);
        (
            recipe_alone,
            skill(m, r, LOCKPICKING),
            skill(m, r, POISONS),
            skill(m, w, LOCKPICKING),
        )
    })
    .await;
    assert_eq!(
        rogue,
        (None, Some((1, 100)), Some((1, 100)), None),
        "a rogue's, from 1; a recipe's or a warrior's none"
    );
    let talents = on(&map, move |m| {
        let data = m.realm.data.clone();
        spells::learn(m, w, MORTAL_STRIKE);
        for s in MORTAL_STRIKE_TRAINED {
            spells::learn(m, w, s);
        }
        let wiped = crate::game::map::misc::talent_spells(&m.players[&w], &data);
        let mortal = [MORTAL_STRIKE]
            .iter()
            .chain(MORTAL_STRIKE_TRAINED.iter())
            .all(|s| wiped.contains(s));
        let first = |id: u32| data.spells.talents[&id].ranks[0];
        spells::learn_talent(m, w, ARCANE_TIER_ONE, 0);
        spells::learn_talent(m, w, ARMS_TIER_ONE, 0);
        let p = &m.players[&w];
        (
            mortal,
            p.spells.contains(&first(ARCANE_TIER_ONE)),
            p.spells.contains(&first(ARMS_TIER_ONE)),
        )
    })
    .await;
    assert_eq!(
        talents,
        (true, false, true),
        "Mortal Strike's ranks are the talent's; a mage's talent isn't a warrior's"
    );
}

const BATTLEMASTER: u32 = 14_981;
const MOUNT_TRAINER: u32 = 4732;
const STORMWIND: u32 = 72;

/// Thistle Tea's gossip on the real data (`gossip_vmangos_test.exs`): a
/// battlemaster whose menu has no options of its own offers the default
/// menu's "I wish to join the battle!", as mangos's `PrepareGossipMenu`;
/// Moonglade's flight options each for their own side's druids; and a
/// race's mount trainer for that race, or another exalted with his
/// faction (`gossip_test.exs`'s `trainer_of?`).
#[tokio::test]
#[ignore = "extended"]
async fn real_gossip_defaults_side_conditions_and_mount_trainers() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut a, alliance, kalimdor) =
        enter(&realm, "P3JGOSS1", "Gossa", Race::NightElf, Class::Druid).await;
    let (_h, horde, _) = enter(&realm, "P3JGOSS2", "Gossb", Race::Tauren, Class::Druid).await;
    // Of the battleground's levels: a battlemaster's option is for those
    // (mangos's `CanInteractWithBattleMaster` in `PrepareGossipMenu`).
    let master = on(&kalimdor, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&alliance).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        dummy(m, alliance, BATTLEMASTER, 60, 2.0)
    })
    .await;
    a.sees_on(&kalimdor, master).await;
    a.send(CMSG_GOSSIP_HELLO { guid: master }).await;
    let gossips = a
        .wait_for("the battlemaster's menu", |msg| match msg {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) => {
                Some(g.gossips.clone())
            }
            _ => None,
        })
        .await;
    assert!(
        gossips
            .iter()
            .any(|g| g.id == 10 && g.message == "I wish to join the battle!"),
        "{gossips:?}"
    );
    let sides = on(&kalimdor, move |m| {
        let met = |id, g| crate::game::map::conditions::met(m, id, Some(g), Some(g));
        [
            met(232, alliance),
            met(232, horde),
            met(206, horde),
            met(206, alliance),
        ]
    })
    .await;
    assert_eq!(sides, [true, false, true, false], "each side's flight");
    // Randal Hunter's horses: for humans, a dwarf only at exalted with
    // Stormwind.
    let (_hu, human, eastern) =
        enter(&realm, "P3JGOSS3", "Gossc", Race::Human, Class::Warrior).await;
    let (_dw, dwarf, _) = enter(&realm, "P3JGOSS4", "Gossd", Race::Dwarf, Class::Warrior).await;
    let trained = on(&eastern, move |m| {
        let data = m.realm.data.clone();
        let randal = data.creature_template(MOUNT_TRAINER).unwrap();
        let trains = |m: &Map, g| crate::game::map::npc::trains(&data, &m.players[&g], randal);
        let before = (trains(m, human), trains(m, dwarf));
        crate::game::map::reputation::modify(m, dwarf, STORMWIND, 42_000, false);
        (before, trains(m, dwarf))
    })
    .await;
    assert_eq!(trained, ((true, false), true), "his race's, or the exalted");
}
