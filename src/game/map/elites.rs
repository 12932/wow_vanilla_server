//! The Stormwind Elite Guard (the user's idea, 2026-10-19): a Stormwind
//! Royal Guard (1756) in looks and side, of level 62 and elite, with the
//! Tower Point Warmaster's (14776) toughness and damage and its fight
//! (vmangos's `npc_WarMaster`, as Thistle Tea ports it for Alterac Valley),
//! but for its Charge, which goes for a healer; and a sword in each hand,
//! swinging both as a creature with a weapon in its off hand does
//! (`combat::dual_wields`). It is defined here and added to the world's
//! data after it loads ([`install`]): the database has no row of it.
//!
//! It comes to Jaina's city (`[guards] jaina_city`, Stormwind) two ways:
//!
//! - **On the alarm:** as the city's alarm rises through `[guards]
//!   elite_from` (50, between the Royal Guards' 40 and Jaina's 60),
//!   `elite_count` (2) come near the civilian whose call took it there,
//!   against its attacker, once for the crossing ([`crossed`]). They come,
//!   fight and go as the guards do (`map/guards.rs`): gone five minutes
//!   out of combat or at death, wandering about where they came.
//! - **With Jaina:** as she arrives, `jaina_elites` (4) come about
//!   `jaina_elites_yards` (40) around her, each at a point a walk from her
//!   reaches, and walk to her side, taking on any enemy they see on the
//!   way; there they guard her, joining her fights ([`with_jaina`],
//!   [`guard`]). They leave with her, with the same flash; if she dies (or
//!   is gone some other way) they stay as guards do ([`released`]).

use std::f32::consts::{PI, TAU};
use std::time::Duration;

use rand::RngExt;
use tracing::info;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::battleground::av_scripts::{self, Builder, ChargeAt};
use crate::game::map::combat::{self, dist};
use crate::game::map::player::PLAYER_FLAGS_GM;

/// Its entry: past vmangos's last (18199), and named by none of its
/// tables.
pub const ELITE_GUARD: u32 = 101_756;
pub const NAME: &str = "Stormwind Elite Guard";
/// Whose looks and side it has, and whose toughness and fight.
pub const ROYAL_GUARD: u32 = 1756;
pub const WARMASTER: u32 = 14776;
pub const LEVEL: u8 = 62;
/// An elite's rank.
const ELITE: u32 = 1;
/// The warmaster's multipliers, swing and class, for a world without its
/// row (a test's).
const HEALTH_MULTIPLIER: f32 = 20.0;
const DAMAGE_MULTIPLIER: f32 = 13.5;
const ATTACK_TIME: u32 = 2000;
const WARRIOR: u8 = 1;
/// Ids for its events and scripts, clear of Thistle Tea's import (90M)
/// and the valley's (91M).
const BASE: u32 = 92_000_000;
/// Its swords (the user's: two one-handed swords, cool but clean): in
/// its main hand the Alliance's PvP blade, the Grand Marshal's
/// Longsword's look that Stormwind's infantry carry (display 34128), and
/// in its off hand the Grand Marshal's Swiftblade, the same blade in its
/// violet steel (display 36066).
pub const MAIN_HAND: u32 = 21573;
pub const OFF_HAND: u32 = 23456;
/// Its equipment's entry, its own (vmangos's equipment has none of it).
pub const EQUIPMENT: u32 = ELITE_GUARD;

/// Charge's reach (spell 22911's range, 95).
pub const CHARGE_MIN: f32 = 8.0;
pub const CHARGE_MAX: f32 = 25.0;
/// The classes that heal: paladin, priest, shaman and druid.
pub const HEALERS: [u8; 4] = [2, 5, 7, 11];
/// How lately a heal makes a healer the one charged.
pub const HEALED_WITHIN: Duration = Duration::from_secs(5);
/// How far from Jaina her Elite Guards keep, around her.
pub const BY_JAINA: f32 = 4.0;
/// Angles tried for a point a walk reaches (the guards' 100).
const TRIES: usize = 100;

