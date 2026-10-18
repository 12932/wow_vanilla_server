//! Spells against the real data, when it is here (it isn't in CI): a mage's
//! and a rogue's spells at 60, talents, crits, resists and line of sight,
//! and how many of the spells players can have wvs runs.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, Targets, fail, outcome};
use crate::game::map::{Map, MapKey, auras};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

/// A training dummy: a Kobold Vermin at `level`, `yards` in front of `g`,
/// with health to spare.
fn dummy(m: &mut Map, g: Guid, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data.creature_template(6).cloned().expect("Kobold Vermin");
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
        guid::unit(6, counter | 0x80_0000),
        &template,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    let c = c.with_base();
    m.add_creature(c)
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// A player made level `level` with full power and the spells.
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

/// The cast under way finishes now.
fn finish(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m
        .players
        .get_mut(&g)
        .and_then(|p| p.caster.casting.as_mut())
    {
        c.ends = now;
    }
    spells::update(m);
}

/// Ready to cast again: no cast, no cooldowns.
fn rested(m: &mut Map, g: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    p.caster.casting = None;
    p.caster.gcd_until = None;
    p.caster.cooldowns.clear();
}

/// A spell that only gives a hundred percent spell crit.
fn sure_crit() -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_MOD_SPELL_CRIT_CHANCE,
        base_points: 99,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id: 990_001,
        name: "Sure Crit".into(),
        duration_index: 0,
        equipped_item_class: -1,
        effects,
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn a_mage_at_sixty_crits_ignites_and_takes_her_talents() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "MAGE");
    let g = c.enter("Maga", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const FIREBALL: u32 = 25306;
            const FROSTBOLT: u32 = 25304;
            ready(m, g, 60, &[FIREBALL, FROSTBOLT]);
            let kobold = dummy(m, g, 60, 20.0);
            let at = |k| Targets {
                unit: Some(k),
                ..Default::default()
            };
            // Fireball, as trained: three and a half seconds.
            spells::cast(m, g, FIREBALL, at(kobold), None);
            let plain = m.players[&g].caster.casting.as_ref().map(|c| c.duration);
            rested(m, g);
            // Improved Fireball takes half a second off.
            spells::learn(m, g, 12341);
            spells::cast(m, g, FIREBALL, at(kobold), None);
            let improved = m.players[&g].caster.casting.as_ref().map(|c| c.duration);
            // A sure crit with Ignite learned: the crit leaves Ignite.
            spells::learn(m, g, 12848);
            let crit = sure_crit();
            let _ = auras::apply_spell(m, g, g, &crit, &[(0, 100)], auras::Context::default());
            let before = health(m, kobold);
            finish(m, g);
            let dealt = before - health(m, kobold);
            let ignite = auras::auras(m, kobold)
                .and_then(|a| a.holders.iter().find(|h| h.spell == 12654).cloned());
            // A wall between them: no line of sight.
            rested(m, g);
            let me = m.players[&g].position();
            let them = m.creature(kobold).unwrap().position;
            let mid = Vector3d {
                x: (me.x + them.x) / 2.0,
                y: (me.y + them.y) / 2.0,
                z: me.z,
            };
            let across = (them.y - me.y).atan2(them.x - me.x) + std::f32::consts::FRAC_PI_2;
            let half = |k: f32| Vector3d {
                x: mid.x + k * across.cos(),
                y: mid.y + k * across.sin(),
                z: mid.z,
            };
            m.spells.walls.push((half(-5.0), half(5.0)));
            let s = m.realm.data.spells.get(FROSTBOLT).unwrap().clone();
            let mods = spells::mods::for_spell(m, g, &s);
            let blocked = spells::checks::check(m, g, &s, &at(kobold), None, false, &mods);
            m.spells.walls.clear();
            let clear = spells::checks::check(m, g, &s, &at(kobold), None, false, &mods);
            // A hundred spell damage of fire: Fireball's coefficient is 1.
            let mut power = sure_crit();
            power.id = 990_002;
            power.effects[0].aura = auras::AURA_MOD_DAMAGE_DONE;
            power.effects[0].misc_value = 0x4;
            let _ = auras::apply_spell(m, g, g, &power, &[(0, 100)], auras::Context::default());
            let fireball = m.realm.data.spells.get(FIREBALL).unwrap().clone();
            let bonus = outcome::bonus(
                m,
                g,
                kobold,
                &fireball,
                0,
                outcome::Kind::Direct,
                &spells::mods::Mods::default(),
            );
            // A level 60 caster at even level hits 96 in 100; a level 1
            // caster against level 60, 22.
            m.spells.rolls = Some(true);
            let none = spells::mods::Mods::default();
            let even = (0..2000)
                .filter(|_| {
                    outcome::roll_hit(m, g, kobold, &fireball, &none) == outcome::Hit::Resist
                })
                .count();
            let data = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            crate::game::map::stats::set_level(p, &data, 1);
            let low = (0..2000)
                .filter(|_| {
                    outcome::roll_hit(m, g, kobold, &fireball, &none) == outcome::Hit::Resist
                })
                .count();
            (
                plain, improved, dealt, ignite, blocked, clear, even, low, bonus,
            )
        })
        .await
        .unwrap();
    let (plain, improved, dealt, ignite, blocked, clear, even, low, bonus) = out;
    assert!((bonus - 100.0).abs() < 0.5, "spell power's bonus: {bonus}");
    assert_eq!(plain, Some(3500), "Fireball's cast");
    assert_eq!(improved, Some(3000), "with Improved Fireball");
    // Rank 12 does 596 to 760, and half again on a crit.
    assert!(dealt >= 894, "a crit: {dealt}");
    let ignite = ignite.expect("Ignite on the target");
    let per_tick = ignite.effects[0].amount;
    // Two ticks of 40% of the crit between them.
    assert!(
        (per_tick as u32 * 2).abs_diff(dealt * 40 / 100) <= 2,
        "{per_tick} a tick from {dealt}"
    );
    assert_eq!(blocked, Some(fail::LINE_OF_SIGHT));
    assert_eq!(clear, None);
    assert!(
        (40..140).contains(&even),
        "{even} of 2000 resisted at even level"
    );
    assert!(low > 1400, "{low} of 2000 resisted 59 levels up");
}

