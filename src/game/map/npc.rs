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
pub const NPC_FLAG_SPIRITGUIDE: u32 = 0x40;
pub const NPC_FLAG_INNKEEPER: u32 = 0x80;
pub const NPC_FLAG_BANKER: u32 = 0x100;
pub const NPC_FLAG_PETITIONER: u32 = 0x200;
pub const NPC_FLAG_TABARDDESIGNER: u32 = 0x400;
pub const NPC_FLAG_BATTLEMASTER: u32 = 0x800;
pub const NPC_FLAG_AUCTIONEER: u32 = 0x1000;

const UNIT_FLAG_TAXI_FLIGHT: u32 = 0x0010_0000;
const UNIT_FLAG_DISABLE_MOVE: u32 = 0x0000_0004;
const UNIT_FLAG_NOT_SELECTABLE: u32 = 0x0200_0000;
/// A flight's speed, in yards a second.
const TAXI_SPEED: f32 = 32.0;
/// How near, on each axis, a flight resumed is to what is left of it to
/// have landed (Thistle Tea's `@move_epsilon`).
const FLIGHT_LANDED: f32 = 0.1;
/// The text an NPC without its own shows.
pub const DEFAULT_TEXT: u32 = 68;
/// A gossip option that opens the stable.
const GOSSIP_OPTION_STABLEPET: u32 = 14;

/// A flight in progress.
#[derive(Debug, Clone)]
pub struct Flight {
    pub walk: Walk,
    pub destination: u32,
    pub mount: u32,
}

/// vmangos's `INTERACTION_DISTANCE`, past both bodies.
pub const INTERACTION_YARDS: f32 = 5.0;

/// Whether `player` is near enough `npc` to use it (Thistle Tea's
/// `NpcReach`): five yards past both their bounding radii.
pub fn within_reach(map: &Map, player: Guid, npc: Guid) -> bool {
    let radius = |g: Guid| {
        map.unit_fields(g)
            .map_or(0.0, |f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS).max(0.0))
    };
    let (Some(p), Some(at)) = (map.players.get(&player), map.position_of(npc)) else {
        return false;
    };
    dist(p.position(), at) <= INTERACTION_YARDS + radius(player) + radius(npc)
}

