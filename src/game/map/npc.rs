//! NPC services: gossip menus and texts, trainers, innkeepers, bankers,
//! flight masters, battlemasters and the auctioneer's hello.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::Body;
use crate::game::data::npc::{GossipOption, TaxiPathNode};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::creature::Walk;
use crate::game::map::{quests, vendor};
use crate::game::registry::Team;

pub const NPC_FLAG_GOSSIP: u32 = 0x1;
pub const NPC_FLAG_QUESTGIVER: u32 = 0x2;
pub const NPC_FLAG_VENDOR: u32 = 0x4;
pub const NPC_FLAG_FLIGHTMASTER: u32 = 0x8;
pub const NPC_FLAG_TRAINER: u32 = 0x10;
pub const NPC_FLAG_SPIRITHEALER: u32 = 0x20;
pub const NPC_FLAG_INNKEEPER: u32 = 0x80;
pub const NPC_FLAG_BANKER: u32 = 0x100;
pub const NPC_FLAG_BATTLEMASTER: u32 = 0x800;
pub const NPC_FLAG_AUCTIONEER: u32 = 0x1000;

const UNIT_FLAG_TAXI_FLIGHT: u32 = 0x0010_0000;
const UNIT_FLAG_DISABLE_MOVE: u32 = 0x0000_0004;
/// A flight's speed, in yards a second.
const TAXI_SPEED: f32 = 32.0;
/// The text an NPC without its own shows.
const DEFAULT_TEXT: u32 = 1;

/// A flight in progress.
#[derive(Debug, Clone)]
pub struct Flight {
    pub walk: Walk,
    pub destination: u32,
    pub mount: u32,
}

/// The creature `npc`, if `player` may talk to it and it has `flag`: its
/// entry.
pub fn npc_entry(map: &Map, player: Guid, npc: Guid, flag: u32) -> Option<u32> {
    let p = map.players.get(&player)?;
    let c = map.creature(npc)?;
    if !c.alive() || (flag != 0 && c.fields.get_u32(UNIT_NPC_FLAGS) & flag == 0) {
        return None;
    }
    if dist(p.position(), c.position) > vendor::NPC_YARDS {
        return None;
    }
    Some(c.entry)
}

fn send(map: &Map, player: Guid, m: &impl ServerMessage) {
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(m);
    }
}

/// `CMSG_GOSSIP_HELLO`: the NPC's menu with its quests, or its one service
/// at once.
pub fn hello(map: &mut Map, player: Guid, npc: Guid) {
    if crate::game::map::death::spirit_healer_hello(map, player, npc)
        || crate::game::map::battleground::gossip_hello(map, player, npc)
    {
        return;
    }
    let Some(entry) = npc_entry(map, player, npc, 0) else {
        return;
    };
    let data = map.realm.data.clone();
    let flags = map
        .creature(npc)
        .map(|c| c.fields.get_u32(UNIT_NPC_FLAGS))
        .unwrap_or(0);
    let menu_id = crate::game::map::event_ai::scripted(map, npc)
        .and_then(|s| s.gossip_menu)
        .or_else(|| data.creature_template(entry).map(|t| t.gossip_menu_id))
        .unwrap_or(0);
    let options = menu_options(map, menu_id, flags, player, npc);
    let quests = quests::giver_quests(map, player, npc);
    if options.is_empty() && quests.is_empty() && menu_id == 0 {
        // No menu: the NPC's one service.
        if flags & NPC_FLAG_VENDOR != 0 {
            return vendor::list(map, player, npc);
        }
        if flags & NPC_FLAG_TRAINER != 0 {
            return trainer_list(map, player, npc);
        }
        if flags & NPC_FLAG_FLIGHTMASTER != 0 {
            return taxi_nodes(map, player, npc);
        }
        if flags & NPC_FLAG_BANKER != 0 {
            return send(map, player, &SMSG_SHOW_BANK { guid: npc });
        }
        if flags & NPC_FLAG_AUCTIONEER != 0 {
            return auction_hello(map, player, npc);
        }
        if flags & NPC_FLAG_BATTLEMASTER != 0 {
            return battlemaster_hello(map, player, npc);
        }
        if flags & crate::game::map::pets::NPC_FLAG_STABLEMASTER != 0 {
            return crate::game::map::pets::list_stable(map, player, npc);
        }
    }
    show_menu(map, player, npc, menu_id, flags);
}

