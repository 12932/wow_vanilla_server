//! The capitals' Elite Guards (the user's idea, 2026-10-19, for Stormwind;
//! 2026-10-20, for every capital): each city's guard in looks and side
//! (`cities.rs`: Stormwind's Royal Guard, an Ironforge Guard, a Darnassus
//! Sentinel, a Kor'kron Elite, a Deathguard Elite, a Bluffwatcher), of
//! level 62 and elite, with the Tower Point Warmaster's (14776) toughness
//! and damage and its fight (vmangos's `npc_WarMaster`, as Thistle Tea
//! ports it for Alterac Valley), but for its Charge, which goes for a
//! healer; and weapons of its own, a weapon in each hand swung as a
//! creature with one in its off hand swings (`combat::dual_wields`). They
//! are defined here and added to the world's data after it loads
//! ([`install`]): the database has no row of them.
//!
//! They come to a defended city (`[guards] jaina_cities`, `rexxar_cities`)
//! two ways:
//!
//! - **On the alarm:** as the city's alarm rises through `[guards]
//!   elite_from` (50, between the elite's 40 and the champion's 60),
//!   `elite_count` (2) come near the civilian whose call took it there,
//!   against its attacker, once for the crossing ([`crossed`]). They come,
//!   fight and go as the guards do (`map/guards.rs`): gone five minutes
//!   out of combat or at death, wandering about where they came.
//! - **With its champion:** as Jaina arrives, `jaina_elites` (4) come
//!   about `jaina_elites_yards` (40) around her, each at a point a walk
//!   from her reaches, and walk to her side, taking on any enemy they see
//!   on the way; Rexxar's `rexxar_elites` (4) come around him out of sight
//!   and run in behind him, with Misha. There they guard the champion,
//!   joining their fights ([`with_champion`], [`guard`]). They leave with
//!   them, with Jaina's flash or at Rexxar's run; if the champion dies (or
//!   is gone some other way) they stay as guards do ([`released`]).

use std::f32::consts::{PI, TAU};
use std::time::Duration;

use rand::RngExt;
use tracing::info;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::champions::Champion;
use crate::game::cities::Elite;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::battleground::av_scripts::{self, Builder, ChargeAt};
use crate::game::map::combat::{self, dist};
use crate::game::map::player::PLAYER_FLAGS_GM;

/// The Stormwind Elite Guard's entry: past vmangos's last (18199), and
/// named by none of its tables (the others' are in `cities.rs`).
pub const ELITE_GUARD: u32 = 101_756;
pub const NAME: &str = "Stormwind Elite Guard";
/// Whose looks and side it has, and whose toughness and fight is every
/// Elite Guard's.
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
/// Ids for their events and scripts, clear of Thistle Tea's import (90M)
/// and the valley's (91M).
const BASE: u32 = 92_000_000;
/// The Stormwind one's swords (the user's: two one-handed swords, cool but clean): in
/// its main hand the Alliance's PvP blade, the Grand Marshal's
/// Longsword's look that Stormwind's infantry carry (display 34128), and
/// in its off hand the Grand Marshal's Swiftblade, the same blade in its
/// violet steel (display 36066).
pub const MAIN_HAND: u32 = 21573;
pub const OFF_HAND: u32 = 23456;
/// Its equipment's entry, its own (vmangos's equipment has none of it):
/// each Elite Guard's is its entry.
pub const EQUIPMENT: u32 = ELITE_GUARD;

/// Charge's reach (spell 22911's range, 95).
pub const CHARGE_MIN: f32 = 8.0;
pub const CHARGE_MAX: f32 = 25.0;
/// The most a player's combat reach is (a tauren's), for the look about.
const MAX_REACH: f32 = 2.5;
/// The classes that heal: paladin, priest, shaman and druid.
pub const HEALERS: [u8; 4] = [2, 5, 7, 11];
/// How lately a heal makes a healer the one charged.
pub const HEALED_WITHIN: Duration = Duration::from_secs(5);
/// How far from a champion their Elite Guards keep, around them.
pub const BY_JAINA: f32 = 4.0;
/// Angles tried for a point a walk reaches (the guards' 100).
const TRIES: usize = 100;

