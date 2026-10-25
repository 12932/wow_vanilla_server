//! The GM and developer commands Thistle Tea has beside wvs's own
//! (`dev_commands.ex`): items, quests, spells and levels; the selected
//! unit's health, threat, death and respawn; money, power, rested XP and
//! god mode; the zone's weather; instances and battlegrounds; mail; and the
//! `.debug` family (cooldowns, durability, events, exploration, flight
//! paths, professions, skills, trainer spells, reputation, transports,
//! random gear). Each is checked against the GM's level.

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::{Map, MapKey};

pub const HELP: &str = ".additem <item> [count], .addquest <quest>, .learn <spell>, .unlearn <spell>, \
.character level <n>, .character spells, .gear [green|blue|raid [1|2]] [tank|healer|caster|dps] [shield|2h|dual|offhand] [dagger|sword|mace|axe|fist|staff|polearm] (raid: damage dealers in the tier's loot with hit, others their tier set), .gear enchant <slot> <enchantment>, .pet [creature], .damage <n>, .kill, .respawn [radius], .ready [save|load], .move [x y z], .threat, .guid, .pos, \
.modify hp|mana|rage|energy|money|speed <n>, .rested [n], .start, .talents reset|learn <talent>[:<ranks>] ..., .tgm [on|off], .god [on|off], .durability [off|on], .infinitemana [on|off] (.thereisnospoon), \
.weather fine|rain|snow|storm [0..1] [permanent]|auto, .instance info|reset|switch <id>|raid-reset <map>|data [field], \
.bg join <warsong|arathi|alterac>|start|info|leave, .mail <name> <text>, .debug cooldowns|durability <percent>|\
events [start|stop <id>]|explore|position [guid]|professions|reputation [<faction>|add|set <faction> <n>|war <faction> on|off|find <name>]|\
skill <id> <value>|skills|spells|taxi|transport|random equipment, .event list [all]|start <id>|stop <id>|<id>, .record [on [name]|off|mark <text>]";

/// The level a command here needs, if it is one of these: looking is for
/// moderators, the rest for game masters.
pub fn needed_level(name: &str) -> Option<u8> {
    use crate::accounts::{GAME_MASTER, MODERATOR};
    Some(match name {
        "guid" | "pos" | "threat" => MODERATOR,
        "additem" | "addquest" | "learn" | "unlearn" | "character" | "gear" | "pet" | "damage"
        | "kill" | "respawn" | "move" | "modify" | "rested" | "talents" | "tgm" | "god"
        | "weather" | "instance" | "bg" | "battleground" | "mail" | "debug" | "invasion"
        | "event" | "durability" | "infinitemana" | "thereisnospoon" | "ready" | "record" => {
            GAME_MASTER
        }
        _ => return None,
    })
}

/// Run `name` if it is one of these; `None` if it isn't.
pub fn command(map: &mut Map, g: Guid, name: &str, args: &[&str]) -> Option<Option<String>> {
    Some(match name {
        "additem" => additem(map, g, args),
        "addquest" => addquest(map, g, args),
        "learn" => learn(map, g, args, true),
        "unlearn" => learn(map, g, args, false),
        "character" => character(map, g, args),
        "gear" => match args {
            ["enchant", rest @ ..] => gear_enchant(map, g, rest),
            _ => match gear_args(args) {
                Ok((pick, wants)) => gear(map, g, pick, wants),
                Err(usage) => Some(usage.into()),
            },
        },
        "pet" => pet(map, g, args),
        "damage" => damage(map, g, args),
        "kill" => kill(map, g),
        "respawn" => match args {
            [] => respawn(map, g),
            [r] => match r.parse::<f32>() {
                Ok(r) if r > 0.0 && r <= crate::game::map::ready::MAX_RADIUS => {
                    Some(crate::game::map::ready::respawn_around(map, g, r))
                }
                _ => Some(RESPAWN_USAGE.into()),
            },
            _ => Some(RESPAWN_USAGE.into()),
        },
        "ready" => {
            let to = player_target(map, g);
            crate::game::map::ready::command(map, g, args, to)
        }
        "move" => move_target(map, g, args),
        "threat" => threat(map, g),
        "guid" => Some(format!("Target: {}", selected(map, g))),
        "pos" => pos(map, g),
        "modify" => modify(map, g, args),
        "rested" => rested(map, g, args),
        "start" => start(map, g),
        "talents" => talents(map, g, args),
        "tgm" => tgm(map, g, args),
        "god" => god(map, g, args),
        "durability" => durability(map, g, args),
        "infinitemana" | "thereisnospoon" => infinite_mana(map, g, args),
        "weather" => weather(map, g, args),
        "instance" => instance(map, g, args),
        "bg" | "battleground" => battleground(map, g, args),
        "mail" => mail(map, g, args),
        "debug" => debug(map, g, args),
        "event" => event(map, args),
        "record" => Some(crate::game::map::record::command(map, g, args)),
        _ => return None,
    })
}

/// `.event list [all]`, `.event start|stop <id>`, `.event <id>`: mangos'
/// event commands. A start or stop holds for the event's turn, as mangos
/// moves its start.
fn event(map: &mut Map, args: &[&str]) -> Option<String> {
    let realm = map.realm.clone();
    let data = &realm.data;
    let on = crate::game::events::current(&realm);
    let label = |e: u32| {
        let name = data.events.get(&e).map(|x| x.name.as_str()).unwrap_or("");
        format!(
            "{e} {name}{}",
            if on.contains(&e) { " (active)" } else { "" }
        )
    };
    Some(match args {
        [] | ["list"] | ["list", "all"] => {
            let mut ids: Vec<u32> = if args.len() == 2 {
                data.events.keys().copied().collect()
            } else {
                on.iter().copied().collect()
            };
            ids.sort_unstable();
            let list: Vec<String> = ids.into_iter().map(label).collect();
            format!(
                "Events: {}",
                if list.is_empty() {
                    "none".into()
                } else {
                    list.join(", ")
                }
            )
        }
        [action @ ("start" | "stop"), id] => {
            let Ok(id) = id.parse::<u32>() else {
                return Some("Usage: .event start|stop <id>".into());
            };
            let start = *action == "start";
            match crate::game::events::force(&realm, id, start) {
                None => format!("No event {id}."),
                Some(false) if start => format!("Event {id} is already active."),
                Some(false) => format!("Event {id} is not active."),
                Some(true) => format!(
                    "Event {} {}.",
                    label(id).trim_end_matches(" (active)"),
                    if start { "started" } else { "stopped" }
                ),
            }
        }
        [id] => {
            let Some(e) = id.parse::<u32>().ok().and_then(|id| data.events.get(&id)) else {
                return Some("Usage: .event list [all] | start <id> | stop <id> | <id>".into());
            };
            format!(
                "Event {}: {}, every {} minutes for {}{}.",
                e.id,
                e.name,
                e.occurence,
                e.length,
                if on.contains(&e.id) { ", active" } else { "" }
            )
        }
        _ => "Usage: .event list [all] | start <id> | stop <id> | <id>".into(),
    })
}

/// The GM's selection.
/// Where `t` is on this map, with its facing; `None` if it isn't here.
fn position_report(map: &Map, t: Guid) -> Option<String> {
    let at = map.position_of(t)?;
    let facing = if let Some(p) = map.players.get(&t) {
        p.movement.orientation
    } else if let Some(c) = map.creature(t) {
        c.orientation
    } else {
        map.gameobject(t).map_or(0.0, |o| o.orientation)
    };
    Some(format!(
        "{t}: {:.2}, {:.2}, {:.2}, orientation {facing:.2} in {}",
        at.x, at.y, at.z, map.key
    ))
}

fn selected(map: &Map, g: Guid) -> Guid {
    map.players
        .get(&g)
        .map(|p| p.fields.get_guid(UNIT_FIELD_TARGET))
        .unwrap_or_default()
}

/// The selected player, or the GM.
fn player_target(map: &Map, g: Guid) -> Guid {
    let t = selected(map, g);
    if map.players.contains_key(&t) { t } else { g }
}

fn num<T: std::str::FromStr>(s: Option<&&str>) -> Option<T> {
    s.and_then(|s| s.parse().ok())
}

/// A number with a fraction: finite (`nan` and `inf` parse, and would
/// spread through what they touch, to a save that won't load).
fn float(s: Option<&&str>) -> Option<f32> {
    num::<f32>(s).filter(|v| v.is_finite())
}

fn additem(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let Some(entry) = num::<u32>(args.first()) else {
        return Some("Usage: .additem <item_id> [count]".into());
    };
    let count = num::<u32>(args.get(1)).unwrap_or(1).max(1);
    let name = map.realm.data.item(entry).map(|t| t.name.clone());
    let Some(name) = name else {
        return Some(format!("No item {entry}."));
    };
    let to = player_target(map, g);
    match crate::game::map::inventory::add_item(map, to, entry, count, true) {
        Ok(()) => Some(format!("Added {count} {name}.")),
        Err(e) => Some(format!("No room for {name}: {e:?}.")),
    }
}

fn addquest(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let Some(quest) = num::<u32>(args.first()) else {
        return Some("Usage: .addquest <quest_id>".into());
    };
    let data = map.realm.data.clone();
    let Some(q) = data.npc.quests.get(&quest) else {
        return Some(format!("No quest {quest}."));
    };
    let to = player_target(map, g);
    let p = map.players.get_mut(&to)?;
    if p.journal.has(quest) {
        return Some("Already in the log.".into());
    }
    if p.journal
        .add(crate::game::store::Logged {
            quest,
            ..Default::default()
        })
        .is_none()
    {
        return Some("The quest log is full.".into());
    }
    crate::game::map::quests::refresh_log(p, &data);
    let title = q.title.clone();
    if q.source_item != 0 {
        let _ = crate::game::map::inventory::add_item(
            map,
            to,
            q.source_item,
            q.source_item_count.max(1),
            true,
        );
    }
    crate::game::map::quests::check(map, to, quest);
    Some(format!("Added {title}."))
}

fn learn(map: &mut Map, g: Guid, args: &[&str], learn: bool) -> Option<String> {
    let Some(spell) = num::<u32>(args.first()) else {
        return Some(format!(
            "Usage: .{} <spell_id>",
            if learn { "learn" } else { "unlearn" }
        ));
    };
    let Some(name) = map.realm.data.spells.get(spell).map(|s| s.name.clone()) else {
        return Some(format!("No spell {spell}."));
    };
    let to = player_target(map, g);
    if learn {
        crate::game::map::spells::learn(map, to, spell);
        Some(format!("Learned {name}."))
    } else {
        crate::game::map::misc::forget_spells(map, to, &[spell]);
        Some(format!("Unlearned {name}."))
    }
}

/// `.character level <n>` and `.character spells`: the selected player,
/// or the GM, at a level (at full health and mana, as a level gained), or
/// taught every spell their class's trainers would teach them now.
fn character(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    if args.first() == Some(&"spells") && args.len() == 1 {
        let to = player_target(map, g);
        let n = crate::game::map::npc::learn_class_spells(map, to);
        return Some(format!("Learned {n} spells."));
    }
    let (Some(&"level"), Some(level)) = (args.first(), num::<u8>(args.get(1))) else {
        return Some("Usage: .character level <level> | .character spells".into());
    };
    let data = map.realm.data.clone();
    let to = player_target(map, g);
    let p = map.players.get_mut(&to)?;
    let level = level.clamp(1, crate::game::map::stats::max_level());
    crate::game::map::stats::set_level(p, &data, level);
    p.fields.set_u32(PLAYER_XP, 0);
    map.realm.registry.set_level(to, level);
    crate::game::map::misc::reset_if_overbudget(map, to);
    Some(format!("Level {level}."))
}

/// `.pet [creature|random]`: the selected player, or the GM, a hunter,
/// given a tamed beast of that creature template (by default a cat or a
/// wolf of their level; `random`, a beast of their level of a family at
/// random) at their level, out at their side, the pet before let go;
/// with the tamer's spells (Tame Beast, Call Pet, Dismiss Pet, Revive Pet,
/// Feed Pet) they lack, which 1.12 teaches by quest.
fn pet(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let to = player_target(map, g);
    let level = map.players.get(&to)?.level();
    let entry = match args.first() {
        None => crate::game::map::pets::beast_for(&map.realm.data, level),
        Some(&"random") => {
            crate::game::map::pets::random_beast_for(&map.realm.data, level, &mut rand::rng())
        }
        Some(_) if args.len() == 1 => num::<u32>(args.first()),
        Some(_) => None,
    };
    let Some(entry) = entry else {
        return Some("Usage: .pet [creature entry|random]".into());
    };
    Some(
        match crate::game::map::pets::give_pet(map, to, entry, level) {
            Ok(said) | Err(said) => said,
        },
    )
}

/// The weapon proficiency spells, by weapon subclass: axes, bows, guns,
/// maces, polearms, swords, staves, fist weapons, daggers, thrown,
/// crossbows and wands.
const WEAPON_SPELLS: &[(u32, u32)] = &[
    (0, 196),
    (1, 197),
    (2, 264),
    (3, 266),
    (4, 198),
    (5, 199),
    (6, 200),
    (7, 201),
    (8, 202),
    (10, 227),
    (13, 15_590),
    (15, 1180),
    (16, 2567),
    (18, 5011),
    (19, 5009),
];
/// The armour proficiency spells, by armour subclass: cloth, leather,
/// mail, plate and shields.
const ARMOR_SPELLS: &[(u32, u32)] = &[(1, 9078), (2, 9077), (3, 8737), (4, 750), (6, 9116)];
const DUAL_WIELD: u32 = 674;
const AUTO_SHOT: u32 = 75;
/// A wand's Shoot.
const SHOOT: u32 = 5019;
const ITEM_WEAPON: u32 = 2;
const ITEM_ARMOR: u32 = 4;
const ITEM_PROJECTILE: u32 = 6;
const ARMOR_SHIELD: u32 = 6;
const WEAPON_WAND: u32 = 19;

