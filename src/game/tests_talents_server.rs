//! The server's gaps the talent audits found (`.gear`'s hands, talents'
//! effects that never took hold, the spells a quest or a book teaches),
//! each on the real data when it is here.

use std::sync::Arc;
use std::time::Duration;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::spells;
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

/// `g` at `level`, knowing `learn`.
fn ready(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
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

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn power(m: &Map, g: Guid, power: u16) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1 + power))
}

/// What `g` has on `c`'s threat list.
fn threat(m: &Map, c: Guid, g: Guid) -> f32 {
    m.creature(c)
        .and_then(|c| c.threat.iter().find(|(t, _)| *t == g).map(|(_, v)| *v))
        .unwrap_or(0.0)
}

/// The map's clock `by` on, and its auras' ticks to then.
fn later(m: &mut Map, by: Duration) {
    m.now += by;
    auras::update(m);
}

const KOBOLD_VERMIN: u32 = 6;

const ANGER_MANAGEMENT: u32 = 12_296;

/// Anger Management (12296, a regeneration of rage with no amplitude in
/// the DBC) gives three fifths of its 15 a tick, every 3 s, in combat
/// only (mangos's `HandleModPowerRegen` and `PeriodicTick`): 0.9 rage.
#[tokio::test]
#[ignore = "extended"]
async fn anger_management_gives_rage_every_three_seconds_in_combat() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, w, map) = enter(&realm, "TSRVAM1", "Angera", Race::Orc, Class::Warrior).await;
    let rage = on(&map, move |m| {
        ready(m, w, 60, &[ANGER_MANAGEMENT]);
        let set = |m: &mut Map, combat: bool| {
            let f = m.unit_fields_mut(w).unwrap();
            f.set_u32(UNIT_FIELD_POWER1 + 1, 0);
            f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, combat);
        };
        set(m, true);
        later(m, Duration::from_millis(3_000));
        let first = power(m, w, 1);
        later(m, Duration::from_millis(1_500));
        let between = power(m, w, 1);
        later(m, Duration::from_millis(1_500));
        let second = power(m, w, 1);
        set(m, false);
        later(m, Duration::from_millis(3_000));
        (first, between, second, power(m, w, 1))
    })
    .await;
    assert_eq!(
        rage,
        (9, 9, 18, 0),
        "rage at 3, 4.5 and 6 s, then out of combat"
    );
}

const THUNDER_CLAP: [u32; 6] = [6343, 8198, 8204, 8205, 11_580, 11_581];

const BATTLE_STANCE: u32 = 2457;

/// Every rank of Thunder Clap has its first's 2.5 times the threat of its
/// damage: `spell_threat` names rank 1 alone, and mangos-classic's
/// `FillHigherRanks` gives the rest its row, down the skill line's chain.
#[tokio::test]
#[ignore = "extended"]
async fn every_rank_of_thunder_clap_has_its_first_ranks_threat() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let rows: Vec<Option<(i32, f32)>> = THUNDER_CLAP
        .iter()
        .map(|id| realm.data.spells.threat.get(id).copied())
        .collect();
    assert!(
        rows.iter().all(|r| *r == Some((0, 2.5))),
        "each rank's row: {rows:?}"
    );
    let (_w, w, map) = enter(&realm, "TSRVTC1", "Clappa", Race::Human, Class::Warrior).await;
    let ratio = on(&map, move |m| {
        ready(m, w, 60, &[]);
        auras::remove_spell(m, w, BATTLE_STANCE, None);
        let kobold = dummy(m, w, KOBOLD_VERMIN, 60, 2.0);
        let before = health(m, kobold);
        spells::cast_triggered(m, w, THUNDER_CLAP[5], Some(w));
        let dealt = (before - health(m, kobold)) as f32;
        (dealt, threat(m, kobold, w) / dealt.max(1.0))
    })
    .await;
    assert!(ratio.0 > 0.0, "Thunder Clap hit: {ratio:?}");
    assert!(
        (ratio.1 - 2.5).abs() < 0.05,
        "rank 6's threat is 2.5 times its damage: {ratio:?}"
    );
}

const AUTO_SHOT: u32 = 75;

const MORTAL_SHOTS_5: u32 = 19_490;

