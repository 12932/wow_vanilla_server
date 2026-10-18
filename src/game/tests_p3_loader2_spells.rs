//! Phase 3's second loader stage, the spells (Thistle Tea's
//! `world/loader/*_dbc_test.exs` and `spell_*_vmangos_test.exs` still
//! open): what wvs reads of the real spells, and what they do once cast,
//! over the real data when it is here.

use std::sync::Arc;
use std::time::Duration;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, mods};
use crate::game::map::{Map, MapHandle, MapKey, auras};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

fn spell(d: &GameData, id: u32) -> &SpellTemplate {
    d.spells.get(id).unwrap_or_else(|| panic!("spell {id}"))
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// A human (a dwarf hunter) of `class` at sixty in the Eastern Kingdoms, their powers full, and their map.
async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let race = if class == Class::Hunter {
        Race::Dwarf
    } else {
        Race::Human
    };
    let g = c.enter(name, race, class).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    on(&map, move |m| ready(m, g)).await;
    (c, g, map)
}

fn ready(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, 60);
    for i in 0..5 {
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
        p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
    }
}

/// A creature of `entry` at `level`, `yards` in front of `g`, facing it,
/// with health to spare.
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

/// `caster` puts real spell `id`'s auras, at their sixtieth-level points,
/// on `target`.
fn apply_real(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let s = m.realm.data.spells.get(id).cloned().unwrap();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default());
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn spells_on(m: &Map, g: Guid) -> Vec<u32> {
    auras::auras(m, g).map_or(Vec::new(), |a| a.holders.iter().map(|h| h.spell).collect())
}

const BESTIAL_WRATH: u32 = 19_574;
const WRATH_BOOSTS: [u32; 4] = [24_395, 24_396, 24_397, 26_592];
const POLYMORPH: u32 = 118;
const FROST_NOVA: u32 = 122;
const HAMMER_OF_JUSTICE: u32 = 853;
const FEAR: u32 = 5782;
const BLIND: u32 = 2094;
const FREEZING_TRAP_EFFECT: u32 = 3355;
const WOLF: u32 = 299;