/// What a `.gear` character holds in their hands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hands {
    TwoHanded,
    Shield,
    DualWield,
    HeldInOffHand,
}

/// The armour a class wears at a level: plate for warriors and paladins
/// from 40 (mail before), mail for hunters and shamans from 40 (leather
/// before), leather for rogues and druids, cloth for the rest.
fn armour_of(class: u8, level: u8) -> u32 {
    match (class, level >= 40) {
        (1 | 2, true) => 4,
        (1 | 2, false) | (3 | 7, true) => 3,
        (3 | 7, false) | (4 | 11, _) => 2,
        _ => 1,
    }
}

/// What a class may hold, at a level: Dual Wield for rogues, and for
/// warriors and hunters from 20; a shield for warriors, paladins and
/// shamans; an off-hand item for the casters.
fn hands_of(class: u8, level: u8) -> Vec<Hands> {
    let mut out = Vec::new();
    if class != 4 {
        out.push(Hands::TwoHanded);
    }
    if matches!(class, 1 | 2 | 7) {
        out.push(Hands::Shield);
    }
    if class == 4 || (matches!(class, 1 | 3) && level >= 20) {
        out.push(Hands::DualWield);
    }
    if matches!(class, 5 | 7 | 8 | 9 | 11) {
        out.push(Hands::HeldInOffHand);
    }
    out
}

/// The ranged weapons a class is given: a bow, gun or crossbow for
/// hunters, those or a thrown weapon for warriors and rogues, a wand for
/// priests, mages and warlocks.
fn ranged_of(class: u8) -> &'static [u32] {
    match class {
        3 => &[2, 3, 18],
        1 | 4 => &[2, 3, 16, 18],
        5 | 8 | 9 => &[WEAPON_WAND],
        _ => &[],
    }
}

/// The proficiency spell an item needs, if any.
fn proficiency_spell(t: &crate::game::data::ItemTemplate) -> Option<u32> {
    let table = match t.class {
        ITEM_WEAPON => WEAPON_SPELLS,
        ITEM_ARMOR => ARMOR_SPELLS,
        _ => return None,
    };
    table
        .iter()
        .find(|(subclass, _)| *subclass == t.subclass)
        .map(|(_, spell)| *spell)
}

/// Whether a class may learn a proficiency spell, by its `SkillLineAbility`
/// classes; in a world without the spell, yes.
fn class_gets(data: &crate::game::data::GameData, spell: u32, class: u8) -> bool {
    if data.spells.get(spell).is_none() {
        return true;
    }
    let bit = 1u32 << (class.max(1) - 1);
    data.spells.skill_abilities.get(&spell).is_some_and(|v| {
        v.iter()
            .any(|a| a.class_mask == 0 || a.class_mask & bit != 0)
    })
}

/// Whether a class may use a weapon subclass.
fn weapon_for(data: &crate::game::data::GameData, subclass: u32, class: u8) -> bool {
    WEAPON_SPELLS
        .iter()
        .find(|(s, _)| *s == subclass)
        .is_some_and(|(_, spell)| class_gets(data, *spell, class))
}

/// How `.gear` chooses: at random among the uncommon and rare items of
/// about the character's level (`.gear`), or only the uncommon (`.gear
/// green`, a fresh character's) or only the rare (`.gear blue`, a
/// dungeon's), a slot with none of that quality filled from both; or the
/// best they may wear, a raider's (`.gear raid`): at 60 a tier set, 1 or 2
/// (at random, or as asked), and the rest of the best quality there is
/// (epic, else rare, else uncommon), the highest item level first. A
/// weapon damage dealer's raid gear is the tier's loot instead, by their
/// stat weights ([`damage_weights`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Random { quality: Option<u32> },
    Raid { tier: Option<u8> },
}

/// Items' qualities: uncommon (green) and rare (blue).
const UNCOMMON: u32 = 2;
const RARE: u32 = 3;

const GEAR_USAGE: &str = "Usage: .gear [green|blue|raid [1|2]] [tank|healer|caster|dps] \
[shield|2h|dual|offhand] [dagger|sword|mace|axe|fist|staff|polearm] | .gear enchant <slot> <enchantment>. \
raid: a tank, healer or caster in their tier set; a weapon damage dealer (dps, a rogue, a hunter, or most \
talents in Arms, Fury, Retribution, Enhancement or Feral) in the tier's raid loot by strength, agility, crit \
and hit, up to 9% hit.";

/// A role `.gear` may be asked to dress for: the hands it holds by
/// ([`role_ways`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Tank,
    Healer,
    Damage,
}

/// What `.gear` is asked for beyond the quality: a role, a way of holding
/// and a kind of weapon (each, or none: the hands then by the talents
/// spent, [`spec_ways`], or at random).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Wants {
    role: Option<Role>,
    hands: Option<Hands>,
    weapon: Option<WeaponKind>,
}

/// A kind of weapon: its one-handed and two-handed item subclasses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WeaponKind {
    one_hand: Option<u32>,
    two_hand: Option<u32>,
}

impl WeaponKind {
    fn named(name: &str) -> Option<Self> {
        let (one_hand, two_hand) = match name {
            "dagger" | "daggers" => (Some(15), None),
            "sword" | "swords" => (Some(7), Some(8)),
            "mace" | "maces" => (Some(4), Some(5)),
            "axe" | "axes" => (Some(0), Some(1)),
            "fist" | "fists" => (Some(13), None),
            "staff" | "staves" => (None, Some(10)),
            "polearm" | "polearms" => (None, Some(6)),
            _ => return None,
        };
        Some(Self { one_hand, two_hand })
    }

    /// Whether a weapon of item subclass `subclass` is of this kind.
    fn fits(self, subclass: u32) -> bool {
        self.one_hand == Some(subclass) || self.two_hand == Some(subclass)
    }

    /// Whether this kind can be held so.
    fn held(self, way: Hands) -> bool {
        match way {
            Hands::TwoHanded => self.two_hand.is_some(),
            _ => self.one_hand.is_some(),
        }
    }
}

/// `.gear`'s arguments: none, `green`, `blue`, `raid`, or `raid 1` or `raid
/// 2`; then, in any order, a role (`tank`, `healer` or `heal`, `caster`,
/// `dps`), a way of holding (`shield`: a one-hander and a shield, `2h`,
/// `dual`, `offhand`: a one-hander and an off-hand item) and a kind of
/// weapon (`dagger`, `sword`, `mace`, `axe`, `fist`, `staff`, `polearm`),
/// one of each at most.
fn gear_args(args: &[&str]) -> Result<(Pick, Wants), &'static str> {
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let words: Vec<&str> = lower.iter().map(String::as_str).collect();
    let (pick, rest) = match words.as_slice() {
        ["green", rest @ ..] => (
            Pick::Random {
                quality: Some(UNCOMMON),
            },
            rest,
        ),
        ["blue", rest @ ..] => (
            Pick::Random {
                quality: Some(RARE),
            },
            rest,
        ),
        ["raid", "1", rest @ ..] => (Pick::Raid { tier: Some(1) }, rest),
        ["raid", "2", rest @ ..] => (Pick::Raid { tier: Some(2) }, rest),
        ["raid", rest @ ..] => (Pick::Raid { tier: None }, rest),
        rest => (Pick::Random { quality: None }, rest),
    };
    let mut wants = Wants::default();
    for w in rest {
        let role = match *w {
            "tank" => Some(Role::Tank),
            "healer" | "heal" | "caster" => Some(Role::Healer),
            "dps" => Some(Role::Damage),
            _ => None,
        };
        let hands = match *w {
            "shield" => Some(Hands::Shield),
            "2h" | "twohand" | "two-hand" => Some(Hands::TwoHanded),
            "dual" => Some(Hands::DualWield),
            "offhand" | "off-hand" => Some(Hands::HeldInOffHand),
            _ => None,
        };
        let weapon = WeaponKind::named(w);
        match (role, hands, weapon) {
            (Some(r), None, None) if wants.role.is_none() => wants.role = Some(r),
            (None, Some(h), None) if wants.hands.is_none() => wants.hands = Some(h),
            (None, None, Some(k)) if wants.weapon.is_none() => wants.weapon = Some(k),
            _ => return Err(GEAR_USAGE),
        }
    }
    Ok((pick, wants))
}

/// The ways of holding a role wants of a class, first first: a tank's
/// shield (a druid's two-hander), a healer's or caster's shield for a
/// paladin or a shaman and an off-hand item for the rest, a fighter's
/// weapons without a shield.
fn role_ways(class: u8, role: Role) -> Vec<Hands> {
    match (role, class) {
        (Role::Tank, 1 | 2 | 7) => vec![Hands::Shield],
        (Role::Tank, 11) => vec![Hands::TwoHanded],
        (Role::Tank, _) => Vec::new(),
        (Role::Healer, 2 | 7) => vec![Hands::Shield],
        (Role::Healer, _) => vec![Hands::HeldInOffHand, Hands::TwoHanded],
        (Role::Damage, _) => vec![Hands::TwoHanded, Hands::DualWield],
    }
}

/// The talent trees whose spec holds a shield: a warrior's Protection, a
/// paladin's Holy and Protection, a shaman's Elemental and Restoration;
/// and those that don't: Arms, Fury, Retribution, Enhancement.
const SHIELD_TREES: [u32; 5] = [163, 382, 383, 261, 262];
const WEAPON_TREES: [u32; 4] = [161, 164, 381, 263];

/// The ways of holding a character's talents want: a shield for one with
/// most of their points in a tree of [`SHIELD_TREES`], none for one with
/// most in a tree of [`WEAPON_TREES`]; nothing for one with no points (or
/// of another class).
fn spec_ways(
    data: &crate::game::data::GameData,
    p: &crate::game::map::player::Player,
) -> Vec<Hands> {
    match best_tree(data, p) {
        Some(tab) if SHIELD_TREES.contains(&tab) => vec![Hands::Shield],
        Some(tab) if WEAPON_TREES.contains(&tab) => role_ways(p.row.class, Role::Damage),
        _ => Vec::new(),
    }
}

/// The talent tree a character has spent most points in, if any (the
/// later tree on a tie).
fn best_tree(
    data: &crate::game::data::GameData,
    p: &crate::game::map::player::Player,
) -> Option<u32> {
    let mut points: Vec<(u32, usize)> = Vec::new();
    for t in data.spells.talents.values() {
        let rank = t
            .ranks
            .iter()
            .rposition(|r| *r != 0 && p.spells.contains(r))
            .map_or(0, |i| i + 1);
        if rank == 0 {
            continue;
        }
        match points.iter_mut().find(|(tab, _)| *tab == t.tab) {
            Some((_, n)) => *n += rank,
            None => points.push((t.tab, rank)),
        }
    }
    points
        .iter()
        .max_by_key(|(tab, n)| (*n, *tab))
        .map(|(tab, _)| *tab)
}

/// What a point of each stat is worth to a class's weapon damage dealer,
/// in attack power: a stat weight table, for `.gear raid`'s damage pieces
/// ([`damage_weights`]). Crit and hit are per 1%; hit is worth nothing
/// past [`HIT_CAP`], and below it more than its share of damage alone (40
/// attack power, about 20 strength), so the pieces aim for the cap, as the
/// user asked. `melee_dps` and `ranged_dps` weigh a weapon's damage a
/// second in the hands, or the ranged slot.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Weights {
    strength: f32,
    agility: f32,
    stamina: f32,
    intellect: f32,
    attack_power: f32,
    ranged_attack_power: f32,
    crit: f32,
    hit: f32,
    melee_dps: f32,
    ranged_dps: f32,
}

/// The weight tables by class, for the builds that deal damage with
/// weapons. 1.12's rules: a warrior's, a paladin's and a shaman's strength
/// is two attack power, their agility only crit (20 a 1%); a rogue's
/// agility is attack power and crit (29 a 1%), its strength one attack
/// power; a hunter's agility two ranged attack power and crit (53 a 1%); a
/// cat's strength two attack power, its weapon's damage nothing (a form's
/// claws).
#[rustfmt::skip]
const DAMAGE_WEIGHTS: [(u8, Weights); 6] = [
    (1, Weights { strength: 2.0, agility: 1.2, stamina: 0.1, intellect: 0.0, attack_power: 1.0, ranged_attack_power: 0.0, crit: 25.0, hit: 40.0, melee_dps: 14.0, ranged_dps: 0.0 }),
    (2, Weights { strength: 2.0, agility: 1.0, stamina: 0.1, intellect: 0.2, attack_power: 1.0, ranged_attack_power: 0.0, crit: 22.0, hit: 40.0, melee_dps: 14.0, ranged_dps: 0.0 }),
    (3, Weights { strength: 0.0, agility: 2.4, stamina: 0.1, intellect: 0.3, attack_power: 0.1, ranged_attack_power: 1.0, crit: 25.0, hit: 40.0, melee_dps: 1.0, ranged_dps: 14.0 }),
    (4, Weights { strength: 1.0, agility: 1.8, stamina: 0.1, intellect: 0.0, attack_power: 1.0, ranged_attack_power: 0.0, crit: 25.0, hit: 40.0, melee_dps: 14.0, ranged_dps: 0.0 }),
    (7, Weights { strength: 2.0, agility: 1.0, stamina: 0.1, intellect: 0.2, attack_power: 1.0, ranged_attack_power: 0.0, crit: 22.0, hit: 40.0, melee_dps: 14.0, ranged_dps: 0.0 }),
    (11, Weights { strength: 2.0, agility: 1.3, stamina: 0.1, intellect: 0.0, attack_power: 1.0, ranged_attack_power: 0.0, crit: 25.0, hit: 40.0, melee_dps: 0.0, ranged_dps: 0.0 }),
];

/// 1.12's chance to miss a boss (three levels up) with no weapon skill over
/// 300: 9%, hit beyond which is wasted.
const HIT_CAP: f32 = 9.0;

