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
.character level <n>, .character spells, .gear, .damage <n>, .kill, .respawn, .move [x y z], .threat, .guid, .pos, \
.modify hp|mana|rage|energy|money|speed <n>, .rested [n], .start, .talents reset, .tgm [on|off], \
.weather fine|rain|snow|storm [0..1] [permanent]|auto, .instance info|reset|switch <id>|raid-reset <map>, \
.bg join <warsong|arathi|alterac>|start|info|leave, .mail <name> <text>, .debug cooldowns|durability <percent>|\
events [start|stop <id>]|explore|position|professions|reputation [<faction>|add|set <faction> <n>|war <faction> on|off|find <name>]|\
skill <id> <value>|skills|spells|taxi|transport|random equipment, .event list [all]|start <id>|stop <id>|<id>";

/// The level a command here needs, if it is one of these: looking is for
/// moderators, the rest for game masters.
pub fn needed_level(name: &str) -> Option<u8> {
    use crate::accounts::{GAME_MASTER, MODERATOR};
    Some(match name {
        "guid" | "pos" | "threat" => MODERATOR,
        "additem" | "addquest" | "learn" | "unlearn" | "character" | "gear" | "damage" | "kill"
        | "respawn" | "move" | "modify" | "rested" | "start" | "talents" | "tgm" | "weather"
        | "instance" | "bg" | "battleground" | "mail" | "debug" | "invasion" | "event" => {
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
        "gear" => gear(map, g),
        "damage" => damage(map, g, args),
        "kill" => kill(map, g),
        "respawn" => respawn(map, g),
        "move" => move_target(map, g, args),
        "threat" => threat(map, g),
        "guid" => Some(format!("Target: {}", selected(map, g))),
        "pos" => pos(map, g),
        "modify" => modify(map, g, args),
        "rested" => rested(map, g, args),
        "start" => start(map, g),
        "talents" => talents(map, g, args),
        "tgm" => tgm(map, g, args),
        "weather" => weather(map, g, args),
        "instance" => instance(map, g, args),
        "bg" | "battleground" => battleground(map, g, args),
        "mail" => mail(map, g, args),
        "debug" => debug(map, g, args),
        "event" => event(map, args),
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
    Some(format!("Level {level}."))
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

/// The items `.gear` may give a character of `class`, `race` and `level`
/// of the inventory types `types` that `fits` takes: uncommon or rare, of
/// a required level within ten of theirs and no higher, asking for no
/// skill, spell, rank or standing, and not temporary.
fn gear_pool(
    data: &crate::game::data::GameData,
    (class, race, level): (u8, u8, u8),
    types: &[u32],
    fits: impl Fn(&crate::game::data::ItemTemplate) -> bool,
) -> Vec<u32> {
    use rand::seq::SliceRandom;
    let allows = |mask: i32, of: u8| mask == -1 || mask == 0 || mask & (1 << (of.max(1) - 1)) != 0;
    let level = u32::from(level);
    let mut out: Vec<u32> = data
        .items
        .values()
        .filter(|t| (2..=3).contains(&t.quality))
        .filter(|t| t.required_level <= level && t.required_level + 10 >= level)
        .filter(|t| types.contains(&t.inventory_type))
        .filter(|t| {
            t.required_skill == 0
                && t.required_spell == 0
                && t.required_honor_rank == 0
                && t.required_reputation_faction == 0
                && t.duration == 0
        })
        .filter(|t| allows(t.allowable_class, class) && allows(t.allowable_race, race))
        .filter(|t| fits(t))
        .map(|t| t.entry)
        .collect();
    out.sort_unstable();
    out.shuffle(&mut rand::rng());
    out
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
/// wand, with ammunition for one that takes it. The proficiencies and
/// weapon skills the items need are taught, the skills at their caps, and
/// Auto Shot and Shoot with a ranged weapon or a wand. One alive is left
/// at full health and mana, as the reply says.
fn gear(map: &mut Map, g: Guid) -> Option<String> {
    use crate::game::map::inventory::{self, Destination, SLOT_MAIN_HAND, SLOT_OFF_HAND};
    use crate::game::store::BAG_PLAYER;
    use rand::seq::SliceRandom;
    let data = map.realm.data.clone();
    let to = player_target(map, g);
    let p = map.players.get(&to)?;
    let who = (p.row.class, p.row.race, p.level());
    let (class, _, level) = who;
    let kind = armour_of(class, level);
    let armour = |t: &crate::game::data::ItemTemplate| t.class == ITEM_ARMOR && t.subclass == kind;
    let trinket = |t: &crate::game::data::ItemTemplate| t.class == ITEM_ARMOR;
    let one_hand = |t: &crate::game::data::ItemTemplate| {
        t.class == ITEM_WEAPON && weapon_for(&data, t.subclass, class)
    };
    let mut plan: Vec<(u8, Vec<u32>)> = vec![
        (0, gear_pool(&data, who, &[1], armour)),
        (1, gear_pool(&data, who, &[2], trinket)),
        (2, gear_pool(&data, who, &[3], armour)),
        (4, gear_pool(&data, who, &[5, 20], armour)),
        (5, gear_pool(&data, who, &[6], armour)),
        (6, gear_pool(&data, who, &[7], armour)),
        (7, gear_pool(&data, who, &[8], armour)),
        (8, gear_pool(&data, who, &[9], armour)),
        (9, gear_pool(&data, who, &[10], armour)),
        (10, gear_pool(&data, who, &[11], trinket)),
        (11, gear_pool(&data, who, &[11], trinket)),
        (12, gear_pool(&data, who, &[12], trinket)),
        (13, gear_pool(&data, who, &[12], trinket)),
        (
            14,
            gear_pool(&data, who, &[16], |t| {
                t.class == ITEM_ARMOR && t.subclass == 1
            }),
        ),
    ];
    // The hands: a way of holding the class has, at random, that the
    // items let them fill both of.
    let mut ways = hands_of(class, level);
    ways.shuffle(&mut rand::rng());
    let fill = |way: Hands| {
        let main = match way {
            Hands::TwoHanded => gear_pool(&data, who, &[17], one_hand),
            _ => gear_pool(&data, who, &[13, 21], one_hand),
        };
        let off = match way {
            Hands::TwoHanded => Vec::new(),
            Hands::Shield => gear_pool(&data, who, &[14], |t| {
                t.class == ITEM_ARMOR
                    && t.subclass == ARMOR_SHIELD
                    && class_gets(&data, 9116, class)
            }),
            Hands::DualWield => gear_pool(&data, who, &[13, 22], one_hand),
            Hands::HeldInOffHand => gear_pool(&data, who, &[23], trinket),
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
        gear_pool(&data, who, &[15, 25, 26], |t| {
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
    for (slot, picks) in plan {
        // A few tries: one may be unique and worn already, as the first
        // ring.
        for entry in picks.into_iter().take(20) {
            let Some(t) = data.item(entry) else {
                continue;
            };
            if let Some(spell) = proficiency_spell(t) {
                teach(map, to, spell);
            }
            if inventory::add_item_to(map, to, entry, 1, Destination::Worn(slot)).is_ok() {
                names.push(t.name.clone());
                if slot == 17 {
                    if t.subclass == WEAPON_WAND {
                        teach(map, to, SHOOT);
                    } else if class == 3 {
                        teach(map, to, AUTO_SHOT);
                    }
                    if t.ammo_type != 0 {
                        names.extend(give_ammo(map, to, t.ammo_type, level));
                    }
                }
                break;
            }
        }
    }
    if names.is_empty() {
        return Some("No gear to equip.".into());
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
        "Equipped {}.{}",
        names.join(", "),
        if whole {
            " At full health and mana."
        } else {
            ""
        }
    ))
}

/// Four stacks of the best ammunition of `ammo_type` (arrows, bullets) of
/// `level` or below, and that chosen for the ranged weapon: its name and
/// count, if any went.
fn give_ammo(map: &mut Map, g: Guid, ammo_type: u32, level: u8) -> Option<String> {
    let data = map.realm.data.clone();
    let ammo = data
        .items
        .values()
        .filter(|t| t.class == ITEM_PROJECTILE && t.subclass == ammo_type)
        .filter(|t| t.required_level <= u32::from(level) && t.quality <= 3)
        .filter(|t| t.required_reputation_faction == 0 && t.required_skill == 0)
        .max_by_key(|t| (t.required_level, t.item_level, std::cmp::Reverse(t.entry)))?;
    let p = map.players.get(&g)?;
    let want = ammo.stackable.max(1) * 4;
    let n = crate::game::map::inventory::fitting(p, &data, ammo.entry, want).ok()?;
    if n == 0 {
        return None;
    }
    crate::game::map::inventory::add_item(map, g, ammo.entry, n, true).ok()?;
    crate::game::map::ranged::set_ammo(map, g, ammo.entry);
    Some(format!("{n} {}", ammo.name))
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

/// `.start`: back to where the race and class begin, as mangos's does.
fn start(map: &mut Map, g: Guid) -> Option<String> {
    let p = map.players.get(&g)?;
    let s = map.realm.data.start_info(p.row.race, p.row.class)?;
    map.teleport(
        g,
        MapKey::world(s.map),
        Vector3d {
            x: s.x,
            y: s.y,
            z: s.z,
        },
        s.orientation,
    );
    None
}

fn talents(map: &mut Map, g: Guid, args: &[&str]) -> Option<String> {
    if args.first() != Some(&"reset") {
        return Some("Usage: .talents reset".into());
    }
    let data = map.realm.data.clone();
    let to = player_target(map, g);
    let spells = crate::game::map::misc::talent_spells(map.players.get(&to)?, &data);
    crate::game::map::misc::forget_spells(map, to, &spells);
    if let Some(p) = map.players.get_mut(&to) {
        crate::game::map::spells::refresh_talent_points(p, &data);
    }
    Some(format!("Unlearned {} talent spells.", spells.len()))
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
        _ => Some("Usage: .instance info|reset|switch <id>|raid-reset <map>".into()),
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
            let p = map.players.get_mut(&to)?;
            let spells: Vec<u32> = p.caster.cooldowns.keys().copied().collect();
            p.caster.cooldowns.clear();
            p.caster.category_cooldowns.clear();
            p.caster.gcd_until = None;
            p.caster.other_gcds.clear();
            for id in &spells {
                p.outbound.send(&SMSG_CLEAR_COOLDOWN {
                    id: *id,
                    target: to,
                });
            }
            Some(format!("Cleared {} cooldowns.", spells.len()))
        }
        ["durability", percent, ..] => {
            let Ok(percent) = percent.parse::<f32>() else {
                return Some("Usage: .debug durability <percent>".into());
            };
            let to = player_target(map, g);
            crate::game::map::vendor::wear(map, to, (percent / 100.0).clamp(0.0, 1.0));
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
            let top = data.npc.taxi_nodes.keys().copied().max().unwrap_or(0);
            p.row.state.taxi_mask = vec![u32::MAX; (top / 32 + 1) as usize];
            Some("Every flight path known.".into())
        }
        ["position"] => {
            let t = selected(map, g);
            let t = if t.is_zero() { g } else { t };
            let at = map.position_of(t)?;
            Some(format!(
                "{t}: {:.2}, {:.2}, {:.2} in {}",
                at.x, at.y, at.z, map.key
            ))
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
            let max = crate::game::map::skills::max_value(p, id).unwrap_or(have);
            crate::game::map::skills::set_value(p, id, value.min(max));
            Some(format!("Skill {id} at {}.", value.min(max)))
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
