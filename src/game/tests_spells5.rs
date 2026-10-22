//! Spells' fifth round, against the real data when it is here: triggered
//! holders' worth, crowd control's timings through refreshes and
//! pushback, Symbol of Life's clean revival, the weapon-change lock,
//! Heart of the Wild's form auras; dispels by the stack, diminishing
//! returns' recovery, one caster's overlapping areas, effects' own
//! mechanics, taunts' and Fade's threat, the moments auras end (mounting,
//! stealth, invulnerability, teleports, flights, silithyst), passives held
//! in their forms, and Honorless Target's kill.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapKey, auras};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::harness::load_real(DB.as_path()))
}

/// A Kobold Vermin at `level`, `yards` in front of `g`, hardy and without
/// armor.
fn dummy(m: &mut Map, g: Guid, level: u8, yards: f32) -> Guid {
    spawn(m, g, 6, level, yards)
}

/// A creature of `entry` at `level`, `yards` in front of `g`, hardy and
/// without armor.
fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the creature");
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
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    let c = c.with_base();
    m.add_creature(c)
}

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

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn apply(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default());
}

fn holder(m: &Map, g: Guid, spell: u32) -> Option<auras::Holder> {
    auras::auras(m, g).and_then(|a| a.holders.iter().find(|h| h.spell == spell).cloned())
}

