//! Round 9, the core's pets and what they share with creatures and gear:
//! Thistle Tea's `attack_power_test.exs`, `swarmguard_dbc_test.exs`,
//! `power_leech_test.exs` and `equipment_stats_test.exs`: a pet's attack
//! power taking melee buffs and no ranged ones, an armed creature disarmed
//! on lowered attack power and its weapon's speed kept, a hunter's pet's
//! happiness counted once in a weapon strike, the Badge of the
//! Swarmguard's six stacks of Insight and their end with it, Drain Mana's
//! tick beside an aura damage breaks, and gear's outdoor spell held off
//! indoors with the item's armor kept.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemSpell;
use crate::game::data::pets::PetLevel;
use crate::game::data::spells::{ATTR_ONLY_OUTDOORS, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::pets::testing as hunter;
use crate::game::map::spells::environment;
use crate::game::map::{Map, derive, inventory, pets, procs, spells};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{
    BRUTE, aura_of, data, forced_swing, setup_with, spawn, template, waits,
};
use crate::game::tests_p3_spells_kit as kit;
use crate::game::tests_p4_items_gear::{item, passive, wear};

const WOLF: u32 = 964_101;
const VOIDWALKER: u32 = 964_102;
const IMP: u32 = 964_103;
/// A pet record's kinds (`summon::KIND_HUNTER`, `KIND_DEMON`).
const KIND_HUNTER: u8 = 1;
const KIND_DEMON: u8 = 2;
/// Happiness of a content pet (its damage as it is) and of a happy one
/// (a quarter more).
const CONTENT: u32 = 500_000;
const HAPPY: u32 = 700_000;

const AP_UP: u32 = 99_701;
const RANGED_AP_UP: u32 = 99_702;
const RANGED_AP_PCT: u32 = 99_703;
const AP_DOWN: u32 = 99_704;
const DISARM: u32 = 99_705;
const STRIKE_WEAPON: u32 = 99_706;

/// [`data`], with a wolf of the hunter pets' level 60 (180 attack power
/// from its 100 strength), a demon of the same and one of 55 strength (90),
/// and a weapon strike of no points of its own.
fn pets_world() -> GameData {
    let mut d = data();
    for (entry, name, family) in [
        (WOLF, "Wolf", 1),
        (VOIDWALKER, "Voidwalker", 16),
        (IMP, "Imp", 23),
    ] {
        let mut t = template(entry, if entry == WOLF { 1 } else { 3 });
        t.name = name.into();
        t.family = family;
        (t.level_min, t.level_max) = (60, 60);
        d.creature_templates.insert(entry, t);
    }
    for (row, strength, damage) in [
        (1, 100, (0.0, 0.0)),
        (VOIDWALKER, 100, (100.0, 120.0)),
        (IMP, 55, (50.0, 60.0)),
    ] {
        d.pets.levels.insert(
            (row, 60),
            PetLevel {
                health: 3_000,
                mana: if row == 1 { 0 } else { 1_000 },
                armor: 0,
                damage,
                stats: [strength, 50, 60, 30, 30],
            },
        );
    }
    let mut s = SpellTemplate {
        id: STRIKE_WEAPON,
        name: "Strike Weapon".into(),
        equipped_item_class: -1,
        range_index: 1,
        dmg_class: 2,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 58,
        base_points: -1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(STRIKE_WEAPON, s);
    d
}

/// `owner`'s pet of `entry` at level 60 from a record of `kind`, as one
/// called or summoned comes out, with `happiness` (a hunter's).
fn summon(m: &mut Map, owner: Guid, entry: u32, kind: u8, happiness: u32) -> Guid {
    let r = crate::game::store::PetRecord {
        number: entry,
        entry,
        kind,
        name: format!("Pet {entry}"),
        level: 60,
        happiness,
        loyalty: 1,
        loyalty_points: 1_000,
        ..Default::default()
    };
    m.players.get_mut(&owner).unwrap().row.state.pets.current = Some(r.clone());
    hunter::from_record(m, owner, &r).expect("the pet comes out")
}

/// Thistle Tea's "pets ignore ranged AP modifiers while retaining melee AP
/// buffs": a hunter's pet, a demon and an imp with a hundred attack power,
/// a hundred ranged attack power and half again as much ranged attack
/// power on them have their base and a hundred, the hundred shown as their
/// attack power's mods, and their ranged attack power as it was, none; and
/// working it out again changes nothing.
#[tokio::test]
async fn a_pets_attack_power_takes_its_melee_buffs_and_none_of_its_ranged_ones() {
    let (_realm, _a, g, map) = setup_with(pets_world(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for (entry, kind) in [
                (WOLF, KIND_HUNTER),
                (VOIDWALKER, KIND_DEMON),
                (IMP, KIND_DEMON),
            ] {
                let pet = summon(m, g, entry, kind, HAPPY);
                let read = |m: &Map| {
                    let c = m.creature(pet).unwrap();
                    let f = &c.fields;
                    (
                        c.derived.attack_power,
                        f.get_u32(UNIT_FIELD_ATTACK_POWER),
                        f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS),
                        c.derived.ranged_attack_power,
                        f.get_u32(UNIT_FIELD_RANGED_ATTACK_POWER),
                        f.get_u32(UNIT_FIELD_RANGED_ATTACK_POWER_MODS),
                    )
                };
                let bare = read(m);
                aura_of(m, pet, AP_UP, AURA_MOD_ATTACK_POWER, 0, 100);
                aura_of(m, pet, RANGED_AP_UP, AURA_MOD_RANGED_ATTACK_POWER, 0, 100);
                aura_of(
                    m,
                    pet,
                    RANGED_AP_PCT,
                    AURA_MOD_RANGED_ATTACK_POWER_PCT,
                    0,
                    50,
                );
                derive::unit(m, pet);
                let buffed = read(m);
                derive::unit(m, pet);
                let again = read(m);
                pets::dismiss(m, pet);
                out.push((entry, bare, buffed, again));
            }
            out
        })
        .await
        .unwrap();
    for ((entry, bare, buffed, again), base) in got.into_iter().zip([180, 180, 90]) {
        assert_eq!(bare, (base, base, 0, 0, 0, 0), "{entry}: as it came out");
        assert_eq!(
            buffed,
            (base + 100, base, 100, 0, 0, 0),
            "{entry}: the melee hundred alone"
        );
        assert_eq!(again, buffed, "{entry}: worked out again");
    }
}

/// Thistle Tea's "preserves creature weapon speed and layers disarm on
/// reduced AP": an armed creature of 200 attack power striking for 100
/// every three seconds, a hundred attack power down and disarmed, keeps its
/// three seconds and strikes for 100 by seven tenths and three tenths of a
/// half (85), by half (mangos-classic's, not Thistle Tea's four tenths):
/// 42, the half cut short as mangos's whole points are.
#[tokio::test]
async fn an_armed_creature_disarmed_on_lowered_attack_power_keeps_its_speed_and_strikes_for_half() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 60, 1.0);
            let c = m.creature_mut(brute).unwrap();
            c.fields.set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY, 1000);
            c.fields.set_u32(UNIT_FIELD_ATTACK_POWER, 200);
            c.fields.set_u32(UNIT_FIELD_BASEATTACKTIME, 3_000);
            c.base = derive::UnitBase::capture(&c.fields, c.base.run_speed);
            derive::creature(c);
            aura_of(m, brute, AP_DOWN, AURA_MOD_ATTACK_POWER, 0, -100);
            aura_of(m, brute, DISARM, AURA_MOD_DISARM, 0, 0);
            derive::unit(m, brute);
            let f = m.unit_fields(brute).unwrap();
            let fields = (
                f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                f.get_f32(UNIT_FIELD_MINDAMAGE),
                f.get_f32(UNIT_FIELD_MAXDAMAGE),
            );
            let blow = combat::weapon_damage(m, brute, g, Hand::Main, &mut rand::rng()).0;
            let f = &mut m.players.get_mut(&g).unwrap().fields;
            f.set_u32(UNIT_FIELD_RESISTANCES, 0);
            f.set_u32(UNIT_FIELD_HEALTH, 100_000);
            f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
            forced_swing(m, brute, g, Outcome::Hit);
            let struck = 100_000 - m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH);
            (fields, blow, struck, waits(m, brute).0)
        })
        .await
        .unwrap();
    let ((time, min, max), blow, struck, next) = got;
    assert_eq!(time, 3_000, "its weapon's speed");
    assert!(
        (min - 85.0).abs() < 0.01 && (max - 85.0).abs() < 0.01,
        "{min} {max}"
    );
    assert_eq!((blow, struck), (42, 42));
    assert_eq!(next, Some(3_000), "the next blow three seconds on");
}