/// The options of a menu this NPC offers.
fn menu_options(
    map: &Map,
    menu_id: u32,
    npc_flags: u32,
    player: Guid,
    npc: Guid,
) -> Vec<GossipOption> {
    map.realm
        .data
        .npc
        .gossip_menus
        .get(&menu_id)
        .map(|m| {
            m.options
                .iter()
                .filter(|o| o.npc_flag == 0 || o.npc_flag & npc_flags != 0)
                .filter(|o| {
                    o.condition == 0
                        || !map.realm.data.scripts.conditions.contains_key(&o.condition)
                        || crate::game::map::conditions::met(
                            map,
                            o.condition,
                            Some(npc),
                            Some(player),
                        )
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub fn show_menu(map: &mut Map, player: Guid, npc: Guid, menu_id: u32, npc_flags: u32) {
    let data = map.realm.data.clone();
    let text = data
        .npc
        .gossip_menus
        .get(&menu_id)
        .and_then(|m| {
            m.texts
                .iter()
                .find(|(_, cond)| {
                    *cond != 0
                        && crate::game::map::conditions::met(map, *cond, Some(npc), Some(player))
                })
                .or_else(|| m.texts.iter().find(|(_, cond)| *cond == 0))
                .or(m.texts.first())
        })
        .map(|(t, _)| *t)
        .unwrap_or(DEFAULT_TEXT);
    if let Some(script) = data.scripts.gossip_menu_script.get(&menu_id).copied() {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::Gossip,
            script,
            npc,
            player,
        );
    }
    let gossips = menu_options(map, menu_id, npc_flags, player, npc)
        .iter()
        .map(|o| GossipItem {
            id: o.id,
            item_icon: o.icon,
            coded: o.coded,
            message: o.text.clone(),
        })
        .collect();
    let quests = quests::giver_quests(map, player, npc);
    if let Some(p) = map.players.get_mut(&player) {
        p.gossip_menu = Some((npc, menu_id));
        p.outbound.send(&SMSG_GOSSIP_MESSAGE {
            guid: npc,
            title_text_id: text,
            gossips,
            quests,
        });
    }
}

/// `CMSG_GOSSIP_SELECT_OPTION`: what the option does.
pub fn select(map: &mut Map, player: Guid, npc: Guid, option: u32) {
    if crate::game::map::battleground::gossip_select(map, player, npc, option) {
        return;
    }
    let Some(entry) = npc_entry(map, player, npc, 0) else {
        return;
    };
    let data = map.realm.data.clone();
    let flags = map
        .creature(npc)
        .map(|c| c.fields.get_u32(UNIT_NPC_FLAGS))
        .unwrap_or(0);
    let menu_id = map
        .players
        .get(&player)
        .and_then(|p| p.gossip_menu)
        .filter(|(g, _)| *g == npc)
        .map(|(_, m)| m)
        .or_else(|| data.creature_template(entry).map(|t| t.gossip_menu_id))
        .unwrap_or(0);
    let Some(o) = menu_options(map, menu_id, flags, player, npc)
        .into_iter()
        .find(|o| o.id == option)
    else {
        return;
    };
    // An option may mark a spot on the map, besides what it does.
    if o.action_poi != 0
        && let Some(poi) = data.world.pois.get(&o.action_poi)
    {
        send(
            map,
            player,
            &SMSG_GOSSIP_POI {
                flags: poi.flags,
                position: Vector2d { x: poi.x, y: poi.y },
                icon: poi.icon,
                data: poi.data,
                location_name: poi.name.clone(),
            },
        );
    }
    if o.script != 0 {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::Gossip,
            o.script,
            npc,
            player,
        );
        if o.kind == 1 && o.action_menu <= 0 {
            return send(map, player, &SMSG_GOSSIP_COMPLETE {});
        }
    }
    match o.kind {
        1 if o.action_menu > 0 => show_menu(map, player, npc, o.action_menu as u32, flags),
        2 => quests::hello(map, player, npc),
        3 => vendor::list(map, player, npc),
        4 => taxi_nodes(map, player, npc),
        5 => trainer_list(map, player, npc),
        6 => {
            crate::game::map::death::spirit_healer_hello(map, player, npc);
        }
        8 => send(map, player, &SMSG_BINDER_CONFIRM { guid: npc }),
        9 => send(map, player, &SMSG_SHOW_BANK { guid: npc }),
        12 => battlemaster_hello(map, player, npc),
        13 => auction_hello(map, player, npc),
        14 => crate::game::map::pets::list_stable(map, player, npc),
        17 => crate::game::map::pets::untrain_offer(map, player),
        15 => vendor::list(map, player, npc),
        16 => crate::game::map::misc::talent_wipe_offer(map, player, npc),
        _ => send(map, player, &SMSG_GOSSIP_COMPLETE {}),
    }
}

/// `CMSG_NPC_TEXT_QUERY`: an NPC text's eight choices.
pub fn npc_text(map: &Map, player: Guid, text_id: u32) {
    let data = &map.realm.data.npc;
    let female = map.players.get(&player).is_some_and(|p| p.row.gender == 1);
    let empty = || NpcTextUpdate {
        probability: 0.0,
        texts: [String::new(), String::new()],
        language: Language::Universal,
        emotes: std::array::from_fn(|_| NpcTextUpdateEmote { delay: 0, emote: 0 }),
    };
    let mut texts: [NpcTextUpdate; 8] = std::array::from_fn(|_| empty());
    match data.npc_texts.get(&text_id) {
        Some(choices) => {
            for (i, c) in choices.iter().enumerate() {
                let Some(b) = data.broadcast_texts.get(&c.broadcast) else {
                    continue;
                };
                let line = data
                    .broadcast_line(c.broadcast, female)
                    .unwrap_or_default()
                    .to_owned();
                texts[i] = NpcTextUpdate {
                    probability: c.probability,
                    texts: [line.clone(), line],
                    language: Language::try_from(b.language).unwrap_or(Language::Universal),
                    emotes: std::array::from_fn(|j| NpcTextUpdateEmote {
                        delay: b.emotes[j].0,
                        emote: b.emotes[j].1,
                    }),
                };
            }
        }
        None => {
            let line = "Greetings, $N.".to_owned();
            texts[0] = NpcTextUpdate {
                probability: 1.0,
                texts: [line.clone(), line],
                ..empty()
            };
        }
    }
    send(map, player, &SMSG_NPC_TEXT_UPDATE { text_id, texts });
}

/// `CMSG_TRAINER_LIST`.
pub fn trainer_list(map: &mut Map, player: Guid, npc: Guid) {
    let Some(entry) = npc_entry(map, player, npc, NPC_FLAG_TRAINER) else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    // A class trainer teaches only its class.
    if t.trainer_type == 0 && t.trainer_class != 0 && t.trainer_class != p.row.class {
        return;
    }
    let spells = data
        .npc
        .trainer_spells(entry, t.trainer_id)
        .iter()
        .map(|s| {
            let learned = crate::game::map::spells::learns(&data, s.spell);
            let known = p.spells.contains(&learned);
            let state = if known {
                TrainerSpellState::Gray
            } else if p.level() >= s.required_level {
                TrainerSpellState::Green
            } else {
                TrainerSpellState::Red
            };
            TrainerSpell {
                spell: s.spell,
                state,
                spell_cost: s.cost,
                talent_point_cost: 0,
                first_rank: 0,
                required_level: s.required_level,
                required_skill: Skill::try_from(s.required_skill as u16).unwrap_or_default(),
                required_skill_value: s.required_skill_value,
                required_spells: [0; 3],
            }
        })
        .collect();
    let greeting = data
        .npc
        .trainer_greetings
        .get(&entry)
        .cloned()
        .unwrap_or_else(|| "Hello! Ready for some training?".to_owned());
    p.outbound.send(&SMSG_TRAINER_LIST {
        guid: npc,
        trainer_type: u32::from(t.trainer_type),
        spells,
        greeting,
    });
}

/// `CMSG_TRAINER_BUY_SPELL`.
pub fn trainer_buy(map: &mut Map, player: Guid, npc: Guid, spell: u32) {
    let Some(entry) = npc_entry(map, player, npc, NPC_FLAG_TRAINER) else {
        return;
    };
    let data = map.realm.data.clone();
    let trainer_id = data
        .creature_template(entry)
        .map(|t| t.trainer_id)
        .unwrap_or(0);
    let Some(s) = data
        .npc
        .trainer_spells(entry, trainer_id)
        .into_iter()
        .find(|s| s.spell == spell)
    else {
        return;
    };
    let learned = crate::game::map::spells::learns(&data, s.spell);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let fail = |p: &crate::game::map::player::Player, error| {
        p.outbound.send(&SMSG_TRAINER_BUY_FAILED {
            guid: npc,
            id: spell,
            error,
        });
    };
    if p.spells.contains(&learned) || p.level() < s.required_level {
        return fail(p, TrainingFailureReason::Unavailable);
    }
    if p.money() < s.cost {
        return fail(p, TrainingFailureReason::NotEnoughMoney);
    }
    let money = p.money() - s.cost;
    p.set_money(money);
    p.outbound.send(&SMSG_TRAINER_BUY_SUCCEEDED {
        guid: npc,
        id: spell,
    });
    crate::game::map::spells::learn(map, player, learned);
}

/// `CMSG_BINDER_ACTIVATE`: the innkeeper binds the player here.
pub fn bind(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_INNKEEPER).is_none() {
        return;
    }
    let key = map.key;
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let pos = p.position();
    p.row.state.bind = Some(crate::game::store::Bind {
        map: key.map,
        zone: p.zone,
        x: pos.x,
        y: pos.y,
        z: pos.z,
    });
    p.outbound.send(&SMSG_BINDPOINTUPDATE {
        position: pos,
        map: crate::game::map::movement::map_enum(key.map),
        area: Area::try_from(p.zone).unwrap_or_default(),
    });
    p.outbound.send(&SMSG_PLAYERBOUND {
        guid: npc,
        area: Area::try_from(p.zone).unwrap_or_default(),
    });
}

/// `CMSG_BANKER_ACTIVATE`.
pub fn banker(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_BANKER).is_some() {
        send(map, player, &SMSG_SHOW_BANK { guid: npc });
    }
}

/// `CMSG_BUY_BANK_SLOT`: one more bank bag slot.
pub fn buy_bank_slot(map: &mut Map, player: Guid, npc: Guid) {
    let banker = npc_entry(map, player, npc, NPC_FLAG_BANKER).is_some();
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let result = if !banker {
        BuyBankSlotResult::NotBanker
    } else {
        let slots = p.fields.get_byte(PLAYER_BYTES_2, 2);
        match data.npc.bank_slot_prices.get(&(u32::from(slots) + 1)) {
            None => BuyBankSlotResult::FailedTooMany,
            Some(price) if p.money() < *price => BuyBankSlotResult::InsufficientFunds,
            Some(price) => {
                let money = p.money() - price;
                p.set_money(money);
                p.fields.set_byte(PLAYER_BYTES_2, 2, slots + 1);
                BuyBankSlotResult::Ok
            }
        }
    };
    p.outbound.send(&SMSG_BUY_BANK_SLOT_RESULT { result });
}

/// `CMSG_BATTLEMASTER_HELLO`: the battleground's list.
pub fn battlemaster_hello(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_BATTLEMASTER).is_none() {
        return;
    }
    let bg_map = crate::game::map::battleground::map_for_battlemaster(map, npc);
    crate::game::map::pvp::battlefield_list(map, player, npc, bg_map);
}

