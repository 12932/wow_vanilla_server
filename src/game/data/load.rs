//! Loading the game data from `vmangos.sqlite` and `dbc.sqlite`.

use std::path::Path;
use std::time::Instant;

use ahash::{AHashMap, AHashSet};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OpenFlags, Row};
use tracing::{error, info, warn};

use super::*;

/// A row's columns by name, read leniently: a number may be stored as an
/// integer or a real.
pub(super) struct Cols<'a, 'b> {
    row: &'a Row<'b>,
    names: &'a ColumnNames,
}

/// A statement's columns by name, found once rather than for each value of
/// each row (rusqlite's lookup by name scans the columns). As in rusqlite,
/// a name matches whatever its ASCII case, and the first such column wins.
struct ColumnNames {
    /// Each name as the statement has it, for the first column of each
    /// case-blind name only: an exact hit is the column rusqlite would find.
    exact: AHashMap<String, usize>,
    /// Each name in lower case.
    folded: AHashMap<String, usize>,
}

impl ColumnNames {
    fn of(stmt: &rusqlite::Statement) -> Self {
        let (mut exact, mut folded) = (AHashMap::new(), AHashMap::new());
        for (i, name) in stmt.column_names().into_iter().enumerate() {
            let lower = name.to_ascii_lowercase();
            if !folded.contains_key(&lower) {
                folded.insert(lower, i);
                exact.insert(name.to_owned(), i);
            }
        }
        Self { exact, folded }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.exact
            .get(name)
            .or_else(|| self.folded.get(&name.to_ascii_lowercase()))
            .copied()
    }
}

impl Cols<'_, '_> {
    /// The value of the column `name`, borrowed; NULL if there is none.
    fn value_ref(&self, name: &str) -> ValueRef<'_> {
        self.names
            .index(name)
            .and_then(|i| self.row.get_ref(i).ok())
            .unwrap_or(ValueRef::Null)
    }
    pub(super) fn value(&self, name: &str) -> Value {
        Value::try_from(self.value_ref(name)).unwrap_or(Value::Null)
    }
    pub(super) fn i64(&self, name: &str) -> i64 {
        match self.value_ref(name) {
            ValueRef::Integer(i) => i,
            ValueRef::Real(f) => f as i64,
            ValueRef::Text(t) => std::str::from_utf8(t)
                .ok()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0),
            _ => 0,
        }
    }
    pub(super) fn u32(&self, name: &str) -> u32 {
        u32::try_from(self.i64(name).max(0)).unwrap_or(u32::MAX)
    }
    pub(super) fn i32(&self, name: &str) -> i32 {
        self.i64(name)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }
    pub(super) fn u8(&self, name: &str) -> u8 {
        self.i64(name).clamp(0, 255) as u8
    }
    pub(super) fn f32(&self, name: &str) -> f32 {
        match self.value_ref(name) {
            ValueRef::Integer(i) => i as f32,
            ValueRef::Real(f) => f as f32,
            ValueRef::Text(t) => std::str::from_utf8(t)
                .ok()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0.0),
            _ => 0.0,
        }
    }
    pub(super) fn string(&self, name: &str) -> String {
        match self.value(name) {
            Value::Text(t) => t,
            Value::Integer(i) => i.to_string(),
            _ => String::new(),
        }
    }
}

/// Whether `path` is a Git LFS pointer, as a clone made without Git LFS
/// leaves the databases.
pub fn lfs_pointer(path: &Path) -> bool {
    use std::io::Read;
    const POINTER: &[u8] = b"version https://git-lfs.github.com/spec/v1";
    let mut head = [0u8; POINTER.len()];
    std::fs::File::open(path).is_ok_and(|mut f| f.read_exact(&mut head).is_ok()) && head == POINTER
}

/// The database at `path`, read-only; `None`, with a warning, if it won't
/// open. A Git LFS pointer is named as such, not left to SQLite.
fn open(path: &Path) -> Option<Connection> {
    if lfs_pointer(path) {
        error!(
            "{} is a Git LFS pointer: run `git lfs pull`",
            path.display()
        );
        return None;
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .inspect_err(|e| warn!("game data: {}: {e}", path.display()))
    .ok()
}

/// Every row of `sql`, through `f`.
pub(super) fn rows<T>(conn: &Connection, sql: &str, mut f: impl FnMut(&Cols) -> T) -> Vec<T> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            warn!("game data: {sql}: {e}");
            return Vec::new();
        }
    };
    let names = ColumnNames::of(&stmt);
    let mut out = Vec::new();
    let result = stmt.query_map([], |row| Ok(f(&Cols { row, names: &names })));
    match result {
        Ok(iter) => {
            for r in iter.flatten() {
                out.push(r);
            }
        }
        Err(e) => warn!("game data: {sql}: {e}"),
    }
    out
}

/// The rows of a table with a `patch` column, keeping each key's latest
/// patch up to [`PATCH`].
fn latest_patch<K: std::hash::Hash + Eq, T>(items: Vec<(K, i64, T)>) -> AHashMap<K, T> {
    let mut best: AHashMap<K, (i64, T)> = AHashMap::new();
    for (k, patch, t) in items {
        if patch > PATCH {
            continue;
        }
        match best.get(&k) {
            Some((p, _)) if *p >= patch => {}
            _ => {
                best.insert(k, (patch, t));
            }
        }
    }
    best.into_iter().map(|(k, (_, t))| (k, t)).collect()
}

const IN_PATCH: &str = "patch_min <= 10 AND patch_max >= 10";

/// The taxi nodes whose every path out is a spell's (`SEND_TAXI`).
fn spell_only_nodes(data: &mut GameData) {
    let spell_paths: AHashSet<u32> = data
        .spells
        .spells
        .values()
        .flat_map(|s| s.effects.iter())
        .filter(|e| e.effect == 123 && e.misc_value > 0)
        .map(|e| e.misc_value as u32)
        .collect();
    let mut out: AHashMap<u32, bool> = AHashMap::new();
    for ((from, _), (path, _)) in &data.npc.taxi_paths {
        let only = out.entry(*from).or_insert(true);
        *only &= spell_paths.contains(path);
    }
    data.npc.spell_only_nodes = out
        .into_iter()
        .filter(|(_, only)| *only)
        .map(|(n, _)| n)
        .collect();
}

/// The real world's data, loaded once for the tests that only read it;
/// `None` where the databases aren't (CI).
#[cfg(test)]
pub fn real_data() -> Option<&'static GameData> {
    static DATA: std::sync::OnceLock<Option<GameData>> = std::sync::OnceLock::new();
    DATA.get_or_init(|| {
        let dir = &crate::game::harness::real_data("db");
        let db = dir.join("vmangos.sqlite");
        (db.exists() && !lfs_pointer(&db)).then(|| crate::game::harness::load_real(dir))
    })
    .as_ref()
}

/// Load the game data from `dir`, which holds `vmangos.sqlite` and
/// `dbc.sqlite`. A missing database gives the empty world, with a warning.
pub fn load(dir: &Path) -> GameData {
    let started = Instant::now();
    let mut data = GameData::empty();
    let dbc = open(&dir.join("dbc.sqlite"));
    if let Some(dbc) = &dbc {
        load_dbc(dbc, &mut data);
    }
    if let Some(db) = open(&dir.join("vmangos.sqlite")) {
        load_vmangos(&db, &mut data);
        load_npcs(&db, dbc.as_ref(), &mut data);
        load_spells(&db, dbc.as_ref(), &mut data);
        crate::game::map::boss_mechanics::fixups(&mut data);
        super::crafts::load(&db, dbc.as_ref(), &mut data.crafts);
        load_pets(&db, dbc.as_ref(), &mut data);
        load_misc(&db, dbc.as_ref(), &mut data);
        super::scripts_load::load(&db, &mut data);
        super::world::load(&db, dbc.as_ref(), &mut data.world);
        spell_only_nodes(&mut data);
        let _ = super::tt_scripts::install(&mut data);
        // Alterac Valley's NPCs as the battleground drives them, over the
        // general import.
        crate::game::map::battleground::av_scripts::install(&mut data);
        super::scripts_load::link_templates(&mut data);
    }
    let spawns: usize = data.creature_spawns.values().map(Vec::len).sum();
    let objects: usize = data.gameobject_spawns.values().map(Vec::len).sum();
    info!(
        "game data from {}: {} creature templates, {spawns} creatures, {} game object templates, \
         {objects} game objects, {} items, {} spells, {} areas, in {:.1} s",
        dir.display(),
        data.creature_templates.len(),
        data.gameobject_templates.len(),
        data.items.len(),
        data.spells.spells.len(),
        data.areas.len(),
        started.elapsed().as_secs_f32()
    );
    data
}

fn load_dbc(dbc: &Connection, data: &mut GameData) {
    for a in rows(dbc, "SELECT * FROM AreaTable", |c| AreaInfo {
        id: c.u32("id"),
        map: c.u32("map"),
        zone: c.u32("parent_area_table"),
        explore_bit: c.u32("area_bit"),
        flags: c.u32("flags"),
        level: c.u32("exploration_level"),
        name: c.string("area_name_en_gb"),
        team: c.u32("faction_group"),
    }) {
        data.areas.insert(a.id, a);
    }
    for t in rows(dbc, "SELECT * FROM AreaTrigger", |c| AreaTrigger {
        id: c.u32("id"),
        map: c.u32("map"),
        x: c.f32("location_x"),
        y: c.f32("location_y"),
        z: c.f32("location_z"),
        radius: c.f32("radius"),
        box_length: c.f32("box_length"),
        box_width: c.f32("box_width"),
        box_height: c.f32("box_height"),
        box_yaw: c.f32("box_yaw"),
        teleport: None,
    }) {
        data.area_triggers.insert(t.id, t);
    }
    for (id, r) in rows(dbc, "SELECT * FROM ChrRaces", |c| {
        (
            c.u8("id"),
            RaceInfo {
                male_display: c.u32("male_display"),
                female_display: c.u32("female_display"),
                faction: c.u32("faction"),
                cinematic: c.u32("cinematic_sequence"),
                // dbc.sqlite names the starting taxi mask `unknown2`.
                starting_taxi_mask: c.u32("unknown2"),
            },
        )
    }) {
        data.races.insert(id, r);
    }
    // The model's collision height. dbc.sqlite names CreatureModelData's
    // columns one off from 1.12's there: its `collision_width` holds an
    // integer, `collision_height` the width (a human's 0.61) and
    // `mount_height` the height (a human's 2.03), which is what
    // mangos-classic's `GetCollisionHeight` reads.
    for (id, scale, height) in rows(
        dbc,
        "SELECT d.id AS id, d.creature_model_scale * COALESCE(m.model_scale, 1.0) AS scale, \
         COALESCE(m.mount_height, 0.0) AS height \
         FROM CreatureDisplayInfo d LEFT JOIN CreatureModelData m ON m.id = d.model",
        |c| (c.u32("id"), c.f32("scale"), c.f32("height")),
    ) {
        data.display_info.insert(
            id,
            DisplayInfo {
                scale,
                height,
                ..Default::default()
            },
        );
    }
    // A humanoid model of a race that rides, or one flagged to, may mount.
    for (id, can_mount) in rows(
        dbc,
        "SELECT d.id AS id, m.flags AS model_flags, x.id AS extra, r.flags AS race_flags \
         FROM CreatureDisplayInfo d JOIN CreatureModelData m ON m.id = d.model \
         LEFT JOIN CreatureDisplayInfoExtra x ON x.id = d.extended_display_info \
         LEFT JOIN ChrRaces r ON r.id = x.display_race",
        |c| {
            let extra = c.u32("extra") > 0;
            let race = match c.value("race_flags") {
                rusqlite::types::Value::Null => None,
                _ => Some(c.u32("race_flags")),
            };
            let can =
                extra && (c.u32("model_flags") & 0x80 != 0 || race.is_none_or(|r| r & 0x04 != 0));
            (c.u32("id"), can)
        },
    ) {
        if let Some(d) = data.display_info.get_mut(&id) {
            d.can_mount = can_mount;
        }
    }
    for (id, emote) in rows(dbc, "SELECT id, emote FROM EmotesText", |c| {
        (c.u32("id"), c.u32("emote"))
    }) {
        data.text_emote_anims.insert(id, emote);
    }
    for (id, spec) in rows(dbc, "SELECT id, spec_proc FROM Emotes", |c| {
        (c.u32("id"), c.u32("spec_proc"))
    }) {
        if spec != 0 {
            data.emote_states.insert(id);
        }
    }
    for (id, loc) in rows(dbc, "SELECT * FROM WorldSafeLocs", |c| {
        (
            c.u32("id"),
            (
                c.u32("map"),
                c.f32("location_x"),
                c.f32("location_y"),
                c.f32("location_z"),
            ),
        )
    }) {
        data.safe_locs.insert(id, loc);
    }
    for f in rows(dbc, "SELECT * FROM FactionTemplate", |c| FactionTemplate {
        id: c.u32("id"),
        faction: c.u32("faction"),
        flags: c.u32("flags"),
        ours: c.u32("faction_group"),
        friend_mask: c.u32("friend_group"),
        enemy_mask: c.u32("enemy_group"),
        enemies: [
            c.u32("enemies_0"),
            c.u32("enemies_1"),
            c.u32("enemies_2"),
            c.u32("enemies_3"),
        ],
        friends: [
            c.u32("friends_0"),
            c.u32("friends_1"),
            c.u32("friends_2"),
            c.u32("friends_3"),
        ],
    }) {
        data.faction_templates.insert(f.id, f);
    }
    load_durability(dbc, data);
    for m in rows(dbc, "SELECT * FROM Map", |c| MapInfo {
        id: c.u32("id"),
        internal_name: c.string("internal_name"),
        name: c.string("map_name_en_gb"),
        instance_type: c.u32("instance_type"),
        player_limit: c.u32("max_players"),
    }) {
        data.maps.insert(m.id, m);
    }
}