/// Thistle Tea's "hunter weapon spells apply happiness once and retain
/// their cast snapshot": a hunter's pet made happy does a quarter more
/// than content, its damage moved by its attack power (180 more of its
/// 180: by 1.3) on top; a weapon strike of its hits for that damage, its
/// happiness counted once; and with the buff gone its damage is its happy
/// own again. (Thistle Tea's snapshot is of a cast in flight; wvs's strike
/// lands as it is cast.)
#[tokio::test]
async fn a_hunters_pets_weapon_strike_counts_its_happiness_once() {
    let (_realm, _a, g, map) = setup_with(pets_world(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let pet = summon(m, g, WOLF, KIND_HUNTER, CONTENT);
            let damage = |m: &Map| {
                let f = m.unit_fields(pet).unwrap();
                (
                    f.get_f32(UNIT_FIELD_MINDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXDAMAGE),
                )
            };
            let content = damage(m);
            hunter::change_happiness(m, pet, (HAPPY - CONTENT) as i32);
            let happy = damage(m);
            aura_of(m, pet, AP_UP, AURA_MOD_ATTACK_POWER, 0, 180);
            derive::unit(m, pet);
            let buffed = damage(m);
            let victim = spawn(m, g, BRUTE, 60, 3.0);
            spells::cast_triggered(m, pet, STRIKE_WEAPON, Some(victim));
            let struck = 100_000 - kit::health(m, victim);
            auras::remove_spell(m, pet, AP_UP, None);
            derive::unit(m, pet);
            (content, happy, buffed, struck, damage(m))
        })
        .await
        .unwrap();
    let (content, happy, buffed, struck, back) = got;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        near(happy.0, content.0 * 1.25) && near(happy.1, content.1 * 1.25),
        "{content:?} {happy:?}"
    );
    assert!(
        near(buffed.0, happy.0 * 1.3) && near(buffed.1, happy.1 * 1.3),
        "{happy:?} {buffed:?}"
    );
    // Spell rolls are off in the tests: the middle of its range.
    assert_eq!(struck, ((buffed.0 + buffed.1) / 2.0) as u32);
    assert!(near(back.0, happy.0) && near(back.1, happy.1), "{back:?}");
}