/// `MSG_AUCTION_HELLO`: which house this auctioneer is of.
pub fn auction_hello(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_AUCTIONEER).is_none() {
        return;
    }
    let house = crate::game::map::auction::house_of(map, npc);
    send(
        map,
        player,
        &MSG_AUCTION_HELLO_Server {
            auctioneer: npc,
            auction_house: house,
        },
    );
}

/// The taxi node nearest a flight master.
fn node_at(map: &Map, npc: Guid) -> Option<u32> {
    let c = map.creature(npc)?;
    map.realm
        .data
        .npc
        .taxi_nodes
        .values()
        .filter(|n| n.map == map.key.map)
        .map(|n| {
            (
                n.id,
                (n.x - c.position.x).powi(2) + (n.y - c.position.y).powi(2),
            )
        })
        .filter(|(_, d)| *d < 40.0 * 40.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn knows_node(mask: &[u32], node: u32) -> bool {
    mask.get((node / 32) as usize)
        .is_some_and(|w| w & (1 << (node % 32)) != 0)
}

fn learn_node(mask: &mut Vec<u32>, node: u32) -> bool {
    let i = (node / 32) as usize;
    if mask.len() <= i {
        mask.resize(i + 1, 0);
    }
    let bit = 1 << (node % 32);
    let new = mask[i] & bit == 0;
    mask[i] |= bit;
    new
}

/// `CMSG_TAXINODE_STATUS_QUERY`.
pub fn taxi_status(map: &mut Map, player: Guid, npc: Guid) {
    let Some(node) = node_at(map, npc) else {
        return;
    };
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_TAXINODE_STATUS {
            guid: npc,
            taxi_mask_node_known: knows_node(&p.row.state.taxi_mask, node),
        });
    }
}

