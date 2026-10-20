//! Town guards as mangos-classic calls them (`CreatureAI::AttackStart` and
//! `HandleSpawnGuard`): each time a creature that calls the guards
//! (`CALLS_GUARDS`, `static_flags1` 0x0800_0000) takes a victim, a new one
//! or another, one guard of its town comes to a reachable point within 30
//! yards of it and attacks its attacker. The guard goes five minutes after
//! its fight (mangos's 30 seconds), or at once when it dies, and wanders
//! about where it came meanwhile.
//!
//! Each town keeps an alarm: the guards called there lately, halved for
//! each `[guards] alarm_half_life_secs` without a call. As it rises the
//! guards come with Avoidance (aura 160, as spell 23198 has it), a call
//! brings two of a capital's elite, and in a defended capital
//! (`[guards] jaina_cities`, `rexxar_cities`; `cities.rs`) rising through
//! `elite_from` brings two of its Elite Guards
//! ([`crate::game::map::elites`]), and at its top its champion comes:
//! Jaina Proudmoore from Theramore to the Alliance's, Rexxar from Desolace
//! to the Horde's ([`crate::game::champions`]).

use std::time::{Duration, Instant};

use ahash::AHashMap;
use rand::RngExt;
use tracing::info;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::dist;

/// `static_flags1`: it calls the guards (mangos's
/// `CreatureStaticFlags::CALLS_GUARDS`, the same bit of vmangos's
/// `static_flags1`, which wvs reads).
pub const STATIC_CALLS_GUARDS: u32 = 0x0800_0000;
/// How far from the caller a guard may come (mangos's `GetRandomPoint`).
pub const SUMMON_RADIUS: f32 = 30.0;
/// Points tried for a reachable one (mangos's 100).
const TRIES: usize = 100;
/// The vmangos despawn type of a summon timed out of combat, gone at
/// death (mangos's `TEMPSPAWN_TIMED_OOC_OR_DEAD_DESPAWN`).
pub(crate) const TIMED_OOC_OR_DEAD: i32 = 2;
/// Avoidance: an aura of less damage from area spells (aura 160).
pub const AVOIDANCE: u32 = 23198;
/// How often a civilian shouts for the guards, at most.
const SHOUT_EVERY: Duration = Duration::from_secs(10);
/// Mulgore's braves, one at random (mangos's Camp Narache and Bloodhoof
/// Village).
const MULGORE_BRAVES: [u32; 4] = [3210, 3211, 3213, 3214];
const MULGORE: u32 = u32::MAX;
/// Razor Hill's civilians call the grunts.
const RAZOR_HILL: u32 = 362;
const ORC_GRUNTS_TEXT: u32 = 4558;