const BADGE: u32 = 26_480;
const INSIGHT: u32 = 26_481;
const AUTO_SHOT: u32 = 75;

/// The real data, with the badge's ten procs a minute made sure, as Thistle
/// Tea's test sets them for its six-second bow.
fn badge_realm() -> Arc<crate::game::Realm> {
    kit::realm_plus(|d| {
        if let Some(p) = d.spells.proc_events.get_mut(&BADGE) {
            p.ppm = 600.0;
        }
    })
}

/// A shot of `g`'s hitting `victim`, for its procs, as `ranged::resolve`
/// reports it.
fn shoot(m: &mut Map, g: Guid, victim: Guid) {
    procs::trigger(
        m,
        procs::Event {
            actor: g,
            target: victim,
            actor_flags: procs::PROC_DONE_RANGED,
            target_flags: procs::PROC_TAKEN_RANGED,
            ex: procs::EX_NORMAL_HIT,
            spell: Some(AUTO_SHOT),
            damage: 30,
            school: 1,
            ranged: true,
            ..Default::default()
        },
    );
}

/// The count the client is shown for `g`'s `spell`: its slot's byte of
/// `UNIT_FIELD_AURAAPPLICATIONS`.
fn shown(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    let slot = kit::holder(m, g, spell)?.slot?;
    let word = m
        .unit_fields(g)?
        .get_u32(UNIT_FIELD_AURAAPPLICATIONS + (slot / 4) as u16);
    Some(word >> ((slot % 4) * 8) & 0xFF)
}

/// `g`'s armor penetration: what its auras take off a target's armor.
fn penetration(m: &Map, g: Guid) -> i32 {
    auras::auras(m, g).map_or(0, |a| a.sum_mask(AURA_MOD_TARGET_RESISTANCE, 1))
}

/// The badge on `g`, then seven shots at `victim`: after each, Insight's
/// stacks, the count shown, the penetration and what 2000 armor leaves of a
/// thousand.
fn badge_and_seven_shots(m: &mut Map, g: Guid, victim: Guid) -> Vec<(u32, Option<u32>, i32, u32)> {
    kit::apply(m, g, g, BADGE);
    (0..7)
        .map(|_| {
            shoot(m, g, victim);
            (
                kit::holder(m, g, INSIGHT).map_or(0, |h| h.stacks),
                shown(m, g, INSIGHT),
                penetration(m, g),
                combat::armor_mitigate(m, g, victim, 1_000),
            )
        })
        .collect()
}

