//! Round 9, the terrain's tiles: Thistle Tea's `tile_test.exs`, on tiles
//! made up byte by byte as its test makes them (MaNGOS's z1.5 map files,
//! build 5875): what a tile must be to be read, which tile a point is in,
//! the ground's height across a cell's four triangles in each of the
//! height encodings, and liquid under a hole in the ground.

use wow_world_messages::vanilla::Vector3d;

use crate::game::map::terrain::{Tile, coordinates};

const GRID_SIZE: f64 = 533.333_333_333_333_4;

fn flat_height(level: f32) -> Vec<u8> {
    let mut b = b"MHGT".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend(level.to_le_bytes());
    b.extend(level.to_le_bytes());
    b
}

fn liquid(kind: u16, level: f32) -> Vec<u8> {
    let mut b = b"MLIQ".to_vec();
    b.extend(3u16.to_le_bytes());
    b.extend(kind.to_le_bytes());
    b.extend([0, 0, 129, 129]);
    b.extend(level.to_le_bytes());
    b
}

#[derive(Clone, Copy)]
enum Encoding {
    Float,
    Int16,
    Int8,
}

/// Corner heights 10, 20, 30 and 40 about the first cell, its center 24
/// (stored halved, as the client's are), every other point 0.
fn height_map(encoding: Encoding) -> Vec<u8> {
    let (flags, maximum) = match encoding {
        Encoding::Float => (0u32, 100.0f32),
        Encoding::Int16 => (2, 65_535.0),
        Encoding::Int8 => (4, 255.0),
    };
    let mut b = b"MHGT".to_vec();
    b.extend(flags.to_le_bytes());
    b.extend(0.0f32.to_le_bytes());
    b.extend(maximum.to_le_bytes());
    for i in 0..33_025u32 {
        let v = match i {
            0 => 10,
            129 => 20,
            1 => 30,
            130 => 40,
            16_641 => 24,
            _ => 0,
        };
        match encoding {
            Encoding::Float => b.extend((v as f32).to_le_bytes()),
            Encoding::Int16 => b.extend((v as u16).to_le_bytes()),
            Encoding::Int8 => b.push(v as u8),
        }
    }
    b
}

fn tile_with(height: &[u8], liquid: &[u8], holes: &[u8]) -> Vec<u8> {
    let liquid_offset = if liquid.is_empty() {
        0
    } else {
        44 + height.len() as u32
    };
    let holes_offset = 44 + (height.len() + liquid.len()) as u32;
    let mut b = b"MAPSz1.5".to_vec();
    for v in [
        5875,
        0,
        0,
        44,
        height.len() as u32,
        liquid_offset,
        liquid.len() as u32,
        holes_offset,
        holes.len() as u32,
    ] {
        b.extend(u32::to_le_bytes(v));
    }
    b.extend(height);
    b.extend(liquid);
    b.extend(holes);
    b
}

fn tile(height: &[u8], liquid: &[u8]) -> Vec<u8> {
    tile_with(height, liquid, &[0; 512])
}

fn replace(mut b: Vec<u8>, offset: usize, value: &[u8]) -> Vec<u8> {
    b[offset..offset + value.len()].copy_from_slice(value);
    b
}

/// The point `row` and `column` cells (fractions too) into the tile at the
/// world's middle.
fn position(row: f64, column: f64) -> Vector3d {
    Vector3d {
        x: (-row * GRID_SIZE / 128.0) as f32,
        y: (-column * GRID_SIZE / 128.0) as f32,
        z: 100.0,
    }
}

/// Thistle Tea's "rejects incompatible builds, versions, corrupt sizes and
/// nonfinite heights": a tile is read whole or not at all, of build 5875
/// and version z1.5, its sections where its header says and no longer,
/// and its heights numbers.
#[test]
fn a_tile_of_another_build_or_version_or_corrupt_is_refused() {
    let good = tile(&flat_height(-20.0), &liquid(0x12, 0.0));
    assert!(Tile::decode(&good).is_some(), "a tile");
    let refused = [
        ("cut short", good[..good.len() - 1].to_vec()),
        (
            "another build",
            replace(good.clone(), 8, &1234u32.to_le_bytes()),
        ),
        ("another version", replace(good.clone(), 4, b"z1.4")),
        (
            "liquid nowhere",
            replace(good.clone(), 28, &0xFFFF_FFFFu32.to_le_bytes()),
        ),
        ("liquid too long", {
            let mut l = liquid(0x12, 0.0);
            l.push(0);
            tile(&flat_height(-20.0), &l)
        }),
        (
            "a height no number",
            tile(
                &replace(flat_height(-20.0), 8, &0x7FC0_0000u32.to_le_bytes()),
                &liquid(0x12, 0.0),
            ),
        ),
    ];
    for (what, b) in refused {
        assert!(Tile::decode(&b).is_none(), "{what}");
    }
}

/// Thistle Tea's "keeps world axes and grid boundaries in file order": a
/// tile's row is from x and its column from y, 32 less a tile a step.
/// (A boundary exactly on a tile's edge is past f32's reach: -533.3333 as
/// an f32 is a little short of the edge, so a point a thousandth over it
/// stands for Thistle Tea's.)
#[test]
fn the_tiles_go_by_x_then_y_from_the_worlds_middle() {
    assert_eq!(coordinates(-9500.0, -220.0), (49, 32));
    assert_eq!(coordinates(0.0, 0.0), (32, 32));
    assert_eq!(coordinates(0.001, -0.001), (31, 32));
    assert_eq!(coordinates(-533.334, 533.333_3), (33, 31));
}

/// Thistle Tea's "interpolates each terrain triangle for float and compact
/// height encodings": the ground across each of a cell's four triangles,
/// from its corners and its center, alike in floats, 16 and 8 bits.
#[test]
fn the_ground_is_each_triangles_in_every_height_encoding() {
    for encoding in [Encoding::Float, Encoding::Int16, Encoding::Int8] {
        let t = Tile::decode(&tile(&height_map(encoding), &liquid(0x12, 100.0))).expect("a tile");
        for (x, y, expected) in [
            (0.75, 0.1, 19.3),
            (0.1, 0.75, 25.8),
            (0.9, 0.75, 33.8),
            (0.75, 0.9, 35.3),
        ] {
            let floor = t.sample(position(x, y)).expect("liquid there").floor;
            assert!(
                (floor - expected).abs() < 1e-4,
                "({x}, {y}): {floor}, not {expected}"
            );
        }
    }
}

/// Thistle Tea's "terrain holes do not hide liquid beneath the missing
/// floor": under a hole there is no ground, and the liquid is still there.
#[test]
fn liquid_under_a_hole_in_the_ground_is_still_there() {
    let mut holes = vec![0u8; 512];
    holes[0] = 1;
    let t = Tile::decode(&tile_with(
        &height_map(Encoding::Float),
        &liquid(0x12, 0.0),
        &holes,
    ))
    .expect("a tile");
    let l = t.sample(position(0.25, 0.25)).expect("the liquid");
    assert_eq!((l.floor, l.flags), (-100_000.0, 0x12));
}