/// Thistle Tea's Bestial Wrath (`spell_bestial_wrath_dbc_test.exs`): its
/// four immunities, of the mechanics their effects name and purging what
/// they guard against, go on with it for its 18 seconds; they take off
/// Polymorph, a stun, a fear, Blind and a trap's freeze and leave its
/// half again of size and damage; a whole control is then resisted, and
/// Frost Nova hurts without holding; at 18 seconds all of it goes, and
/// control takes again; and the pet's death takes it all, and its size.
#[tokio::test]
#[ignore = "extended"]
async fn real_bestial_wrath_guards_its_pet_for_its_duration() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    let wrath = spell(d, BESTIAL_WRATH);
    assert_eq!(d.spells.duration(wrath, 60), Some(18_000));
    let misc: Vec<Vec<i32>> = WRATH_BOOSTS
        .iter()
        .map(|id| {
            spell(d, *id)
                .effects
                .iter()
                .filter(|e| e.aura == auras::AURA_MECHANIC_IMMUNITY)
                .map(|e| e.misc_value)
                .collect()
        })
        .collect();
    assert_eq!(
        misc,
        vec![vec![1, 5, 17], vec![14, 13, 24], vec![7, 10, 11], vec![2]]
    );
    for id in WRATH_BOOSTS {
        let s = spell(d, id);
        assert!(!s.passive(), "{id}");
        assert!(s.attr(1, 0x8000), "{id}: the immunity purges");
    }

    let realm = realm();
    let (_h, hunter, map) = enter(&realm, "P3OWRAT1", "Wratha", Class::Hunter).await;
    let (_m, mage, _) = enter(&realm, "P3OWRAT2", "Novaa", Class::Mage).await;
    let out = on(&map, move |m| {
        let pet = dummy(m, mage, WOLF, 60, 4.0);
        let scale = |m: &Map| {
            m.unit_fields(pet)
                .map_or(0.0, |f| f.get_f32(OBJECT_FIELD_SCALE_X))
        };
        let damage = |m: &Map| {
            auras::auras(m, pet).map_or(0, |a| a.sum_mask(auras::AURA_MOD_DAMAGE_PERCENT_DONE, 1))
        };
        let before = scale(m);
        let mut cleansed = Vec::new();
        for id in [
            POLYMORPH,
            FROST_NOVA,
            HAMMER_OF_JUSTICE,
            FEAR,
            BLIND,
            FREEZING_TRAP_EFFECT,
        ] {
            apply_real(m, mage, pet, id);
            let on = has(m, pet, id);
            apply_real(m, hunter, pet, BESTIAL_WRATH);
            cleansed.push((
                id,
                on,
                has(m, pet, id),
                spells_on(m, pet),
                scale(m) / before,
                damage(m),
            ));
            auras::remove_spell(m, pet, BESTIAL_WRATH, None);
        }
        apply_real(m, hunter, pet, BESTIAL_WRATH);
        let t0 = m.now;
        let mut resisted = Vec::new();
        for id in [POLYMORPH, HAMMER_OF_JUSTICE, FEAR, BLIND] {
            spells::cast_triggered(m, mage, id, Some(pet));
            resisted.push(has(m, pet, id));
        }
        let health = m.unit_fields(pet).unwrap().get_u32(UNIT_FIELD_HEALTH);
        spells::cast_triggered(m, mage, FROST_NOVA, None);
        let nova = (
            m.unit_fields(pet).unwrap().get_u32(UNIT_FIELD_HEALTH) < health,
            has(m, pet, FROST_NOVA),
        );
        m.now = t0 + Duration::from_millis(17_900);
        auras::update(m);
        let late = spells_on(m, pet);
        m.now = t0 + Duration::from_millis(18_100);
        auras::update(m);
        let gone = (
            spells_on(m, pet),
            (scale(m) / before * 100.0).round(),
            damage(m),
        );
        apply_real(m, mage, pet, POLYMORPH);
        let again = has(m, pet, POLYMORPH);
        auras::remove_spell(m, pet, POLYMORPH, None);
        apply_real(m, hunter, pet, BESTIAL_WRATH);
        crate::game::map::combat::kill(m, mage, pet);
        let dead = (spells_on(m, pet), (scale(m) / before * 100.0).round());
        (cleansed, resisted, nova, late, gone, again, dead)
    })
    .await;
    let (cleansed, resisted, nova, late, gone, again, dead) = out;
    let mut guarded = vec![BESTIAL_WRATH];
    guarded.extend(WRATH_BOOSTS);
    for (id, on, after, holders, scale, damage) in cleansed {
        assert!(on && !after, "{id}: on, then cleansed");
        let mut holders = holders;
        holders.sort();
        let mut want = guarded.clone();
        want.sort();
        assert_eq!(holders, want, "{id}");
        assert!((scale - 1.5).abs() < 0.01, "{id}: {scale}");
        assert_eq!(damage, 50, "{id}");
    }
    assert_eq!(resisted, vec![false; 4], "immune");
    assert_eq!(nova, (true, false), "hurt, not held");
    assert!(guarded.iter().all(|s| late.contains(s)), "{late:?}");
    assert_eq!(gone, (vec![], 100.0, 0), "all gone at 18 seconds");
    assert!(again, "control takes again");
    assert_eq!(dead, (vec![], 100.0), "death takes it all");
}

/// Thistle Tea's class masks and interrupt flags from vmangos's
/// `spell_effect_mod` and `spell_mod` (`spell_modifier_masks_vmangos_test.exs`):
/// Improved Bloodrage's and Revenge's, Revealed Flaw's and Shadow
/// Weaving's, Plagueheart's whole 64 bits (Siphon Life's high word among
/// them, which its threat cut touches), and Vile Poisons' three; Cannibalize
/// ends on damage; Fire Ward and Ascendance's bonus keep their templates'
/// flags where the row says -1; 24322's are the row's explicit none.
#[test]
#[ignore = "extended"]
fn the_real_modifier_masks_and_interrupt_flags_load_as_thistle_tea_reads_them() {
    let Some(d) = real() else {
        return;
    };
    let mask = |id: u32, i: usize| spell(d, id).effects[i].item_type;
    assert_eq!(mask(12_301, 0), 256);
    assert_eq!(mask(12_818, 0), 256);
    assert_eq!(mask(28_844, 0), 1024);
    assert_eq!(mask(28_814, 0), 0x20000);
    assert_eq!(mask(15_334, 0), 42_508_288);
    let plagueheart = mask(28_746, 1);
    assert_eq!(plagueheart, 4_294_968_326);
    assert!(
        mods::touches(5, plagueheart, spell(d, 18_265)),
        "Siphon Life"
    );
    assert!(mods::touches(5, plagueheart, spell(d, 172)), "Corruption");
    assert!(!mods::touches(5, plagueheart, spell(d, 686)), "Shadow Bolt");
    assert_eq!(
        [0, 1, 2].map(|i| mask(16_720, i)),
        [73_728, 65_536, 268_550_144]
    );
    assert_eq!(spell(d, 20_578).aura_interrupt_flags, 2);
    // -1 keeps the template's: Fire Ward's none, Ascendance's bonus's
    // 0x80000.
    assert_eq!(spell(d, 543).aura_interrupt_flags, 0);
    assert_eq!(spell(d, 28_204).aura_interrupt_flags, 0x8_0000);
    assert_eq!(spell(d, 24_322).aura_interrupt_flags, 0);
}