/// Thistle Tea's "ranged procs stack six armor penetration buffs on the
/// bearer": the Badge of the Swarmguard on for its thirty seconds, each
/// shot that procs it gives a stack of Insight of the Qiraji (a lasting
/// aura of six stacks, each 200 armor through), shown on the client; a
/// seventh holds at six. Insight goes with the badge: at its thirty
/// seconds, as it is cancelled, and at its bearer's death.
#[tokio::test]
#[ignore = "extended"]
async fn the_badge_of_the_swarmguards_shots_stack_six_insights_which_go_with_it() {
    if !kit::have_data() {
        return;
    }
    let realm = badge_realm();
    let (_c, g, map) =
        kit::player(&realm, "P9CP4", "Badgehunter", Race::Dwarf, Class::Hunter).await;
    let got = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let data = m.realm.data.clone();
            let (badge, insight) = (
                data.spells.get(BADGE).unwrap(),
                data.spells.get(INSIGHT).unwrap(),
            );
            let lifetimes = (
                data.spells.duration(badge, 60),
                data.spells.duration(insight, 60),
                insight.stack_amount,
            );
            let victim = kit::dummy(m, g, 60, 5.0);
            m.unit_fields_mut(victim)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 2_000);
            let shots = badge_and_seven_shots(m, g, victim);
            let source = kit::holder(m, g, INSIGHT).and_then(|h| h.source);
            kit::pass(m, 30_000);
            let expired = (
                kit::has(m, g, BADGE),
                kit::has(m, g, INSIGHT),
                penetration(m, g),
            );
            badge_and_seven_shots(m, g, victim);
            auras::remove_spell(m, g, BADGE, None);
            let cancelled = (kit::has(m, g, INSIGHT), penetration(m, g));
            badge_and_seven_shots(m, g, victim);
            combat::deal_damage(m, victim, g, 10_000_000);
            let dead = (
                kit::health(m, g),
                kit::has(m, g, BADGE),
                kit::has(m, g, INSIGHT),
            );
            (lifetimes, shots, source, expired, cancelled, dead)
        })
        .await
        .unwrap();
    let (lifetimes, shots, source, expired, cancelled, dead) = got;
    assert_eq!(lifetimes, (Some(30_000), None, 6), "the data");
    let mitigated =
        |n: i32| 1_000 - (1_000.0 * combat::armor_reduction((2_000 - 200 * n) as u32, 60)) as u32;
    for (hit, got) in shots.into_iter().enumerate() {
        let n = (hit as u32 + 1).min(6);
        assert_eq!(
            got,
            (n, Some(n - 1), -200 * n as i32, mitigated(n as i32)),
            "shot {}",
            hit + 1
        );
    }
    assert_eq!(source, Some((BADGE, g)), "Insight goes with the badge");
    assert_eq!(expired, (false, false, 0), "the badge's thirty seconds out");
    assert_eq!(cancelled, (false, 0), "the badge cancelled");
    assert_eq!(dead, (0, false, false), "its bearer dead");
}

/// Thistle Tea's "queued procs cannot restore the permanent buff after the
/// badge expires": a shot a moment before the badge's end gives its stack,
/// which goes at the end; a shot after it gives none back. (Thistle Tea
/// delivers a proc's spell later and checks its source then; wvs casts it
/// as the shot lands, from the badge's own aura, so none is left to come
/// late.)
#[tokio::test]
#[ignore = "extended"]
async fn a_shot_after_the_badge_of_the_swarmguard_has_ended_gives_no_insight_back() {
    if !kit::have_data() {
        return;
    }
    let realm = badge_realm();
    let (_c, g, map) = kit::player(&realm, "P9CP5", "Latehunter", Race::Dwarf, Class::Hunter).await;
    let got = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let victim = kit::dummy(m, g, 60, 5.0);
            kit::apply(m, g, g, BADGE);
            kit::pass(m, 29_999);
            shoot(m, g, victim);
            let last = kit::holder(m, g, INSIGHT).map(|h| (h.stacks, h.source));
            kit::pass(m, 1);
            let ended = (kit::has(m, g, BADGE), kit::has(m, g, INSIGHT));
            shoot(m, g, victim);
            let after = (kit::has(m, g, INSIGHT), penetration(m, g));
            (last, ended, after)
        })
        .await
        .unwrap();
    let (last, ended, after) = got;
    assert_eq!(
        last,
        Some((1, Some((BADGE, g)))),
        "a stack at its last moment"
    );
    assert_eq!(ended, (false, false), "both gone at its end");
    assert_eq!(after, (false, 0), "nothing back from a shot after");
}

