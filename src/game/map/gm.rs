//! GM commands: a chat line starting with `.`; and players' few, as
//! mangos-classic's `SEC_PLAYER` commands (`PlayerCommands`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::CreatureTemplate;
use crate::game::guid;
use crate::game::map::creature::Creature;
use crate::game::map::{Map, MapKey, chat};

const HELP: &str = "Commands: .go [xyz] <x> <y> <z> [map], .tele <place>, .gps, .hyperspawn, .die, .revive, .levelup [n], .gm [on|off], .speed <multiplier>, \
.online, .maps, .save, .announce <text>, .kick <name>, .rename [name], .spawn <entry>, .account create|password <name> <password>, \
.account gm <name> <level>, .reload config, .ban <account> <minutes|perm> [reason], .unban <account>, .ticket list|<id>|respond <id> <answer>|close <id>, .honor [week|points <n>], .arena start|stop|status|history, .help";

/// The commands a player has, as mangos-classic's `SEC_PLAYER` table
/// (`Level0.cpp`) has them.
const PLAYER_HELP: &str = "Commands: .help, .commands, .save, .start, .dismount, .server info|motd, .gm ingame, .account, .account password <old> <new> <new>";

/// The level `name` needs with these `args`: a few of a GM command's are
/// a player's (`.gm ingame`, `.account`, `.account password <old> <new>
/// <new>`), as in mangos's tables; else the command's own.
fn needed_level_with(name: &str, args: &[&str]) -> u8 {
    use crate::accounts::PLAYER;
    match (name, args) {
        ("gm", [sub]) if sub.eq_ignore_ascii_case("ingame") => PLAYER,
        ("account", []) => PLAYER,
        ("account", [sub, _, _, _]) if sub.eq_ignore_ascii_case("password") => PLAYER,
        _ => needed_level(name),
    }
}

/// The GM level a command needs: moderators look and talk, game masters
/// change the world, administrators run accounts.
pub fn needed_level(command: &str) -> u8 {
    use crate::accounts::{ADMINISTRATOR, GAME_MASTER, MODERATOR};
    if let Some(level) = crate::game::map::gm_dev::needed_level(command) {
        return level;
    }
    match command {
        // A player's (mangos's `SEC_PLAYER`).
        "help" | "commands" | "save" | "start" | "dismount" | "server" => crate::accounts::PLAYER,
        "gps" | "hyperspawn" | "online" | "maps" | "announce" | "kick" | "gm" | "ticket" => {
            MODERATOR
        }
        "account" | "ban" | "unban" | "reload" => ADMINISTRATOR,
        // Anyone may ask about the arena; starting and stopping is checked
        // within.
        "arena" => crate::accounts::PLAYER,
        _ => GAME_MASTER,
    }
}

/// `.account create|password <name> <password>`, `.account gm <name> <level>`.
fn account(map: &Map, args: &[&str]) -> String {
    let Some(accounts) = crate::accounts::get() else {
        return "There are no accounts here.".into();
    };
    match args {
        ["create" | "password", name, password] => match accounts.set_password(name, password) {
            Ok(()) => format!("Account {} set.", name.to_uppercase()),
            Err(e) => e,
        },
        ["gm", name, level] => match level.parse::<u8>() {
            Ok(level) if accounts.set_gm_level(name, level) => {
                // It takes effect at the next login.
                let _ = map;
                format!(
                    "{} is GM level {level} from their next login.",
                    name.to_uppercase()
                )
            }
            _ => format!("No account {name}, or no level {level}."),
        },
        _ => "Use: .account create|password <name> <password>, .account gm <name> <level>".into(),
    }
}