/// Each town's guard for the Alliance and for the Horde: mangos-classic's
/// `HandleSpawnGuard` (its classic areas), with what vmangos's `GuardMgr`
/// knew that mangos's table lacks (the zones, and both sides' guards where
/// an area has both). Shadowglen keeps vmangos's Shadowglen Sentinel:
/// mangos's 4844 is a Shadowforge Surveyor of the Badlands.
const GUARDS: &[(u32, u32, u32)] = &[
    (1519, 68, 0),     // Stormwind City
    (1537, 5595, 0),   // Ironforge
    (1657, 4262, 0),   // Darnassus
    (1637, 0, 3296),   // Orgrimmar
    (1638, 0, 3084),   // Thunder Bluff
    (1497, 0, 5624),   // Undercity
    (9, 1642, 0),      // Northshire
    (24, 1642, 0),     // Northshire Abbey
    (42, 10038, 0),    // Darkshire
    (69, 10037, 0),    // Lakeshire
    (87, 1423, 0),     // Goldshire
    (108, 8096, 0),    // Sentinel Hill
    (131, 727, 0),     // Kharanos
    (132, 853, 0),     // Coldridge Valley
    (144, 8055, 0),    // Thelsamar
    (152, 0, 7980),    // The Bulwark
    (154, 0, 7980),    // Deathknell
    (159, 0, 7980),    // Brill
    (186, 3571, 0),    // Dolanaar
    (188, 12160, 0),   // Shadowglen
    (221, 0, MULGORE), // Camp Narache
    (222, 0, MULGORE), // Bloodhoof Village
    (228, 0, 7489),    // The Sepulcher
    (271, 2386, 0),    // Southshore
    (272, 0, 2405),    // Tarren Mill
    (320, 10696, 0),   // Refuge Pointe
    (321, 0, 2621),    // Hammerfall
    (340, 0, 8155),    // Kargath
    (362, 0, 5953),    // Razor Hill
    (363, 0, 5952),    // Valley of Trials
    (367, 0, 8017),    // Sen'jin Village
    (380, 0, 3501),    // The Crossroads
    (415, 6087, 0),    // Astranaar
    (431, 0, 12903),   // Splintertree Post
    (442, 6086, 0),    // Auberdine
    (460, 0, 7730),    // Sun Rock Retreat
    (484, 0, 9525),    // Freewind Post
    (513, 4979, 0),    // Theramore Isle
    (597, 0, 8154),    // Ghost Walker Post
    (608, 8151, 0),    // Nijel's Point
    (1099, 0, 8147),   // Camp Mojache
    (1116, 7939, 0),   // Feathermoon Stronghold
    (2408, 0, 12338),  // Shadowprey Village
    (2897, 0, 12903),  // Zoram'gar Outpost
    // vmangos's, mangos lacking them.
    (11, 1475, 0),
    (15, 4979, 10036),
    (35, 4624, 4624),
    (45, 10696, 2621),
    (75, 0, 866),
    (85, 0, 7980),
    (117, 0, 1064),
    (130, 0, 7489),
    (141, 3571, 0),
    (150, 15137, 0),
    (215, 0, 7975),
    (267, 2386, 2405),
    (392, 3502, 3502),
    (406, 0, 7730),
    (496, 0, 10036),
    (702, 4262, 0),
    (976, 9460, 9460),
    (2255, 11190, 11190),
    (2268, 16378, 16378),
    (2361, 11822, 11822),
    (3317, 0, 14730),
];

/// A capital's elite, two to a call once its alarm is high: Stormwind's
/// Royal Guard, Orgrimmar's Kor'kron Elite, Undercity's Royal Dreadguard
/// and Thunder Bluff's Honor Guard. Ironforge and Darnassus have none in
/// the data, and send two of their guards.
const ELITE: &[(u32, u32)] = &[(1519, 1756), (1637, 14304), (1497, 13839), (1638, 3083)];

/// The lines a civilian of a faction shouts: the human's, orc's, dwarf's,
/// gnome's, undead's, night elf's, tauren's and troll's (vmangos's).
const FACTION_TEXTS: &[(u32, &[u32])] = &[
    (
        4403,
        &[
            11, 12, 123, 1078, 1575, 53, 56, 84, 210, 534, 1315, 149, 150, 151, 894, 1075, 1077,
            1096, 1577, 371, 1576,
        ],
    ),
    (
        4561,
        &[
            29, 65, 85, 125, 1074, 1174, 1595, 1612, 1619, 83, 106, 714, 1034, 1314, 1215, 1515,
        ],
    ),
    (4583, &[55, 57, 122, 1611, 1618, 694, 1054, 1055, 1217]),
    (8546, &[23, 64, 875]),
    (4484, &[68, 71, 98, 118, 1134, 1154, 412]),
    (4564, &[79, 80, 124, 1076, 1097, 1594, 1600, 1514]),
    (4560, &[104, 105, 995]),
    (4559, &[126, 876, 877]),
];

/// The town an area or zone is of, as the table names it: the area's own
/// row, else its zone's.
pub fn town(area: u32, zone: u32) -> Option<u32> {
    [area, zone]
        .into_iter()
        .find(|a| *a != 0 && GUARDS.iter().any(|(t, _, _)| t == a))
}

/// A town's guards for the Alliance and the Horde (0 for none).
pub fn guards_of(town: u32) -> Option<(u32, u32)> {
    GUARDS
        .iter()
        .find(|(t, _, _)| *t == town)
        .map(|(_, a, h)| (*a, *h))
}