#[tokio::test]
#[ignore = "extended"]
async fn a_rogues_eviscerate_scales_with_combo_points_and_spends_them() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "ROGUE");
    let g = c.enter("Rogua", Race::Human, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const EVISCERATE: u32 = 31016;
            ready(m, g, 60, &[EVISCERATE]);
            let kobold = dummy(m, g, 60, 2.0);
            let mut hits = Vec::new();
            for points in [1u8, 5] {
                rested(m, g);
                let p = m.players.get_mut(&g).unwrap();
                p.caster.combo_target = kobold;
                p.caster.combo_points = points;
                let before = health(m, kobold);
                spells::cast(
                    m,
                    g,
                    EVISCERATE,
                    Targets {
                        unit: Some(kobold),
                        ..Default::default()
                    },
                    None,
                );
                hits.push((
                    before - health(m, kobold),
                    m.players[&g].caster.combo_points,
                ));
            }
            // With none, it can't be cast.
            rested(m, g);
            let s = m.realm.data.spells.get(EVISCERATE).unwrap().clone();
            let mods = spells::mods::for_spell(m, g, &s);
            let none = spells::checks::check(
                m,
                g,
                &s,
                &Targets {
                    unit: Some(kobold),
                    ..Default::default()
                },
                None,
                false,
                &mods,
            );
            (hits, none)
        })
        .await
        .unwrap();
    let (hits, none) = out;
    let (one, five) = (hits[0].0, hits[1].0);
    assert!(one > 0, "it hit");
    assert!(
        five > one * 25 / 10,
        "five points ({five}) far above one ({one})"
    );
    assert_eq!(hits[0].1, 0, "the points are spent");
    assert_eq!(hits[1].1, 0);
    assert_eq!(none, Some(fail::NO_COMBO_POINTS));
}

