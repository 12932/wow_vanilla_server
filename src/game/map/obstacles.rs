//! The game objects that stand in the way: a closed door, a portcullis, a
//! cage, a crate. mangos-classic keeps them in its map's dynamic tree
//! (`GameObjectModel`), which its hit test and heights take beside the
//! VMaps: each spawned object whose display has a collision model, placed
//! at its position, turned by its facing about the vertical and scaled by
//! its size; a door only while it is closed (`GO_STATE_READY`), a trap
//! never (`GameObject::IsCollisionEnabled`). The models are the bake's
//! (`models/`, rustigator's `build_bvh`: one per display of
//! `GameObjectDisplayInfo.dbc` with collision triangles).
//!
//! The models a map's spawns show are read as it starts (`world::populate`),
//! once a process, and shared; an object a script brings in later with a
//! display none of them has stands in no one's way.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ahash::AHashMap;
use parking_lot::Mutex;
use tracing::{debug, warn};
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::{GAMEOBJECT_DISPLAYID, GAMEOBJECT_STATE, OBJECT_FIELD_SCALE_X};
use crate::game::map::Map;
use crate::game::map::objects::GO_STATE_READY;

/// `GAMEOBJECT_TYPE_DOOR`, `_TRAP`, `_TRANSPORT` and `_MO_TRANSPORT`.
const DOOR: u8 = 0;
const TRAP: u8 = 6;
const TRANSPORT: u8 = 11;
const MO_TRANSPORT: u8 = 15;
/// How far from a way an object's place may be and its model still reach
/// it, beyond the way's own length: the largest models a spawn shows (a
/// gate, a ship's hull) are some tens of yards across.
const REACH: f32 = 60.0;
/// A parallel ray's determinant, for the triangle test.
const EPS: f32 = 1e-6;

/// A collision model, in its own space: x ahead of its facing, z up.
#[derive(Debug)]
pub struct Model {
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    low: [f32; 3],
    high: [f32; 3],
    /// How far across its place it reaches, at its own size.
    reach: f32,
}