/// Apply `id` from `caster` on `target` as a triggered spell (or not).
fn apply_as(m: &mut Map, caster: Guid, target: Guid, id: u32, triggered: bool) {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    let _ = auras::apply_spell(
        m,
        caster,
        target,
        &s,
        &effects,
        auras::Context {
            triggered,
            ..Default::default()
        },
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn triggered_worth_and_crowd_control_timings() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "MAGE7");
    let mage = c.enter("Magef", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let data = m.realm.data.clone();
            let kobold = dummy(m, mage, 60, 5.0);
            let other = dummy(m, mage, 60, 6.0);
            // A stun is worth 4 on the bar, triggered 2.
            apply_as(m, mage, kobold, 5164, false);
            apply_as(m, mage, other, 5164, true);
            let worth = (
                spells::capacity::worth(&data, kobold, &holder(m, kobold, 5164).unwrap()),
                spells::capacity::worth(&data, other, &holder(m, other, 5164).unwrap()),
            );
            // A player's Polymorph refreshed keeps its drawn delay.
            m.spells.rolls = Some(true);
            apply(m, kobold, mage, 12826);
            let first = holder(m, mage, 12826).and_then(|h| h.heartbeat);
            m.now += Duration::from_secs(3);
            apply(m, kobold, mage, 12826);
            let again = holder(m, mage, 12826).and_then(|h| h.heartbeat);
            let kept = first.and_then(|b| b.player_delay) == again.and_then(|b| b.player_delay)
                && again.and_then(|b| b.break_at) > first.and_then(|b| b.break_at);
            // A creature's Hibernate refreshed keeps its next roll.
            apply(m, mage, other, 2637);
            let roll = holder(m, other, 2637).and_then(|h| h.heartbeat.and_then(|b| b.next_check));
            m.now += Duration::from_secs(2);
            apply(m, mage, other, 2637);
            let roll_again =
                holder(m, other, 2637).and_then(|h| h.heartbeat.and_then(|b| b.next_check));
            m.spells.rolls = None;
            // Pushed back, a channel's aura (and its break) come sooner.
            let before = holder(m, mage, 12826).unwrap();
            auras::delay_source(m, mage, 12826, kobold, Duration::from_secs(3));
            let after = holder(m, mage, 12826).unwrap();
            let sooner = (
                before.expires.zip(after.expires).map(|(a, b)| a - b),
                before
                    .heartbeat
                    .and_then(|b| b.break_at)
                    .zip(after.heartbeat.and_then(|b| b.break_at))
                    .map(|(a, b)| a - b),
            );
            (worth, kept, roll == roll_again && roll.is_some(), sooner)
        })
        .await
        .unwrap();
    let (worth, kept, roll_kept, sooner) = out;
    assert_eq!(worth, (4, 2), "a triggered stun is worth less");
    assert!(kept, "a refresh keeps the drawn delay, from now");
    assert!(roll_kept, "and a creature's next roll");
    assert_eq!(
        sooner,
        (Some(Duration::from_secs(3)), Some(Duration::from_secs(3))),
        "the aura and its break, three seconds sooner"
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn revival_weapon_lock_and_heart_of_the_wild() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "WARRIOR7");
    let w = c.enter("Warre", Race::Human, Class::Warrior).await;
    let mut d = Client::connect(&realm, "DRUID5");
    let dr = d.enter("Druidc", Race::Tauren, Class::Druid).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let map1 = realm.maps.get(&realm, MapKey::world(1));
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            // Symbol of Life: the revived corpse has no loot and no owner.
            let corpse = spawn(m, w, 6172, 10, 5.0);
            // Killed outright: Henze Faulk is unkillable by damage, which
            // left him at 1 health, alive, the revive not reached (and,
            // damage tapping him, tapped by the warrior).
            if let Some(f) = m.unit_fields_mut(corpse) {
                f.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 0);
            }
            crate::game::map::combat::kill(m, w, corpse);
            let killed = m
                .creature(corpse)
                .is_some_and(|c| !c.alive() && c.tapped_by == Some(w));
            assert!(killed, "a corpse, the warrior's");
            spells::cast_triggered(m, w, 8593, Some(corpse));
            let revived = m
                .creature(corpse)
                .map(|c| (c.alive(), c.loot.is_none(), c.tapped_by.is_none()));
            // In a fight armor stays on; a weapon changes once, then waits.
            for entry in [1364, 2488, 2490] {
                let _ = crate::game::map::inventory::add_item(m, w, entry, 1, false);
            }
            m.players.get_mut(&w).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                true,
            );
            let at = |m: &Map, entry: u32| {
                m.players[&w]
                    .items
                    .iter()
                    .find(|i| i.entry == entry)
                    .map(|i| (i.bag, i.slot))
                    .unwrap()
            };
            let from = at(m, 1364);
            crate::game::map::inventory::move_item(m, w, from, (255, 4));
            let vest = at(m, 1364) == from;
            let from = at(m, 2488);
            crate::game::map::inventory::move_item(m, w, from, (255, 15));
            let gladius = at(m, 2488) == (255, 15);
            let locked = m.players[&w].caster.weapon_change_until.is_some();
            let from = at(m, 2490);
            crate::game::map::inventory::move_item(m, w, from, (255, 15));
            let tomahawk = at(m, 2490) == from;
            (revived, vest, gladius, locked, tomahawk)
        })
        .await
        .unwrap();
    let (revived, vest, gladius, locked, tomahawk) = out;
    assert_eq!(revived, Some((true, true, true)), "revived, clean");
    assert!(vest, "no armor change in a fight");
    assert!(gladius && locked, "a weapon change, and its lock");
    assert!(tomahawk, "no second change while locked");
    let out = map1
        .query(move |m| {
            ready(m, dr, 60, &[24894]);
            // Heart of the Wild: stamina in bear form, strength in cat.
            spells::cast_triggered(m, dr, 5487, None);
            let bear = holder(m, dr, 24899).and_then(|h| h.effects.first().map(|x| x.amount));
            spells::cast_triggered(m, dr, 768, None);
            let cat = (
                has(m, dr, 24899),
                holder(m, dr, 24900).and_then(|h| h.effects.first().map(|x| x.amount)),
            );
            auras::remove_spell(m, dr, 768, None);
            let human = has(m, dr, 24900);
            (bear, cat, human)
        })
        .await
        .unwrap();
    let (bear, cat, human) = out;
    let amount = Some(20);
    assert_eq!(bear, amount, "bear form's stamina");
    assert_eq!(cat, (false, amount), "cat form's strength instead");
    assert!(!human, "none out of form");
}