fn load_vmangos(db: &Connection, data: &mut GameData) {
    load_creatures(db, data);
    load_gameobjects(db, data);
    load_items(db, data);
    load_players(db, data);
    load_loot(db, data);
    load_vendors(db, data);

    let teleports = latest_patch(rows(db, "SELECT * FROM areatrigger_teleport", |c| {
        (
            c.u32("id"),
            c.i64("patch"),
            Teleport {
                required_level: c.u8("required_level"),
                message: c.string("message"),
                map: c.u32("target_map"),
                x: c.f32("target_position_x"),
                y: c.f32("target_position_y"),
                z: c.f32("target_position_z"),
                orientation: c.f32("target_orientation"),
            },
        )
    }));
    for t in data.area_triggers.values_mut() {
        t.teleport = teleports.get(&t.id).cloned();
    }
    let facing: AHashMap<u32, f32> = rows(db, "SELECT * FROM world_safe_locs_facing", |c| {
        (c.u32("id"), c.f32("orientation"))
    })
    .into_iter()
    .collect();
    let links = rows(
        db,
        &format!("SELECT * FROM game_graveyard_zone WHERE {IN_PATCH}"),
        |c| (c.u32("id"), c.u32("ghost_zone"), c.u32("faction")),
    );
    for (id, zone, faction) in links {
        if let Some(loc) = data.safe_locs.get(&id) {
            data.graveyards.push(Graveyard {
                id,
                map: loc.0,
                x: loc.1,
                y: loc.2,
                z: loc.3,
                orientation: facing.get(&id).copied().unwrap_or(0.0),
                zone,
                faction,
            });
        }
    }
    data.tele = rows(db, "SELECT * FROM game_tele", |c| Tele {
        name: c.string("name"),
        map: c.u32("map"),
        x: c.f32("position_x"),
        y: c.f32("position_y"),
        z: c.f32("position_z"),
        orientation: c.f32("orientation"),
    });
}

fn load_creatures(db: &Connection, data: &mut GameData) {
    for c in rows(db, "SELECT * FROM creature_classlevelstats", |c| {
        (
            (c.u8("class"), c.u8("level")),
            ClassLevelStats {
                health: c.u32("health"),
                mana: c.u32("mana"),
                melee_damage: c.f32("melee_damage"),
                ranged_damage: c.f32("ranged_damage"),
                attack_power: c.u32("attack_power"),
                ranged_attack_power: c.u32("ranged_attack_power"),
                armor: c.u32("armor"),
                strength: c.u32("strength"),
                intellect: c.u32("intellect"),
                spirit: c.u32("spirit"),
            },
        )
    }) {
        data.class_level_stats.insert(c.0, c.1);
    }
    for (id, radius, reach) in rows(db, "SELECT * FROM creature_display_info_addon", |c| {
        (
            c.u32("display_id"),
            c.f32("bounding_radius"),
            c.f32("combat_reach"),
        )
    }) {
        let d = data.display_info.entry(id).or_insert(DisplayInfo {
            scale: 1.0,
            ..Default::default()
        });
        d.bounding_radius = radius;
        d.combat_reach = reach;
    }

    let templates = latest_patch(rows(db, "SELECT * FROM creature_template", |c| {
        let static1 = c.u32("static_flags1");
        let static2 = c.u32("static_flags2");
        let t = CreatureTemplate {
            entry: c.u32("entry"),
            name: c.string("name"),
            sub_name: c.string("subname"),
            display_ids: [
                c.u32("display_id1"),
                c.u32("display_id2"),
                c.u32("display_id3"),
                c.u32("display_id4"),
            ],
            display_scales: [
                c.f32("display_scale1"),
                c.f32("display_scale2"),
                c.f32("display_scale3"),
                c.f32("display_scale4"),
            ],
            display_probabilities: [
                c.u32("display_probability1"),
                c.u32("display_probability2"),
                c.u32("display_probability3"),
                c.u32("display_probability4"),
            ],
            mount_display_id: c.u32("mount_display_id"),
            level_min: c.u8("level_min"),
            level_max: c.u8("level_max"),
            faction: c.u32("faction"),
            npc_flags: c.u32("npc_flags"),
            gossip_menu_id: c.u32("gossip_menu_id"),
            unit_flags: unit_flags(static1),
            dynamic_flags: 0,
            creature_type: c.u32("type"),
            family: c.u32("pet_family"),
            rank: c.u32("rank"),
            type_flags: type_flags(static1, static2),
            unit_class: c.u8("unit_class").max(1),
            speed_walk: c.f32("speed_walk"),
            speed_run: c.f32("speed_run"),
            detection_range: c.f32("detection_range"),
            call_for_help_range: c.f32("call_for_help_range"),
            leash_range: c.f32("leash_range"),
            health_multiplier: c.f32("health_multiplier"),
            mana_multiplier: c.f32("mana_multiplier"),
            armor_multiplier: c.f32("armor_multiplier"),
            damage_multiplier: c.f32("damage_multiplier"),
            damage_variance: c.f32("damage_variance"),
            damage_school: c.u8("damage_school"),
            base_attack_time: c.u32("base_attack_time"),
            ranged_attack_time: c.u32("ranged_attack_time"),
            resistances: [
                c.i32("holy_res"),
                c.i32("fire_res"),
                c.i32("nature_res"),
                c.i32("frost_res"),
                c.i32("shadow_res"),
                c.i32("arcane_res"),
            ],
            xp_multiplier: match c.value("xp_multiplier") {
                Value::Null => None,
                _ => Some(c.f32("xp_multiplier")),
            },
            loot_id: c.u32("loot_id"),
            pickpocket_loot_id: c.u32("pickpocket_loot_id"),
            skinning_loot_id: c.u32("skinning_loot_id"),
            gold_min: c.u32("gold_min"),
            gold_max: c.u32("gold_max"),
            spell_list_id: c.u32("spell_list_id"),
            movement_type: c.u8("movement_type"),
            inhabit_type: c.u8("inhabit_type"),
            civilian: c.i64("civilian") != 0,
            racial_leader: c.i64("racial_leader") != 0,
            equipment_id: c.u32("equipment_id"),
            trainer_id: c.u32("trainer_id"),
            vendor_id: c.u32("vendor_id"),
            trainer_type: c.u8("trainer_type"),
            trainer_spell: c.u32("trainer_spell"),
            trainer_class: c.u8("trainer_class"),
            trainer_race: c.u8("trainer_race"),
            static_flags1: static1,
            static_flags2: static2,
            flags_extra: c.u32("flags_extra"),
            mechanic_immune_mask: c.u32("mechanic_immune_mask"),
            school_immune_mask: c.u32("school_immune_mask"),
            auras: parse_ids(&c.string("auras")),
            ai_name: c.string("ai_name"),
            script_name: c.string("script_name"),
            health_min: 0,
            health_max: 0,
        };
        (t.entry, c.i64("patch"), t)
    }));
    for (entry, mut t) in templates {
        let health = |level: u8| {
            data.class_level_stats
                .get(&(t.unit_class, level))
                .map(|s| creature_health(s.health, t.health_multiplier))
                .unwrap_or(0)
        };
        t.health_min = health(t.level_min.max(1));
        t.health_max = health(t.level_max.max(t.level_min).max(1));
        data.creature_templates.insert(entry, t);
    }

    let mut skipped = 0;
    for s in rows(
        db,
        &format!("SELECT * FROM creature WHERE {IN_PATCH}"),
        |c| CreatureSpawn {
            guid: c.u32("guid"),
            entries: ["id", "id2", "id3", "id4", "id5"]
                .iter()
                .map(|n| c.u32(n))
                .filter(|e| *e != 0)
                .collect(),
            map: c.u32("map"),
            x: c.f32("position_x"),
            y: c.f32("position_y"),
            z: c.f32("position_z"),
            orientation: c.f32("orientation"),
            respawn_min: c.u32("spawntimesecsmin"),
            respawn_max: c.u32("spawntimesecsmax"),
            wander_distance: c.f32("wander_distance"),
            health_percent: c.f32("health_percent"),
            movement_type: c.u8("movement_type"),
            spawn_flags: c.u32("spawn_flags"),
        },
    ) {
        if s.entries
            .iter()
            .any(|e| !data.creature_templates.contains_key(e))
            || s.entries.is_empty()
        {
            skipped += 1;
            continue;
        }
        data.creature_spawns.entry(s.map).or_default().push(s);
    }
    if skipped > 0 {
        info!("game data: {skipped} creature spawns without a template skipped");
    }

    data.creature_addons = latest_patch(rows(db, "SELECT * FROM creature_addon", |c| {
        (
            c.u32("guid"),
            c.i64("patch"),
            CreatureAddon {
                display_id: c.u32("display_id"),
                mount_display_id: c.i32("mount_display_id"),
                equipment_id: c.i32("equipment_id"),
                stand_state: c.u8("stand_state"),
                sheath_state: c.u8("sheath_state"),
                emote_state: c.u32("emote_state"),
                auras: parse_ids(&c.string("auras")),
            },
        )
    }));
    for (entry, items) in rows(
        db,
        &format!(
            "SELECT * FROM creature_equip_template WHERE {IN_PATCH} ORDER BY probability DESC"
        ),
        |c| {
            (
                c.u32("entry"),
                [c.u32("item1"), c.u32("item2"), c.u32("item3")],
            )
        },
    ) {
        data.creature_equipment.entry(entry).or_insert(items);
    }
    for (id, w) in rows(
        db,
        "SELECT * FROM creature_movement ORDER BY id, point",
        |c| {
            (
                c.u32("id"),
                Waypoint {
                    point: c.u32("point"),
                    x: c.f32("position_x"),
                    y: c.f32("position_y"),
                    z: c.f32("position_z"),
                    orientation: super::waypoint_facing(c.f32("orientation")),
                    wait_ms: c.u32("waittime"),
                    script_id: c.u32("script_id"),
                },
            )
        },
    ) {
        data.waypoints.entry(id).or_default().push(w);
    }
    for (entry, w) in rows(
        db,
        "SELECT * FROM creature_movement_template ORDER BY entry, path_id, point",
        |c| {
            (
                c.u32("entry"),
                Waypoint {
                    point: c.u32("point"),
                    x: c.f32("position_x"),
                    y: c.f32("position_y"),
                    z: c.f32("position_z"),
                    orientation: super::waypoint_facing(c.f32("orientation")),
                    wait_ms: c.u32("waittime"),
                    script_id: c.u32("script_id"),
                },
            )
        },
    ) {
        data.waypoints_by_entry.entry(entry).or_default().push(w);
    }
    for (entry, w) in rows(
        db,
        "SELECT * FROM creature_movement_special ORDER BY id, point",
        |c| {
            (
                c.u32("id"),
                Waypoint {
                    point: c.u32("point"),
                    x: c.f32("position_x"),
                    y: c.f32("position_y"),
                    z: c.f32("position_z"),
                    orientation: super::waypoint_facing(c.f32("orientation")),
                    wait_ms: c.u32("waittime"),
                    script_id: c.u32("script_id"),
                },
            )
        },
    ) {
        data.waypoints_special.entry(entry).or_default().push(w);
    }
    for (entry, w) in rows(
        db,
        "SELECT * FROM script_waypoint ORDER BY entry, pointid",
        |c| {
            (
                c.u32("entry"),
                Waypoint {
                    point: c.u32("pointid"),
                    x: c.f32("location_x"),
                    y: c.f32("location_y"),
                    z: c.f32("location_z"),
                    orientation: None,
                    wait_ms: c.u32("waittime"),
                    script_id: 0,
                },
            )
        },
    ) {
        data.script_waypoints.entry(entry).or_default().push(w);
    }
}

