//! Omen's watch over Elune's lake in Moonglade (vmangos's `boss_omen`),
//! which drives the Lunar Festival's Minions of Omen (event 43), as Thistle
//! Tea's `GameEvent.MinionsOfOmen` keeps it. Fireworks sent up from Omen's
//! cluster launchers wake the lake: the third brings his minions out, the
//! twentieth calls Omen up from the water, unless he fell less than fifteen
//! minutes ago. While he is out the launches go uncounted; the minions stay
//! until he leaves the world: his corpse gone, or himself, alive, after two
//! hours out of a fight (a summon of despawn type 9).

use std::time::Duration;

use wow_world_messages::Guid;

use crate::game::map::Map;

pub const MINIONS_OF_OMEN: u32 = 43;
pub const OMEN: u32 = 15_467;
pub const LAUNCHER: u32 = 180_874;
const MINIONS_AT: u32 = 3;
const OMEN_AT: u32 = 20;
const REST_SECS: i64 = 15 * 60;
const RISES_AT: (f32, f32, f32, f32) = (7_560.01, -2_838.36, 449.575, 4.01426);
/// Omen's time in the world, out of a fight (Thistle Tea's
/// `@omen_lifetime_ms`, despawn type `@timed_combat_or_dead`).
const LIFETIME: Duration = Duration::from_secs(2 * 3_600);
const TIMED_COMBAT_OR_DEAD: i32 = 9;

#[derive(Debug, Clone, Default)]
pub struct Watch {
    pub fireworks: u32,
    /// Omen out and alive: the launches go uncounted.
    pub omen: Option<Guid>,
    /// Omen in the world, alive or his corpse: his minions stay with him.
    pub body: Option<Guid>,
    pub minions: bool,
    pub rests_until: i64,
}

fn now() -> i64 {
    crate::game::social::mail::now()
}

/// A firework from one of Omen's launchers. Whether `object` was one.
pub fn use_object(map: &mut Map, object: Guid) -> bool {
    if map.gameobject(object).is_none_or(|o| o.entry != LAUNCHER) {
        return false;
    }
    let realm = map.realm.clone();
    let (call, minions_now) = {
        let mut w = realm.social.omen.lock();
        if w.omen.is_some() {
            return true;
        }
        w.fireworks += 1;
        let minions_now = !w.minions && w.fireworks >= MINIONS_AT;
        w.minions |= w.fireworks >= MINIONS_AT;
        let call = w.fireworks >= OMEN_AT && now() >= w.rests_until;
        if call {
            w.fireworks = 0;
        }
        (call, minions_now)
    };
    if minions_now {
        crate::game::events::drive(&realm, MINIONS_OF_OMEN, true);
    }
    if call {
        let data = realm.data.clone();
        let counter = map.creature_counter();
        let spawn = crate::game::data::CreatureSpawn {
            guid: counter,
            entries: vec![OMEN],
            map: map.key.map,
            x: RISES_AT.0,
            y: RISES_AT.1,
            z: RISES_AT.2,
            orientation: RISES_AT.3,
            ..Default::default()
        };
        if let Some(mut c) =
            crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())
        {
            c.spawn_guid = None;
            c.summoned_at = Some(map.now);
            let s = c.scripted.get_or_insert_with(Default::default);
            s.despawn_type = TIMED_COMBAT_OR_DEAD;
            s.despawn_after = Some(LIFETIME);
            s.despawn_at = Some(map.now + LIFETIME);
            let g = map.add_creature(c);
            let mut w = realm.social.omen.lock();
            w.omen = Some(g);
            w.body = Some(g);
        }
    }
    true
}

/// A creature died: Omen fallen rests the lake for fifteen minutes. His
/// minions stay until his corpse goes.
pub fn creature_died(map: &mut Map, entry: u32) {
    if entry != OMEN {
        return;
    }
    let mut w = map.realm.social.omen.lock();
    w.omen = None;
    w.rests_until = now() + REST_SECS;
}

/// A creature left the world (its corpse gone, or itself despawned): Omen
/// gone takes his minions with him (Thistle Tea's `MinionsOfOmen.gone`).
/// One who never fell may be called again at once.
pub fn left(map: &mut Map, g: Guid, entry: u32) {
    if entry != OMEN {
        return;
    }
    let realm = map.realm.clone();
    {
        let mut w = realm.social.omen.lock();
        if w.body != Some(g) {
            return;
        }
        w.body = None;
        w.omen = None;
        w.minions = false;
    }
    crate::game::events::drive(&realm, MINIONS_OF_OMEN, false);
}
