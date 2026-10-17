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
async fn real_teleports_to_a_caster_land_where_their_targets_say() {
    if !have_data() {
        return;
    }
    let mut d = crate::game::data::load::load(DB.as_path());
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
/// trained rank (Mortal Strike's) to one without the talent.
#[tokio::test]
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
            let left = outcome::split(m, brute, warrior, s, 100);
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