/// The Stormwind Elite Guard's template ([`template_of`]).
pub fn template(data: &GameData) -> Option<CreatureTemplate> {
    template_of(data, &crate::game::cities::CITIES[0].elite)
}

/// A city's Elite Guard's template: its city guard's (`elite.looks`), at
/// level 62, elite, with the warmaster's multipliers, swing and class, its
/// own weapons, and no spells of the guard's (its fight is the
/// warmaster's). None without the guard.
pub fn template_of(data: &GameData, elite: &Elite) -> Option<CreatureTemplate> {
    let looks = data.creature_template(elite.looks)?;
    let warmaster = data.creature_template(WARMASTER);
    let mut t = CreatureTemplate {
        entry: elite.entry,
        name: elite.name.into(),
        sub_name: String::new(),
        level_min: LEVEL,
        level_max: LEVEL,
        rank: ELITE,
        health_multiplier: warmaster.map_or(HEALTH_MULTIPLIER, |w| w.health_multiplier),
        damage_multiplier: warmaster.map_or(DAMAGE_MULTIPLIER, |w| w.damage_multiplier),
        base_attack_time: warmaster.map_or(ATTACK_TIME, |w| w.base_attack_time),
        unit_class: warmaster.map_or(WARRIOR, |w| w.unit_class),
        equipment_id: elite.entry,
        spell_list_id: 0,
        gossip_menu_id: 0,
        ai_name: "EventAI".into(),
        script_name: String::new(),
        ..looks.clone()
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

/// Add the cities' Elite Guards to the world's data: their templates,
/// their weapons, and their fight as EventAI (Stormwind's first, its ids
/// as they were).
pub fn install(data: &mut GameData) {
    let mut made = Vec::new();
    for city in crate::game::cities::CITIES {
        let Some(t) = template_of(data, &city.elite) else {
            continue;
        };
        data.creature_templates.insert(city.elite.entry, t);
        data.creature_equipment
            .insert(city.elite.entry, city.elite.weapons);
        data.scripts.ai_events.remove(&city.elite.entry);
        made.push(city.elite.entry);
    }
    let mut b = Builder::new(&mut data.scripts, BASE);
    for entry in made {
        av_scripts::warmaster(
            &mut b,
            entry,
            ChargeAt::Picked(crate::game::map::scripts::T_ELITE_CHARGE),
            None,
        );
    }
}

/// Whom an Elite Guard charges (the user's twist on the warmaster's): a
/// hostile player of a class that heals, in sight, between 8 and 25
/// yards, at random, one who cast a heal in the last five seconds over one
/// who hasn't; with none, its victim at 8 to 25 yards, as the warmaster
/// charges. The yards are past both combat reaches, as Charge's cast
/// measures them (`checks::check_target`, mangos's `GetMinMaxRange`): one
/// nearer would be picked only for the cast to be refused, too close.
pub fn charge_target(map: &Map, g: Guid) -> Option<Guid> {
    let me = map.position_of(g)?;
    let now = map.now;
    let reach = |u: Guid| map.unit_view(u).map_or(0.0, |v| v.combat_reach.max(0.0));
    let mine = reach(g);
    let in_reach = |u: Guid, p: Vector3d| {
        let d = dist(me, p) - mine - reach(u);
        (CHARGE_MIN..=CHARGE_MAX).contains(&d)
    };
    let mut healers: Vec<(Guid, bool)> = Vec::new();
    for u in map.grid.around(me.x, me.y, CHARGE_MAX + mine + MAX_REACH) {
        let Some(p) = map.players.get(&u) else {
            continue;
        };
        let able = p.death == crate::game::map::death::Death::Alive
            && !p.awaiting_worldport
            && !p.fields.has_flag(PLAYER_FLAGS, PLAYER_FLAGS_GM)
            && HEALERS.contains(&p.row.class)
            && in_reach(u, p.position());
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
    in_reach(victim, at).then_some(victim)
}

/// An Elite Guard's charge reached `target`: it fights it, its threat
/// brought level with the most on its list, so that another takes it back
/// only by passing it, as any must pass a victim.
pub fn charged(map: &mut Map, g: Guid, target: Guid) {
    let Some(c) = map
        .creature(g)
        .filter(|c| crate::game::cities::is_elite(c.entry))
    else {
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

/// A defended city's alarm rose through `elite_from` on `caller`'s call
/// against `victim`: `count` of its Elite Guards (`entry`) come as its
/// guards come, with the call's Avoidance. How many came.
pub fn crossed(
    map: &mut Map,
    caller: Guid,
    victim: Guid,
    avoidance: u32,
    count: u32,
    entry: u32,
) -> u32 {
    let mut came = 0;
    for _ in 0..count {
        if crate::game::map::guards::summon(map, caller, entry, victim, avoidance).is_some() {
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

/// How far around Rexxar, where he comes in, his Elite Guards come.
pub const WITH_REXXAR: f32 = 5.0;

/// `who` arrived in `city` as `g`: the city's Elite Guards come with them.
/// Around Jaina, `jaina_elites` of them about `jaina_elites_yards` from
/// her, each at a point a walk from her reaches, facing her, to walk to
/// their places by her (a quarter of a circle apart, for four), taking on
/// any enemy they see on the way; around Rexxar, `rexxar_elites` of them
/// a few yards from him out of sight, to run in behind him. They come with
/// the city's Avoidance, as its guards would, and go as its guards do.
pub fn with_champion(map: &mut Map, who: Champion, g: Guid, city: u32) -> Vec<Guid> {
    let cfg = map.cfg.guards.clone();
    let Some(entry) = crate::game::cities::city(city).map(|c| c.elite.entry) else {
        return Vec::new();
    };
    let Some(at) = map.creature(g).map(|c| c.position) else {
        return Vec::new();
    };
    let avoidance = crate::game::map::guards::Escalation::of(&cfg)
        .avoidance(crate::game::map::guards::alarm(map, city));
    let n = who.settings(&cfg).elites;
    let yards = if who.runs() {
        WITH_REXXAR
    } else {
        cfg.jaina_elites_yards
    };
    let mut came = Vec::new();
    for i in 0..n {
        let Some(spot) = around(map, at, yards) else {
            continue;
        };
        let facing = (at.y - spot.y).atan2(at.x - spot.x);
        let Some(e) = crate::game::map::scripts::summon(
            map,
            g,
            entry,
            spot,
            facing,
            Some((
                Duration::from_secs(cfg.despawn_secs),
                crate::game::map::guards::TIMED_OOC_OR_DEAD,
            )),
        ) else {
            continue;
        };
        // Its place by them: off their facing, the four about them.
        let angle = PI / n as f32 + i as f32 * TAU / n as f32;
        keep_by(map, e, g, angle);
        crate::game::map::guards::shield(map, e, avoidance);
        came.push(e);
    }
    if !came.is_empty() {
        info!(
            "{}: {} {}s come with {}, {yards:.0} yards around",
            map.key,
            came.len(),
            map.realm
                .data
                .creature_template(entry)
                .map_or("Elite Guard", |t| t.name.as_str()),
            who.name(),
        );
    }
    came
}

/// `e` keeps by `leader` at `angle` off its facing: it follows, and its
/// place by it is where a fight's end takes it back.
fn keep_by(map: &mut Map, e: Guid, leader: Guid, angle: f32) {
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, e) {
        s.follow = Some((leader, BY_JAINA, angle));
        s.run = true;
    }
    crate::game::map::creature_groups::join_with(
        map,
        e,
        leader,
        BY_JAINA,
        angle,
        crate::game::data::scripts::GROUP_FORMATION,
    );
}

/// Rexxar's companion (Misha, `entry`) comes with him as `leader`, at his
/// left, facing as he does, to go as his Elite Guards go.
pub fn companion(
    map: &mut Map,
    leader: Guid,
    entry: u32,
    at: Vector3d,
    facing: f32,
) -> Option<Guid> {
    let side = facing + PI / 2.0;
    let spot = Vector3d {
        x: at.x + 2.0 * side.cos(),
        y: at.y + 2.0 * side.sin(),
        z: at.z,
    };
    let despawn = map.cfg.guards.despawn_secs;
    let g = crate::game::map::scripts::summon(
        map,
        leader,
        entry,
        spot,
        facing,
        Some((
            Duration::from_secs(despawn),
            crate::game::map::guards::TIMED_OOC_OR_DEAD,
        )),
    )?;
    keep_by(map, g, leader, PI / 2.0);
    Some(g)
}

/// Who came with `who`, alive.
pub fn company(map: &mut Map, who: Champion) -> Vec<Guid> {
    let mut company = std::mem::take(&mut who.local(map).company);
    company.retain(|e| map.creature(*e).is_some_and(|c| c.alive()));
    who.local(map).company = company.clone();
    company
}

/// Each look at `who` in a city as `g`: their company free to fight joins
/// their fight.
pub fn guard(map: &mut Map, who: Champion, g: Guid) {
    let company = company(map, who);
    let foe = map
        .creature(g)
        .filter(|c| c.alive() && c.combat.attacking)
        .map(|c| c.combat.victim)
        .filter(|v| map.unit_view(*v).is_some_and(|u| u.alive));
    let Some(foe) = foe else {
        return;
    };
    for e in company {
        let free = map
            .creature(e)
            .is_some_and(|c| !c.combat.attacking && !c.evading);
        if free && combat::can_attack(map, e, foe) {
            crate::game::map::ai::assist(map, e, foe);
        }
    }
}

/// Jaina says farewell to her city: her Elite Guards flash as she does.
pub fn flash_with(map: &mut Map, who: Champion) {
    for e in company(map, who) {
        crate::game::map::spells::cast_triggered(
            map,
            e,
            crate::game::champions::TELEPORT_VISUAL,
            None,
        );
    }
}

/// `who` is gone home: their company is gone with them.
pub fn gone_with(map: &mut Map, who: Champion) {
    let company = company(map, who);
    who.local(map).company.clear();
    if company.is_empty() {
        return;
    }
    for e in &company {
        crate::game::map::creature_groups::leave(map, *e);
        crate::game::map::scripts::remove_for_good(map, *e);
    }
    info!(
        "{}: {} who came with {} leave with them",
        map.key,
        company.len(),
        who.name()
    );
}

/// `who` died in a city (or is gone some other way): their company stays
/// as guards do, wandering about where they stand, gone five minutes out
/// of combat or at their deaths, at their own run.
pub fn released(map: &mut Map, who: Champion) {
    let company = company(map, who);
    who.local(map).company.clear();
    let wander = map.cfg.guards.wander_yards;
    let now = map.now;
    for e in &company {
        crate::game::map::creature_groups::leave(map, *e);
        crate::game::champions::set_run(map, *e, 1.0);
        map.keep_awake(*e, false);
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, *e) {
            s.follow = None;
        }
        // Out of a fight, one following its champion stops where it has
        // come to, to wander from there.
        if wander > 0.0
            && map
                .creature(*e)
                .is_some_and(|c| !c.combat.attacking && !c.evading)
        {
            crate::game::map::ai::rest(map, *e, Some(now));
        }
        if let Some(c) = map.creature_mut(*e) {
            c.home = c.position;
            c.home_orientation = c.orientation;
            if wander > 0.0 {
                c.movement_type = crate::game::map::creature::MOVEMENT_RANDOM;
                c.wander_distance = wander;
            }
        }
    }
    if !company.is_empty() {
        info!(
            "{}: {}'s {} who came stay as guards",
            map.key,
            who.name(),
            company.len()
        );
    }
}
