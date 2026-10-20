//! The Ghost Magnet of Ghost-o-plasm Round Up (6134) in Desolace, as
//! vmangos's `go_ghost_magnetAI` runs it (and Thistle Tea's
//! `GameObject.GhostMagnet`): a magnet set down where no other magnet's
//! aura glows within thirty yards raises its own aura for two minutes, and
//! in those two minutes calls Magrami Spectres from up to forty yards out:
//! eight of them, the first after five seconds and the rest three to eight
//! seconds apart, and another twenty seconds after any of them dies. Each
//! makes the magnet its home and walks to it, to turn on those there (its
//! own script, `npc_magrami_spetre`).
//!
//! mangos-classic's world script calls one every fifteen seconds for three
//! minutes from a fixed angle; vmangos's is the one for the data wvs runs.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::map::Map;
use crate::game::map::combat::dist;

pub const MAGNET: u32 = 177_746;
pub const AURA: u32 = 177_749;
pub const SPECTRE: u32 = 11_560;
/// No magnet starts within this of another's aura.
const AURA_REACH: f32 = 30.0;
/// How long a magnet calls, and its aura glows.
pub const CALLING: Duration = Duration::from_secs(120);
pub const SPECTRES: u32 = 8;
pub const FIRST_CALL: Duration = Duration::from_secs(5);
const CALL_GAP_MS: std::ops::RangeInclusive<u64> = 3_000..=8_000;
pub const REPLACE_AFTER: Duration = Duration::from_secs(20);
/// A spectre's time, timed or dead (despawn type 1): out of a fight it
/// goes two minutes after it came.
pub const SPECTRE_LIFETIME: Duration = Duration::from_secs(120);
const TIMED_OR_DEAD: i32 = 1;
pub const CALL_RADIUS: f32 = 40.0;
/// A spectre's home is this short of the magnet, on its side.
pub const CONTACT: f32 = 1.5;
/// The point its arrival tells its script of.
pub const MAGNET_POINT: u32 = 2;

/// A calling magnet.
#[derive(Debug, Clone)]
pub struct Magnet {
    pub until: Instant,
    /// Spectres still to call.
    pub remaining: u32,
    pub next_call: Option<Instant>,
    /// When a slain spectre's replacement comes.
    pub replacements: Vec<Instant>,
}

impl Magnet {
    pub fn calling(&self, now: Instant) -> bool {
        now < self.until
    }
}

/// A magnet `g` was set down: unless another's aura glows near, it raises
/// its own and starts calling.
pub fn start(map: &mut Map, g: Guid) {
    let Some((entry, at, o)) = map
        .gameobject(g)
        .map(|o| (o.entry, o.position, o.orientation))
    else {
        return;
    };
    if entry != MAGNET {
        return;
    }
    let glowing = map
        .gameobjects
        .iter()
        .any(|(_, x)| x.entry == AURA && dist(x.position, at) <= AURA_REACH);
    if glowing {
        return;
    }
    let now = map.now;
    map.magnets.insert(
        g,
        Magnet {
            until: now + CALLING,
            remaining: SPECTRES,
            next_call: Some(now + FIRST_CALL),
            replacements: Vec::new(),
        },
    );
    crate::game::map::objects::summon(
        map,
        None,
        AURA,
        at,
        o,
        CALLING.as_millis() as u32,
        0,
        0,
        None,
    );
}

/// Each calling magnet calls the spectres due: the next of its eight, and
/// those that replace the slain. A magnet gone, or done calling, is
/// forgotten.
pub fn update(map: &mut Map) {
    if map.magnets.is_empty() {
        return;
    }
    let now = map.now;
    let gone: Vec<Guid> = map
        .magnets
        .iter()
        .filter(|(g, m)| map.gameobject(**g).is_none() || !m.calling(now))
        .map(|(g, _)| *g)
        .collect();
    for g in gone {
        map.magnets.remove(&g);
    }
    let magnets: Vec<Guid> = map.magnets.keys().copied().collect();
    for g in magnets {
        let mut calls = 0;
        if let Some(m) = map.magnets.get_mut(&g) {
            if m.remaining > 0 && m.next_call.is_some_and(|t| t <= now) {
                m.remaining -= 1;
                m.next_call = (m.remaining > 0)
                    .then(|| now + Duration::from_millis(rand::rng().random_range(CALL_GAP_MS)));
                calls += 1;
            }
            let before = m.replacements.len();
            m.replacements.retain(|t| *t > now);
            calls += before - m.replacements.len();
        }
        for _ in 0..calls {
            call(map, g);
        }
    }
}

/// A spectre a magnet called died: another comes twenty seconds on, if the
/// magnet still calls then.
pub fn creature_died(map: &mut Map, creature: Guid, entry: u32) {
    if entry != SPECTRE {
        return;
    }
    let Some(magnet) = crate::game::map::event_ai::scripted(map, creature).and_then(|s| s.summoner)
    else {
        return;
    };
    let now = map.now;
    if let Some(m) = map.magnets.get_mut(&magnet) {
        m.replacements.push(now + REPLACE_AFTER);
    }
}

/// A spectre called to magnet `g`, from a walkable spot up to forty yards
/// out (anywhere within them where there is no navmesh), made to take the
/// magnet's side for its home and walk there.
pub fn call(map: &mut Map, g: Guid) -> Option<Guid> {
    let at = map.gameobject(g)?.position;
    let from = map
        .nav
        .as_mut()
        .and_then(|n| n.random_point(at, CALL_RADIUS))
        .unwrap_or_else(|| {
            let mut rng = rand::rng();
            let angle = rng.random_range(0.0..std::f32::consts::TAU);
            let reach = rng.random_range(0.0..CALL_RADIUS);
            Vector3d {
                x: at.x + reach * angle.cos(),
                y: at.y + reach * angle.sin(),
                z: at.z,
            }
        });
    let spectre = crate::game::map::scripts::summon(
        map,
        g,
        SPECTRE,
        from,
        0.0,
        Some((SPECTRE_LIFETIME, TIMED_OR_DEAD)),
    )?;
    let home = contact_point(at, from);
    if let Some(c) = map.creature_mut(spectre) {
        c.home = home;
        c.home_orientation = 0.0;
    }
    crate::game::map::scripts::move_with(
        map,
        spectre,
        home,
        crate::game::map::scripts::Move {
            pathfind: true,
            point: Some(MAGNET_POINT),
            ..Default::default()
        },
    );
    Some(spectre)
}

/// The spot `CONTACT` yards short of `magnet` on `from`'s side; the
/// magnet's own for one nearer.
pub fn contact_point(magnet: Vector3d, from: Vector3d) -> Vector3d {
    let (dx, dy) = (from.x - magnet.x, from.y - magnet.y);
    let d = (dx * dx + dy * dy).sqrt();
    let f = if d > CONTACT { CONTACT / d } else { 0.0 };
    Vector3d {
        x: magnet.x + dx * f,
        y: magnet.y + dy * f,
        z: magnet.z,
    }
}
