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

const BEAR_FORM: u32 = 5487;

const BEAR_FORM_THREAT: u32 = 21_178;

const FERAL_INSTINCT_5: u32 = 16_951;

/// Feral Instinct's rank 5 raises Bear Form's threat passive (21178) by 15,
/// 30 to 45 (its modifier of all effects, mangos's
/// `CalculateSpellEffectValue`): taken into the form; and learned or lost
/// in it, the passive is put on again (mangos's
/// `ReapplyAffectedPassiveAuras`).
#[tokio::test]
#[ignore = "extended"]
async fn feral_instinct_raises_bear_forms_threat() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_d, d, map) = enter(&realm, "TSRVFI1", "Bruina", Race::NightElf, Class::Druid).await;
    let out = on(&map, move |m| {
        ready(m, d, 60, &[BEAR_FORM]);
        spells::cast_triggered(m, d, BEAR_FORM, None);
        let plain = amount(m, d, BEAR_FORM_THREAT);
        spells::learn(m, d, FERAL_INSTINCT_5);
        let learned_in_form = amount(m, d, BEAR_FORM_THREAT);
        auras::remove_spell(m, d, BEAR_FORM, None);
        spells::cast_triggered(m, d, BEAR_FORM, None);
        let taken_into_it = amount(m, d, BEAR_FORM_THREAT);
        auras::remove_spell(m, d, FERAL_INSTINCT_5, None);
        let lost = amount(m, d, BEAR_FORM_THREAT);
        (plain, learned_in_form, taken_into_it, lost)
    })
    .await;
    assert_eq!(
        out,
        (Some(30), Some(45), Some(45), Some(30)),
        "the form's threat: plain, the talent learned in it, the form taken with it, the talent gone"
    );
}

const IMPROVED_FIRESTONE_2: u32 = 18_768;

const MAJOR_FIRESTONE: u32 = 13_701;

const MAJOR_FIRESTONE_FIRE: u32 = 23_483;

/// Improved Firestone's rank 2 raises a worn Firestone's fire damage, the
/// Major's 21 to 27 (its 30% damage modifier on the item's aura); without
/// it, or the talent gone, 21.
#[tokio::test]
#[ignore = "extended"]
async fn improved_firestone_raises_a_worn_firestones_fire_damage() {
    if !have_data() {
        return;
    }
    use crate::game::map::inventory::{self, Destination, SLOT_OFF_HAND};
    let realm = realm();
    let (_w, w, map) = enter(&realm, "TSRVIF1", "Stona", Race::Human, Class::Warlock).await;
    let out = on(&map, move |m| {
        ready(m, w, 60, &[]);
        let worn = |m: &mut Map| {
            inventory::add_item_to(m, w, MAJOR_FIRESTONE, 1, Destination::Worn(SLOT_OFF_HAND))
                .expect("the Firestone worn");
            inventory::sync_gear(m, w);
            amount(m, w, MAJOR_FIRESTONE_FIRE)
        };
        let plain = worn(m);
        let i = m.players[&w]
            .item_at(crate::game::store::BAG_PLAYER, SLOT_OFF_HAND)
            .unwrap();
        inventory::take_item(m, w, i, 1);
        spells::learn(m, w, IMPROVED_FIRESTONE_2);
        let talented = worn(m);
        auras::remove_spell(m, w, IMPROVED_FIRESTONE_2, None);
        let lost = amount(m, w, MAJOR_FIRESTONE_FIRE);
        (plain, talented, lost)
    })
    .await;
    assert_eq!(
        out,
        (Some(21), Some(27), Some(21)),
        "the Firestone's fire damage: plain, with the talent, the talent gone"
    );
}

const DRAIN_MANA_4: u32 = 11_704;

const IMPROVED_DRAIN_MANA_2: u32 = 18_393;

/// A caster with mana: a Kobold Geomancer.
const KOBOLD_GEOMANCER: u32 = 476;

