//! The capitals' champions ([`crate::game::cities`]): Lady Jaina Proudmoore
//! comes from Theramore to Stormwind, Ironforge and Darnassus, and Rexxar
//! from Desolace to Orgrimmar, the Undercity and Thunder Bluff, when a
//! defended city's alarm reaches `[guards] jaina_from` or `rexxar_from`
//! (`map/guards.rs`), if alive, at home and at peace there. Once no one is
//! left to fight and a while out of combat (`jaina_stays_secs`,
//! `rexxar_stays_secs`), they go home. Dead in a city, a champion's corpse
//! lies there as any creature's, and is back home after `jaina_respawn_secs`
//! or `rexxar_respawn_secs`.
//!
//! - **Jaina** says she is needed, teleports out (spell 12980's visual, as
//!   mangos's own scenes of her bring her in and out), and comes in by the
//!   fight with the same visual; her Teleport there sends its victim to the
//!   city's prison (`jaina.rs`); she leaves with the visual.
//! - **Rexxar** is no mage, and has no flash: he says he is needed and runs
//!   off `rexxar_run_yards` at `rexxar_run_speed` times his run, Misha
//!   (10204) after him, and is gone; he comes into the city out of sight,
//!   `rexxar_arrive_yards` from the fight, and runs in, Misha and the
//!   city's Elite Guards behind him, his words as he reaches it. He fights
//!   with his template's spells (Thrash, Knock Away, Sundering Cleave,
//!   Puncture) and his EventAI's Frenzy below 30%, Misha with hers. Leaving,
//!   he says he must go, runs off as he came, his company with him, and
//!   they are gone at the run's end.
//!
//! Each is in one place at a time: the realm knows where ([`State`], behind
//! its lock in `Realm::social`), and the maps hand them over by calls. A
//! champion out (or dead) answers no other city's call: that city's next
//! call after the champion is home brings them. The home spawn (and
//! Misha's) stays dead, unseen, its respawn put off, while they are away.
//!
//! The city's Elite Guards come with them ([`crate::game::map::elites`]):
//! around Jaina, walking to her side; with Rexxar, running in behind him.

use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::info;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapKey};

/// Both champions' homes are on Kalimdor: Theramore's tower, and
/// Desolace's Kodo Graveyard road.
pub const HOME_MAP: u32 = 1;
/// Jaina, and her spawn in Theramore.
pub const JAINA: u32 = 4968;
pub const JAINA_SPAWN: u32 = 7895;
/// Rexxar, Champion of the Horde, and his spawn in Desolace; Misha, his
/// bear, and hers (his formation's).
pub const REXXAR: u32 = 10182;
pub const REXXAR_SPAWN: u32 = 29113;
pub const MISHA: u32 = 10204;
pub const MISHA_SPAWN: u32 = 300_850;
/// mangos's `SPELL_TELEPORT_VISUAL`, "Simple Teleport".
pub const TELEPORT_VISUAL: u32 = 12980;
/// How long the teleport's flash shows before Jaina goes.
pub const VANISH: Duration = Duration::from_millis(1500);
/// How long her flash as she comes waits for those near to know her.
const FLASH_WAIT: Duration = Duration::from_secs(1);
/// How near the fight a champion comes.
const ARRIVE_WITHIN: f32 = 8.0;
/// How near the fight's point Rexxar is when he has run in.
const RUN_IN_DONE: f32 = 3.0;
/// The longest Rexxar's run in or off may take.
const LONGEST_RUN: Duration = Duration::from_secs(20);
/// How often a map looks after a champion.
const LOOK_EVERY: Duration = Duration::from_millis(500);
/// A spawn put off "for good".
const NEVER: Duration = Duration::from_secs(10 * 365 * 86_400);
/// How long home may take to see a champion off before the call is
/// forgotten.
const STALE: Duration = Duration::from_secs(30);
/// The ordinary walk and run of a creature with none of its own.
const RUN: f32 = 1.14286;

/// A capital's champion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Champion {
    Jaina,
    Rexxar,
}

pub const ALL: [Champion; 2] = [Champion::Jaina, Champion::Rexxar];

/// How a champion comes and goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    /// With the teleport's flash.
    Teleport,
    /// Running, out of sight.
    Run,
}

