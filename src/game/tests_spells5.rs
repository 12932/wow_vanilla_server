//! Spells' fifth round, against the real data when it is here: triggered
//! holders' worth, crowd control's timings through refreshes and
//! pushback, Symbol of Life's clean revival, the weapon-change lock, and
//! Heart of the Wild's form auras.

use std::path::Path;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapKey, auras};

const DB: &str = "<runs>/tt-data/db";

fn have_data() -> bool {
    Path::new(DB).join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(Path::new(DB)))
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
            crate::game::map::combat::deal_damage(m, w, corpse, 2_000_000);
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