/// vmangos's creature health: the class's health at the level times the
/// template's multiplier, at least 1.
pub fn creature_health(base: u32, multiplier: f32) -> u32 {
    let m = if multiplier > 0.0 { multiplier } else { 1.0 };
    ((base as f32 * m).round() as u32).max(1)
}

/// The unit flags a template's static flags give (as Thistle Tea maps
/// them, after vmangos).
pub(crate) fn unit_flags(static1: u32) -> u32 {
    let mut flags = 0;
    for (stat, unit) in [
        (0x0000_0020, 0x0000_0100),
        (0x0000_0040, 0x0000_0200),
        (0x0000_0200, 0x0200_0000),
        (0x1000_0000, 0x0000_8000),
        (0x0040_0000, 0x0000_1000),
    ] {
        if static1 & stat != 0 {
            flags |= unit;
        }
    }
    flags
}

/// The type flags the client reads from a creature query (vmangos's
/// `CreatureInfo::GetTypeFlags`).
pub(crate) fn type_flags(static1: u32, static2: u32) -> u32 {
    let mut flags = 0;
    for (stat, ty) in [
        (0x0000_0010, 0x01),
        (0x0020_0000, 0x02),
        (0x0001_0000, 0x04),
        (0x0080_0000, 0x08),
        (0x0100_0000, 0x10),
        (0x4000_0000, 0x20),
    ] {
        if static1 & stat != 0 {
            flags |= ty;
        }
    }
    if static2 & 0x8 != 0 {
        flags |= 0x40;
    }
    flags
}

fn parse_ids(s: &str) -> Vec<u32> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .filter(|v| *v != 0)
        .collect()
}

fn load_gameobjects(db: &Connection, data: &mut GameData) {
    data.gameobject_templates = latest_patch(rows(db, "SELECT * FROM gameobject_template", |c| {
        let mut d = [0u32; 24];
        for (i, v) in d.iter_mut().enumerate() {
            *v = c.u32(&format!("data{i}"));
        }
        let t = GameObjectTemplate {
            entry: c.u32("entry"),
            kind: c.u8("type"),
            display_id: c.u32("displayId"),
            name: c.string("name"),
            icon: c.string("icon"),
            faction: c.u32("faction"),
            flags: c.u32("flags"),
            size: c.f32("size"),
            data: d,
            gold_min: c.u32("mingold"),
            gold_max: c.u32("maxgold"),
            bounds: None,
        };
        (t.entry, c.i64("patch"), t)
    }));
    let boxes: AHashMap<u32, ([f32; 3], [f32; 3])> =
        rows(db, "SELECT * FROM gameobject_display_info_addon", |c| {
            (
                c.u32("display_id"),
                (
                    [c.f32("min_x"), c.f32("min_y"), c.f32("min_z")],
                    [c.f32("max_x"), c.f32("max_y"), c.f32("max_z")],
                ),
            )
        })
        .into_iter()
        .collect();
    for t in data.gameobject_templates.values_mut() {
        t.bounds = boxes.get(&t.display_id).copied();
    }
    for s in rows(
        db,
        &format!("SELECT * FROM gameobject WHERE {IN_PATCH}"),
        |c| GameObjectSpawn {
            guid: c.u32("guid"),
            entry: c.u32("id"),
            map: c.u32("map"),
            x: c.f32("position_x"),
            y: c.f32("position_y"),
            z: c.f32("position_z"),
            orientation: c.f32("orientation"),
            rotation: [
                c.f32("rotation0"),
                c.f32("rotation1"),
                c.f32("rotation2"),
                c.f32("rotation3"),
            ],
            respawn_min: c.i64("spawntimesecsmin").unsigned_abs() as u32,
            respawn_max: c.i64("spawntimesecsmax").unsigned_abs() as u32,
            anim_progress: c.u8("animprogress"),
            state: c.u8("state"),
            spawn_flags: c.u32("spawn_flags"),
            despawned: c.i64("spawntimesecsmin") < 0,
        },
    ) {
        if data.gameobject_templates.contains_key(&s.entry) {
            data.gameobject_spawns.entry(s.map).or_default().push(s);
        }
    }
}

fn load_items(db: &Connection, data: &mut GameData) {
    data.items = latest_patch(rows(db, "SELECT * FROM item_template", |c| {
        let mut stats = [(0u32, 0i32); 10];
        for (i, s) in stats.iter_mut().enumerate() {
            *s = (
                c.u32(&format!("stat_type{}", i + 1)),
                c.i32(&format!("stat_value{}", i + 1)),
            );
        }
        let mut damages = [(0f32, 0f32, 0u32); 5];
        for (i, d) in damages.iter_mut().enumerate() {
            *d = (
                c.f32(&format!("dmg_min{}", i + 1)),
                c.f32(&format!("dmg_max{}", i + 1)),
                c.u32(&format!("dmg_type{}", i + 1)),
            );
        }
        let mut spells = [items::ItemSpell::default(); 5];
        for (i, s) in spells.iter_mut().enumerate() {
            let n = i + 1;
            *s = items::ItemSpell {
                id: c.u32(&format!("spellid_{n}")),
                trigger: c.u32(&format!("spelltrigger_{n}")),
                charges: c.i32(&format!("spellcharges_{n}")),
                ppm_rate: c.f32(&format!("spellppmrate_{n}")),
                cooldown: c.i32(&format!("spellcooldown_{n}")),
                category: c.u32(&format!("spellcategory_{n}")),
                category_cooldown: c.i32(&format!("spellcategorycooldown_{n}")),
            };
        }
        let t = ItemTemplate {
            entry: c.u32("entry"),
            class: c.u32("class"),
            subclass: c.u32("subclass"),
            name: c.string("name"),
            description: c.string("description"),
            display_id: c.u32("display_id"),
            quality: c.u32("quality"),
            flags: c.u32("flags"),
            buy_count: c.u32("buy_count"),
            buy_price: c.u32("buy_price"),
            sell_price: c.u32("sell_price"),
            inventory_type: c.u32("inventory_type"),
            allowable_class: c.i32("allowable_class"),
            allowable_race: c.i32("allowable_race"),
            item_level: c.u32("item_level"),
            required_level: c.u32("required_level"),
            required_skill: c.u32("required_skill"),
            required_skill_rank: c.u32("required_skill_rank"),
            required_spell: c.u32("required_spell"),
            required_honor_rank: c.u32("required_honor_rank"),
            required_city_rank: c.u32("required_city_rank"),
            required_reputation_faction: c.u32("required_reputation_faction"),
            required_reputation_rank: c.u32("required_reputation_rank"),
            max_count: c.u32("max_count"),
            stackable: c.u32("stackable"),
            container_slots: c.u32("container_slots"),
            stats,
            delay: c.u32("delay"),
            range_mod: c.f32("range_mod"),
            ammo_type: c.u32("ammo_type"),
            damages,
            block: c.u32("block"),
            armor: c.i32("armor"),
            resistances: [
                c.i32("holy_res"),
                c.i32("fire_res"),
                c.i32("nature_res"),
                c.i32("frost_res"),
                c.i32("shadow_res"),
                c.i32("arcane_res"),
            ],
            spells,
            bonding: c.u32("bonding"),
            page_text: c.u32("page_text"),
            page_language: c.u32("page_language"),
            page_material: c.u32("page_material"),
            start_quest: c.u32("start_quest"),
            lock_id: c.u32("lock_id"),
            material: c.i32("material"),
            sheath: c.u32("sheath"),
            random_property: c.u32("random_property"),
            set_id: c.u32("set_id"),
            max_durability: c.u32("max_durability"),
            area_bound: c.u32("area_bound"),
            map_bound: c.u32("map_bound"),
            duration: c.u32("duration"),
            bag_family: c.u32("bag_family"),
            disenchant_id: c.u32("disenchant_id"),
            food_type: c.u32("food_type"),
            min_money_loot: c.u32("min_money_loot"),
            max_money_loot: c.u32("max_money_loot"),
            extra_flags: c.u32("extra_flags"),
        };
        (t.entry, c.i64("patch"), t)
    }));
}

/// `forbidden_items`: what patch 1.12 hasn't (removed by a patch up to it,
/// or not yet added before one from it), off vendors and out of loot.
fn forbidden_items(db: &Connection) -> ahash::AHashSet<u32> {
    rows(
        db,
        "SELECT entry FROM forbidden_items \
         WHERE (after_or_before = 0 AND patch <= 10) OR (after_or_before = 1 AND patch >= 10)",
        |c| c.u32("entry"),
    )
    .into_iter()
    .collect()
}

fn load_loot(db: &Connection, data: &mut GameData) {
    let forbidden = forbidden_items(db);
    let table = |name: &str| {
        let mut out: AHashMap<u32, Vec<LootEntry>> = AHashMap::new();
        for (entry, e) in rows(db, &format!("SELECT * FROM {name} WHERE {IN_PATCH}"), |c| {
            (
                c.u32("entry"),
                LootEntry {
                    item: c.u32("item"),
                    chance: c.f32("ChanceOrQuestChance"),
                    group: c.u8("groupid"),
                    min: c.i32("mincountOrRef"),
                    max: c.u32("maxcount"),
                    condition: c.u32("condition_id"),
                },
            )
        }) {
            // A reference stays; a forbidden item doesn't drop.
            if e.min >= 0 && forbidden.contains(&e.item) {
                continue;
            }
            out.entry(entry).or_default().push(e);
        }
        out
    };
    data.loot = LootTables {
        creature: table("creature_loot_template"),
        gameobject: table("gameobject_loot_template"),
        reference: table("reference_loot_template"),
        pickpocket: table("pickpocketing_loot_template"),
        skinning: table("skinning_loot_template"),
        fishing: table("fishing_loot_template"),
        item: table("item_loot_template"),
        disenchant: table("disenchant_loot_template"),
    };
}

fn load_vendors(db: &Connection, data: &mut GameData) {
    let forbidden = forbidden_items(db);
    let list = |name: &str| {
        let mut out: AHashMap<u32, Vec<VendorItem>> = AHashMap::new();
        for (entry, v) in rows(
            db,
            &format!("SELECT * FROM {name} ORDER BY entry, slot"),
            |c| {
                (
                    c.u32("entry"),
                    VendorItem {
                        slot: c.u32("slot"),
                        item: c.u32("item"),
                        max_count: c.u32("maxcount"),
                        restock_secs: c.u32("incrtime"),
                        condition: c.u32("condition_id"),
                        flags: c.u32("itemflags"),
                    },
                )
            },
        ) {
            if forbidden.contains(&v.item) {
                continue;
            }
            out.entry(entry).or_default().push(v);
        }
        out
    };
    data.vendors = list("npc_vendor");
    data.vendor_templates = list("npc_vendor_template");
}

fn load_durability(dbc: &Connection, data: &mut GameData) {
    for (id, costs) in rows(dbc, "SELECT * FROM DurabilityCosts", |c| {
        let mut costs = Vec::with_capacity(29);
        for i in 0..21 {
            costs.push(c.u32(&format!("weapon_subclass_cost_{i}")));
        }
        for i in 0..8 {
            costs.push(c.u32(&format!("armour_subclass_cost_{i}")));
        }
        (c.u32("id"), costs)
    }) {
        data.durability_costs.insert(id, costs);
    }
    for (id, q) in rows(dbc, "SELECT * FROM DurabilityQuality", |c| {
        (c.u32("id"), c.f32("data"))
    }) {
        data.durability_quality.insert(id, q);
    }
}

