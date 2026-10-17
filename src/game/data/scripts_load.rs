//! Loading vmangos's scripting tables into [`super::scripts::ScriptData`].

use rusqlite::Connection;
use tracing::info;

use super::GameData;
use super::load::{Cols, rows};
use super::scripts::*;

fn step(c: &Cols) -> ScriptStep {
    ScriptStep {
        id: c.u32("id"),
        delay_ms: c.u32("delay").saturating_mul(1000),
        priority: c.u32("priority"),
        command: c.u32("command"),
        datalong: [
            c.u32("datalong"),
            c.u32("datalong2"),
            c.u32("datalong3"),
            c.u32("datalong4"),
        ],
        dataint: [
            c.i32("dataint"),
            c.i32("dataint2"),
            c.i32("dataint3"),
            c.i32("dataint4"),
        ],
        target_type: c.u32("target_type"),
        target_param1: c.u32("target_param1"),
        target_param2: c.u32("target_param2"),
        data_flags: c.u32("data_flags"),
        x: c.f32("x"),
        y: c.f32("y"),
        z: c.f32("z"),
        o: c.f32("o"),
        condition: c.u32("condition_id"),
    }
}

fn table(db: &Connection, name: &str) -> ahash::AHashMap<u32, Vec<ScriptStep>> {
    let mut out: ahash::AHashMap<u32, Vec<ScriptStep>> = ahash::AHashMap::new();
    for s in rows(db, &format!("SELECT * FROM {name}"), step) {
        out.entry(s.id).or_default().push(s);
    }
    for steps in out.values_mut() {
        steps.sort_by_key(|s| (s.delay_ms, s.priority));
    }
    out
}