/// The talent trees whose builds deal damage with weapons: Arms, Fury,
/// Retribution, Enhancement and Feral Combat (a cat; a bear asks `tank`).
const DAMAGE_TREES: [u32; 5] = [161, 164, 381, 263, 281];

/// The weights `.gear raid` dresses a character by, if it dresses them as a
/// weapon damage dealer: one asked for `dps`, or asked for no role and a
/// rogue, a hunter, or one with most points in a tree of [`DAMAGE_TREES`];
/// of a class in [`DAMAGE_WEIGHTS`]. A tank, a healer or a caster keeps
/// their tier set.
fn damage_weights(
    data: &crate::game::data::GameData,
    p: &crate::game::map::player::Player,
    wants: Wants,
) -> Option<&'static Weights> {
    let class = p.row.class;
    let w = DAMAGE_WEIGHTS
        .iter()
        .find(|(c, _)| *c == class)
        .map(|(_, w)| w)?;
    let damage = match wants.role {
        Some(Role::Damage) => true,
        Some(_) => false,
        None => {
            matches!(class, 3 | 4) || best_tree(data, p).is_some_and(|t| DAMAGE_TREES.contains(&t))
        }
    };
    damage.then_some(w)
}

/// What an item's on-equip spells give a weapon damage dealer: hit and
/// crit (per cent), attack power and ranged attack power.
fn equip_bonuses(
    data: &crate::game::data::GameData,
    t: &crate::game::data::ItemTemplate,
) -> (f32, f32, f32, f32) {
    use crate::game::map::auras::{
        AURA_MOD_ATTACK_POWER, AURA_MOD_CRIT_PERCENT, AURA_MOD_HIT_CHANCE,
        AURA_MOD_RANGED_ATTACK_POWER,
    };
    const APPLY_AURA: u32 = 6;
    let mut out = (0.0, 0.0, 0.0, 0.0);
    for e in t
        .spells
        .iter()
        .filter(|s| s.trigger == ON_EQUIP && s.id != 0)
        .filter_map(|s| data.spells.get(s.id))
        .flat_map(|s| s.effects.iter())
        .filter(|e| e.effect == APPLY_AURA)
    {
        let v = (e.base_points + 1) as f32;
        match e.aura {
            AURA_MOD_HIT_CHANCE => out.0 += v,
            AURA_MOD_CRIT_PERCENT => out.1 += v,
            AURA_MOD_ATTACK_POWER => out.2 += v,
            AURA_MOD_RANGED_ATTACK_POWER => out.3 += v,
            _ => {}
        }
    }
    out
}

/// The hit an item gives, per cent.
fn item_hit(data: &crate::game::data::GameData, t: &crate::game::data::ItemTemplate) -> f32 {
    equip_bonuses(data, t).0
}

/// What item `t` is worth to a damage dealer of weights `w` with
/// `hit_left` hit to go to the cap, in `slot`: its stats and its on-equip
/// bonuses by their weights, its hit up to the cap, and a weapon's damage
/// a second by where it is held.
fn damage_score(
    data: &crate::game::data::GameData,
    w: &Weights,
    t: &crate::game::data::ItemTemplate,
    slot: u8,
    hit_left: f32,
) -> f32 {
    let mut score = 0.0;
    for (kind, value) in t.stats {
        let v = value as f32;
        score += v * match kind {
            3 => w.agility,
            4 => w.strength,
            5 => w.intellect,
            7 => w.stamina,
            _ => 0.0,
        };
    }
    let (hit, crit, ap, rap) = equip_bonuses(data, t);
    score += hit.min(hit_left.max(0.0)) * w.hit
        + crit * w.crit
        + ap * w.attack_power
        + rap * w.ranged_attack_power;
    if t.class == ITEM_WEAPON && t.delay > 0 {
        let dps = (t.damages[0].0 + t.damages[0].1) / 2.0 / (t.delay as f32 / 1000.0);
        score += dps
            * if slot == 17 {
                w.ranged_dps
            } else {
                w.melee_dps
            };
    }
    score
}

/// The hit a character's talents give, per cent: their passive auras'
/// (Precision, Surefooted, Nature's Guidance), counted against the cap.
fn talent_hit(data: &crate::game::data::GameData, p: &crate::game::map::player::Player) -> f32 {
    use crate::game::map::auras::AURA_MOD_HIT_CHANCE;
    data.spells
        .talents
        .values()
        .filter_map(|t| {
            t.ranks
                .iter()
                .rev()
                .find(|r| **r != 0 && p.spells.contains(r))
        })
        .filter_map(|r| data.spells.get(*r))
        .flat_map(|s| s.effects.iter())
        .filter(|e| e.effect == 6 && e.aura == AURA_MOD_HIT_CHANCE)
        .map(|e| (e.base_points + 1) as f32)
        .sum()
}

/// `.gear raid`'s choices for a slot, for a damage dealer: those the tier
/// drops (`loot`) first, the best by [`damage_score`] first; then the rest
/// as they came, should the tier have none that fits.
fn damage_order(
    data: &crate::game::data::GameData,
    w: &Weights,
    loot: &ahash::AHashSet<u32>,
    slot: u8,
    hit_left: f32,
    picks: Vec<u32>,
) -> Vec<u32> {
    let (dropped, rest): (Vec<u32>, Vec<u32>) = picks.into_iter().partition(|e| loot.contains(e));
    let mut scored: Vec<(f32, u32)> = dropped
        .into_iter()
        .map(|e| {
            (
                data.item(e)
                    .map_or(0.0, |t| damage_score(data, w, t, slot, hit_left)),
                e,
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, e)| e).chain(rest).collect()
}

/// A class's tier sets (`ItemSet.dbc`): tier 1's, Molten Core's, and tier
/// 2's, Blackwing Lair's (with Onyxia's and Ragnaros's helms and legs).
pub(crate) fn tier_sets(class: u8) -> Option<(u32, u32)> {
    Some(match class {
        1 => (209, 218),  // Battlegear of Might, of Wrath
        2 => (208, 217),  // Lawbringer, Judgement
        3 => (206, 215),  // Giantstalker, Dragonstalker
        4 => (204, 213),  // Nightslayer, Bloodfang
        5 => (202, 211),  // Vestments of Prophecy, of Transcendence
        7 => (207, 216),  // The Earthfury, The Ten Storms
        8 => (201, 210),  // Arcanist, Netherwind
        9 => (203, 212),  // Felheart, Nemesis
        11 => (205, 214), // Cenarion, Stormrage
        _ => return None,
    })
}

/// The equipment slot an armour piece of `inventory_type` goes in: head,
/// shoulders, chest (or robe), waist, legs, feet, wrists, hands.
fn armour_slot(inventory_type: u32) -> Option<u8> {
    Some(match inventory_type {
        1 => 0,
        3 => 2,
        5 | 20 => 4,
        6 => 5,
        7 => 6,
        8 => 7,
        9 => 8,
        10 => 9,
        _ => return None,
    })
}

/// The pieces of `class`'s tier set `tier` (1 or 2; either at random if
/// `None`) a character of `level` may wear, and the slots they go in:
/// none below 60, the sets' level.
fn tier_pieces(
    data: &crate::game::data::GameData,
    class: u8,
    level: u8,
    tier: Option<u8>,
) -> Vec<(u8, u32)> {
    let Some((one, two)) = tier_sets(class) else {
        return Vec::new();
    };
    let tier = tier.unwrap_or_else(|| rand::rng().random_range(1..=2));
    let set = if tier == 1 { one } else { two };
    let mut pieces: Vec<(u8, u32)> = data
        .items
        .values()
        .filter(|t| t.set_id == set && t.required_level <= u32::from(level))
        .filter_map(|t| Some((armour_slot(t.inventory_type)?, t.entry)))
        .collect();
    pieces.sort_unstable();
    pieces
}

/// An item no player could have: a developer's test, a placeholder, one
/// deprecated or unused, a creature's weapon, a level's test kit
/// (`item_template` has them, some epic: "TEST Level 80 Epic", "Naxxramas
/// Sword 1H 1 [PH]", "90 Epic Warrior Ring").
pub(crate) fn unreal(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("monster")
        // "90 Epic Warrior Ring", "63 Green Frost Staff": a level's test kit.
        || name
            .split(' ')
            .next()
            .is_some_and(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()))
        || name.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| {
            matches!(
                w,
                "test"
                    | "testing"
                    | "qatest"
                    | "deprecated"
                    | "depricated"
                    | "ph"
                    | "unused"
            )
        })
}

/// The stats a class looks for first, the best of a quality being of
/// them: strength and agility for warriors, rogues and hunters, intellect
/// for priests, mages and warlocks; paladins, shamans and druids any.
fn favours(class: u8, t: &crate::game::data::ItemTemplate) -> bool {
    const AGILITY: u32 = 3;
    const STRENGTH: u32 = 4;
    const INTELLECT: u32 = 5;
    let has = |want: &[u32]| t.stats.iter().any(|(s, v)| *v > 0 && want.contains(s));
    match class {
        1 | 3 | 4 => has(&[AGILITY, STRENGTH]),
        5 | 8 | 9 => has(&[INTELLECT]),
        _ => true,
    }
}

/// The items `.gear` may give a character of `class`, `race` and `level`
/// of the inventory types `types` that `fits` takes, asking for no skill,
/// spell, rank or standing, and not temporary: at random, uncommon or
/// rare, of a required level within ten of theirs and no higher; or the
/// best first, of any required level up to theirs, real (not
/// [`unreal`]), by quality, then the stats the class [`favours`], then
/// item level.
fn gear_pool(
    data: &crate::game::data::GameData,
    who: (u8, u8, u8),
    pick: Pick,
    types: &[u32],
    fits: impl Fn(&crate::game::data::ItemTemplate) -> bool,
) -> Vec<u32> {
    let pool = gear_pool_of(data, who, pick, types, &fits);
    match pick {
        // None of the quality asked for in this slot: of both.
        Pick::Random { quality: Some(_) } if pool.is_empty() => {
            gear_pool_of(data, who, Pick::Random { quality: None }, types, &fits)
        }
        _ => pool,
    }
}

/// [`gear_pool`], of `pick`'s quality alone.
fn gear_pool_of(
    data: &crate::game::data::GameData,
    (class, race, level): (u8, u8, u8),
    pick: Pick,
    types: &[u32],
    fits: &impl Fn(&crate::game::data::ItemTemplate) -> bool,
) -> Vec<u32> {
    use rand::seq::SliceRandom;
    let allows = |mask: i32, of: u8| mask == -1 || mask == 0 || mask & (1 << (of.max(1) - 1)) != 0;
    let level = u32::from(level);
    // Each type's items, not all of them: a look through every item for
    // each of the twenty or so slots was 2.3 ms of the map's thread a
    // `.gear` (docs/PERFORMANCE.md, "Arrival").
    let mut out: Vec<&crate::game::data::ItemTemplate> = types
        .iter()
        .flat_map(|t| data.items_of_type(*t))
        .filter(|t| match pick {
            Pick::Random { quality } => {
                quality.map_or((UNCOMMON..=RARE).contains(&t.quality), |q| t.quality == q)
                    && t.required_level <= level
                    && t.required_level + 10 >= level
            }
            Pick::Raid { .. } => {
                (2..=4).contains(&t.quality) && t.required_level <= level && !unreal(&t.name)
            }
        })
        .filter(|t| {
            t.required_skill == 0
                && t.required_spell == 0
                && t.required_honor_rank == 0
                && t.required_reputation_faction == 0
                && t.duration == 0
        })
        .filter(|t| allows(t.allowable_class, class) && allows(t.allowable_race, race))
        .filter(|t| fits(t))
        .collect();
    out.sort_unstable_by_key(|t| t.entry);
    match pick {
        Pick::Random { .. } => out.shuffle(&mut rand::rng()),
        Pick::Raid { .. } => out.sort_by_key(|t| {
            std::cmp::Reverse((t.quality, favours(class, t), t.item_level, t.required_level))
        }),
    }
    out.into_iter().map(|t| t.entry).collect()
}

/// Learn a proficiency (or other) spell, and the skills of its line at
/// their caps.
fn teach(map: &mut Map, g: Guid, spell: u32) {
    use crate::game::map::skills;
    crate::game::map::spells::learn(map, g, spell);
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    if !p.spells.contains(&spell) {
        return;
    }
    let lines: Vec<u32> = data
        .spells
        .skill_abilities
        .get(&spell)
        .into_iter()
        .flatten()
        .map(|a| a.skill)
        .collect();
    for skill in lines {
        if skills::value(p, skill).is_none() {
            skills::learn_skill(p, skill, 1);
        }
        if let Some(max) = skills::max_value(p, skill) {
            skills::set_value(p, skill, max);
        }
    }
}

