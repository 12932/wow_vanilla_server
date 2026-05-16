use crate::world::world_opcode_handler::gm_command::{next_rand, random_name};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use wow_items::vanilla::{all_items, InventoryType};
use wow_world_base::movement::DEFAULT_RUNNING_SPEED;
use wow_world_base::vanilla::{Level, Map, PlayerGender, RaceClass};
use wow_world_messages::vanilla::{MovementInfo, MovementInfo_MovementFlags, Vector3d};
use wow_world_messages::Guid;

pub const NUM_VISIBLE_SLOTS: usize = 19;

/// Slots populated with random gear (indices in the player visible-item mask).
pub const SLOT_HEAD: usize = 0;
pub const SLOT_SHOULDERS: usize = 2;
pub const SLOT_CHEST: usize = 4;
pub const SLOT_WAIST: usize = 5;
pub const SLOT_LEGS: usize = 6;
pub const SLOT_MAIN_HAND: usize = 15;

const GATE: Vector3d = Vector3d {
    x: -9083.265,
    y: 419.14047,
    z: 92.569046,
};
const BRIDGE_MIDDLE: Vector3d = Vector3d {
    x: -9008.224,
    y: 478.0392,
    z: 96.51068,
};
const BRIDGE_END: Vector3d = Vector3d {
    x: -8953.993,
    y: 521.2531,
    z: 96.355354,
};
const LEFT_WAY_1: Vector3d = Vector3d {
    x: -8973.1875,
    y: 557.4108,
    z: 93.84703,
};
const LEFT_WAY_2: Vector3d = Vector3d {
    x: -8943.09,
    y: 557.5153,
    z: 93.8348,
};
const RIGHT_WAY_1: Vector3d = Vector3d {
    x: -8928.538,
    y: 494.91766,
    z: 93.839935,
};
const RIGHT_WAY_2: Vector3d = Vector3d {
    x: -8911.56,
    y: 507.449,
    z: 93.858665,
};
const GATEWAY_MIDDLE: Vector3d = Vector3d {
    x: -8926.955,
    y: 542.3428,
    z: 94.28875,
};
const GATEWAY_END: Vector3d = Vector3d {
    x: -8890.761,
    y: 571.73267,
    z: 92.48749,
};
const TRADE_DISTRICT_MIDDLE: Vector3d = Vector3d {
    x: -8824.353,
    y: 628.5146,
    z: 93.93376,
};

/// Left-hand walking route from Stormwind south gate to Trade District fountain.
pub const LEFT_ROUTE: &[Vector3d] = &[
    GATE,
    BRIDGE_MIDDLE,
    BRIDGE_END,
    LEFT_WAY_1,
    LEFT_WAY_2,
    GATEWAY_MIDDLE,
    GATEWAY_END,
    TRADE_DISTRICT_MIDDLE,
];

/// Right-hand walking route from Stormwind south gate to Trade District fountain.
pub const RIGHT_ROUTE: &[Vector3d] = &[
    GATE,
    BRIDGE_MIDDLE,
    BRIDGE_END,
    RIGHT_WAY_1,
    RIGHT_WAY_2,
    GATEWAY_MIDDLE,
    GATEWAY_END,
    TRADE_DISTRICT_MIDDLE,
];

#[derive(Debug, Clone)]
pub struct SimulatedPlayer {
    pub guid: Guid,
    pub name: String,
    pub race_class: RaceClass,
    pub gender: PlayerGender,
    pub skin: u8,
    pub face: u8,
    pub hairstyle: u8,
    pub haircolor: u8,
    pub facialhair: u8,
    pub level: Level,
    pub map: Map,
    pub info: MovementInfo,
    pub movement_speed: f32,
    pub equipment: [Option<u32>; NUM_VISIBLE_SLOTS],
    pub current_wp: usize,
    pub waypoints: Vec<Vector3d>,
    pub last_heartbeat: Instant,
    pub last_advanced_at: Instant,
    pub next_jump_at: Instant,
    pub root_until: Option<Instant>,
}

impl SimulatedPlayer {
    pub fn is_rooted(&self) -> bool {
        self.root_until
            .map(|t| t > Instant::now())
            .unwrap_or(false)
    }
}