#[tokio::test]
#[ignore = "extended"]
async fn addon_auras_custom_flags_and_single_targets() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "MAGE8");
    let mage = c.enter("Mageg", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let data = m.realm.data.clone();
            // A Defias Conjurer comes in its template's Frost Armor, and
            // gets it back at home.
            let conjurer = spawn(m, mage, 619, 20, 8.0);
            let armored = has(m, conjurer, 12544);
            auras::remove_spell(m, conjurer, 12544, None);
            crate::game::map::spawn_auras::restore(m, conjurer);
            let restored = has(m, conjurer, 12544);
            // The Explosive Sheep leaves its passive and suicide to its fuse.
            let sheep = spawn(m, mage, 2675, 20, 9.0);
            let sheep_ids = crate::game::map::spawn_auras::spells(m, sheep);
            // vmangos's own word on helpful and harmful.
            let polarity = (
                auras::positive(data.spells.get(18788).unwrap()),
                auras::positive(data.spells.get(24131).unwrap()),
            );
            // Fixed damage takes no done bonus; ignored armor none of it.
            let kobold = dummy(m, mage, 60, 5.0);
            let doubled = plain_aura_done(990_700);
            let _ = auras::apply_spell(
                m,
                mage,
                mage,
                &doubled,
                &[(0, 100)],
                auras::Context::default(),
            );
            let mut fire = data.spells.get(133).unwrap().clone();
            let l = spells::outcome::Landing::default();
            let plain = spells::outcome::damage(m, mage, kobold, &fire, 0, 100.0, &l).amount;
            fire.custom_flags |= crate::game::data::spells::CUSTOM_FIXED_DAMAGE;
            let fixed = spells::outcome::damage(m, mage, kobold, &fire, 0, 100.0, &l).amount;
            auras::remove_spell(m, mage, 990_700, None);
            m.creature_mut(kobold)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_RESISTANCES, 10_000);
            let mut blow = data.spells.get(78).unwrap().clone();
            let armored_blow = spells::outcome::damage(m, mage, kobold, &blow, 0, 100.0, &l).amount;
            blow.custom_flags |= crate::game::data::spells::CUSTOM_IGNORE_ARMOR;
            let bare_blow = spells::outcome::damage(m, mage, kobold, &blow, 0, 100.0, &l).amount;
            // Blessing of Freedom takes the root it guards against.
            let mut root = plain_aura_root(990_701);
            root.mechanic = 7;
            let _ =
                auras::apply_spell(m, kobold, mage, &root, &[(0, 1)], auras::Context::default());
            let rooted = has(m, mage, 990_701);
            apply(m, mage, mage, 1044);
            let freed = rooted && !has(m, mage, 990_701);
            // One Polymorph at a time: on another, the first ends.
            let (a, b) = (dummy(m, mage, 60, 6.0), dummy(m, mage, 60, 7.0));
            apply(m, mage, a, 118);
            apply(m, mage, b, 118);
            let sheeped = (has(m, a, 118), has(m, b, 118));
            (
                armored,
                restored,
                sheep_ids,
                polarity,
                plain,
                fixed,
                armored_blow,
                bare_blow,
                sheeped,
                freed,
            )
        })
        .await
        .unwrap();
    let (
        armored,
        restored,
        sheep_ids,
        polarity,
        plain,
        fixed,
        armored_blow,
        bare_blow,
        sheeped,
        freed,
    ) = out;
    assert!(freed, "Blessing of Freedom purges the root");
    assert!(armored && restored, "its template's aura, back at home");
    assert!(
        !sheep_ids.contains(&4051) && !sheep_ids.contains(&8327),
        "the Sheep's own: {sheep_ids:?}"
    );
    assert_eq!(polarity, (true, false), "vmangos's helpful and harmful");
    assert!(
        plain >= 190 && fixed == 100,
        "fixed damage {fixed}, else {plain}"
    );
    assert!(
        armored_blow < bare_blow && bare_blow == 100,
        "armor ignored: {armored_blow} {bare_blow}"
    );
    assert_eq!(sheeped, (false, true), "one Polymorph at a time");
}