/// `.gear`: the selected player, or the GM, in random uncommon and rare
/// gear for their class and level in every slot it can fill (but the
/// shirt and tabard), what was worn there before gone: armour of the kind
/// the class wears, a cloak, a necklace, two rings and two trinkets, the
/// weapons the class may hold (a two-hander, or a one-hander with a
/// shield, a second weapon or an off-hand item), and a ranged weapon or a
/// wand, with ammunition for one that takes it: a hunter's in the best
/// quiver or ammo pouch for them, on the bag bar, filled
/// ([`quiver_and_ammo`]); another's four stacks loose. The proficiencies and
/// weapon skills the items need are taught, the skills at their caps, and
/// Auto Shot and Shoot with a ranged weapon or a wand. One alive is left
/// at full health and mana, as the reply says.
///
/// `.gear green` and `.gear blue` take only the uncommon, or only the
/// rare, where a slot has any of them (see [`Pick`]): a fresh level's
/// gear, or a dungeon's.
///
/// `.gear raid` is a raider's gear instead (see [`Pick`]): at 60 their
/// class's tier set, 1 or 2, where it goes, and everywhere else the best
/// there is for them. A weapon damage dealer ([`damage_weights`]: `dps`, a
/// rogue or a hunter, or most talents in Arms, Fury, Retribution,
/// Enhancement or Feral Combat) wears no tier set: every slot is the best
/// of the tier's raid loot ([`GameData::raid_loot`]) by their class's
/// weights ([`DAMAGE_WEIGHTS`]), hit counted up to [`HIT_CAP`] less their
/// talents' (the user's call, 2026-10-24); a slot the loot has nothing for
/// is filled as before.
///
/// [`GameData::raid_loot`]: crate::game::data::GameData::raid_loot
///
/// After the quality may come what the hands are for ([`Wants`]): a role
/// (`tank`, `healer` or `caster`, `dps`: [`role_ways`]), a way of holding
/// (`shield`, `2h`, `dual`, `offhand`) and a kind of weapon (`dagger`,
/// `sword`, `mace`, `axe`, `fist`, `staff`, `polearm`), each held as the
/// class may; with neither a role nor a way, the talents spent say
/// ([`spec_ways`]: a Protection warrior's or paladin's shield, a Holy
/// paladin's, an Elemental or Restoration shaman's), else it is at random.
fn gear(map: &mut Map, g: Guid, pick: Pick, wants: Wants) -> Option<String> {
    use crate::game::map::inventory::{self, Destination, SLOT_MAIN_HAND, SLOT_OFF_HAND};
    use crate::game::store::BAG_PLAYER;
    use rand::seq::SliceRandom;
    let data = map.realm.data.clone();
    let to = player_target(map, g);
    let p = map.players.get(&to)?;
    let who = (p.row.class, p.row.race, p.level());
    let (class, _, level) = who;
    // The tier, at random if not asked; a weapon damage dealer's weights
    // and the hit their talents give.
    let pick = match pick {
        Pick::Raid { tier: None } => Pick::Raid {
            tier: Some(rand::rng().random_range(1..=2)),
        },
        other => other,
    };
    let damage = match pick {
        Pick::Raid { tier: Some(tier) } => {
            damage_weights(&data, p, wants).map(|w| (w, tier, HIT_CAP - talent_hit(&data, p)))
        }
        _ => None,
    };
    let kind = armour_of(class, level);
    let armour = |t: &crate::game::data::ItemTemplate| t.class == ITEM_ARMOR && t.subclass == kind;
    let trinket = |t: &crate::game::data::ItemTemplate| t.class == ITEM_ARMOR;
    let one_hand = |t: &crate::game::data::ItemTemplate| {
        t.class == ITEM_WEAPON && weapon_for(&data, t.subclass, class)
    };
    // The kind of weapon asked for, in the hands that hold one.
    let of_kind = |t: &crate::game::data::ItemTemplate, kind: Option<WeaponKind>| {
        one_hand(t) && kind.is_none_or(|k| k.fits(t.subclass))
    };
    let mut plan: Vec<(u8, Vec<u32>)> = vec![
        (0, gear_pool(&data, who, pick, &[1], armour)),
        (1, gear_pool(&data, who, pick, &[2], trinket)),
        (2, gear_pool(&data, who, pick, &[3], armour)),
        (4, gear_pool(&data, who, pick, &[5, 20], armour)),
        (5, gear_pool(&data, who, pick, &[6], armour)),
        (6, gear_pool(&data, who, pick, &[7], armour)),
        (7, gear_pool(&data, who, pick, &[8], armour)),
        (8, gear_pool(&data, who, pick, &[9], armour)),
        (9, gear_pool(&data, who, pick, &[10], armour)),
        (10, gear_pool(&data, who, pick, &[11], trinket)),
        (11, gear_pool(&data, who, pick, &[11], trinket)),
        (12, gear_pool(&data, who, pick, &[12], trinket)),
        (13, gear_pool(&data, who, pick, &[12], trinket)),
        (
            14,
            gear_pool(&data, who, pick, &[16], |t| {
                t.class == ITEM_ARMOR && t.subclass == 1
            }),
        ),
    ];
    // A raider at 60: their tier set's pieces first where they go; but a
    // weapon damage dealer's pieces are the tier's loot by their weights.
    if let Pick::Raid { tier } = pick
        && damage.is_none()
    {
        for (slot, piece) in tier_pieces(&data, class, level, tier) {
            if let Some((_, picks)) = plan.iter_mut().find(|(s, _)| *s == slot) {
                picks.insert(0, piece);
            }
        }
    }
    // The hands: a way of holding the class has that the items let them
    // fill both of: the one asked for, else the role's, else the talents',
    // first; the rest at random. A kind of weapon asked for is held a way
    // it can be, if the class may use it at all.
    let mut ways = hands_of(class, level);
    ways.shuffle(&mut rand::rng());
    let mut wanted = match (wants.hands, wants.role) {
        (Some(h), _) => vec![h],
        (None, Some(role)) => role_ways(class, role),
        (None, None) => map
            .players
            .get(&to)
            .map_or_else(Vec::new, |p| spec_ways(&data, p)),
    };
    wanted.retain(|w| ways.contains(w));
    ways.retain(|w| !wanted.contains(w));
    let mut ways: Vec<Hands> = wanted.into_iter().chain(ways).collect();
    let kind = wants.weapon.filter(|k| {
        data.items
            .values()
            .any(|t| t.class == ITEM_WEAPON && k.fits(t.subclass) && one_hand(t))
    });
    if let Some(k) = kind {
        ways.sort_by_key(|w| !k.held(*w));
    }
    let fill = |way: Hands| {
        let kind = kind.filter(|k| k.held(way));
        let main = match way {
            Hands::TwoHanded => gear_pool(&data, who, pick, &[17], |t| of_kind(t, kind)),
            _ => gear_pool(&data, who, pick, &[13, 21], |t| of_kind(t, kind)),
        };
        let off = match way {
            Hands::TwoHanded => Vec::new(),
            Hands::Shield => gear_pool(&data, who, pick, &[14], |t| {
                t.class == ITEM_ARMOR
                    && t.subclass == ARMOR_SHIELD
                    && class_gets(&data, 9116, class)
            }),
            Hands::DualWield => gear_pool(&data, who, pick, &[13, 22], |t| of_kind(t, kind)),
            Hands::HeldInOffHand => gear_pool(&data, who, pick, &[23], trinket),
        };
        (way, main, off)
    };
    let filled: Vec<(Hands, Vec<u32>, Vec<u32>)> = ways.into_iter().map(fill).collect();
    let hands = filled
        .iter()
        .find(|(way, main, off)| !main.is_empty() && (*way == Hands::TwoHanded || !off.is_empty()))
        .or_else(|| filled.iter().find(|(_, main, _)| !main.is_empty()))
        .cloned();
    if let Some((way, main, off)) = &hands {
        plan.push((SLOT_MAIN_HAND, main.clone()));
        if *way == Hands::TwoHanded {
            plan.push((SLOT_OFF_HAND, Vec::new()));
        } else {
            plan.push((SLOT_OFF_HAND, off.clone()));
        }
    }
    let ranged = ranged_of(class);
    plan.push((
        17,
        gear_pool(&data, who, pick, &[15, 25, 26], |t| {
            t.class == ITEM_WEAPON
                && ranged.contains(&t.subclass)
                && weapon_for(&data, t.subclass, class)
        }),
    ));
    // What was worn where something new goes is gone; and the off hand
    // with a two-hander.
    let two_handed = hands
        .as_ref()
        .is_some_and(|(w, m, _)| *w == Hands::TwoHanded && !m.is_empty());
    for (slot, picks) in &plan {
        if picks.is_empty() && !(two_handed && *slot == SLOT_OFF_HAND) {
            continue;
        }
        let worn = map.players.get(&to).and_then(|p| {
            p.item_at(BAG_PLAYER, *slot)
                .map(|i| (i, p.items[i].count()))
        });
        if let Some((i, n)) = worn {
            inventory::take_item(map, to, i, n);
        }
    }
    if hands
        .as_ref()
        .is_some_and(|(w, _, _)| *w == Hands::DualWield)
    {
        teach(map, to, DUAL_WIELD);
    }
    let mut names = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut hit_left = damage.map_or(0.0, |(_, _, cap)| cap);
    for (slot, picks) in plan {
        let picks = match damage {
            Some((w, tier, _)) => {
                damage_order(&data, w, data.raid_loot(tier), slot, hit_left, picks)
            }
            None => picks,
        };
        // A few tries: one may be unique and worn already, as the first
        // ring (the store refuses a second of it, `may_equip_new`), or a
        // unique one of the same name worn (the battlegrounds' rings are
        // one name at several levels, alike to see): one of them.
        for entry in picks.into_iter().take(20) {
            let Some(t) = data.item(entry) else {
                continue;
            };
            if t.max_count > 0 && wears_named(map, to, &data, &t.name) {
                continue;
            }
            if let Some(spell) = proficiency_spell(t) {
                teach(map, to, spell);
            }
            if inventory::add_item_to(map, to, entry, 1, Destination::Worn(slot)).is_ok() {
                names.push(t.name.clone());
                hit_left -= item_hit(&data, t);
                if slot == 17 {
                    if t.subclass == WEAPON_WAND {
                        teach(map, to, SHOOT);
                    } else if class == 3 {
                        teach(map, to, AUTO_SHOT);
                    }
                    if t.ammo_type != 0 && class == 3 {
                        let (got, why) = quiver_and_ammo(map, to, t.ammo_type, pick);
                        names.extend(got);
                        notes.extend(why);
                    } else if t.ammo_type != 0 {
                        names.extend(give_ammo(map, to, t.ammo_type, level, pick));
                    }
                }
                break;
            }
        }
    }
    if names.is_empty() {
        return Some("No gear to equip.".into());
    }
    if let Some((_, tier, cap)) = damage {
        notes.push(format!(
            "Damage pieces of raid tier {tier}, {:.0}% hit from gear.",
            cap - hit_left
        ));
    }
    // A GM's convenience, not 1.12's: the gear's stamina and intellect
    // raise their maximum health and mana, and they are left at it, not
    // where they were (the user's raiders came at half health).
    let whole = map.players.get_mut(&to).is_some_and(|p| {
        if p.death != crate::game::map::death::Death::Alive {
            return false;
        }
        let (health, mana) = (
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXPOWER1),
        );
        p.fields.set_u32(UNIT_FIELD_HEALTH, health);
        p.fields.set_u32(UNIT_FIELD_POWER1, mana);
        true
    });
    if whole {
        crate::game::map::combat::sync_aura_state(map, to);
    }
    Some(format!(
        "Equipped {}.{}{}",
        names.join(", "),
        if whole {
            " At full health and mana."
        } else {
            ""
        },
        notes.iter().map(|n| format!(" {n}")).collect::<String>()
    ))
}

const ENCHANT_USAGE: &str = "Usage: .gear enchant <slot> <enchantment>: a slot 0 to 18 or head, \
neck, shoulders, chest, waist, legs, feet, wrists, hands, finger1, finger2, trinket1, trinket2, back, \
mainhand, offhand, ranged";

/// An equipment slot by its number (0 to 18) or name.
fn equipment_slot(s: &str) -> Option<u8> {
    if let Ok(n) = s.parse::<u8>() {
        return (n < crate::game::map::inventory::EQUIPMENT_END).then_some(n);
    }
    Some(match s.to_ascii_lowercase().as_str() {
        "head" => 0,
        "neck" => 1,
        "shoulders" | "shoulder" => 2,
        "shirt" => 3,
        "chest" => 4,
        "waist" => 5,
        "legs" => 6,
        "feet" => 7,
        "wrists" | "wrist" => 8,
        "hands" => 9,
        "finger1" => 10,
        "finger2" => 11,
        "trinket1" => 12,
        "trinket2" => 13,
        "back" => 14,
        "mainhand" => 15,
        "offhand" => 16,
        "ranged" => 17,
        "tabard" => 18,
        _ => return None,
    })
}

/// `.gear enchant <slot> <enchantment>`: a GM's, not 1.12's: the selected
/// player's, or the GM's, item worn in `slot` given the lasting
/// enchantment (`SpellItemEnchantment`'s id) an enchanter would put on, in
/// its permanent slot, what was there replaced; its stats, procs and glow
/// as an enchanter's (`enchant::enchant_item`).
fn gear_enchant(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    use crate::game::store::BAG_PLAYER;
    let (Some(slot), Some(id)) = (
        args.first().and_then(|s| equipment_slot(s)),
        num::<u32>(args.get(1)),
    ) else {
        return Some(ENCHANT_USAGE.into());
    };
    if args.len() != 2 {
        return Some(ENCHANT_USAGE.into());
    }
    let data = map.realm.data.clone();
    let Some(enchantment) = data.spells.enchantments.get(&id) else {
        return Some(format!("No enchantment {id}."));
    };
    let to = player_target(map, g);
    let p = map.players.get(&to)?;
    let Some(i) = p.item_at(BAG_PLAYER, slot) else {
        return Some(format!("Nothing worn in slot {slot}."));
    };
    let (item, entry) = (p.items[i].guid, p.items[i].entry);
    let name = data
        .item(entry)
        .map_or_else(|| format!("item {entry}"), |t| t.name.clone());
    if !crate::game::map::enchant::enchant_item(map, to, item, id, None) {
        return Some(format!("Nothing worn in slot {slot}."));
    }
    Some(format!("Enchanted {name} with {}.", enchantment.name))
}

/// Whether `g` wears an item named `name`.
fn wears_named(map: &Map, g: Guid, data: &crate::game::data::GameData, name: &str) -> bool {
    use crate::game::map::inventory::EQUIPMENT_END;
    use crate::game::store::BAG_PLAYER;
    map.players.get(&g).is_some_and(|p| {
        p.items.iter().any(|i| {
            i.bag == BAG_PLAYER
                && i.slot < EQUIPMENT_END
                && data.item(i.entry).is_some_and(|t| t.name == name)
        })
    })
}

/// `item_template`'s flags: deprecated (a "Depricated Razor Arrow", a
/// "BKP \"Impact\" Shot").
const ITEM_FLAG_DEPRECATED: u32 = 0x10;