/// `.ban <account> <minutes|perm> [reason]`: they can't log in, and go now.
fn ban(map: &Map, args: &[&str]) -> String {
    let Some(accounts) = crate::accounts::get() else {
        return "There are no accounts here.".into();
    };
    let (Some(name), Some(time)) = (args.first(), args.get(1)) else {
        return "Use: .ban <account> <minutes|perm> [reason]".into();
    };
    let until = if time.eq_ignore_ascii_case("perm") {
        -1
    } else {
        // Past the end of time is for good, not a wrap into the past (no
        // ban at all).
        match time.parse::<i64>() {
            Ok(m) if m > 0 => m
                .checked_mul(60)
                .and_then(|s| chrono::Utc::now().timestamp().checked_add(s))
                .unwrap_or(-1),
            _ => return "Use: .ban <account> <minutes|perm> [reason]".into(),
        }
    };
    let reason = args[2..].join(" ");
    if !accounts.ban(name, until, &reason) {
        return format!("No account {name}.");
    }
    // The account's session, on the character screen or in the world (as
    // mangos's ban kicks the account's session), and a player of it its
    // session left in the world.
    let account = crate::auth::account_name(name);
    let registry = &map.realm.registry;
    if let Some(s) = registry.session_for_account(&account) {
        crate::game::conn::uncork();
        let _ = s.send(crate::game::session::Control::Kick);
    }
    if let Some(o) = registry.online_for_account(&account) {
        crate::game::conn::uncork();
        let _ = o.session.send(crate::game::session::Control::Kick);
    }
    format!("{account} is banned.")
}

fn unban(args: &[&str]) -> String {
    match (crate::accounts::get(), args.first()) {
        (Some(a), Some(name)) if a.ban(name, 0, "") => {
            format!("{} may log in again.", name.to_uppercase())
        }
        _ => "Use: .unban <account>".into(),
    }
}

/// `.reload config`: the file and the environment read again, off the
/// map, and what is safe to change live put in; the reply, when it is
/// done, says what changed and what needs a restart.
fn reload(map: &Map, guid: Guid, args: &[&str]) -> String {
    if !matches!(args, ["config"]) {
        return "Use: .reload config".into();
    }
    let realm = map.realm.clone();
    tokio::spawn(async move {
        let reply = match crate::game::reload_from_disk(realm.clone()).await {
            Ok(report) => report.to_string(),
            Err(e) => format!("The config wasn't reloaded: {e}"),
        };
        if let Some(o) = realm.registry.online(guid) {
            chat::send_system(&o.outbound, &reply);
        }
    });
    "Reloading the config.".into()
}

/// Run a command (`text` without its `.`).
pub fn command(map: &mut Map, guid: Guid, text: &str) {
    let Some(p) = map.players.get(&guid) else {
        return;
    };
    let level = p.gm_level;
    let mut words = text.split_whitespace();
    let Some(name) = words.next() else {
        return;
    };
    let name = name.to_ascii_lowercase();
    let args: Vec<&str> = words.collect();
    // The config's level for it (`[gm] command_levels`), else the server's.
    let needed = map
        .cfg
        .gm
        .command_levels
        .get(&name)
        .copied()
        .unwrap_or_else(|| needed_level_with(&name, &args));
    if level < needed {
        // A player isn't told of a GM's commands (mangos's
        // `LANG_NO_CMD`).
        let refusal = if level == crate::accounts::PLAYER {
            "There is no such command."
        } else {
            "You don't have permission to use that command."
        };
        chat::send_system(&p.outbound, refusal);
        return;
    }
    let reply = match name.as_str() {
        "account" => Some(match args.as_slice() {
            [] => format!("Your account level is: {level}"),
            [sub, old, new, again] if sub.eq_ignore_ascii_case("password") => {
                own_password(map, guid, old, new, again)
            }
            _ => account(map, &args),
        }),
        "commands" => Some(if level == crate::accounts::PLAYER {
            PLAYER_HELP.to_owned()
        } else {
            format!("{HELP} {} {PLAYER_HELP}", crate::game::map::gm_dev::HELP)
        }),
        "server" => Some(server(map, &args)),
        "dismount" => {
            let mounted = map.players.get(&guid).is_some_and(|p| {
                p.fields
                    .get_u32(crate::game::fields::UNIT_FIELD_MOUNTDISPLAYID)
                    != 0
            });
            if mounted {
                crate::game::map::mounts::dismount(map, guid);
                None
            } else {
                Some("You are not mounted.".to_owned())
            }
        }
        "gm" if args
            .first()
            .is_some_and(|a| a.eq_ignore_ascii_case("ingame")) =>
        {
            Some(gms_online(map))
        }
        "ban" => Some(ban(map, &args)),
        "unban" => Some(unban(&args)),
        "reload" => Some(reload(map, guid, &args)),
        "go" => go(map, guid, &args),
        "gps" => gps(map, guid),
        "hyperspawn" => crate::game::map::hyperspawn::report(map, guid),
        "speed" => speed(map, guid, &args),
        "online" => Some(format!(
            "{} players online, {} characters.",
            map.realm.registry.online_count(),
            map.realm.registry.character_count()
        )),
        "maps" => Some(maps(map)),
        "invasion" => Some(invasion(map, &args)),
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
        "rename" => Some(rename(map, guid, &args)),
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
                let level = i32::from(p.level())
                    .saturating_add(n)
                    .clamp(1, i32::from(crate::game::map::stats::max_level()))
                    as u8;
                crate::game::map::stats::set_level(p, &data, level);
                level
            });
            if let Some(level) = level {
                map.realm.registry.set_level(target, level);
                crate::game::map::misc::reset_if_overbudget(map, target);
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
            // A game master sees what others don't (the concealed, the
            // stealthed, ghosts): from now, not from their next steps.
            map.visibility_dirty.insert(guid);
            Some(format!("GM mode {}.", if on { "on" } else { "off" }))
        }
        "ticket" => Some(crate::game::map::tickets::command(map, guid, &args)),
        "honor" => Some(crate::game::map::pvp::gm(map, guid, &args)),
        "arena" => Some(crate::game::map::arena::command(map, guid, &args)),
        "help" if level == crate::accounts::PLAYER => Some(PLAYER_HELP.to_owned()),
        "help" => Some(format!("{HELP} {}", crate::game::map::gm_dev::HELP)),
        other => crate::game::map::gm_dev::command(map, guid, other, &args)
            .unwrap_or_else(|| Some(format!("No command {other}. {HELP}"))),
    };
    if let Some(reply) = reply {
        map.system_message(guid, &reply);
    }
}