/// A root.
fn plain_aura_root(id: u32) -> crate::game::data::spells::SpellTemplate {
    let mut s = plain_aura_done(id);
    s.effects[0].aura = auras::AURA_MOD_ROOT;
    s.effects[0].misc_value = 0;
    s
}

/// +100% of all schools' damage done.
fn plain_aura_done(id: u32) -> crate::game::data::spells::SpellTemplate {
    let mut effects = [crate::game::data::spells::SpellEffect::default(); 3];
    effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: 79,
        misc_value: 0x7f,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    crate::game::data::spells::SpellTemplate {
        id,
        name: "Test Aura".into(),
        equipped_item_class: -1,
        effects,
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn facing_pushback_and_temporary_health() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "ROGUE6");
    let r = c.enter("Roguec", Race::Human, Class::Rogue).await;
    let mut d = Client::connect(&realm, "MAGE9");
    let mage = d.enter("Mageh", Race::Human, Class::Mage).await;
    let mut e = Client::connect(&realm, "WARRIOR8");
    let w = e.enter("Warrf", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, r, 60, &[1776]);
            ready(m, mage, 60, &[116, 689]);
            ready(m, w, 60, &[12975]);
            let data = m.realm.data.clone();
            let mods = spells::mods::Mods::default();
            // Gouge needs its target to face the rogue.
            let kobold = dummy(m, r, 60, 2.0);
            let o = m.players[&r].movement.orientation;
            m.creature_mut(kobold).unwrap().orientation = o;
            let gouge = data.spells.get(1776).unwrap().clone();
            let at = spells::Targets {
                unit: Some(kobold),
                ..Default::default()
            };
            let turned = spells::checks::check(m, r, &gouge, &at, None, false, &mods);
            // A cast pushed back a second, then less.
            let target = dummy(m, mage, 60, 20.0);
            let at = spells::Targets {
                unit: Some(target),
                ..Default::default()
            };
            spells::cast(m, mage, 116, at, None);
            let ends = |m: &Map| m.players[&mage].caster.casting.as_ref().map(|c| c.ends);
            let start = ends(m);
            m.now += Duration::from_millis(1500);
            spells::pushback(m, mage);
            let first = ends(m).zip(start).map(|(a, b)| a - b);
            spells::pushback(m, mage);
            let second = ends(m).zip(start).map(|(a, b)| a - b);
            // Last Stand's health comes and goes with it.
            let before = m.players[&w].fields.get_u32(UNIT_FIELD_HEALTH);
            spells::cast_triggered(m, w, 12975, None);
            let stood = m.players[&w].fields.get_u32(UNIT_FIELD_HEALTH);
            auras::remove_spell(m, w, 12976, None);
            let after = m.players[&w].fields.get_u32(UNIT_FIELD_HEALTH);
            (turned, first, second, before, stood, after)
        })
        .await
        .unwrap();
    let (turned, first, second, before, stood, after) = out;
    assert_eq!(
        turned,
        Some(spells::fail::NOT_INFRONT),
        "Gouge's target must face"
    );
    assert_eq!(first, Some(Duration::from_millis(1000)), "a second back");
    assert_eq!(
        second,
        Some(Duration::from_millis(1500)),
        "then the half second it has"
    );
    assert!(stood > before, "Last Stand's health: {before} to {stood}");
    assert_eq!(after, before, "and gone with it");
}

/// A plain aura of `kind` and `misc`, ended by interrupt `flags`.
fn plain_aura(
    id: u32,
    kind: u32,
    misc: i32,
    flags: u32,
) -> crate::game::data::spells::SpellTemplate {
    let mut s = plain_aura_done(id);
    s.effects[0].aura = kind;
    s.effects[0].misc_value = misc;
    s.aura_interrupt_flags = flags;
    s
}