/// `CMSG_TAXIQUERYAVAILABLENODES`: the nodes the player knows, this one
/// learned now if it wasn't.
pub fn taxi_nodes(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_FLIGHTMASTER).is_none() {
        return;
    }
    let Some(node) = node_at(map, npc) else {
        return;
    };
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if learn_node(&mut p.row.state.taxi_mask, node) {
        p.outbound.send(&SMSG_NEW_TAXI_PATH {});
    }
    let mut nodes = p.row.state.taxi_mask.clone();
    nodes.resize(8, 0);
    p.outbound.send(&SMSG_SHOWTAXINODES {
        unknown1: 1,
        guid: npc,
        nearest_node: node,
        nodes,
    });
}

/// A flight's `SMSG_MONSTER_MOVE`: flying, every point in full.
fn flight_packet(g: Guid, from: Vector3d, walk: &Walk, spline_id: u32) -> Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, g);
    b.f32(from.x).f32(from.y).f32(from.z).u32(spline_id).u8(0);
    b.u32(0x0000_0300).u32(walk.duration.as_millis() as u32);
    let points = &walk.points[1..];
    b.u32(points.len() as u32);
    for p in points {
        b.f32(p.x).f32(p.y).f32(p.z);
    }
    b.frame(crate::game::map::ai::SMSG_MONSTER_MOVE)
}