fn load_npcs(db: &Connection, dbc: Option<&Connection>, data: &mut GameData) {
    use super::npc::*;
    let npc = &mut data.npc;
    for (menu, text, condition) in rows(db, "SELECT * FROM gossip_menu", |c| {
        (c.u32("entry"), c.u32("text_id"), c.u32("condition_id"))
    }) {
        npc.gossip_menus
            .entry(menu)
            .or_default()
            .texts
            .push((text, condition));
    }
    for (menu, o) in rows(
        db,
        "SELECT * FROM gossip_menu_option ORDER BY menu_id, id",
        |c| {
            (
                c.u32("menu_id"),
                GossipOption {
                    id: c.u32("id"),
                    icon: c.u8("option_icon"),
                    text: c.string("option_text"),
                    kind: c.u32("option_id"),
                    npc_flag: c.u32("npc_option_npcflag"),
                    action_menu: c.i32("action_menu_id"),
                    action_poi: c.u32("action_poi_id"),
                    coded: c.i64("box_coded") != 0,
                    box_money: c.u32("box_money"),
                    box_text: c.string("box_text"),
                    condition: c.u32("condition_id"),
                    script: c.u32("action_script_id"),
                },
            )
        },
    ) {
        npc.gossip_menus.entry(menu).or_default().options.push(o);
    }
    for (id, choices) in rows(db, "SELECT * FROM npc_text", |c| {
        let mut choices = [NpcTextChoice::default(); 8];
        for (i, ch) in choices.iter_mut().enumerate() {
            *ch = NpcTextChoice {
                broadcast: c.u32(&format!("BroadcastTextID{i}")),
                probability: c.f32(&format!("Probability{i}")),
            };
        }
        (c.u32("ID"), choices)
    }) {
        npc.npc_texts.insert(id, choices);
    }
    for (id, b) in rows(db, "SELECT * FROM broadcast_text", |c| {
        (
            c.u32("entry"),
            BroadcastText {
                male: c.string("male_text"),
                female: c.string("female_text"),
                language: c.u32("language_id"),
                emotes: [
                    (c.u32("emote_delay1"), c.u32("emote_id1")),
                    (c.u32("emote_delay2"), c.u32("emote_id2")),
                    (c.u32("emote_delay3"), c.u32("emote_id3")),
                ],
                chat_type: c.u32("chat_type"),
                sound: c.u32("sound_id"),
            },
        )
    }) {
        npc.broadcast_texts.insert(id, b);
    }
    let trainer = |c: &Cols| {
        (
            c.u32("entry"),
            TrainerSpell {
                spell: c.u32("spell"),
                cost: c.u32("spellcost"),
                required_skill: c.u32("reqskill"),
                required_skill_value: c.u32("reqskillvalue"),
                required_level: c.u8("reqlevel"),
            },
        )
    };
    let in_build = "WHERE build_min <= 5875 AND build_max >= 5875";
    for (entry, s) in rows(
        db,
        &format!("SELECT * FROM npc_trainer {in_build}"),
        trainer,
    ) {
        npc.trainers.entry(entry).or_default().push(s);
    }
    for (entry, s) in rows(
        db,
        &format!("SELECT * FROM npc_trainer_template {in_build}"),
        trainer,
    ) {
        npc.trainer_templates.entry(entry).or_default().push(s);
    }
    for (entry, text) in rows(db, "SELECT * FROM npc_trainer_greeting", |c| {
        (c.u32("entry"), c.string("content_default"))
    }) {
        npc.trainer_greetings.insert(entry, text);
    }
    for (entry, text) in rows(db, "SELECT * FROM quest_greeting", |c| {
        (c.u32("entry"), c.string("content_default"))
    }) {
        npc.quest_greetings.insert(entry, text);
    }
    for n in rows(
        db,
        "SELECT * FROM taxi_nodes WHERE build <= 5875 ORDER BY build",
        |c| TaxiNode {
            id: c.u32("id"),
            map: c.u32("map_id"),
            x: c.f32("x"),
            y: c.f32("y"),
            z: c.f32("z"),
            name: c.string("name"),
            // The first is the Horde's (a wind rider), the second the
            // Alliance's (a gryphon), as mangos's `GetTaxiMountDisplayId`.
            mounts: [c.u32("mount_creature_id2"), c.u32("mount_creature_id1")],
        },
    ) {
        npc.taxi_nodes.insert(n.id, n);
    }
    if let Some(dbc) = dbc {
        for (id, from, to, cost) in rows(dbc, "SELECT * FROM TaxiPath", |c| {
            (
                c.u32("id"),
                c.u32("source_taxi_node"),
                c.u32("destination_taxi_node"),
                c.u32("cost"),
            )
        }) {
            npc.taxi_paths.insert((from, to), (id, cost));
        }
        for (paths, nodes) in rows(
            db,
            "SELECT * FROM taxi_path_transitions WHERE build_min <= 5875",
            |c| {
                (
                    (c.u32("in_path"), c.u32("out_path")),
                    (c.u32("in_node") as usize, c.u32("out_node") as usize),
                )
            },
        ) {
            npc.taxi_transitions.insert(paths, nodes);
        }
        for (path, n) in rows(
            dbc,
            "SELECT * FROM TaxiPathNode ORDER BY taxi_path, node_index",
            |c| {
                (
                    c.u32("taxi_path"),
                    TaxiPathNode {
                        map: c.u32("map"),
                        x: c.f32("location_x"),
                        y: c.f32("location_y"),
                        z: c.f32("location_z"),
                        flags: c.u32("flags"),
                        delay: c.u32("delay"),
                    },
                )
            },
        ) {
            npc.taxi_path_nodes.entry(path).or_default().push(n);
        }
        for (id, cost) in rows(dbc, "SELECT * FROM BankBagSlotPrices", |c| {
            (c.u32("id"), c.u32("cost"))
        }) {
            npc.bank_slot_prices.insert(id, cost);
        }
    }
    let pair = |c: &Cols, a: &str, b: &str| (c.u32(a), c.u32(b));
    npc.quests = latest_patch(rows(db, "SELECT * FROM quest_template", |c| {
        let q = Quest {
            id: c.u32("entry"),
            method: c.u32("Method"),
            auto_complete: c.u32("Method") == 0,
            zone_or_sort: c.i32("ZoneOrSort"),
            min_level: c.u8("MinLevel"),
            level: c.i32("QuestLevel"),
            kind: c.u32("Type"),
            required_classes: c.u32("RequiredClasses"),
            required_races: c.u32("RequiredRaces"),
            required_skill: c.u32("RequiredSkill"),
            required_skill_value: c.u32("RequiredSkillValue"),
            prev_quest: c.i32("PrevQuestId"),
            next_quest: c.i32("NextQuestId"),
            exclusive_group: c.i32("ExclusiveGroup"),
            next_in_chain: c.u32("NextQuestInChain"),
            breadcrumb_for: c.u32("BreadcrumbForQuestId"),
            source_item: c.u32("SrcItemId"),
            source_item_count: c.u32("SrcItemCount"),
            limit_time: c.u32("LimitTime"),
            flags: c.u32("QuestFlags"),
            special_flags: c.u32("SpecialFlags"),
            suggested_players: c.u32("SuggestedPlayers"),
            title: c.string("Title"),
            details: c.string("Details"),
            objectives: c.string("Objectives"),
            offer_reward_text: c.string("OfferRewardText"),
            request_items_text: c.string("RequestItemsText"),
            end_text: c.string("EndText"),
            objective_texts: std::array::from_fn(|i| c.string(&format!("ObjectiveText{}", i + 1))),
            required_items: std::array::from_fn(|i| {
                pair(
                    c,
                    &format!("ReqItemId{}", i + 1),
                    &format!("ReqItemCount{}", i + 1),
                )
            }),
            required_kills: std::array::from_fn(|i| {
                (
                    c.i32(&format!("ReqCreatureOrGOId{}", i + 1)),
                    c.u32(&format!("ReqCreatureOrGOCount{}", i + 1)),
                )
            }),
            required_spell_casts: std::array::from_fn(|i| c.u32(&format!("ReqSpellCast{}", i + 1))),
            source_items: std::array::from_fn(|i| {
                pair(
                    c,
                    &format!("ReqSourceId{}", i + 1),
                    &format!("ReqSourceCount{}", i + 1),
                )
            }),
            choice_rewards: std::array::from_fn(|i| {
                pair(
                    c,
                    &format!("RewChoiceItemId{}", i + 1),
                    &format!("RewChoiceItemCount{}", i + 1),
                )
            }),
            rewards: std::array::from_fn(|i| {
                pair(
                    c,
                    &format!("RewItemId{}", i + 1),
                    &format!("RewItemCount{}", i + 1),
                )
            }),
            reputation_rewards: std::array::from_fn(|i| {
                (
                    c.u32(&format!("RewRepFaction{}", i + 1)),
                    c.i32(&format!("RewRepValue{}", i + 1)),
                )
            }),
            reputation_no_spillover: c.u32("RewRepSpilloverMask"),
            required_min_rep: (c.u32("RequiredMinRepFaction"), c.i32("RequiredMinRepValue")),
            required_max_rep: (c.u32("RequiredMaxRepFaction"), c.i32("RequiredMaxRepValue")),
            rep_objective: (c.u32("RepObjectiveFaction"), c.i32("RepObjectiveValue")),
            reward_xp: c.u32("RewXP"),
            reward_money: c.i32("RewOrReqMoney"),
            reward_money_max_level: c.u32("RewMoneyMaxLevel"),
            reward_spell: c.u32("RewSpell"),
            reward_spell_cast: c.u32("RewSpellCast"),
            point: (
                c.u32("PointMapId"),
                c.f32("PointX"),
                c.f32("PointY"),
                c.u32("PointOpt"),
            ),
            details_emotes: std::array::from_fn(|i| {
                pair(
                    c,
                    &format!("DetailsEmote{}", i + 1),
                    &format!("DetailsEmoteDelay{}", i + 1),
                )
            }),
            incomplete_emote: c.u32("IncompleteEmote"),
            complete_emote: c.u32("CompleteEmote"),
            offer_reward_emotes: std::array::from_fn(|i| {
                pair(
                    c,
                    &format!("OfferRewardEmote{}", i + 1),
                    &format!("OfferRewardEmoteDelay{}", i + 1),
                )
            }),
        };
        (q.id, c.i64("patch"), q)
    }));
    let relation = |table: &str| {
        let mut out: AHashMap<u32, Vec<u32>> = AHashMap::new();
        for (id, quest) in rows(
            db,
            &format!("SELECT * FROM {table} WHERE {IN_PATCH}"),
            |c| (c.u32("id"), c.u32("quest")),
        ) {
            out.entry(id).or_default().push(quest);
        }
        out
    };
    npc.creature_starts = relation("creature_questrelation");
    npc.creature_ends = relation("creature_involvedrelation");
    let relation_any = |table: &str| {
        let mut out: AHashMap<u32, Vec<u32>> = AHashMap::new();
        for (id, quest) in rows(db, &format!("SELECT * FROM {table}"), |c| {
            (c.u32("id"), c.u32("quest"))
        }) {
            out.entry(id).or_default().push(quest);
        }
        out
    };
    npc.object_starts = relation_any("gameobject_questrelation");
    npc.object_ends = relation_any("gameobject_involvedrelation");
    for (entry, bg) in rows(db, "SELECT * FROM battlemaster_entry", |c| {
        (c.u32("entry"), c.u32("bg_template"))
    }) {
        data.battlemasters.insert(entry, bg);
    }
    for (guid, event) in rows(db, "SELECT * FROM creature_battleground", |c| {
        (c.u32("guid"), (c.u8("event1"), c.u8("event2")))
    }) {
        data.bg_creature_events.entry(guid).or_default().push(event);
    }
    for (guid, event) in rows(db, "SELECT * FROM gameobject_battleground", |c| {
        (c.u32("guid"), (c.u8("event1"), c.u8("event2")))
    }) {
        data.bg_object_events.entry(guid).or_default().push(event);
    }
    let routed = crate::game::map::battleground::av_quests::ROUTED
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    for (entry, point) in rows(
        db,
        &format!("SELECT * FROM script_waypoint WHERE entry IN ({routed}) ORDER BY entry, pointid"),
        |c| {
            (
                c.u32("entry"),
                (
                    c.f32("location_x"),
                    c.f32("location_y"),
                    c.f32("location_z"),
                ),
            )
        },
    ) {
        data.bg_routes.entry(entry).or_default().push(point);
    }
    for e in rows(db, "SELECT * FROM areatrigger_bg_entrance", |c| {
        BgEntrance {
            trigger: c.u32("id"),
            bg: c.u32("bg_template"),
            team: c.u32("team"),
            exit_map: c.u32("exit_map"),
            exit: (
                c.f32("exit_position_x"),
                c.f32("exit_position_y"),
                c.f32("exit_position_z"),
                c.f32("exit_orientation"),
            ),
        }
    }) {
        data.bg_entrances.insert(e.trigger, e);
    }
    let parse_time = |s: &str| {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .ok()
            .map(|t| t.and_utc().timestamp())
    };
    for (e, disabled, hardcoded, patch_min, patch_max) in
        rows(db, "SELECT * FROM game_event", |c| {
            (
                GameEvent {
                    id: c.u32("entry"),
                    name: c.string("description"),
                    start: parse_time(&c.string("start_time")).unwrap_or(i64::MAX),
                    end: parse_time(&c.string("end_time")).unwrap_or(0),
                    occurence: c.i64("occurence"),
                    length: c.i64("length"),
                    hardcoded: c.u32("hardcoded") != 0,
                },
                c.u32("disabled"),
                c.u32("hardcoded"),
                c.i64("patch_min"),
                c.i64("patch_max"),
            )
        })
    {
        // Hard-coded events follow their own rules (`events::rules`).
        let _ = hardcoded;
        if disabled == 0 && patch_min <= PATCH && patch_max >= PATCH {
            data.events.insert(e.id, e);
        }
    }
    for (guid, event) in rows(db, "SELECT * FROM game_event_creature", |c| {
        (c.u32("guid"), c.i64("event") as i32)
    }) {
        data.event_creatures.insert(guid, event);
    }
    for (guid, event) in rows(db, "SELECT * FROM game_event_gameobject", |c| {
        (c.u32("guid"), c.i64("event") as i32)
    }) {
        data.event_objects.insert(guid, event);
    }
    for (guid, req) in rows(db, "SELECT * FROM gameobject_requirement", |c| {
        (c.u32("guid"), (c.u8("reqType"), c.u32("reqGuid")))
    }) {
        data.object_requirements.insert(guid, req);
    }
    for (zone, chances) in rows(db, "SELECT * FROM game_weather", |c| {
        let season = |s: &str| {
            [
                c.u32(&format!("{s}_rain_chance")),
                c.u32(&format!("{s}_snow_chance")),
                c.u32(&format!("{s}_storm_chance")),
            ]
        };
        (
            c.u32("zone"),
            [
                season("spring"),
                season("summer"),
                season("fall"),
                season("winter"),
            ],
        )
    }) {
        data.weather.insert(zone, chances);
    }
    let routes: AHashMap<u32, (i64, TransportRoute)> = rows(db, "SELECT * FROM transports", |c| {
        (
            c.u32("entry"),
            c.i64("build"),
            TransportRoute {
                entry: c.u32("entry"),
                name: c.string("name"),
                period: c.u32("period"),
            },
        )
    })
    .into_iter()
    .filter(|(_, build, _)| *build <= 5875)
    .fold(AHashMap::new(), |mut m, (entry, build, r)| {
        if m.get(&entry).is_none_or(|(b, _)| *b < build) {
            m.insert(entry, (build, r));
        }
        m
    });
    data.transports = routes.into_values().map(|(_, r)| r).collect();
    data.transports.sort_by_key(|r| r.entry);
    data.battlegrounds = latest_patch(rows(db, "SELECT * FROM battleground_template", |c| {
        (
            c.u32("id"),
            c.i64("patch"),
            BgTemplate {
                id: c.u32("id"),
                min_players: c.u32("min_players_per_team"),
                max_players: c.u32("max_players_per_team"),
                min_level: c.u32("min_level"),
                max_level: c.u32("max_level"),
                win_spells: [c.u32("alliance_win_spell"), c.u32("horde_win_spell")],
                lose_spells: [c.u32("alliance_lose_spell"), c.u32("horde_lose_spell")],
                starts: [
                    c.u32("alliance_start_location"),
                    c.u32("horde_start_location"),
                ],
            },
        )
    }));
    let npc = &mut data.npc;
    for (trigger, quest) in rows(db, "SELECT * FROM areatrigger_involvedrelation", |c| {
        (c.u32("id"), c.u32("quest"))
    }) {
        npc.area_trigger_quests.insert(trigger, quest);
    }
    for id in rows(db, "SELECT * FROM areatrigger_tavern", |c| c.u32("id")) {
        npc.taverns.insert(id);
    }
    for q in npc.quests.values() {
        if q.prev_quest > 0 {
            npc.quests_after
                .entry(q.prev_quest as u32)
                .or_default()
                .push(q.id);
        }
    }
}