/// Improved Drain Mana's rank 2: each tick of Drain Mana deals 30% of the
/// mana it took as damage too; without the talent, none.
#[tokio::test]
#[ignore = "extended"]
async fn improved_drain_mana_deals_a_share_of_the_mana_drained() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, w, map) = enter(&realm, "TSRVDM1", "Draina", Race::Human, Class::Warlock).await;
    let out = on(&map, move |m| {
        ready(m, w, 60, &[DRAIN_MANA_4]);
        let mut out = Vec::new();
        for talent in [false, true] {
            if talent {
                spells::learn(m, w, IMPROVED_DRAIN_MANA_2);
            }
            let caster = dummy(m, w, KOBOLD_GEOMANCER, 60, 10.0);
            let f = m.unit_fields_mut(caster).unwrap();
            f.set_u32(UNIT_FIELD_MAXPOWER1, 100_000);
            f.set_u32(UNIT_FIELD_POWER1, 100_000);
            spells::cast_triggered(m, w, DRAIN_MANA_4, Some(caster));
            let (hp, mana) = (health(m, caster), power(m, caster, 0));
            later(m, Duration::from_millis(1_000));
            out.push((hp - health(m, caster), mana - power(m, caster, 0)));
        }
        out
    })
    .await;
    let (plain, talented) = (out[0], out[1]);
    assert!(plain.1 > 0 && talented.1 > 0, "mana drained: {out:?}");
    assert_eq!(plain.0, 0, "no damage without the talent: {out:?}");
    assert_eq!(
        talented.0,
        talented.1 * 30 / 100,
        "30% of the drained: {out:?}"
    );
}

/// A race, class and level, the spells they should know and those they
/// shouldn't.
type Case = (Race, Class, u8, &'static [u32], &'static [u32]);

/// `.character spells` on `g`, and whether they then know each of `spells`.
fn knows_after_character_spells(m: &mut Map, g: Guid, spells: &[u32]) -> Vec<(u32, bool)> {
    crate::game::map::gm_dev::command(m, g, "character", &["spells"]);
    spells
        .iter()
        .map(|s| (*s, m.players[&g].spells.contains(s)))
        .collect()
}