/// A town's elite, if it has one.
pub fn elite_of(town: u32) -> Option<u32> {
    ELITE.iter().find(|(t, _)| *t == town).map(|(_, e)| *e)
}

/// The line a civilian of `faction` shouts in `town`.
pub fn text(town: u32, faction: u32) -> Option<u32> {
    if town == RAZOR_HILL {
        return Some(ORC_GRUNTS_TEXT);
    }
    FACTION_TEXTS
        .iter()
        .find(|(_, factions)| factions.contains(&faction))
        .map(|(t, _)| *t)
}

/// A town's alarm: the guards called there lately.
#[derive(Debug, Clone, Copy)]
pub struct Alarm {
    pub guards: u32,
    pub last_call: Instant,
}

impl Alarm {
    /// The alarm at `now`: halved for each whole `half_life` since the
    /// last call.
    pub fn at(&self, now: Instant, half_life: Duration) -> u32 {
        let halvings =
            now.saturating_duration_since(self.last_call).as_secs() / half_life.as_secs().max(1);
        self.guards
            .checked_shr(halvings.min(32) as u32)
            .unwrap_or(0)
    }
}

/// What a map keeps of its towns' guards, and of the champions where they
/// are.
#[derive(Debug, Default)]
pub struct Guards {
    /// Each town's alarm, by its area (the map and copy are the map's).
    pub alarms: AHashMap<u32, Alarm>,
    /// Jaina's comings and goings through this map.
    pub jaina: crate::game::champions::Local,
    /// Rexxar's.
    pub rexxar: crate::game::champions::Local,
}

/// The settings that escalate a call.
pub struct Escalation {
    pub avoidance_from: u32,
    pub avoidance_step_pct: u32,
    pub avoidance_step_guards: u32,
    pub avoidance_max_pct: u32,
    pub royal_from: u32,
}

impl Escalation {
    pub fn of(cfg: &crate::config::GuardsConfig) -> Self {
        Self {
            avoidance_from: cfg.avoidance_from,
            avoidance_step_pct: cfg.avoidance_step_pct,
            avoidance_step_guards: cfg.avoidance_step_guards,
            avoidance_max_pct: cfg.avoidance_max_pct,
            royal_from: cfg.royal_from,
        }
    }

    /// The Avoidance, in percent, a guard comes with at an alarm of
    /// `alarm`: a step at the threshold and one for each further step's
    /// worth of guards, up to the cap.
    pub fn avoidance(&self, alarm: u32) -> u32 {
        if alarm < self.avoidance_from {
            return 0;
        }
        let steps = 1 + (alarm - self.avoidance_from) / self.avoidance_step_guards.max(1);
        steps
            .saturating_mul(self.avoidance_step_pct)
            .min(self.avoidance_max_pct)
    }

    /// Whether a call at an alarm of `alarm` brings two of the elite.
    pub fn royal(&self, alarm: u32) -> bool {
        alarm >= self.royal_from
    }
}

/// Whether `g` calls the guards: alive, its own, and its template's flag.
pub fn calls_guards(map: &Map, g: Guid) -> bool {
    map.creature(g).is_some_and(|c| {
        c.alive()
            && c.owner.is_none()
            && c.pet.is_none()
            && map
                .realm
                .data
                .creature_template(c.entry)
                .is_some_and(|t| t.static_flags1 & STATIC_CALLS_GUARDS != 0)
    })
}

/// `caller` took `victim` as its victim, a new one or another (mangos's
/// `CreatureAI::AttackStart`): one that calls the guards calls one.
pub fn attack_started(map: &mut Map, caller: Guid, victim: Guid) {
    if victim.is_zero() || victim == caller || !calls_guards(map, caller) {
        return;
    }
    call(map, caller, victim);
}

/// The town a creature stands in: its area's, or its zone's.
fn town_at(map: &mut Map, g: Guid) -> Option<u32> {
    let c = map.creature(g)?;
    let (at, known) = (c.position, c.zone_area);
    let (zone, area) = crate::game::cities::zone_and_area(map, at).or(known)?;
    town(area, zone)
}