/// The damage a second a projectile adds, in hundredths: its damage's
/// mean (`derive`'s ranged bonus, the mean times the weapon's speed).
fn ammo_dps(t: &crate::game::data::ItemTemplate) -> u32 {
    ((t.damages[0].0 + t.damages[0].1) * 50.0).max(0.0) as u32
}

/// The best ammunition of `ammo_type` (arrows, bullets) for `level`: the
/// most damage a second, then the higher level; of their level or below
/// ([`gear_level`]), real, uncommon or rare at most, needing no skill or
/// standing. `.gear`'s (and `green`'s, `blue`'s) is what anyone may buy
/// or make: sold to all, or a craft's (Explosive Arrows, Thorium Shells);
/// `.gear raid`'s is the best got any way but a standing: a dungeon
/// boss's drop (Doomshot, Miniature Cannon Balls) or a quest's reward
/// too. Without the databases' sources (the fallback world), any.
pub(crate) fn best_ammo(
    data: &crate::game::data::GameData,
    ammo_type: u32,
    level: u8,
    raid: bool,
) -> Option<&crate::game::data::ItemTemplate> {
    use crate::game::data::ammo_source::*;
    let from = if raid {
        VENDOR | CRAFTED | DROP | QUEST
    } else {
        VENDOR | CRAFTED
    };
    let known = data.is_loaded();
    data.items
        .values()
        .filter(|t| t.class == ITEM_PROJECTILE && t.subclass == ammo_type)
        .filter(|t| gear_level(t) <= u32::from(level) && t.quality <= 3)
        .filter(|t| t.required_reputation_faction == 0 && t.required_skill == 0)
        .filter(|t| t.flags & ITEM_FLAG_DEPRECATED == 0 && !unreal(&t.name))
        .filter(|t| !known || data.ammo_source(t.entry) & from != 0)
        .max_by_key(|t| {
            (
                ammo_dps(t),
                gear_level(t),
                t.item_level,
                std::cmp::Reverse(t.entry),
            )
        })
}

/// Four stacks of the best ammunition of `ammo_type` (arrows, bullets) of
/// `level` or below, topped up to that (a `.gear` again adds none to four
/// stacks carried), and that chosen for the ranged weapon: its name and
/// count, if any went. A hunter's goes in a quiver instead
/// ([`quiver_and_ammo`]); this is a warrior's or a rogue's, or a hunter's
/// with no quiver for them.
fn give_ammo(map: &mut Map, g: Guid, ammo_type: u32, level: u8, pick: Pick) -> Option<String> {
    let data = map.realm.data.clone();
    let ammo = best_ammo(&data, ammo_type, level, matches!(pick, Pick::Raid { .. }))?;
    let p = map.players.get(&g)?;
    let want = (ammo.stackable.max(1) * 4).saturating_sub(p.carried_count_of(ammo.entry));
    let n = crate::game::map::inventory::fitting(p, &data, ammo.entry, want).ok()?;
    if n == 0 {
        crate::game::map::ranged::set_ammo(map, g, ammo.entry);
        return None;
    }
    crate::game::map::inventory::add_item(map, g, ammo.entry, n, true).ok()?;
    crate::game::map::ranged::set_ammo(map, g, ammo.entry);
    Some(format!("{} {}", grouped(n), ammo.name))
}

/// `n` with its thousands grouped: 3,600.
fn grouped(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Item class 11: a quiver (subclass 2, for arrows) or an ammo pouch (3,
/// for bullets), worn on the bag bar.
const ITEM_QUIVER: u32 = 11;
/// An item spell's trigger: on equip.
const ON_EQUIP: u32 = 1;

/// The ranged haste a quiver's or pouch's on-equip spell gives, in per
/// cent (`SPELL_AURA_MOD_RANGED_AMMO_HASTE`: Ancient Sinew Wrapped
/// Lamina's 15).
fn ammo_haste(data: &crate::game::data::GameData, t: &crate::game::data::ItemTemplate) -> i32 {
    t.spells
        .iter()
        .filter(|s| s.trigger == ON_EQUIP && s.id != 0)
        .filter_map(|s| data.spells.get(s.id))
        .flat_map(|s| s.effects.iter())
        .filter(|e| e.effect == 6 && e.aura == crate::game::map::auras::AURA_MOD_RANGED_AMMO_HASTE)
        .map(|e| e.base_points + 1)
        .sum()
}

/// How a quiver ranks: its slots, then its haste.
fn quiver_rank(
    data: &crate::game::data::GameData,
    t: &crate::game::data::ItemTemplate,
) -> (u32, i32) {
    (t.container_slots, ammo_haste(data, t))
}

/// The level `.gear` takes an item at: its required level, or, with none
/// (the Quiver of the Night Watch, a 12-slot quest reward of item level
/// 25, would be a level 1's), five under its item level, as the quivers
/// with one have it (the Medium Quiver's 10 and 15).
fn gear_level(t: &crate::game::data::ItemTemplate) -> u32 {
    if t.required_level > 0 {
        t.required_level
    } else {
        t.item_level.saturating_sub(5)
    }
}

/// The best quiver (for `ammo_type` 2, arrows) or ammo pouch (3, bullets)
/// for a character of `class`, `race` and `level`: the most slots, then
/// the most haste; of their level or below ([`gear_level`]), uncommon or
/// rare (epic too for `.gear raid`: Ancient Sinew Wrapped Lamina), real
/// (not [`unreal`]), needing no skill, spell, rank or standing.
fn best_quiver(
    data: &crate::game::data::GameData,
    (class, race, level): (u8, u8, u8),
    ammo_type: u32,
    pick: Pick,
) -> Option<&crate::game::data::ItemTemplate> {
    let subclass = match ammo_type {
        2 => 2,
        3 => 3,
        _ => return None,
    };
    let best = match pick {
        Pick::Random { .. } => RARE,
        Pick::Raid { .. } => 4,
    };
    let allows = |mask: i32, of: u8| mask == -1 || mask == 0 || mask & (1 << (of.max(1) - 1)) != 0;
    data.items_of_type(18)
        .filter(|t| t.class == ITEM_QUIVER && t.subclass == subclass && t.container_slots > 0)
        .filter(|t| t.quality <= best && gear_level(t) <= u32::from(level))
        .filter(|t| {
            t.required_skill == 0
                && t.required_spell == 0
                && t.required_honor_rank == 0
                && t.required_reputation_faction == 0
                && t.duration == 0
                && !unreal(&t.name)
        })
        .filter(|t| allows(t.allowable_class, class) && allows(t.allowable_race, race))
        .max_by_key(|t| (quiver_rank(data, t), t.quality, std::cmp::Reverse(t.entry)))
}

/// A hunter's quiver or ammo pouch, for the ranged weapon's `ammo_type`,
/// and a full one of the best ammunition: what is named in the reply (the
/// bag if it is new, the ammunition if any went), and why there is no bag
/// if there could be none (no room for the items of the bag it would
/// replace). With no bag, four stacks loose ([`give_ammo`]).
fn quiver_and_ammo(
    map: &mut Map,
    g: Guid,
    ammo_type: u32,
    pick: Pick,
) -> (Vec<String>, Option<String>) {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get(&g) else {
        return (Vec::new(), None);
    };
    let who = (p.row.class, p.row.race, p.level());
    let level = who.2;
    let (Some(bag), Some(ammo)) = (
        best_quiver(&data, who, ammo_type, pick),
        best_ammo(&data, ammo_type, level, matches!(pick, Pick::Raid { .. })),
    ) else {
        return (
            give_ammo(map, g, ammo_type, level, pick)
                .into_iter()
                .collect(),
            None,
        );
    };
    let mut names = Vec::new();
    let slot = match wear_quiver(map, g, bag) {
        Ok((slot, new)) => {
            if new {
                names.push(bag.name.clone());
            }
            slot
        }
        Err(why) => {
            names.extend(give_ammo(map, g, ammo_type, level, pick));
            return (names, why);
        }
    };
    if let Some(n) = fill_quiver(map, g, slot, ammo.entry) {
        names.push(format!("{} {}", grouped(n), ammo.name));
    }
    crate::game::map::ranged::set_ammo(map, g, ammo.entry);
    (names, None)
}

/// Put the quiver or pouch `bag` on `g`'s bag bar, unless one as good of
/// its kind is there: its slot, and whether it is new. It goes in place
/// of the quiver or pouch worn (its ammunition with it if of the other
/// kind; kept in it if of this kind, whose slots it has at least), else
/// into an empty bag slot, else in place of the smallest bag, whose items
/// are moved to other room. With no room for them, or no bag to replace,
/// nothing changes: why, for the reply, if there is a why.
fn wear_quiver(
    map: &mut Map,
    g: Guid,
    bag: &crate::game::data::ItemTemplate,
) -> Result<(u8, bool), Option<String>> {
    use crate::game::map::inventory::{self, BAG_END, BAG_START, Destination, Proficiency};
    use crate::game::store::BAG_PLAYER;
    let data = map.realm.data.clone();
    let p = map.players.get(&g).ok_or(None)?;
    let worn = |s: u8| {
        p.item_at(BAG_PLAYER, s)
            .and_then(|i| data.item(p.items[i].entry))
    };
    let quiver = (BAG_START..BAG_END).find(|s| worn(*s).is_some_and(|t| t.class == ITEM_QUIVER));
    if let Some(s) = quiver
        && let Some(t) = worn(s)
        && t.bag_family == bag.bag_family
        && quiver_rank(&data, t) >= quiver_rank(&data, bag)
    {
        return Ok((s, false));
    }
    let slot = quiver
        .or_else(|| (BAG_START..BAG_END).find(|s| p.item_at(BAG_PLAYER, *s).is_none()))
        .or_else(|| {
            (BAG_START..BAG_END)
                .filter(|s| worn(*s).is_some_and(|t| t.class == 1))
                .min_by_key(|s| p.bag_size(&data, *s))
        })
        .ok_or(None)?;
    if inventory::can_equip(p, &Proficiency::of(p, &data), bag, slot).is_err()
        || (bag.max_count > 0 && p.count_of(bag.entry) >= bag.max_count)
    {
        return Err(None);
    }
    let old = worn(slot);
    let inside: Vec<usize> = (0..p.items.len())
        .filter(|i| p.items[*i].bag == slot)
        .collect();
    // What is in the bag replaced: a quiver's of this kind stays where it
    // is (the new one has as many slots or more); one of the other kind
    // goes with it, its ammunition no use to the new weapon; a bag's goes
    // to free room elsewhere, or the bag stays.
    let mut gone = Vec::new();
    let mut moves = Vec::new();
    match old {
        Some(o) if o.class == ITEM_QUIVER && o.bag_family == bag.bag_family => {}
        Some(o) if o.class == ITEM_QUIVER => gone = inside,
        Some(o) => {
            let mut taken: Vec<(u8, u8)> = Vec::new();
            for i in inside {
                let t = data.item(p.items[i].entry);
                let to = crate::game::map::items::carry_slots(&data, p, t)
                    .into_iter()
                    .find(|at| {
                        at.0 != slot && p.item_at(at.0, at.1).is_none() && !taken.contains(at)
                    });
                let Some(to) = to else {
                    return Err(Some(format!(
                        "No room for the {}'s items, so no {}.",
                        o.name, bag.name
                    )));
                };
                taken.push(to);
                moves.push((p.items[i].guid, to));
            }
        }
        None => {}
    }
    let gone: Vec<Guid> = gone.iter().map(|i| p.items[*i].guid).collect();
    let old = p.item_at(BAG_PLAYER, slot).map(|i| p.items[i].guid);
    if let Some(p) = map.players.get_mut(&g) {
        for (item, (b, s)) in moves {
            if let Some(i) = p.item_by_guid(item) {
                p.items[i].bag = b;
                p.items[i].slot = s;
            }
        }
        p.refresh_inventory(&data);
    }
    for item in gone.into_iter().chain(old) {
        if let Some(i) = map.players.get(&g).and_then(|p| p.item_by_guid(item)) {
            let n = map.players[&g].items[i].count();
            inventory::take_item(map, g, i, n);
        }
    }
    if let Err(f) = inventory::add_item_to(map, g, bag.entry, 1, Destination::Worn(slot)) {
        tracing::debug!(?f, entry = bag.entry, "gear: the quiver wasn't put on");
        // What was left in the slot's place goes, not left in no bag.
        while let Some(i) = map
            .players
            .get(&g)
            .and_then(|p| p.items.iter().position(|i| i.bag == slot))
        {
            let n = map.players[&g].items[i].count();
            inventory::take_item(map, g, i, n);
        }
        return Err(None);
    }
    Ok((slot, true))
}

/// Fill the quiver or pouch in bag slot `slot` with `ammo`, to its
/// slots × the stack: other ammunition in it goes (as `.gear` replaces
/// what is worn; it would be shot no more), stacks of `ammo` carried
/// elsewhere are moved in, and what is still missing is given, so a
/// `.gear` again gives none. How many were given, if any.
fn fill_quiver(map: &mut Map, g: Guid, slot: u8, ammo: u32) -> Option<u32> {
    use crate::game::map::inventory::{self, Destination, EQUIPMENT_END};
    use crate::game::store::BAG_PLAYER;
    let data = map.realm.data.clone();
    while let Some(i) = map.players.get(&g).and_then(|p| {
        p.items
            .iter()
            .position(|i| i.bag == slot && i.entry != ammo)
    }) {
        let n = map.players[&g].items[i].count();
        inventory::take_item(map, g, i, n);
    }
    let stack = data.item(ammo)?.stackable.max(1);
    let p = map.players.get_mut(&g)?;
    let size = p.bag_size(&data, slot);
    let loose: Vec<usize> = (0..p.items.len())
        .filter(|i| {
            let it = &p.items[*i];
            it.entry == ammo
                && it.carried()
                && it.bag != slot
                && !(it.bag == BAG_PLAYER && it.slot < EQUIPMENT_END)
        })
        .collect();
    let mut free = (0..size)
        .filter(|s| p.item_at(slot, *s).is_none())
        .collect::<Vec<_>>()
        .into_iter();
    let mut moved = false;
    for i in loose {
        let Some(s) = free.next() else {
            break;
        };
        p.items[i].bag = slot;
        p.items[i].slot = s;
        moved = true;
    }
    if moved {
        p.refresh_inventory(&data);
    }
    let room: u32 = (0..size)
        .map(|s| {
            p.item_at(slot, s)
                .map_or(stack, |i| stack.saturating_sub(p.items[i].count()))
        })
        .sum();
    if room == 0 {
        return None;
    }
    inventory::add_item_to(map, g, ammo, room, Destination::Bag(slot, None)).ok()?;
    Some(room)
}

fn damage(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let Some(amount) = num::<u32>(args.first()).filter(|a| *a > 0) else {
        return Some("Usage: .damage <amount>".into());
    };
    let t = selected(map, g);
    if map.unit_view(t).is_none_or(|u| !u.alive) || t == g {
        return Some("Select a living unit.".into());
    }
    if guid::is_unit(t) {
        crate::game::map::ai::attacked(map, t, g);
    }
    crate::game::map::combat::deal_damage(map, g, t, amount);
    None
}

fn kill(map: &mut Map, g: Guid) -> Option<String> {
    let t = selected(map, g);
    if map.unit_view(t).is_none_or(|u| !u.alive) || t == g {
        return Some("Select a living unit.".into());
    }
    let health = map
        .unit_fields(t)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH));
    if guid::is_unit(t) {
        crate::game::map::ai::attacked(map, t, g);
    }
    crate::game::map::combat::deal_damage(map, g, t, health.max(1));
    None
}

