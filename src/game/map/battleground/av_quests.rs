//! Alterac Valley's quest-driven extras, as Thistle Tea models them: each
//! side's armor scraps and its blacksmith's defender upgrades; the
//! offerings that bring Ivus the Forest Lord and Lokholar the Ice Lord;
//! the wing commanders rescued from the enemy, their supplies and air
//! attacks; the cavalry's hides and mounts and its charge; and the mine
//! supplies that send a ground assault. Turned-in quests count toward
//! each; gossip with the NPCs who keep them shows where they stand and
//! launches them.
//!
//! What the launched NPCs then do, Thistle Tea's creature scripts for
//! them, is [`super::av_scripts`]'s.

use std::time::Instant;

use ahash::AHashMap;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::map::Map;
use crate::game::map::battleground::{self as bg, Match};

/// Stormpike Guard and Frostwolf Clan.
const REPUTATION: [u32; 2] = [730, 729];
/// The standing (Honored) a launch or an upgrade needs.
pub const HONORED: i32 = 9000;

/// Each side's blacksmith.
const SMITHS: [u32; 2] = [13257, 13176];
/// Each side's summoner.
const SUMMONERS: [u32; 2] = [13442, 13236];
pub const OFFERING_GOAL: u32 = 200;
/// Each side's cavalry commander and quartermaster, and the orders it
/// gives.
const CAVALRY: [u32; 2] = [13577, 13441];
const QUARTERMASTERS: [u32; 2] = [12096, 12097];
const ORDERS: [u32; 2] = [17353, 17442];

/// The wing commanders: entry, side, supply quest, goal, reputation per
/// turn-in, beacon item, beacon text, launch text.
struct WingCommander {
    entry: u32,
    side: usize,
    quest: u32,
    goal: u32,
    reputation: i32,
    beacon: u32,
    beacon_text: u32,
    launch_text: u32,
}

const WING_COMMANDERS: [WingCommander; 6] = [
    WingCommander {
        entry: 13179,
        side: 1,
        quest: 6825,
        goal: 90,
        reputation: 1,
        beacon: 17324,
        beacon_text: 8667,
        launch_text: 10341,
    },
    WingCommander {
        entry: 13180,
        side: 1,
        quest: 6826,
        goal: 60,
        reputation: 2,
        beacon: 17325,
        beacon_text: 8669,
        launch_text: 10343,
    },
    WingCommander {
        entry: 13181,
        side: 1,
        quest: 6827,
        goal: 30,
        reputation: 5,
        beacon: 17323,
        beacon_text: 8671,
        launch_text: 10346,
    },
    WingCommander {
        entry: 13438,
        side: 0,
        quest: 6942,
        goal: 90,
        reputation: 1,
        beacon: 17506,
        beacon_text: 8796,
        launch_text: 10351,
    },
    WingCommander {
        entry: 13439,
        side: 0,
        quest: 6941,
        goal: 60,
        reputation: 2,
        beacon: 17507,
        beacon_text: 8799,
        launch_text: 10353,
    },
    WingCommander {
        entry: 13437,
        side: 0,
        quest: 6943,
        goal: 30,
        reputation: 5,
        beacon: 17505,
        beacon_text: 8793,
        launch_text: 10349,
    },
];

/// The entries whose routes the valley walks: the wing commanders, the
/// summoners and their lords, the cavalry and the ground assault.
pub const ROUTED: [u32; 24] = [
    13179, 13180, 13181, 13438, 13439, 13437, 13236, 13442, 13419, 13256, 13577, 13441, 13446,
    13449, 13440, 13576, 13524, 13525, 13526, 13527, 13528, 13529, 13530, 13531,
];