/// mangos's `HandleSpawnGuard`, escalated by the town's alarm.
fn call(map: &mut Map, caller: Guid, victim: Guid) {
    let Some(town) = town_at(map, caller) else {
        return;
    };
    let Some((alliance, horde)) = guards_of(town) else {
        return;
    };
    let now = map.now;
    let cfg = map.cfg.guards.clone();
    let half_life = Duration::from_secs(cfg.alarm_half_life_secs);
    let (cooled_from, before) = map
        .guards
        .alarms
        .get(&town)
        .map_or((0, 0), |a| (a.guards, a.at(now, half_life)));
    let esc = Escalation::of(&cfg);
    let Some(entry) = pick(map, caller, victim, alliance, horde) else {
        return;
    };
    let (entry, count) = if esc.royal(before) {
        (elite_of(town).unwrap_or(entry), 2)
    } else {
        (entry, 1)
    };
    shout(map, caller, town, victim);
    let avoidance = esc.avoidance(before);
    let mut came = 0;
    for _ in 0..count {
        // A fresh pick for each: Mulgore's braves are one at random.
        let entry = if count == 2 && elite_of(town).is_none() {
            pick(map, caller, victim, alliance, horde).unwrap_or(entry)
        } else {
            entry
        };
        if summon(map, caller, entry, victim, avoidance).is_some() {
            came += 1;
        }
    }
    if came == 0 {
        return;
    }
    let after = before + came;
    map.guards.alarms.insert(
        town,
        Alarm {
            guards: after,
            last_call: now,
        },
    );
    // Rising through `elite_from` in a defended city: its Elite Guards,
    // once for the crossing (they aren't counted in the alarm).
    let city = crate::game::cities::defended(&cfg, town);
    let elites = match city {
        Some(c) if before < cfg.elite_from && after >= cfg.elite_from => {
            crate::game::map::elites::crossed(
                map,
                caller,
                victim,
                avoidance,
                cfg.elite_count,
                c.elite.entry,
            )
        }
        _ => 0,
    };
    escalated(map, town, cooled_from, before, after, &esc, &cfg, elites);
    if let Some(c) = city
        && after >= c.champion.settings(&cfg).from
    {
        let at = map.creature(caller).map(|c| c.position);
        if let Some(at) = at {
            crate::game::champions::call(map, c.champion, town, at, victim);
        }
    }
}

/// Log the thresholds an alarm crossed, either way, and the Elite Guards
/// that came of it.
#[allow(clippy::too_many_arguments)]
fn escalated(
    map: &Map,
    town: u32,
    cooled_from: u32,
    before: u32,
    after: u32,
    esc: &Escalation,
    cfg: &crate::config::GuardsConfig,
    elites: u32,
) {
    let name = map
        .realm
        .data
        .area(town)
        .map_or_else(|| town.to_string(), |a| format!("{} ({town})", a.name));
    let crossed = |t: u32| before < t && after >= t;
    let city = crate::game::cities::defended(cfg, town);
    let champion = city.map(|c| (c.champion.name(), c.champion.settings(cfg).from));
    // Cooled since the last call below a threshold it had passed.
    if cooled_from > before {
        let mut fell: Vec<&str> = [
            (esc.avoidance_from, "Avoidance"),
            (esc.royal_from, "the elite"),
        ]
        .into_iter()
        .chain(city.map(|c| (cfg.elite_from, c.elite.name)))
        .chain(champion.map(|(name, from)| (from, name)))
        .filter(|(t, _)| cooled_from >= *t && before < *t)
        .map(|(_, what)| what)
        .collect();
        fell.dedup();
        if !fell.is_empty() {
            info!(
                "{}: {name}'s alarm cooled from {cooled_from} to {before} guards: no more {}",
                map.key,
                fell.join(", ")
            );
        }
    }
    if esc.avoidance(after) > esc.avoidance(before) {
        info!(
            "{}: {name}'s alarm at {after} guards: they come with {}% Avoidance",
            map.key,
            esc.avoidance(after)
        );
    }
    if crossed(esc.royal_from) {
        info!(
            "{}: {name}'s alarm at {after} guards: a call brings two of its elite",
            map.key
        );
    }
    if let Some(c) = city
        && elites > 0
    {
        info!(
            "{}: {name}'s alarm at {after} guards: {elites} {}s come",
            map.key, c.elite.name
        );
    }
    if let Some((champion, from)) = champion
        && crossed(from)
    {
        info!(
            "{}: {name}'s alarm at {after} guards: {champion} is called",
            map.key
        );
    }
}

