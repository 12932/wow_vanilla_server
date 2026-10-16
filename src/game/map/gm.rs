//! GM commands: a chat line starting with `.`.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{SMSG_FORCE_RUN_SPEED_CHANGE, Vector3d};

use crate::game::data::CreatureTemplate;
use crate::game::guid;
use crate::game::map::creature::Creature;
use crate::game::map::{Map, MapKey, chat};

const HELP: &str = "Commands: .go [xyz] <x> <y> <z> [map], .tele <place>, .gps, .die, .revive, .levelup [n], .gm [on|off], .speed <multiplier>, \
.online, .maps, .save, .announce <text>, .kick <name>, .spawn <entry>, .help";

/// Run a command (`text` without its `.`).
pub fn command(map: &mut Map, guid: Guid, text: &str) {
    let Some(p) = map.players.get(&guid) else {
        return;
    };
    if !p.gm {
        chat::send_system(&p.outbound, "You can't use commands.");
        return;
    }
    let mut words = text.split_whitespace();
    let Some(name) = words.next() else {
        return;
    };
    let args: Vec<&str> = words.collect();
    let reply = match name.to_ascii_lowercase().as_str() {
        "go" => go(map, guid, &args),
        "gps" => gps(map, guid),
        "speed" => speed(map, guid, &args),
        "online" => Some(format!(
            "{} players online, {} characters.",
            map.realm.registry.online_count(),
            map.realm.registry.character_count()
        )),
        "maps" => Some(maps(map)),
        "save" => {
            map.save_player(guid);
            Some("Saved.".to_owned())
        }
        "announce" => {
            let text = args.join(" ");
            for o in map.realm.registry.all_online() {
                chat::send_system(&o.outbound, &text);
            }
            None
        }
        "kick" => Some(kick(map, &args)),
        "spawn" => spawn(map, guid, &args),
        "tele" => tele(map, guid, &args),
        "die" => {
            let target = target_or_self(map, guid);
            crate::game::map::death::kill(map, target);
            None
        }
        "revive" => {
            let target = target_or_self(map, guid);
            crate::game::map::death::resurrect(map, target, 1.0);
            None
        }
        "levelup" => {
            let n: i32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(1);
            let data = map.realm.data.clone();
            let target = target_or_self(map, guid);
            let level = map.players.get_mut(&target).map(|p| {
                let level = (i32::from(p.level()) + n).clamp(1, 60) as u8;
                crate::game::map::stats::set_level(p, &data, level);
                level
            });
            if let Some(level) = level {
                map.realm.registry.set_level(target, level);
            }
            None
        }
        "gm" => {
            let on = args.first().is_none_or(|a| !a.eq_ignore_ascii_case("off"));
            if let Some(p) = map.players.get_mut(&guid) {
                p.fields.set_flag(
                    crate::game::fields::PLAYER_FLAGS,
                    crate::game::map::player::PLAYER_FLAGS_GM,
                    on,
                );
            }
            Some(format!("GM mode {}.", if on { "on" } else { "off" }))
        }
        "help" => Some(HELP.to_owned()),
        other => Some(format!("No command {other}. {HELP}")),
    };
    if let Some(reply) = reply {
        map.system_message(guid, &reply);
    }
}

fn parse_f32(s: Option<&&str>) -> Option<f32> {
    s.and_then(|s| s.parse::<f32>().ok())
        .filter(|v| v.is_finite())
}

/// `.go <x> <y> <z> [map]`, or `.go xyz <x> <y> <z> [map]` as the mangos
/// family spells it.
fn go(map: &mut Map, guid: Guid, args: &[&str]) -> Option<String> {
    let args = if args.first().is_some_and(|a| a.eq_ignore_ascii_case("xyz")) {
        &args[1..]
    } else {
        args
    };
    let (Some(x), Some(y), Some(z)) = (
        parse_f32(args.first()),
        parse_f32(args.get(1)),
        parse_f32(args.get(2)),
    ) else {
        return Some("Usage: .go [xyz] <x> <y> <z> [map]".to_owned());
    };
    let target = match args.get(3) {
        Some(m) => match m.parse::<u32>() {
            Ok(id) => MapKey::world(id),
            Err(_) => return Some(format!("{m} isn't a map id.")),
        },
        None => map.key,
    };
    if target != map.key && wow_world_messages::vanilla::Map::try_from(target.map).is_err() {
        return Some(format!("There's no map {}.", target.map));
    }
    let orientation = map
        .players
        .get(&guid)
        .map(|p| p.movement.orientation)
        .unwrap_or(0.0);
    map.teleport(guid, target, Vector3d { x, y, z }, orientation);
    None
}

