//! Bugs the mapping against Thistle Tea's tests turned up, through the
//! protocol and on the real data when it is here: the dead killed again,
//! containers looted twice, mana in druid forms, quests from items, the
//! rituals' cooldowns, pets' positive autocasts, and the smaller checks
//! after them.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapKey};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// A creature of `entry` at `level`, `yards` in front of `g`, with
/// `health` and no armor.
fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32, health: u32) -> Guid {
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
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
    c.fields.set_u32(UNIT_FIELD_HEALTH, health);
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

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn xp(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(PLAYER_XP)
}

#[tokio::test]
async fn the_dead_are_not_killed_again() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "BUGMAGE1");
    let mage = c.enter("Bugmagea", Race::Human, Class::Mage).await;
    let mut d = Client::connect(&realm, "BUGROGUE1");
    let rogue = d.enter("Bugroguea", Race::Undead, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 10, &[]);
            // A blow on a corpse: no second reward.
            let kobold = spawn(m, mage, 6, 10, 5.0, 100);
            crate::game::map::combat::deal_damage(m, mage, kobold, 100);
            let first = xp(m, mage);
            crate::game::map::combat::deal_damage(m, mage, kobold, 100);
            crate::game::map::combat::deal_damage_school(m, mage, kobold, 100, 4);
            crate::game::map::combat::kill(m, mage, kobold);
            let blows = xp(m, mage);
            // A Frostbolt in flight when its target dies lands on nothing.
            m.spells.travel = Some(true);
            let other = spawn(m, mage, 6, 10, 20.0, 100);
            spells::cast_triggered(m, mage, 116, Some(other));
            let flying = m.spells.in_flight.len();
            crate::game::map::combat::deal_damage(m, mage, other, 100);
            let second = xp(m, mage);
            m.now += Duration::from_secs(3);
            spells::update(m);
            let landed = xp(m, mage);
            m.spells.travel = None;
            // A player killed, then hit again: one honorable kill.
            ready(m, rogue, 60, &[]);
            ready(m, mage, 60, &[]);
            let h = health(m, rogue);
            crate::game::map::combat::deal_damage(m, mage, rogue, h);
            crate::game::map::combat::deal_damage(m, mage, rogue, h);
            crate::game::map::combat::kill(m, mage, rogue);
            let kills = m
                .realm
                .social
                .honor
                .lock()
                .victim_kills(guid::counter(mage), u64::from(guid::counter(rogue)));
            (first, blows, flying, second, landed, kills)
        })
        .await
        .unwrap();
    let (first, blows, flying, second, landed, kills) = out;
    assert!(first > 0, "the kill's experience");
    assert_eq!(blows, first, "a corpse hit again is worth nothing");
    assert_eq!(flying, 1, "the Frostbolt flies");
    assert!(second > first, "the second kobold's experience");
    assert_eq!(landed, second, "a missile on a corpse kills nothing");
    assert_eq!(kills, 1, "one honorable kill of a corpse hit twice");
}