/// A script sends `player` along taxi path `path`, free (`SCRIPT_COMMAND_SEND_TAXI_PATH`).
pub fn fly_path(map: &mut Map, player: Guid, path: u32) {
    let data = map.realm.data.clone();
    let nodes: Vec<TaxiPathNode> = data
        .npc
        .taxi_path_nodes
        .get(&path)
        .map(|n| n.iter().filter(|n| n.map == map.key.map).copied().collect())
        .unwrap_or_default();
    if nodes.len() < 2 {
        return;
    }
    let (from, to) = data
        .npc
        .taxi_paths
        .iter()
        .find(|(_, (p, _))| *p == path)
        .map(|((f, t), _)| (*f, *t))
        .unwrap_or((0, 0));
    let now = map.now;
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let mount_creature = data
        .npc
        .taxi_nodes
        .get(&from)
        .map(|n| n.mounts[if p.team == Team::Alliance { 0 } else { 1 }])
        .unwrap_or(0);
    let mount = data
        .creature_template(mount_creature)
        .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
        .unwrap_or(if p.team == Team::Alliance { 1149 } else { 295 });
    p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, mount);
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        UNIT_FLAG_TAXI_FLIGHT | UNIT_FLAG_DISABLE_MOVE,
        true,
    );
    let start = p.position();
    let mut points = vec![start];
    points.extend(nodes.iter().map(|n| Vector3d {
        x: n.x,
        y: n.y,
        z: n.z,
    }));
    let walk = Walk::new(points, TAXI_SPEED, now, spline_id, true);
    let frame = flight_packet(player, start, &walk, spline_id);
    p.flight = Some(Flight {
        walk,
        destination: to,
        mount,
    });
    map.send_to_observers(player, &frame, true);
}

/// `CMSG_ACTIVATETAXI`: pay, mount and fly.
pub fn activate_taxi(map: &mut Map, player: Guid, npc: Guid, from: u32, to: u32) {
    activate_route(map, player, npc, &[from, to]);
}

/// `CMSG_ACTIVATETAXIEXPRESS`: a route of several hops, flown as one, as
/// Thistle Tea stitches it.
pub fn activate_express(map: &mut Map, player: Guid, npc: Guid, nodes: &[u32]) {
    activate_route(map, player, npc, nodes);
}