/// Mortal Shots raises an Auto Shot's crit bonus, 30% at rank 5: twice
/// the shot without it, 2.3 times with it (mangos's `GetCritMultiplier`).
#[tokio::test]
#[ignore = "extended"]
async fn mortal_shots_raises_an_auto_shots_crit() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, h, map) = enter(&realm, "TSRVMS1", "Shota", Race::Dwarf, Class::Hunter).await;
    let dealt = on(&map, move |m| {
        ready(m, h, 60, &[]);
        let mut out = Vec::new();
        for (talent, crit) in [(false, false), (false, true), (true, true)] {
            if talent {
                spells::learn(m, h, MORTAL_SHOTS_5);
            }
            let kobold = dummy(m, h, KOBOLD_VERMIN, 60, 10.0);
            let before = health(m, kobold);
            crate::game::map::ranged::land_for_test(m, h, kobold, AUTO_SHOT, 1_000, crit);
            out.push(before - health(m, kobold));
        }
        out
    })
    .await;
    let (hit, crit, mortal) = (dealt[0] as f32, dealt[1] as f32, dealt[2] as f32);
    assert!(hit > 0.0, "{dealt:?}");
    assert!(
        (crit / hit - 2.0).abs() < 0.01,
        "a crit is twice a hit: {dealt:?}"
    );
    assert!(
        (mortal / hit - 2.3).abs() < 0.01,
        "with Mortal Shots, 2.3 times: {dealt:?}"
    );
}

/// The unit `a` swings at `b` once, a plain hit.
fn swing_at(m: &mut Map, a: Guid, b: Guid) {
    use crate::game::map::combat;
    let now = m.now;
    if let Some(c) = m.combat_mut(a) {
        c.next_swing = Some(now);
    }
    combat::FORCED_OUTCOME.with(|f| f.set(Some(combat::Outcome::Hit)));
    combat::start_attack(m, a, b);
    combat::FORCED_OUTCOME.with(|f| f.set(None));
    combat::stop_attack(m, a);
}

const BLOODTHIRST_4: u32 = 23_894;

const BLOODTHIRST_4_BUFF: u32 = 23_888;

/// Bloodthirst's rank 4 leaves its buff on the warrior (vmangos's
/// `spell_effect_mod`, mangos's `EffectSchoolDMG`), and each of the next
/// five hits heals them 20 (23891), the buff gone after.
#[tokio::test]
#[ignore = "extended"]
async fn bloodthirst_heals_on_each_of_the_next_five_hits() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, w, map) = enter(&realm, "TSRVBT1", "Thirsta", Race::Orc, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, w, 60, &[BLOODTHIRST_4]);
        let kobold = dummy(m, w, KOBOLD_VERMIN, 60, 2.0);
        spells::cast_triggered(m, w, BLOODTHIRST_4, Some(kobold));
        let buffed = auras::auras(m, w).is_some_and(|a| a.has_spell(BLOODTHIRST_4_BUFF));
        let max = m.unit_fields(w).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
        let mut gains = Vec::new();
        for _ in 0..6 {
            m.unit_fields_mut(w)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, max / 2);
            swing_at(m, w, kobold);
            gains.push(health(m, w) - max / 2);
        }
        let left = auras::auras(m, w).is_some_and(|a| a.has_spell(BLOODTHIRST_4_BUFF));
        (buffed, gains, left)
    })
    .await;
    assert!(out.0, "Bloodthirst's buff on the warrior");
    assert_eq!(out.1, vec![20, 20, 20, 20, 20, 0], "a heal a hit, five");
    assert!(!out.2, "the buff gone after five");
}

const SUMMON_IMP: u32 = 688;

const MASTER_DEMONOLOGIST_5: u32 = 23_825;

const MASTER_DEMONOLOGIST_5_IMP: u32 = 23_829;

const SPIRIT_BOND_2: u32 = 20_895;

const SPIRIT_BOND_2_AURA: u32 = 24_529;

/// `g`'s pet, if out.
fn pet_of(m: &Map, g: Guid) -> Option<Guid> {
    m.players.get(&g).and_then(|p| p.owned.pet)
}