/// The aura types wvs gives an effect: in the auras, the stats, the
/// spell outcomes and modifiers, stealth, regeneration and the AI.
pub const AURAS_RUN: &[u32] = &[
    // auras.rs: control, periodics, shapes, mounts, tracking, absorbs,
    // charm, taunt, stealth, sight, scale, feign death, unit flags, Fade.
    1, 2, 3, 5, 6, 7, 8, 11, 12, 16, 18, 20, 21, 23, 24, 25, 26, 27, 36, 44, 45, 53, 56, 60, 61, 62,
    64, 66, 67, 69, 76, 78, 84, 85, 89, 93, 97, 100, 103, 104, 105, 106, 162,
    // derive.rs: stats, resistances, power, speed, haste, chances.
    9, 13, 22, 29, 30, 31, 32, 33, 34, 35, 47, 49, 51, 52, 54, 55, 57, 58, 65, 71, 79, 80, 83, 98,
    99, 101, 122, 124, 129, 130, 132, 133, 137, 138, 140, 141, 142, 143, 150, 158, 166, 167, 171,
    172, 174, 175,
    // spells: immunities, reflection, damage and healing done and taken,
    // resistance penetration and chance, mechanic resistance, costs,
    // pushback, threat, modifiers, Shatter, crit versus types, soul shards.
    10, 14, 28, 39, 40, 41, 59, 72, 73, 74, 77, 86, 87, 90, 107, 108, 112, 113, 114, 115, 117, 118,
    123, 125, 126, 135, 136, 147, 149, 160, 163, 168, 169,
    // stealth's detection and level, Hunter's Mark; the AI's detect range.
    17, 19, 68, 91, 127, 154, // splitting damage, Safe Fall.
    81, 153, 144, // stats.rs: regeneration.
    88, 94, 110, 116, 134, 161, // death.rs: the ghost.
    95,
];

/// Aura types the procs run (the combat agent's `procs::trigger`).
pub const AURAS_PROCS: &[u32] = &[15, 42, 43, 70, 109, 111];

/// Spells whose dummy effect or dummy aura has code here, or carries
/// nothing the spell needs (riding skills, finishers' and Arcane Missiles'
/// markers).
const DUMMIES: &[&str] = &[
    "Berserking",
    "Blood Fury",
    "Cannibalize",
    "Last Stand",
    "Preparation",
    "Cold Snap",
    "Ignite",
    "Execute",
    "Life Tap",
    "Improved Life Tap",
    "Improved Healthstone",
    "Holy Shock",
    "Eviscerate",
    "Expose Armor",
    "Kidney Shot",
    "Rupture",
    "Slice and Dice",
    "Blessing of Light",
    "Greater Blessing of Light",
    "Healing Way",
    "Combustion",
    "Master of Elements",
    "Vampiric Embrace",
    "Arcane Missiles",
    "Seal of Justice",
    "Seal of Light",
    "Seal of Wisdom",
    "Seal of the Crusader",
    "Seal of Command",
    "Seal of Righteousness",
    "Horse Riding",
    "Kodo Riding",
    "Ram Riding",
    "Raptor Riding",
    "Tiger Riding",
    "Wolf Riding",
    "Riding",
    "Mechanostrider Piloting",
    "Undead Horsemanship",
    "Dummy Trigger",
    "Lightning Shield",
    "Enrage",
    "Shadowguard",
    "Magic Absorption",
    "Predatory Strikes",
    "Frost Warding",
    "Improved Fire Ward",
    "Eye for an Eye",
    "Furor",
    "Leader of the Pack",
];

/// The spells a player can have: trainers' to 60, talents, the starting
/// spells, and every spell they trigger.
fn player_spells(data: &crate::game::data::GameData) -> Vec<u32> {
    let mut roots: Vec<u32> = data
        .npc
        .trainers
        .values()
        .chain(data.npc.trainer_templates.values())
        .flatten()
        .map(|t| spells::learns(data, t.spell))
        .collect();
    for t in data.spells.talents.values() {
        roots.extend(t.ranks.iter().copied().filter(|r| *r != 0));
    }
    for race in 1..=8u8 {
        for class in 1..=11u8 {
            if let Some(s) = data.start_info(race, class) {
                roots.extend(s.spells.iter().copied());
            }
        }
    }
    let mut seen = ahash::AHashSet::new();
    let mut todo = roots;
    while let Some(s) = todo.pop() {
        if !seen.insert(s) {
            continue;
        }
        if let Some(t) = data.spells.get(s) {
            todo.extend(
                t.effects
                    .iter()
                    .map(|e| e.trigger_spell)
                    .filter(|t| *t != 0),
            );
        }
    }
    let mut v: Vec<u32> = seen
        .into_iter()
        .filter(|s| data.spells.get(*s).is_some())
        .collect();
    v.sort_unstable();
    v
}