impl Model {
    fn new(mesh: rustigator_pathfind::format::CollisionMesh) -> Option<Self> {
        let n = mesh.vertices.len() as u32;
        let triangles: Vec<[u32; 3]> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .copied()
            .filter(|t| t.iter().all(|&i| i < n) && t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .collect();
        if triangles.is_empty() {
            return None;
        }
        let mut low = [f32::MAX; 3];
        let mut high = [f32::MIN; 3];
        for v in &mesh.vertices {
            for k in 0..3 {
                low[k] = low[k].min(v[k]);
                high[k] = high[k].max(v[k]);
            }
        }
        let reach = mesh
            .vertices
            .iter()
            .map(|v| v[0].hypot(v[1]))
            .fold(0.0f32, f32::max);
        Some(Self {
            vertices: mesh.vertices,
            triangles,
            low,
            high,
            reach,
        })
    }

    /// The least `t` in `[0, 1]` at which the segment `a + t (b - a)`
    /// meets one of its triangles, from either face (mangos's
    /// `IntersectTriangle`, Möller and Trumbore's); only those that face up
    /// if `up` (a floor's, as rustigator's heights take them).
    fn hit(&self, a: [f32; 3], b: [f32; 3], up: bool) -> Option<f32> {
        let d = sub(b, a);
        if !slab(a, d, self.low, self.high) {
            return None;
        }
        let mut best: Option<f32> = None;
        for t in &self.triangles {
            let [p0, p1, p2] = t.map(|i| self.vertices[i as usize]);
            let (e1, e2) = (sub(p1, p0), sub(p2, p0));
            if up && cross(e1, e2)[2] <= 0.0 {
                continue;
            }
            let p = cross(d, e2);
            let det = dot(e1, p);
            if det.abs() < EPS {
                continue;
            }
            let inv = 1.0 / det;
            let s = sub(a, p0);
            let u = dot(s, p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = cross(s, e1);
            let v = dot(d, q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let at = dot(e2, q) * inv;
            if (0.0..=1.0).contains(&at) && best.is_none_or(|b| at < b) {
                best = Some(at);
            }
        }
        best
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Whether the segment from `a` along `d` (`t` in `[0, 1]`) meets the box.
fn slab(a: [f32; 3], d: [f32; 3], low: [f32; 3], high: [f32; 3]) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for k in 0..3 {
        if d[k].abs() < EPS {
            if a[k] < low[k] || a[k] > high[k] {
                return false;
            }
            continue;
        }
        let (mut near, mut far) = ((low[k] - a[k]) / d[k], (high[k] - a[k]) / d[k]);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// How far `p` is from the segment `a` to `b`, over the ground.
fn from_segment(p: Vector3d, a: Vector3d, b: Vector3d) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.x - (a.x + t * dx)).hypot(p.y - (a.y + t * dy))
}

/// The bake's models, by display, read once a process for every map.
#[derive(Default)]
struct Cache {
    /// Each bake's index: display to its model's file under `models/`.
    index: AHashMap<PathBuf, AHashMap<u32, String>>,
    models: AHashMap<(PathBuf, u32), Option<Arc<Model>>>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The models of `displays` in the bake at `dir`, read now if they weren't
/// (a map's start): those that have one.
pub fn read(dir: &Path, displays: impl IntoIterator<Item = u32>) -> AHashMap<u32, Arc<Model>> {
    let mut c = cache().lock();
    if !c.index.contains_key(dir) {
        let index = match rustigator_mapbuild::bvh_constructor::load_index(dir) {
            Ok(i) => i
                .entries
                .into_iter()
                .map(|e| (e.display_id, e.path))
                .collect(),
            Err(e) => {
                warn!(
                    "no game object models in {}: {e}; doors stand in no one's way",
                    dir.display()
                );
                AHashMap::new()
            }
        };
        c.index.insert(dir.to_owned(), index);
    }
    let mut out = AHashMap::new();
    for display in displays {
        let key = (dir.to_owned(), display);
        if !c.models.contains_key(&key) {
            let model = c.index[dir].get(&display).and_then(|file| {
                let path = dir.join("models").join(file);
                let mesh = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| {
                        rustigator_pathfind::format::CollisionMesh::from_bytes(&b)
                            .map_err(|e| e.to_string())
                    });
                match mesh {
                    Ok(m) => Model::new(m).map(Arc::new),
                    Err(e) => {
                        debug!("game object model {}: {e}", path.display());
                        None
                    }
                }
            });
            c.models.insert(key.clone(), model);
        }
        if let Some(m) = &c.models[&key] {
            out.insert(display, m.clone());
        }
    }
    out
}

/// An object's model where it stands.
#[derive(Debug, Clone)]
struct Placed {
    model: Arc<Model>,
    at: [f32; 3],
    cos: f32,
    sin: f32,
    scale: f32,
}

impl Placed {
    /// A point of the world in the model's space.
    fn local(&self, p: [f32; 3]) -> [f32; 3] {
        let (dx, dy, dz) = (p[0] - self.at[0], p[1] - self.at[1], p[2] - self.at[2]);
        let s = 1.0 / self.scale;
        [
            (dx * self.cos + dy * self.sin) * s,
            (-dx * self.sin + dy * self.cos) * s,
            dz * s,
        ]
    }
}

/// The objects in the way of what goes about one place: those whose
/// models stand in the way now.
#[derive(Debug, Clone, Default)]
pub struct Obstacles {
    placed: Vec<Placed>,
}

impl Obstacles {
    /// The objects of `map` whose models may reach within `radius` of
    /// `at`, as they stand now.
    pub fn near(map: &Map, at: Vector3d, radius: f32) -> Self {
        Self::along(map, at, at, radius)
    }

    /// The objects of `map` whose models reach within `margin` of the way
    /// from `a` to `b` (over the ground), as they stand now.
    pub fn along(map: &Map, a: Vector3d, b: Vector3d, margin: f32) -> Self {
        let Some(models) = map.nav.as_ref().map(|n| &n.obstacle_models) else {
            return Self::default();
        };
        if models.is_empty() {
            return Self::default();
        }
        let (mx, my) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let half = (b.x - a.x).hypot(b.y - a.y) / 2.0;
        let placed = map
            .grid
            .around(mx, my, half + margin + REACH)
            .filter_map(|g| map.gameobject(g))
            .filter(|o| {
                let closed = o.fields.get_u32(GAMEOBJECT_STATE) == GO_STATE_READY;
                !matches!(o.kind, TRAP | TRANSPORT | MO_TRANSPORT) && (o.kind != DOOR || closed)
            })
            .filter_map(|o| {
                let model = models.get(&o.fields.get_u32(GAMEOBJECT_DISPLAYID))?;
                let scale = o.fields.get_f32(OBJECT_FIELD_SCALE_X);
                let scale = if scale.is_finite() && scale > 0.0 {
                    scale
                } else {
                    1.0
                };
                let p = o.position;
                (from_segment(p, a, b) <= model.reach * scale + margin).then(|| Placed {
                    model: model.clone(),
                    at: [p.x, p.y, p.z],
                    cos: o.orientation.cos(),
                    sin: o.orientation.sin(),
                    scale,
                })
            })
            .collect();
        Self { placed }
    }

    /// Whether any of them stands across the segment from `a` to `b`.
    pub fn crossed(&self, a: Vector3d, b: Vector3d) -> bool {
        let (a, b) = ([a.x, a.y, a.z], [b.x, b.y, b.z]);
        self.placed
            .iter()
            .any(|p| p.model.hit(p.local(a), p.local(b), false).is_some())
    }

    /// The upward faces of their models over `(x, y)` between `low` and
    /// `high` (a closed cage's roof, a crate's lid), into `out`.
    pub fn surfaces(&self, x: f32, y: f32, low: f32, high: f32, out: &mut Vec<f32>) {
        for p in &self.placed {
            let (a, b) = (p.local([x, y, high]), p.local([x, y, low]));
            // A ray straight down, each upward face it meets one, the next
            // looked for under the last.
            let mut from = a;
            for _ in 0..8 {
                let Some(t) = p.model.hit(from, b, true) else {
                    break;
                };
                let z_local = from[2] + (b[2] - from[2]) * t;
                let z = p.at[2] + z_local * p.scale;
                if !out.iter().any(|h| (h - z).abs() < 0.01) {
                    out.push(z);
                }
                from = [from[0], from[1], z_local - 0.01 / p.scale];
                if from[2] <= b[2] {
                    break;
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube's six faces, two triangles each, at the origin.
    fn cube() -> Model {
        let v = |x: f32, y: f32, z: f32| [x, y, z];
        let vertices = vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(1.0, 1.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 1.0),
            v(1.0, 0.0, 1.0),
            v(1.0, 1.0, 1.0),
            v(0.0, 1.0, 1.0),
        ];
        // The floor faces down, the lid up.
        let indices = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 5, 0, 5, 1, 1, 5, 6, 1, 6, 2, 2, 6, 7, 2, 7,
            3, 3, 7, 4, 3, 4, 0,
        ];
        Model::new(rustigator_pathfind::format::CollisionMesh {
            vertices,
            indices,
            ..Default::default()
        })
        .unwrap()
    }

    /// A segment across a model meets it from either side; one past it, or
    /// short of it, doesn't; one from inside it meets its face.
    #[test]
    fn a_segment_meets_a_model_from_either_side() {
        let c = cube();
        let hit = |a, b| c.hit(a, b, false);
        assert!(hit([-1.0, 0.5, 0.5], [2.0, 0.5, 0.5]).is_some(), "across");
        assert!(
            hit([2.0, 0.5, 0.5], [-1.0, 0.5, 0.5]).is_some(),
            "back across"
        );
        assert!(
            hit([-1.0, 0.5, 0.5], [-0.1, 0.5, 0.5]).is_none(),
            "short of it"
        );
        assert!(
            hit([-1.0, 2.5, 0.5], [2.0, 2.5, 0.5]).is_none(),
            "beside it"
        );
        let t = hit([0.5, 0.5, 0.5], [0.5, 0.5, 3.0]).unwrap();
        assert!((t - 0.2).abs() < 1e-4, "from inside, at its lid: {t}");
        // Down from over it: its lid faces up, its floor down.
        let t = c.hit([0.5, 0.5, 3.0], [0.5, 0.5, -1.0], true).unwrap();
        assert!((t - 0.5).abs() < 1e-4, "its lid: {t}");
        assert!(
            c.hit([0.5, 0.5, 0.5], [0.5, 0.5, -1.0], true).is_none(),
            "not its floor"
        );
    }

    /// A placed model is turned by its facing and scaled by its size: a
    /// cube turned a quarter round from its place at (10, 20) stands to its
    /// west (+y) and south (-x), and twice its size reaches two yards; its
    /// one surface is its lid.
    #[test]
    fn a_placed_model_is_turned_and_scaled() {
        let o = std::f32::consts::FRAC_PI_2;
        let p = Placed {
            model: Arc::new(cube()),
            at: [10.0, 20.0, 5.0],
            cos: o.cos(),
            sin: o.sin(),
            scale: 2.0,
        };
        let obstacles = Obstacles { placed: vec![p] };
        let at = |x: f32, y: f32, z: f32| Vector3d { x, y, z };
        // Turned: its x is the world's +y, its y the world's -x, so it
        // spans x 8 to 10, y 20 to 22, z 5 to 7.
        assert!(
            obstacles.crossed(at(7.0, 21.0, 6.0), at(11.0, 21.0, 6.0)),
            "across it"
        );
        assert!(
            obstacles.crossed(at(9.0, 19.0, 6.0), at(9.0, 23.0, 6.0)),
            "and along"
        );
        assert!(
            !obstacles.crossed(at(11.0, 19.0, 6.0), at(11.0, 23.0, 6.0)),
            "north of it"
        );
        assert!(
            !obstacles.crossed(at(9.0, 19.0, 7.5), at(9.0, 23.0, 7.5)),
            "over it"
        );
        let mut out = Vec::new();
        obstacles.surfaces(9.0, 21.0, 0.0, 20.0, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!((out[0] - 7.0).abs() < 0.02, "its lid: {out:?}");
    }
}