/// `.character spells` teaches the class spells quests and books teach
/// and no trainer does, by race and level, and the trainers' later ranks
/// of them: each priest race's two racial spells at their top ranks at 60
/// (Desperate Prayer and Feedback, Fear Ward, Starshards and Elune's
/// Grace, Touch of Weakness and Devouring Plague, Hex of Weakness and
/// Shadowguard), Prayer of Fortitude's second rank from its codex; a
/// hunter's Tranquilizing Shot from its book; a paladin's Redemption, its
/// first rank a quest's, to its fifth; a warrior's Intercept; and nothing
/// above a character's level (a priest of 10, no Feedback).
#[tokio::test]
#[ignore = "extended"]
async fn character_spells_teaches_the_quests_and_books_class_spells() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let cases: [Case; 9] = [
        (
            Race::Human,
            Class::Priest,
            60,
            &[19_243, 19_275, 21_564],
            &[6346, 19_305],
        ),
        (Race::Dwarf, Class::Priest, 60, &[19_243, 6346], &[19_275]),
        (
            Race::NightElf,
            Class::Priest,
            60,
            &[19_305, 19_293],
            &[19_243],
        ),
        (Race::Undead, Class::Priest, 60, &[19_266, 19_280], &[]),
        (Race::Troll, Class::Priest, 60, &[19_285, 19_312], &[]),
        (
            Race::Human,
            Class::Priest,
            10,
            &[13_908],
            &[13_896, 19_275, 21_562],
        ),
        (Race::Dwarf, Class::Hunter, 60, &[19_801], &[]),
        (Race::Human, Class::Paladin, 60, &[7328, 20_773], &[]),
        (Race::Orc, Class::Warrior, 30, &[20_252, 2458], &[]),
    ];
    let mut wrong = Vec::new();
    for (i, (race, class, level, known, unknown)) in cases.into_iter().enumerate() {
        let (_c, g, map) = enter(
            &realm,
            &format!("TSRVCS{i}"),
            &format!("Learna{}", (b'a' + i as u8) as char),
            race,
            class,
        )
        .await;
        let spells: Vec<u32> = known.iter().chain(unknown).copied().collect();
        let got = on(&map, move |m| {
            ready(m, g, level, &[]);
            knows_after_character_spells(m, g, &spells)
        })
        .await;
        for (spell, has) in got {
            if has != known.contains(&spell) {
                wrong.push((race, class, level, spell, has));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "(race, class, level, spell, known): {wrong:?}"
    );
}

const COMMAND_HUNTER: u32 = 20_576;

const COMMAND_WARLOCK: u32 = 20_575;

const HUNTER_PET_PASSIVE: u32 = 8875;

const DEMON_PASSIVES: [u32; 4] = [18_727, 18_728, 18_729, 18_730];

const FEL_STAMINA_5: u32 = 18_752;

const IMP_HEALTH_PASSIVE: u32 = 18_737;

/// The amounts of `g`'s auras of `spells`.
fn amounts_of(m: &Map, g: Guid, spells: &[u32]) -> Vec<(u32, i32)> {
    auras::auras(m, g).map_or_else(Vec::new, |a| {
        a.holders
            .iter()
            .filter(|h| spells.contains(&h.spell))
            .flat_map(|h| h.effects.iter().map(move |x| (h.spell, x.amount)))
            .collect()
    })
}

/// The orcs' Command: its modifiers (5 to all effects of the pets' hidden
/// passives) give an orc hunter's beast and an orc warlock's imp 5% more
/// damage (8875's and the demon passive's damage done, 0 without it): a
/// demon has its family's passives too (mangos's `LearnPetPassives`), and
/// so Fel Stamina's 15% more health at rank 5 on the imp's (18737).
#[tokio::test]
#[ignore = "extended"]
async fn command_gives_an_orcs_pet_five_percent_damage() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_h, h, map) = enter(&realm, "TSRVCM1", "Comma", Race::Orc, Class::Hunter).await;
    let (_w, w, _) = enter(&realm, "TSRVCM2", "Commb", Race::Orc, Class::Warlock).await;
    let (_t, t, _) = enter(&realm, "TSRVCM3", "Commc", Race::Troll, Class::Hunter).await;
    let (_f, f, hmap) = enter(&realm, "TSRVCM4", "Commd", Race::Human, Class::Warlock).await;
    let stamina = on(&hmap, move |m| {
        ready(m, f, 60, &[SUMMON_IMP, FEL_STAMINA_5]);
        spells::cast_triggered(m, f, SUMMON_IMP, None);
        amounts_of(m, pet_of(m, f).unwrap(), &[IMP_HEALTH_PASSIVE])
    })
    .await;
    assert_eq!(
        stamina,
        vec![(IMP_HEALTH_PASSIVE, 15)],
        "Fel Stamina on the imp"
    );
    let out = on(&map, move |m| {
        ready(m, h, 60, &[]);
        ready(m, t, 60, &[]);
        ready(m, w, 60, &[SUMMON_IMP]);
        let racial = (
            m.players[&h].spells.contains(&COMMAND_HUNTER),
            m.players[&w].spells.contains(&COMMAND_WARLOCK),
        );
        crate::game::map::gm_dev::command(m, h, "pet", &[]);
        crate::game::map::gm_dev::command(m, t, "pet", &[]);
        spells::cast_triggered(m, w, SUMMON_IMP, None);
        let beast = amounts_of(m, pet_of(m, h).unwrap(), &[HUNTER_PET_PASSIVE]);
        let trolls = amounts_of(m, pet_of(m, t).unwrap(), &[HUNTER_PET_PASSIVE]);
        let imp = amounts_of(m, pet_of(m, w).unwrap(), &DEMON_PASSIVES);
        (racial, beast, trolls, imp)
    })
    .await;
    assert_eq!(out.0, (true, true), "the orcs know Command");
    assert_eq!(out.1, vec![(HUNTER_PET_PASSIVE, 5)], "an orc's beast");
    assert_eq!(out.2, vec![(HUNTER_PET_PASSIVE, 0)], "a troll's beast");
    assert!(
        !out.3.is_empty() && out.3.iter().all(|(_, a)| *a == 5),
        "an orc's imp: {:?}",
        out.3
    );
}

/// The item class and subclass of what is held in the main and off hands.
type Held = (Option<(u32, u32)>, Option<(u32, u32)>);

/// What `g` holds in their main and off hands.
fn hands_held(m: &Map, g: Guid) -> Held {
    use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
    let p = &m.players[&g];
    let of = |slot: u8| {
        p.item_at(crate::game::store::BAG_PLAYER, slot)
            .and_then(|i| m.realm.data.item(p.items[i].entry))
            .map(|t| (t.class, t.subclass))
    };
    (of(SLOT_MAIN_HAND), of(SLOT_OFF_HAND))
}

/// `.gear` with `args` on `g`, `times` times: what they held each time,
/// and its last reply.
fn geared(m: &mut Map, g: Guid, args: &[&str], times: usize) -> (Vec<Held>, String) {
    let mut held = Vec::new();
    let mut reply = String::new();
    for _ in 0..times {
        reply = crate::game::map::gm_dev::command(m, g, "gear", args)
            .flatten()
            .unwrap_or_default();
        held.push(hands_held(m, g));
    }
    (held, reply)
}