/// The creature `npc`, if `player` may talk to it and it has `flag`: its
/// entry.
pub fn npc_entry(map: &Map, player: Guid, npc: Guid, flag: u32) -> Option<u32> {
    let p = map.players.get(&player)?;
    let c = map.creature(npc)?;
    let flags = c.fields.get_u32(UNIT_NPC_FLAGS);
    if !c.alive() || (flag != 0 && flags & flag == 0) {
        return None;
    }
    // The dead talk to spirit healers and guides alone; no one in flight
    // or out of their own control, nor to a creature hostile to them
    // (vmangos's `GetNPCIfCanInteractWith`, Thistle Tea's service checks).
    let spirit = flags & (NPC_FLAG_SPIRITHEALER | NPC_FLAG_SPIRITGUIDE) != 0;
    if p.death != crate::game::map::death::Death::Alive && !spirit {
        return None;
    }
    use crate::game::map::auras::{
        AURA_MOD_CHARM, AURA_MOD_CONFUSE, AURA_MOD_FEAR, AURA_MOD_POSSESS, AURA_MOD_STUN,
    };
    let held = [
        AURA_MOD_STUN,
        AURA_MOD_FEAR,
        AURA_MOD_CONFUSE,
        AURA_MOD_CHARM,
        AURA_MOD_POSSESS,
    ]
    .iter()
    .any(|a| p.auras.has(*a));
    if p.flight.is_some() || held || crate::game::map::combat::hostile(map, npc, player) {
        return None;
    }
    // Nor to one unfriendly or worse, either way: a standing of Unfriendly
    // with its faction (mangos's `CanInteract`, both reactions above
    // `REP_UNFRIENDLY`).
    let unfriendly = |a: Guid, b: Guid| {
        crate::game::map::reputation::reaction_rank(map, a, b)
            .is_some_and(|r| r <= crate::game::data::misc::RANK_UNFRIENDLY)
    };
    if unfriendly(npc, player) || unfriendly(player, npc) {
        return None;
    }
    // Not one fighting, nor one charmed (mangos's `CanInteractNow`); a
    // stable master serves hunters alone (`GetNPCIfCanInteractWith`).
    let fighting = map
        .combat(npc)
        .is_some_and(|f| f.attacking && !f.victim.is_zero());
    if fighting || !c.fields.get_guid(UNIT_FIELD_CHARMEDBY).is_zero() {
        return None;
    }
    // Nor one that can't be selected (mangos's `CanInteract`,
    // `UNIT_FLAG_UNINTERACTIBLE`).
    if c.fields.get_u32(UNIT_FIELD_FLAGS) & UNIT_FLAG_NOT_SELECTABLE != 0 {
        return None;
    }
    if flag == crate::game::map::pets::NPC_FLAG_STABLEMASTER && p.row.class != 3 {
        return None;
    }
    if !within_reach(map, player, npc) {
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
/// The menu an NPC opens with: its script's, else its template's.
fn creature_menu(map: &Map, npc: Guid) -> u32 {
    let entry = map.creature(npc).map_or(0, |c| c.entry);
    crate::game::map::event_ai::scripted(map, npc)
        .and_then(|s| s.gossip_menu)
        .or_else(|| {
            map.realm
                .data
                .creature_template(entry)
                .map(|t| t.gossip_menu_id)
        })
        .unwrap_or(0)
}

pub fn hello(map: &mut Map, player: Guid, npc: Guid) {
    if crate::game::map::death::spirit_healer_hello(map, player, npc)
        || crate::game::map::battleground::gossip_hello(map, player, npc)
    {
        return;
    }
    let Some(entry) = npc_entry(map, player, npc, 0) else {
        return;
    };
    crate::game::map::creature_motion::talk_pause(map, npc);
    if crate::game::map::escorts::gossip_hello(map, player, npc) {
        return;
    }
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
    let hunter = is_hunter(map, player);
    // No menu at all (none in the data, not even the default): the NPC's
    // one service. A menu whose options aren't for this player is still
    // shown, its greeting alone (mangos's `PrepareGossipMenu`).
    let no_menu = menu_id == 0 && !data.npc.gossip_menus.contains_key(&0);
    if options.is_empty() && quests.is_empty() && no_menu {
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
            return crate::game::map::bank::show(map, player, npc);
        }
        if flags & NPC_FLAG_AUCTIONEER != 0 {
            return auction_hello(map, player, npc);
        }
        if flags & NPC_FLAG_BATTLEMASTER != 0 {
            return battlemaster_hello(map, player, npc);
        }
        // A stable master's stable is a hunter's alone; anyone else gets
        // its greeting (mangos's `PrepareGossipMenu`).
        if flags & crate::game::map::pets::NPC_FLAG_STABLEMASTER != 0 && hunter {
            return crate::game::map::pets::list_stable(map, player, npc);
        }
    }
    show_menu(map, player, npc, menu_id, flags);
}

/// Hunters stable pets.
fn is_hunter(map: &Map, player: Guid) -> bool {
    map.players.get(&player).is_some_and(|p| p.row.class == 3)
}

/// The options of a menu this NPC offers. An NPC's own menu with no
/// options of its own offers the default menu's (0) its flags allow, as
/// mangos's `PrepareGossipMenu` and Thistle Tea's inherited defaults: a
/// battlemaster's "I wish to join the battle!".
fn menu_options(
    map: &Map,
    menu_id: u32,
    npc_flags: u32,
    player: Guid,
    npc: Guid,
) -> Vec<GossipOption> {
    let menus = &map.realm.data.npc.gossip_menus;
    let own_menu = !crate::game::guid::is_game_object(npc) && menu_id == creature_menu(map, npc);
    let menu = menus
        .get(&menu_id)
        .filter(|m| !m.options.is_empty())
        .or_else(|| (menu_id != 0 && own_menu).then(|| menus.get(&0)).flatten());
    menu.map(|m| {
        m.options
            .iter()
            // An object's menu shows its gossip options alone, whatever
            // their NPC flag (mangos's `PrepareGossipMenu`, Thistle
            // Tea's `npc_flag_allowed?`).
            .filter(|o| {
                if crate::game::guid::is_game_object(npc) {
                    o.kind == 1
                } else {
                    o.npc_flag == 0 || o.npc_flag & npc_flags != 0
                }
            })
            .filter(|o| {
                o.condition == 0
                    || !map.realm.data.scripts.conditions.contains_key(&o.condition)
                    || crate::game::map::conditions::met(map, o.condition, Some(npc), Some(player))
            })
            // The stable is for hunters (mangos's `GOSSIP_OPTION_STABLEPET`
            // in `PrepareGossipMenu`).
            .filter(|o| o.kind != GOSSIP_OPTION_STABLEPET || is_hunter(map, player))
            .filter(|o| crate::game::guid::is_game_object(npc) || offered(map, o.kind, player, npc))
            .cloned()
            .collect()
    })
    .unwrap_or_default()
}

/// Whether a creature's option of `kind` is for `player`, as mangos's
/// `PrepareGossipMenu` has each: the quest giver's and the armorer's
/// placeholders never (the quests are listed below the options); a
/// spirit healer's to the dead; a vendor's with something to sell; a
/// trainer's to those it teaches, with something to teach; unlearning
/// talents at one's own class trainer from level 10; untraining a pet to
/// a hunter whose pet has spells, at a hunter's pet trainer; a
/// battlemaster's to those of its battleground's levels. Options mangos
/// doesn't know aren't shown.
fn offered(map: &Map, kind: u32, player: Guid, npc: Guid) -> bool {
    let data = &map.realm.data;
    let (Some(p), Some(c)) = (map.players.get(&player), map.creature(npc)) else {
        return false;
    };
    let template = data.creature_template(c.entry);
    match kind {
        1 | 4 | 7..=11 | 13 | 14 | 100..=104 => true,
        6 => p.death != crate::game::map::death::Death::Alive,
        3 => !data.vendor_items(c.entry).is_empty(),
        5 => template.is_some_and(|t| {
            !data.npc.trainer_spells(c.entry, t.trainer_id).is_empty() && trains(data, p, t)
        }),
        16 => template.is_some_and(|t| {
            p.level() >= 10 && t.trainer_type == 0 && t.trainer_class == p.row.class
        }),
        17 => {
            template.is_some_and(|t| t.trainer_type == 3 && t.trainer_class == 3)
                && p.owned
                    .pet
                    .is_some_and(|pet| crate::game::map::pets::untrainable(map, player, pet))
        }
        12 => {
            let bg = crate::game::battlegrounds::bg_of_map(
                crate::game::map::battleground::map_for_battlemaster(map, npc),
            );
            let level = u32::from(p.level());
            bg.and_then(|bg| data.battlegrounds.get(&bg))
                .is_none_or(|t| level >= t.min_level && level <= t.max_level)
        }
        _ => false,
    }
}

/// A menu's greeting for `player`, as Thistle Tea picks it: of the texts
/// whose condition holds (or that have none), the one with the highest
/// condition, the lowest text of those; with none, the default.
pub fn greeting(map: &Map, menu_id: u32, npc: Guid, player: Guid) -> u32 {
    let conditions = &map.realm.data.scripts.conditions;
    map.realm
        .data
        .npc
        .gossip_menus
        .get(&menu_id)
        .and_then(|m| {
            m.texts
                .iter()
                .filter(|(_, cond)| {
                    *cond == 0
                        || !conditions.contains_key(cond)
                        || crate::game::map::conditions::met(map, *cond, Some(npc), Some(player))
                })
                .min_by_key(|(text, cond)| (std::cmp::Reverse(*cond), *text))
        })
        .map(|(t, _)| *t)
        // With no menu, a creature spawn's own (`npc_gossip`); else the
        // default (mangos's `SendPreparedGossip` and `GetGossipTextId`).
        .or_else(|| {
            if menu_id != 0 {
                return None;
            }
            let spawn = map.creature(npc)?.spawn_guid?;
            map.realm.data.npc.npc_gossip.get(&spawn).copied()
        })
        .unwrap_or(DEFAULT_TEXT)
}

pub fn show_menu(map: &mut Map, player: Guid, npc: Guid, menu_id: u32, npc_flags: u32) {
    let data = map.realm.data.clone();
    let text = greeting(map, menu_id, npc, player);
    // The script of the greeting shown (mangos's `GetGossipTextId`).
    if let Some(script) = data
        .scripts
        .gossip_menu_script
        .get(&(menu_id, text))
        .copied()
    {
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::Gossip,
            script,
            npc,
            player,
        );
    }
    let options = menu_options(map, menu_id, npc_flags, player, npc);
    // A flight master's node is learned as its menu is shown; with no
    // quests to list, the discovery is all the player is sent (mangos's
    // `PrepareGossipMenu`, `SendLearnNewTaxiNode`, and
    // `SendPreparedGossip`'s `IsJustDiscoveredNode`).
    let discovered = !crate::game::guid::is_game_object(npc)
        && options.iter().any(|o| o.kind == 4)
        && discover_node(map, player, npc);
    let gossips = options
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
        if discovered && quests.is_empty() {
            return;
        }
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
    let data = map.realm.data.clone();
    // An object's menu, as Thistle Tea's `source_allowed?` lets it be
    // chosen from: the living player within reach of it.
    let default_menu = if crate::game::guid::is_game_object(npc) {
        let alive = map
            .players
            .get(&player)
            .is_some_and(|p| p.death == crate::game::map::death::Death::Alive);
        if !alive || !crate::game::map::objects::within_reach(map, player, npc) {
            return;
        }
        map.gameobject(npc)
            .and_then(|o| data.gameobject_templates.get(&o.entry))
            .map_or(0, |t| if t.kind == 10 { t.data[19] } else { t.data[3] })
    } else {
        let Some(entry) = npc_entry(map, player, npc, 0) else {
            return;
        };
        crate::game::map::creature_motion::talk_pause(map, npc);
        if crate::game::map::escorts::gossip_select(map, player, npc, option) {
            return;
        }
        data.creature_template(entry)
            .map_or(0, |t| t.gossip_menu_id)
    };
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
        .unwrap_or(default_menu);
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
    // An option that closes a creature's gossip is talking to it, once the
    // window is closed (mangos's GOSSIP_OPTION_GOSSIP with a menu below 0).
    let talked = o.kind == 1 && o.action_menu < 0;
    if o.script != 0 {
        // An object's option runs on the player, at the object; a
        // creature's on the creature, at the player (Thistle Tea's
        // `dispatch`).
        let (source, target) = if crate::game::guid::is_game_object(npc) {
            (player, npc)
        } else {
            (npc, player)
        };
        crate::game::map::scripts::start(
            map,
            crate::game::data::scripts::ScriptTable::Gossip,
            o.script,
            source,
            target,
        );
        if o.kind == 1 && o.action_menu <= 0 {
            send(map, player, &SMSG_GOSSIP_COMPLETE {});
            if talked {
                quests::talked(map, player, npc);
            }
            return;
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
        9 => crate::game::map::bank::show(map, player, npc),
        12 => battlemaster_hello(map, player, npc),
        13 => auction_hello(map, player, npc),
        14 => crate::game::map::pets::list_stable(map, player, npc),
        // The window closes, and the client asks to confirm (mangos's
        // `GOSSIP_OPTION_UNLEARNPETSKILLS`).
        17 => {
            send(map, player, &SMSG_GOSSIP_COMPLETE {});
            crate::game::map::pets::untrain_offer(map, player)
        }
        15 => vendor::list(map, player, npc),
        16 => crate::game::map::misc::talent_wipe_offer(map, player, npc),
        // The window closes, and the guild master's charter, or the tabard
        // designer, opens (mangos's `GOSSIP_OPTION_PETITIONER` and
        // `GOSSIP_OPTION_TABARDDESIGNER`).
        10 => {
            send(map, player, &SMSG_GOSSIP_COMPLETE {});
            if npc_entry(map, player, npc, NPC_FLAG_PETITIONER).is_some() {
                crate::game::social::guilds::petition_showlist(map, player, npc);
            }
        }
        11 => {
            send(map, player, &SMSG_GOSSIP_COMPLETE {});
            if npc_entry(map, player, npc, NPC_FLAG_TABARDDESIGNER).is_some() {
                send(map, player, &MSG_TABARDVENDOR_ACTIVATE { guid: npc });
            }
        }
        _ => send(map, player, &SMSG_GOSSIP_COMPLETE {}),
    }
    if talked {
        quests::talked(map, player, npc);
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

/// The professions a player may have two of.
pub(crate) const PRIMARY_PROFESSIONS: [u32; 9] = [164, 165, 171, 182, 186, 197, 202, 333, 393];
const EFFECT_SKILL_STEP: u32 = 44;

/// What a trainer's spell is to a player, as Thistle Tea's `Trainer`
/// works it out.
struct Teaching {
    state: TrainerSpellState,
    /// A primary profession's first rank, and whether a profession is
    /// free for it.
    first_profession: bool,
    free_profession: bool,
    prev: u32,
    req: u32,
}

/// The profession rank a trainer's spell teaches: its skill and the rank's
/// most.
fn skill_step(data: &crate::game::data::GameData, teach: u32) -> Option<(u32, u32)> {
    data.spells.get(teach)?.effects.iter().find_map(|e| {
        (e.effect == EFFECT_SKILL_STEP && e.misc_value > 0)
            .then(|| (e.misc_value as u32, (e.base_points.max(0) as u32 + 1) * 75))
    })
}

/// Whether a spell is for a player's class and race (`SkillLineAbility`).
pub fn fits_class_race(
    data: &crate::game::data::GameData,
    teach: u32,
    class: u8,
    race: u8,
) -> bool {
    let learned = crate::game::map::spells::learns(data, teach);
    let Some(lines) = data
        .spells
        .skill_abilities
        .get(&learned)
        .filter(|l| !l.is_empty())
    else {
        return true;
    };
    let (class_bit, race_bit) = (1u32 << (class.max(1) - 1), 1u32 << (race.max(1) - 1));
    lines.iter().any(|a| {
        (a.class_mask == 0 || a.class_mask & class_bit != 0)
            && (a.race_mask == 0 || a.race_mask & race_bit != 0)
    })
}

/// Gray if known, red if a level, an earlier rank or a skill is missing,
/// else green; and a first profession rank's slot, of the `most` a
/// player may have. The rank before a talent's trained rank is the one in
/// the talent's line (Mortal Strike's second wants the talent), as
/// mangos-classic's `GetTrainerSpellState` asks for its chain's:
/// vmangos's `spell_chain` doesn't chain them.
fn teaching(
    data: &crate::game::data::GameData,
    p: &crate::game::map::player::Player,
    most: usize,
    s: &crate::game::data::npc::TrainerSpell,
) -> Teaching {
    use crate::game::map::skills;
    let learned = crate::game::map::spells::learns(data, s.spell);
    let step = skill_step(data, s.spell);
    let mut rank = data.spells.ranks.get(&learned).copied().unwrap_or_default();
    if rank.prev == 0
        && let Some(prev) = data.spells.talent_prev(learned)
    {
        rank.prev = prev;
    }
    let missing = |spell: u32| spell != 0 && !p.spells.contains(&spell);
    let known = p.spells.contains(&learned)
        || step.is_some_and(|(skill, max)| skills::max_value(p, skill).is_some_and(|m| m >= max));
    let state = if known {
        TrainerSpellState::Gray
    } else if p.level() < s.required_level
        || missing(rank.prev)
        || missing(rank.req)
        || (s.required_skill > 0
            && skills::value(p, s.required_skill).unwrap_or(0) < s.required_skill_value)
    {
        TrainerSpellState::Red
    } else {
        TrainerSpellState::Green
    };
    let first_profession =
        step.is_some_and(|(skill, max)| PRIMARY_PROFESSIONS.contains(&skill) && max == 75);
    let primaries = PRIMARY_PROFESSIONS
        .iter()
        .filter(|sk| skills::value(p, **sk).is_some())
        .count();
    let already = step.is_some_and(|(skill, _)| skills::value(p, skill).is_some());
    Teaching {
        state,
        first_profession,
        // `[players] max_primary_trade_skill`, mangos's two.
        free_profession: already || primaries < most,
        prev: rank.prev,
        req: rank.req,
    }
}

/// Whether trainer `t` teaches `p`, as mangos's `IsTrainerOf` and Thistle
/// Tea's `trainer_of?`: a class trainer its class; a pet trainer hunters; a
/// mount trainer of a race that race, or another at exalted with the
/// trainer's faction.
pub fn trains(
    data: &crate::game::data::GameData,
    p: &crate::game::map::player::Player,
    t: &crate::game::data::CreatureTemplate,
) -> bool {
    match t.trainer_type {
        0 => t.trainer_class == 0 || t.trainer_class == p.row.class,
        1 => {
            t.trainer_race == 0
                || t.trainer_race == p.row.race
                || data.faction_templates.get(&t.faction).is_some_and(|f| {
                    crate::game::map::reputation::rank(p, data, f.faction) >= EXALTED
                })
        }
        3 => p.row.class == 3,
        // A profession's trainer teaches those who have the profession it
        // asks for (`TRAINER_TYPE_TRADESKILLS`, `TrainerSpell`).
        2 => t.trainer_spell == 0 || p.spells.contains(&t.trainer_spell),
        _ => true,
    }
}

/// The reputation rank of the exalted.
const EXALTED: u32 = 7;

/// `CMSG_TRAINER_LIST`.
pub fn trainer_list(map: &mut Map, player: Guid, npc: Guid) {
    let Some(entry) = npc_entry(map, player, npc, NPC_FLAG_TRAINER) else {
        return;
    };
    let data = map.realm.data.clone();
    let most = map.cfg.players.max_primary_trade_skill as usize;
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let Some(p) = map.players.get(&player) else {
        return;
    };
    if !trains(&data, p, t) {
        return;
    }
    let spells = data
        .npc
        .trainer_spells(entry, t.trainer_id)
        .iter()
        .filter(|s| fits_class_race(&data, s.spell, p.row.class, p.row.race))
        .map(|s| {
            let t = teaching(&data, p, most, s);
            TrainerSpell {
                spell: s.spell,
                state: t.state,
                spell_cost: crate::game::map::reputation::price(map, player, npc, s.cost),
                // Equal, the learn button is on: a first profession rank
                // needs a free profession.
                talent_point_cost: u32::from(t.first_profession && t.free_profession),
                first_rank: u32::from(t.first_profession),
                required_level: s.required_level,
                required_skill: Skill::try_from(s.required_skill as u16).unwrap_or_default(),
                required_skill_value: s.required_skill_value,
                // The rank before first, then the spell required; with no
                // rank before, the required spell first, as mangos-zero's
                // SendTrainerSpellHelper and Thistle Tea lay them out.
                required_spells: if t.prev != 0 {
                    [t.prev, t.req, 0]
                } else {
                    [t.req, 0, 0]
                },
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
    let most = map.cfg.players.max_primary_trade_skill as usize;
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
    let cost = crate::game::map::reputation::price(map, player, npc, s.cost);
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
    let trainer = data.creature_template(entry);
    let t = teaching(&data, p, most, &s);
    let learnable = t.state == TrainerSpellState::Green
        && (!t.first_profession || t.free_profession)
        && trainer.is_some_and(|tr| trains(&data, p, tr))
        && fits_class_race(&data, s.spell, p.row.class, p.row.race);
    if !learnable {
        return fail(p, TrainingFailureReason::Unavailable);
    }
    if p.money() < cost {
        return fail(p, TrainingFailureReason::NotEnoughMoney);
    }
    let money = p.money() - cost;
    p.set_money(money);
    p.outbound.send(&SMSG_TRAINER_BUY_SUCCEEDED {
        guid: npc,
        id: spell,
    });
    crate::game::map::spells::learn(map, player, learned);
}

/// Every spell `player`'s class trainers would teach them now, each as
/// [`trainer_buy`] teaches it but free (`.character spells`): what their
/// class's trainers have for their class and race, each once its level,
/// its rank before and the spell it needs are had, so the ranks come one
/// after another up to the highest for their level; a talent's ranks
/// (and those trained after it) only to one with the talent. Pet and
/// profession trainers (weapon masters among them) aren't their class's.
/// First, the class spells their class's quests and books teach and no
/// trainer does, at their levels, for their race
/// ([`crate::game::data::GameData::quest_class_spells`]): a priest's
/// racial spells, whose later ranks the trainers then teach. The number
/// learned.
pub fn learn_class_spells(map: &mut Map, player: Guid) -> usize {
    let data = map.realm.data.clone();
    let most = map.cfg.players.max_primary_trade_skill as usize;
    let Some(p) = map.players.get(&player) else {
        return 0;
    };
    let (class, race) = (p.row.class, p.row.race);
    let mut offered: Vec<crate::game::data::npc::TrainerSpell> = data
        .creature_templates
        .values()
        .filter(|t| t.trainer_type == 0 && t.trainer_class != 0 && t.trainer_class == class)
        .flat_map(|t| data.npc.trainer_spells(t.entry, t.trainer_id))
        .filter(|s| fits_class_race(&data, s.spell, class, race))
        .collect();
    offered.sort_by_key(|s| (s.required_level, s.spell));
    offered.dedup_by_key(|s| s.spell);
    // A talent's rank, or one trained after it (Mortal Strike's), only to
    // one with a rank of the talent, as Thistle Tea's
    // `trainable_spell_ids`: vmangos's `spell_chain` doesn't chain them to
    // the talent, so the rank before isn't asked for.
    let talents: Vec<u32> = map.players.get(&player).map_or_else(Vec::new, |p| {
        p.spells
            .iter()
            .filter_map(|s| data.spells.talent_of(*s))
            .collect()
    });
    offered.retain(|s| {
        let learned = crate::game::map::spells::learns(&data, s.spell);
        data.spells
            .talent_of(learned)
            .is_none_or(|t| talents.contains(&t))
    });
    let mut learned = 0;
    // The class spells their quests and books teach, that no trainer
    // does, at their levels: the trainers' later ranks of them follow.
    let level = map.players.get(&player).map_or(0, |p| p.level());
    let (class_bit, race_bit) = (1u32 << (class.max(1) - 1), 1u32 << (race.max(1) - 1));
    let quested: Vec<u32> = data
        .quest_class_spells()
        .iter()
        .filter(|q| {
            q.level <= level
                && q.classes & class_bit != 0
                && (q.races == 0 || q.races & race_bit != 0)
                && fits_class_race(&data, q.spell, class, race)
        })
        .map(|q| q.spell)
        .collect();
    for spell in quested {
        if map
            .players
            .get(&player)
            .is_none_or(|p| p.spells.contains(&spell))
        {
            continue;
        }
        crate::game::map::spells::learn(map, player, spell);
        if map
            .players
            .get(&player)
            .is_some_and(|p| p.spells.contains(&spell))
        {
            learned += 1;
        }
    }
    // A pass for each rank at most: each teaches what the last made
    // green.
    while let Some(p) = map.players.get(&player) {
        let green: Vec<u32> = offered
            .iter()
            .filter(|s| {
                let t = teaching(&data, p, most, s);
                t.state == TrainerSpellState::Green && (!t.first_profession || t.free_profession)
            })
            .map(|s| crate::game::map::spells::learns(&data, s.spell))
            .collect();
        let mut more = false;
        for spell in green {
            let known = map
                .players
                .get(&player)
                .is_none_or(|p| p.spells.contains(&spell));
            if known {
                continue;
            }
            crate::game::map::spells::learn(map, player, spell);
            if map
                .players
                .get(&player)
                .is_some_and(|p| p.spells.contains(&spell))
            {
                learned += 1;
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    learned
}

/// The innkeeper's Bind, and the Hearthstone it gives one without.
const SPELL_BIND: u32 = 3286;
const HEARTHSTONE: u32 = 6948;

/// `CMSG_BINDER_ACTIVATE`: the innkeeper binds the player here, as
/// Thistle Tea's `HomeBind`: not in a dungeon or battleground copy. The
/// innkeeper casts Bind on them, whose effects bind them ([`bind_home`])
/// and give them a Hearthstone if they have none; without the spell, both
/// are done here.
pub fn bind(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_INNKEEPER).is_none() {
        return;
    }
    let key = map.key;
    let kind = crate::game::instances::instance_type(&map.realm, key.map);
    if key.instance != 0 || matches!(kind, 1..=4) {
        return;
    }
    send(map, player, &SMSG_GOSSIP_COMPLETE {});
    if map.realm.data.spells.get(SPELL_BIND).is_some() {
        crate::game::map::spells::cast_triggered(map, npc, SPELL_BIND, Some(player));
        return;
    }
    bind_home(map, player, npc);
    let lacks = map
        .players
        .get(&player)
        .is_some_and(|p| p.count_of(HEARTHSTONE) == 0);
    if lacks && map.realm.data.item(HEARTHSTONE).is_some() {
        let _ = crate::game::map::inventory::add_item(map, player, HEARTHSTONE, 1, true);
    }
}

/// A player bound where they stand, to the area they stand in, by
/// `binder`: an innkeeper's Bind (Thistle Tea's `HomeBind.complete`), told
/// whose it was; or a Bind they cast themselves.
pub fn bind_home(map: &mut Map, player: Guid, binder: Guid) {
    let key = map.key;
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let pos = p.position();
    let area = if p.area != 0 { p.area } else { p.zone };
    p.row.state.bind = Some(crate::game::store::Bind {
        map: key.map,
        zone: area,
        x: pos.x,
        y: pos.y,
        z: pos.z,
    });
    p.outbound.send(&SMSG_BINDPOINTUPDATE {
        position: pos,
        map: crate::game::map::movement::map_enum(key.map),
        area: Area::try_from(area).unwrap_or_default(),
    });
    if binder != player {
        p.outbound.send(&SMSG_PLAYERBOUND {
            guid: binder,
            area: Area::try_from(area).unwrap_or_default(),
        });
    }
}

/// `CMSG_BANKER_ACTIVATE`.
pub fn banker(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_BANKER).is_some() {
        crate::game::map::bank::show(map, player, npc);
    } else {
        crate::game::map::bank::close(map, player);
    }
}

/// `CMSG_BUY_BANK_SLOT`: one more bank bag slot.
/// The bank's bag slots.
pub const MAX_BANK_BAG_SLOTS: u8 = 6;

pub fn buy_bank_slot(map: &mut Map, player: Guid, npc: Guid) {
    let banker = npc_entry(map, player, npc, NPC_FLAG_BANKER).is_some();
    if !banker {
        crate::game::map::bank::close(map, player);
    }
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let result = if !banker {
        BuyBankSlotResult::NotBanker
    } else {
        let slots = p.fields.get_byte(PLAYER_BYTES_2, 2);
        // Six bag slots in the bank, however many prices the table has
        // (its seventh, 999,999,999, a sentinel; Thistle Tea's six).
        match data
            .npc
            .bank_slot_prices
            .get(&(u32::from(slots) + 1))
            .filter(|_| slots < MAX_BANK_BAG_SLOTS)
        {
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

/// The taxi node nearest a flight master, of those with a mount for
/// `player`'s side (or none named for either) and flown from by more than
/// spells (Thistle Tea's `Network.nearest_node`).
fn node_at(map: &Map, npc: Guid, player: Guid) -> Option<u32> {
    let c = map.creature(npc)?;
    let side = usize::from(map.players.get(&player)?.team != Team::Alliance);
    let data = &map.realm.data.npc;
    data.taxi_nodes
        .values()
        .filter(|n| {
            n.map == map.key.map
                && (n.mounts[side] != 0 || n.mounts == [0, 0])
                && !data.spell_only_nodes.contains(&n.id)
        })
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

/// Whether `mask` holds `node`. wvs keeps node N at bit N; the client
/// counts from one, node N at bit N - 1 ([`client_mask`]).
pub(crate) fn knows_node(mask: &[u32], node: u32) -> bool {
    mask.get((node / 32) as usize)
        .is_some_and(|w| w & (1 << (node % 32)) != 0)
}

/// The nodes `mask` holds as the client reads them, in eight words: node
/// N at bit N - 1 (mangos's `SetTaximaskNode`, Thistle Tea's
/// `Network.mask`), of the flight masters' nodes only.
pub fn client_mask(data: &crate::game::data::GameData, mask: &[u32]) -> Vec<u32> {
    let mut out = vec![0u32; 8];
    for node in 1..=256u32 {
        if knows_node(mask, node) && !data.npc.spell_only_nodes.contains(&node) {
            out[((node - 1) / 32) as usize] |= 1 << ((node - 1) % 32);
        }
    }
    out
}

/// A mask as the client counts it (`ChrRaces`' starting nodes), as wvs
/// keeps it.
pub fn mask_from_client(words: &[u32]) -> Vec<u32> {
    let mut out = vec![0u32; words.len() + 1];
    for (i, w) in words.iter().enumerate() {
        for bit in 0..32 {
            if w & (1 << bit) != 0 {
                let node = i as u32 * 32 + bit + 1;
                out[(node / 32) as usize] |= 1 << (node % 32);
            }
        }
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    out
}

pub(crate) fn learn_node(mask: &mut Vec<u32>, node: u32) -> bool {
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
    let Some(node) = node_at(map, npc, player) else {
        return;
    };
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_TAXINODE_STATUS {
            guid: npc,
            taxi_mask_node_known: knows_node(&p.row.state.taxi_mask, node),
        });
    }
}

/// The flight master's node learned, if it wasn't known: the player is
/// told of it (mangos's `SendLearnNewTaxiNode`).
fn discover_node(map: &mut Map, player: Guid, npc: Guid) -> bool {
    let Some(node) = node_at(map, npc, player) else {
        return false;
    };
    let Some(p) = map.players.get_mut(&player) else {
        return false;
    };
    if !learn_node(&mut p.row.state.taxi_mask, node) {
        return false;
    }
    p.outbound.send(&SMSG_NEW_TAXI_PATH {});
    p.outbound.send(&SMSG_TAXINODE_STATUS {
        guid: npc,
        taxi_mask_node_known: true,
    });
    true
}

/// `CMSG_TAXIQUERYAVAILABLENODES`: the nodes the player knows, this one
/// learned now if it wasn't.
pub fn taxi_nodes(map: &mut Map, player: Guid, npc: Guid) {
    if npc_entry(map, player, npc, NPC_FLAG_FLIGHTMASTER).is_none() {
        return;
    }
    let Some(node) = node_at(map, npc, player) else {
        return;
    };
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if learn_node(&mut p.row.state.taxi_mask, node) {
        p.outbound.send(&SMSG_NEW_TAXI_PATH {});
    }
    let nodes = client_mask(&data, &p.row.state.taxi_mask);
    p.outbound.send(&SMSG_SHOWTAXINODES {
        unknown1: 1,
        guid: npc,
        nearest_node: node,
        nodes,
    });
}

/// A flight's spline flags: run mode and flying (a Catmull-Rom spline),
/// in its `SMSG_MONSTER_MOVE` and in a create of the flier on its way.
pub const FLIGHT_SPLINE: u32 =
    crate::game::map::creature::SPLINE_RUN_MODE | crate::game::map::creature::SPLINE_FLYING;

/// A flight's `SMSG_MONSTER_MOVE`: flying, every point in full.
fn flight_packet(g: Guid, from: Vector3d, walk: &Walk, spline_id: u32) -> Arc<[u8]> {
    let mut b = Body::default();
    crate::game::update::write_packed_guid(&mut b.0, g);
    b.f32(from.x).f32(from.y).f32(from.z).u32(spline_id).u8(0);
    b.u32(FLIGHT_SPLINE).u32(walk.duration.as_millis() as u32);
    let points = &walk.points[1..];
    b.u32(points.len() as u32);
    for p in points {
        b.f32(p.x).f32(p.y).f32(p.z);
    }
    b.frame(crate::game::map::ai::SMSG_MONSTER_MOVE)
}

/// The creature whose look a flight from node `from` takes: the side's
/// own mount, else the other side's (mangos's `GetTaxiMountDisplayId`).
fn taxi_mount(data: &crate::game::data::GameData, from: u32, team: Team) -> u32 {
    let ours = usize::from(team != Team::Alliance);
    data.npc.taxi_nodes.get(&from).map_or(0, |n| {
        if n.mounts[ours] != 0 {
            n.mounts[ours]
        } else {
            n.mounts[1 - ours]
        }
    })
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
    // Not in a fight, and from near the path's first node if it has a
    // place, as mangos's `ActivateTaxiPathTo` without a flight master
    // has it (free: the fare is a flight master's).
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let fighting = p.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    );
    let far = data.npc.taxi_nodes.get(&from).is_some_and(|n| {
        let placed = n.x != 0.0 || n.y != 0.0 || n.z != 0.0;
        let at = Vector3d {
            x: n.x,
            y: n.y,
            z: n.z,
        };
        placed && (n.map != map.key.map || dist(p.position(), at) > TAXI_START_YARDS)
    });
    let refusal = if fighting {
        Some(ActivateTaxiReply::PlayerBusy)
    } else if far {
        Some(ActivateTaxiReply::TooFarAway)
    } else {
        None
    };
    if let Some(reply) = refusal {
        p.outbound.send(&SMSG_ACTIVATETAXIREPLY { reply });
        return;
    }
    let now = map.now;
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    crate::game::map::spells::interrupts::taking_flight(map, player);
    // A cast under way ends (Thistle Tea's flight start); a spell that
    // sends one is done casting by its effect.
    if map.caster(player).is_some_and(|c| c.casting.is_some()) {
        crate::game::map::spells::interrupt(map, player);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    // From a node with a mount for neither side, no mount: mangos's
    // `ActivateTaxiPathTo` lets a spell's route fly on a model of 0.
    let unmounted = data
        .npc
        .taxi_nodes
        .get(&from)
        .is_some_and(|n| n.mounts == [0, 0]);
    let mount_creature = taxi_mount(&data, from, p.team);
    let mount = data
        .creature_template(mount_creature)
        .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
        .unwrap_or(match (unmounted, p.team) {
            (true, _) => 0,
            (false, Team::Alliance) => 1149,
            (false, _) => 295,
        });
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

/// Shapeshift forms a flight takes (Thistle Tea's `disallowed_form?`):
/// none, the warrior's stances, Shadowform and Stealth.
pub const FLIGHT_FORMS: [u8; 6] = [0, 17, 18, 19, 28, 30];
/// How near the flight's first node its flier must be.
const TAXI_START_YARDS: f32 = 31.622_776;
/// The most nodes an express route names: 1.12 has 80 flight nodes, and its
/// longest shortest route passes 9 of them.
const MAX_ROUTE_NODES: usize = 128;
/// The most points a flight's path has. 1.12's longest shortest route has
/// 317 (Thistle Tea's stitch trims a few); 12 bytes each, 2,000 make a
/// 24 KB `SMSG_MONSTER_MOVE`, well inside a frame's 64 KB.
const MAX_FLIGHT_POINTS: usize = 2_000;

/// Why `player` can't take a flight along `route` now, as Thistle Tea
/// refuses it: busy (dead, flying, fighting, casting), mounted, in a form,
/// not standing, at the node already, or far from its first node.
fn taxi_refusal(map: &Map, player: Guid, route: &[u32]) -> Option<ActivateTaxiReply> {
    let p = map.players.get(&player)?;
    let busy = p.death != crate::game::map::death::Death::Alive
        || p.flight.is_some()
        || p.fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
        || p.caster.casting.is_some();
    if busy {
        return Some(ActivateTaxiReply::PlayerBusy);
    }
    if p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0 {
        return Some(ActivateTaxiReply::PlayerAlreadyMounted);
    }
    if !FLIGHT_FORMS.contains(&crate::game::map::auras::form(map, player)) {
        return Some(ActivateTaxiReply::PlayerShapeShifted);
    }
    if p.stand_state() != 0 {
        return Some(ActivateTaxiReply::NotStanding);
    }
    if route.last() == route.first() {
        return Some(ActivateTaxiReply::SameNode);
    }
    let node = map.realm.data.npc.taxi_nodes.get(route.first()?)?;
    let at = Vector3d {
        x: node.x,
        y: node.y,
        z: node.z,
    };
    let unplaced = node.x == 0.0 && node.y == 0.0 && node.z == 0.0;
    if !unplaced && (node.map != map.key.map || dist(p.position(), at) > TAXI_START_YARDS) {
        return Some(ActivateTaxiReply::TooFarAway);
    }
    None
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
    if let Some(r) = taxi_refusal(map, player, route) {
        return reply(map, r);
    }
    // The client's route is its shortest, which passes a node once: one back
    // and forth between two nodes would make a flight with no end of points.
    let mut seen = ahash::AHashSet::with_capacity(route.len());
    if route.len() > MAX_ROUTE_NODES || !route.iter().all(|n| seen.insert(*n)) {
        return reply(map, ActivateTaxiReply::NoSuchPath);
    }
    let mut cost = 0u32;
    let mut hops: Vec<(u32, Vec<TaxiPathNode>)> = Vec::new();
    for hop in route.windows(2) {
        let Some(&(path, hop_cost)) = data.npc.taxi_paths.get(&(hop[0], hop[1])) else {
            return reply(map, ActivateTaxiReply::NoSuchPath);
        };
        // A tenth off each hop from honored up.
        let hop_cost = crate::game::map::reputation::price(map, player, npc, hop_cost);
        cost = cost.saturating_add(hop_cost);
        let hop_nodes = data
            .npc
            .taxi_path_nodes
            .get(&path)
            .cloned()
            .unwrap_or_default();
        if hop_nodes.iter().filter(|n| n.map == map.key.map).count() < 2 {
            return reply(map, ActivateTaxiReply::NoSuchPath);
        }
        hops.push((path, hop_nodes));
    }
    let nodes: Vec<TaxiPathNode> = stitch(&data.npc.taxi_transitions, &hops)
        .into_iter()
        .filter(|n| n.map == map.key.map)
        .collect();
    // The flight goes out as one SMSG_MONSTER_MOVE, every point in it.
    if nodes.len() > MAX_FLIGHT_POINTS {
        return reply(map, ActivateTaxiReply::NoSuchPath);
    }
    let (from, to) = (route[0], route[route.len() - 1]);
    let now = map.now;
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if route[1..]
        .iter()
        .any(|n| !knows_node(&p.row.state.taxi_mask, *n))
    {
        p.outbound.send(&SMSG_ACTIVATETAXIREPLY {
            reply: ActivateTaxiReply::NotVisited,
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
    // A trade doesn't fly along.
    crate::game::map::trade::cancel(map, player, SMSG_TRADE_STATUS_TradeStatus::TargetToFar);
    // Out of stealth, off the mount, out of a form that can't fly.
    crate::game::map::spells::interrupts::taking_flight(map, player);
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let mount_creature = taxi_mount(&data, from, p.team);
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

/// A route's hops as one path (Thistle Tea's `stitch_paths`): each hop is
/// left at the node `taxi_path_transitions` names (else its next to last)
/// and the next joined at its (else its second).
pub fn stitch(
    transitions: &ahash::AHashMap<(u32, u32), (usize, usize)>,
    hops: &[(u32, Vec<TaxiPathNode>)],
) -> Vec<TaxiPathNode> {
    let Some((first, rest)) = hops.split_first() else {
        return Vec::new();
    };
    let mut nodes = first.1.clone();
    let mut previous = first;
    for hop in rest {
        let (in_node, out_node) = transitions.get(&(previous.0, hop.0)).copied().unwrap_or((
            previous.1.len().saturating_sub(2),
            1.min(hop.1.len().saturating_sub(1)),
        ));
        let keep = (nodes.len() + in_node + 1)
            .saturating_sub(previous.1.len())
            .min(nodes.len());
        nodes.truncate(keep);
        nodes.extend(hop.1.iter().skip(out_node).copied());
        previous = hop;
    }
    nodes
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
    // A dead passenger's flight is over where they are (Thistle Tea's
    // `Taxi.resume`).
    if saved.map != map.key.map
        || saved.points.is_empty()
        || p.death != crate::game::map::death::Death::Alive
    {
        return;
    }
    let start = p.position();
    // Saved a hair from its end, it is over: landed where they are, never
    // mounted, with no spline of no length to fly (Thistle Tea's
    // `Taxi.resume`, its `at_destination?`).
    let near = |a: f32, b: f32| (a - b).abs() <= FLIGHT_LANDED;
    if saved
        .points
        .iter()
        .all(|&(x, y, z)| near(start.x, x) && near(start.y, y) && near(start.z, z))
    {
        return;
    }
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
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

/// A flight ended where its flier is (a teleport, Thistle Tea's
/// `Taxi.cancel`): off the mount, theirs to move again.
pub fn cancel_flight(map: &mut Map, player: Guid) {
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    if p.flight.take().is_none() {
        return;
    }
    p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
    p.fields.set_flag(
        UNIT_FIELD_FLAGS,
        UNIT_FLAG_TAXI_FLIGHT | UNIT_FLAG_DISABLE_MOVE,
        false,
    );
}

/// A flight under way told again, to a client that has just taken the
/// player over (mangos's `TaxiFlightResume`): it goes on from where it
/// is, as a resumed one does. One about to land lands as it would.
pub fn retell_flight(map: &mut Map, player: Guid) {
    let now = map.now;
    let on = map.key.map;
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let Some(f) = &p.flight else {
        return;
    };
    let points: Vec<(f32, f32, f32)> = f
        .walk
        .remaining(now)
        .into_iter()
        .map(|v| (v.x, v.y, v.z))
        .collect();
    if points.is_empty() {
        return;
    }
    let (destination, mount) = (f.destination, f.mount);
    p.flight = None;
    p.row.state.flight = Some(crate::game::store::SavedFlight {
        map: on,
        points,
        destination,
        mount,
    });
    resume_flight(map, player);
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