/// A champion's own: who, where home is, how they come and go, and their
/// words (`{city}` the city's name).
struct Kind {
    name: &'static str,
    entry: u32,
    spawn: u32,
    home: &'static str,
    /// The one who comes along: Misha.
    companion: Option<(u32, u32)>,
    way: Way,
    leaving: &'static [&'static str],
    arriving: &'static [&'static str],
    going: &'static [&'static str],
}

const JAINA_KIND: Kind = Kind {
    name: "Jaina Proudmoore",
    entry: JAINA,
    spawn: JAINA_SPAWN,
    home: "Theramore",
    companion: None,
    way: Way::Teleport,
    leaving: &[
        "I have urgent matters to tend to. Tervosh, Theramore is in your hands until I return.",
        "{city} calls for aid. I will not be long.",
        "Forgive me, I am needed elsewhere. Keep watch while I am gone.",
    ],
    arriving: &[
        "You dare bring war to the heart of {city}? Not while I stand!",
        "Stand fast, {city}! Help has come.",
        "Enough of this! Leave this city, or I will see you leave it.",
    ],
    going: &[
        "The city is safe for now. Theramore needs me.",
        "{city} can look after itself again. I must return home.",
    ],
};

const REXXAR_KIND: Kind = Kind {
    name: "Rexxar",
    entry: REXXAR,
    spawn: REXXAR_SPAWN,
    home: "Desolace",
    companion: Some((MISHA, MISHA_SPAWN)),
    way: Way::Run,
    leaving: &[
        "{city} is under attack. Misha, with me!",
        "The Horde calls. Come, Misha.",
    ],
    arriving: &[
        "You picked the wrong city to raid. Misha, attack!",
        "{city} does not stand alone!",
        "For the Horde! Get out of {city}!",
    ],
    going: &["I must go.", "I must go. Come, Misha."],
};

/// A champion's settings.
pub struct Settings<'a> {
    pub cities: &'a [u32],
    pub from: u32,
    pub stays_secs: u64,
    pub elites: u32,
    pub respawn_secs: u64,
}

impl Champion {
    fn kind(self) -> &'static Kind {
        match self {
            Champion::Jaina => &JAINA_KIND,
            Champion::Rexxar => &REXXAR_KIND,
        }
    }

    pub fn name(self) -> &'static str {
        self.kind().name
    }

    pub fn entry(self) -> u32 {
        self.kind().entry
    }

    /// Their spawn at home.
    pub fn spawn(self) -> u32 {
        self.kind().spawn
    }

    /// The champion of `entry`.
    pub fn of_entry(entry: u32) -> Option<Champion> {
        ALL.into_iter().find(|c| c.entry() == entry)
    }

    /// Where the realm knows them to be.
    pub fn state(self, realm: &crate::game::Realm) -> &Mutex<State> {
        match self {
            Champion::Jaina => &realm.social.jaina,
            Champion::Rexxar => &realm.social.rexxar,
        }
    }

    /// What a map keeps of their comings and goings through it.
    pub fn local(self, map: &mut Map) -> &mut Local {
        match self {
            Champion::Jaina => &mut map.guards.jaina,
            Champion::Rexxar => &mut map.guards.rexxar,
        }
    }

    pub fn settings(self, cfg: &crate::config::GuardsConfig) -> Settings<'_> {
        match self {
            Champion::Jaina => Settings {
                cities: &cfg.jaina_cities,
                from: cfg.jaina_from,
                stays_secs: cfg.jaina_stays_secs,
                elites: cfg.jaina_elites,
                respawn_secs: cfg.jaina_respawn_secs,
            },
            Champion::Rexxar => Settings {
                cities: &cfg.rexxar_cities,
                from: cfg.rexxar_from,
                stays_secs: cfg.rexxar_stays_secs,
                elites: cfg.rexxar_elites,
                respawn_secs: cfg.rexxar_respawn_secs,
            },
        }
    }

    /// Whether they come running, not with a flash.
    pub fn runs(self) -> bool {
        self.kind().way == Way::Run
    }
}