const WEAPON: u32 = 2;

const ARMOR: u32 = 4;

const SHIELD: u32 = 6;

const PROTECTION_TALENTS: [u32; 2] = [12_753, 12_727];

const RESTORATION_TALENTS: [u32; 2] = [16_229, 16_217];

const ARMS_TALENT: u32 = 16_466;

/// `.gear`'s hands: `tank` gives a warrior a one-hander and a shield,
/// `healer` and `caster` a paladin and a shaman, `shield` anyone who may
/// carry one, `raid tank` too; with no word, a warrior or a shaman with
/// most of their talents in Protection or Restoration carries a shield,
/// and an Arms warrior none; `dps` never a shield; `sword` and `dagger`
/// arm a rogue with swords in both hands or a dagger in the main; and a
/// word it doesn't know, or two of a kind, is its usage.
#[tokio::test]
#[ignore = "extended"]
async fn gear_holds_what_its_role_spec_or_weapon_wants() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, w, map) = enter(&realm, "TSRVGR1", "Geara", Race::Human, Class::Warrior).await;
    let (_p, p, _) = enter(&realm, "TSRVGR2", "Gearb", Race::Human, Class::Paladin).await;
    let (_r, r, _) = enter(&realm, "TSRVGR3", "Gearc", Race::Human, Class::Rogue).await;
    let (_s, s, hmap) = enter(&realm, "TSRVGR4", "Geard", Race::Orc, Class::Shaman).await;
    let (_a, a, _) = enter(&realm, "TSRVGR5", "Geare", Race::Orc, Class::Warrior).await;
    let out = on(&map, move |m| {
        for g in [w, p, r] {
            ready(m, g, 60, &[]);
        }
        let shielded = |held: &[Held]| {
            held.iter().all(|(main, off)| {
                main.is_some_and(|(c, _)| c == WEAPON) && *off == Some((ARMOR, SHIELD))
            })
        };
        let unshielded = |held: &[Held]| held.iter().all(|(_, off)| *off != Some((ARMOR, SHIELD)));
        let tank = shielded(&geared(m, w, &["tank"], 5).0);
        let raid_tank = shielded(&geared(m, w, &["raid", "tank"], 2).0);
        let dps = unshielded(&geared(m, w, &["dps"], 5).0);
        let healer = shielded(&geared(m, p, &["healer"], 5).0);
        let shield = shielded(&geared(m, p, &["blue", "shield"], 3).0);
        let swords = geared(m, r, &["sword"], 5)
            .0
            .iter()
            .all(|h| *h == (Some((WEAPON, 7)), Some((WEAPON, 7))));
        let dagger = geared(m, r, &["green", "dagger"], 5)
            .0
            .iter()
            .all(|(main, _)| *main == Some((WEAPON, 15)));
        for s in PROTECTION_TALENTS {
            spells::learn(m, w, s);
        }
        let protection = shielded(&geared(m, w, &[], 5).0);
        let usage = [&["sword", "dagger"][..], &["shield", "2h"], &["juggle"]]
            .iter()
            .map(|args| geared(m, w, args, 1).1)
            .all(|reply| reply.starts_with("Usage"));
        (
            tank, raid_tank, dps, healer, shield, swords, dagger, protection, usage,
        )
    })
    .await;
    assert_eq!(
        out,
        (true, true, true, true, true, true, true, true, true),
        "(tank, raid tank, dps, paladin healer, paladin shield, rogue swords, rogue dagger, \
         a Protection warrior's plain .gear, usage)"
    );
    let out = on(&hmap, move |m| {
        ready(m, s, 60, &[]);
        ready(m, a, 60, &[ARMS_TALENT]);
        let caster = geared(m, s, &["caster"], 5)
            .0
            .iter()
            .all(|(_, off)| *off == Some((ARMOR, SHIELD)));
        for t in RESTORATION_TALENTS {
            spells::learn(m, s, t);
        }
        let restoration = geared(m, s, &[], 5)
            .0
            .iter()
            .all(|(_, off)| *off == Some((ARMOR, SHIELD)));
        let arms = geared(m, a, &[], 5)
            .0
            .iter()
            .all(|(_, off)| *off != Some((ARMOR, SHIELD)));
        (caster, restoration, arms)
    })
    .await;
    assert_eq!(
        out,
        (true, true, true),
        "(shaman caster, a Restoration shaman's plain .gear, an Arms warrior's plain .gear)"
    );
}