/// `caster`'s Hammer of Justice on `target`: how long it holds.
fn hammer(m: &mut Map, caster: Guid, target: Guid) -> Option<u32> {
    spells::cast_triggered(m, caster, 853, Some(target));
    holder(m, target, 853).and_then(|h| h.duration)
}

#[tokio::test]
#[ignore = "extended"]
async fn dispels_by_the_stack_diminishing_recovery_and_shared_areas() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "MAGE10");
    let mage = c.enter("Magei", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            // Purge takes one stack a try: two of three, then the last.
            let kobold = dummy(m, mage, 60, 5.0);
            let mut buff = plain_aura_done(990_710);
            buff.dispel = 1;
            buff.stack_amount = 3;
            for _ in 0..3 {
                let _ = auras::apply_spell(
                    m,
                    kobold,
                    kobold,
                    &buff,
                    &[(0, 10)],
                    auras::Context::default(),
                );
            }
            let stacked = holder(m, kobold, 990_710).map(|h| h.stacks);
            spells::cast_triggered(m, mage, 8012, Some(kobold));
            let purged = holder(m, kobold, 990_710).map(|h| h.stacks);
            spells::cast_triggered(m, mage, 370, Some(kobold));
            let gone = !has(m, kobold, 990_710);
            // Hammer of Justice: whole, a half within 15 seconds of the
            // last going, whole again after; no clock while one is on.
            let stunned = dummy(m, mage, 60, 6.0);
            let whole = hammer(m, mage, stunned);
            auras::remove_spell(m, stunned, 853, None);
            m.now += Duration::from_secs(10);
            let half = hammer(m, mage, stunned);
            auras::remove_spell(m, stunned, 853, None);
            m.now += Duration::from_secs(16);
            let again = hammer(m, mage, stunned);
            m.now += Duration::from_secs(20);
            let held_on = hammer(m, mage, stunned);
            // Two of one caster's Blizzards over a kobold tick it once.
            let data = m.realm.data.clone();
            let blizzard = data.spells.get(10).unwrap().clone();
            let (a, b) = (dummy(m, mage, 60, 10.0), dummy(m, mage, 60, 40.0));
            let now = m.now;
            // Each its own dynamic object, as a cast's are.
            let mut object = 0xF110_0000_0000_9000u64;
            for (target, n) in [(a, 2), (b, 1)] {
                let at = m.position_of(target).unwrap();
                for _ in 0..n {
                    object += 1;
                    m.spells.areas.push(spells::Area {
                        caster: mage,
                        spell: blizzard.id,
                        index: 0,
                        amount: 100,
                        at,
                        radius: 5.0,
                        ends: now + Duration::from_secs(8),
                        next_tick: now,
                        period: Duration::from_secs(1),
                        object: Guid::new(object),
                    });
                }
            }
            spells::update(m);
            let lost = [a, b]
                .map(|g| 1_000_000 - m.creature(g).unwrap().fields.get_u32(UNIT_FIELD_HEALTH));
            (stacked, purged, gone, [whole, half, again, held_on], lost)
        })
        .await
        .unwrap();
    let (stacked, purged, gone, hammers, lost) = out;
    assert_eq!(stacked, Some(3));
    assert_eq!(purged, Some(1), "two tries, two stacks");
    assert!(gone, "and the last");
    let whole = hammers[0].expect("stunned");
    assert_eq!(
        hammers,
        [Some(whole), Some(whole / 2), Some(whole), Some(whole / 2)],
        "diminished, recovered, and not while on"
    );
    assert!(lost[1] > 0 && lost[0] == lost[1], "one tick each: {lost:?}");
}