const BARKSKIN: u32 = 22_812;
const BARKSKIN_EFFECT: u32 = 22_839;
const WRATH: u32 = 5176;
const KOBOLD_VERMIN: u32 = 6;

/// `g` hits for exactly `damage` with each hand.
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

/// Thistle Tea's Barkskin (`spell_linked_aura_dbc_test.exs`,
/// `spell_linked_aura_vmangos_test.exs`): vmangos's row links it (aura
/// 192) to 22839 for its 15 seconds, which slows swings by a quarter and
/// takes a fifth off physical blows; Barkskin itself adds a second to
/// Wrath's cast and holds a cast against pushback; at its end all of it
/// goes, and a blow pushes Wrath back again.
#[tokio::test]
#[ignore = "extended"]
async fn real_barkskin_brings_its_linked_aura_for_its_duration() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    let barkskin = spell(d, BARKSKIN);
    assert!(
        barkskin
            .effects
            .iter()
            .any(|e| e.aura == auras::AURA_LINKED && e.trigger_spell == BARKSKIN_EFFECT)
    );
    let linked = spell(d, BARKSKIN_EFFECT);
    assert_eq!(d.spells.duration(linked, 60), Some(15_000));
    assert_eq!(spells::points(linked, 0, 60), -25);
    assert_eq!(
        (spells::points(linked, 1, 60), linked.effects[1].misc_value),
        (-20, 1)
    );

    let realm = realm();
    let mut c = Client::connect(&realm, "P3OBARK");
    let druid = c.enter("Barka", Race::NightElf, Class::Druid).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    let out = on(&map, move |m| {
        ready(m, druid);
        let brute = dummy(m, druid, KOBOLD_VERMIN, 60, 3.0);
        hits_for(m, brute, 100.0);
        let swing = |m: &Map| m.players[&druid].fields.get_u32(UNIT_FIELD_BASEATTACKTIME);
        let blow = |m: &Map| {
            crate::game::map::combat::weapon_damage(
                m,
                brute,
                druid,
                crate::game::map::combat::Hand::Main,
                &mut rand::rng(),
            )
            .0
        };
        let data = m.realm.data.clone();
        let wrath = data.spells.get(WRATH).unwrap().clone();
        let cast_time = |m: &Map| {
            let mods = mods::for_spell(m, druid, &wrath);
            spells::cast_time(m, druid, &wrath, &mods)
        };
        // A cast of Wrath under way, and a blow: how far it is put off.
        let pushed = |m: &mut Map| {
            let now = m.now;
            if let Some(c) = m.caster_mut(druid) {
                c.casting = None;
                c.gcd_until = None;
            }
            spells::cast(
                m,
                druid,
                WRATH,
                spells::Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
                None,
            );
            let ends = |m: &Map| {
                m.caster(druid)
                    .and_then(|c| c.casting.as_ref())
                    .map(|c| c.ends)
            };
            // A second in, so that there is time to put back.
            m.now += Duration::from_secs(1);
            let before = ends(m);
            spells::pushback_by(m, druid, brute);
            let after = ends(m);
            if let Some(c) = m.caster_mut(druid) {
                c.casting = None;
            }
            let _ = now;
            before.zip(after).map(|(b, a)| a.duration_since(b))
        };
        let base = (swing(m), blow(m), cast_time(m), pushed(m));
        apply_real(m, druid, druid, BARKSKIN);
        let holders = spells_on(m, druid);
        let on = (swing(m), blow(m), cast_time(m), pushed(m));
        let t0 = m.now;
        m.now = t0 + Duration::from_millis(15_100);
        auras::update(m);
        let after = (swing(m), blow(m), cast_time(m), pushed(m));
        let left = spells_on(m, druid);
        (base, holders, on, after, left)
    })
    .await;
    let (base, holders, on, after, left) = out;
    assert!(
        holders.contains(&BARKSKIN) && holders.contains(&BARKSKIN_EFFECT),
        "{holders:?}"
    );
    assert_eq!(on.0, (base.0 as f32 * 1.25) as u32, "swings slowed");
    assert_eq!(on.1, base.1 * 4 / 5, "a fifth off");
    assert_eq!(on.2, base.2 + 1000, "a second on Wrath");
    assert!(base.3.is_some_and(|d| d > Duration::ZERO), "{base:?}");
    assert_eq!(on.3, Some(Duration::ZERO), "no pushback");
    assert_eq!(after, base, "all as it was");
    assert!(
        !left.contains(&BARKSKIN) && !left.contains(&BARKSKIN_EFFECT),
        "{left:?}"
    );
}