const RESPAWN_USAGE: &str = "Usage: .respawn [radius], a radius of 500 yards or less";

/// `.respawn`: the selected creature, dead or alive, back at its spawn as
/// new.
fn respawn(map: &mut Map, g: Guid) -> Option<String> {
    use crate::game::map::creature::Life;
    let t = selected(map, g);
    let now = map.now;
    let Some(c) = map.creature_mut(t) else {
        return Some("Select a creature.".into());
    };
    if c.spawn_guid.is_none() {
        return Some("That creature has no spawn to come back to.".into());
    }
    let was_alive = c.alive();
    c.life = Life::Dead {
        corpse_until: now,
        respawn_at: now,
    };
    if was_alive || !map.dead_creatures.contains(&t) {
        map.dead_creatures.push(t);
    }
    Some("Respawning.".into())
}

/// `.move [x y z]`: the selected creature to the GM, or to a place.
fn move_target(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let t = selected(map, g);
    let to = match (
        crate::game::map::gm::parse_coordinate(args.first()),
        crate::game::map::gm::parse_coordinate(args.get(1)),
        crate::game::map::gm::parse_coordinate(args.get(2)),
    ) {
        (Some(x), Some(y), Some(z)) => Vector3d { x, y, z },
        _ => map.players.get(&g)?.position(),
    };
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(t) else {
        return Some("Select a creature.".into());
    };
    let (old_cell, anchor) = (c.cell, c.vis_anchor);
    c.position = to;
    c.home = to;
    c.motion = crate::game::map::creature::Motion::Idle { until: None };
    let new_cell = crate::game::map::grid::cell_of(to.x, to.y);
    c.cell = new_cell;
    map.grid.relocate(t, old_cell, new_cell);
    let frame = crate::game::map::ai::stop_packet(t, to, spline_id);
    map.send_to_observers(t, &frame, false);
    crate::game::map::visibility::moved(map, t, anchor, to);
    map.visibility_dirty.insert(t);
    Some(format!("Moved to {:.1}, {:.1}, {:.1}.", to.x, to.y, to.z))
}

fn threat(map: &Map, g: Guid) -> Option<String> {
    let t = selected(map, g);
    let Some(c) = map.creature(t) else {
        return Some("Select a creature.".into());
    };
    if c.threat.is_empty() {
        return Some("No threat.".into());
    }
    let mut list = c.threat.clone();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    let names: Vec<String> = list
        .iter()
        .map(|(who, amount)| {
            let name = map
                .players
                .get(who)
                .map(|p| p.name.to_string())
                .or_else(|| map.creature(*who).map(|c| format!("creature {}", c.entry)))
                .unwrap_or_else(|| who.to_string());
            format!("{name} {amount:.0}")
        })
        .collect();
    Some(format!("Threat: {}", names.join(", ")))
}

fn pos(map: &Map, g: Guid) -> Option<String> {
    let p = map.players.get(&g)?;
    let at = p.position();
    Some(format!(
        "{}: x {:.2}, y {:.2}, z {:.2}, orientation {:.2}, zone {}, area {}",
        map.key, at.x, at.y, at.z, p.movement.orientation, p.zone, p.area
    ))
}

fn modify(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let (Some(what), Some(value)) = (args.first(), float(args.get(1))) else {
        return Some("Usage: .modify hp|mana|rage|energy|money|speed <value>".into());
    };
    let to = player_target(map, g);
    let p = map.players.get_mut(&to)?;
    let set_power = |p: &mut crate::game::map::player::Player, i: u16, v: f32| {
        let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1 + i);
        p.fields
            .set_u32(UNIT_FIELD_POWER1 + i, (v.max(0.0) as u32).min(max));
    };
    match *what {
        "hp" | "health" => {
            let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
            p.fields
                .set_u32(UNIT_FIELD_HEALTH, (value.max(1.0) as u32).min(max));
        }
        "mana" => set_power(p, 0, value),
        // Rage is kept in tenths.
        "rage" => set_power(p, 1, value * 10.0),
        "energy" => set_power(p, 3, value),
        "money" => {
            let money = (i64::from(p.money()) + value as i64).clamp(0, i64::from(u32::MAX));
            p.set_money(money as u32);
        }
        "speed" => {
            if !(0.1..=10.0).contains(&value) {
                return Some("Speed goes from 0.1 to 10.".into());
            }
            p.speed_rate = value;
            crate::game::map::derive::unit(map, to);
        }
        other => return Some(format!("Can't modify {other}.")),
    }
    None
}

fn rested(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let to = player_target(map, g);
    let p = map.players.get_mut(&to)?;
    let amount =
        float(args.first()).unwrap_or_else(|| p.fields.get_u32(PLAYER_NEXT_LEVEL_XP) as f32);
    crate::game::map::rest::add_bonus(p, amount);
    Some(format!("Rested XP: {:.0}.", p.row.state.rest_bonus))
}

/// The client's "Stuck" spell.
pub const STUCK: u32 = 7355;

/// `.start`: the Stuck spell cast, its ten seconds and all, as
/// mangos-classic's `HandleStartCommand` and Thistle Tea's `.start` have
/// it; not on a flight nor in a fight.
fn start(map: &mut Map, g: Guid) -> Option<String> {
    let p = map.players.get(&g)?;
    if p.flight.is_some() {
        return Some("You are in flight.".into());
    }
    if p.fields.has_flag(
        crate::game::fields::UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    ) {
        return Some("You are in combat.".into());
    }
    let at = crate::game::map::spells::Targets {
        unit: Some(g),
        ..Default::default()
    };
    crate::game::map::spells::cast(map, g, STUCK, at, None);
    None
}

const TALENTS_USAGE: &str = "Usage: .talents reset | .talents learn <talent>[:<ranks>] ...";

/// `.talents reset`: the selected player's (or the GM's) talents
/// unlearned, their points back. `.talents learn <talent>[:<ranks>] ...`:
/// each talent (`Talent.dbc`'s id) learned up to that many ranks (1 by
/// default), in order, a point a rank, as `CMSG_LEARN_TALENT` would learn
/// it: with the points, the tier open and what it needs learned; the rest
/// refused, and named.
fn talents(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let data = map.realm.data.clone();
    let to = player_target(map, g);
    match args {
        ["reset"] => {
            let spells = crate::game::map::misc::talent_spells(map.players.get(&to)?, &data);
            crate::game::map::misc::forget_spells(map, to, &spells);
            if let Some(p) = map.players.get_mut(&to) {
                crate::game::map::spells::refresh_talent_points(p, &data);
            }
            Some(format!("Unlearned {} talent spells.", spells.len()))
        }
        ["learn", words @ ..] if !words.is_empty() => {
            // A talent's ranks the player has.
            let has = |map: &Map, id: u32| {
                let (Some(t), Some(p)) = (data.spells.talents.get(&id), map.players.get(&to))
                else {
                    return 0;
                };
                t.ranks
                    .iter()
                    .rposition(|r| *r != 0 && p.spells.contains(r))
                    .map_or(0, |i| i as u32 + 1)
            };
            let (mut learned, mut refused) = (0u32, Vec::new());
            for w in words {
                let (id, ranks) = match w.split_once(':') {
                    Some((id, r)) => (id.parse::<u32>().ok(), r.parse::<u32>().ok()),
                    None => (w.parse::<u32>().ok(), Some(1)),
                };
                let (Some(id), Some(ranks @ 1..=5)) = (id, ranks) else {
                    return Some(TALENTS_USAGE.into());
                };
                while has(map, id) < ranks {
                    let before = has(map, id);
                    crate::game::map::spells::learn_talent(map, to, id, before);
                    if has(map, id) == before {
                        refused.push(format!("{id}:{}", before + 1));
                        break;
                    }
                    learned += 1;
                }
            }
            let left = map.players.get(&to).map_or(0, |p| {
                crate::game::map::spells::free_talent_points(p, &data)
            });
            Some(if refused.is_empty() {
                format!("Talents: learned {learned} ranks, {left} points left.")
            } else {
                format!(
                    "Talents: learned {learned} ranks, {left} points left; refused {}.",
                    refused.join(" ")
                )
            })
        }
        _ => Some(TALENTS_USAGE.into()),
    }
}

fn tgm(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let p = map.players.get_mut(&g)?;
    let on = match args.first() {
        Some(&"on") => true,
        Some(&"off") => false,
        _ => !p.misc.god,
    };
    p.misc.god = on;
    Some(format!("God mode {}.", if on { "on" } else { "off" }))
}

/// `.god [on|off]`: a load test's god mode (mangos-classic has none): the
/// player takes damage as usual, but their health never goes below 1.
fn god(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let p = map.players.get_mut(&g)?;
    let on = match args.first() {
        Some(a) if a.eq_ignore_ascii_case("on") => true,
        Some(a) if a.eq_ignore_ascii_case("off") => false,
        Some(_) => return Some("Usage: .god [on|off]".into()),
        None => !p.misc.undying,
    };
    p.misc.undying = on;
    Some(format!(
        "Undying {}: blows land, but health stays at 1 or more.",
        if on { "on" } else { "off" }
    ))
}

/// `.durability [off|on]`: a load test's unbreakable gear (mangos-classic
/// has none). Off mends everything the GM owns, free, broken gear giving
/// what it gives again, and nothing wears from then on: not death, blows,
/// a spirit healer, a thrown weapon or a spell. On wears as usual again,
/// mending nothing. Kept on the character; no argument says which it is.
fn durability(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let data = map.realm.data.clone();
    let p = map.players.get_mut(&g)?;
    let off = match args {
        [] => {
            return Some(format!(
                "Durability is {}.",
                if p.row.state.no_wear {
                    "off: your gear doesn't wear"
                } else {
                    "on: your gear wears as usual"
                }
            ));
        }
        [a] if a.eq_ignore_ascii_case("off") => true,
        [a] if a.eq_ignore_ascii_case("on") => false,
        _ => return Some("Usage: .durability [off|on]".into()),
    };
    p.row.state.no_wear = off;
    if !off {
        return Some("Durability on: your gear wears as usual.".into());
    }
    let mended = crate::game::map::vendor::mend_everything(p, &data);
    Some(format!(
        "Durability off: your gear doesn't wear ({mended} items mended)."
    ))
}

/// Every cooldown of `to` cleared, its client told: its spells' and
/// items' (a potion's, a healthstone's), their categories', the global
/// cooldowns, those waiting for an aura's end, and an interrupt's school
/// lockouts (Thistle Tea's `.debug cooldowns`). How many spells were
/// cooling.
pub fn clear_cooldowns(map: &mut Map, to: Guid) -> usize {
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&to) else {
        return 0;
    };
    let spells: Vec<u32> = p.caster.cooldowns.keys().copied().collect();
    // And each known spell of a category cooling, its client told of it at
    // the cast (Frost Nova's ranks, a wait of the category's alone), and
    // each waiting for its aura to go.
    let categories = &p.caster.category_cooldowns;
    let of_categories: Vec<u32> = p
        .spells
        .iter()
        .copied()
        .filter(|id| {
            !p.caster.cooldowns.contains_key(id)
                && data
                    .spells
                    .get(*id)
                    .is_some_and(|t| t.category != 0 && categories.contains_key(&t.category))
        })
        .chain(
            p.caster
                .pending_cooldowns
                .iter()
                .copied()
                .filter(|id| !p.caster.cooldowns.contains_key(id)),
        )
        .collect();
    p.caster.cooldowns.clear();
    p.caster.category_cooldowns.clear();
    p.caster.pending_cooldowns.clear();
    p.caster.pending_times.clear();
    p.caster.cooldown_sources.clear();
    p.caster.gcd_until = None;
    p.caster.other_gcds.clear();
    p.caster.school_lockout.clear();
    for id in spells.iter().chain(&of_categories) {
        p.outbound.send(&SMSG_CLEAR_COOLDOWN {
            id: *id,
            target: to,
        });
    }
    spells.len()
}

