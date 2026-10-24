//! A rogue's damage against a raid boss's numbers, on the real data: its
//! hit table from behind, its energy, its combo points, Sinister Strike and
//! Eviscerate, its poisons, measured against 1.12's arithmetic
//! (mangos-classic's), with the map's clock skipped.

use std::sync::Arc;
use std::time::Duration;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, MapHandle, MapKey, combat};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, Class::Rogue).await;
    let map = realm
        .maps
        .get(realm, MapKey::world(if race == Race::Orc { 1 } else { 0 }));
    (c, g, map)
}

/// [PH] Alex's Test DPS Mob: a level-63 boss, no spells.
const DUMMY: u32 = 16_077;

/// The dummy `yards` in front of `g`, its back to it.
fn dummy_behind(m: &mut Map, g: Guid, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let spawn = crate::game::data::CreatureSpawn {
        guid: m.creature_counter(),
        entries: vec![DUMMY],
        map: m.key.map,
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
        orientation: o,
        ..Default::default()
    };
    let mut c = Creature::from_spawn(&data, &spawn, &mut rand::rng()).expect("the dummy");
    c.spawn_guid = None;
    m.add_creature(c)
}

/// `g` at 60, geared by `.gear` with `args`, its skills at their cap, and
/// `learn` learned.
fn ready(m: &mut Map, g: Guid, gear: &[&str], learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, 60);
    for s in learn {
        spells::learn(m, g, *s);
    }
    let said = crate::game::map::gm_dev::command(m, g, "gear", gear);
    eprintln!("gear {gear:?}: {said:?}");
    crate::game::map::gm_dev::command(m, g, "debug", &["skills"]);
    crate::game::map::derive::unit(m, g);
}

/// peon's Combat build's talents (its `talents::ROGUE`, "Combat"):
/// Improved Sinister Strike 2, Lightning Reflexes 3, Precision 5,
/// Endurance 2, Deflection 3, Dual Wield Specialization 5, Blade Flurry,
/// Sword Specialization 5, Weapon Expertise 2, Aggression 2, Adrenaline
/// Rush, Improved Eviscerate 3, Malice 5, Murder 2, Relentless Strikes,
/// Lethality 4, Opportunity 5.
const COMBAT: &[u32] = &[
    13_863, 13_789, 13_845, 13_872, 13_854, 13_852, 13_877, 13_964, 30_920, 18_428, 13_750, 14_164,
    14_142, 14_159, 14_179, 14_136, 14_075,
];

const SINISTER_STRIKE_8: u32 = 11_294;
const EVISCERATE_9: u32 = 31_016;
const SLICE_AND_DICE_2: u32 = 6774;
const ADRENALINE_RUSH: u32 = 13_750;

fn energy(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_POWER1 + 3)
}

fn set_energy(m: &mut Map, g: Guid, e: u32) {
    m.unit_fields_mut(g)
        .unwrap()
        .set_u32(UNIT_FIELD_POWER1 + 3, e);
}

fn combo(m: &Map, g: Guid) -> u8 {
    m.caster(g).map_or(0, |c| c.combo_points)
}

fn at(d: Guid) -> Targets {
    Targets {
        unit: Some(d),
        ..Default::default()
    }
}

