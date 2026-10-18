//! A map's terrain tiles, as Thistle Tea's `World.Terrain` reads them:
//! MaNGOS z1.5 `.map` files for build 5875 (`MMMMRRCC.map`, one an ADT),
//! each cell's area, ground height, liquid and holes. The navmesh knows a
//! liquid's surface but not its kind outside WMOs; these give the kind
//! (water, ocean, magma, slime, the deep sea) and an area where the navmesh
//! has none. Tiles are read as they are first asked for (off the map's
//! thread on the server: `map/tiles.rs`), and only a wet tile keeps its
//! heights.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ahash::{AHashMap, AHashSet};
use std::collections::hash_map::Entry;
use tracing::{info, warn};
use wow_world_messages::vanilla::Vector3d;

/// An ADT's side, in yards.
const GRID_SIZE: f64 = 533.333_333_333_333_4;
const CORNERS: usize = 129 * 129;
const CENTERS: usize = 128 * 128;
const HEADER: usize = 44;

// MaNGOS's liquid flags.
pub const LIQUID_MAGMA: u16 = 0x01;
pub const LIQUID_OCEAN: u16 = 0x02;
pub const LIQUID_SLIME: u16 = 0x04;
pub const LIQUID_WATER: u16 = 0x08;
/// The deep sea, where swimmers tire.
pub const LIQUID_DEEP: u16 = 0x10;

/// Naxxramas's slime: the spell standing in it puts on.
pub const SPELL_NAXX_SLIME: u32 = 28801;

/// The liquid at a point: its flags, its surface and the floor under it,
/// and its `LiquidType.dbc` entry where a WMO holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Liquid {
    pub flags: u16,
    pub surface: f32,
    pub floor: f32,
    pub entry: Option<u8>,
}

impl Liquid {
    /// A WMO's liquid, by its entry: 1 water, 2 ocean, 3 magma, 4 slime,
    /// 21 Naxxramas slime.
    pub fn from_wmo(entry: u8, surface: f32, floor: f32) -> Option<Self> {
        let flags = match entry {
            1 => LIQUID_WATER,
            2 => LIQUID_OCEAN,
            3 => LIQUID_MAGMA,
            4 | 21 => LIQUID_SLIME,
            _ => return None,
        };
        Some(Self {
            flags,
            surface,
            floor,
            entry: Some(entry),
        })
    }

    pub fn deep(&self) -> bool {
        self.flags & LIQUID_DEEP != 0
    }

    pub fn magma(&self) -> bool {
        self.flags & LIQUID_MAGMA != 0
    }

    /// Tenths of a yard the surface is over `z`, as Thistle Tea counts them.
    fn tenths_under(&self, z: f32) -> f64 {
        ((self.surface as f64 - z as f64 - 0.01) * 10.0).trunc()
    }

    /// Whether someone at `z` touches it: in it, or within a tenth of a
    /// yard over it.
    pub fn touching(&self, z: f32) -> bool {
        self.tenths_under(z) > -1.0
    }

    /// The spell being in it at `z` puts on.
    pub fn spell(&self, z: f32) -> Option<u32> {
        (self.entry == Some(21) && self.tenths_under(z) > 0.0).then_some(SPELL_NAXX_SLIME)
    }
}

/// The tile `(x, y)` is in: (row, column).
pub fn coordinates(x: f32, y: f32) -> (i32, i32) {
    let at = |c: f32| (32.0 - c as f64 / GRID_SIZE).floor() as i32;
    (at(x), at(y))
}