/// Where a champion is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// At home, alive or dead.
    #[default]
    Home,
    /// Called: home is seeing them off.
    Leaving,
    /// In `city`, on `map`, as `copy`.
    Away { map: MapKey, copy: Guid, city: u32 },
    /// Saying farewell to the city.
    Returning,
}

/// What the realm knows of a champion.
#[derive(Debug, Default)]
pub struct State {
    pub place: Place,
    /// When they were called, while they are leaving: a call lost (a map's
    /// actor gone) gives way after [`STALE`].
    pub called_at: Option<Instant>,
    /// Their visits to their cities.
    pub visits: u32,
}

/// What a map keeps of a champion's comings and goings through it.
#[derive(Debug, Default)]
pub struct Local {
    /// Home: their going, at its time.
    pub departing: Option<Departure>,
    /// The city: their going home, at its time.
    pub returning: Option<(Instant, Guid)>,
    /// The city: since when they have been at peace.
    pub calm_since: Option<Instant>,
    pub look_at: Option<Instant>,
    /// Jaina's flash as she comes, and her words, once those near know her
    /// (or at the time given): `send_to_observers` reaches only those who
    /// do, and she is seen at the update.
    pub flash: Option<(Guid, Instant, Option<String>)>,
    /// Rexxar on his way in: the fight's point, his words there, and the
    /// time he is taken to be there by.
    pub coming: Option<Coming>,
    /// The city: who came with them, the city's Elite Guards and Misha.
    pub company: Vec<Guid>,
}

#[derive(Debug, Clone)]
pub struct Coming {
    pub to: Vector3d,
    pub foe: Guid,
    pub words: String,
    pub by: Instant,
}

#[derive(Debug, Clone, Copy)]
pub struct Departure {
    pub at: Instant,
    pub who: Guid,
    pub to: MapKey,
    pub city: u32,
    pub near: Vector3d,
    pub foe: Guid,
}