fn load_spells(db: &Connection, dbc: Option<&Connection>, data: &mut GameData) {
    use super::spells::*;
    let s = &mut data.spells;
    let mut best: AHashMap<u32, (i64, SpellTemplate)> = AHashMap::new();
    for (build, t) in rows(
        db,
        "SELECT * FROM spell_template WHERE build <= 5875",
        |c| {
            let effect = |i: usize| {
                let n = i + 1;
                SpellEffect {
                    effect: c.u32(&format!("effect{n}")),
                    die_sides: c.i32(&format!("effectDieSides{n}")),
                    base_dice: c.i32(&format!("effectBaseDice{n}")),
                    dice_per_level: c.f32(&format!("effectDicePerLevel{n}")),
                    real_points_per_level: c.f32(&format!("effectRealPointsPerLevel{n}")),
                    base_points: c.i32(&format!("effectBasePoints{n}")),
                    bonus_coefficient: c.f32(&format!("effectBonusCoefficient{n}")),
                    mechanic: c.u32(&format!("effectMechanic{n}")),
                    target_a: c.u32(&format!("effectImplicitTargetA{n}")),
                    target_b: c.u32(&format!("effectImplicitTargetB{n}")),
                    radius_index: c.u32(&format!("effectRadiusIndex{n}")),
                    aura: c.u32(&format!("effectApplyAuraName{n}")),
                    amplitude: c.u32(&format!("effectAmplitude{n}")),
                    multiple_value: c.f32(&format!("effectMultipleValue{n}")),
                    chain_target: c.u32(&format!("effectChainTarget{n}")),
                    item_type: c.i64(&format!("effectItemType{n}")) as u64,
                    misc_value: c.i32(&format!("effectMiscValue{n}")),
                    trigger_spell: c.u32(&format!("effectTriggerSpell{n}")),
                    points_per_combo_point: c.f32(&format!("effectPointsPerComboPoint{n}")),
                }
            };
            let t = SpellTemplate {
                id: c.u32("entry"),
                school: c.u32("school"),
                category: c.u32("category"),
                dispel: c.u32("dispel"),
                mechanic: c.u32("mechanic"),
                attributes: [
                    c.u32("attributes"),
                    c.u32("attributesEx"),
                    c.u32("attributesEx2"),
                    c.u32("attributesEx3"),
                    c.u32("attributesEx4"),
                ],
                stances: c.u32("stances"),
                targets: c.u32("targets"),
                casting_time_index: c.u32("castingTimeIndex"),
                recovery_time: c.u32("recoveryTime"),
                category_recovery_time: c.u32("categoryRecoveryTime"),
                interrupt_flags: c.u32("interruptFlags"),
                aura_interrupt_flags: c.u32("auraInterruptFlags"),
                channel_interrupt_flags: c.u32("channelInterruptFlags"),
                proc_flags: c.u32("procFlags"),
                proc_chance: c.u32("procChance"),
                proc_charges: c.u32("procCharges"),
                max_level: c.u32("maxLevel"),
                base_level: c.u32("baseLevel"),
                spell_level: c.u32("spellLevel"),
                duration_index: c.u32("durationIndex"),
                power_type: c.u32("powerType"),
                mana_cost: c.u32("manaCost"),
                mana_cost_per_level: c.u32("manCostPerLevel"),
                mana_per_second: c.u32("manaPerSecond"),
                mana_cost_percentage: c.u32("manaCostPercentage"),
                range_index: c.u32("rangeIndex"),
                speed: c.f32("speed"),
                stack_amount: c.u32("stackAmount"),
                reagents: std::array::from_fn(|i| {
                    (
                        c.u32(&format!("reagent{}", i + 1)),
                        c.u32(&format!("reagentCount{}", i + 1)),
                    )
                }),
                equipped_item_class: c.i32("equippedItemClass"),
                equipped_item_subclass_mask: c.i32("equippedItemSubClassMask"),
                equipped_item_inventory_type_mask: c.i32("equippedItemInventoryTypeMask"),
                stances_not: c.u32("stancesNot"),
                target_creature_type: c.u32("targetCreatureType"),
                requires_spell_focus: c.u32("requiresSpellFocus"),
                caster_aura_state: c.u32("casterAuraState"),
                target_aura_state: c.u32("targetAuraState"),
                mana_per_second_per_level: c.u32("manaPerSecondPerLevel"),
                totems: [c.u32("totem1"), c.u32("totem2")],
                max_affected_targets: c.u32("maxAffectedTargets"),
                max_target_level: c.u32("maxTargetLevel"),
                spell_icon: c.u32("spellIconId"),
                custom_flags: c.u32("customFlags"),
                script_name: c.string("script_name"),
                effects: [effect(0), effect(1), effect(2)],
                spell_visual: c.u32("spellVisual1"),
                name: c.string("name"),
                rank: c.string("nameSubtext"),
                start_recovery_category: c.u32("startRecoveryCategory"),
                start_recovery_time: c.u32("startRecoveryTime"),
                family_name: c.u32("spellFamilyName"),
                family_flags: c.i64("spellFamilyFlags") as u64,
                dmg_class: c.u32("dmgClass"),
                prevention_type: c.u32("preventionType"),
                dmg_multiplier: [
                    c.f32("dmgMultiplier1"),
                    c.f32("dmgMultiplier2"),
                    c.f32("dmgMultiplier3"),
                ],
            };
            (c.i64("build"), t)
        },
    ) {
        match best.get(&t.id) {
            Some((b, _)) if *b >= build => {}
            _ => {
                best.insert(t.id, (build, t));
            }
        }
    }
    s.spells = best.into_iter().map(|(k, (_, t))| (k, t)).collect();
    apply_spell_mods(db, &mut s.spells);
    load_spell_tables(db, s);
    crate::game::map::spells::class_scripts::patch(s);
    for (id, map, x, y, z, o) in rows(
        db,
        "SELECT * FROM spell_target_position WHERE build_min <= 5875 AND build_max >= 5875",
        |c| {
            (
                c.u32("id"),
                c.u32("target_map"),
                c.f32("target_position_x"),
                c.f32("target_position_y"),
                c.f32("target_position_z"),
                c.f32("target_orientation"),
            )
        },
    ) {
        s.target_positions.insert(id, (map, x, y, z, o));
    }
    for (list, spells) in rows(db, "SELECT * FROM creature_spells", |c| {
        let spells: Vec<CreatureSpell> = (1..=8)
            .map(|n| {
                let ms = |column: &str| c.u32(&format!("{column}_{n}")).saturating_mul(1000);
                CreatureSpell {
                    spell: c.u32(&format!("spellId_{n}")),
                    probability: c.u32(&format!("probability_{n}")),
                    target: c.u32(&format!("castTarget_{n}")),
                    target_param1: c.u32(&format!("targetParam1_{n}")),
                    target_param2: c.u32(&format!("targetParam2_{n}")),
                    flags: c.u32(&format!("castFlags_{n}")),
                    delay_initial: (ms("delayInitialMin"), ms("delayInitialMax")),
                    delay_repeat: (ms("delayRepeatMin"), ms("delayRepeatMax")),
                    script: c.u32(&format!("scriptId_{n}")),
                }
            })
            .filter(|s| s.spell != 0)
            .collect();
        (c.u32("entry"), spells)
    }) {
        s.creature_spells.insert(list, spells);
    }
    let Some(dbc) = dbc else {
        s.inherit_proc_events();
        s.inherit_groups();
        return;
    };
    for (id, base) in rows(dbc, "SELECT * FROM SpellCastTimes", |c| {
        (c.u32("id"), c.i32("base"))
    }) {
        s.cast_times.insert(id, base);
    }
    for (id, d) in rows(dbc, "SELECT * FROM SpellDuration", |c| {
        (
            c.u32("id"),
            SpellDuration {
                base: c.i32("duration"),
                per_level: c.i32("duration_per_level"),
                max: c.i32("max_duration"),
            },
        )
    }) {
        s.durations.insert(id, d);
    }
    for (id, r) in rows(dbc, "SELECT * FROM SpellRange", |c| {
        (c.u32("id"), (c.f32("range_min"), c.f32("range_max")))
    }) {
        s.ranges.insert(id, r);
    }
    for (id, r) in rows(dbc, "SELECT * FROM SpellRadius", |c| {
        (c.u32("id"), c.f32("radius"))
    }) {
        s.radii.insert(id, r);
    }
    for t in rows(dbc, "SELECT * FROM Talent", |c| TalentInfo {
        id: c.u32("id"),
        tab: c.u32("tab"),
        row: c.u32("tier"),
        column: c.u32("column_index"),
        ranks: std::array::from_fn(|i| c.u32(&format!("spell_rank_{i}"))),
        depends_on: c.u32("prereq_talents_0"),
        depends_rank: c.u32("prereq_ranks_0"),
    }) {
        s.talents.insert(t.id, t);
    }
    for (tab, classes) in rows(dbc, "SELECT * FROM TalentTab", |c| {
        (c.u32("id"), c.u32("class_mask"))
    }) {
        s.talent_tab_classes.insert(tab, classes);
    }
    for a in rows(dbc, "SELECT * FROM SkillLineAbility", |c| SkillAbility {
        skill: c.u32("skill_line"),
        spell: c.u32("spell"),
        race_mask: c.u32("race_mask"),
        class_mask: c.u32("class_mask"),
        min_rank: c.u32("min_skill_line_rank"),
        superseded_by: c.u32("superseded_by"),
        acquire: c.u32("acquire_method"),
        trivial_high: c.u32("trivial_skill_line_rank_high"),
        trivial_low: c.u32("trivial_skill_line_rank_low"),
    }) {
        s.spell_skills.insert(a.spell, a.skill);
        s.skill_abilities.entry(a.spell).or_default().push(a);
    }
    for (skill, info) in rows(dbc, "SELECT * FROM SkillRaceClassInfo", |c| {
        (
            c.u32("skill_line"),
            (c.u32("race_mask"), c.u32("class_mask"), c.u32("flags")),
        )
    }) {
        s.skill_race_class.entry(skill).or_default().push(info);
    }
    for e in rows(dbc, "SELECT * FROM SpellItemEnchantment", |c| Enchantment {
        id: c.u32("id"),
        effects: std::array::from_fn(|i| {
            (
                c.u32(&format!("enchantment_type_{i}")),
                c.i32(&format!("effect_points_min_{i}")),
                c.i32(&format!("effect_points_max_{i}")),
                c.u32(&format!("effect_arg_{i}")),
            )
        }),
        name: c.string("name_en_gb"),
    }) {
        s.enchantments.insert(e.id, e);
    }
    s.inherit_proc_events();
    s.inherit_groups();
    s.index_talent_lines();
}