/// The cell a coordinate is in, of the tile's 128, and how far across it.
fn cell(c: f32) -> (usize, f32) {
    let v = 128.0 * (32.0 - c as f64 / GRID_SIZE);
    let i = v.floor();
    ((i as i64 & 127) as usize, (v - i) as f32)
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn f32_at(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Whether every float in `b` is a number (Erlang's binaries take no NaN or
/// infinity, so Thistle Tea turns such a tile down).
fn finite_floats(b: &[u8]) -> bool {
    let (floats, rest) = b.as_chunks::<4>();
    rest.is_empty() && floats.iter().all(|c| f32::from_le_bytes(*c).is_finite())
}

#[derive(Debug)]
enum Area {
    None,
    One(u16),
    /// Each 8×8 block's area bit, 16×16 of them.
    Grid(Box<[u8]>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Encoding {
    Float,
    Flat,
    Int16,
    Int8,
}

#[derive(Debug)]
struct Height {
    encoding: Encoding,
    base: f32,
    multiplier: f32,
    data: Box<[u8]>,
}

impl Height {
    fn at(&self, i: usize) -> f32 {
        match self.encoding {
            Encoding::Float => f32_at(&self.data, i * 4),
            Encoding::Flat => self.base,
            Encoding::Int16 => self.base + u16_at(&self.data, i * 2) as f32 * self.multiplier,
            Encoding::Int8 => self.base + self.data[i] as f32 * self.multiplier,
        }
    }
}

#[derive(Debug)]
struct LiquidGrid {
    kind: u16,
    /// Each 8×8 block's flags, 16×16 of them; `None` for one kind all over.
    types: Option<Box<[u8]>>,
    /// The surface over the liquid's cells; empty for one level all over.
    heights: Box<[u8]>,
    offset_x: usize,
    offset_y: usize,
    width: usize,
    height: usize,
    level: f32,
}

/// One tile: a dry one keeps only its areas.
#[derive(Debug)]
pub struct Tile {
    area: Area,
    height: Option<Height>,
    liquid: Option<LiquidGrid>,
    holes: Box<[u8]>,
}

fn section(b: &[u8], offset: u32, size: u32) -> Option<&[u8]> {
    let (offset, size) = (offset as usize, size as usize);
    match (offset, size) {
        (0, 0) => Some(&[]),
        _ if offset >= HEADER && size > 0 && offset + size <= b.len() => {
            Some(&b[offset..offset + size])
        }
        _ => None,
    }
}

fn decode_area(d: &[u8]) -> Option<Area> {
    if d.is_empty() {
        return Some(Area::None);
    }
    if d.len() < 8 || &d[..4] != b"AREA" {
        return None;
    }
    match (u16_at(d, 4), d.len()) {
        (1, 8) => Some(Area::One(u16_at(d, 6))),
        (0, 520) => Some(Area::Grid(d[8..].into())),
        _ => None,
    }
}

fn decode_height(d: &[u8]) -> Option<Height> {
    if d.len() < 16 || &d[..4] != b"MHGT" {
        return None;
    }
    let (flags, base, max) = (u32_at(d, 4), f32_at(d, 8), f32_at(d, 12));
    if !base.is_finite() || !max.is_finite() || max < base {
        return None;
    }
    let (encoding, bytes, multiplier) = match flags {
        0 => (Encoding::Float, 4, 1.0),
        1 => (Encoding::Flat, 0, 0.0),
        2 => (Encoding::Int16, 2, (max - base) / 65_535.0),
        4 => (Encoding::Int8, 1, (max - base) / 255.0),
        _ => return None,
    };
    let data = &d[16..];
    if data.len() != bytes * (CORNERS + CENTERS)
        || (encoding == Encoding::Float && !finite_floats(data))
    {
        return None;
    }
    Some(Height {
        encoding,
        base,
        multiplier,
        data: data.into(),
    })
}

/// A tile's liquid: `Some(None)` for none, `None` if it doesn't decode.
fn decode_liquid(d: &[u8]) -> Option<Option<LiquidGrid>> {
    if d.is_empty() {
        return Some(None);
    }
    if d.len() < 16 || &d[..4] != b"MLIQ" {
        return None;
    }
    let (flags, kind) = (u16_at(d, 4), u16_at(d, 6));
    let (offset_x, offset_y, width, height) =
        (d[8] as usize, d[9] as usize, d[10] as usize, d[11] as usize);
    let level = f32_at(d, 12);
    if flags > 3
        || width == 0
        || height == 0
        || offset_x + width > 129
        || offset_y + height > 129
        || !level.is_finite()
    {
        return None;
    }
    let data = &d[16..];
    let type_size = if flags & 1 == 0 { 768 } else { 0 };
    let height_size = if flags & 2 == 0 {
        width * height * 4
    } else {
        0
    };
    if data.len() != type_size + height_size || !finite_floats(&data[type_size..]) {
        return None;
    }
    Some(Some(LiquidGrid {
        kind,
        types: (type_size != 0).then(|| data[512..768].into()),
        heights: data[type_size..].into(),
        offset_x,
        offset_y,
        width,
        height,
        level,
    }))
}

impl Tile {
    /// A tile from its file; `None` if it isn't one.
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER || &b[..8] != b"MAPSz1.5" || u32_at(b, 8) != 5875 {
            return None;
        }
        let at = |i: usize| (u32_at(b, 12 + i * 8), u32_at(b, 16 + i * 8));
        let ((area_offset, area_size), (height_offset, height_size)) = (at(0), at(1));
        let ((liquid_offset, liquid_size), (holes_offset, holes_size)) = (at(2), at(3));
        let liquid = decode_liquid(section(b, liquid_offset, liquid_size)?)?;
        let area = decode_area(section(b, area_offset, area_size)?)?;
        if liquid.is_none() {
            return Some(Self {
                area,
                height: None,
                liquid: None,
                holes: Box::default(),
            });
        }
        let height = decode_height(section(b, height_offset, height_size)?)?;
        let holes = section(b, holes_offset, holes_size)?;
        if !matches!(holes.len(), 0 | 512) {
            return None;
        }
        Some(Self {
            area,
            height: Some(height),
            liquid,
            holes: holes.into(),
        })
    }

    /// The liquid at a point: there if the cell has some, its surface is
    /// over the ground, and the point no more than two yards under it.
    pub fn sample(&self, p: Vector3d) -> Option<Liquid> {
        let (liquid, height) = (self.liquid.as_ref()?, self.height.as_ref()?);
        let (row, row_fraction) = cell(p.x);
        let (column, column_fraction) = cell(p.y);
        let flags = match &liquid.types {
            Some(types) => types[(row / 8) * 16 + column / 8] as u16,
            None => liquid.kind,
        };
        let surface = liquid.surface(row, column)?;
        let ground = self.ground(height, row, column, row_fraction, column_fraction);
        (flags != 0 && surface >= ground && p.z >= ground - 2.0).then_some(Liquid {
            flags,
            surface,
            floor: ground,
            entry: None,
        })
    }

    fn ground(&self, height: &Height, row: usize, column: usize, x: f32, y: f32) -> f32 {
        if height.encoding == Encoding::Flat {
            return height.base;
        }
        if self.hole(row, column) {
            return -100_000.0;
        }
        let h1 = height.at(row * 129 + column);
        let h2 = height.at((row + 1) * 129 + column);
        let h3 = height.at(row * 129 + column + 1);
        let h4 = height.at((row + 1) * 129 + column + 1);
        let center = 2.0 * height.at(CORNERS + row * 128 + column);
        if x + y < 1.0 && x > y {
            (h2 - h1) * x + (center - h1 - h2) * y + h1
        } else if x + y < 1.0 {
            (center - h1 - h3) * x + (h3 - h1) * y + h1
        } else if x > y {
            (h2 + h4 - center) * x + (h4 - h2) * y + center - h4
        } else {
            (h4 - h3) * x + (h3 + h4 - center) * y + center - h4
        }
    }

    fn hole(&self, row: usize, column: usize) -> bool {
        if self.holes.is_empty() {
            return false;
        }
        let bits = u16_at(&self.holes, ((row / 8) * 16 + column / 8) * 2);
        let bit = ((row % 8) / 2) * 4 + (column % 8) / 2;
        bits & (1 << bit) != 0
    }

    /// The area bit (`AreaTable.dbc`'s `area_bit`) of the cell at `(x, y)`.
    pub fn area_bit(&self, x: f32, y: f32) -> Option<u16> {
        match &self.area {
            Area::None => None,
            Area::One(id) => Some(*id),
            Area::Grid(areas) => {
                let ((row, _), (column, _)) = (cell(x), cell(y));
                Some(u16_at(areas, ((row / 8) * 16 + column / 8) * 2))
            }
        }
    }
}

impl LiquidGrid {
    fn surface(&self, row: usize, column: usize) -> Option<f32> {
        let r = row
            .checked_sub(self.offset_y)
            .filter(|&r| r < self.height)?;
        let c = column
            .checked_sub(self.offset_x)
            .filter(|&c| c < self.width)?;
        Some(if self.heights.is_empty() {
            self.level
        } else {
            f32_at(&self.heights, (r * self.width + c) * 4)
        })
    }
}

/// A map's tiles, read as asked for: at once, or off the map's thread
/// (`map/tiles.rs`), when one not in yet answers as no terrain would.
pub struct Terrain {
    dir: PathBuf,
    map: u32,
    /// The tiles the directory has for the map.
    present: AHashSet<(u8, u8)>,
    /// Those asked for, with when each was last used: loading (`None`), or
    /// read (`Some(None)` where one didn't decode).
    tiles: AHashMap<(u8, u8), Slot>,
    /// Where tiles read off the map's thread arrive; `None` to read them at
    /// once.
    background: Option<(kanal::Sender<Arrival>, kanal::Receiver<Arrival>)>,
    /// The map's time, as of its last update.
    now: Instant,
}

#[derive(Debug)]
struct Slot {
    tile: Option<Option<Tile>>,
    used: Instant,
}

type Arrival = ((u8, u8), Option<Tile>);

impl std::fmt::Debug for Terrain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Terrain({} of {} tiles read)",
            self.tiles_read(),
            self.present.len()
        )
    }
}

/// Tile `key` of map `map` in `dir`, read; `None` if it doesn't decode.
fn read(dir: &Path, map: u32, key: (u8, u8)) -> Option<Tile> {
    let _z = crate::zone!("terrain tile");
    let path = dir.join(format!("{map:04}{:02}{:02}.map", key.0, key.1));
    let tile = std::fs::read(&path).ok().and_then(|b| Tile::decode(&b));
    if tile.is_none() {
        warn!("invalid terrain tile {}", path.display());
    }
    tile
}

impl Terrain {
    /// The tiles in `dir` for map `map`; `None` if it has none. They are
    /// read at once as they are asked for.
    pub fn open(dir: &Path, map: u32) -> Option<Self> {
        let prefix = format!("{map:04}");
        let present: AHashSet<(u8, u8)> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| {
                let name = e.ok()?.file_name().into_string().ok()?;
                let digits = name.strip_suffix(".map")?.strip_prefix(&prefix)?;
                if digits.len() != 4 {
                    return None;
                }
                Some((digits[..2].parse().ok()?, digits[2..].parse().ok()?))
            })
            .collect();
        if present.is_empty() {
            return None;
        }
        info!(
            "{} terrain tiles for map {map} in {}",
            present.len(),
            dir.display()
        );
        Some(Self {
            dir: dir.to_owned(),
            map,
            present,
            tiles: AHashMap::new(),
            background: None,
            now: Instant::now(),
        })
    }

    /// From now its tiles are read off the map's thread, and taken in at
    /// [`Terrain::receive`].
    pub fn in_background(&mut self) {
        self.background = Some(kanal::unbounded());
    }

    fn key(x: f32, y: f32) -> Option<(u8, u8)> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (row, column) = coordinates(x, y);
        Some((u8::try_from(row).ok()?, u8::try_from(column).ok()?))
    }

    /// The tile `key`, read at once or asked for; `None` while it loads.
    fn slot(&mut self, key: (u8, u8)) -> Option<&Tile> {
        if !self.present.contains(&key) {
            return None;
        }
        let now = self.now;
        let (dir, map) = (&self.dir, self.map);
        let slot = match self.tiles.entry(key) {
            Entry::Occupied(e) => {
                let s = e.into_mut();
                s.used = now;
                s
            }
            Entry::Vacant(v) => {
                let tile = match &self.background {
                    Some((tx, _)) => {
                        let (tx, dir) = (tx.clone(), dir.clone());
                        crate::game::map::tiles::read(move || {
                            let _ = tx.send((key, read(&dir, map, key)));
                        });
                        None
                    }
                    None => Some(read(dir, map, key)),
                };
                v.insert(Slot { tile, used: now })
            }
        };
        slot.tile.as_ref()?.as_ref()
    }

    fn tile(&mut self, x: f32, y: f32) -> Option<&Tile> {
        let key = Self::key(x, y)?;
        self.slot(key)
    }

    /// The tile under the navmesh's tile `t` (`nav::tile_of`) is wanted
    /// soon: read in the background if it isn't, and kept. Reading at
    /// once, it is only kept.
    pub fn want(&mut self, (tx, ty): (i32, i32)) {
        let (Ok(row), Ok(column)) = (u8::try_from(ty), u8::try_from(tx)) else {
            return;
        };
        if self.background.is_some() {
            self.slot((row, column));
        } else if let Some(s) = self.tiles.get_mut(&(row, column)) {
            s.used = self.now;
        }
    }

    /// The map's update: its time, and the tiles read since the last.
    /// How many came in.
    pub fn receive(&mut self, now: Instant) -> usize {
        self.now = now;
        let Some((_, rx)) = &self.background else {
            return 0;
        };
        let arrived: Vec<Arrival> = std::iter::from_fn(|| rx.try_recv().ok().flatten()).collect();
        let n = arrived.len();
        for (key, tile) in arrived {
            // One let go of meanwhile is asked for again when needed.
            if let Some(s) = self.tiles.get_mut(&key)
                && s.tile.is_none()
            {
                s.tile = Some(tile);
            }
        }
        n
    }

    /// The terrain's liquid at a point.
    pub fn liquid(&mut self, p: Vector3d) -> Option<Liquid> {
        self.tile(p.x, p.y)?.sample(p)
    }

    /// The area bit of the terrain at `(x, y)`.
    pub fn area_bit(&mut self, x: f32, y: f32) -> Option<u16> {
        self.tile(x, y)?.area_bit(x, y)
    }

    /// Tiles read (or that didn't decode).
    pub fn tiles_read(&self) -> usize {
        self.tiles.values().filter(|s| s.tile.is_some()).count()
    }

    /// Tiles being read.
    pub fn tiles_loading(&self) -> usize {
        self.tiles.values().filter(|s| s.tile.is_none()).count()
    }
}
