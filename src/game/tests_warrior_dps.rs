//! A warrior's damage against a raid boss's numbers, on the real data: its
//! white swings, its rage from them, and its blows (Bloodthirst, Mortal
//! Strike, Whirlwind, Heroic Strike, Execute), measured against 1.12's
//! arithmetic (mangos-classic's tables), with the map's clock skipped.

use std::sync::Arc;
use std::time::Duration;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::combat;
use crate::game::map::creature::Creature;
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

async fn enter(realm: &Arc<Realm>, account: &str, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Orc, Class::Warrior).await;
    let map = realm.maps.get(realm, MapKey::world(1));
    (c, g, map)
}

/// [PH] Alex's Test DPS Mob: a level-63 boss, no spells.
const DUMMY: u32 = 16_077;

/// The dummy as `.spawn` makes it, `yards` in front of `g`, its back to
/// it (a raid's melee stands behind the boss).
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

/// `g` at 60, geared by `.gear` with `args`, its skills at their cap.
fn ready(m: &mut Map, g: Guid, gear: &[&str], learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, 60);
    for s in learn {
        crate::game::map::spells::learn(m, g, *s);
    }
    let said = crate::game::map::gm_dev::command(m, g, "gear", gear);
    eprintln!("gear {gear:?}: {said:?}");
    crate::game::map::gm_dev::command(m, g, "debug", &["skills"]);
    crate::game::map::derive::unit(m, g);
}

/// What `g`'s swings at `victim` came to over `secs` of the map's clock,
/// stepped 10 ms at a time, white swings only.
#[derive(Debug, Default)]
struct White {
    damage: u64,
    swings: u32,
    rage: u32,
}

fn swing_for(m: &mut Map, g: Guid, victim: Guid, secs: u32, spend_rage: bool) -> White {
    let health = |m: &Map| m.unit_fields(victim).unwrap().get_u32(UNIT_FIELD_HEALTH);
    let start = health(m);
    combat::start_attack(m, g, victim);
    let mut out = White::default();
    let mut last_main = m.combat(g).and_then(|c| c.next_swing);
    let mut last_off = m.combat(g).and_then(|c| c.next_offhand);
    for _ in 0..secs * 100 {
        m.now += Duration::from_millis(10);
        let before = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_POWER1 + 1);
        combat::try_swing(m, g);
        crate::game::map::auras::update(m);
        let after = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_POWER1 + 1);
        out.rage += after.saturating_sub(before);
        if spend_rage {
            m.unit_fields_mut(g)
                .unwrap()
                .set_u32(UNIT_FIELD_POWER1 + 1, 0);
        }
        // A swing sets its hand's next a whole period on; the other hand's
        // nudged 200 ms on is no swing.
        let (main, off) = m
            .combat(g)
            .map_or((None, None), |c| (c.next_swing, c.next_offhand));
        let long = |t: Option<std::time::Instant>| {
            t.is_some_and(|t| t >= m.now + Duration::from_millis(500))
        };
        if main != last_main && long(main) {
            out.swings += 1;
        }
        if off != last_off && long(off) {
            out.swings += 1;
        }
        last_main = main;
        last_off = off;
    }
    combat::stop_attack(m, g);
    out.damage = u64::from(start - health(m));
    out
}

/// The Fury build's talents that work on a white swing (17/34's): Cruelty
/// 5, Unbridled Wrath 5, Dual Wield Specialization 5, Enrage 5, Flurry 5,
/// Impale 2, Deep Wounds 3, Anger Management.
const FURY_WHITE: &[u32] = &[
    12_856, 13_002, 23_588, 13_048, 12_974, 16_494, 12_868, 12_296,
];

/// Probe: a raid-geared warrior's white damage at the dummy from behind,
/// two-handed and dual-wielding, bare and with Fury's talents.
#[tokio::test]
#[ignore = "extended"]
async fn probe_white_damage() {
    if !have_data() {
        return;
    }
    for (k, (hands, talents)) in [
        ("2h", FURY_WHITE),
        ("dual", FURY_WHITE),
        ("dual sword", FURY_WHITE),
        ("dual axe", FURY_WHITE),
        ("dual mace", FURY_WHITE),
    ]
    .into_iter()
    .enumerate()
    {
        let realm = realm();
        let (_c, g, map) = enter(
            &realm,
            "WDPS",
            ["Whirla", "Whirlb", "Whirlc", "Whirld", "Whirle"][k],
        )
        .await;
        let out = map
            .query(move |m| {
                let mut gear = vec!["raid", "dps"];
                gear.extend(hands.split(' '));
                ready(m, g, &gear, talents);
                let d = dummy_behind(m, g, 2.0);
                let f = m.unit_fields(g).unwrap();
                let fields = (
                    f.get_f32(UNIT_FIELD_MINDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXDAMAGE),
                    f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                    f.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE),
                    f.get_f32(UNIT_FIELD_MAXOFFHANDDAMAGE),
                    f.get_u32(UNIT_FIELD_BASEATTACKTIME + 1),
                    f.get_u32(UNIT_FIELD_ATTACK_POWER) + f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS),
                );
                let armor = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_RESISTANCES);
                let r = combat::roll_for(m, g, d, combat::Hand::Main).unwrap();
                let white = swing_for(m, g, d, 120, true);
                (fields, armor, r.crit, r.hit, white)
            })
            .await
            .unwrap();
        eprintln!(
            "{hands} {} talents: fields {:?}, armor {}, crit {}, hit {}: {} damage in 120 s ({:.0} dps), {} swings, {:.1} rage a second",
            talents.len(),
            out.0,
            out.1,
            out.2,
            out.3,
            out.4.damage,
            out.4.damage as f64 / 120.0,
            out.4.swings,
            f64::from(out.4.rage) / 10.0 / 120.0
        );
    }
}