/// Probe: a raid-geared Combat rogue's hit table at the dummy from behind,
/// its numbers, and an ideal rotation's damage over two minutes (Slice and
/// Dice kept up, Eviscerate at five, Sinister Strike else; Adrenaline Rush
/// at the start), the rolls the dice's.
#[tokio::test]
#[ignore = "extended"]
async fn probe_rogue_damage() {
    if !have_data() {
        return;
    }
    for (k, (race, gear)) in [
        (Race::Human, "sword"),
        (Race::Gnome, "sword"),
        (Race::Human, "dagger"),
        (Race::Gnome, "medium sword"),
    ]
    .into_iter()
    .enumerate()
    {
        let realm = realm();
        let (_c, g, map) = enter(
            &realm,
            "RDPS",
            ["Stabba", "Stabbb", "Stabbc", "Stabbd"][k],
            race,
        )
        .await;
        let out = map
            .query(move |m| {
                let mut kit: Vec<&str> = if gear.starts_with("medium") { vec![] } else { vec!["raid"] };
                kit.extend(gear.split(' ').filter(|w| *w != "medium"));
                let mut learn = COMBAT.to_vec();
                learn.extend([SINISTER_STRIKE_8, EVISCERATE_9, SLICE_AND_DICE_2]);
                ready(m, g, &kit, &learn);
                let d = dummy_behind(m, g, 2.0);
                let f = m.unit_fields(g).unwrap();
                let fields = format!(
                    "main {:.0}-{:.0} at {}, off {:.0}-{:.0} at {}, ap {}",
                    f.get_f32(UNIT_FIELD_MINDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXDAMAGE),
                    f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                    f.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXOFFHANDDAMAGE),
                    f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1),
                    f.get_u32(UNIT_FIELD_ATTACK_POWER) + f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS),
                );
                let mut table = String::new();
                for hand in [combat::Hand::Main, combat::Hand::Off] {
                    let mut r = combat::roll_for(m, g, d, hand).unwrap();
                    r.dual_wield = true;
                    let white = r.miss_bp();
                    let special = combat::Roll { special: true, ..r }.miss_bp();
                    let mut counts = std::collections::BTreeMap::new();
                    for roll in 0..10_000 {
                        *counts
                            .entry(format!("{:?}", combat::roll_outcome(&r, roll)))
                            .or_insert(0u32) += 1;
                    }
                    table += &format!(
                        "\n  {hand:?}: skill {} vs defense {}, hit {}, crit {}, miss white {} special {}, table {counts:?}",
                        r.attacker_skill, r.victim_defense, r.hit, r.crit, white, special
                    );
                }
                // The ideal rotation, two minutes, the dice rolling.
                m.spells.rolls = Some(true);
                let max = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
                let health = |m: &Map| m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH);
                let start = health(m);
                set_energy(m, g, 100);
                combat::start_attack(m, g, d);
                let mut gcd = m.now;
                let mut casts = std::collections::BTreeMap::<&str, u32>::new();
                let mut full_ms = 0u32;
                let mut snd_ms = 0u32;
                spells::cast(m, g, ADRENALINE_RUSH, Targets::default(), None);
                let t0 = m.now;
                let mut dealt = 0u64;
                for _ in 0..120 * 100 {
                    let h0 = health(m);
                    m.now += Duration::from_millis(10);
                    crate::game::map::stats::regen(m);
                    combat::update_players(m);
                    spells::update(m);
                    crate::game::map::auras::update(m);
                    crate::game::map::enchant::update(m);
                    dealt += u64::from(h0.saturating_sub(health(m)));
                    if energy(m, g) >= 100 {
                        full_ms += 10;
                    }
                    let sliced = crate::game::map::auras::auras(m, g)
                        .is_some_and(|a| a.has_spell(SLICE_AND_DICE_2));
                    if sliced {
                        snd_ms += 10;
                    }
                    if m.now < gcd {
                        continue;
                    }
                    let e = energy(m, g);
                    let cp = combo(m, g);
                    let pick = if !sliced && cp >= 1 && e >= 25 {
                        Some(("Slice and Dice", SLICE_AND_DICE_2))
                    } else if cp >= 5 && e >= 35 {
                        Some(("Eviscerate", EVISCERATE_9))
                    } else if cp < 5 && e >= 40 {
                        Some(("Sinister Strike", SINISTER_STRIKE_8))
                    } else {
                        None
                    };
                    if let Some((name, spell)) = pick {
                        let before = energy(m, g);
                        let targets = if spell == SLICE_AND_DICE_2 { Targets::default() } else { at(d) };
                        let hb = health(m);
                        spells::cast(m, g, spell, targets, None);
                        dealt += u64::from(hb.saturating_sub(health(m)));
                        if energy(m, g) < before {
                            *casts.entry(name).or_default() += 1;
                            gcd = m.now + Duration::from_secs(1);
                        }
                    }
                    if health(m) < max / 2 {
                        m.unit_fields_mut(d).unwrap().set_u32(UNIT_FIELD_HEALTH, max);
                    }
                }
                let secs = m.now.duration_since(t0).as_secs_f64();
                combat::stop_attack(m, g);
                let _ = start;
                (fields, table, casts, dealt, secs, full_ms, snd_ms)
            })
            .await
            .unwrap();
        eprintln!(
            "{race:?} {gear}: {}{}\n  rotation: {:?}, {} damage in {:.0} s ({:.0} dps), energy full {} ms, SnD {} ms",
            out.0,
            out.1,
            out.2,
            out.3,
            out.4,
            out.3 as f64 / out.4,
            out.5,
            out.6
        );
    }
}