/// vmangos's corrections to `spell_template` (`spell_mod` and
/// `spell_effect_mod`): -1 leaves a value as it is.
fn apply_spell_mods(db: &Connection, spells: &mut AHashMap<u32, super::spells::SpellTemplate>) {
    let mods = rows(db, "SELECT * FROM spell_mod", |c| {
        let v = |n: &str| c.i64(n);
        (
            c.u32("Id"),
            [
                v("procChance"),
                v("procFlags"),
                v("procCharges"),
                v("DurationIndex"),
                v("Category"),
                v("CastingTimeIndex"),
                v("StackAmount"),
                v("manaCost"),
                v("Attributes"),
                v("AttributesEx"),
                v("AttributesEx2"),
                v("AttributesEx3"),
                v("AttributesEx4"),
                v("InterruptFlags"),
                v("AuraInterruptFlags"),
                v("ChannelInterruptFlags"),
                v("Dispel"),
                v("Stances"),
                v("StancesNot"),
                v("ManaCostPercentage"),
                v("StartRecoveryCategory"),
                v("StartRecoveryTime"),
                v("MaxAffectedTargets"),
                v("MaxTargetLevel"),
                v("DmgClass"),
                v("rangeIndex"),
                v("RecoveryTime"),
                v("CategoryRecoveryTime"),
                v("SpellFamilyName"),
                v("SpellFamilyFlags"),
                v("Mechanic"),
                v("EquippedItemClass"),
            ],
        )
    });
    for (id, m) in mods {
        let Some(t) = spells.get_mut(&id) else {
            continue;
        };
        let set = |dst: &mut u32, v: i64| {
            if v != -1 {
                *dst = v.max(0) as u32;
            }
        };
        set(&mut t.proc_chance, m[0]);
        set(&mut t.proc_flags, m[1]);
        set(&mut t.proc_charges, m[2]);
        set(&mut t.duration_index, m[3]);
        set(&mut t.category, m[4]);
        set(&mut t.casting_time_index, m[5]);
        set(&mut t.stack_amount, m[6]);
        set(&mut t.mana_cost, m[7]);
        for w in 0..5 {
            if m[8 + w] != -1 {
                t.attributes[w] = m[8 + w] as u32;
            }
        }
        set(&mut t.interrupt_flags, m[13]);
        set(&mut t.aura_interrupt_flags, m[14]);
        set(&mut t.channel_interrupt_flags, m[15]);
        set(&mut t.dispel, m[16]);
        set(&mut t.stances, m[17]);
        set(&mut t.stances_not, m[18]);
        set(&mut t.mana_cost_percentage, m[19]);
        set(&mut t.start_recovery_category, m[20]);
        set(&mut t.start_recovery_time, m[21]);
        set(&mut t.max_affected_targets, m[22]);
        set(&mut t.max_target_level, m[23]);
        set(&mut t.dmg_class, m[24]);
        set(&mut t.range_index, m[25]);
        set(&mut t.recovery_time, m[26]);
        set(&mut t.category_recovery_time, m[27]);
        set(&mut t.family_name, m[28]);
        // Family flags of 0 mean no change, as most rows have.
        if m[29] != -1 && m[29] != 0 {
            t.family_flags = m[29] as u64;
        }
        set(&mut t.mechanic, m[30]);
        if m[31] != -1 {
            t.equipped_item_class = m[31] as i32;
        }
    }
    let effect_mods = rows(db, "SELECT * FROM spell_effect_mod", |c| {
        (
            c.u32("Id"),
            c.u32("EffectIndex") as usize,
            [
                c.i64("Effect"),
                c.i64("EffectDieSides"),
                c.i64("EffectBaseDice"),
                c.i64("EffectBasePoints"),
                c.i64("EffectAmplitude"),
                c.i64("EffectChainTarget"),
                c.i64("EffectMechanic"),
                c.i64("EffectImplicitTargetA"),
                c.i64("EffectImplicitTargetB"),
                c.i64("EffectRadiusIndex"),
                c.i64("EffectApplyAuraName"),
                c.i64("EffectItemType"),
                c.i64("EffectMiscValue"),
                c.i64("EffectTriggerSpell"),
            ],
            [
                c.f32("EffectDicePerLevel"),
                c.f32("EffectRealPointsPerLevel"),
                c.f32("EffectPointsPerComboPoint"),
                c.f32("EffectMultipleValue"),
            ],
        )
    });
    for (id, i, v, f) in effect_mods {
        let Some(e) = spells.get_mut(&id).and_then(|t| t.effects.get_mut(i)) else {
            continue;
        };
        let set = |dst: &mut u32, v: i64| {
            if v != -1 {
                *dst = v.max(0) as u32;
            }
        };
        set(&mut e.effect, v[0]);
        if v[1] != -1 {
            e.die_sides = v[1] as i32;
        }
        if v[2] != -1 {
            e.base_dice = v[2] as i32;
        }
        if v[3] != -1 {
            e.base_points = v[3] as i32;
        }
        set(&mut e.amplitude, v[4]);
        set(&mut e.chain_target, v[5]);
        set(&mut e.mechanic, v[6]);
        set(&mut e.target_a, v[7]);
        set(&mut e.target_b, v[8]);
        set(&mut e.radius_index, v[9]);
        set(&mut e.aura, v[10]);
        if v[11] != -1 {
            e.item_type = v[11] as u64;
        }
        if v[12] != -1 {
            e.misc_value = v[12] as i32;
        }
        set(&mut e.trigger_spell, v[13]);
        if f[0] != -1.0 {
            e.dice_per_level = f[0];
        }
        if f[1] != -1.0 {
            e.real_points_per_level = f[1];
        }
        if f[2] != -1.0 {
            e.points_per_combo_point = f[2];
        }
        if f[3] != -1.0 {
            e.multiple_value = f[3];
        }
    }
}

/// The `spell_*` tables beside the templates.
fn load_spell_tables(db: &Connection, s: &mut super::spells::SpellData) {
    use super::spells::*;
    let in_build = "WHERE build_min <= 5875 AND build_max >= 5875";
    for (id, p) in rows(
        db,
        &format!("SELECT * FROM spell_proc_event {in_build}"),
        |c| {
            (
                c.u32("entry"),
                ProcEvent {
                    school_mask: c.u32("SchoolMask"),
                    family: c.u32("SpellFamilyName"),
                    family_mask: [
                        c.i64("SpellFamilyMask0") as u64,
                        c.i64("SpellFamilyMask1") as u64,
                        c.i64("SpellFamilyMask2") as u64,
                    ],
                    flags: c.u32("procFlags"),
                    ex: c.u32("procEx"),
                    ppm: c.f32("ppmRate"),
                    chance: c.f32("CustomChance"),
                    cooldown: c.u32("Cooldown"),
                },
            )
        },
    ) {
        s.proc_events.insert(id, p);
    }
    for (id, r) in rows(db, &format!("SELECT * FROM spell_chain {in_build}"), |c| {
        (
            c.u32("spell_id"),
            SpellRank {
                prev: c.u32("prev_spell"),
                first: c.u32("first_spell"),
                rank: c.u32("rank"),
                req: c.u32("req_spell"),
            },
        )
    }) {
        s.ranks.insert(id, r);
    }
    for (id, t) in rows(db, &format!("SELECT * FROM spell_threat {in_build}"), |c| {
        (c.u32("entry"), (c.i32("threat"), c.f32("multiplier")))
    }) {
        s.threat.insert(id, t);
    }
    for (group, order, spell) in rows(db, &format!("SELECT * FROM spell_group {in_build}"), |c| {
        (
            c.u32("group_id"),
            c.u32("group_spell_id"),
            c.u32("spell_id"),
        )
    }) {
        if spell != 0 {
            s.groups.entry(spell).or_default().push(group);
            s.group_orders.insert((group, spell), order);
        }
    }
    for (group, rule) in rows(db, "SELECT * FROM spell_group_stack_rules", |c| {
        (c.u32("group_id"), c.u32("stack_rule"))
    }) {
        s.group_rules.insert(group, rule);
    }
    for (id, mask) in rows(db, &format!("SELECT * FROM spell_elixir {in_build}"), |c| {
        (c.u32("entry"), c.u32("mask"))
    }) {
        s.elixirs.insert(id, mask);
    }
    for (id, ppm) in rows(db, "SELECT * FROM spell_proc_item_enchant", |c| {
        (c.u32("entry"), c.f32("ppmRate"))
    }) {
        s.enchant_ppm.insert(id, ppm);
    }
    for (id, degrees) in rows(db, "SELECT * FROM spell_cone", |c| {
        (c.u32("entry"), c.i32("cone_degrees"))
    }) {
        s.cones.insert(id, degrees);
    }
    for (id, kind, entry, condition) in rows(
        db,
        &format!("SELECT * FROM spell_script_target {in_build}"),
        |c| {
            (
                c.u32("entry"),
                c.u32("type"),
                c.u32("targetEntry"),
                c.u32("conditionId"),
            )
        },
    ) {
        s.script_targets.entry(id).or_default().push((kind, entry));
        if condition != 0 {
            s.script_target_conditions
                .insert((id, kind, entry), condition);
        }
    }
    for (spell, learns) in rows(db, "SELECT * FROM spell_learn_spell", |c| {
        (c.u32("entry"), c.u32("SpellID"))
    }) {
        s.learn_with.entry(spell).or_default().push(learns);
    }
}