/// `.account password <old> <new> <new>`: a player's own, the old one
/// checked (mangos's `HandleAccountPasswordCommand`).
fn own_password(map: &Map, guid: Guid, old: &str, new: &str, again: &str) -> String {
    let Some(accounts) = crate::accounts::get() else {
        return "There are no accounts here.".into();
    };
    let Some(account) = map.players.get(&guid).map(|p| p.account.to_string()) else {
        return String::new();
    };
    if !accounts.check_password(&account, old) {
        return "The old password is wrong.".into();
    }
    if new != again {
        return "The new passwords don't match.".into();
    }
    match accounts.set_password(&account, new) {
        Ok(()) => "Your password is changed.".into(),
        Err(e) => e,
    }
}

/// `.server info` and `.server motd` (mangos's `HandleServerInfoCommand`,
/// `HandleServerMotdCommand`).
fn server(map: &Map, args: &[&str]) -> String {
    match args.first().map(|a| a.to_ascii_lowercase()).as_deref() {
        Some("motd") => {
            let motd = map.realm.config().motd.clone();
            if motd.is_empty() {
                "There is no message of the day.".into()
            } else {
                format!("Message of the day: {motd}")
            }
        }
        Some("info") | None => {
            let up = crate::game::uptime_ms() / 1000;
            format!(
                "wow_vanilla_server {}. Players online: {}. Server uptime: {}h {}m {}s.",
                crate::VERSION,
                map.realm.registry.online_count(),
                up / 3600,
                up / 60 % 60,
                up % 60
            )
        }
        _ => "Use: .server info|motd".into(),
    }
}

/// `.gm ingame`: the GMs online (mangos's `HandleGMListIngameCommand`).
fn gms_online(map: &Map) -> String {
    let mut names: Vec<String> = map
        .realm
        .registry
        .all_online()
        .iter()
        .filter(|o| o.gm)
        .map(|o| o.name.to_string())
        .collect();
    if names.is_empty() {
        return "There are no GMs currently logged in on this server.".into();
    }
    names.sort();
    format!("Active GMs on server: {}", names.join(", "))
}

fn parse_f32(s: Option<&&str>) -> Option<f32> {
    s.and_then(|s| s.parse::<f32>().ok())
        .filter(|v| v.is_finite())
}

/// The farthest from a map's middle a place can be: past every map's
/// edge (64 tiles of 533 yards), short of where distances overflow.
pub const MAX_COORDINATE: f32 = 100_000.0;

