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