#[tokio::test]
#[ignore = "extended"]
async fn effect_mechanics_taunts_and_fade() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "MAGE11");
    let mage = c.enter("Magej", Race::Human, Class::Mage).await;
    let mut e = Client::connect(&realm, "WARRIOR9");
    let w = e.enter("Warrg", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            ready(m, w, 60, &[]);
            // An effect's own mechanic rolls its own resistance; the
            // spell's is in its hit.
            let kobold = dummy(m, mage, 60, 5.0);
            let resisting = plain_aura(990_720, auras::AURA_MOD_MECHANIC_RESISTANCE, 7, 0);
            let _ = auras::apply_spell(
                m,
                kobold,
                kobold,
                &resisting,
                &[(0, 100)],
                auras::Context::default(),
            );
            let mut root = plain_aura_root(990_721);
            root.effects[0].mechanic = 7;
            let apart = spells::outcome::effect_resisted(m, kobold, &root, 0);
            root.mechanic = 7;
            let in_hit = spells::outcome::effect_resisted(m, kobold, &root, 0);
            // Taunt matches the target's threat for good; Challenging
            // Shout's aura lends it, given back when it ends.
            let threat = |m: &Map, c: Guid| {
                m.creature(c)
                    .unwrap()
                    .threat
                    .iter()
                    .find(|(g, _)| *g == w)
                    .map(|(_, t)| *t)
            };
            let (a, b) = (dummy(m, w, 60, 4.0), dummy(m, w, 60, 5.0));
            for c in [a, b] {
                let cr = m.creature_mut(c).unwrap();
                cr.threat = vec![(mage, 100.0), (w, 10.0)];
                cr.combat.victim = mage;
                cr.combat.attacking = true;
            }
            spells::cast_triggered(m, w, 355, Some(a));
            let taunted = (threat(m, a), m.creature(a).unwrap().combat.victim == w);
            auras::remove_spell(m, a, 355, None);
            let kept = threat(m, a);
            // (Its neighbour came to help: back on the mage.)
            let cr = m.creature_mut(b).unwrap();
            cr.threat = vec![(mage, 100.0), (w, 10.0)];
            cr.combat.victim = mage;
            apply(m, w, b, 1161);
            let lent = (threat(m, b), m.creature(b).unwrap().combat.victim == w);
            auras::remove_spell(m, b, 1161, None);
            let back = threat(m, b);
            // Fade on those already fighting, not on those met after.
            let fighting = dummy(m, w, 60, 6.0);
            m.creature_mut(fighting).unwrap().threat = vec![(w, 1000.0)];
            apply(m, w, w, 586);
            let faded = threat(m, fighting);
            let met = dummy(m, w, 60, 7.0);
            m.creature_mut(met).unwrap().threat = vec![(w, 500.0)];
            auras::remove_spell(m, w, 586, None);
            let unfaded = (threat(m, fighting), threat(m, met));
            // Defiance holds in Defensive Stance only.
            spells::learn(m, w, 12303);
            let unstanced = has(m, w, 12303);
            spells::cast_triggered(m, w, 71, None);
            let defensive = has(m, w, 12303);
            spells::cast_triggered(m, w, 2457, None);
            let battle = has(m, w, 12303);
            let defiance = (unstanced, defensive, battle);
            (
                apart, in_hit, taunted, kept, lent, back, faded, unfaded, defiance,
            )
        })
        .await
        .unwrap();
    let (apart, in_hit, taunted, kept, lent, back, faded, unfaded, defiance) = out;
    assert_eq!(defiance, (false, true, false), "Defiance in its stance");
    assert!(apart && !in_hit, "an effect's mechanic resisted apart");
    assert_eq!(taunted, (Some(100.0), true), "Taunt levels and holds");
    assert_eq!(kept, Some(100.0), "for good");
    assert_eq!(lent, (Some(100.0), true), "Challenging Shout lends");
    assert_eq!(back, Some(10.0), "and gives back");
    assert!(faded.is_some_and(|t| t < 1000.0), "faded: {faded:?}");
    assert_eq!(unfaded, (Some(1000.0), Some(500.0)), "back, and untouched");
}