fn gps(map: &Map, guid: Guid) -> Option<String> {
    let p = map.players.get(&guid)?;
    let pos = p.position();
    Some(format!(
        "{}: x {:.2}, y {:.2}, z {:.2}, orientation {:.2}, zone {}, area {}",
        map.key, pos.x, pos.y, pos.z, p.movement.orientation, p.zone, p.area
    ))
}

fn speed(map: &mut Map, guid: Guid, args: &[&str]) -> Option<String> {
    let Some(mult) = parse_f32(args.first()).filter(|m| *m > 0.0 && *m <= 10.0) else {
        return Some("Usage: .speed <multiplier from 0.1 to 10>".to_owned());
    };
    let p = map.players.get_mut(&guid)?;
    p.speeds.run = 7.0 * mult;
    p.outbound.send(&SMSG_FORCE_RUN_SPEED_CHANGE {
        guid,
        move_event: 0,
        speed: p.speeds.run,
    });
    None
}

fn maps(map: &Map) -> String {
    let s = map.stats;
    format!(
        "{}: {} players, {} creatures, {} grid cells; {} updates, last {} us, max {} us; {} events.",
        map.key,
        map.players.len(),
        map.creatures.len(),
        map.grid.cell_count(),
        s.updates,
        s.last_update.as_micros(),
        s.max_update.as_micros(),
        s.events
    )
}

fn kick(map: &Map, args: &[&str]) -> String {
    let Some(name) = args.first() else {
        return "Usage: .kick <name>".to_owned();
    };
    match map.realm.registry.online_by_name(name) {
        Some(o) => {
            let _ = o.session.send(crate::game::session::Control::Kick);
            format!("Kicked {}.", o.name)
        }
        None => format!("{name} isn't online."),
    }
}

fn spawn(map: &mut Map, guid: Guid, args: &[&str]) -> Option<String> {
    let entry = args
        .first()
        .and_then(|a| a.parse::<u32>().ok())
        .unwrap_or(0);
    let p = map.players.get(&guid)?;
    let template = map
        .realm
        .data
        .creature_template(entry)
        .cloned()
        .unwrap_or_else(|| CreatureTemplate {
            entry,
            name: "Creature".to_owned(),
            display_ids: [262, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 14,
            health_max: 100,
            ..Default::default()
        });
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let level = template.level_min.max(1);
    let c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &template,
        level,
        p.position(),
        p.movement.orientation,
    );
    let g = map.add_creature(c);
    Some(format!("Spawned {} ({g}).", template.name))
}

/// `.tele <place>`: a place from vmangos's `game_tele`, by name or the
/// start of one.
fn tele(map: &mut Map, guid: Guid, args: &[&str]) -> Option<String> {
    let wanted = args.join(" ").to_lowercase();
    if wanted.is_empty() {
        return Some("Usage: .tele <place>".to_owned());
    }
    let data = map.realm.data.clone();
    let place = data
        .tele
        .iter()
        .find(|t| t.name.to_lowercase() == wanted)
        .or_else(|| {
            data.tele
                .iter()
                .find(|t| t.name.to_lowercase().starts_with(&wanted))
        })?;
    map.teleport(
        guid,
        MapKey::world(place.map),
        Vector3d {
            x: place.x,
            y: place.y,
            z: place.z,
        },
        place.orientation,
    );
    None
}

/// The player a command acts on: the GM's target if it is a player in the
/// map, else the GM.
fn target_or_self(map: &Map, guid: Guid) -> Guid {
    map.players
        .get(&guid)
        .map(|p| p.fields.get_guid(crate::game::fields::UNIT_FIELD_TARGET))
        .filter(|t| map.players.contains_key(t))
        .unwrap_or(guid)
}