const HORDE: &[RaceClass] = &[
    RaceClass::OrcWarrior,
    RaceClass::OrcRogue,
    RaceClass::OrcHunter,
    RaceClass::OrcShaman,
    RaceClass::OrcWarlock,
    RaceClass::TaurenWarrior,
    RaceClass::TaurenDruid,
    RaceClass::TaurenHunter,
    RaceClass::TaurenShaman,
    RaceClass::TrollWarrior,
    RaceClass::TrollRogue,
    RaceClass::TrollHunter,
    RaceClass::TrollMage,
    RaceClass::TrollPriest,
    RaceClass::TrollShaman,
    RaceClass::UndeadWarrior,
    RaceClass::UndeadRogue,
    RaceClass::UndeadMage,
    RaceClass::UndeadPriest,
    RaceClass::UndeadWarlock,
];

struct EquipmentByType {
    head: Vec<u32>,
    shoulders: Vec<u32>,
    chest: Vec<u32>,
    waist: Vec<u32>,
    legs: Vec<u32>,
    main_hand: Vec<u32>,
}

fn equipment_index() -> &'static EquipmentByType {
    static IDX: OnceLock<EquipmentByType> = OnceLock::new();
    IDX.get_or_init(|| {
        let mut idx = EquipmentByType {
            head: Vec::new(),
            shoulders: Vec::new(),
            chest: Vec::new(),
            waist: Vec::new(),
            legs: Vec::new(),
            main_hand: Vec::new(),
        };
        for item in all_items() {
            let entry = item.entry();
            match item.inventory_type() {
                InventoryType::Head => idx.head.push(entry),
                InventoryType::Shoulders => idx.shoulders.push(entry),
                InventoryType::Chest | InventoryType::Robe => idx.chest.push(entry),
                InventoryType::Waist => idx.waist.push(entry),
                InventoryType::Legs => idx.legs.push(entry),
                InventoryType::Weapon
                | InventoryType::WeaponMainHand
                | InventoryType::TwoHandedWeapon => idx.main_hand.push(entry),
                _ => {}
            }
        }
        idx
    })
}

fn pick<T: Copy>(v: &[T]) -> Option<T> {
    if v.is_empty() {
        None
    } else {
        Some(v[(next_rand() as usize) % v.len()])
    }
}

fn pick_horde_race_class() -> RaceClass {
    HORDE[(next_rand() as usize) % HORDE.len()]
}

fn pick_gender() -> PlayerGender {
    if next_rand() & 1 == 0 {
        PlayerGender::Male
    } else {
        PlayerGender::Female
    }
}

/// Returns a seed position along the route and the index of the next waypoint.
/// Sampling is length-weighted so puppets snake out evenly instead of clumping.
fn pick_route_start(waypoints: &[Vector3d]) -> (Vector3d, usize) {
    let mut cum = Vec::with_capacity(waypoints.len());
    cum.push(0.0_f32);
    for i in 1..waypoints.len() {
        let a = waypoints[i - 1];
        let b = waypoints[i];
        let d = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
        cum.push(cum[i - 1] + d);
    }
    let total = *cum.last().unwrap();
    // Bias toward the start of the route: cubing a uniform [0,1) sample gives a
    // density heavy near 0, so most puppets spawn near the gate with a tail
    // trailing toward the fountain.
    let u = crate::numeric::rand_unit_f32(next_rand());
    let t = u.powi(4) * total;
    let mut seg = 0;
    while seg + 1 < waypoints.len() - 1 && cum[seg + 1] < t {
        seg += 1;
    }
    let seg_len = cum[seg + 1] - cum[seg];
    let frac = if seg_len > 0.0 {
        (t - cum[seg]) / seg_len
    } else {
        0.0
    };
    let a = waypoints[seg];
    let b = waypoints[seg + 1];
    let pos = Vector3d {
        x: a.x + (b.x - a.x) * frac,
        y: a.y + (b.y - a.y) * frac,
        z: a.z + (b.z - a.z) * frac,
    };
    (pos, seg + 1)
}