#[test]
#[ignore = "extended"]
fn nine_in_ten_of_the_spells_players_have_run_every_effect() {
    if !have_data() {
        return;
    }
    let data = crate::game::harness::load_real(DB.as_path());
    let all = player_spells(&data);
    assert!(all.len() > 3000, "{}", all.len());
    // Effects other modules run, and the passives that need nothing.
    const ELSEWHERE: &[u32] = &[
        20, 21, 22, 23, 26, 39, 40, 47, 48, 49, 53, 54, 28, 41, 42, 55, 56, 57, 87, 88, 89, 90, 95,
        50, 76, 104, 105, 106, 107, 71, 99, 102, 101, 112, 109, 110, 73, 97, 59, 94, 85, 61, 19,
        45, 72, 37,
    ];
    let runs = |s: &SpellTemplate, procs: bool| {
        s.effects.iter().filter(|e| e.effect != 0).all(|e| {
            let aura_ok = !matches!(e.effect, 6 | 27 | 35)
                || AURAS_RUN.contains(&e.aura)
                || (procs && AURAS_PROCS.contains(&e.aura))
                || (e.aura == 4 && DUMMIES.contains(&s.name.as_str()));
            let effect_ok = (spells::RUN.contains(&e.effect) || ELSEWHERE.contains(&e.effect))
                && (e.effect != 3 || DUMMIES.contains(&s.name.as_str()) || e.trigger_spell != 0);
            effect_ok
                && aura_ok
                && spells::target_supported(e.target_a)
                && spells::target_supported(e.target_b)
        })
    };
    let alone = all
        .iter()
        .filter(|id| runs(data.spells.get(**id).unwrap(), false))
        .count();
    let mut missing: ahash::AHashMap<String, usize> = ahash::AHashMap::new();
    let mut ok = 0;
    for id in &all {
        let s = data.spells.get(*id).unwrap();
        if runs(s, true) {
            ok += 1;
            continue;
        }
        for e in s.effects.iter().filter(|e| e.effect != 0) {
            let dummy_short = (e.effect == 3 && e.trigger_spell == 0 || e.aura == 4)
                && !DUMMIES.contains(&s.name.as_str());
            let key = if dummy_short {
                format!("dummy {}", s.name)
            } else if !spells::RUN.contains(&e.effect) && !ELSEWHERE.contains(&e.effect) {
                format!("effect {}", e.effect)
            } else if matches!(e.effect, 6 | 27 | 35)
                && !AURAS_RUN.contains(&e.aura)
                && !AURAS_PROCS.contains(&e.aura)
            {
                format!("aura {}", e.aura)
            } else {
                format!(
                    "target {}/{} effect {} aura {} ({})",
                    e.target_a, e.target_b, e.effect, e.aura, s.name
                )
            };
            *missing.entry(key).or_default() += 1;
        }
    }
    let share = ok as f32 / all.len() as f32;
    let mut top: Vec<(String, usize)> = missing.into_iter().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1));
    println!(
        "{ok} of {} run every effect ({:.1}%), {alone} without counting the procs ({:.1}%); short: {top:?}",
        all.len(),
        share * 100.0,
        alone as f32 * 100.0 / all.len() as f32
    );
    assert!(share >= 0.9, "{ok} of {}: {top:?}", all.len());
}