pub fn load(db: &Connection, data: &mut GameData) {
    let s = &mut data.scripts;
    s.ai_scripts = table(db, "creature_ai_scripts");
    s.generic = table(db, "generic_scripts");
    s.event = table(db, "event_scripts");
    s.spell = table(db, "spell_scripts");
    s.quest_start = table(db, "quest_start_scripts");
    s.quest_end = table(db, "quest_end_scripts");
    s.gossip = table(db, "gossip_scripts");
    s.movement = table(db, "creature_movement_scripts");
    s.gameobject = table(db, "gameobject_scripts");
    s.areatrigger = table(db, "areatrigger_scripts");
    s.creature_spells = table(db, "creature_spells_scripts");
    for e in rows(db, "SELECT * FROM creature_ai_events ORDER BY id", |c| {
        AiEvent {
            id: c.u32("id"),
            creature: c.u32("creature_id"),
            condition: c.u32("condition_id"),
            kind: c.u32("event_type"),
            inverse_phase_mask: c.u32("event_inverse_phase_mask"),
            chance: c.u32("event_chance"),
            flags: c.u32("event_flags"),
            params: [
                c.i32("event_param1"),
                c.i32("event_param2"),
                c.i32("event_param3"),
                c.i32("event_param4"),
            ],
            actions: [
                c.u32("action1_script"),
                c.u32("action2_script"),
                c.u32("action3_script"),
            ],
        }
    }) {
        s.ai_events.entry(e.creature).or_default().push(e);
    }
    for c in rows(db, "SELECT * FROM conditions", |c| Condition {
        entry: c.u32("condition_entry"),
        kind: c.i32("type"),
        values: [
            c.i32("value1"),
            c.i32("value2"),
            c.i32("value3"),
            c.i32("value4"),
        ],
        flags: c.u32("flags"),
    }) {
        s.conditions.insert(c.entry, c);
    }
    for (quest, start, complete) in rows(
        db,
        "SELECT entry, StartScript, CompleteScript FROM quest_template",
        |c| {
            (
                c.u32("entry"),
                c.u32("StartScript"),
                c.u32("CompleteScript"),
            )
        },
    ) {
        if start != 0 {
            s.quest_start_script.insert(quest, start);
        }
        if complete != 0 {
            s.quest_complete_script.insert(quest, complete);
        }
    }
    for (menu, script) in rows(db, "SELECT entry, script_id FROM gossip_menu", |c| {
        (c.u32("entry"), c.u32("script_id"))
    }) {
        if script != 0 {
            s.gossip_menu_script.insert(menu, script);
        }
    }
    for (menu, option, script) in rows(
        db,
        "SELECT menu_id, id, action_script_id FROM gossip_menu_option",
        |c| (c.u32("menu_id"), c.u32("id"), c.u32("action_script_id")),
    ) {
        if script != 0 {
            s.gossip_option_script.insert((menu, option), script);
        }
    }
    for (leader, m) in rows(db, "SELECT * FROM creature_groups", |c| {
        (
            c.u32("leader_guid"),
            GroupMember {
                guid: c.u32("member_guid"),
                distance: c.f32("dist"),
                angle: c.f32("angle"),
                flags: c.u32("flags"),
            },
        )
    }) {
        if m.guid != leader {
            s.group_of.insert(m.guid, leader);
        }
        s.group_of.insert(leader, leader);
        s.groups.entry(leader).or_default().push(m);
    }
    for (guid, master, flag) in rows(db, "SELECT * FROM creature_linking", |c| {
        (c.u32("guid"), c.u32("master_guid"), c.u32("flag"))
    }) {
        s.links.insert(guid, (master, flag));
    }
    for (entry, t) in rows(db, "SELECT * FROM creature_linking_template", |c| {
        (
            c.u32("entry"),
            (
                c.u32("map"),
                c.u32("master_entry"),
                c.u32("flag"),
                c.f32("search_range"),
            ),
        )
    }) {
        s.link_templates.insert(entry, t);
    }
    // Pools, in the patch.
    let in_patch = "patch_min <= 10 AND patch_max >= 10";
    for (entry, max) in rows(
        db,
        &format!("SELECT * FROM pool_template WHERE {in_patch}"),
        |c| (c.u32("entry"), c.u32("max_limit")),
    ) {
        s.pools.insert(
            entry,
            Pool {
                entry,
                max_limit: max,
                ..Default::default()
            },
        );
    }
    for (guid, pool, chance) in rows(
        db,
        &format!("SELECT * FROM pool_creature WHERE {in_patch}"),
        |c| (c.u32("guid"), c.u32("pool_entry"), c.f32("chance")),
    ) {
        if let Some(p) = s.pools.get_mut(&pool) {
            p.creatures.push((guid, chance));
            s.creature_pool.insert(guid, pool);
        }
    }
    for (guid, pool, chance) in rows(
        db,
        &format!("SELECT * FROM pool_gameobject WHERE {in_patch}"),
        |c| (c.u32("guid"), c.u32("pool_entry"), c.f32("chance")),
    ) {
        if let Some(p) = s.pools.get_mut(&pool) {
            p.objects.push((guid, chance));
            s.object_pool.insert(guid, pool);
        }
    }
    for (pool, mother, chance) in rows(db, "SELECT * FROM pool_pool", |c| {
        (c.u32("pool_id"), c.u32("mother_pool"), c.f32("chance"))
    }) {
        if s.pools.contains_key(&pool)
            && let Some(m) = s.pools.get_mut(&mother)
        {
            m.pools.push((pool, chance));
            if let Some(p) = s.pools.get_mut(&pool) {
                p.mother = mother;
            }
        }
    }
    // The templates' pools: every spawn of an entry in the template's
    // pool.
    let template_pools: Vec<(u32, u32, f32)> = rows(
        db,
        &format!("SELECT * FROM pool_creature_template WHERE {in_patch}"),
        |c| (c.u32("id"), c.u32("pool_entry"), c.f32("chance")),
    );
    if !template_pools.is_empty() {
        let by_entry: ahash::AHashMap<u32, (u32, f32)> = template_pools
            .into_iter()
            .map(|(e, p, ch)| (e, (p, ch)))
            .collect();
        for spawn in data.creature_spawns.values().flatten() {
            if let Some((pool, chance)) =
                spawn.entries.first().and_then(|e| by_entry.get(e)).copied()
                && let Some(p) = s.pools.get_mut(&pool)
            {
                p.creatures.push((spawn.guid, chance));
                s.creature_pool.insert(spawn.guid, pool);
            }
        }
    }
    let object_template_pools: Vec<(u32, u32, f32)> = rows(
        db,
        &format!("SELECT * FROM pool_gameobject_template WHERE {in_patch}"),
        |c| (c.u32("id"), c.u32("pool_entry"), c.f32("chance")),
    );
    if !object_template_pools.is_empty() {
        let by_entry: ahash::AHashMap<u32, (u32, f32)> = object_template_pools
            .into_iter()
            .map(|(e, p, ch)| (e, (p, ch)))
            .collect();
        for spawn in data.gameobject_spawns.values().flatten() {
            if let Some((pool, chance)) = by_entry.get(&spawn.entry).copied()
                && let Some(p) = s.pools.get_mut(&pool)
            {
                p.objects.push((spawn.guid, chance));
                s.object_pool.insert(spawn.guid, pool);
            }
        }
    }
    for t in rows(db, "SELECT * FROM map_template ORDER BY patch", |c| {
        MapTemplate {
            map: c.u32("entry"),
            map_type: c.u32("map_type"),
            player_limit: c.u32("player_limit"),
            reset_delay: c.u32("reset_delay"),
            script_name: c.string("script_name"),
        }
    }) {
        s.map_templates.insert(t.map, t);
    }
    for (id, script) in rows(db, "SELECT * FROM areatrigger_template", |c| {
        (c.u32("id"), c.u32("script_id"))
    }) {
        if script != 0 {
            s.areatrigger_script.insert(id, script);
        }
    }
    info!(
        "scripts: {} creatures with EventAI, {} conditions, {} pools, {} creature groups",
        s.ai_events.len(),
        s.conditions.len(),
        s.pools.len(),
        s.groups.len()
    );
}
