use crate::world::world::client::Client;
use crate::world::world_opcode_handler::entities::{Entities, Entity};
use wow_items::vanilla::{lookup_item, lookup_item_by_name};
use wow_world_base::geometry::trace_point_2d;
use wow_world_base::shared::Guid;
use wow_world_base::vanilla::position::{position_from_str, Position};
use wow_world_base::vanilla::{Item, Map, Vector2d};

pub(crate) enum GmCommand {
    WhereAmI,
    Teleport(Position),
    SetRunSpeed(f32),
    Mark { names: Vec<String>, p: Position },
    RangeToTarget(f32),
    AddItem(&'static Item),
    MoveNpc,
    Spawn {
        display_id: Option<u16>,
        name: Option<String>,
    },
    Boom,
    Nova,
    WorldDbInfo,
    Information(Guid),
    ShouldHaveLineOfSight(Guid),
    ShouldNotHaveLineOfSight(Guid),
    Swifty,
    Players,
    Cells,
}

impl GmCommand {
    pub(crate) fn from_player_command(
        message: &str,
        client: &Client,
        entities: &mut Entities,
    ) -> Result<Self, String> {
        let (command, args) = message.split_once(' ').unwrap_or((message, ""));

        match command {
            "north" | "south" | "east" | "west" => {
                let mut p = client.position();
                match command {
                    "north" => p.x += 5.0,
                    "south" => p.x -= 5.0,
                    "east" => p.y -= 5.0,
                    "west" => p.y += 5.0,
                    _ => unreachable!(),
                }
                Ok(Self::Teleport(p))
            }
            "whereami" => Ok(Self::WhereAmI),
            "info" => {
                let target = args.trim();
                let target = if let Ok(target) = target.parse::<u64>() {
                    Guid::new(target)
                } else if !client.character().target.is_zero() {
                    client.character().target
                } else if target.is_empty() {
                    return Err("No target selected".to_string());
                } else {
                    return Err(format!("Parameter '{target}' is not a valid GUID"));
                };
                Ok(Self::Information(target))
            }
            "tp" => {
                let location = args.trim();
                position_from_str(location)
                    .map(Self::Teleport)
                    .ok_or_else(|| format!("Location not found: '{}'", location))
            }
            "go" => {
                let coordinates: Vec<&str> = args.split_whitespace().collect();
                match coordinates.as_slice() {
                    [] => {
                        // `.go` with no args: use the GM's selected
                        // target. Look in the GM's local cell first
                        // (fast path); fall back to the process-wide
                        // player registry so a target in a neighbor
                        // cell resolves correctly.
                        let target = client.character().target;
                        if target.is_zero() {
                            return Err(
                                "Must have a target for .go command without arguments".to_string()
                            );
                        }
                        if let Some(pos) = entities.find_position(target) {
                            return Ok(Self::Teleport(pos));
                        }
                        crate::world::cell::lookup_player_position(target)
                            .map(|(map, p, orientation)| Self::Teleport(Position {
                                map,
                                x: p.x,
                                y: p.y,
                                z: p.z,
                                orientation,
                            }))
                            .ok_or_else(|| {
                                format!("Unable to find target '{}'", target)
                            })
                    }
                    [name] => {
                        // `.go <name>`: cross-cell lookup by name.
                        // Pre-Stage-5 the [name] branch silently used
                        // `client.character().target` (which made
                        // `.go SomeOnlinePlayer` indistinguishable
                        // from `.go` with no args). Now it actually
                        // resolves the name against the global
                        // player registry.
                        let name_lc = name.to_lowercase();
                        crate::world::cell::lookup_player_position_by_name(&name_lc)
                            .map(|(map, p, orientation)| Self::Teleport(Position {
                                map,
                                x: p.x,
                                y: p.y,
                                z: p.z,
                                orientation,
                            }))
                            .ok_or_else(|| format!("Unable to find player '{}'", name))
                    }
                    [_, _] => Err("Can not teleport with only x and y coordinates".to_string()),
                    [x, y, z] => {
                        let [x, y, z] = [x, y, z].map(|coord| parse_float(coord, "coordinate"));
                        let [x, y, z] = [x?, y?, z?];
                        Ok(Self::Teleport(Position {
                            map: client.character().map,
                            x,
                            y,
                            z,
                            orientation: client.character().info.orientation,
                        }))
                    }
                    [x, y, z, map] => {
                        let [x, y, z] = [x, y, z].map(|coord| parse_float(coord, "coordinate"));
                        let [x, y, z] = [x?, y?, z?];
                        let map = parse_int(map, "map")?;
                        let map =
                            Map::try_from(map).map_err(|_| format!("{map} is not a valid map"))?;
                        Ok(Self::Teleport(Position {
                            map,
                            x,
                            y,
                            z,
                            orientation: client.character().info.orientation,
                        }))
                    }
                    _ => Err("Incorrect '.go' command: Too many arguments".to_string()),
                }
            }
            "speed" => {
                let speed = parse_float(args.trim(), "speed argument")?;
                Ok(Self::SetRunSpeed(speed))
            }
            "mark" => {
                if args.is_empty() {
                    return Err(
                        ".mark a list of names separated by a comma, like '.mark Honor Hold,HH'"
                            .to_string(),
                    );
                }
                let names = args.split(',').map(|a| a.trim().to_string()).collect();
                Ok(Self::Mark {
                    names,
                    p: client.position(),
                })
            }
            "range" => {
                let c = client.character();
                let target = c.target;
                if target.is_zero() {
                    return Err("Unable to find range: No target".to_string());
                }
                if target == c.guid {
                    return Err("Unable to find range: You are targeting yourself".to_string());
                }
                let (position, name) = entities
                    .find_guid(target)
                    .map(|entity| match entity {
                        Entity::Player(c) => (c.position(), c.character().name.as_str()),
                        Entity::Creature(c) => (c.position(), c.name.as_str()),
                    })
                    .ok_or_else(|| {
                        format!("Unable to find range: Unable to find target '{}'", target)
                    })?;

                client.distance_to_position(&position)
                    .map(Self::RangeToTarget)
                    .ok_or_else(|| format!(
                        "Unable to find range: Target '{name}' ({target}) is on map '{}' while you are on '{}'",
                        position.map, c.map
                    ))
            }
            "extend" => {
                let distance = args.trim().parse().unwrap_or(5.0);
                let mut p = client.position();
                let (x, y) = trace_point_2d(Vector2d { x: p.x, y: p.y }, p.orientation, distance);
                p.x = x;
                p.y = y;
                Ok(Self::Teleport(p))
            }
            "float" => {
                let distance = args.trim().parse().unwrap_or(5.0);
                let mut p = client.position();
                p.z += distance;
                Ok(Self::Teleport(p))
            }
            "additem" => {
                let entry = args.trim();
                let entry = entry
                    .parse::<u32>()
                    .ok()
                    .and_then(lookup_item)
                    .or_else(|| lookup_item_by_name(entry))
                    .ok_or_else(|| format!("Unable to additem: '{entry}' is not a valid entry"))?;
                Ok(Self::AddItem(entry))
            }
            "move" => Ok(Self::MoveNpc),
            "spawn" => {
                let args = args.trim();
                if args.is_empty() {
                    return Ok(Self::Spawn {
                        display_id: None,
                        name: None,
                    });
                }
                let (id_part, name_part) = args.split_once(' ').unwrap_or((args, ""));
                let display_id = id_part
                    .parse::<u16>()
                    .map_err(|_| format!("Invalid display_id: '{id_part}'"))?;
                let name = if name_part.trim().is_empty() {
                    None
                } else {
                    Some(name_part.trim().to_string())
                };
                Ok(Self::Spawn {
                    display_id: Some(display_id),
                    name,
                })
            }
            "boom" => Ok(Self::Boom),
            "nova" => Ok(Self::Nova),
            "worlddbinfo" => Ok(Self::WorldDbInfo),
            "los" => Ok(Self::ShouldHaveLineOfSight(client.character().target)),
            "nolos" => Ok(Self::ShouldNotHaveLineOfSight(client.character().target)),
            "swifty" => Ok(Self::Swifty),
            "players" | "playercount" => Ok(Self::Players),
            "cells" => Ok(Self::Cells),
            _ => Err(format!("Invalid GM command: {message}")),
        }
    }
}

fn parse_int(v: &str, argument_name: &str) -> Result<i32, String> {
    match v.parse::<i32>() {
        Ok(e) => Ok(e),
        Err(_) => Err(format!("invalid {argument_name}: '{v}'")),
    }
}

fn parse_float(v: &str, argument_name: &str) -> Result<f32, String> {
    match v.parse::<f32>() {
        Ok(e) => Ok(e),
        Err(_) => Err(format!("invalid {argument_name}: '{v}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_accepts_positive() {
        assert_eq!(parse_int("42", "x"), Ok(42));
    }

    #[test]
    fn parse_int_accepts_negative() {
        assert_eq!(parse_int("-7", "x"), Ok(-7));
    }

    #[test]
    fn parse_int_rejects_garbage() {
        assert_eq!(
            parse_int("abc", "count"),
            Err("invalid count: 'abc'".to_string())
        );
    }

    #[test]
    fn parse_int_rejects_float() {
        assert!(parse_int("1.5", "x").is_err());
    }

    #[test]
    fn parse_float_accepts_decimal() {
        assert_eq!(parse_float("2.5", "y"), Ok(2.5));
    }

    #[test]
    fn parse_float_accepts_negative() {
        assert_eq!(parse_float("-9083.265", "x"), Ok(-9083.265));
    }

    #[test]
    fn parse_float_rejects_garbage() {
        assert!(parse_float("not-a-number", "x").is_err());
    }
}