const DRAIN_MANA: u32 = 5_138;
/// A made-up aura any damage ends (`AURA_INTERRUPT_FLAG_DAMAGE`).
const FRAGILE: u32 = 990_771;

/// Thistle Tea's "ticks remove damage-interrupted auras without restoring
/// the removed holder", as mangos-classic has it instead: Drain Mana's
/// tick takes the victim's mana and gives it to the warlock, deals no
/// damage, and so leaves an aura damage ends (mangos's `PeriodicTick` for
/// `SPELL_AURA_PERIODIC_MANA_LEECH` doesn't go through `DealDamage`, whose
/// `AURA_INTERRUPT_FLAG_DAMAGE` is what ends one); Drain Mana stays. A blow
/// then ends it, and it doesn't come back.
#[tokio::test]
#[ignore = "extended"]
async fn drain_manas_tick_takes_mana_and_leaves_an_aura_only_damage_ends() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm_plus(|d| {
        let mut s = kit::made_up(FRAGILE, 6, AURA_DUMMY, 0, 0);
        s.aura_interrupt_flags = INTERRUPT_DAMAGE;
        d.spells.spells.insert(FRAGILE, s);
    });
    let (_c, g, map) = kit::player(&realm, "P9CP6", "Drainer", Race::Human, Class::Warlock).await;
    let got = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            kit::set_power(m, g, 0, 0);
            let victim = kit::dummy(m, g, 60, 5.0);
            let f = m.unit_fields_mut(victim).unwrap();
            f.set_byte(UNIT_FIELD_BYTES_0, 3, 0);
            f.set_u32(UNIT_FIELD_MAXPOWER1, 1_000);
            f.set_u32(UNIT_FIELD_POWER1, 1_000);
            kit::apply(m, victim, victim, FRAGILE);
            kit::apply(m, g, victim, DRAIN_MANA);
            let on = (
                kit::has(m, victim, FRAGILE),
                kit::has(m, victim, DRAIN_MANA),
            );
            let health = kit::health(m, victim);
            kit::tick_now(m, victim, DRAIN_MANA);
            let ticked = (
                kit::power(m, victim, 0),
                kit::power(m, g, 0),
                kit::health(m, victim) == health,
                kit::has(m, victim, FRAGILE),
                kit::has(m, victim, DRAIN_MANA),
            );
            combat::deal_damage(m, g, victim, 1);
            let struck = kit::has(m, victim, FRAGILE);
            kit::tick_now(m, victim, DRAIN_MANA);
            let later = (
                kit::has(m, victim, FRAGILE),
                kit::has(m, victim, DRAIN_MANA),
            );
            (on, ticked, struck, later)
        })
        .await
        .unwrap();
    let (on, (left, gained, unhurt, fragile, drain), struck, later) = got;
    assert_eq!(on, (true, true));
    assert!(
        left < 1_000 && gained > 0,
        "drained: {left} left, {gained} gained"
    );
    assert!(unhurt, "no damage");
    assert!(fragile, "what damage ends stays through a drain's tick");
    assert!(drain, "Drain Mana stays");
    assert!(!struck, "a blow ends it");
    assert_eq!(later, (false, true), "and it doesn't come back");
}

const OUTDOOR_AP: u32 = 990_781;
const OUTDOOR_VEST: u32 = 990_782;
const CHEST: u8 = 4;

/// A vest of 20 armor whose worn spell gives 50 attack power outdoors only.
fn outdoor_world() -> GameData {
    let mut d = GameData::empty();
    let mut s = passive(OUTDOOR_AP, &[(AURA_MOD_ATTACK_POWER, 50, 0)]);
    s.attributes[0] |= ATTR_ONLY_OUTDOORS;
    d.spells.spells.insert(OUTDOOR_AP, s);
    let mut vest = item(OUTDOOR_VEST, 4, 1, 5);
    vest.armor = 20;
    vest.spells[0] = ItemSpell {
        id: OUTDOOR_AP,
        trigger: inventory::ITEM_SPELL_ON_EQUIP,
        ..Default::default()
    };
    d.items.insert(OUTDOOR_VEST, vest);
    d
}