/// A Combat rogue's numbers as 1.12 has them, the rolls fixed (a hit, no
/// crit): Sinister Strike the weapon's swing and 68, Aggression's 4% on it;
/// Eviscerate at five points 904 to 1012 and 3% of attack power a point,
/// Improved Eviscerate's 15% and Aggression's 4% added (mangos-classic's
/// `Spell::EffectSchoolDMG` and `Player::ApplySpellMod`, its percents
/// summed); both less the dummy's armor. Its energy: 20 a tick, 40 under
/// Adrenaline Rush (`Player::Regenerate`); Sinister Strike 40 with
/// Improved Sinister Strike; a point on its hit; Relentless Strikes' 25
/// back at five points.
#[tokio::test]
#[ignore = "extended"]
async fn a_rogues_blows_and_energy_are_1_12s() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "RDPSB", "Rogblows", Race::Gnome).await;
    let out = map
        .query(move |m| {
            let mut learn = COMBAT.to_vec();
            learn.extend([SINISTER_STRIKE_8, EVISCERATE_9, SLICE_AND_DICE_2]);
            ready(m, g, &["raid", "sword"], &learn);
            let d = dummy_behind(m, g, 2.0);
            m.spells.rolls = Some(false);
            let f = m.unit_fields(g).unwrap();
            let ap = f.get_u32(UNIT_FIELD_ATTACK_POWER) + f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS);
            // Sinister Strike is normalized in 1.12's data (effect 121): a
            // one-hander's 2.4 s, not the sword's own speed.
            let speed = f.get_u32(UNIT_FIELD_BASEATTACKTIME) as f32 / 1000.0;
            let weapon = (f.get_f32(UNIT_FIELD_MINDAMAGE) + f.get_f32(UNIT_FIELD_MAXDAMAGE)) / 2.0
                - ap as f32 / 14.0 * (speed - 2.4);
            let armor = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_RESISTANCES);
            let max = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            let health = |m: &Map| m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH);
            // Sinister Strike, cast: its cost, its damage, its point.
            set_energy(m, g, 100);
            let before = health(m);
            spells::cast(m, g, SINISTER_STRIKE_8, at(d), None);
            let ss = before - health(m);
            let ss_cost = 100 - energy(m, g);
            let ss_combo = combo(m, g);
            // Its crit: Lethality's 24% more on the bonus.
            m.now += Duration::from_secs(2);
            m.spells
                .queued_rolls
                .lock()
                .unwrap()
                .extend([9_999, 9_999, 0]);
            let before = health(m);
            spells::cast(m, g, SINISTER_STRIKE_8, at(d), None);
            m.spells.queued_rolls.lock().unwrap().clear();
            let ss_crit = before - health(m);
            // Eviscerate at five: its damage, and Relentless Strikes'
            // energy.
            m.now += Duration::from_secs(2);
            spells::add_combo_points(m, g, d, 5);
            set_energy(m, g, 50);
            let before = health(m);
            spells::cast(m, g, EVISCERATE_9, at(d), None);
            let evis = before - health(m);
            let evis_energy = energy(m, g);
            let evis_combo = combo(m, g);
            m.unit_fields_mut(d)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, max);
            // Slice and Dice at five: 21 s, its swings 30% faster.
            let period = |m: &Map| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_BASEATTACKTIME);
            let slow = period(m);
            m.now += Duration::from_secs(2);
            spells::add_combo_points(m, g, d, 5);
            set_energy(m, g, 100);
            spells::cast(m, g, SLICE_AND_DICE_2, Targets::default(), None);
            crate::game::map::derive::unit(m, g);
            let now = m.now;
            let snd = crate::game::map::auras::auras(m, g)
                .and_then(|a| a.holders.iter().find(|h| h.spell == SLICE_AND_DICE_2))
                .and_then(|h| h.expires)
                .map(|t| t.duration_since(now).as_millis());
            let sliced = (slow, period(m), snd);
            // Energy, ten ticks; then under Adrenaline Rush.
            let mut ticks = Vec::new();
            for rush in [false, true] {
                if rush {
                    spells::cast_triggered(m, g, ADRENALINE_RUSH, None);
                }
                set_energy(m, g, 0);
                let start = m.now;
                let mut last = 0;
                let mut gaps = Vec::new();
                let mut at = start;
                while m.now < start + Duration::from_millis(5_000) {
                    m.now += Duration::from_millis(10);
                    crate::game::map::stats::regen(m);
                    let e = energy(m, g);
                    if e != last {
                        gaps.push((e - last, m.now.duration_since(at).as_millis() / 100 * 100));
                        at = m.now;
                        last = e;
                    }
                }
                ticks.push(gaps[1..].to_vec());
            }
            (
                ap,
                weapon,
                armor,
                ss,
                ss_cost,
                ss_combo,
                ss_crit,
                evis,
                evis_energy,
                evis_combo,
                ticks,
                sliced,
            )
        })
        .await
        .unwrap();
    eprintln!("{out:?}");
    let (
        ap,
        weapon,
        armor,
        ss,
        ss_cost,
        ss_combo,
        ss_crit,
        evis,
        evis_energy,
        evis_combo,
        ticks,
        sliced,
    ) = out;
    assert_eq!(sliced.2, Some(21_000), "Slice and Dice's 21 s at five");
    assert!(
        (f64::from(sliced.1) - f64::from(sliced.0) / 1.3).abs() <= 2.0,
        "its swings 30% faster: {} to {}",
        sliced.0,
        sliced.1
    );
    let kept = 1.0 - armor as f32 / (armor as f32 + 400.0 + 85.0 * 60.0);
    let ap = ap as f32;
    let want_ss = (weapon + 68.0) * 1.04 * kept;
    // Its dice roll whatever the map's rolls: 904 to 1012.
    let evis_range = |dice: f32| (dice + 0.03 * 5.0 * ap) * 1.19 * kept;
    assert!(
        (ss as f32 - want_ss).abs() <= 2.0,
        "Sinister Strike {ss}, 1.12's {want_ss:.1}"
    );
    assert!(
        (ss_crit as f32 - ss as f32 * 2.24).abs() <= 3.0,
        "its crit {ss_crit}, 2.24 times {ss}"
    );
    assert!(
        (evis_range(904.0) - 2.0..=evis_range(1012.0) + 2.0).contains(&(evis as f32)),
        "Eviscerate {evis}, 1.12's {:.1} to {:.1}",
        evis_range(904.0),
        evis_range(1012.0)
    );
    assert_eq!(ss_cost, 40, "Improved Sinister Strike's cost");
    assert_eq!(ss_combo, 1, "a point on its hit");
    assert_eq!(evis_combo, 0, "the finisher spends them");
    assert_eq!(
        evis_energy,
        50 - 35 + 25,
        "Relentless Strikes at five points"
    );
    assert!(
        !ticks[0].is_empty() && ticks[0].iter().all(|t| *t == (20, 2000)),
        "a tick of 20 every 2 s: {:?}",
        ticks[0]
    );
    assert!(
        !ticks[1].is_empty() && ticks[1].iter().all(|t| *t == (40, 2000)),
        "of 40 under Adrenaline Rush: {:?}",
        ticks[1]
    );
}