fn line(lines: &[&'static str], city: u32) -> String {
    use rand::RngExt;
    let name = crate::game::cities::city(city).map_or("the city", |c| c.name);
    lines[rand::rng().random_range(0..lines.len())].replace("{city}", name)
}

fn say(map: &mut Map, g: Guid, text: &str) {
    crate::game::map::scripts::say(
        map,
        g,
        text,
        crate::game::map::scripts::CHAT_SAY,
        0,
        Guid::zero(),
    );
}

/// The copy of `who` in a city on this map, and the city, if there now.
pub fn away_here(map: &Map, who: Champion) -> Option<(Guid, u32)> {
    match who.state(&map.realm).lock().place {
        Place::Away { map: m, copy, city } if m == map.key => Some((copy, city)),
        _ => None,
    }
}

/// The copy of `who` in a city on this map, if there now.
pub fn copy(map: &Map, who: Champion) -> Option<Guid> {
    away_here(map, who).map(|(g, _)| g)
}

fn home(map: &Map) -> bool {
    map.key == MapKey::world(HOME_MAP)
}

/// `city`'s alarm called `who`, near `near`, against `foe`: if they are at
/// home, home sees them off.
pub fn call(map: &mut Map, who: Champion, city: u32, near: Vector3d, foe: Guid) {
    let realm = map.realm.clone();
    {
        let mut s = who.state(&realm).lock();
        if s.place != Place::Home {
            return;
        }
        s.place = Place::Leaving;
        s.called_at = Some(Instant::now());
    }
    info!("{}: {} is called to area {city}", map.key, who.name());
    let to = map.key;
    realm
        .maps
        .get(&realm, MapKey::world(HOME_MAP))
        .call(move |m| depart(m, who, to, city, near, foe));
}

/// The home spawn of `who` (or of their companion), alive or dead.
fn at_home(map: &Map, spawn: u32) -> Option<Guid> {
    crate::game::map::scripts::creature_of_spawn(map, spawn)
}

/// Home: they go if alive, at home and at peace; they say so and teleport
/// or run out.
fn depart(map: &mut Map, who: Champion, to: MapKey, city: u32, near: Vector3d, foe: Guid) {
    let at_peace = at_home(map, who.spawn()).filter(|g| {
        map.creature(*g)
            .is_some_and(|c| c.alive() && !c.combat.attacking && !c.evading)
    });
    let Some(g) = at_peace else {
        who.state(&map.realm).lock().place = Place::Home;
        info!(
            "{}: {} can't leave {} (dead, gone or fighting)",
            map.key,
            who.name(),
            who.kind().home
        );
        return;
    };
    let kind = who.kind();
    say(map, g, &line(kind.leaving, city));
    let takes = match kind.way {
        Way::Teleport => {
            crate::game::map::spells::cast_triggered(map, g, TELEPORT_VISUAL, None);
            VANISH
        }
        Way::Run => {
            let (yards, mult) = (
                map.cfg.guards.rexxar_run_yards,
                map.cfg.guards.rexxar_run_speed,
            );
            let companion = kind.companion.and_then(|(_, s)| at_home(map, s));
            run_off(map, g, yards, mult, companion.into_iter().collect())
        }
    };
    who.local(map).departing = Some(Departure {
        at: map.now + takes,
        who: g,
        to,
        city,
        near,
        foe,
    });
}

/// `g`'s run speed at `mult` times its own (its template's).
pub fn set_run(map: &mut Map, g: Guid, mult: f32) {
    let data = map.realm.data.clone();
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let own = data
        .creature_template(c.entry)
        .map(|t| t.speed_run)
        .filter(|s| *s > 0.0)
        .unwrap_or(RUN);
    c.base.run_speed = 7.0 * own * mult;
    crate::game::map::derive::unit(map, g);
}

/// `g` runs off about `yards` at `mult` times its run, taking on no one,
/// `with` after it at its speed: how long its run takes.
fn run_off(map: &mut Map, g: Guid, yards: f32, mult: f32, with: Vec<Guid>) -> Duration {
    use crate::game::map::assistance::Reaction;
    let Some(at) = map.creature(g).map(|c| c.position) else {
        return Duration::ZERO;
    };
    for u in std::iter::once(g).chain(with.iter().copied()) {
        set_run(map, u, mult);
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, u) {
            s.reaction = Some(Reaction::Passive);
        }
    }
    let to = crate::game::map::elites::around(map, at, yards).unwrap_or(Vector3d {
        x: at.x + yards,
        ..at
    });
    crate::game::map::scripts::move_to(map, g, to, true);
    let takes = match map.creature(g).map(|c| &c.motion) {
        Some(crate::game::map::creature::Motion::Moving(w)) => w.duration,
        _ => Duration::ZERO,
    };
    // Those with it go where it goes, beside it.
    for u in with {
        let beside = Vector3d {
            x: to.x - 2.0,
            ..to
        };
        crate::game::map::scripts::move_to(map, u, beside, true);
    }
    takes.clamp(Duration::from_millis(500), LONGEST_RUN)
}

/// Home: the flash or the run over, they are gone, and come to the city.
fn gone_from_home(map: &mut Map, who: Champion, d: Departure) {
    if map.creature(d.who).is_none_or(|c| !c.alive()) {
        who.state(&map.realm).lock().place = Place::Home;
        return;
    }
    let mut going = vec![d.who];
    going.extend(who.kind().companion.and_then(|(_, s)| at_home(map, s)));
    for g in going {
        // As they were, for their return.
        set_run(map, g, 1.0);
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
            s.reaction = None;
        }
        crate::game::map::scripts::despawn_with(map, g, Some(NEVER));
    }
    info!("{}: {} leaves {}", map.key, who.name(), who.kind().home);
    let realm = map.realm.clone();
    let (city, near, foe) = (d.city, d.near, d.foe);
    realm
        .maps
        .get(&realm, d.to)
        .call(move |m| arrive(m, who, city, near, foe));
}

/// A point about `yards` from `at` that a walk from it reaches, nearer
/// where none is: out of the fight's sight, for Rexxar to run in from.
fn out_of_sight(map: &mut Map, at: Vector3d, yards: f32) -> Vector3d {
    for share in [1.0, 0.75, 0.5, 0.25] {
        if let Some(p) = crate::game::map::elites::around(map, at, yards * share) {
            return p;
        }
    }
    at
}