/// Pay for each hop between `route`'s nodes, mount and fly them all.
pub fn activate_route(map: &mut Map, player: Guid, npc: Guid, route: &[u32]) {
    let reply = |map: &Map, r| send(map, player, &SMSG_ACTIVATETAXIREPLY { reply: r });
    if npc_entry(map, player, npc, NPC_FLAG_FLIGHTMASTER).is_none() {
        return reply(map, ActivateTaxiReply::NoVendorNearby);
    }
    if route.len() < 2 {
        return reply(map, ActivateTaxiReply::NoSuchPath);
    }
    let data = map.realm.data.clone();
    let mut cost = 0u32;
    let mut nodes: Vec<TaxiPathNode> = Vec::new();
    for hop in route.windows(2) {
        let Some(&(path, hop_cost)) = data.npc.taxi_paths.get(&(hop[0], hop[1])) else {
            return reply(map, ActivateTaxiReply::NoSuchPath);
        };
        cost = cost.saturating_add(hop_cost);
        let hop_nodes = data
            .npc
            .taxi_path_nodes
            .get(&path)
            .map(|n| n.iter().filter(|n| n.map == map.key.map).copied().collect::<Vec<_>>())
            .unwrap_or_default();
        if hop_nodes.len() < 2 {
            return reply(map, ActivateTaxiReply::NoSuchPath);
        }
        nodes.extend(hop_nodes);
    }
    let (from, to) = (route[0], route[route.len() - 1]);
    let now = map.now;
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if route[1..].iter().any(|n| !knows_node(&p.row.state.taxi_mask, *n)) {
        p.outbound.send(&SMSG_ACTIVATETAXIREPLY {
            reply: ActivateTaxiReply::NoSuchPath,
        });
        return;
    }
    if p.money() < cost {
        p.outbound.send(&SMSG_ACTIVATETAXIREPLY {
            reply: ActivateTaxiReply::NotEnoughMoney,
        });
        return;
    }
    let money = p.money() - cost;
    p.set_money(money);
    p.outbound.send(&SMSG_ACTIVATETAXIREPLY {
        reply: ActivateTaxiReply::Ok,
    });
    let mount_creature = data
        .npc
        .taxi_nodes
        .get(&from)
        .map(|n| n.mounts[if p.team == Team::Alliance { 0 } else { 1 }])
        .unwrap_or(0);
    let mount = data
        .creature_template(mount_creature)
        .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
        .unwrap_or(if p.team == Team::Alliance { 1149 } else { 295 });
    p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, mount);
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        UNIT_FLAG_TAXI_FLIGHT | UNIT_FLAG_DISABLE_MOVE,
        true,
    );
    let start = p.position();
    let mut points = vec![start];
    points.extend(nodes.iter().map(|n| Vector3d {
        x: n.x,
        y: n.y,
        z: n.z,
    }));
    let walk = Walk::new(points, TAXI_SPEED, now, spline_id, true);
    let frame = flight_packet(player, start, &walk, spline_id);
    p.flight = Some(Flight {
        walk,
        destination: to,
        mount,
    });
    map.send_to_observers(player, &frame, true);
}

/// A flight under way at logout goes on from where the player is.
pub fn resume_flight(map: &mut Map, player: Guid) {
    let now = map.now;
    let spline_id = map.next_spline_id;
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(saved) = p.row.state.flight.take() else {
        return;
    };
    if saved.map != map.key.map || saved.points.is_empty() {
        return;
    }
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let start = p.position();
    let mut points = vec![start];
    points.extend(saved.points.iter().map(|(x, y, z)| Vector3d {
        x: *x,
        y: *y,
        z: *z,
    }));
    p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, saved.mount);
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        UNIT_FLAG_TAXI_FLIGHT | UNIT_FLAG_DISABLE_MOVE,
        true,
    );
    let walk = Walk::new(points, TAXI_SPEED, now, spline_id, true);
    let frame = flight_packet(player, start, &walk, spline_id);
    p.flight = Some(Flight {
        walk,
        destination: saved.destination,
        mount: saved.mount,
    });
    p.outbound.send_frame(frame.clone());
    map.send_to_observers(player, &frame, false);
}

/// Players in flight move along; one who lands is let go.
pub fn update_flights(map: &mut Map) {
    let now = map.now;
    let flying: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.flight.is_some())
        .map(|p| p.guid)
        .collect();
    for g in flying {
        let Some(p) = map.players.get_mut(&g) else {
            continue;
        };
        let Some(f) = p.flight.as_ref() else {
            continue;
        };
        let done = f.walk.done(now);
        let (pos, o) = if done {
            (f.walk.destination(), p.movement.orientation)
        } else {
            f.walk.position_at(now)
        };
        p.movement.position = pos;
        p.movement.orientation = o;
        let (anchor, old_cell) = (p.vis_anchor, p.cell);
        let new_cell = crate::game::map::grid::cell_of(pos.x, pos.y);
        p.cell = new_cell;
        if done {
            p.flight = None;
            p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
            p.fields.set_flag(
                UNIT_FIELD_FLAGS,
                UNIT_FLAG_TAXI_FLIGHT | UNIT_FLAG_DISABLE_MOVE,
                false,
            );
        }
        map.grid.relocate(g, old_cell, new_cell);
        crate::game::map::visibility::moved(map, g, anchor, pos);
        if done {
            map.teleport(g, map.key, pos, o);
        }
    }
}