/// A warrior's blows at a boss's armor, rolls fixed (a hit, no crit), as
/// 1.12 has them: Bloodthirst 45% of attack power, Mortal Strike and
/// Whirlwind normalized weapon damage (a two-hander's 3.3 speed; Mortal
/// Strike's 160 more), Heroic Strike the swing and 157, Execute 600 and 15
/// a rage point (cast here without its cost), each less the dummy's armor
/// (mangos-classic's `CalcArmorReducedDamage`).
#[tokio::test]
#[ignore = "extended"]
async fn a_warriors_blows_deal_their_1_12_damage() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "WDPSB", "Blowsa").await;
    let (ap, weapon, speed, armor, dealt) = map
        .query(move |m| {
            ready(
                m,
                g,
                &["raid", "dps", "2h"],
                &[23_894, 21_553, 1680, 25_286, EXECUTE_5],
            );
            let d = dummy_behind(m, g, 2.0);
            let f = m.unit_fields(g).unwrap();
            let ap = f.get_u32(UNIT_FIELD_ATTACK_POWER) + f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS);
            let weapon = (f.get_f32(UNIT_FIELD_MINDAMAGE) + f.get_f32(UNIT_FIELD_MAXDAMAGE)) / 2.0;
            let speed = f.get_u32(UNIT_FIELD_BASEATTACKTIME) as f32 / 1000.0;
            let armor = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_RESISTANCES);
            let max = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            let health = |m: &Map| m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let mut dealt = Vec::new();
            for spell in [23_894u32, 21_553, 1680, EXECUTE_5] {
                if spell == EXECUTE_5 {
                    m.unit_fields_mut(d)
                        .unwrap()
                        .set_u32(UNIT_FIELD_HEALTH, max / 10);
                    m.unit_fields_mut(g)
                        .unwrap()
                        .set_u32(UNIT_FIELD_POWER1 + 1, 450);
                }
                let before = health(m);
                crate::game::map::spells::cast_triggered(m, g, spell, Some(d));
                dealt.push(before - health(m));
                m.unit_fields_mut(d)
                    .unwrap()
                    .set_u32(UNIT_FIELD_HEALTH, max);
                m.now += Duration::from_secs(20);
            }
            // Heroic Strike, queued: the next swing.
            m.unit_fields_mut(g)
                .unwrap()
                .set_u32(UNIT_FIELD_POWER1 + 1, 1000);
            combat::start_attack(m, g, d);
            let now = m.now;
            if let Some(c) = m.combat_mut(g) {
                c.queued = Some(25_286);
                c.next_swing = Some(now);
            }
            let before = health(m);
            combat::try_swing(m, g);
            dealt.push(before - health(m));
            combat::stop_attack(m, g);
            (ap, weapon, speed, armor, dealt)
        })
        .await
        .unwrap();
    assert!(armor > 3000, "a boss's armor: {armor}");
    let kept = 1.0 - armor as f32 / (armor as f32 + 400.0 + 85.0 * 60.0);
    let ap = ap as f32;
    let normalized = weapon - ap / 14.0 * speed + ap / 14.0 * 3.3;
    let want = [
        0.45 * ap * kept,
        (normalized + 160.0) * kept,
        normalized * kept,
        (600.0 + 15.0 * 45.0) * kept,
        (weapon + 157.0) * kept,
    ];
    for (name, (got, want)) in [
        "Bloodthirst",
        "Mortal Strike",
        "Whirlwind",
        "Execute",
        "Heroic Strike",
    ]
    .iter()
    .zip(dealt.iter().zip(want))
    {
        assert!(
            (*got as f32 - want).abs() <= 2.0,
            "{name}: {got}, 1.12's {want:.1} (attack power {ap}, weapon {weapon:.1} at {speed} s, armor {armor})"
        );
    }
}

const EXECUTE_5: u32 = 20_662;

/// Execute's strike crits as a blow does: its own roll on the warrior's
/// crit (mangos-classic's `WarriorExecute` casts the strike, 20647, a spell
/// of its own). Before, the strike took the dummy spell's landing, which
/// deals nothing and so never crits: Execute never crit.
#[tokio::test]
#[ignore = "extended"]
async fn executes_strike_crits_on_its_own_roll() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = enter(&realm, "WDPSX", "Executa").await;
    let (plain, crit) = map
        .query(move |m| {
            ready(m, g, &["raid", "dps", "2h"], &[EXECUTE_5]);
            let d = dummy_behind(m, g, 2.0);
            let max = m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            let strike = |m: &mut Map, rolls: &[i32]| {
                m.unit_fields_mut(d)
                    .unwrap()
                    .set_u32(UNIT_FIELD_HEALTH, max / 10);
                m.unit_fields_mut(g)
                    .unwrap()
                    .set_u32(UNIT_FIELD_POWER1 + 1, 450);
                m.spells
                    .queued_rolls
                    .lock()
                    .unwrap()
                    .extend(rolls.iter().copied());
                crate::game::map::spells::cast_triggered(m, g, EXECUTE_5, Some(d));
                m.spells.queued_rolls.lock().unwrap().clear();
                max / 10 - m.unit_fields(d).unwrap().get_u32(UNIT_FIELD_HEALTH)
            };
            // Hit, no crit on the dummy's roll; the strike's own roll 0.
            (strike(m, &[]), strike(m, &[9_999, 9_999, 0]))
        })
        .await
        .unwrap();
    assert!(plain > 0, "Execute lands");
    assert_eq!(crit, plain * 2, "the strike's own crit doubles it");
}