/// The city: they come near `near`, Jaina with the teleport's flash and
/// Rexxar running in from out of sight, to fight `foe`.
fn arrive(map: &mut Map, who: Champion, city: u32, near: Vector3d, foe: Guid) {
    let data = map.realm.data.clone();
    let spawn = data
        .creature_spawns
        .get(&HOME_MAP)
        .and_then(|v| v.iter().find(|s| s.guid == who.spawn()))
        .cloned();
    let fight = crate::game::map::guards::reachable_point(map, near, ARRIVE_WITHIN).unwrap_or(near);
    let runs = who.runs();
    let start = if runs {
        out_of_sight(map, fight, map.cfg.guards.rexxar_arrive_yards)
    } else {
        fight
    };
    // Facing the fight.
    let o = if runs {
        (fight.y - start.y).atan2(fight.x - start.x)
    } else {
        map.position_of(foe)
            .map_or(0.0, |p| (p.y - start.y).atan2(p.x - start.x))
    };
    let made = spawn.and_then(|spawn| {
        let counter = map.next_creature_counter;
        map.next_creature_counter += 1;
        let s = crate::game::data::CreatureSpawn {
            guid: counter | 0x80_0000,
            map: map.key.map,
            x: start.x,
            y: start.y,
            z: start.z,
            orientation: o,
            movement_type: 0,
            wander_distance: 0.0,
            spawn_flags: 0,
            ..spawn
        };
        let mut c = crate::game::map::creature::Creature::from_spawn(&data, &s, &mut rand::rng())?;
        c.spawn_guid = None;
        // Its fights end where it came to.
        c.home = fight;
        Some(map.add_creature(c))
    });
    let Some(g) = made else {
        // They can't come: home again.
        who.state(&map.realm).lock().place = Place::Home;
        bring_home(map, who);
        return;
    };
    {
        let mut s = who.state(&map.realm).lock();
        s.place = Place::Away {
            map: map.key,
            copy: g,
            city,
        };
        s.visits += 1;
    }
    let now = map.now;
    let words = line(who.kind().arriving, city);
    let local = who.local(map);
    local.calm_since = None;
    local.returning = None;
    if runs {
        local.coming = Some(Coming {
            to: fight,
            foe,
            words,
            by: now + LONGEST_RUN,
        });
    } else {
        // Seen, then the flash, then her words.
        local.flash = Some((g, now + FLASH_WAIT, Some(words)));
    }
    info!(
        "{}: {} arrives at ({:.1}, {:.1}, {:.1})",
        map.key,
        who.name(),
        start.x,
        start.y,
        start.z
    );
    // At home Jaina's template takes no one on sight (`flags_extra`'s
    // no-aggro); come to defend the city, she does, and after each kill
    // takes the next. (The user saw her stand before 80 Horde: the one she
    // came for had died.) Rexxar too.
    if let Some(s) = crate::game::map::event_ai::scripted_mut(map, g) {
        s.reaction = Some(crate::game::map::assistance::Reaction::Aggressive);
    }
    let mut company = crate::game::map::elites::with_champion(map, who, g, city);
    if runs {
        let mult = map.cfg.guards.rexxar_run_speed;
        set_run(map, g, mult);
        crate::game::map::scripts::move_to(map, g, fight, true);
        if let Some((entry, _)) = who.kind().companion
            && let Some(misha) = crate::game::map::elites::companion(map, g, entry, start, o)
        {
            company.push(misha);
        }
        for e in &company {
            set_run(map, *e, mult);
        }
    } else {
        let fights = map.unit_view(foe).is_some_and(|u| u.alive);
        if fights && crate::game::map::combat::can_attack(map, g, foe) {
            crate::game::map::ai::attacked(map, g, foe);
        }
    }
    who.local(map).company = company;
}