/// `g`'s gear made to match what is worn and where they are, and the
/// stats derived.
fn resync(m: &mut Map, g: Guid) {
    let now = m.now;
    environment::reconcile(m, g, now);
    let d = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    inventory::sync_gear(m, g);
    derive::unit(m, g);
}

/// `g`'s attack power, its mods and armor.
fn ap_and_armor(m: &Map, g: Guid) -> (u32, u32, u32) {
    let p = &m.players[&g];
    (
        p.derived.attack_power,
        p.fields.get_u32(UNIT_FIELD_ATTACK_POWER_MODS),
        p.fields.get_u32(UNIT_FIELD_RESISTANCES),
    )
}

/// What the vest gives `g` outdoors, indoors, worn anew indoors, and
/// outdoors again, with `inside(m, bool)` putting them in or out.
fn the_vest_in_and_out(
    m: &mut Map,
    g: Guid,
    inside: impl Fn(&mut Map, bool),
) -> [(u32, u32, u32); 5] {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.items.clear();
    crate::game::map::stats::set_level(p, &data, 60);
    inside(m, false);
    resync(m, g);
    let bare = ap_and_armor(m, g);
    wear(m, g, OUTDOOR_VEST, CHEST);
    let outside = ap_and_armor(m, g);
    inside(m, true);
    resync(m, g);
    let indoors = ap_and_armor(m, g);
    m.players
        .get_mut(&g)
        .unwrap()
        .items
        .retain(|i| !(i.bag == BAG_PLAYER && i.slot == CHEST));
    resync(m, g);
    wear(m, g, OUTDOOR_VEST, CHEST);
    let anew = ap_and_armor(m, g);
    inside(m, false);
    resync(m, g);
    [bare, outside, indoors, anew, ap_and_armor(m, g)]
}

fn assert_the_vest(got: [(u32, u32, u32); 5]) {
    let [bare, outside, indoors, anew, back] = got;
    assert_eq!(
        outside,
        (bare.0 + 50, bare.1 + 50, bare.2 + 20),
        "outdoors, all of it"
    );
    assert_eq!(indoors, (bare.0, bare.1, bare.2 + 20), "indoors, its armor");
    assert_eq!(anew, indoors, "put on indoors, its armor");
    assert_eq!(back, outside, "outdoors again, all of it");
}

/// Thistle Tea's "excludes outdoor on-equip stats indoors without dropping
/// the item's own stats": a vest of 20 armor whose worn spell gives 50
/// attack power outdoors only gives both outdoors, its armor alone indoors
/// (taken in, or put on there), and both again outside.
#[tokio::test]
async fn gears_outdoor_spell_is_held_off_indoors_and_its_armor_kept() {
    let (_realm, _a, g, map) = setup_with(outdoor_world(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            the_vest_in_and_out(m, g, move |m, inside| {
                m.spells.outdoors_override.insert(g, !inside);
            })
        })
        .await
        .unwrap();
    assert_the_vest(got);
}

/// The same on the real navmesh: Northshire's open air in front of the
/// abbey, and the abbey's nave under its roof.
#[tokio::test]
#[ignore = "extended"]
async fn gears_outdoor_spell_is_held_off_in_northshire_abbey_and_its_armor_kept() {
    let nav = crate::game::harness::real_data("nav");
    if !nav.join("Azeroth.map").exists() {
        return;
    }
    let mut cfg = crate::game::harness::test_config();
    cfg.nav_dir = Some(nav);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, outdoor_world(), None);
    let mut a = Client::connect(&realm, "P9CP7");
    let g = a.enter("Abbeygoer", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let got = map
        .query(move |m| {
            let open = Vector3d {
                x: -8949.95,
                y: -132.49,
                z: 83.53,
            };
            let nave = Vector3d {
                x: -8914.0,
                y: -164.0,
                z: 82.0,
            };
            let mut wheres = Vec::new();
            for at in [open, nave, open] {
                super::tests_p7_scripts::stand(m, g, at);
                wheres.push(environment::outdoors(m, g));
            }
            let got = the_vest_in_and_out(m, g, move |m, inside| {
                super::tests_p7_scripts::stand(m, g, if inside { nave } else { open });
            });
            (wheres, got)
        })
        .await
        .unwrap();
    let (wheres, got) = got;
    assert_eq!(wheres, [Some(true), Some(false), Some(true)], "the navmesh");
    assert_the_vest(got);
}