/// A coordinate: a finite number, in the world.
pub fn parse_coordinate(s: Option<&&str>) -> Option<f32> {
    parse_f32(s).filter(|v| v.abs() <= MAX_COORDINATE)
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
        parse_coordinate(args.first()),
        parse_coordinate(args.get(1)),
        parse_coordinate(args.get(2)),
    ) else {
        return Some("Usage: .go [xyz] <x> <y> <z> [map] [facing]".to_owned());
    };
    // Nothing past the facing, and a facing given is a number (Thistle
    // Tea's `.go xyz` takes no stray words).
    if args.len() > 5 || (args.len() == 5 && parse_f32(args.get(4)).is_none()) {
        return Some("Usage: .go [xyz] <x> <y> <z> [map] [facing]".to_owned());
    }
    let target = match args.get(3) {
        Some(&"here") | None => map.key,
        Some(m) => match m.parse::<u32>() {
            Ok(id) => MapKey::world(id),
            Err(_) => return Some(format!("{m} isn't a map id.")),
        },
    };
    if target != map.key && wow_world_messages::vanilla::Map::try_from(target.map).is_err() {
        return Some(format!("There's no map {}.", target.map));
    }
    // A facing in radians, or the GM's own.
    let orientation = parse_f32(args.get(4)).unwrap_or_else(|| {
        map.players
            .get(&guid)
            .map(|p| p.movement.orientation)
            .unwrap_or(0.0)
    });
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
    // The run's base, kept through what changes speeds; the run worked out
    // again and told (to those watching too).
    map.players.get_mut(&guid)?.speed_rate = mult;
    crate::game::map::derive::unit(map, guid);
    None
}

fn maps(map: &Map) -> String {
    let s = map.stats;
    format!(
        "{}: {} players, {} creatures, {} grid cells; {} updates, last {} us, max {} us; {} events; panics {}, quarantined {}, restarts {}.",
        map.key,
        map.players.len(),
        map.creatures.len(),
        map.grid.cell_count(),
        s.updates,
        s.last_update.as_micros(),
        s.max_update.as_micros(),
        s.events,
        s.panics,
        s.quarantined,
        s.restarts
    )
}

fn kick(map: &Map, args: &[&str]) -> String {
    let Some(name) = args.first() else {
        return "Usage: .kick <name>".to_owned();
    };
    match map.realm.registry.online_by_name(name) {
        Some(o) => {
            crate::game::conn::uncork();
            let _ = o.session.send(crate::game::session::Control::Kick);
            format!("Kicked {}.", o.name)
        }
        None => format!("{name} isn't online."),
    }
}

/// `.rename [name]`: the character named, or the player targeted, must
/// take a new name before they next enter the world (mangos's
/// `.character rename`, `AT_LOGIN_RENAME`): kept on them if they are
/// online, else in the store.
fn rename(map: &mut Map, guid: Guid, args: &[&str]) -> String {
    use crate::game::store::AT_LOGIN_RENAME;
    let target = match args.first() {
        Some(name) => match map.realm.registry.guid_by_name(name) {
            Some(g) => g,
            None => return format!("No character {name}."),
        },
        None => target_or_self(map, guid),
    };
    let name = map
        .realm
        .registry
        .name(target)
        .map_or_else(String::new, |e| e.name.to_string());
    if map.realm.registry.online(target).is_some() {
        crate::game::map::call_player(&map.realm, target, move |m| {
            if let Some(p) = m.players.get_mut(&target) {
                p.row.at_login |= AT_LOGIN_RENAME;
            }
        });
        return format!("{name} will be asked for a new name at their next login.");
    }
    let realm = map.realm.clone();
    tokio::spawn(async move {
        let Some(mut row) = realm.store.load(crate::game::guid::counter(target)).await else {
            return;
        };
        // Logged in meanwhile: flagged where they are.
        if realm.registry.online(target).is_some() {
            crate::game::map::call_player(&realm, target, move |m| {
                if let Some(p) = m.players.get_mut(&target) {
                    p.row.at_login |= AT_LOGIN_RENAME;
                }
            });
            return;
        }
        row.at_login |= AT_LOGIN_RENAME;
        realm.store.save(row);
    });
    format!("{name} will be asked for a new name at their next login.")
}