/// The Elite Guard's template: the Royal Guard's, at level 62, elite, with
/// the warmaster's multipliers, swing and class, its own two swords, and
/// no spells of the Royal Guard's (its fight is the warmaster's). None
/// without the Royal Guard.
pub fn template(data: &GameData) -> Option<CreatureTemplate> {
    let royal = data.creature_template(ROYAL_GUARD)?;
    let warmaster = data.creature_template(WARMASTER);
    let mut t = CreatureTemplate {
        entry: ELITE_GUARD,
        name: NAME.into(),
        sub_name: String::new(),
        level_min: LEVEL,
        level_max: LEVEL,
        rank: ELITE,
        health_multiplier: warmaster.map_or(HEALTH_MULTIPLIER, |w| w.health_multiplier),
        damage_multiplier: warmaster.map_or(DAMAGE_MULTIPLIER, |w| w.damage_multiplier),
        base_attack_time: warmaster.map_or(ATTACK_TIME, |w| w.base_attack_time),
        unit_class: warmaster.map_or(WARRIOR, |w| w.unit_class),
        equipment_id: EQUIPMENT,
        spell_list_id: 0,
        gossip_menu_id: 0,
        ai_name: "EventAI".into(),
        script_name: String::new(),
        ..royal.clone()
    };
    let health = data
        .class_level_stats
        .get(&(t.unit_class, LEVEL))
        .map_or(0, |s| {
            crate::game::data::load::creature_health(s.health, t.health_multiplier)
        });
    t.health_min = health;
    t.health_max = health;
    Some(t)
}

/// Add the Elite Guard to the world's data: its template, its swords,
/// and its fight as EventAI.
pub fn install(data: &mut GameData) {
    let Some(t) = template(data) else {
        return;
    };
    data.creature_templates.insert(ELITE_GUARD, t);
    data.creature_equipment
        .insert(EQUIPMENT, [MAIN_HAND, OFF_HAND, 0]);
    data.scripts.ai_events.remove(&ELITE_GUARD);
    let mut b = Builder::new(&mut data.scripts, BASE);
    av_scripts::warmaster(
        &mut b,
        ELITE_GUARD,
        ChargeAt::Picked(crate::game::map::scripts::T_ELITE_CHARGE),
        None,
    );
}

/// Whom an Elite Guard charges (the user's twist on the warmaster's): a
/// hostile player of a class that heals, in sight, between 8 and 25
/// yards, at random, one who cast a heal in the last five seconds over one
/// who hasn't; with none, its victim at 8 to 25 yards, as the warmaster
/// charges.
pub fn charge_target(map: &Map, g: Guid) -> Option<Guid> {
    let me = map.position_of(g)?;
    let now = map.now;
    let in_reach = |p: Vector3d| (CHARGE_MIN..=CHARGE_MAX).contains(&dist(me, p));
    let mut healers: Vec<(Guid, bool)> = Vec::new();
    for u in map.grid.around(me.x, me.y, CHARGE_MAX) {
        let Some(p) = map.players.get(&u) else {
            continue;
        };
        let able = p.death == crate::game::map::death::Death::Alive
            && !p.awaiting_worldport
            && !p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_GM)
            && HEALERS.contains(&p.row.class)
            && in_reach(p.position());
        if !able
            || !combat::hostile(map, g, u)
            || !combat::can_attack(map, g, u)
            || !crate::game::map::spells::checks::sight(map, me, p.position())
        {
            continue;
        }
        let lately = p
            .misc
            .healed_at
            .is_some_and(|t| now.saturating_duration_since(t) <= HEALED_WITHIN);
        healers.push((u, lately));
    }
    let lately: Vec<Guid> = healers
        .iter()
        .filter(|(_, l)| *l)
        .map(|(u, _)| *u)
        .collect();
    let pool = if lately.is_empty() {
        healers.into_iter().map(|(u, _)| u).collect()
    } else {
        lately
    };
    if !pool.is_empty() {
        return Some(pool[rand::rng().random_range(0..pool.len())]);
    }
    let victim = map
        .creature(g)
        .map(|c| c.combat.victim)
        .filter(|v| !v.is_zero())?;
    let at = map.unit_view(victim).filter(|u| u.alive)?.position;
    in_reach(at).then_some(victim)
}