#[tokio::test]
#[ignore = "extended"]
async fn auras_end_on_mounting_concealment_teleports_and_flights() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "MAGE12");
    let mage = c.enter("Magek", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let on = |m: &mut Map, id: u32, flags: u32| {
                let s = plain_aura(id, auras::AURA_DUMMY, 0, flags);
                let _ = auras::apply_spell(m, mage, mage, &s, &[(0, 1)], auras::Context::default());
            };
            // Mounting ends silithyst and what mounting ends; dismounting
            // what needs the mount.
            apply(m, mage, mage, 29519);
            on(m, 990_730, auras::INTERRUPT_MOUNTING);
            let carrying = has(m, mage, 29519);
            apply(m, mage, mage, 458);
            let mounted = (
                has(m, mage, 458),
                has(m, mage, 29519),
                has(m, mage, 990_730),
            );
            on(m, 990_731, auras::INTERRUPT_NOT_MOUNTED);
            auras::remove_spell(m, mage, 458, None);
            let dismounted = has(m, mage, 990_731);
            // Stealth ends what losing sight does; invulnerability its own.
            on(m, 990_732, spells::interrupts::IMMUNE_OR_LOST_SELECTION);
            apply(m, mage, mage, 1784);
            let stealthed = (has(m, mage, 1784), has(m, mage, 990_732));
            on(m, 990_733, spells::interrupts::INVULNERABLE);
            apply(m, mage, mage, 642);
            let shielded = has(m, mage, 990_733);
            auras::remove_spell(m, mage, 642, None);
            // A teleport in the map ends a teleport's, not a map change's.
            on(m, 990_734, auras::INTERRUPT_TELEPORTED);
            on(m, 990_735, auras::INTERRUPT_CHANGE_MAP);
            let (key, at, o) = {
                let p = &m.players[&mage];
                (m.key, p.position(), p.movement.orientation)
            };
            m.teleport(mage, key, at, o);
            let teleported = (has(m, mage, 990_734), has(m, mage, 990_735));
            // A flight: out of stealth, and what mounting ends.
            on(m, 990_736, auras::INTERRUPT_MOUNTING);
            spells::interrupts::taking_flight(m, mage);
            let flying = (has(m, mage, 1784), has(m, mage, 990_736));
            // Discombobulate knocks a rider off.
            apply(m, mage, mage, 458);
            apply(m, mage, mage, 4060);
            let flying = (flying.0, flying.1 || has(m, mage, 458));
            auras::remove_spell(m, mage, 4060, None);
            // The client's cost fields by school, and the cost they show.
            let flat = plain_aura(990_737, auras::AURA_MOD_POWER_COST_SCHOOL, 0x10, 0);
            let share = plain_aura(990_738, auras::AURA_MOD_POWER_COST_SCHOOL_PCT, 0x10, 0);
            let _ = auras::apply_spell(m, mage, mage, &flat, &[(0, -5)], auras::Context::default());
            let _ = auras::apply_spell(
                m,
                mage,
                mage,
                &share,
                &[(0, -10)],
                auras::Context::default(),
            );
            let f = &m.players[&mage].fields;
            let fields = (
                f.get_u32(UNIT_FIELD_POWER_COST_MODIFIER + 4) as i32,
                f.get_f32(UNIT_FIELD_POWER_COST_MULTIPLIER + 4),
                f.get_f32(UNIT_FIELD_POWER_COST_MULTIPLIER + 2),
            );
            let data = m.realm.data.clone();
            let frostbolt = data.spells.get(116).unwrap();
            let cost = (frostbolt.mana_cost, spells::power_cost(m, mage, frostbolt));
            (
                carrying, mounted, dismounted, stealthed, shielded, teleported, flying, fields,
                cost,
            )
        })
        .await
        .unwrap();
    let (carrying, mounted, dismounted, stealthed, shielded, teleported, flying, fields, cost) =
        out;
    assert!(carrying, "silithyst on");
    assert_eq!(mounted, (true, false, false), "mounting ends them");
    assert!(!dismounted, "dismounting ends what needs the mount");
    assert_eq!(stealthed, (true, false), "stealth ends the seen");
    assert!(!shielded, "Divine Shield ends invulnerability's");
    assert_eq!(teleported, (false, true), "a teleport's, in the map");
    assert_eq!(flying, (false, false), "a flight's, and Discombobulate's");
    assert_eq!(fields, (-5, -0.1, 0.0), "frost's cost fields");
    let (base, now) = cost;
    assert_eq!(
        now,
        ((base as f32 - 5.0) * 0.9) as u32,
        "cost {base} to {now}"
    );
}

