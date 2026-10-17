//! A first login's intro cinematic, whose camera the server follows so the
//! creatures it flies past are sent to the client, as vmangos and Thistle
//! Tea's `Core.Player.Cinematic` do: the camera sits on the latest waypoint
//! it has passed (the first before then), the view goes back to the player
//! while the camera is in line with them, and a camera the client never
//! reports done stops a few seconds after its last waypoint.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::map::Map;
use crate::game::map::player::Player;

/// How long after its last waypoint a camera stops by itself.
const ENDS_AFTER: Duration = Duration::from_secs(5);
/// Within this (squared, on either axis) of the player, the camera is in
/// line with them.
const IN_LINE_SQUARED: f32 = 20.0;

#[derive(Debug, Clone)]
pub struct Camera {
    pub started: Instant,
    pub start: Vector3d,
    /// (milliseconds, where).
    pub waypoints: Vec<(u32, Vector3d)>,
}

impl Camera {
    fn end(&self) -> Duration {
        Duration::from_millis(u64::from(self.waypoints.iter().map(|w| w.0).max().unwrap_or(0)))
            + ENDS_AFTER
    }

    /// Where the camera looks from at `now`: `None` when it is over,
    /// `Some(None)` when in line with the player.
    pub fn view(&self, now: Instant) -> Option<Option<Vector3d>> {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed > self.end() {
            return None;
        }
        let ms = elapsed.as_millis() as u32;
        let at = self
            .waypoints
            .iter()
            .filter(|(t, _)| *t > 0 && *t < ms)
            .max_by_key(|(t, _)| *t)
            .or_else(|| self.waypoints.iter().find(|(t, _)| *t == 0))
            .map(|(_, at)| *at);
        Some(at.filter(|a| {
            (a.x - self.start.x).powi(2) > IN_LINE_SQUARED
                && (a.y - self.start.y).powi(2) > IN_LINE_SQUARED
        }))
    }
}

/// Where a player sees from: the camera's view in a cinematic, else where
/// they stand.
pub fn view_point(p: &Player) -> Vector3d {
    p.misc.view.unwrap_or_else(|| p.position())
}

/// The intro starts: its camera is followed if the data has its path.
pub fn begin(map: &Map, p: &mut Player, sequence: u32) {
    let Some(points) = map.realm.data.misc.cinematic_waypoints.get(&sequence) else {
        return;
    };
    p.misc.camera = Some(Camera {
        started: map.now,
        start: p.position(),
        waypoints: points
            .iter()
            .map(|(t, x, y, z)| (*t, Vector3d { x: *x, y: *y, z: *z }))
            .collect(),
    });
}

/// The cameras move on; a moved view is seen from.
pub fn update(map: &mut Map) {
    let now = map.now;
    let mut moved = Vec::new();
    for p in map.players.values_mut() {
        let Some(c) = &p.misc.camera else {
            continue;
        };
        let view = match c.view(now) {
            Some(v) => v,
            None => {
                p.misc.camera = None;
                None
            }
        };
        if view != p.misc.view {
            p.misc.view = view;
            moved.push(p.guid);
        }
    }
    for g in moved {
        map.visibility_dirty.insert(g);
    }
}

/// `CMSG_COMPLETE_CINEMATIC`: back to the player.
pub fn finish(map: &mut Map, g: Guid) {
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let had = p.misc.view.is_some();
    p.misc.camera = None;
    p.misc.view = None;
    if had {
        map.visibility_dirty.insert(g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_sits_on_the_last_waypoint_passed_and_ends() {
        let t0 = Instant::now();
        let at = |x: f32| Vector3d { x, y: x, z: 0.0 };
        let c = Camera {
            started: t0,
            start: at(0.0),
            waypoints: vec![(0, at(100.0)), (1000, at(100.0)), (30_000, at(200.0)), (40_000, at(1.0))],
        };
        assert_eq!(c.view(t0), Some(Some(at(100.0))));
        assert_eq!(c.view(t0 + Duration::from_secs(31)), Some(Some(at(200.0))));
        // Back in line with the player.
        assert_eq!(c.view(t0 + Duration::from_secs(41)), Some(None));
        assert_eq!(c.view(t0 + Duration::from_secs(46)), None);
    }
}