fn load_players(db: &Connection, data: &mut GameData) {
    for (key, s) in rows(db, "SELECT * FROM playercreateinfo", |c| {
        (
            (c.u8("race"), c.u8("class")),
            StartInfo {
                map: c.u32("map"),
                zone: c.u32("zone"),
                x: c.f32("position_x"),
                y: c.f32("position_y"),
                z: c.f32("position_z"),
                orientation: c.f32("orientation"),
                ..Default::default()
            },
        )
    }) {
        data.start.insert(key, s);
    }
    for (key, item, count) in rows(db, "SELECT * FROM playercreateinfo_item", |c| {
        (
            (c.u8("race"), c.u8("class")),
            c.u32("itemid"),
            c.u32("amount"),
        )
    }) {
        if let Some(s) = data.start.get_mut(&key) {
            s.items.push((item, u8::MAX, count.max(1)));
        }
    }
    for (key, spell) in rows(
        db,
        "SELECT * FROM playercreateinfo_spell WHERE build_min <= 5875 AND build_max >= 5875",
        |c| ((c.u8("race"), c.u8("class")), c.u32("spell")),
    ) {
        if let Some(s) = data.start.get_mut(&key) {
            s.spells.push(spell);
        }
    }
    for (key, action) in rows(db, "SELECT * FROM playercreateinfo_action", |c| {
        (
            (c.u8("race"), c.u8("class")),
            (c.u8("button"), c.u32("action") as u16, c.u8("type")),
        )
    }) {
        if let Some(s) = data.start.get_mut(&key) {
            s.actions.push(action);
        }
    }
    for (key, stats) in rows(db, "SELECT * FROM player_levelstats", |c| {
        (
            (c.u8("race"), c.u8("class"), c.u8("level")),
            [
                c.u32("str"),
                c.u32("agi"),
                c.u32("sta"),
                c.u32("inte"),
                c.u32("spi"),
            ],
        )
    }) {
        data.player_level_stats.insert(key, stats);
    }
    for (key, hp, mana) in rows(db, "SELECT * FROM player_classlevelstats", |c| {
        (
            (c.u8("class"), c.u8("level")),
            c.u32("basehp"),
            c.u32("basemana"),
        )
    }) {
        data.player_class_stats.insert(key, (hp, mana));
    }
    for (level, xp) in rows(db, "SELECT * FROM player_xp_for_level", |c| {
        (c.u8("lvl"), c.u32("xp_for_next_level"))
    }) {
        data.xp_for_level.insert(level, xp);
    }
}

/// What pets are made of.
fn load_pets(db: &Connection, dbc: Option<&Connection>, data: &mut GameData) {
    use super::pets::*;
    let pets = &mut data.pets;
    for (key, level) in rows(db, "SELECT * FROM pet_levelstats", |c| {
        (
            (c.u32("entry"), c.u8("level")),
            PetLevel {
                health: c.u32("health"),
                mana: c.u32("mana"),
                armor: c.u32("armor"),
                damage: (c.f32("dmg_min"), c.f32("dmg_max")),
                stats: [
                    c.u32("strength"),
                    c.u32("agility"),
                    c.u32("stamina"),
                    c.u32("intellect"),
                    c.u32("spirit"),
                ],
            },
        )
    }) {
        pets.levels.insert(key, level);
    }
    for (entry, word, half) in rows(db, "SELECT * FROM pet_name_generation", |c| {
        (c.u32("entry"), c.string("word"), c.u8("half"))
    }) {
        pets.names.entry(entry).or_default().push((word, half));
    }
    for (entry, spells) in rows(
        db,
        "SELECT * FROM petcreateinfo_spell WHERE patch_min <= 10 AND patch_max >= 10",
        |c| {
            (
                c.u32("entry"),
                (1..=4)
                    .map(|i| c.u32(&format!("spell{i}")))
                    .filter(|s| *s != 0)
                    .collect::<Vec<u32>>(),
            )
        },
    ) {
        pets.create_spells.insert(entry, spells);
    }
    for (list, build, spells) in rows(
        db,
        "SELECT * FROM pet_spell_data WHERE build <= 5875",
        |c| {
            (
                c.u32("entry"),
                c.i64("build"),
                (1..=4)
                    .map(|i| c.u32(&format!("spell_id{i}")))
                    .filter(|s| *s != 0)
                    .collect::<Vec<u32>>(),
            )
        },
    ) {
        let _ = build;
        pets.spell_lists.insert(list, spells);
    }
    // Cooldowns are seconds, but for a few in milliseconds already (as
    // Thistle Tea reads them: 1000 or more).
    let ms = |v: u32| if v >= 1000 { v } else { v * 1000 };
    for (entry, c) in rows(
        db,
        "SELECT * FROM creature_charm_spells \
         WHERE patch_min <= 10 AND patch_max >= 10 AND availability > 0 AND slot <= 3 \
         ORDER BY entry, slot, spell_id",
        |c| {
            let (a, b) = (c.u32("cooldown_min"), c.u32("cooldown_max"));
            (
                c.u32("entry"),
                super::pets::CharmSpell {
                    slot: c.u32("slot"),
                    spell: c.u32("spell_id"),
                    availability: c.f32("availability"),
                    cooldown_min_ms: ms(a.min(b)),
                    cooldown_max_ms: ms(a.max(b)),
                },
            )
        },
    ) {
        pets.charm_spells.entry(entry).or_default().push(c);
    }
    for (entry, list) in rows(
        db,
        "SELECT entry, pet_spell_list_id FROM creature_template WHERE pet_spell_list_id > 0",
        |c| (c.u32("entry"), c.u32("pet_spell_list_id")),
    ) {
        pets.creature_spell_list.insert(entry, list);
    }
    for (entry, spell) in rows(
        db,
        "SELECT entry, totem_spell_id FROM creature_template WHERE totem_spell_id > 0",
        |c| (c.u32("entry"), c.u32("totem_spell_id")),
    ) {
        pets.totem_spells.insert(entry, spell);
    }
    pets.owner_auras = rows(db, "SELECT * FROM spell_pet_auras", |c| {
        (c.u32("spell"), c.u32("pet"), c.u32("aura"))
    });
    // The Grimoires: each teaches a demon a spell.
    let mut grimoires: Vec<u32> = data
        .items
        .values()
        .filter(|t| t.name.starts_with("Grimoire of "))
        .filter_map(|t| t.spells.iter().find(|s| s.id != 0).map(|s| s.id))
        .filter_map(|teach| {
            data.spells.get(teach).and_then(|s| {
                s.effects
                    .iter()
                    .find(|e| matches!(e.effect, 36 | 57) && e.trigger_spell != 0)
                    .map(|e| e.trigger_spell)
            })
        })
        .collect();
    grimoires.sort_unstable();
    grimoires.dedup();
    pets.grimoire_spells = grimoires;
    for t in data.spells.spells.values() {
        // Grimoires' `LEARN_PET_SPELL`, and hunters' `LEARN_SPELL` at the
        // pet.
        let teaches = |e: &&super::spells::SpellEffect| {
            e.trigger_spell != 0 && (e.effect == 57 || (e.effect == 36 && e.target_a == 5))
        };
        for e in t.effects.iter().filter(teaches) {
            pets.teaching.insert(e.trigger_spell, t.id);
        }
    }
    let Some(dbc) = dbc else {
        return;
    };
    // The export's CreatureFamily columns run one off: its "pet_food_mask"
    // and "pet_talent_type" hold the two skill lines, and its "category"
    // the food mask.
    for f in rows(dbc, "SELECT * FROM CreatureFamily", |c| Family {
        id: c.u32("id"),
        name: c.string("name_en_gb"),
        skills: [c.u32("pet_food_mask"), c.u32("pet_talent_type")],
        food_mask: c.u32("category"),
        min_scale: c.f32("min_scale"),
        max_scale: c.f32("max_scale"),
        min_scale_level: c.u32("min_scale_level"),
        max_scale_level: c.u32("max_scale_level"),
    }) {
        pets.families.insert(f.id, f);
    }
    let mut skills: Vec<u32> = pets.families.values().flat_map(|f| f.skills).collect();
    skills.extend([261, 270]);
    skills.retain(|s| *s != 0);
    skills.sort_unstable();
    skills.dedup();
    for a in rows(dbc, "SELECT * FROM SkillLineAbility", |c| PetAbility {
        spell: c.u32("spell"),
        skill: c.u32("skill_line"),
        acquire: c.u32("acquire_method"),
        superseded_by: c.u32("superseded_by"),
        cost: c.u32("num_skills_up"),
    }) {
        if skills.contains(&a.skill) {
            pets.abilities.entry(a.skill).or_default().push(a);
        }
    }
    let mut prices = rows(dbc, "SELECT * FROM StableSlotPrices", |c| {
        (c.u32("id"), c.u32("cost"))
    });
    prices.sort_unstable();
    pets.stable_prices = prices.into_iter().map(|(_, c)| c).collect();
}