/// Rexxar on his way in: at the fight (or fighting on the way, or his time
/// up), his run is his own again, he says so, and fights the one he came
/// for if no one yet.
fn coming_in(map: &mut Map, who: Champion, g: Guid) {
    let Some(coming) = who.local(map).coming.clone() else {
        return;
    };
    let foe = coming.foe;
    let Some(c) = map.creature(g) else {
        return;
    };
    let flat = ((c.position.x - coming.to.x).powi(2) + (c.position.y - coming.to.y).powi(2)).sqrt();
    let there = flat <= RUN_IN_DONE || c.combat.attacking || coming.by <= map.now;
    if !there {
        return;
    }
    who.local(map).coming = None;
    set_run(map, g, 1.0);
    let company = who.local(map).company.clone();
    for e in company {
        set_run(map, e, 1.0);
    }
    say(map, g, &coming.words);
    let fighting = map.creature(g).is_some_and(|c| c.combat.attacking);
    let alive = map.unit_view(foe).is_some_and(|u| u.alive);
    if !fighting && alive && crate::game::map::combat::can_attack(map, g, foe) {
        crate::game::map::ai::attacked(map, g, foe);
    }
}

/// Tell home to bring `who`'s spawn back now.
fn bring_home(map: &Map, who: Champion) {
    let realm = map.realm.clone();
    realm
        .maps
        .get(&realm, MapKey::world(HOME_MAP))
        .call(move |m| come_home(m, who));
}

/// Home: they are back at their spawn, Jaina with the teleport's flash.
fn come_home(map: &mut Map, who: Champion) {
    let kind = who.kind();
    for spawn in std::iter::once(kind.spawn).chain(kind.companion.map(|(_, s)| s)) {
        if let Some(g) = at_home(map, spawn)
            && map.creature(g).is_some_and(|c| !c.alive())
        {
            crate::game::map::ai::respawn_now(map, g);
        }
    }
    who.state(&map.realm).lock().place = Place::Home;
    if kind.way == Way::Teleport
        && let Some(g) = at_home(map, kind.spawn)
        && map.creature(g).is_some_and(|c| c.alive())
    {
        let at = map.now + FLASH_WAIT;
        who.local(map).flash = Some((g, at, None));
    }
    info!("{}: {} is home in {}", map.key, who.name(), kind.home);
}

/// A creature died on `map`: if it was a champion in a city, they come
/// back home after their respawn time. Their corpse lies as any's.
pub fn creature_died(map: &mut Map, g: Guid) {
    let Some(who) = map.creature(g).and_then(|c| Champion::of_entry(c.entry)) else {
        return;
    };
    {
        let mut s = who.state(&map.realm).lock();
        match s.place {
            Place::Away { map: m, copy, .. } if m == map.key && copy == g => s.place = Place::Home,
            _ => return,
        }
    }
    let local = who.local(map);
    local.returning = None;
    local.calm_since = None;
    local.coming = None;
    crate::game::map::elites::released(map, who);
    let secs = who.settings(&map.cfg.guards).respawn_secs;
    info!(
        "{}: {} died; back in {} in {secs} s",
        map.key,
        who.name(),
        who.kind().home
    );
    let realm = map.realm.clone();
    let kind = who.kind();
    let spawns: Vec<u32> = std::iter::once(kind.spawn)
        .chain(kind.companion.map(|(_, s)| s))
        .collect();
    realm
        .maps
        .get(&realm, MapKey::world(HOME_MAP))
        .call(move |m| {
            let now = m.now;
            for spawn in spawns {
                if let Some(g) = at_home(m, spawn)
                    && let Some(c) = m.creature_mut(g)
                    && let crate::game::map::creature::Life::Dead { respawn_at, .. } = &mut c.life
                {
                    *respawn_at = now + Duration::from_secs(secs);
                }
            }
        });
}

/// Whether `g` is at peace: no one to fight, no one fighting it.
fn calm(map: &Map, g: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return false;
    };
    c.alive()
        && !c.evading
        && !c.combat.attacking
        && c.threat.is_empty()
        && !c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT)
        && !map
            .players
            .keys()
            .any(|p| map.combat(*p).is_some_and(|f| f.attacking && f.victim == g))
}

/// Each update, on the open world's maps, for each champion: home sees
/// them off at their time; their city watches over their stay, keeps them
/// in it, and sees them home.
pub fn update(map: &mut Map) {
    if map.key.instance != 0 {
        return;
    }
    for who in ALL {
        update_one(map, who);
    }
}