fn jitter(v: Vector3d, radius: f32) -> Vector3d {
    let dx = (crate::numeric::rand_unit_f32(next_rand()) - 0.5) * 2.0 * radius;
    let dy = (crate::numeric::rand_unit_f32(next_rand()) - 0.5) * 2.0 * radius;
    Vector3d {
        x: v.x + dx,
        y: v.y + dy,
        z: v.z,
    }
}

pub fn random_horde_at(guid: Guid) -> SimulatedPlayer {
    let race_class = pick_horde_race_class();
    let gender = pick_gender();
    let eq = equipment_index();
    let mut equipment: [Option<u32>; NUM_VISIBLE_SLOTS] = Default::default();
    equipment[SLOT_HEAD] = pick(&eq.head);
    equipment[SLOT_SHOULDERS] = pick(&eq.shoulders);
    equipment[SLOT_CHEST] = pick(&eq.chest);
    equipment[SLOT_WAIST] = pick(&eq.waist);
    equipment[SLOT_LEGS] = pick(&eq.legs);
    equipment[SLOT_MAIN_HAND] = pick(&eq.main_hand);

    let base_route: &'static [Vector3d] = if next_rand() & 1 == 0 {
        LEFT_ROUTE
    } else {
        RIGHT_ROUTE
    };
    let waypoints: Vec<Vector3d> = base_route.iter().map(|wp| jitter(*wp, 3.0)).collect();

    let (seed_pos, current_wp) = pick_route_start(&waypoints);
    let position = jitter(seed_pos, 2.0);
    let target = waypoints[current_wp];
    let orientation = (target.y - position.y).atan2(target.x - position.x);

    SimulatedPlayer {
        guid,
        name: random_name(),
        race_class,
        gender,
        skin: (next_rand() % 10) as u8,
        face: (next_rand() % 10) as u8,
        hairstyle: (next_rand() % 10) as u8,
        haircolor: (next_rand() % 10) as u8,
        facialhair: (next_rand() % 6) as u8,
        level: Level::new_vanilla_max_level_player(),
        map: Map::EasternKingdoms,
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_forward(),
            timestamp: 0,
            position,
            orientation,
            fall_time: 0.0,
        },
        movement_speed: DEFAULT_RUNNING_SPEED,
        equipment,
        current_wp,
        waypoints,
        last_heartbeat: Instant::now(),
        last_advanced_at: Instant::now(),
        next_jump_at: Instant::now() + Duration::from_millis(5000 + next_rand() % 10000),
        root_until: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pick_route_start` should always pick a waypoint index strictly less
    /// than the number of waypoints so the puppet has somewhere to walk
    /// toward, and should never produce a NaN/inf position.
    #[test]
    fn pick_route_start_returns_valid_index_and_finite_pos() {
        let route = LEFT_ROUTE;
        for _ in 0..200 {
            let (pos, idx) = pick_route_start(route);
            assert!(idx < route.len(), "current_wp out of range: {idx}");
            assert!(idx >= 1, "should head toward at least waypoint 1");
            assert!(pos.x.is_finite() && pos.y.is_finite() && pos.z.is_finite());
        }
    }

    #[test]
    fn random_horde_at_produces_walking_puppet_pointing_at_first_target() {
        let g = Guid::new(0xCAFE_BABE);
        let puppet = random_horde_at(g);
        // Always heading toward a real waypoint.
        assert!(puppet.current_wp < puppet.waypoints.len());
        // Equipment slots that we explicitly populate are some real item.
        assert!(puppet.equipment[SLOT_HEAD].is_some());
        assert!(puppet.equipment[SLOT_CHEST].is_some());
        assert!(puppet.equipment[SLOT_MAIN_HAND].is_some());
        // Movement state starts in FORWARD so the spawn broadcast triggers
        // the walk animation on viewers.
        assert!(puppet.info.flags.get_forward());
    }

    #[test]
    fn both_routes_share_gate_endpoint() {
        // Both branches must start at the same southern gate so the
        // length-weighted distribution behaves predictably regardless of
        // which side was rolled.
        assert_eq!(LEFT_ROUTE[0], RIGHT_ROUTE[0]);
        // Both routes terminate at the Trade District fountain.
        assert_eq!(LEFT_ROUTE.last(), RIGHT_ROUTE.last());
    }
}
