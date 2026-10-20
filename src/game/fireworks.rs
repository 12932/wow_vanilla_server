//! The Lunar Festival's firework launchers (vmangos's
//! `npc_pats_firework_guy`, Thistle Tea's `FireworkGuy`). A rocket or a
//! rocket cluster, used at a launcher, calls up its firework guy there
//! (spell effect 41 at target 46, the launcher its script names); the guy
//! sends the firework up as he appears: a rocket three yards over him, a
//! cluster's five fanned around and over him, a large cluster's wider and
//! higher. Whoever fired it earns the rocket or cluster credit toward the
//! festival's firework quest, a Lucky Rocket Cluster brings Lunar Fortune
//! three seconds after it bursts, and one sent up by Omen's cluster
//! launcher by Elune's lake counts toward calling him out of it.
//!
//! The client's data leaves the small red, green, purple, white and yellow
//! cluster rockets' summons in their second effect, which vmangos's guy
//! never sends up; as Thistle Tea, each sends its color's rocket, as the
//! blue cluster does.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;

/// The quest credit markers a rocket and a cluster earn (quest 8867,
/// Lunar Fireworks: eight rockets and two clusters).
pub const ROCKET_CREDIT: u32 = 15_893;
pub const CLUSTER_CREDIT: u32 = 15_894;
/// The Lucky Rocket Cluster's guy, and what it brings three seconds after.
pub const LUCKY: u32 = 15_918;
pub const LUNAR_FORTUNE: u32 = 26_522;
const FORTUNE_DELAY: Duration = Duration::from_secs(3);
/// How near Omen's launcher a firework counts toward him (mangos's
/// `INTERACTION_DISTANCE`).
const LAUNCHER_REACH: f32 = 5.0;
/// A rocket goes off at once: it bursts as it despawns, on the next
/// update after its create (vmangos's `go_firework_rocket`, from sniffs;
/// Thistle Tea keeps it five seconds).
const ROCKET_LIFETIME_MS: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Rocket,
    LargeRocket,
    Cluster,
    LargeCluster,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Color {
    Blue,
    Green,
    Purple,
    Red,
    White,
    Yellow,
}
use Color::*;

/// A color's rocket and its large one.
fn rockets(c: Color) -> (u32, u32) {
    match c {
        Blue => (180_854, 180_861),
        Green => (180_855, 180_862),
        Purple => (180_856, 180_863),
        Red => (180_851, 180_860),
        White => (180_857, 180_864),
        Yellow => (180_858, 180_865),
    }
}

/// What each guy sends up: its kind and its five rockets' colors (one for
/// a rocket).
fn guy(entry: u32) -> Option<(Kind, [Color; 5])> {
    use Kind::*;
    let all = |c| [c; 5];
    Some(match entry {
        15_872 => (Cluster, all(Blue)),
        15_873 => (Cluster, all(Red)),
        15_874 => (Cluster, all(Green)),
        15_875 => (Cluster, all(Purple)),
        15_876 => (Cluster, all(White)),
        15_877 => (Cluster, all(Yellow)),
        15_879 => (Rocket, all(Blue)),
        15_880 => (Rocket, all(Green)),
        15_881 => (Rocket, all(Purple)),
        15_882 => (Rocket, all(Red)),
        15_883 => (Rocket, all(Yellow)),
        15_884 => (Rocket, all(White)),
        15_885 => (LargeRocket, all(Blue)),
        15_886 => (LargeRocket, all(Green)),
        15_887 => (LargeRocket, all(Purple)),
        15_888 => (LargeRocket, all(Red)),
        15_889 => (LargeRocket, all(White)),
        15_890 => (LargeRocket, all(Yellow)),
        15_911 => (LargeCluster, all(Blue)),
        15_912 => (LargeCluster, all(Green)),
        15_913 => (LargeCluster, all(Purple)),
        15_914 => (LargeCluster, all(Red)),
        15_915 => (LargeCluster, all(White)),
        15_916 => (LargeCluster, all(Yellow)),
        LUCKY => (LargeCluster, [Blue, White, White, Blue, Blue]),
        _ => return None,
    })
}

/// Where a kind's rockets burst, about its guy.
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

/// Whether `entry` is a firework guy.
pub fn is_guy(entry: u32) -> bool {
    guy(entry).is_some()
}

/// A guy's launch: each rocket and where it bursts, the credit its firer
/// earns, and whether it brings Lunar Fortune.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub fireworks: Vec<(u32, Vector3d)>,
    pub credit: u32,
    pub lucky: bool,
}

/// What guy `entry` at `at` sends up; nothing for any other creature.
pub fn launch(entry: u32, at: Vector3d) -> Option<Launch> {
    let (kind, colors) = guy(entry)?;
    let large = matches!(kind, Kind::LargeRocket | Kind::LargeCluster);
    let fireworks = offsets(kind)
        .iter()
        .zip(colors)
        .map(|((dx, dy, dz), c)| {
            let (small, big) = rockets(c);
            let at = Vector3d {
                x: at.x + dx,
                y: at.y + dy,
                z: at.z + dz,
            };
            (if large { big } else { small }, at)
        })
        .collect();
    let cluster = matches!(kind, Kind::Cluster | Kind::LargeCluster);
    Some(Launch {
        fireworks,
        credit: if cluster {
            CLUSTER_CREDIT
        } else {
            ROCKET_CREDIT
        },
        lucky: entry == LUCKY,
    })
}

/// A creature came into the world: a firework guy sends his firework up.
pub fn spawned(map: &mut Map, g: Guid) {
    let Some((entry, at, summoner)) = map.creature(g).map(|c| {
        (
            c.entry,
            c.position,
            c.scripted.as_deref().and_then(|s| s.summoner),
        )
    }) else {
        return;
    };
    let Some(l) = launch(entry, at) else {
        return;
    };
    for (rocket, burst) in &l.fireworks {
        crate::game::map::objects::summon(
            map,
            None,
            *rocket,
            *burst,
            0.0,
            ROCKET_LIFETIME_MS,
            0,
            0,
            None,
        );
    }
    if let Some(p) = summoner.filter(|s| guid::is_player(*s)) {
        crate::game::map::quests::killed(map, p, guid::unit(l.credit, 0));
    }
    if l.lucky {
        let when = map.now + FORTUNE_DELAY;
        map.objects.pending_spells.push((when, g, LUNAR_FORTUNE, g));
    }
    // Omen's launcher, the nearest within reach, hears of it.
    let launcher = map
        .grid
        .around(at.x, at.y, LAUNCHER_REACH)
        .filter(|o| guid::is_game_object(*o))
        .filter_map(|o| map.gameobject(o))
        .filter(|o| o.entry == crate::game::omen::LAUNCHER)
        .map(|o| (o.guid, dist(o.position, at)))
        .filter(|(_, d)| *d <= LAUNCHER_REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((o, _)) = launcher {
        crate::game::omen::use_object(map, o);
    }
}
