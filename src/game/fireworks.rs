//! The Lunar Festival's rockets, as vmangos's `npc_pats_firework_guy` (and
//! Thistle Tea's `FireworkGuy`) sends them up: a rocket or a rocket
//! cluster fired at a launcher calls up its firework guy (a wild summon of
//! the rocket's spell), which, as it appears, sends up its firework
//! there: a rocket three yards above it; a cluster's five fanned around
//! and over it, a large cluster's wider and higher. Whoever fired it earns
//! the rocket's or the cluster's credit marker toward Lunar Fireworks
//! (8867), and a Lucky Lunar Rocket's cluster gives Lunar Fortune three
//! seconds after it bursts.
//!
//! The client's spell data leaves the small red, green, purple, white and
//! yellow clusters with no rockets of their own, so vmangos sends nothing
//! up for them; as Thistle Tea has it, each sends up its colour's rocket,
//! as the blue cluster does.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::guid;
use crate::game::map::Map;

pub const ROCKET_CREDIT: u32 = 15_893;
pub const CLUSTER_CREDIT: u32 = 15_894;
pub const LUNAR_FORTUNE: u32 = 26_522;
pub const FORTUNE_DELAY: Duration = Duration::from_secs(3);
/// How long a rocket's object stays to burst.
pub const LIFETIME: Duration = Duration::from_secs(5);
/// The Lucky Lunar Rocket's guy.
pub const LUCKY: u32 = 15_918;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rocket,
    LargeRocket,
    Cluster,
    LargeCluster,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Colour {
    Blue,
    Green,
    Purple,
    Red,
    White,
    Yellow,
}

/// Each colour's rocket objects, small and large.
fn rocket(kind: Kind, colour: Colour) -> u32 {
    let (small, large) = match colour {
        Colour::Blue => (180_854, 180_861),
        Colour::Green => (180_855, 180_862),
        Colour::Purple => (180_856, 180_863),
        Colour::Red => (180_851, 180_860),
        Colour::White => (180_857, 180_864),
        Colour::Yellow => (180_858, 180_865),
    };
    match kind {
        Kind::Rocket | Kind::Cluster => small,
        Kind::LargeRocket | Kind::LargeCluster => large,
    }
}

/// Where a firework's rockets burst, about its guy.
fn offsets(kind: Kind) -> &'static [(f32, f32, f32)] {
    match kind {
        Kind::Rocket | Kind::LargeRocket => &[(0.0, 0.0, 3.0)],
        Kind::Cluster => &[
            (0.0, 0.0, 8.0),
            (3.5, -1.0, 5.0),
            (0.0, 2.0, 5.0),
            (0.0, 0.0, 2.0),
            (-3.5, -1.0, 5.0),
        ],
        Kind::LargeCluster => &[
            (0.0, 0.0, 3.0),
            (0.0, 3.0, 7.5),
            (5.25, -1.5, 7.5),
            (-5.25, -1.5, 7.5),
            (0.0, 0.0, 12.0),
        ],
    }
}

/// The firework a guy of `entry` sends up, and its colours (one for each
/// rocket, the Lucky Lunar Rocket's mixed).
fn guy(entry: u32) -> Option<(Kind, [Colour; 5])> {
    use Colour::*;
    use Kind::*;
    let (kind, colour) = match entry {
        15_872 => (Cluster, Blue),
        15_873 => (Cluster, Red),
        15_874 => (Cluster, Green),
        15_875 => (Cluster, Purple),
        15_876 => (Cluster, White),
        15_877 => (Cluster, Yellow),
        15_879 => (Rocket, Blue),
        15_880 => (Rocket, Green),
        15_881 => (Rocket, Purple),
        15_882 => (Rocket, Red),
        15_883 => (Rocket, Yellow),
        15_884 => (Rocket, White),
        15_885 => (LargeRocket, Blue),
        15_886 => (LargeRocket, Green),
        15_887 => (LargeRocket, Purple),
        15_888 => (LargeRocket, Red),
        15_889 => (LargeRocket, White),
        15_890 => (LargeRocket, Yellow),
        15_911 => (LargeCluster, Blue),
        15_912 => (LargeCluster, Green),
        15_913 => (LargeCluster, Purple),
        15_914 => (LargeCluster, Red),
        15_915 => (LargeCluster, White),
        15_916 => (LargeCluster, Yellow),
        LUCKY => return Some((LargeCluster, [Blue, White, White, Blue, Blue])),
        _ => return None,
    };
    Some((kind, [colour; 5]))
}

/// The rockets a guy of `entry` at `at` sends up: each object and where it
/// bursts. None for any other creature.
pub fn rockets(entry: u32, at: Vector3d) -> Option<Vec<(u32, Vector3d)>> {
    let (kind, colours) = guy(entry)?;
    Some(
        offsets(kind)
            .iter()
            .zip(colours)
            .map(|((dx, dy, dz), c)| {
                (
                    rocket(kind, c),
                    Vector3d {
                        x: at.x + dx,
                        y: at.y + dy,
                        z: at.z + dz,
                    },
                )
            })
            .collect(),
    )
}

/// A creature came into the world: a firework guy sends its firework up,
/// its credit to the player who fired it, and Lunar Fortune to come for a
/// lucky one.
pub fn spawned(map: &mut Map, g: Guid) {
    let Some((entry, at)) = map.creature(g).map(|c| (c.entry, c.position)) else {
        return;
    };
    let Some(rockets) = rockets(entry, at) else {
        return;
    };
    for (object, at) in rockets {
        crate::game::map::objects::summon(
            map,
            None,
            object,
            at,
            0.0,
            LIFETIME.as_millis() as u32,
            0,
            0,
            None,
        );
    }
    let cluster = matches!(guy(entry), Some((Kind::Cluster | Kind::LargeCluster, _)));
    let credit = if cluster {
        CLUSTER_CREDIT
    } else {
        ROCKET_CREDIT
    };
    let fired_by = crate::game::map::event_ai::scripted(map, g).and_then(|s| s.summoner);
    if let Some(p) = fired_by.filter(|p| guid::is_player(*p)) {
        crate::game::map::quests::killed(map, p, guid::unit(credit, 0));
    }
    if entry == LUCKY {
        let when = map.now + FORTUNE_DELAY;
        map.objects
            .pending_spells
            .push((when, Guid::zero(), LUNAR_FORTUNE, g));
    }
}