/// The guard sent against `victim`: a town's own, or, where it has one of
/// each side, the side's against the victim's (a player's, or a pet's
/// master's), and against a creature the caller's side's.
fn pick(map: &Map, caller: Guid, victim: Guid, alliance: u32, horde: u32) -> Option<u32> {
    let side = |e: u32| -> Option<u32> {
        match e {
            0 => None,
            MULGORE => Some(MULGORE_BRAVES[rand::rng().random_range(0..MULGORE_BRAVES.len())]),
            e => Some(e),
        }
    };
    match (side(alliance), side(horde)) {
        (Some(a), None) => Some(a),
        (None, Some(h)) => Some(h),
        (None, None) => None,
        (Some(a), Some(h)) => {
            let master = crate::game::map::pets::master(map, victim);
            if let Some(p) = map.players.get(&master) {
                return Some(if p.team == crate::game::registry::Team::Alliance {
                    h
                } else {
                    a
                });
            }
            // Against a creature: the one of the caller's side.
            let data = &map.realm.data;
            let faction = map
                .unit_fields(caller)
                .map(|f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
                .and_then(|f| data.faction_templates.get(&f));
            let friendly = |e: u32| {
                data.creature_template(e)
                    .and_then(|t| data.faction_templates.get(&t.faction))
                    .zip(faction)
                    .is_some_and(|(g, c)| g.friendly_to(c))
            };
            Some(if friendly(h) && !friendly(a) { h } else { a })
        }
    }
}

/// The caller shouts for the guards, at most every ten seconds.
fn shout(map: &mut Map, caller: Guid, town: u32, victim: Guid) {
    let now = map.now;
    let Some(c) = map.creature_mut(caller) else {
        return;
    };
    if c.engagement.guard_shout_at.is_some_and(|t| t > now) {
        return;
    }
    c.engagement.guard_shout_at = Some(now + SHOUT_EVERY);
    let faction = c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
    if let Some(text) = text(town, faction) {
        crate::game::map::scripts::talk(map, caller, text, victim, None);
    }
}

/// A random point within `radius` of `at` a walk reaches (mangos's
/// `GetRandomPoint` and `GetReachableRandomPosition`, 100 tries): on the
/// navmesh, one a path reaches; with none (or its tile still being read),
/// any on the circle, at `at`'s height.
pub fn reachable_point(map: &mut Map, at: Vector3d, radius: f32) -> Option<Vector3d> {
    let mut rng = rand::rng();
    let ready = map.nav.as_mut().is_some_and(|n| n.ready_at(&[at]));
    let Some(nav) = map.nav.as_mut().filter(|_| ready) else {
        let angle = rng.random_range(0.0..std::f32::consts::TAU);
        let d = radius * rng.random_range(0.0f32..1.0).sqrt();
        return Some(Vector3d {
            x: at.x + d * angle.cos(),
            y: at.y + d * angle.sin(),
            z: at.z,
        });
    };
    for _ in 0..TRIES {
        let Some(p) = crate::game::map::world::wander_point(nav, at, radius, &mut rng) else {
            continue;
        };
        if ((p.x - at.x).powi(2) + (p.y - at.y).powi(2)).sqrt() > radius {
            continue;
        }
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

/// One guard of `entry` within 30 yards of `caller`, against `victim`:
/// gone five minutes out of combat or at its death, wandering where it
/// came meanwhile, with `avoidance` percent of Avoidance.
pub fn summon(
    map: &mut Map,
    caller: Guid,
    entry: u32,
    victim: Guid,
    avoidance: u32,
) -> Option<Guid> {
    let (at, facing) = map.creature(caller).map(|c| (c.position, c.orientation))?;
    let spot = reachable_point(map, at, SUMMON_RADIUS)?;
    let cfg = map.cfg.guards.clone();
    let guard = crate::game::map::scripts::summon(
        map,
        caller,
        entry,
        spot,
        facing,
        Some((Duration::from_secs(cfg.despawn_secs), TIMED_OOC_OR_DEAD)),
    )?;
    let now = map.now;
    if let Some(c) = map.creature_mut(guard)
        && cfg.wander_yards > 0.0
    {
        c.movement_type = crate::game::map::creature::MOVEMENT_RANDOM;
        c.wander_distance = cfg.wander_yards;
        c.motion = crate::game::map::creature::Motion::Idle { until: Some(now) };
    }
    shield(map, guard, avoidance);
    crate::game::map::ai::attacked(map, guard, victim);
    Some(guard)
}

/// A guard comes with `avoidance` percent of Avoidance (none for 0), kept
/// through its evades.
pub fn shield(map: &mut Map, g: Guid, avoidance: u32) {
    if avoidance == 0 {
        return;
    }
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.guard_avoidance = Some(avoidance as i32);
    }
    restore(map, g);
}

/// A guard's Avoidance on it (as it came, and back after an evade).
pub fn restore(map: &mut Map, g: Guid) {
    let Some(amount) = crate::game::map::event_ai::scripted(map, g).and_then(|s| s.guard_avoidance)
    else {
        return;
    };
    if crate::game::map::auras::auras(map, g).is_some_and(|a| a.has_spell(AVOIDANCE)) {
        return;
    }
    let data = map.realm.data.clone();
    if let Some(s) = data.spells.get(AVOIDANCE) {
        let _ = crate::game::map::auras::apply_spell(
            map,
            g,
            g,
            s,
            &[(0, amount)],
            crate::game::map::auras::Context::default(),
        );
    }
}

/// The alarm of `town` on this map now.
pub fn alarm(map: &Map, town: u32) -> u32 {
    let half_life = Duration::from_secs(map.cfg.guards.alarm_half_life_secs);
    map.guards
        .alarms
        .get(&town)
        .map_or(0, |a| a.at(map.now, half_life))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_mangos_with_vmangos_where_mangos_has_none() {
        assert_eq!(guards_of(1519), Some((68, 0)), "Stormwind");
        assert_eq!(guards_of(87), Some((1423, 0)), "Goldshire, mangos's");
        assert_eq!(guards_of(1497), Some((0, 5624)), "Undercity, mangos's");
        assert_eq!(guards_of(267), Some((2386, 2405)), "Hillsbrad, both sides");
        assert_eq!(
            town(1617, 1519),
            Some(1519),
            "the Valley of Heroes is Stormwind's"
        );
        assert_eq!(town(999_999, 0), None);
        assert_eq!(elite_of(1519), Some(1756));
        assert_eq!(elite_of(1537), None, "Ironforge has none");
        assert_eq!(text(362, 11), Some(4558));
        assert_eq!(text(1, 11), Some(4403));
        // Every area once.
        let mut areas: Vec<u32> = GUARDS.iter().map(|(a, _, _)| *a).collect();
        areas.sort_unstable();
        let n = areas.len();
        areas.dedup();
        assert_eq!(areas.len(), n);
    }

    #[test]
    fn an_alarm_halves_for_each_half_life_without_a_call() {
        let t0 = Instant::now();
        let h = Duration::from_secs(300);
        let a = Alarm {
            guards: 64,
            last_call: t0,
        };
        assert_eq!(a.at(t0 + Duration::from_secs(299), h), 64);
        assert_eq!(a.at(t0 + Duration::from_secs(300), h), 32);
        assert_eq!(a.at(t0 + Duration::from_secs(1500), h), 2);
        assert_eq!(a.at(t0 + Duration::from_secs(100_000), h), 0);
    }

    #[test]
    fn avoidance_steps_from_its_threshold_to_its_cap() {
        let e = Escalation::of(&crate::config::GuardsConfig::default());
        let seen: Vec<u32> = [0, 19, 20, 29, 30, 65, 110, 500]
            .iter()
            .map(|n| e.avoidance(*n))
            .collect();
        assert_eq!(seen, [0, 0, 5, 5, 10, 25, 50, 50]);
        assert!(!e.royal(39) && e.royal(40));
    }
}