fn spawn(map: &mut Map, guid: Guid, args: &[&str]) -> Option<String> {
    let entry = args
        .first()
        .and_then(|a| a.parse::<u32>().ok())
        .unwrap_or(0);
    let p = map.players.get(&guid)?;
    let (at, orientation) = (p.position(), p.movement.orientation);
    let counter = map.creature_counter();
    // A known entry is built as a world spawn of it is, with its weapons,
    // stats, flags and auras (built bare, Drek'Thar held nothing). It
    // doesn't respawn.
    let data = map.realm.data.clone();
    let spawn = crate::game::data::CreatureSpawn {
        guid: counter,
        entries: vec![entry],
        map: map.key.map,
        x: at.x,
        y: at.y,
        z: at.z,
        orientation,
        ..Default::default()
    };
    if let Some(mut c) = Creature::from_spawn(&data, &spawn, &mut rand::rng()) {
        c.spawn_guid = None;
        let g = map.add_creature(c);
        let name = data
            .creature_template(entry)
            .map_or("", |t| t.name.as_str());
        return Some(format!("Spawned {name} ({g})."));
    }
    let template = CreatureTemplate {
        entry,
        name: "Creature".to_owned(),
        display_ids: [262, 0, 0, 0],
        level_min: 1,
        level_max: 1,
        faction: 14,
        health_max: 100,
        ..Default::default()
    };
    let mut c = Creature::new(guid::unit(entry, counter), &template, 1, at, orientation);
    c.fit_model(&data, Some(&template));
    let g = map.add_creature(c);
    Some(format!("Spawned {} ({g}).", template.name))
}

/// `.tele <place>`: a place from vmangos's `game_tele`, by name, the
/// start of one, or a part of one.
fn tele(map: &mut Map, guid: Guid, args: &[&str]) -> Option<String> {
    let wanted = args.join(" ").to_lowercase();
    if wanted.is_empty() {
        return Some("Usage: .tele <place>".to_owned());
    }
    let wanted = wanted.trim();
    let data = map.realm.data.clone();
    let mut starting: Vec<&crate::game::data::Tele> = data
        .tele
        .iter()
        .filter(|t| t.name.to_lowercase().starts_with(wanted))
        .collect();
    // None starts with it: those with it anywhere in their name, as
    // mangos's `GetGameTele` and Thistle Tea's `GameTele.search`.
    if starting.is_empty() {
        starting = data
            .tele
            .iter()
            .filter(|t| t.name.to_lowercase().contains(wanted))
            .collect();
    }
    let exact = data.tele.iter().find(|t| t.name.to_lowercase() == wanted);
    // Several fit and none exactly: they are listed.
    if exact.is_none() && starting.len() > 1 {
        let names: Vec<&str> = starting.iter().take(15).map(|t| t.name.as_str()).collect();
        return Some(format!("Places: {}", names.join(", ")));
    }
    let Some(place) = exact.or(starting.first().copied()) else {
        return Some(format!("No place {wanted}."));
    };
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

/// `.invasion [on|off|fell <zone>|attack <zone>]`: drive the Scourge
/// Invasion.
fn invasion(map: &mut Map, args: &[&str]) -> String {
    let realm = map.realm.clone();
    match args {
        ["on"] => {
            crate::game::invasion::enable(&realm);
            "The Scourge invade.".to_owned()
        }
        ["off"] => {
            crate::game::invasion::disable(&realm);
            "The Scourge are called off.".to_owned()
        }
        ["fell", zone] => match zone.parse() {
            Ok(z) => {
                crate::game::invasion::necropolis_fell(&realm, z);
                format!("A necropolis over zone {z} falls.")
            }
            Err(_) => "Usage: .invasion fell <zone id>".to_owned(),
        },
        ["attack", zone] => match zone.parse() {
            Ok(z) => {
                use crate::game::invasion::AttackRefused;
                match crate::game::invasion::attack(&realm, z) {
                    Ok(()) => format!("The Scourge fall on zone {z}."),
                    Err(AttackRefused::Disabled) => "The invasion is off.".to_owned(),
                    Err(AttackRefused::UnknownZone) => {
                        format!("The Scourge don't attack zone {z}.")
                    }
                    Err(AttackRefused::AlreadyAttacked) => {
                        format!("Zone {z} is under attack already.")
                    }
                }
            }
            Err(_) => "Usage: .invasion attack <zone id>".to_owned(),
        },
        _ => {
            let s = realm.social.invasion.lock();
            format!(
                "The invasion is {}: {} victories, {} zones under attack.",
                if s.enabled { "on" } else { "off" },
                s.victories,
                s.active.len()
            )
        }
    }
}