#[tokio::test]
async fn honorless_target_keeps_a_kill_from_counting() {
    let mut d = crate::game::data::GameData::empty();
    d.creature_templates.insert(
        700,
        crate::game::data::CreatureTemplate {
            entry: 700,
            name: "Peasant".into(),
            display_ids: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            display_probabilities: [100, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 35,
            health_max: 10,
            civilian: true,
            xp_multiplier: Some(0.0),
            ..Default::default()
        },
    );
    let mut honorless = plain_aura(
        crate::game::map::pvp::SPELL_HONORLESS_TARGET,
        auras::AURA_NO_PVP_CREDIT,
        0,
        0,
    );
    honorless.name = "Honorless Target".into();
    d.spells.spells.insert(honorless.id, honorless.clone());
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(alice).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let kills = map
        .query(move |m| {
            let dishonorable = |m: &Map| {
                m.realm
                    .social
                    .honor
                    .lock()
                    .project(guid::counter(alice))
                    .lifetime_dishonorable
            };
            let at = m.players[&alice].position();
            let marked = crate::game::map::scripts::summon(m, alice, 700, at, 0.0, None).unwrap();
            let _ = auras::apply_spell(
                m,
                marked,
                marked,
                &honorless,
                &[(0, 0)],
                auras::Context::default(),
            );
            crate::game::map::ai::die(m, marked, alice);
            let after_marked = dishonorable(m);
            let plain = crate::game::map::scripts::summon(m, alice, 700, at, 0.0, None).unwrap();
            crate::game::map::ai::die(m, plain, alice);
            (after_marked, dishonorable(m))
        })
        .await
        .unwrap();
    assert_eq!(kills, (0, 1), "the honorless one doesn't count");
}

#[tokio::test]
async fn silithyst_falls_the_moment_its_carrier_mounts() {
    use crate::game::map::outdoor_pvp::SPELL_CARRYING;
    let mut d = crate::game::data::GameData::empty();
    super::tests_pvp::banner(&mut d, 181597, 10);
    let carrying = plain_aura(SPELL_CARRYING, auras::AURA_DUMMY, 0, 0x3A_0000);
    let team = plain_aura(29894, auras::AURA_DUMMY, 0, 0);
    let mount = plain_aura(990_740, auras::AURA_MOUNTED, 0, 0);
    for s in [&carrying, &team, &mount] {
        d.spells.spells.insert(s.id, s.clone());
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::NightElf, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    let map = realm.maps.get(&realm, key);
    let out = map
        .query(move |m| {
            let at = m.players[&g].position();
            for s in [&carrying, &team] {
                let _ = auras::apply_spell(m, g, g, s, &[(0, 0)], auras::Context::default());
            }
            let si = m.outdoor.as_mut().unwrap().silithus.as_mut().unwrap();
            si.carriers.insert(g, at);
            let _ = auras::apply_spell(m, g, g, &mount, &[(0, 0)], auras::Context::default());
            let mounds: Vec<Vector3d> = m
                .gameobjects
                .iter()
                .filter(|(_, o)| o.entry == 181597)
                .map(|(_, o)| o.position)
                .collect();
            (has(m, g, SPELL_CARRYING), has(m, g, 29894), mounds, at)
        })
        .await
        .unwrap();
    let (carried, team_aura, mounds, at) = out;
    assert!(!carried && !team_aura, "off the moment she mounts");
    assert_eq!(mounds.len(), 1, "one mound");
    assert!(
        crate::game::map::combat::dist(mounds[0], at) < 0.01,
        "where she is"
    );
}