/// An Elite Guard's charge reached `target`: it fights it, its threat
/// brought level with the most on its list, so that another takes it back
/// only by passing it, as any must pass a victim.
pub fn charged(map: &mut Map, g: Guid, target: Guid) {
    let Some(c) = map.creature(g).filter(|c| c.entry == ELITE_GUARD) else {
        return;
    };
    let top = c.threat.iter().map(|(_, t)| *t).fold(0.0f32, f32::max);
    let had = c
        .threat
        .iter()
        .find(|(u, _)| *u == target)
        .map_or(0.0, |(_, t)| *t);
    if top > had {
        crate::game::map::ai::add_threat(map, g, target, top - had);
    }
}

/// The alarm of Jaina's city rose through `elite_from` on `caller`'s call
/// against `victim`: `count` Elite Guards come as its guards come, with
/// the call's Avoidance. How many came.
pub fn crossed(map: &mut Map, caller: Guid, victim: Guid, avoidance: u32, count: u32) -> u32 {
    let mut came = 0;
    for _ in 0..count {
        if crate::game::map::guards::summon(map, caller, ELITE_GUARD, victim, avoidance).is_some() {
            came += 1;
        }
    }
    came
}

/// A point about `yards` from `at`, at a random angle, that a walk from
/// `at` reaches: on the navmesh, its ground there, with other angles tried
/// while a path from `at` doesn't reach it; with no navmesh (or its tile
/// not in yet), the point on the circle at `at`'s height.
pub fn around(map: &mut Map, at: Vector3d, yards: f32) -> Option<Vector3d> {
    let mut rng = rand::rng();
    let on_circle = |a: f32| Vector3d {
        x: at.x + yards * a.cos(),
        y: at.y + yards * a.sin(),
        z: at.z,
    };
    let ready = map.nav.as_mut().is_some_and(|n| n.ready_at(&[at]));
    let Some(nav) = map.nav.as_mut().filter(|_| ready) else {
        return Some(on_circle(rng.random_range(0.0..TAU)));
    };
    for _ in 0..TRIES {
        let p = on_circle(rng.random_range(0.0..TAU));
        let Some(z) = nav.ground(p.x, p.y, at.z) else {
            continue;
        };
        let p = Vector3d { z, ..p };
        let reached = nav
            .path(at, p)
            .and_then(|path| path.last().copied())
            .is_some_and(|end| dist(end, p) <= 1.0);
        if reached {
            return Some(p);
        }
    }
    None
}

/// Jaina arrived in her city as `jaina`: her Elite Guards come around
/// her, each at a point a walk from her reaches, facing her, and walk to
/// their places by her (a quarter of a circle apart, for four), taking on
/// any enemy they see on the way. They come with the city's Avoidance, as
/// its guards would, and go as its guards do.
pub fn with_jaina(map: &mut Map, jaina: Guid) {
    let cfg = map.cfg.guards.clone();
    let Some(at) = map.creature(jaina).map(|c| c.position) else {
        return;
    };
    let avoidance = crate::game::map::guards::Escalation::of(&cfg)
        .avoidance(crate::game::map::guards::alarm(map, cfg.jaina_city));
    let n = cfg.jaina_elites;
    let mut came = Vec::new();
    for i in 0..n {
        let Some(spot) = around(map, at, cfg.jaina_elites_yards) else {
            continue;
        };
        let facing = (at.y - spot.y).atan2(at.x - spot.x);
        let Some(e) = crate::game::map::scripts::summon(
            map,
            jaina,
            ELITE_GUARD,
            spot,
            facing,
            Some((
                Duration::from_secs(cfg.despawn_secs),
                crate::game::map::guards::TIMED_OOC_OR_DEAD,
            )),
        ) else {
            continue;
        };
        // Its place by her: off her facing, the four about her.
        let angle = PI / n as f32 + i as f32 * TAU / n as f32;
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, e) {
            s.follow = Some((jaina, BY_JAINA, angle));
            s.run = true;
        }
        // Its place by her is where a fight's end takes it back.
        crate::game::map::creature_groups::join_with(
            map,
            e,
            jaina,
            BY_JAINA,
            angle,
            crate::game::data::scripts::GROUP_FORMATION,
        );
        crate::game::map::guards::shield(map, e, avoidance);
        came.push(e);
    }
    if !came.is_empty() {
        info!(
            "{}: {} Stormwind Elite Guards come with Jaina Proudmoore, {:.0} yards around her",
            map.key,
            came.len(),
            cfg.jaina_elites_yards
        );
    }
    map.guards.jaina.elites = came;
}