/// An ability crits on its crit chance of its hits: one table roll for the
/// hit, then the crit on a roll of its own (mangos-classic's two rolls for
/// a special attack, `combat::roll_special`). Before, the crit came of a
/// second roll of the whole table, its miss and dodge (7% at a boss)
/// taken off the crit chance: Sinister Strike crit 11.8% of its hits for
/// its 13.6% (its hits 85%).
#[tokio::test]
#[ignore = "extended"]
async fn an_abilitys_crit_is_its_crit_chance_of_its_hits() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "RDPSC", "Rogcrit", Race::Gnome).await;
    let (hits, crits, crit_bp) = map
        .query(move |m| {
            ready(m, g, &["raid", "sword"], &[SINISTER_STRIKE_8]);
            let d = dummy_behind(m, g, 2.0);
            m.spells.rolls = Some(true);
            let crit_bp = {
                let mut r = combat::roll_for(m, g, d, combat::Hand::Main).unwrap();
                r.special = true;
                r.crit_bp()
            };
            let max = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            let (mut hits, mut crits) = (0u32, 0u32);
            // The most a strike deals that doesn't crit: the weapon's top
            // roll at 2.4 s and 68, less the armor; a crit is twice that or
            // more of its least, and its least is above 0.7 of its most.
            let mut most = 0u32;
            let mut dealt = Vec::with_capacity(20_000);
            for _ in 0..20_000 {
                let before = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH);
                spells::cast_triggered(m, g, SINISTER_STRIKE_8, Some(d));
                let got = before - m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH);
                m.unit_fields_mut(d)
                    .unwrap()
                    .set_u32(UNIT_FIELD_HEALTH, max);
                m.now += Duration::from_millis(1100);
                if got > 0 {
                    dealt.push(got);
                }
                most = most.max(got);
            }
            for got in dealt {
                hits += 1;
                if got as f32 > most as f32 / 2.24 * 1.35 {
                    crits += 1;
                }
            }
            (hits, crits, crit_bp)
        })
        .await
        .unwrap();
    let rate = f64::from(crits) / f64::from(hits) * 10_000.0;
    eprintln!("{crits} crits of {hits} hits: {rate:.0} bp, the chance {crit_bp} bp");
    assert!(hits > 15_000, "{hits} hits of 20,000");
    assert!(
        (rate - f64::from(crit_bp)).abs() < 80.0,
        "{rate:.0} bp of its hits crit, its chance {crit_bp} bp"
    );
}