fn update_one(map: &mut Map, who: Champion) {
    let now = map.now;
    if let Some(d) = who.local(map).departing
        && d.at <= now
    {
        who.local(map).departing = None;
        gone_from_home(map, who, d);
    }
    if let Some((at, g)) = who.local(map).returning
        && at <= now
    {
        who.local(map).returning = None;
        crate::game::map::scripts::remove_for_good(map, g);
        crate::game::map::elites::gone_with(map, who);
        bring_home(map, who);
    }
    if let Some((g, until, words)) = who.local(map).flash.clone()
        && (until <= now || map.creature(g).is_none_or(|c| !c.observers.is_empty()))
    {
        who.local(map).flash = None;
        if map.creature(g).is_some_and(|c| c.alive()) {
            crate::game::map::spells::cast_triggered(map, g, TELEPORT_VISUAL, None);
            if let Some(words) = words {
                say(map, g, &words);
            }
        }
    }
    if who.local(map).look_at.is_some_and(|t| t > now) {
        return;
    }
    who.local(map).look_at = Some(now + LOOK_EVERY);
    let place = who.state(&map.realm).lock().place;
    if let Place::Away {
        map: key,
        copy,
        city,
    } = place
        && key == map.key
    {
        watch(map, who, copy, city);
    }
    if !home(map) || who.local(map).departing.is_some() {
        return;
    }
    match place {
        // Home, the call lost: they stay.
        Place::Leaving => {
            let mut s = who.state(&map.realm).lock();
            if s.called_at.is_some_and(|t| t.elapsed() > STALE) {
                s.place = Place::Home;
            }
        }
        // Home, after its actor started again while they were away: the
        // spawn back alive goes again.
        Place::Away { .. } | Place::Returning => {
            let kind = who.kind();
            for spawn in std::iter::once(kind.spawn).chain(kind.companion.map(|(_, s)| s)) {
                if let Some(g) = at_home(map, spawn)
                    && map.creature(g).is_some_and(|c| c.alive())
                {
                    crate::game::map::scripts::despawn_with(map, g, Some(NEVER));
                }
            }
        }
        Place::Home => {}
    }
}

/// The city: they are kept in it, and at peace long enough, they go home.
fn watch(map: &mut Map, who: Champion, g: Guid, city: u32) {
    let now = map.now;
    if map.creature(g).is_none() {
        // Gone some other way: home, their company left as guards.
        who.state(&map.realm).lock().place = Place::Returning;
        crate::game::map::elites::released(map, who);
        bring_home(map, who);
        return;
    }
    if map.creature(g).is_some_and(|c| !c.alive()) {
        return;
    }
    if who.local(map).coming.is_some() {
        coming_in(map, who, g);
        if who.local(map).coming.is_some() {
            return;
        }
    }
    crate::game::map::elites::guard(map, who, g);
    // Fighting beyond the city, they go back to where they came.
    let fighting = map.creature(g).is_some_and(|c| c.combat.attacking);
    if fighting {
        let at = map.creature(g).map(|c| c.position);
        let place = at.and_then(|at| crate::game::map::world::zone_and_area(map, at));
        if place.is_some_and(|(zone, area)| zone != city && area != city) {
            crate::game::map::ai::enter_evade(map, g);
        }
    }
    if !calm(map, g) {
        who.local(map).calm_since = None;
        return;
    }
    let since = *who.local(map).calm_since.get_or_insert(now);
    let stays = Duration::from_secs(who.settings(&map.cfg.guards).stays_secs);
    if now.duration_since(since) < stays || who.local(map).returning.is_some() {
        return;
    }
    say(map, g, &line(who.kind().going, city));
    let takes = match who.kind().way {
        Way::Teleport => {
            crate::game::map::spells::cast_triggered(map, g, TELEPORT_VISUAL, None);
            crate::game::map::elites::flash_with(map, who);
            VANISH
        }
        Way::Run => {
            let (yards, mult) = (
                map.cfg.guards.rexxar_run_yards,
                map.cfg.guards.rexxar_run_speed,
            );
            let company = crate::game::map::elites::company(map, who);
            run_off(map, g, yards, mult, company)
        }
    };
    who.local(map).returning = Some((now + takes, g));
    who.state(&map.realm).lock().place = Place::Returning;
    info!(
        "{}: {} goes home to {}",
        map.key,
        who.name(),
        who.kind().home
    );
}