/// Jaina's Elite Guards, alive.
fn hers(map: &mut Map) -> Vec<Guid> {
    let mut elites = std::mem::take(&mut map.guards.jaina.elites);
    elites.retain(|e| map.creature(*e).is_some_and(|c| c.alive()));
    map.guards.jaina.elites = elites.clone();
    elites
}

/// Each look at Jaina in her city: her Elite Guards free to fight join her
/// fight.
pub fn guard(map: &mut Map, jaina: Guid) {
    let elites = hers(map);
    let foe = map
        .creature(jaina)
        .filter(|c| c.alive() && c.combat.attacking)
        .map(|c| c.combat.victim)
        .filter(|v| map.unit_view(*v).is_some_and(|u| u.alive));
    let Some(foe) = foe else {
        return;
    };
    for e in elites {
        let free = map
            .creature(e)
            .is_some_and(|c| !c.combat.attacking && !c.evading);
        if free && combat::can_attack(map, e, foe) {
            crate::game::map::ai::assist(map, e, foe);
        }
    }
}

/// Jaina says farewell to her city: her Elite Guards flash as she does.
pub fn leaving_with_jaina(map: &mut Map) {
    for e in hers(map) {
        crate::game::map::spells::cast_triggered(map, e, crate::game::jaina::TELEPORT_VISUAL, None);
    }
}

/// Jaina is gone home: her Elite Guards are gone with her.
pub fn gone_with_jaina(map: &mut Map) {
    let elites = hers(map);
    map.guards.jaina.elites.clear();
    if elites.is_empty() {
        return;
    }
    for e in &elites {
        crate::game::map::creature_groups::leave(map, *e);
        crate::game::map::scripts::remove_for_good(map, *e);
    }
    info!(
        "{}: {} Stormwind Elite Guards leave with Jaina Proudmoore",
        map.key,
        elites.len()
    );
}

/// Jaina died in her city (or is gone some other way): her Elite Guards
/// stay as guards do, wandering about where they stand, gone five minutes
/// out of combat or at their deaths.
pub fn released(map: &mut Map) {
    let elites = hers(map);
    map.guards.jaina.elites.clear();
    let wander = map.cfg.guards.wander_yards;
    let now = map.now;
    for e in &elites {
        crate::game::map::creature_groups::leave(map, *e);
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, *e) {
            s.follow = None;
        }
        if let Some(c) = map.creature_mut(*e) {
            c.home = c.position;
            c.home_orientation = c.orientation;
            if wander > 0.0 {
                c.movement_type = crate::game::map::creature::MOVEMENT_RANDOM;
                c.wander_distance = wander;
                if !c.combat.attacking && !c.evading {
                    c.motion = crate::game::map::creature::Motion::Idle { until: Some(now) };
                }
            }
        }
    }
    if !elites.is_empty() {
        info!(
            "{}: Jaina Proudmoore's {} Stormwind Elite Guards stay as guards",
            map.key,
            elites.len()
        );
    }
}