#[derive(Debug, Clone, Copy, Default)]
pub struct Armor {
    pub scraps: u32,
    pub tier: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Offering {
    pub count: u32,
    pub launched: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FleetPhase {
    #[default]
    Prisoner,
    Returning,
    Ready,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Fleet {
    pub phase: FleetPhase,
    pub count: u32,
    pub launched: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum March {
    #[default]
    Ready,
    Assembled,
    Marching,
    Dead,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Cavalry {
    pub hides: u32,
    pub mounts: u32,
    pub phase: March,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Ground {
    pub irondeep: u32,
    pub coldtooth: u32,
    pub phase: March,
    pub quartermaster_dead: bool,
    pub commander: Option<Guid>,
}

/// What an option of a menu shown does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    ArmorStatus,
    Upgrade(u8),
    OfferingStatus,
    TakeBeacon(usize),
    LaunchAir(usize),
    LaunchCavalry,
    GroundStatus,
    TakeOrders,
    Vendor,
}

/// The beacons a side's air attack plants: the Horde's, the Alliance's.
const BEACONS: [[u32; 3]; 2] = [[178724, 178725, 178726], [178545, 178547, 178549]];
/// What a beacon calls down: a gryphon, a war rider.
const BEACON_CALLS: [u32; 2] = [13161, 13178];
pub const BEACON_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub struct Beacon {
    pub object: Guid,
    pub side: usize,
    pub ready_at: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub armor: [Armor; 2],
    pub offerings: [Offering; 2],
    /// By wing commander (index into the table).
    pub air: [Fleet; 6],
    pub cavalry: [Cavalry; 2],
    pub ground: [Ground; 2],
    menus: AHashMap<Guid, (Guid, Vec<(u32, Action)>)>,
    /// The routes walked, and the formations behind them.
    pub routes: crate::game::map::battleground::av_routes::Routes,
    /// What the NPCs' scripts keep.
    pub scripts: crate::game::map::battleground::av_scripts::State,
    /// Beacons planted and waiting.
    pub beacons: Vec<Beacon>,
    /// Beacons already seen, waiting or done.
    seen: ahash::AHashSet<Guid>,
    next_scan: Option<Instant>,
}

pub(super) fn extras(map: &Map) -> Option<&Extras> {
    map.battleground.as_ref()?.av.as_ref().map(|av| &av.extras)
}

pub(super) fn extras_mut(map: &mut Map) -> Option<&mut Extras> {
    map.battleground
        .as_mut()?
        .av
        .as_mut()
        .map(|av| &mut av.extras)
}

fn playing_side(m: &Match, g: Guid) -> Option<usize> {
    m.playing().then(|| m.team_of(g)).flatten()
}

/// The defenders' tier a side's armor has reached.
pub fn tier(m: &Match, s: usize) -> u8 {
    m.av.as_ref().map_or(0, |av| av.extras.armor[s].tier)
}

/// A quest turned in during the match: what it counts toward.
pub fn quest_rewarded(map: &mut Map, g: Guid, quest: u32) {
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let Some(s) = playing_side(m, g) else {
        return;
    };
    // Armor scraps.
    let armor_side = match quest {
        7223 | 6781 => Some(0),
        7224 | 6741 => Some(1),
        _ => None,
    };
    if armor_side == Some(s) {
        let Some(x) = extras_mut(map) else {
            return;
        };
        let before = x.armor[s];
        x.armor[s].scraps += 20;
        let after = x.armor[s];
        let first = if s == 0 { 80 } else { 81 };
        for index in 1..=4u32 {
            let state = |a: Armor| if a.scraps % 500 >= index * 100 { 0 } else { 2 };
            if state(before) != state(after) {
                bg::set_event(map, first + ((index - 1) * 2) as u8, Some(state(after)));
            }
        }
        bg::reward_team_reputation(map, s, REPUTATION[s], 1);
        return;
    }
    // Offerings.
    let offering = match quest {
        7386 => Some((0, 5)),
        6881 => Some((0, 1)),
        7385 => Some((1, 5)),
        6801 => Some((1, 1)),
        _ => None,
    };
    if let Some((side, amount)) = offering
        && side == s
    {
        let Some(x) = extras_mut(map) else {
            return;
        };
        let o = &mut x.offerings[s];
        o.count += amount;
        let launch = !o.launched && o.count >= OFFERING_GOAL;
        if launch {
            o.launched = true;
        }
        bg::reward_team_reputation(map, s, REPUTATION[s], amount as i32);
        if launch {
            launch_summoner(map, s);
        }
        return;
    }
    // A wing commander's supplies.
    if let Some(i) = WING_COMMANDERS
        .iter()
        .position(|w| w.quest == quest && w.side == s)
    {
        let Some(x) = extras_mut(map) else {
            return;
        };
        let f = &mut x.air[i];
        if f.phase != FleetPhase::Ready {
            return;
        }
        let was = f.count;
        f.count += 1;
        let ready = was < WING_COMMANDERS[i].goal && f.count >= WING_COMMANDERS[i].goal;
        bg::reward_team_reputation(map, s, REPUTATION[s], WING_COMMANDERS[i].reputation);
        if ready {
            let who = if s == 0 { "Stormpike" } else { "the Horde" };
            yell(
                map,
                WING_COMMANDERS[i].entry,
                &format!("Soldiers of {who}, come to my aid! The beacon must be planted."),
            );
        }
        return;
    }
    // The cavalry's hides and mounts.
    let cavalry = match quest {
        7026 => Some((0, false)),
        7027 => Some((0, true)),
        7002 => Some((1, false)),
        7001 => Some((1, true)),
        _ => None,
    };
    if let Some((side, mounts)) = cavalry
        && side == s
    {
        let Some(x) = extras_mut(map) else {
            return;
        };
        let c = &mut x.cavalry[s];
        if mounts {
            c.mounts += 1;
        } else {
            c.hides += 1;
        }
        let m = c.mounts % 25;
        bg::reward_team_reputation(map, s, REPUTATION[s], 1);
        if mounts && matches!(m, 5 | 10 | 15 | 20) {
            let event = 90 + s as u32 + (m - 5) / 5 * 2;
            bg::set_event(map, event as u8, Some(0));
        }
        return;
    }
    // Mine supplies, and the ground assault's orders.
    let supply = match quest {
        5892 => Some((0, true, 2)),
        6982 => Some((0, false, 3)),
        5893 => Some((1, false, 2)),
        6985 => Some((1, true, 3)),
        _ => None,
    };
    if let Some((side, irondeep, reputation)) = supply
        && side == s
    {
        let Some(x) = extras_mut(map) else {
            return;
        };
        if irondeep {
            x.ground[s].irondeep += 10;
        } else {
            x.ground[s].coldtooth += 10;
        }
        bg::reward_team_reputation(map, s, REPUTATION[s], reputation);
        return;
    }
    let orders = match quest {
        6846 => Some(0),
        6901 => Some(1),
        _ => None,
    };
    if orders == Some(s) {
        let commander = extras(map).and_then(|x| {
            (x.ground[s].phase == March::Assembled)
                .then_some(x.ground[s].commander)
                .flatten()
        });
        if let Some(c) = commander {
            if let Some(x) = extras_mut(map) {
                x.ground[s].phase = March::Marching;
            }
            super::av_scripts::ground_marches(map, c, s);
        }
    }
}

fn standing(map: &Map, g: Guid, s: usize) -> i32 {
    map.players.get(&g).map_or(0, |p| {
        crate::game::map::reputation::standing(p, &map.realm.data, REPUTATION[s])
    })
}

/// `CMSG_GOSSIP_HELLO` with one of the valley's NPCs: their own menu.
/// Returns whether it was one.
pub fn gossip_hello(map: &mut Map, g: Guid, npc: Guid) -> bool {
    let Some(m) = map.battleground.as_ref() else {
        return false;
    };
    let Some(s) = playing_side(m, g) else {
        return false;
    };
    let entry = crate::game::guid::entry(npc);
    let near = match (map.players.get(&g), map.creature(npc)) {
        (Some(p), Some(c)) => crate::game::map::combat::dist(p.position(), c.position) <= 10.0,
        _ => false,
    };
    if !near {
        return false;
    }
    let standing = standing(map, g, s);
    let Some(x) = extras(map) else {
        return false;
    };
    let menu: Option<(u32, Vec<(u32, Action)>)> = if SMITHS[s] == entry {
        let a = x.armor[s];
        let need = (u32::from(a.tier) + 1) * 500;
        let ready = a.tier < 3 && a.scraps >= need && standing >= HONORED;
        if a.tier == 3 {
            Some((6222, Vec::new()))
        } else {
            let text = if a.scraps >= need {
                6219 + u32::from(a.tier)
            } else {
                [6073, 6217, 6218][usize::from(a.tier)]
            };
            let mut options = vec![(9130, Action::ArmorStatus)];
            if ready {
                options.push((
                    [8718, 8719, 8723][usize::from(a.tier)],
                    Action::Upgrade(a.tier + 1),
                ));
            }
            Some((text, options))
        }
    } else if SUMMONERS[s] == entry {
        let o = x.offerings[s];
        let progress = offering_progress(o.count);
        if o.launched {
            Some((offering_status_text(s, progress), Vec::new()))
        } else {
            let option = (if s == 0 { 8757 } else { 8641 }) + progress * 2;
            Some(([6174, 6093][s], vec![(option, Action::OfferingStatus)]))
        }
    } else if let Some(i) = WING_COMMANDERS
        .iter()
        .position(|w| w.entry == entry && w.side == s)
    {
        let f = x.air[i];
        match f.phase {
            FleetPhase::Prisoner => {
                // Talking to a captured commander starts their rescue.
                rescue(map, i, npc);
                send_gossip_complete(map, g);
                return true;
            }
            FleetPhase::Ready
                if standing >= 0 && !f.launched && f.count >= WING_COMMANDERS[i].goal =>
            {
                Some((
                68,
                    vec![
                        (WING_COMMANDERS[i].beacon_text, Action::TakeBeacon(i)),
                        (WING_COMMANDERS[i].launch_text, Action::LaunchAir(i)),
                    ],
                ))
            }
            _ => None,
        }
    } else if CAVALRY[s] == entry {
        let c = x.cavalry[s];
        (standing >= HONORED && c.phase == March::Ready && c.hides >= 25 && c.mounts >= 25)
            .then(|| (68, vec![(8903, Action::LaunchCavalry)]))
    } else if QUARTERMASTERS[s] == entry {
        let gr = x.ground[s];
        let (iron, cold) = if s == 0 { (280, 70) } else { (70, 280) };
        let ready = matches!(gr.phase, March::Ready | March::Assembled)
            && !gr.quartermaster_dead
            && standing >= HONORED
            && (gr.irondeep >= iron || gr.coldtooth >= cold);
        let mut options = vec![(9128, Action::GroundStatus)];
        if ready {
            options.push((9050, Action::TakeOrders));
        }
        options.push((0, Action::Vendor));
        Some((6255, options))
    } else {
        None
    };
    let Some((text, options)) = menu else {
        return false;
    };
    show(map, g, npc, text, options);
    true
}

fn offering_progress(count: u32) -> u32 {
    match count {
        160.. => 2,
        100..=159 => 1,
        _ => 0,
    }
}

fn offering_status_text(s: usize, progress: u32) -> u32 {
    (if s == 0 { 6175 } else { 6098 }) + progress
}

fn show(map: &mut Map, g: Guid, npc: Guid, text: u32, options: Vec<(u32, Action)>) {
    let data = map.realm.data.clone();
    let mut gossips = Vec::new();
    let mut kept = Vec::new();
    for (i, (line, action)) in options.into_iter().enumerate() {
        let (message, icon) = if action == Action::Vendor {
            ("Let me browse your goods.".to_owned(), 1)
        } else {
            (
                data.npc
                    .broadcast_line(line, false)
                    .unwrap_or("...")
                    .to_owned(),
                0,
            )
        };
        gossips.push(GossipItem {
            id: i as u32,
            item_icon: icon,
            coded: false,
            message,
        });
        kept.push((i as u32, action));
    }
    let quests = crate::game::map::quests::giver_quests(map, g, npc);
    if let Some(x) = extras_mut(map) {
        x.menus.insert(g, (npc, kept));
    }
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_GOSSIP_MESSAGE {
            guid: npc,
            title_text_id: text,
            gossips,
            quests,
        });
    }
}

fn send_gossip_complete(map: &Map, g: Guid) {
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_GOSSIP_COMPLETE {});
    }
}

/// `CMSG_GOSSIP_SELECT_OPTION` on one of the valley's menus. Returns
/// whether it was one.
pub fn gossip_select(map: &mut Map, g: Guid, npc: Guid, option: u32) -> bool {
    let Some(action) = extras(map)
        .and_then(|x| x.menus.get(&g))
        .filter(|(n, _)| *n == npc)
        .and_then(|(_, options)| options.iter().find(|(id, _)| *id == option))
        .map(|(_, a)| *a)
    else {
        return false;
    };
    let Some(s) = map.battleground.as_ref().and_then(|m| playing_side(m, g)) else {
        return true;
    };
    match action {
        Action::ArmorStatus => {
            let a = extras(map).map(|x| x.armor[s]).unwrap_or_default();
            show(map, g, npc, armor_progress_text(a), Vec::new());
        }
        Action::Upgrade(t) => {
            upgrade(map, g, s, t);
            send_gossip_complete(map, g);
        }
        Action::OfferingStatus => {
            let count = extras(map).map_or(0, |x| x.offerings[s].count);
            show(
                map,
                g,
                npc,
                offering_status_text(s, offering_progress(count)),
                Vec::new(),
            );
        }
        Action::TakeBeacon(i) => {
            let ready = extras(map).is_some_and(|x| {
                let f = x.air[i];
                f.phase == FleetPhase::Ready && !f.launched && f.count >= WING_COMMANDERS[i].goal
            });
            if ready
                && crate::game::map::inventory::add_item(map, g, WING_COMMANDERS[i].beacon, 1, true)
                    .is_ok()
                && let Some(x) = extras_mut(map)
            {
                x.air[i].count = 0;
            }
            send_gossip_complete(map, g);
        }
        Action::LaunchAir(i) => {
            launch_air(map, i, npc);
            send_gossip_complete(map, g);
        }
        Action::LaunchCavalry => {
            launch_cavalry(map, g, s, npc);
            send_gossip_complete(map, g);
        }
        Action::GroundStatus => {
            let gr = extras(map).map(|x| x.ground[s]).unwrap_or_default();
            let text = match gr.irondeep + gr.coldtooth {
                231.. => 6731,
                141..=230 => 6732,
                _ => 6733,
            };
            show(map, g, npc, text, Vec::new());
        }
        Action::TakeOrders => {
            take_orders(map, g, s, npc);
            send_gossip_complete(map, g);
        }
        Action::Vendor => crate::game::map::vendor::list(map, g, npc),
    }
    true
}

fn armor_progress_text(a: Armor) -> u32 {
    if a.tier == 3 {
        return 6222;
    }
    let tier = usize::from(a.tier);
    let remaining = (a.tier as i64 + 1) * 500 - i64::from(a.scraps);
    if remaining <= 0 {
        6219 + u32::from(a.tier)
    } else if remaining <= 100 {
        [6778, 6779, 6782][tier]
    } else if remaining < 300 {
        [6780, 6781, 6783][tier]
    } else {
        6784
    }
}

/// The blacksmith upgrades the side's defenders.
fn upgrade(map: &mut Map, g: Guid, s: usize, to: u8) {
    let standing = standing(map, g, s);
    let Some(x) = extras_mut(map) else {
        return;
    };
    let a = x.armor[s];
    let need = (u32::from(a.tier) + 1) * 500;
    if a.tier >= 3 || to != a.tier + 1 || a.scraps < need || standing < HONORED {
        return;
    }
    x.armor[s].tier = to;
    // The defenders of what the side holds come back at the new tier.
    let changes: Vec<(u8, u8)> = map
        .battleground
        .as_ref()
        .and_then(|m| m.av.as_ref())
        .map(|av| {
            (0..15)
                .filter(|id| av.controlled_by(*id) == Some(s))
                .flat_map(|id| av.defender_events(id))
                .collect()
        })
        .unwrap_or_default();
    for (event, state) in changes {
        bg::set_event(map, event, Some(state));
    }
    bg::team_spell(map, s, 28417 + u32::from(to));
    let rank = ["Seasoned", "Veteran", "Champion"][usize::from(to - 1)];
    bg::announce(
        map,
        Some(s),
        &format!("{rank} units are entering the battle!"),
    );
}

/// The first creature of `entry` in the valley.
fn creature_of(map: &Map, entry: u32) -> Option<Guid> {
    map.creatures
        .iter()
        .find(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
}

/// A creature of `entry` yells `text`.
fn yell(map: &Map, entry: u32, text: &str) {
    let Some(c) = creature_of(map, entry) else {
        return;
    };
    let name = map
        .realm
        .data
        .creature_template(entry)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let Some(frame) = crate::game::conn::encode(&SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::MonsterYell {
            sender1: c,
            sender_name: name,
            target: Guid::zero(),
        },
        language: Language::Universal,
        message: text.to_owned(),
        tag: PlayerChatTag::None,
    }) else {
        return;
    };
    for p in map.players.values() {
        p.outbound.send_frame(frame.clone());
    }
}

/// Each update: the NPCs' scripts and routes, and the beacons planted
/// calling down their flyers.
pub fn update(map: &mut Map) {
    beacons(map);
    super::av_scripts::update(map);
}

/// Beacons once a second: new ones planted wait a minute; one not
/// disabled by then calls down its side's flyer.
fn beacons(map: &mut Map) {
    let now = map.now;
    let Some(x) = extras(map) else {
        return;
    };
    if x.next_scan.is_some_and(|t| t > now) {
        return;
    }
    let planted: Vec<(Guid, usize)> = map
        .gameobjects
        .iter()
        .filter_map(|(_, o)| {
            let side = BEACONS.iter().position(|b| b.contains(&o.entry))?;
            (!x.seen.contains(&o.guid)).then_some((o.guid, side))
        })
        .collect();
    let due: Vec<Beacon> = x
        .beacons
        .iter()
        .filter(|b| b.ready_at <= now)
        .copied()
        .collect();
    let Some(x) = extras_mut(map) else {
        return;
    };
    x.next_scan = Some(now + std::time::Duration::from_secs(1));
    for (object, side) in planted {
        x.seen.insert(object);
        x.beacons.push(Beacon {
            object,
            side,
            ready_at: now + BEACON_WAIT,
        });
    }
    x.beacons.retain(|b| b.ready_at > now);
    for b in due {
        let Some(at) = map
            .gameobject(b.object)
            .map(|o| (o.position, o.orientation))
        else {
            continue;
        };
        map.remove_gameobject(b.object);
        crate::game::map::outdoor_pvp::summon(
            map,
            BEACON_CALLS[b.side],
            (at.0.x, at.0.y, at.0.z + 30.0, at.1),
            0,
        );
    }
}

/// `CMSG_GAMEOBJ_USE` on a beacon: the enemy disables it before it calls.
/// Returns whether it was one.
pub fn use_beacon(map: &mut Map, g: Guid, object: Guid) -> bool {
    let Some(i) = extras(map).and_then(|x| x.beacons.iter().position(|b| b.object == object))
    else {
        return false;
    };
    let side = map.battleground.as_ref().and_then(|m| playing_side(m, g));
    let Some(x) = extras_mut(map) else {
        return true;
    };
    if side.is_some_and(|s| s != x.beacons[i].side) {
        x.beacons.remove(i);
        map.remove_gameobject(object);
    }
    true
}

/// A captured wing commander is freed: they run home.
fn rescue(map: &mut Map, i: usize, npc: Guid) {
    if let Some(x) = extras_mut(map) {
        x.air[i].phase = FleetPhase::Returning;
    }
    if !super::av_scripts::commander_departs(map, npc)
        && let Some(x) = extras_mut(map)
    {
        x.air[i].phase = FleetPhase::Ready;
    }
}

/// A rescued wing commander reached home: their fleet takes supplies.
pub(super) fn fleet_home(map: &mut Map, entry: u32) {
    let Some(i) = WING_COMMANDERS.iter().position(|w| w.entry == entry) else {
        return;
    };
    if let Some(x) = extras_mut(map)
        && x.air[i].phase == FleetPhase::Returning
    {
        x.air[i].phase = FleetPhase::Ready;
    }
}

/// The offerings are made: the summoner calls, and sets out for the
/// altar.
fn launch_summoner(map: &mut Map, s: usize) {
    if s == 0 {
        if let Some(g) = creature_of(map, SUMMONERS[0]) {
            crate::game::map::scripts::talk(map, g, 8732, Guid::zero(), None);
        }
    } else {
        yell(
            map,
            SUMMONERS[1],
            "Soldiers of Frostwolf, come to my aid! The Ice Lord has granted us his protection. He's accepted the offering! The time has come to unleash him upon the Stormpike Army!",
        );
    }
    super::av_scripts::summoner_departs(map, s);
}

/// A wing commander launches the air attack: they mount, rise, and send
/// out their rider.
fn launch_air(map: &mut Map, i: usize, npc: Guid) {
    let ready = extras(map).is_some_and(|x| {
        let f = x.air[i];
        f.phase == FleetPhase::Ready && !f.launched && f.count >= WING_COMMANDERS[i].goal
    });
    if !ready {
        return;
    }
    if let Some(x) = extras_mut(map) {
        x.air[i].launched = true;
    }
    super::av_scripts::commander_launches(map, npc);
}

/// The cavalry charges: the stables empty, and the commander rides out
/// with their riders.
fn launch_cavalry(map: &mut Map, g: Guid, s: usize, npc: Guid) {
    let standing = standing(map, g, s);
    let Some(x) = extras_mut(map) else {
        return;
    };
    let c = x.cavalry[s];
    if standing < HONORED || c.phase != March::Ready || c.hides < 25 || c.mounts < 25 {
        return;
    }
    x.cavalry[s] = Cavalry {
        phase: March::Marching,
        ..Default::default()
    };
    for e in [90u8, 92, 94, 96] {
        bg::set_event(map, e + s as u8, Some(2));
    }
    super::av_scripts::cavalry_charges(map, npc, s);
}

/// The quartermaster gives the orders; the first time, the ground
/// commander and their troops assemble.
fn take_orders(map: &mut Map, g: Guid, s: usize, npc: Guid) {
    let standing = standing(map, g, s);
    let Some(gr) = extras(map).map(|x| x.ground[s]) else {
        return;
    };
    let (iron, cold) = if s == 0 { (280, 70) } else { (70, 280) };
    let ready = matches!(gr.phase, March::Ready | March::Assembled)
        && !gr.quartermaster_dead
        && standing >= HONORED
        && (gr.irondeep >= iron || gr.coldtooth >= cold);
    if !ready || crate::game::map::inventory::add_item(map, g, ORDERS[s], 1, true).is_err() {
        return;
    }
    let first = gr.phase == March::Ready;
    if let Some(x) = extras_mut(map) {
        x.ground[s].irondeep = 0;
        x.ground[s].coldtooth = 0;
        x.ground[s].phase = March::Assembled;
    }
    if first {
        let t = extras(map).map_or(0, |x| x.armor[s].tier);
        super::av_scripts::ground_assembles(map, npc, s, t);
    }
}

/// The ground commander assembled: theirs are the orders.
pub(super) fn ground_commander(map: &mut Map, s: usize, g: Guid) {
    if let Some(x) = extras_mut(map)
        && x.ground[s].phase == March::Assembled
        && x.ground[s].commander.is_none()
    {
        x.ground[s].commander = Some(g);
    }
}

/// A cavalry commander or quartermaster back after dying: their side can
/// charge again, or take orders.
pub(super) fn back_from_the_dead(map: &mut Map) {
    let (cavalry, quartermasters) = (
        [0, 1].map(|s| creature_of(map, CAVALRY[s]).is_some()),
        [0, 1].map(|s| creature_of(map, QUARTERMASTERS[s]).is_some()),
    );
    let Some(x) = extras_mut(map) else {
        return;
    };
    for s in 0..2 {
        if x.cavalry[s].phase == March::Dead && cavalry[s] {
            x.cavalry[s].phase = March::Ready;
        }
        if x.ground[s].quartermaster_dead && quartermasters[s] {
            x.ground[s] = Ground {
                irondeep: x.ground[s].irondeep,
                coldtooth: x.ground[s].coldtooth,
                ..Default::default()
            };
        }
    }
}

/// One of the valley's NPCs fell: a cavalry or ground assault is over, a
/// quartermaster gives no more orders, a wing commander is a prisoner
/// again; those who followed them go on alone.
pub fn creature_died(map: &mut Map, creature: Guid) {
    let entry = crate::game::guid::entry(creature);
    if let Some(x) = extras_mut(map) {
        for s in 0..2 {
            if CAVALRY[s] == entry {
                x.cavalry[s].phase = March::Dead;
            }
            if QUARTERMASTERS[s] == entry {
                x.ground[s].quartermaster_dead = true;
            }
            if x.ground[s].commander == Some(creature)
                && matches!(x.ground[s].phase, March::Assembled | March::Marching)
            {
                x.ground[s].phase = March::Dead;
                x.ground[s].commander = None;
            }
        }
        if let Some(i) = WING_COMMANDERS.iter().position(|w| w.entry == entry) {
            x.air[i].phase = FleetPhase::Prisoner;
        }
    }
    super::av_scripts::died(map, creature, entry);
}

/// Whether `object` is a planted beacon.
pub fn is_beacon(map: &Map, object: Guid) -> bool {
    extras(map).is_some_and(|x| x.beacons.iter().any(|b| b.object == object))
}

#[cfg(test)]
impl Extras {
    /// The beacons looked at again at once.
    pub fn hurry(&mut self, _now: Instant) {
        self.next_scan = None;
    }
}