/// Factions and reputation, page texts, mail templates, exploration XP
/// and meeting stones.
fn load_misc(db: &Connection, dbc: Option<&Connection>, data: &mut GameData) {
    use super::misc::*;
    if let Some(dbc) = dbc {
        for (race, gender, kind, variation, colour) in
            rows(dbc, "SELECT * FROM CharSections WHERE npc_only = 0", |c| {
                (
                    c.u8("race"),
                    c.u8("gender"),
                    c.u8("ty"),
                    c.u8("variation_index"),
                    c.u8("colour_index"),
                )
            })
        {
            data.appearances
                .add_section(race, gender, kind, variation, colour);
        }
        for (race, gender, variation) in rows(dbc, "SELECT * FROM CharacterFacialHairStyles", |c| {
            (c.u8("race"), c.u8("gender"), c.u8("variation_id"))
        }) {
            data.appearances.add_facial_hair(race, gender, variation);
        }
    }
    let m = &mut data.misc;
    if let Some(dbc) = dbc {
        for f in rows(dbc, "SELECT * FROM Faction", |c| FactionDef {
            id: c.u32("id"),
            index: c.i64("reputation_index") as u32,
            parent: c.u32("parent_faction"),
            name: c.string("name_en_gb"),
            variants: std::array::from_fn(|i| {
                (
                    c.i64(&format!("reputation_race_mask_{i}")) as u32,
                    c.i64(&format!("reputation_class_mask_{i}")) as u32,
                    c.i64(&format!("reputation_base_{i}")) as i32,
                    c.i64(&format!("reputation_flags_{i}")) as u32,
                )
            }),
        }) {
            if f.index < 64 {
                m.faction_by_index.insert(f.index, f.id);
                m.factions.insert(f.id, f);
            }
        }
        for (id, subject) in rows(dbc, "SELECT * FROM MailTemplate", |c| {
            (c.u32("id"), c.string("body_en_gb"))
        }) {
            // The DBC's one text is the mail's body, as Thistle Tea reads it.
            m.mail_templates.insert(id, (String::new(), subject));
        }
    }
    for (faction, spills) in rows(db, "SELECT * FROM reputation_spillover_template", |c| {
        let spills: Vec<Spillover> = (1..=4)
            .map(|i| Spillover {
                faction: c.u32(&format!("faction{i}")),
                rate: c.f32(&format!("rate_{i}")),
                max_rank: c.u32(&format!("rank_{i}")),
            })
            .filter(|s| s.faction != 0)
            .collect();
        (c.u32("faction"), spills)
    }) {
        m.spillovers.insert(faction, spills);
    }
    let mut best: AHashMap<u32, (u32, Vec<KillReputation>)> = AHashMap::new();
    for (creature, patch, rewards) in rows(db, "SELECT * FROM creature_onkill_reputation", |c| {
        let team_dependent = c.u32("TeamDependent") != 0;
        let rewards: Vec<KillReputation> = (1..=2)
            .map(|i| KillReputation {
                faction: c.u32(&format!("RewOnKillRepFaction{i}")),
                value: c.i32(&format!("RewOnKillRepValue{i}")),
                max_rank: c.u32(&format!("MaxStanding{i}")),
                team: if team_dependent { i as u8 } else { 0 },
                team_award: c.u32(&format!("IsTeamAward{i}")) != 0,
            })
            .filter(|r| r.faction != 0 && r.value != 0)
            .collect();
        (c.u32("creature_id"), c.u32("patch"), rewards)
    }) {
        if patch > 10 {
            continue;
        }
        match best.get(&creature) {
            Some((p, _)) if *p > patch => {}
            _ => {
                best.insert(creature, (patch, rewards));
            }
        }
    }
    m.kill_reputation = best.into_iter().map(|(k, (_, v))| (k, v)).collect();
    for (entry, text, next) in rows(db, "SELECT * FROM page_text", |c| {
        (c.u32("entry"), c.string("text"), c.u32("next_page"))
    }) {
        m.page_texts.insert(entry, (text, next));
    }
    for (id, body) in rows(db, "SELECT * FROM mail_text_template", |c| {
        (c.u32("entry"), c.string("content_default"))
    }) {
        let t = m.mail_templates.entry(id).or_default();
        if t.1.is_empty() {
            t.1 = body;
        }
    }
    for (entry, item, count) in rows(
        db,
        "SELECT * FROM mail_loot_template WHERE patch_min <= 10",
        |c| (c.u32("entry"), c.u32("item"), c.u32("mincountOrRef").max(1)),
    ) {
        if item != 0 {
            m.mail_items.entry(entry).or_default().push((item, count));
        }
    }
    for (quest, mail) in rows(
        db,
        "SELECT entry, RewMailTemplateId, RewMailDelaySecs, RewMailMoney FROM quest_template \
         WHERE RewMailTemplateId > 0",
        |c| {
            (
                c.u32("entry"),
                QuestMail {
                    template: c.u32("RewMailTemplateId"),
                    delay_secs: c.u32("RewMailDelaySecs"),
                    money: c.u32("RewMailMoney"),
                },
            )
        },
    ) {
        m.quest_mail.insert(quest, mail);
    }
    for (faction, rates) in rows(db, "SELECT * FROM reputation_reward_rate", |c| {
        (
            c.u32("faction"),
            (
                c.f32("quest_rate"),
                c.f32("creature_rate"),
                c.f32("spell_rate"),
            ),
        )
    }) {
        m.reward_rates.insert(faction, rates);
    }
    for (level, xp) in rows(db, "SELECT * FROM exploration_basexp", |c| {
        (c.u32("level"), c.u32("basexp"))
    }) {
        m.exploration_xp.insert(level, xp);
    }
    for (key, rate) in rows(db, "SELECT * FROM player_crit_per_agility", |c| {
        ((c.u32("class") as u8, c.u32("level") as u8), c.f32("rate"))
    }) {
        m.crit_per_agility.insert(key, rate);
    }
    for (entry, stone) in rows(
        db,
        "SELECT entry, data0, data1, data2 FROM gameobject_template WHERE type = 23",
        |c| {
            (
                c.u32("entry"),
                MeetingStone {
                    min_level: c.u32("data0"),
                    max_level: c.u32("data1"),
                    area: c.u32("data2"),
                },
            )
        },
    ) {
        m.meeting_stones.insert(entry, stone);
    }
    for (cinematic, point) in rows(db, "SELECT * FROM cinematic_waypoints", |c| {
        (
            c.u32("cinematic"),
            (
                c.u32("timer"),
                c.f32("position_x"),
                c.f32("position_y"),
                c.f32("position_z"),
            ),
        )
    }) {
        m.cinematic_waypoints
            .entry(cinematic)
            .or_default()
            .push(point);
    }
    for points in m.cinematic_waypoints.values_mut() {
        points.sort_by_key(|p| p.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client's own tables: the looks the bots that crashed a watching
    /// client had are refused, and the one the tests make every character
    /// with is good for every race.
    #[test]
    #[ignore = "extended"]
    fn the_looks_the_client_has_textures_for() {
        use crate::game::data::appearance::Look;
        let Some(d) = real_data() else {
            return;
        };
        let a = &d.appearances;
        assert!(a.is_loaded());
        let look = |skin, face, hair_style, hair_color, facial_hair| Look {
            skin,
            face,
            hair_style,
            hair_color,
            facial_hair,
        };
        assert!(!a.valid(7, 0, look(5, 2, 4, 2, 0)), "a gnome's sixth skin");
        assert!(
            !a.valid(6, 0, look(3, 3, 5, 4, 4)),
            "a tauren's fourth hair colour"
        );
        for race in [1, 2, 3, 4, 5, 6, 7, 8] {
            for gender in [0, 1] {
                assert!(
                    a.valid(race, gender, look(1, 1, 1, 1, 1)),
                    "{race} {gender}"
                );
                assert!(
                    a.valid(race, gender, look(4, 3, 4, 2, 4)),
                    "everyone's range, {race} {gender}"
                );
            }
        }
    }

    /// A clone made without Git LFS has pointers for databases: they are
    /// named as such (not "file is not a database" a query at a time), and
    /// the world is empty.
    #[test]
    fn a_git_lfs_pointer_is_named_and_the_world_is_empty() {
        #[derive(Clone, Default)]
        struct Log(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Log {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let dir = std::env::temp_dir().join(format!("wvs-lfs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pointer = "version https://git-lfs.github.com/spec/v1\n\
                       oid sha256:190cb3b341a8f6b5c1d7e0f2a9c4b3e6d5f8a7b0c9d2e1f4a3b6c5d8e7f0a9b2\n\
                       size 140783616\n";
        for db in ["vmangos.sqlite", "dbc.sqlite"] {
            std::fs::write(dir.join(db), pointer).unwrap();
        }
        assert!(lfs_pointer(&dir.join("vmangos.sqlite")));
        assert!(!lfs_pointer(&dir.join("nowhere.sqlite")));
        let log = Log::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let log = log.clone();
                move || log.clone()
            })
            .with_ansi(false)
            .finish();
        let data = tracing::subscriber::with_default(subscriber, || load(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(data.creature_templates.is_empty());
        let log = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        assert!(
            log.contains("vmangos.sqlite is a Git LFS pointer: run `git lfs pull`"),
            "{log}"
        );
        assert!(log.contains("dbc.sqlite is a Git LFS pointer"), "{log}");
        assert!(!log.contains("not a database"), "{log}");
    }

    /// The real database, where it is, isn't a Git LFS pointer.
    #[test]
    #[ignore = "extended"]
    fn the_real_database_is_not_a_git_lfs_pointer() {
        let real = crate::game::harness::real_data("db").join("vmangos.sqlite");
        if real.exists() {
            assert!(!lfs_pointer(&real), "{} is a pointer", real.display());
        }
    }

    /// Against the real databases, when they are here (they aren't in CI).
    #[test]
    #[ignore = "extended"]
    fn vmangos_loads_with_patch_ten_spawns() {
        let Some(data) = real_data() else {
            return;
        };
        let creatures: usize = data.creature_spawns.values().map(Vec::len).sum();
        let objects: usize = data.gameobject_spawns.values().map(Vec::len).sum();
        assert!(creatures > 60_000, "{creatures}");
        assert!(objects > 50_000, "{objects}");
        let hogger = data.creature_template(448).expect("Hogger");
        assert_eq!(hogger.name, "Hogger");
        assert!(hogger.health_max > 0);
        let start = data.start_info(1, 1).unwrap();
        assert!(
            start.items.iter().any(|i| i.0 == 25),
            "a human warrior's sword"
        );
        assert!(
            data.areas
                .get(&2177)
                .is_some_and(|a| a.flags & AREA_FLAG_ARENA != 0)
        );
    }

    /// Every spell a trainer teaches up to level 20 has only effects,
    /// auras and targets the server knows.
    #[test]
    #[ignore = "extended"]
    fn trainers_teach_only_what_the_server_can_cast_to_twenty() {
        use crate::game::map::{auras, spells};
        let Some(data) = real_data() else {
            return;
        };
        assert!(
            data.spells.spells.len() > 20_000,
            "{}",
            data.spells.spells.len()
        );
        let mut taught: Vec<u32> = data
            .npc
            .trainers
            .values()
            .chain(data.npc.trainer_templates.values())
            .flatten()
            .filter(|t| t.required_level <= 20)
            .map(|t| spells::learns(data, t.spell))
            .collect();
        taught.sort_unstable();
        taught.dedup();
        assert!(taught.len() > 300, "{}", taught.len());
        let mut missing = Vec::new();
        for id in taught {
            let Some(s) = data.spells.get(id) else {
                missing.push(format!("{id}: no template"));
                continue;
            };
            for e in s.effects.iter().filter(|e| e.effect != 0) {
                if !spells::effect_supported(e.effect) {
                    missing.push(format!("{id} {}: effect {}", s.name, e.effect));
                }
                if matches!(e.effect, 6 | 27 | 35) && !auras::supported(e.aura) {
                    missing.push(format!("{id} {}: aura {}", s.name, e.aura));
                }
                for t in [e.target_a, e.target_b] {
                    if !spells::target_supported(t) {
                        missing.push(format!("{id} {}: target {t}", s.name));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "{}", missing.join("\n"));
    }

    /// The boats, events and weather of the real data.
    #[test]
    #[ignore = "extended"]
    fn boats_events_and_weather_load() {
        let Some(data) = real_data() else {
            return;
        };
        let routes: Vec<_> = data
            .transports
            .iter()
            .filter_map(|t| {
                let nodes = data
                    .npc
                    .taxi_path_nodes
                    .get(&data.gameobject_templates.get(&t.entry)?.data[0])?;
                crate::game::transports::Route::new(t.entry, t.period, nodes)
            })
            .collect();
        assert!(routes.len() >= 8, "{}", routes.len());
        // The Booty Bay boat crosses between the continents.
        let ratchet = routes
            .iter()
            .find(|r| r.entry == 20808)
            .expect("Ratchet to Booty Bay");
        let maps = ratchet.maps();
        assert_eq!(maps.into_iter().collect::<Vec<_>>(), vec![0, 1]);
        assert!(data.events.len() > 20, "{}", data.events.len());
        assert!(data.event_creatures.len() > 1000);
        assert!(data.weather.len() > 20);
        assert_eq!(data.battlegrounds[&2].max_players, 10);
    }

    #[test]
    fn columns_are_read_by_name_whatever_their_case_and_leniently() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE a (Entry INTEGER, speed REAL, name TEXT, count TEXT, raw BLOB);
             INSERT INTO a VALUES (7, 2.75, 'Hogger', ' 12 ', x'ff');
             CREATE TABLE b (entry INTEGER, other INTEGER);
             INSERT INTO b VALUES (99, 3);",
        )
        .unwrap();
        let got = rows(&db, "SELECT * FROM a, b", |c| {
            (
                (c.u32("Entry"), c.u32("entry"), c.u32("ENTRY")),
                (c.i64("speed"), c.f32("speed"), c.u8("missing")),
                (c.string("name"), c.string("entry"), c.string("speed")),
                (c.i32("count"), c.f32("count"), c.i64("raw")),
                (c.value("raw"), c.value("nothing"), c.u32("other")),
            )
        });
        assert_eq!(
            got,
            vec![(
                // Of two columns named alike, the first.
                (7, 7, 7),
                (2, 2.75, 0),
                ("Hogger".to_owned(), "7".to_owned(), String::new()),
                (12, 12.0, 0),
                (Value::Blob(vec![0xff]), Value::Null, 3),
            )]
        );
    }

    #[test]
    fn the_latest_patch_up_to_ten_wins() {
        let rows = vec![(1, 0, "a"), (1, 5, "b"), (1, 11, "c"), (2, 3, "d")];
        let m = latest_patch(rows);
        assert_eq!(m[&1], "b");
        assert_eq!(m[&2], "d");
    }
}