/// `.infinitemana [on|off]` (`.thereisnospoon`): a GM's spells cost no
/// mana, and on fills it (mangos-classic has none; later cores'
/// `.cheat power`). Kept on the character; no argument turns it over.
fn infinite_mana(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let p = map.players.get_mut(&g)?;
    let on = match args {
        [] => !p.row.state.infinite_mana,
        [a] if a.eq_ignore_ascii_case("on") => true,
        [a] if a.eq_ignore_ascii_case("off") => false,
        _ => return Some("Usage: .infinitemana [on|off]".into()),
    };
    p.row.state.infinite_mana = on;
    if on {
        let max = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
        p.fields.set_u32(UNIT_FIELD_POWER1, max);
    }
    Some(if on {
        "Infinite mana on: there is no spoon.".into()
    } else {
        "Infinite mana off.".into()
    })
}

/// Whether `g`'s spells take no mana of it (`.infinitemana`).
pub fn infinite_mana_of(map: &Map, g: Guid) -> bool {
    map.players
        .get(&g)
        .is_some_and(|p| p.row.state.infinite_mana)
}

/// Whether a player is undying (`.god`): hurt, never below 1 health.
pub fn undying(map: &Map, g: Guid) -> bool {
    map.players.get(&g).is_some_and(|p| p.misc.undying)
}

/// Whether a unit takes no damage: a GM in god mode.
pub fn invulnerable(map: &Map, g: Guid) -> bool {
    map.players.get(&g).is_some_and(|p| p.misc.god)
}

fn weather(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let zone = map.players.get(&g)?.zone;
    let realm = map.realm.clone();
    let kind = match args.first().copied() {
        Some("fine") => 0,
        Some("rain") => 1,
        Some("snow") => 2,
        Some("storm") => 3,
        Some("auto") => {
            crate::game::weather::release(&realm, zone);
            return Some("The zone's weather follows the season again.".into());
        }
        _ => return Some("Usage: .weather fine|rain|snow|storm [0..1] [permanent]|auto".into()),
    };
    let grade = float(args.get(1)).unwrap_or(0.5);
    // A grade the client may be told, or none (Thistle Tea's `Weather.set/2`).
    if !(0.0..=1.0).contains(&grade) {
        return Some("The weather's grade is from 0 to 1.".into());
    }
    let permanent = args.contains(&"permanent");
    crate::game::weather::set(&realm, zone, kind, grade, permanent);
    None
}

fn instance(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let realm = map.realm.clone();
    match args {
        [] | ["info"] => {
            let me = guid::counter(g);
            let binds: Vec<String> = realm
                .social
                .instances
                .lock()
                .raids
                .values()
                .filter(|r| r.players.contains(&me))
                .map(|r| format!("map {} copy {}", r.map, r.id))
                .collect();
            Some(format!(
                "In {}; raid binds: {}.",
                map.key,
                if binds.is_empty() {
                    "none".to_owned()
                } else {
                    binds.join(", ")
                }
            ))
        }
        ["reset"] => {
            crate::game::instances::reset(map, g);
            None
        }
        ["switch", id] => {
            let Ok(id) = id.parse::<u32>() else {
                return Some("Usage: .instance switch <id>".into());
            };
            // Only into a copy of this map there is (Thistle Tea's
            // `join_copy/3`).
            let known = realm
                .social
                .instances
                .lock()
                .saves
                .get(&id)
                .is_some_and(|s| s.map == map.key.map);
            if !known {
                return Some(format!("No copy {id} of map {}.", map.key.map));
            }
            let p = map.players.get(&g)?;
            let (at, o) = (p.position(), p.movement.orientation);
            let key = MapKey {
                map: map.key.map,
                instance: id,
            };
            map.teleport(g, key, at, o);
            None
        }
        ["raid-reset", m] => {
            let Ok(m) = m.parse::<u32>() else {
                return Some("Usage: .instance raid-reset <map>".into());
            };
            let gone = crate::game::instances::reset_raid(&realm, m);
            Some(format!("{gone} raid copies of map {m} reset."))
        }
        // The copy's script's data, to read (Thistle Tea's `.instance
        // data [field]`): every field set, or the one asked for.
        ["data"] | ["data", _] => {
            if map.key.instance == 0 {
                return Some("Current world is not an instance copy.".into());
            }
            let script = map
                .instance
                .script
                .map_or_else(|| "none".to_owned(), |s| format!("{s:?}"));
            let fields: Vec<String> = match args.get(1) {
                Some(f) => {
                    let Ok(f) = f.parse::<u32>() else {
                        return Some("Usage: .instance data [field]".into());
                    };
                    vec![format!(
                        "{f}={}",
                        crate::game::map::instance_script::get_data(map, f)
                    )]
                }
                None => map
                    .instance
                    .data
                    .iter()
                    .map(|(f, v)| format!("{f}={v}"))
                    .collect(),
            };
            Some(format!(
                "Instance data ({script}): {}",
                if fields.is_empty() {
                    "none".to_owned()
                } else {
                    fields.join(", ")
                }
            ))
        }
        _ => Some("Usage: .instance info|reset|switch <id>|raid-reset <map>|data [field]".into()),
    }
}

fn battleground(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    use crate::game::battlegrounds;
    let realm = map.realm.clone();
    let bg_of = |name: &str| match name {
        "alterac" | "av" => Some(1),
        "warsong" | "wsg" => Some(2),
        "arathi" | "ab" => Some(3),
        _ => None,
    };
    match args {
        ["join"] | ["join", _] => {
            let bg = args.get(1).map_or(Some(2), |n| bg_of(n))?;
            let p = map.players.get(&g)?;
            let team = p.team;
            // A match of one: invited at once, whoever else is waiting.
            let id = realm.social.instances.lock().new_id();
            {
                let mut bgs = realm.social.battlegrounds.lock();
                let mut m = battlegrounds::Match {
                    id,
                    bg,
                    bracket: battlegrounds::bracket_for(bg, p.level()),
                    invited: Default::default(),
                    members: Default::default(),
                    offline: Default::default(),
                    started: std::time::Instant::now(),
                    ended: false,
                    began: None,
                    ended_at: None,
                };
                m.invited
                    .insert(g, (team, std::time::Instant::now() + map.cfg.bg_invite_for));
                bgs.matches.insert(id, m);
            }
            let frame = battlegrounds::status_frame(
                battlegrounds::map_of(bg),
                id,
                2,
                map.cfg.bg_invite_for.as_millis() as u32,
                0,
            );
            p.outbound.send_frame(frame);
            Some("Invited to a battleground of one. Click Enter Battle.".into())
        }
        ["start"] => {
            let now = map.now;
            let Some(m) = map.battleground.as_mut() else {
                return Some("Not in a battleground.".into());
            };
            if matches!(
                m.phase,
                crate::game::map::battleground::Phase::Waiting
                    | crate::game::map::battleground::Phase::Starting { .. }
            ) {
                m.phase = crate::game::map::battleground::Phase::Starting { at: now };
                Some("The battle begins.".into())
            } else {
                Some("The battle is already under way.".into())
            }
        }
        ["info"] => {
            let in_match = map
                .battleground
                .as_ref()
                .map(|m| format!("{:?}, score {}-{}", m.phase, m.score[0], m.score[1]));
            let queued = realm
                .social
                .battlegrounds
                .lock()
                .match_of(g)
                .map(|m| format!("match {} of battleground {}", m.id, m.bg));
            Some(format!(
                "Battleground: {}.",
                in_match.or(queued).unwrap_or_else(|| "none".into())
            ))
        }
        ["leave"] => {
            if map.battleground.is_some() {
                battlegrounds::leave(map, g);
            } else {
                battlegrounds::logged_out(&realm, g);
                if let Some(p) = map.players.get(&g) {
                    p.outbound
                        .send_frame(battlegrounds::status_frame(0, 0, 0, 0, 0));
                }
            }
            Some("Left the battleground.".into())
        }
        _ => Some("Usage: .bg join <warsong|arathi|alterac>|start|info|leave".into()),
    }
}

/// `.mail <name> <text>`: a letter, there at once.
fn mail(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let (Some(name), true) = (args.first(), args.len() > 1) else {
        return Some("Usage: .mail <recipient> <message>".into());
    };
    let realm = map.realm.clone();
    let Some(to) = realm.registry.guid_by_name(name) else {
        return Some(format!("No character {name}."));
    };
    let now = crate::game::social::mail::now();
    crate::game::social::mail::post(
        &realm,
        crate::game::social::mail::Mail {
            kind: crate::game::social::mail::NORMAL,
            sender: guid::counter(g),
            receiver: guid::counter(to),
            subject: "A letter from a GM".into(),
            body: args[1..].join(" "),
            deliver_at: now,
            expires_at: now + 30 * crate::game::social::mail::DAY,
            ..Default::default()
        },
    );
    Some(format!("Mailed {name}."))
}

fn debug(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    let data = map.realm.data.clone();
    match args {
        ["cooldowns"] => {
            let to = player_target(map, g);
            let cleared = clear_cooldowns(map, to);
            Some(format!("Cleared {cleared} cooldowns."))
        }
        ["durability", percent, ..] => {
            // A percent of 0 to 100; anything else is refused (Thistle
            // Tea's `.debug durability`).
            let Some(percent) = percent
                .parse::<f32>()
                .ok()
                .filter(|p| (0.0..=100.0).contains(p))
            else {
                return Some("Usage: .debug durability <percent, 0 to 100>".into());
            };
            let to = player_target(map, g);
            if !map
                .players
                .get(&to)
                .is_some_and(crate::game::map::vendor::wears)
            {
                return Some("That gear doesn't wear (.durability on first).".into());
            }
            crate::game::map::vendor::wear(map, to, percent / 100.0);
            Some(format!("Wore gear down {percent}%."))
        }
        ["events"] => {
            let mut on: Vec<String> = map
                .realm
                .social
                .events
                .lock()
                .iter()
                .map(|e| {
                    let name = data
                        .events
                        .get(e)
                        .map(|x| x.name.clone())
                        .unwrap_or_default();
                    format!("{e} {name}")
                })
                .collect();
            on.sort();
            Some(format!(
                "Events on: {}",
                if on.is_empty() {
                    "none".into()
                } else {
                    on.join(", ")
                }
            ))
        }
        ["events", action @ ("start" | "stop"), id] => event(map, &[*action, *id]),
        ["explore"] => {
            let to = player_target(map, g);
            let p = map.players.get_mut(&to)?;
            p.row.state.explored_zones = vec![u32::MAX; crate::game::map::exploration::WORDS];
            crate::game::map::exploration::init(p);
            Some("Every area explored.".into())
        }
        ["taxi"] => {
            let to = player_target(map, g);
            let p = map.players.get_mut(&to)?;
            // Every node a flight master can teach, not those only a
            // spell flies from (Thistle Tea's `Taxi.unlock_all`).
            for node in data.npc.taxi_nodes.keys() {
                if !data.npc.spell_only_nodes.contains(node) {
                    crate::game::map::npc::learn_node(&mut p.row.state.taxi_mask, *node);
                }
            }
            Some("Every flight path known.".into())
        }
        ["position"] => {
            let t = selected(map, g);
            let t = if t.is_zero() { g } else { t };
            Some(position_report(map, t)?)
        }
        // By guid, in decimal or 0x hex (Thistle Tea's `.debug position
        // <guid>`): one on this map, or none.
        ["position", id] => {
            let raw = id.strip_prefix("0x").map_or_else(
                || id.parse::<u64>().ok(),
                |hex| u64::from_str_radix(hex, 16).ok(),
            );
            let Some(raw) = raw else {
                return Some("Usage: .debug position [guid]".into());
            };
            Some(
                position_report(map, Guid::new(raw))
                    .unwrap_or_else(|| format!("Entity {id} is inactive or missing.")),
            )
        }
        ["professions"] => {
            let to = player_target(map, g);
            let p = map.players.get_mut(&to)?;
            let mut n = 0;
            let known: Vec<u32> = crate::game::map::skills::PROFESSIONS
                .iter()
                .copied()
                .filter(|s| crate::game::map::skills::value(p, *s).is_some())
                .collect();
            for skill in known {
                crate::game::map::skills::learn_skill(p, skill, 4);
                let have = crate::game::map::skills::value(p, skill).unwrap_or(0);
                crate::game::map::skills::raise(p, skill, 300u32.saturating_sub(have));
                n += 1;
            }
            Some(format!("{n} professions at 300."))
        }
        ["skills"] => {
            let to = player_target(map, g);
            let p = map.players.get_mut(&to)?;
            let skills = crate::game::map::skills::known(p);
            for skill in &skills {
                let (have, max) = (
                    crate::game::map::skills::value(p, *skill).unwrap_or(0),
                    crate::game::map::skills::max_value(p, *skill).unwrap_or(0),
                );
                crate::game::map::skills::raise(p, *skill, max.saturating_sub(have));
            }
            Some(format!("{} skills at their caps.", skills.len()))
        }
        ["skill", id, value] => {
            let (Ok(id), Ok(value)) = (id.parse::<u32>(), value.parse::<u32>()) else {
                return Some("Usage: .debug skill <id> <value>".into());
            };
            let to = player_target(map, g);
            let p = map.players.get_mut(&to)?;
            let Some(have) = crate::game::map::skills::value(p, id) else {
                return Some(format!("No skill {id}."));
            };
            // From 1 to its cap; past the cap it goes to the cap, and says
            // so (mangos's `.setskill` refuses, with a reply that read as
            // done); its stats follow (Defense's dodge and parry).
            let max = crate::game::map::skills::max_value(p, id).unwrap_or(have);
            if value == 0 {
                return Some(format!("Skill {id} goes from 1 to {max}."));
            }
            let at = value.min(max);
            crate::game::map::skills::set_value(p, id, at);
            p.apply_stats(&data);
            Some(if at < value {
                format!(
                    "Skill {id} at {at}, its cap: {value} is past it (learn the skill's next rank to raise the cap)."
                )
            } else {
                format!("Skill {id} at {at}.")
            })
        }
        ["spells"] => {
            let to = player_target(map, g);
            let spells = trainer_spells(map, to);
            let n = spells.len();
            for s in spells {
                crate::game::map::spells::learn(map, to, s);
            }
            Some(format!("Learned {n} trainer spells."))
        }
        ["reputation", rest @ ..] => reputation(map, g, rest),
        ["transport"] | ["transport", "list"] => {
            let routes = crate::game::transports::routes(&map.realm);
            let here: Vec<String> = routes
                .iter()
                .filter(|r| r.maps().contains(&map.key.map))
                .map(|r| format!("{} ({} s)", r.entry, r.period / 1000))
                .collect();
            Some(format!(
                "Transports here: {}",
                if here.is_empty() {
                    "none".into()
                } else {
                    here.join(", ")
                }
            ))
        }
        ["random", "equipment"] => random_equipment(map, g),
        _ => Some(format!("Usage: {HELP}")),
    }
}