const SEAL_OF_THE_CRUSADER: u32 = 21_082;
const THUNDER_CLAP: u32 = 6343;
const MALGENS_LONG_BOW: u32 = 22_318;

/// Thistle Tea's two hastes (`spell_linked_aura_dbc_test.exs`): Seal of
/// the Crusader's attack speed (aura 9) is every attack's, Thunder Clap's
/// melee haste (138) the swing's alone.
#[tokio::test]
#[ignore = "extended"]
async fn real_attack_speed_is_every_attacks_and_melee_haste_the_swings() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    assert!(spell(d, SEAL_OF_THE_CRUSADER).has_aura(auras::AURA_MOD_ATTACKSPEED));
    assert!(spell(d, THUNDER_CLAP).has_aura(auras::AURA_MOD_HASTE));
    let realm = realm();
    let (_h, hunter, map) = enter(&realm, "P3OHAST", "Hasta", Class::Hunter).await;
    let out = on(&map, move |m| {
        let data = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = m.players.get_mut(&hunter).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &data,
            guid::item(counter),
            hunter,
            MALGENS_LONG_BOW,
            crate::game::store::BAG_PLAYER,
            17,
            1,
        ));
        p.refresh_inventory(&data);
        crate::game::map::inventory::sync_gear(m, hunter);
        crate::game::map::derive::unit(m, hunter);
        let times = |m: &Map| {
            let f = &m.players[&hunter].fields;
            (
                f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                f.get_u32(UNIT_FIELD_RANGEDATTACKTIME),
            )
        };
        let base = times(m);
        let mut out = vec![base];
        for id in [SEAL_OF_THE_CRUSADER, THUNDER_CLAP] {
            apply_real(m, hunter, hunter, id);
            crate::game::map::derive::unit(m, hunter);
            out.push(times(m));
            auras::remove_spell(m, hunter, id, None);
            crate::game::map::derive::unit(m, hunter);
        }
        out
    })
    .await;
    let (base, seal, clap) = (out[0], out[1], out[2]);
    assert!(seal.0 != base.0 && seal.1 != base.1, "{out:?}");
    assert!(clap.0 > base.0 && clap.1 == base.1, "{out:?}");
}

const CREATURE_BEAR_FORM: u32 = 19_030;
const GHOST_WOLF: u32 = 2645;
const ORB_OF_DECEPTION: u32 = 16_739;