/// Whether `g` has `spell`'s aura from `caster`.
fn has_from(m: &Map, g: Guid, spell: u32, caster: Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| {
        a.holders
            .iter()
            .any(|h| h.spell == spell && h.caster == caster)
    })
}

/// A `spell_pet_auras` aura that is the pet's area aura is cast by the
/// pet, and so reaches its master too (mangos's `Pet::CastPetAura`):
/// Master Demonologist's imp half on the warlock as on the imp, Spirit
/// Bond's regeneration on the hunter as on the beast.
#[tokio::test]
#[ignore = "extended"]
async fn a_pet_area_aura_of_a_masters_talent_reaches_the_master() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, w, map) = enter(&realm, "TSRVMD1", "Demona", Race::Human, Class::Warlock).await;
    let (_h, h, _) = enter(&realm, "TSRVSB1", "Bonda", Race::Dwarf, Class::Hunter).await;
    let out = on(&map, move |m| {
        ready(m, w, 60, &[SUMMON_IMP, MASTER_DEMONOLOGIST_5]);
        spells::cast_triggered(m, w, SUMMON_IMP, None);
        ready(m, h, 60, &[SPIRIT_BOND_2]);
        crate::game::map::gm_dev::command(m, h, "pet", &[]);
        crate::game::map::pets::refresh_area_auras(m);
        let imp = pet_of(m, w).expect("the imp");
        let beast = pet_of(m, h).expect("the beast");
        (
            has_from(m, imp, MASTER_DEMONOLOGIST_5_IMP, imp),
            has_from(m, w, MASTER_DEMONOLOGIST_5_IMP, imp),
            has_from(m, beast, SPIRIT_BOND_2_AURA, beast),
            has_from(m, h, SPIRIT_BOND_2_AURA, beast),
        )
    })
    .await;
    assert_eq!(
        out,
        (true, true, true, true),
        "the imp's and warlock's Master Demonologist, the beast's and hunter's Spirit Bond"
    );
}

const SCORPID_STING_4: u32 = 14_277;

const IMPROVED_SCORPID_STING_3: u32 = 19_494;

const IMPROVED_SCORPID_STING_DRAIN: u32 = 19_486;

/// The amount of `spell`'s first effect on `g`, if it has the aura.
fn amount(m: &Map, g: Guid, spell: u32) -> Option<i32> {
    auras::auras(m, g).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.spell == spell)
            .and_then(|h| h.effects.first().map(|x| x.amount))
    })
}

/// Improved Scorpid Sting's rank 3: a Scorpid Sting takes 30% of the
/// strength it takes in stamina too (19486, mangos's `HandleAuraModStat`),
/// gone with the sting; without the talent, none.
#[tokio::test]
#[ignore = "extended"]
async fn improved_scorpid_sting_takes_stamina_with_the_sting() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, h, map) = enter(&realm, "TSRVSS1", "Stinga", Race::Dwarf, Class::Hunter).await;
    let out = on(&map, move |m| {
        ready(m, h, 60, &[SCORPID_STING_4]);
        let plain = dummy(m, h, KOBOLD_VERMIN, 60, 10.0);
        spells::cast_triggered(m, h, SCORPID_STING_4, Some(plain));
        let without = amount(m, plain, IMPROVED_SCORPID_STING_DRAIN);
        spells::learn(m, h, IMPROVED_SCORPID_STING_3);
        let stung = dummy(m, h, KOBOLD_VERMIN, 60, 10.0);
        spells::cast_triggered(m, h, SCORPID_STING_4, Some(stung));
        let strength = amount(m, stung, SCORPID_STING_4);
        let stamina = amount(m, stung, IMPROVED_SCORPID_STING_DRAIN);
        auras::remove_spell(m, stung, SCORPID_STING_4, None);
        let after = amount(m, stung, IMPROVED_SCORPID_STING_DRAIN);
        (without, strength, stamina, after)
    })
    .await;
    let (without, strength, stamina, after) = out;
    assert_eq!(without, None, "no drain without the talent");
    let strength = strength.expect("the sting's strength");
    assert!(strength < 0, "{strength}");
    assert_eq!(stamina, Some(strength * 30 / 100), "30% of {strength}");
    assert_eq!(after, None, "gone with the sting");
}