/// Deadly Poison V at the boss: five stacks at most, 34 a stack every 3 s
/// over 12 s (its data's), a sixth putting its time back (mangos-classic's
/// `SpellAuraHolder` stacking to `StackAmount`).
#[tokio::test]
#[ignore = "extended"]
async fn deadly_poison_stacks_to_five_on_a_boss() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "RDPSP", "Rogpois", Race::Gnome).await;
    let (stacks, ticked) = map
        .query(move |m| {
            ready(m, g, &["raid", "sword"], &[]);
            let d = dummy_behind(m, g, 2.0);
            m.spells.rolls = Some(false);
            for _ in 0..6 {
                spells::cast_triggered(m, g, 25_349, Some(d));
                m.now += Duration::from_millis(100);
            }
            let stacks = crate::game::map::auras::auras(m, d)
                .and_then(|a| {
                    a.holders
                        .iter()
                        .find(|h| h.spell == 25_349)
                        .map(|h| h.stacks)
                })
                .unwrap_or(0);
            let health = |m: &Map| m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let before = health(m);
            for _ in 0..1200 {
                m.now += Duration::from_millis(10);
                crate::game::map::auras::update(m);
            }
            (stacks, before - health(m))
        })
        .await
        .unwrap();
    assert_eq!(stacks, 5, "five stacks at most");
    // 12 s from the last: four ticks in the 12 s after it, of 34 a stack,
    // nature, the dummy resisting none of it.
    assert_eq!(ticked, 4 * 5 * 34, "four ticks of five stacks of 34");
}