/// Thistle Tea's looks (`spell_appearance_dbc_test.exs`): a creature's
/// bear form is the creature bear's (902), and its own model comes back;
/// Ghost Wolf is the wolf's (4613); the Orb of Deception takes every race
/// and sex to the other side's, each to its own model, and back.
#[tokio::test]
#[ignore = "extended"]
async fn real_forms_and_the_orb_of_deception_change_the_look() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_s, shaman, kalimdor) = {
        let mut c = Client::connect(&realm, "P3OLOOK0");
        let g = c.enter("Looka", Race::Orc, Class::Shaman).await;
        (c, g, realm.maps.get(&realm, MapKey::world(1)))
    };
    let (bear, wolf) = on(&kalimdor, move |m| {
        ready(m, shaman);
        let display = |m: &Map, g: Guid| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_DISPLAYID);
        let c = dummy(m, shaman, KOBOLD_VERMIN, 60, 3.0);
        let own = display(m, c);
        apply_real(m, c, c, CREATURE_BEAR_FORM);
        let bear = display(m, c);
        auras::remove_spell(m, c, CREATURE_BEAR_FORM, None);
        let back = display(m, c);
        apply_real(m, shaman, shaman, GHOST_WOLF);
        let wolf = display(m, shaman);
        ((own, bear, back), wolf)
    })
    .await;
    assert_eq!((bear.1, bear.2), (902, bear.0));
    assert_eq!(wolf, 4613);

    let mut looks = Vec::new();
    // Each race's own models, male and female, and the orb's for each.
    for (i, (race, native, want)) in [
        (Race::Human, [49, 50], [10_137, 10_138]),
        (Race::Orc, [51, 52], [10_139, 10_140]),
        (Race::Dwarf, [53, 54], [10_141, 10_142]),
        (Race::NightElf, [55, 56], [10_143, 10_144]),
        (Race::Undead, [57, 58], [10_146, 10_145]),
        (Race::Tauren, [59, 60], [10_136, 10_147]),
        (Race::Gnome, [1563, 1564], [10_148, 10_149]),
        (Race::Troll, [1478, 1479], [10_135, 10_134]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = Client::connect(&realm, &format!("P3OLOOK{}", i + 1));
        let g = c
            .enter(
                &format!("Orb{}", (b'a' + i as u8) as char),
                race,
                Class::Warrior,
            )
            .await;
        let map = realm.registry.online(g).unwrap().map();
        let seen = on(&map, move |m| {
            let display = |m: &Map| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_DISPLAYID);
            native.map(|model| {
                let f = m.unit_fields_mut(g).unwrap();
                f.set_u32(UNIT_FIELD_NATIVEDISPLAYID, model);
                f.set_u32(UNIT_FIELD_DISPLAYID, model);
                apply_real(m, g, g, ORB_OF_DECEPTION);
                let orb = display(m);
                auras::remove_spell(m, g, ORB_OF_DECEPTION, None);
                (orb, display(m) == model)
            })
        })
        .await;
        looks.push((race, seen, want));
        drop(c);
    }
    assert!(
        looks
            .iter()
            .all(|(_, seen, want)| seen[0] == (want[0], true) && seen[1] == (want[1], true)),
        "{looks:?}"
    );
}

/// A spell of `effects` (effect, aura, points, trigger) for the linked
/// auras' test, lasting `duration_index`'s time.
fn linking(id: u32, effects: &[(u32, u32, i32, u32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Link {id}"),
        duration_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    for (i, (effect, aura, points, trigger)) in effects.iter().enumerate() {
        s.effects[i].effect = *effect;
        s.effects[i].aura = *aura;
        s.effects[i].base_points = *points - 1;
        s.effects[i].base_dice = 1;
        s.effects[i].target_a = 1;
        s.effects[i].trigger_spell = *trigger;
    }
    s
}

/// Thistle Tea's linked auras' edges (`spell_linked_aura_dbc_test.exs`):
/// a link back to its parent is cut, the linked spell's own effects kept
/// (its haste on), and a link to a spell there isn't brings nothing.
#[tokio::test]
async fn a_linked_aura_cycle_is_cut_and_a_missing_link_brings_nothing() {
    let mut d = GameData::empty();
    d.spells.durations.insert(
        1,
        crate::game::data::spells::SpellDuration {
            base: 15_000,
            per_level: 0,
            max: 15_000,
        },
    );
    let (parent, child, lonely) = (90_001, 90_002, 90_003);
    for s in [
        linking(parent, &[(6, auras::AURA_LINKED, 0, child)]),
        linking(
            child,
            &[
                (6, auras::AURA_MOD_HASTE, -25, 0),
                (6, auras::AURA_LINKED, 0, parent),
            ],
        ),
        linking(lonely, &[(6, auras::AURA_LINKED, 0, 999_999)]),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let realm = harness::realm_with(d);
    let mut c = Client::connect(&realm, "P3OLINK");
    let g = c.enter("Linka", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        apply_real(m, g, g, parent);
        let cycle = spells_on(m, g);
        let haste = auras::auras(m, g).map_or(0, |a| a.sum(auras::AURA_MOD_HASTE));
        apply_real(m, g, g, lonely);
        (cycle, haste, spells_on(m, g))
    })
    .await;
    let (cycle, haste, after) = out;
    assert_eq!(
        cycle.iter().filter(|s| **s == parent).count(),
        1,
        "{cycle:?}"
    );
    assert_eq!(
        cycle.iter().filter(|s| **s == child).count(),
        1,
        "{cycle:?}"
    );
    assert_eq!(haste, -25);
    assert!(
        after.contains(&lonely) && !after.contains(&999_999),
        "{after:?}"
    );
}