/// The class spells quests give, with the level each is had at (Thistle
/// Tea's `ClassSpell` `@quest_reward_spells`): the warrior's stances, the
/// hunter's pet spells, the shaman's totems, the warlock's demons and the
/// druid's forms.
const QUEST_CLASS_SPELLS: &[(u8, &[(u32, u8)])] = &[
    (1, &[(71, 10), (2458, 30)]),
    (
        3,
        &[
            (1515, 10),
            (883, 10),
            (2641, 10),
            (6991, 10),
            (982, 10),
            (136, 12),
        ],
    ),
    (7, &[(8071, 4), (3599, 10), (5394, 20), (8512, 30)]),
    (
        9,
        &[
            (688, 1),
            (697, 10),
            (712, 20),
            (691, 30),
            (1122, 50),
            (18_540, 60),
        ],
    ),
    (11, &[(5487, 10), (1066, 16), (768, 20)]),
];

/// The spells a player's class trainers teach up to their level, and those
/// the class's quests give, as Thistle Tea's `ClassSpell.trainable_spell_ids`:
/// a talent's rank only to one who has a rank of it.
fn trainer_spells(map: &Map, g: Guid) -> Vec<u32> {
    let data = &map.realm.data;
    let Some(p) = map.players.get(&g) else {
        return Vec::new();
    };
    let level = p.level();
    let class = p.row.class;
    let quests = QUEST_CLASS_SPELLS
        .iter()
        .filter(|(c, _)| *c == class)
        .flat_map(|(_, spells)| spells.iter())
        .filter(|(_, at)| *at <= level)
        .map(|(spell, _)| *spell);
    let talent_of = |spell: u32| data.spells.talent_of(spell);
    let known_talents: Vec<u32> = p.spells.iter().filter_map(|s| talent_of(*s)).collect();
    let mut out: Vec<u32> = data
        .creature_templates
        .values()
        .filter(|t| t.trainer_class == class && t.trainer_type == 0)
        .flat_map(|t| {
            data.npc
                .trainers
                .get(&t.entry)
                .into_iter()
                .chain(data.npc.trainer_templates.get(&t.trainer_id))
                .flatten()
        })
        .filter(|s| s.required_level <= level && s.required_skill == 0)
        .map(|s| crate::game::map::spells::learns(data, s.spell))
        .chain(quests)
        .filter(|s| talent_of(*s).is_none_or(|t| known_talents.contains(&t)))
        .filter(|s| !p.spells.contains(s))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn reputation(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    use crate::game::map::reputation as rep;
    let data = map.realm.data.clone();
    let to = player_target(map, g);
    let show = |map: &Map, f: u32| -> String {
        let p = &map.players[&to];
        let flags = p
            .row
            .state
            .reputation
            .iter()
            .find(|s| s.faction == f)
            .map_or(0, |s| s.flags);
        let name = data
            .misc
            .faction(f)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        format!(
            "{f} {name}: {} (rank {}), flags 0x{flags:X}",
            rep::standing(p, &data, f),
            rep::rank(p, &data, f)
        )
    };
    match args {
        [f] => {
            let f = f.parse::<u32>().ok()?;
            Some(show(map, f))
        }
        ["add", f, n] => {
            let (f, n) = (f.parse::<u32>().ok()?, n.parse::<i32>().ok()?);
            rep::modify(map, to, f, n, false);
            Some(show(map, f))
        }
        ["set", f, n] => {
            let (f, n) = (f.parse::<u32>().ok()?, n.parse::<i32>().ok()?);
            let have = rep::standing(map.players.get(&to)?, &data, f);
            rep::modify(map, to, f, n - have, false);
            Some(show(map, f))
        }
        ["war", f, on] => {
            let f = f.parse::<u32>().ok()?;
            let index = data.misc.faction(f)?.index;
            rep::set_at_war(map, to, index, *on == "on");
            Some(show(map, f))
        }
        ["find", name @ ..] => {
            let wanted = name.join(" ").to_lowercase();
            let mut found: Vec<String> = data
                .misc
                .factions
                .values()
                .filter(|d| d.name.to_lowercase().contains(&wanted))
                .map(|d| format!("{} {}", d.id, d.name))
                .collect();
            found.sort();
            found.truncate(10);
            Some(if found.is_empty() { "No such faction.".into() } else { found.join(", ") })
        }
        _ => Some("Usage: .debug reputation <faction>|add|set <faction> <n>|war <faction> on|off|find <name>".into()),
    }
}

/// `.debug random equipment`: a random piece the player may wear in each
/// empty equipment slot, of their level or below.
fn random_equipment(map: &mut Map, g: Guid) -> Option<String> {
    use crate::game::map::inventory::{self, slots_for};
    use crate::game::store::BAG_PLAYER;
    let data = map.realm.data.clone();
    let p = map.players.get(&g)?;
    let level = u32::from(p.level());
    let mut rng = rand::rng();
    let mut picks = Vec::new();
    let prof = inventory::Proficiency::of(p, &data);
    for slot in 0..19u8 {
        if p.item_at(BAG_PLAYER, slot).is_some() || matches!(slot, 3 | 18) {
            continue;
        }
        let fits: Vec<u32> = data
            .items
            .values()
            .filter(|t| t.required_level <= level && t.quality <= 4 && t.quality >= 1)
            .filter(|t| slots_for(t.inventory_type).first() == Some(&slot))
            .filter(|t| inventory::can_equip(p, &prof, t, slot).is_ok())
            .map(|t| t.entry)
            .collect();
        if !fits.is_empty() {
            picks.push((slot, fits[rng.random_range(0..fits.len())]));
        }
    }
    let mut n = 0;
    for (slot, entry) in picks {
        if inventory::add_item(map, g, entry, 1, true).is_err() {
            continue;
        }
        let from = map
            .players
            .get(&g)
            .and_then(|p| {
                p.items
                    .iter()
                    .rev()
                    .find(|i| i.entry == entry && !(i.bag == BAG_PLAYER && i.slot < 19))
            })
            .map(|i| (i.bag, i.slot));
        if let Some(from) = from {
            inventory::move_item(map, g, from, (BAG_PLAYER, slot));
            n += 1;
        }
    }
    Some(format!("Equipped {n} random pieces."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_takes_a_quality_or_raid_and_a_tier() {
        let pick = |args: &[&str]| gear_args(args).map(|(p, _)| p);
        assert_eq!(pick(&[]), Ok(Pick::Random { quality: None }));
        assert_eq!(
            pick(&["green"]),
            Ok(Pick::Random {
                quality: Some(UNCOMMON)
            })
        );
        assert_eq!(
            pick(&["Blue"]),
            Ok(Pick::Random {
                quality: Some(RARE)
            })
        );
        assert_eq!(pick(&["purple"]), Err(GEAR_USAGE));
        assert_eq!(pick(&["raid"]), Ok(Pick::Raid { tier: None }));
        assert_eq!(pick(&["RAID", "2"]), Ok(Pick::Raid { tier: Some(2) }));
        assert_eq!(pick(&["raid", "3"]), Err(GEAR_USAGE));
        assert_eq!(pick(&["epic"]), Err(GEAR_USAGE));
    }

    /// After the quality, in any order: a role, a way of holding and a
    /// kind of weapon, one of each.
    #[test]
    fn gear_takes_a_role_a_way_of_holding_and_a_weapon() {
        let wants = |args: &[&str]| gear_args(args).map(|(_, w)| w);
        assert_eq!(
            gear_args(&["raid", "1", "tank"]),
            Ok((
                Pick::Raid { tier: Some(1) },
                Wants {
                    role: Some(Role::Tank),
                    ..Wants::default()
                }
            ))
        );
        assert_eq!(
            wants(&["Sword", "dual", "dps"]),
            Ok(Wants {
                role: Some(Role::Damage),
                hands: Some(Hands::DualWield),
                weapon: WeaponKind::named("sword"),
            })
        );
        for (word, hands) in [
            ("shield", Hands::Shield),
            ("2h", Hands::TwoHanded),
            ("offhand", Hands::HeldInOffHand),
        ] {
            assert_eq!(wants(&["green", word]).map(|w| w.hands), Ok(Some(hands)));
        }
        for word in ["healer", "heal", "caster"] {
            assert_eq!(wants(&[word]).map(|w| w.role), Ok(Some(Role::Healer)));
        }
        for args in [
            &["sword", "dagger"][..],
            &["tank", "dps"],
            &["shield", "2h"],
            &["juggle"],
        ] {
            assert_eq!(gear_args(args), Err(GEAR_USAGE), "{args:?}");
        }
    }

    #[test]
    fn the_developers_items_are_no_raiders_gear() {
        for name in [
            "TEST Level 80 Epic",
            "Ring of Critical Testing 2",
            "Naxxramas Sword 1H 1 [PH]",
            "Blackwing Mace1H[PH]",
            "Mandokir's Sting DEPRECATED",
            "Blessed Qiraji Naturalist Staff UNUSED",
            "Monster - Staff, Lord Valthalak",
            "QATest +1000 Spell Dmg Ring",
            "90 Epic Warrior Ring",
            "63 Green Frost Staff",
        ] {
            assert!(unreal(name), "{name}");
        }
        for name in [
            "Kingsfall",
            "Testament of Hope",
            "Phase Blade",
            "Monstrous Glaive",
        ] {
            assert!(!unreal(name), "{name}");
        }
    }

    #[test]
    fn the_misspelt_deprecated_items_are_no_ones() {
        assert!(unreal("Depricated Razor Arrow"));
        assert!(!unreal("Razor Arrow"));
    }

    #[test]
    fn counts_are_grouped_by_thousands() {
        let got: Vec<String> = [0, 7, 200, 999, 1000, 3600, 12_345, 1_000_000]
            .into_iter()
            .map(grouped)
            .collect();
        assert_eq!(
            got,
            [
                "0",
                "7",
                "200",
                "999",
                "1,000",
                "3,600",
                "12,345",
                "1,000,000"
            ]
        );
    }

    /// The best quiver or pouch for a level, without the real data: the
    /// most slots of the kind the weapon shoots, of the level or below
    /// (one with no required level by its item level less five), rare at
    /// most but for `.gear raid`, and never a developer's.
    #[test]
    fn the_best_quiver_has_the_most_slots_of_the_level_and_kind() {
        let mut data = crate::game::data::GameData::empty();
        let bag =
            |entry: u32, name: &str, sub: u32, quality: u32, level: u32, item: u32, slots: u32| {
                crate::game::data::ItemTemplate {
                    entry,
                    class: ITEM_QUIVER,
                    subclass: sub,
                    name: name.into(),
                    quality,
                    inventory_type: 18,
                    allowable_class: -1,
                    allowable_race: -1,
                    required_level: level,
                    item_level: item,
                    container_slots: slots,
                    bag_family: sub - 1,
                    ..Default::default()
                }
            };
        for t in [
            bag(1, "Light Quiver", 2, 1, 1, 1, 6),
            bag(2, "Medium Quiver", 2, 1, 10, 15, 10),
            bag(3, "Night Watch Quiver", 2, 2, 0, 25, 12),
            bag(4, "Heavy Quiver", 2, 2, 30, 35, 14),
            bag(5, "Big Test Quiver", 2, 2, 1, 1, 30),
            bag(6, "Lamina", 2, 4, 60, 75, 18),
            bag(7, "Harpy Hide Quiver", 2, 3, 55, 60, 16),
            bag(8, "Small Ammo Pouch", 3, 1, 1, 1, 6),
            bag(9, "Medium Shot Pouch", 3, 1, 10, 15, 10),
        ] {
            data.items.insert(t.entry, t);
        }
        let random = Pick::Random { quality: None };
        let raid = Pick::Raid { tier: None };
        let best = |level: u8, ammo: u32, pick: Pick| {
            best_quiver(&data, (3, 3, level), ammo, pick).map(|t| t.entry)
        };
        assert_eq!(best(1, 2, random), Some(1));
        assert_eq!(best(19, 2, random), Some(2), "the Night Watch's at 20");
        assert_eq!(best(20, 2, random), Some(3));
        assert_eq!(best(40, 2, random), Some(4));
        assert_eq!(best(60, 2, random), Some(7), "rare at most");
        assert_eq!(best(60, 2, raid), Some(6), "an epic for a raider");
        assert_eq!(best(59, 2, raid), Some(7));
        assert_eq!(best(40, 3, random), Some(9), "a pouch for a gun");
        assert_eq!(best(40, 4, random), None, "no bag for thrown");
        assert_eq!(
            best_quiver(&data, (3, 3, 40), 2, random).map(|t| t.name.as_str()),
            Some("Heavy Quiver")
        );
    }

    /// Every class has its two sets, and they are the classes' own:
    /// eighteen sets, none twice.
    #[test]
    fn each_class_has_its_tier_one_and_two() {
        let mut sets: Vec<u32> = [1, 2, 3, 4, 5, 7, 8, 9, 11]
            .into_iter()
            .flat_map(|c| {
                let (one, two) = tier_sets(c).expect("a class's sets");
                assert!((201..=209).contains(&one) && (210..=218).contains(&two));
                [one, two]
            })
            .collect();
        sets.sort_unstable();
        sets.dedup();
        assert_eq!(sets.len(), 18);
        assert_eq!(tier_sets(6), None);
    }
}