#[tokio::test]
#[ignore = "extended"]
async fn auras_cooldowns_and_crowd_control_hold_as_they_should() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "PALA");
    let g = c.enter("Pala", Race::Human, Class::Paladin).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const FORTITUDE: u32 = 1243;
            const HAMMER: u32 = 853;
            const BLIZZARD: u32 = 10;
            ready(m, g, 60, &[FORTITUDE, HAMMER, BLIZZARD]);
            let kobold = dummy(m, g, 60, 4.0);
            // A buff and a cooldown, saved at logout and back at login.
            spells::cast(m, g, FORTITUDE, Targets::default(), None);
            rested(m, g);
            spells::cast(
                m,
                g,
                HAMMER,
                Targets {
                    unit: Some(kobold),
                    ..Default::default()
                },
                None,
            );
            let now = m.now;
            let row = m.players.get_mut(&g).unwrap().to_row(now);
            let saved = (
                row.state.auras.clone(),
                row.state.category_cooldowns.clone(),
            );
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.auras = row.state.auras;
            p.row.state.cooldowns = row.state.cooldowns;
            p.row.state.category_cooldowns = row.state.category_cooldowns;
            p.caster.cooldowns.clear();
            p.caster.category_cooldowns.clear();
            auras::remove_spell(m, g, FORTITUDE, None);
            spells::cooldowns::restore(m, g);
            let p = &m.players[&g];
            let back = p
                .auras
                .holders
                .iter()
                .any(|h| h.spell == FORTITUDE && h.expires.is_some());
            let category = m.realm.data.spells.get(HAMMER).map_or(0, |s| s.category);
            let cooling = p.caster.cooldowns.contains_key(&HAMMER)
                || p.caster.category_cooldowns.contains_key(&category);
            // Stuns diminish: whole, half, a quarter, then immune.
            let mut stuns = Vec::new();
            for _ in 0..4 {
                auras::remove_spell(m, kobold, HAMMER, None);
                rested(m, g);
                m.players
                    .get_mut(&g)
                    .unwrap()
                    .caster
                    .category_cooldowns
                    .clear();
                spells::cast(
                    m,
                    g,
                    HAMMER,
                    Targets {
                        unit: Some(kobold),
                        ..Default::default()
                    },
                    None,
                );
                let now = m.now;
                stuns.push(auras::auras(m, kobold).and_then(|a| {
                    a.holders
                        .iter()
                        .find(|h| h.spell == HAMMER)
                        .and_then(|h| h.expires)
                        .map(|t| t.saturating_duration_since(now).as_millis() as u32)
                }));
            }
            // Blizzard on the kobold's ground ticks on it.
            rested(m, g);
            auras::remove_spell(m, kobold, HAMMER, None);
            let at = m.creature(kobold).unwrap().position;
            let before = health(m, kobold);
            spells::cast(
                m,
                g,
                BLIZZARD,
                Targets {
                    dest: Some(at),
                    ..Default::default()
                },
                None,
            );
            let now = m.now;
            for a in m.spells.areas.iter_mut() {
                a.next_tick = now;
            }
            spells::update(m);
            let froze = before - health(m, kobold);
            let hammer = m.realm.data.spells.get(HAMMER).unwrap().clone();
            let full = m.realm.data.spells.duration(&hammer, 60).unwrap_or(0);
            (saved, back, cooling, stuns, froze, full)
        })
        .await
        .unwrap();
    let (saved, back, cooling, stuns, froze, full) = out;
    assert!(
        saved.0.iter().any(|a| a.spell == 1243),
        "Fortitude saved: {saved:?}"
    );
    assert!(
        !saved.1.is_empty(),
        "the Hammer's cooldown saved: {saved:?}"
    );
    assert!(back, "Fortitude back with its time");
    assert!(cooling, "the Hammer still cooling down");
    // The first stun was the one before the logout: these are the second,
    // third and fourth.
    assert!(
        stuns[0].is_some_and(|d| d.abs_diff(full / 2) <= 50),
        "{full}: {stuns:?}"
    );
    assert!(
        stuns[1].is_some_and(|d| d.abs_diff(full / 4) <= 50),
        "{full}: {stuns:?}"
    );
    assert_eq!(stuns[2], None, "immune the fourth time");
    assert!(froze > 0, "Blizzard ticked");
}