/// A rogue's threat at a boss: 71% of its damage (Rogue Passive, 21184,
/// every rogue's from its creation), Feint's 800 off (rank 5's), and
/// Vanish off the list whole (mangos-classic's `EffectSanctuary`).
#[tokio::test]
#[ignore = "extended"]
async fn a_rogues_threat_is_its_damages_71_feint_and_vanish_take_it_off() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "RDPST", "Rogthreat", Race::Gnome).await;
    let (passive, dealt, after_hit, after_feint, after_vanish) = map
        .query(move |m| {
            ready(m, g, &["raid", "sword"], &[SINISTER_STRIKE_8, 25_302, 1857]);
            let d = dummy_behind(m, g, 2.0);
            m.spells.rolls = Some(false);
            let passive = crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(21_184));
            let threat = |m: &Map| {
                m.creature(d)
                    .and_then(|c| c.threat.iter().find(|(x, _)| *x == g).map(|(_, t)| *t))
            };
            let health = |m: &Map| m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH);
            // Two strikes, so that Feint leaves some.
            let before = health(m);
            for _ in 0..6 {
                spells::cast_triggered(m, g, SINISTER_STRIKE_8, Some(d));
            }
            let dealt = before - health(m);
            let after_hit = threat(m);
            spells::cast_triggered(m, g, 25_302, Some(d));
            let after_feint = threat(m);
            spells::cast_triggered(m, g, 1857, None);
            let after_vanish = threat(m);
            (passive, dealt, after_hit, after_feint, after_vanish)
        })
        .await
        .unwrap();
    assert!(passive, "Rogue Passive on it");
    let hit = after_hit.expect("on its list");
    assert!(
        (hit - dealt as f32 * 0.71).abs() <= 2.0,
        "threat {hit} for {dealt} damage, 71% of it"
    );
    let feint = after_feint.expect("still on it");
    assert!(
        (hit - feint - 800.0).abs() <= 1.0,
        "Feint: {hit} to {feint}"
    );
    assert!(
        after_vanish.is_none_or(|t| t == 0.0),
        "Vanish: {after_vanish:?}"
    );
}

/// Seal Fate (rank 5, every time) puts a second point on a builder's
/// crit, none on its plain hit.
#[tokio::test]
#[ignore = "extended"]
async fn seal_fate_adds_a_point_on_a_crit() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "RDPSF", "Rogfate", Race::Gnome).await;
    let (plain, crit) = map
        .query(move |m| {
            ready(m, g, &["raid", "dagger"], &[SINISTER_STRIKE_8, 14_195]);
            let d = dummy_behind(m, g, 2.0);
            m.spells.rolls = Some(false);
            spells::cast_triggered(m, g, SINISTER_STRIKE_8, Some(d));
            let plain = combo(m, g);
            spells::clear_combo_points(m, g);
            // The table's roll a hit (and its own crit roll, unused), then
            // the crit's.
            m.spells
                .queued_rolls
                .lock()
                .unwrap()
                .extend([9_999, 9_999, 0]);
            spells::cast_triggered(m, g, SINISTER_STRIKE_8, Some(d));
            m.spells.queued_rolls.lock().unwrap().clear();
            (plain, combo(m, g))
        })
        .await
        .unwrap();
    assert_eq!(plain, 1, "a hit's one point");
    assert_eq!(crit, 2, "a crit's two");
}
