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
