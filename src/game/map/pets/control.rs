//! The master's side: the action bar a pet's spells and commands sit on
//! (`SMSG_PET_SPELLS`), and the opcodes that command it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage as C;
use wow_world_messages::vanilla::*;

use super::*;
use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;

/// A button: the action in the low three bytes, its kind in the top.
pub fn button(action: u32, kind: u32) -> u32 {
    (action & 0x00FF_FFFF) | (kind << 24)
}

pub fn unpack(b: u32) -> (u32, u32) {
    (b & 0x00FF_FFFF, b >> 24)
}

/// `AttributesEx`: the AI doesn't cast it of its own accord.
const ATTR_EX_NO_AUTOCAST_AI: u32 = 0x2_0000;

/// Whether a pet may cast `s` of its own accord.
pub fn autocastable(s: &SpellTemplate) -> bool {
    !s.passive() && s.attributes[1] & ATTR_EX_NO_AUTOCAST_AI == 0
}

/// A spell's button kind on the bar.
fn spell_kind(data: &GameData, spell: u32, autocast: &[u32]) -> u32 {
    match data.spells.get(spell) {
        Some(s) if autocastable(s) => {
            if autocast.contains(&spell) {
                ACT_ENABLED
            } else {
                ACT_DISABLED
            }
        }
        _ => ACT_PASSIVE,
    }
}

/// The bar a pet starts with: attack, follow and stay; four of its spells;
/// aggressive, defensive and passive. A possessed unit's: attack and its
/// spells.
pub fn default_bar(data: &GameData, spells: &[u32], autocast: &[u32], possess: bool) -> [u32; 10] {
    let mut active: Vec<u32> = spells
        .iter()
        .copied()
        .filter(|s| data.spells.get(*s).is_some_and(|t| !t.passive()))
        .collect();
    active.sort_unstable();
    let mut bar = [0u32; 10];
    if possess {
        bar[0] = button(u32::from(COMMAND_ATTACK), ACT_COMMAND);
        for (i, s) in active.iter().take(9).enumerate() {
            bar[i + 1] = button(*s, ACT_PASSIVE);
        }
        return bar;
    }
    bar[0] = button(u32::from(COMMAND_ATTACK), ACT_COMMAND);
    bar[1] = button(u32::from(COMMAND_FOLLOW), ACT_COMMAND);
    bar[2] = button(u32::from(COMMAND_STAY), ACT_COMMAND);
    for i in 0..4 {
        bar[3 + i] = match active.get(i) {
            Some(s) => button(*s, spell_kind(data, *s, autocast)),
            None => button(0, ACT_DISABLED),
        };
    }
    bar[7] = button(u32::from(REACT_AGGRESSIVE), ACT_REACTION);
    bar[8] = button(u32::from(REACT_DEFENSIVE), ACT_REACTION);
    bar[9] = button(u32::from(REACT_PASSIVE), ACT_REACTION);
    bar
}

/// Each spell button's kind again, after autocast changed.
fn repaint(data: &GameData, s: &mut PetState) {
    let autocast = s.autocast.clone();
    for b in s.bar.iter_mut() {
        let (action, kind) = unpack(*b);
        if matches!(kind, ACT_COMMAND | ACT_REACTION) || action == 0 {
            continue;
        }
        *b = button(action, spell_kind(data, action, &autocast));
    }
}

fn react_state(r: u8) -> PetReactState {
    match r {
        REACT_PASSIVE => PetReactState::Passive,
        REACT_AGGRESSIVE => PetReactState::Aggressive,
        _ => PetReactState::Defensive,
    }
}

fn command_state(c: u8) -> PetCommandState {
    match c {
        COMMAND_STAY => PetCommandState::Stay,
        COMMAND_ATTACK => PetCommandState::Attack,
        _ => PetCommandState::Follow,
    }
}

/// `SMSG_PET_SPELLS` for `owner`'s pet (or charmed unit), or an empty one.
pub fn send_spells(map: &mut Map, owner: Guid) {
    let data = map.realm.data.clone();
    // A possessed player: attack, and their spells.
    if let Some(v) = map
        .players
        .get(&owner)
        .and_then(|p| p.owned.charm)
        .filter(|c| {
            map.players
                .get(c)
                .and_then(|v| v.owned.possessed_by)
                .is_some_and(|pos| pos.master == owner)
        })
    {
        let msg = SMSG_PET_SPELLS {
            pet: v,
            action_bars: Some(SMSG_PET_SPELLS_action_bars {
                duration: 0,
                react: PetReactState::Passive,
                command: PetCommandState::Stay,
                unknown: 0,
                pet_enabled: PetEnabled::Disabled,
                action_bars: possess::bar(map, v),
                spells: Vec::new(),
                cooldowns: Vec::new(),
            }),
        };
        if let Some(p) = map.players.get(&owner) {
            p.outbound.send(&msg);
        }
        return;
    }
    let pet = pet_of(map, owner).or_else(|| {
        map.players
            .get(&owner)
            .and_then(|p| p.owned.charm)
            .filter(|c| map.creature(*c).is_some())
    });
    let msg = match pet.and_then(|g| state(map, g).map(|s| (g, s))) {
        Some((g, s)) if s.kind == Kind::Possessed => SMSG_PET_SPELLS {
            pet: g,
            action_bars: Some(SMSG_PET_SPELLS_action_bars {
                duration: 0,
                react: PetReactState::Passive,
                command: PetCommandState::Stay,
                unknown: 0,
                pet_enabled: PetEnabled::Disabled,
                action_bars: s.bar,
                spells: Vec::new(),
                cooldowns: Vec::new(),
            }),
        },
        Some((g, s)) => {
            let mut spells: Vec<u32> = s.spells.clone();
            spells.sort_unstable();
            let spells = spells
                .into_iter()
                .map(|id| button(id, spell_kind(&data, id, &s.autocast)))
                .collect();
            let cooldowns = map
                .caster(g)
                .map(|c| {
                    c.cooldowns
                        .iter()
                        .filter(|(_, t)| **t > map.now)
                        .map(|(spell, t)| PetSpellCooldown {
                            spell: (*spell).min(u32::from(u16::MAX)) as u16,
                            spell_category: 0,
                            cooldown: t.duration_since(map.now),
                            category_cooldown: std::time::Duration::ZERO,
                        })
                        .collect()
                })
                .unwrap_or_default();
            SMSG_PET_SPELLS {
                pet: g,
                action_bars: Some(SMSG_PET_SPELLS_action_bars {
                    duration: s
                        .expires
                        .map(|t| t.saturating_duration_since(map.now).as_millis() as u32)
                        .unwrap_or(0),
                    react: react_state(s.react),
                    command: command_state(s.command),
                    unknown: 0,
                    pet_enabled: PetEnabled::Disabled,
                    action_bars: s.bar,
                    spells,
                    cooldowns,
                }),
            }
        }
        None => SMSG_PET_SPELLS {
            pet: Guid::zero(),
            action_bars: None,
        },
    };
    if let Some(p) = map.players.get(&owner) {
        p.outbound.send(&msg);
    }
}

/// The pet learns `spell` (a higher rank replaces the lower).
pub fn learn(map: &mut Map, pet: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some(t) = data.spells.get(spell) else {
        return;
    };
    let owner = owner_of(map, pet);
    let Some(s) = state_mut(map, pet) else {
        return;
    };
    if s.spells.contains(&spell) {
        return;
    }
    let old = s
        .spells
        .iter()
        .position(|x| *x != spell && data.spells.same_kind(*x, spell));
    match old {
        Some(i) => {
            let was = s.spells[i];
            s.spells[i] = spell;
            for b in s.bar.iter_mut() {
                let (action, kind) = unpack(*b);
                if action == was {
                    *b = button(spell, kind);
                }
            }
            if let Some(a) = s.autocast.iter_mut().find(|a| **a == was) {
                *a = spell;
            }
        }
        None => {
            s.spells.push(spell);
            // A rank trained over another has its place already, on the
            // bar and in the autocast (Thistle Tea's `PetTraining.learn`).
            if autocastable(t) && !s.autocast.contains(&spell) {
                s.autocast.push(spell);
            }
            // Onto the bar's first empty spell slot.
            if !t.passive()
                && !s.bar.iter().any(|b| unpack(*b).0 == spell)
                && let Some(b) = s.bar[3..7].iter_mut().find(|b| unpack(**b).0 == 0)
            {
                *b = button(spell, spell_kind(&data, spell, &s.autocast));
            }
        }
    }
    if let Some(owner) = owner {
        send_spells(map, owner);
        sync_record(map, owner);
    }
}

/// Whether `pet` is `player`'s to command.
fn commands(map: &Map, player: Guid, pet: Guid) -> bool {
    if let Some(v) = map.players.get(&pet) {
        return v
            .owned
            .possessed_by
            .is_some_and(|pos| pos.master == player && pos.manual);
    }
    state(map, pet).is_some_and(|s| {
        s.owner == player
            && matches!(
                s.kind,
                Kind::Hunter | Kind::Demon | Kind::Charmed | Kind::Possessed
            )
    })
}

fn feedback(map: &Map, player: Guid, f: PetFeedback) {
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_PET_ACTION_FEEDBACK { feedback: f });
    }
}

/// The pet casts `spell` at `target` (its victim, or itself if helpful).
fn cast(map: &mut Map, player: Guid, pet: Guid, spell: u32, target: Option<Guid>) {
    let data = map.realm.data.clone();
    let knows = match map.players.get(&pet) {
        Some(v) => v.spells.contains(&spell),
        None => {
            state(map, pet).is_some_and(|s| s.spells.contains(&spell) || s.kind == Kind::Possessed)
        }
    };
    if !knows {
        return;
    }
    let Some(t) = data.spells.get(spell) else {
        return;
    };
    let helpful = crate::game::map::auras::positive(t);
    let victim = map.combat(pet).filter(|c| c.attacking).map(|c| c.victim);
    let unit = target
        .filter(|g| !g.is_zero())
        .or(if helpful { Some(pet) } else { victim });
    if !helpful
        && let Some(u) = unit
        && !crate::game::map::combat::can_attack(map, pet, u)
    {
        feedback(map, player, PetFeedback::CantAttackTarget);
        return;
    }
    if !helpful && unit.is_none() {
        feedback(map, player, PetFeedback::NothingToAttack);
        return;
    }
    let targets = Targets {
        unit,
        ..Default::default()
    };
    // A cast that can't be made is told to its master, as Thistle Tea's
    // `PetCasting` tells it.
    let m = crate::game::map::spells::mods::for_spell(map, pet, t);
    if let Some(r) = crate::game::map::spells::checks::check(map, pet, t, &targets, None, false, &m)
    {
        cast_failed(map, player, pet, spell, r);
        return;
    }
    let cast = crate::game::map::spells::cast(map, pet, spell, targets, None);
    // A harmful spell starts the fight with its target, an instant one of
    // no cooldown too.
    if cast
        && !helpful
        && let Some(u) = unit
    {
        ai::attack(map, pet, u);
    }
}

pub const SMSG_PET_CAST_FAILED: u16 = 0x138;

/// `SMSG_PET_CAST_FAILED` to the master, and the spell's cooldown on the
/// pet's bar cleared if it has none.
pub fn cast_failed(map: &Map, player: Guid, pet: Guid, spell: u32, reason: u8) {
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let mut b = crate::game::data::items::Body::default();
    b.u32(spell).u8(2).u8(reason);
    p.outbound.send_frame(b.frame(SMSG_PET_CAST_FAILED));
    let cooling = map
        .caster(pet)
        .is_some_and(|c| c.cooldowns.get(&spell).is_some_and(|t| *t > map.now));
    if !cooling {
        p.outbound.send(&SMSG_CLEAR_COOLDOWN {
            id: spell,
            target: pet,
        });
    }
}

/// `CMSG_PET_ACTION`: a command, a stance, or a spell.
fn action(map: &mut Map, player: Guid, pet: Guid, data: u32, target: Guid) {
    if !commands(map, player, pet) {
        return;
    }
    let alive = map.unit_view(pet).is_some_and(|u| u.alive);
    let (action, kind) = unpack(data);
    match kind {
        ACT_COMMAND => match action as u8 {
            COMMAND_STAY => {
                let at = map.position_of(pet);
                crate::game::map::combat::stop_attack(map, pet);
                ai::stop(map, pet);
                if let Some(s) = state_mut(map, pet) {
                    s.command = COMMAND_STAY;
                    s.stay_at = at;
                }
            }
            COMMAND_FOLLOW => {
                crate::game::map::combat::stop_attack(map, pet);
                if let Some(s) = state_mut(map, pet) {
                    s.command = COMMAND_FOLLOW;
                    s.stay_at = None;
                    s.returning = true;
                }
            }
            COMMAND_ATTACK => {
                if !alive {
                    return feedback(map, player, PetFeedback::PetDead);
                }
                if target.is_zero() || map.unit_view(target).is_none() {
                    return feedback(map, player, PetFeedback::NothingToAttack);
                }
                if !crate::game::map::combat::can_attack(map, pet, target) {
                    return feedback(map, player, PetFeedback::CantAttackTarget);
                }
                ai::attack(map, pet, target);
                let now = map.now;
                if let Some(s) = state_mut(map, pet) {
                    s.commanded = Some(target);
                    s.commanded_at = Some(now);
                }
            }
            COMMAND_DISMISS if map.players.contains_key(&pet) => possess::release(map, pet),
            COMMAND_DISMISS => dismiss(map, pet),
            _ => {}
        },
        ACT_REACTION => {
            if let Some(s) = state_mut(map, pet) {
                s.react = (action as u8).min(REACT_AGGRESSIVE);
            }
            if action as u8 == REACT_PASSIVE {
                crate::game::map::combat::stop_attack(map, pet);
            }
        }
        _ if action != 0 => {
            if !alive {
                return feedback(map, player, PetFeedback::PetDead);
            }
            cast(map, player, pet, action, Some(target));
        }
        _ => {}
    }
}

/// `CMSG_PET_SET_ACTION`: a button moved on the bar.
fn set_action(map: &mut Map, player: Guid, pet: Guid, slots: &[(u32, u32)]) {
    if !commands(map, player, pet) {
        return;
    }
    let data = map.realm.data.clone();
    let Some(s) = state_mut(map, pet) else {
        return;
    };
    // The edit is checked whole before either half is put on (Thistle
    // Tea's `PetControls.update`): each slot on the bar and named once; a
    // spell the pet knows, in a state its kind allows; and the commands
    // and stances moved about, but none taken off nor doubled.
    let mut bar = s.bar;
    let mut seen = Vec::new();
    for &(position, b) in slots {
        let Some(slot) = bar.get_mut(position as usize) else {
            return;
        };
        if seen.contains(&position) {
            return;
        }
        seen.push(position);
        let (action, kind) = unpack(b);
        let fits = match kind {
            ACT_COMMAND => action <= u32::from(COMMAND_DISMISS),
            ACT_REACTION => action <= u32::from(REACT_AGGRESSIVE),
            _ if action == 0 => matches!(kind, ACT_DISABLED | ACT_ENABLED | ACT_PASSIVE | 0),
            ACT_DISABLED | ACT_ENABLED | ACT_PASSIVE => {
                s.spells.contains(&action)
                    && data
                        .spells
                        .get(action)
                        .is_some_and(|t| !t.passive() && (kind != ACT_ENABLED || autocastable(t)))
            }
            _ => false,
        };
        if !fits {
            return;
        }
        *slot = b;
    }
    let fixed = |bar: &[u32; 10]| {
        let mut f: Vec<u32> = bar
            .iter()
            .copied()
            .filter(|b| matches!(unpack(*b).1, ACT_COMMAND | ACT_REACTION))
            .collect();
        f.sort_unstable();
        f
    };
    if fixed(&bar) != fixed(&s.bar) {
        return;
    }
    s.bar = bar;
    for &(_, b) in slots {
        let (action, kind) = unpack(b);
        if action != 0 && data.spells.get(action).is_some_and(autocastable) {
            match kind {
                ACT_ENABLED if !s.autocast.contains(&action) => s.autocast.push(action),
                ACT_DISABLED => s.autocast.retain(|a| *a != action),
                _ => {}
            }
        }
    }
    repaint(&data, s);
}

/// `CMSG_PET_SPELL_AUTOCAST`.
fn autocast(map: &mut Map, player: Guid, pet: Guid, spell: u32, on: bool) {
    if !commands(map, player, pet) {
        return;
    }
    let data = map.realm.data.clone();
    if !data.spells.get(spell).is_some_and(autocastable) {
        return;
    }
    let Some(s) = state_mut(map, pet) else {
        return;
    };
    if !s.spells.contains(&spell) {
        return;
    }
    s.autocast.retain(|a| *a != spell);
    if on {
        s.autocast.push(spell);
    }
    repaint(&data, s);
}

/// Whether a pet's name may be had: two to twelve letters, all of one
/// script, Latin (with its accents), Cyrillic or East Asian (mangos's
/// `CheckPetName` against mixed languages, Thistle Tea's `valid?`).
pub fn good_name(name: &str) -> bool {
    let script = |c: char| -> Option<u8> {
        match c as u32 {
            0x41..=0x5A | 0x61..=0x7A | 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x24F => Some(0),
            0x400..=0x4FF => Some(1),
            0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x4E00..=0x9FFF => Some(2),
            0xAC00..=0xD7AF => Some(2),
            _ => None,
        }
    };
    let first = name.chars().next().and_then(script);
    (2..=12).contains(&name.chars().count())
        && first.is_some()
        && name.chars().all(|c| script(c) == first)
}

/// `CMSG_PET_RENAME`: a hunter's pet may be named once.
fn rename(map: &mut Map, player: Guid, pet: Guid, name: &str) {
    let ok =
        state(map, pet).is_some_and(|s| s.owner == player && s.kind == Kind::Hunter && !s.renamed);
    if !ok {
        return;
    }
    if !good_name(name) {
        if let Some(p) = map.players.get(&player) {
            p.outbound.send(&SMSG_PET_NAME_INVALID {});
        }
        return;
    }
    let mut name = name.to_ascii_lowercase();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    let unix = u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
    if let Some(c) = map.creature_mut(pet) {
        // A new stamp, so the clients' caches take the name, even within
        // the second the last was stamped (Thistle Tea's `rename`); and no
        // more renaming.
        let stamp = unix.max(c.fields.get_u32(UNIT_FIELD_PET_NAME_TIMESTAMP) + 1);
        c.fields.set_u32(UNIT_FIELD_PET_NAME_TIMESTAMP, stamp);
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, super::summon::UNIT_FLAG_PET_RENAME, false);
        if let Some(s) = c.pet.as_deref_mut() {
            s.name = name;
            s.renamed = true;
        }
    }
    sync_record(map, player);
}

/// `CMSG_PET_NAME_QUERY`.
fn name_query(map: &Map, player: Guid, number: u32, g: Guid) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let Some(s) = c.pet.as_deref() else {
        return;
    };
    if s.number != number && number != 0 {
        return;
    }
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_PET_NAME_QUERY_RESPONSE {
            pet_number: s.number,
            name: s.name.clone(),
            pet_name_timestamp: c.fields.get_u32(UNIT_FIELD_PET_NAME_TIMESTAMP),
        });
    }
}

/// `CMSG_PET_ABANDON`: a hunter's pet is let go for good.
fn abandon(map: &mut Map, player: Guid, pet: Guid) {
    let Some(kind) = state(map, pet)
        .filter(|s| s.owner == player)
        .map(|s| s.kind)
    else {
        return;
    };
    summon::remove(map, pet);
    if kind == Kind::Hunter
        && let Some(p) = map.players.get_mut(&player)
    {
        p.row.state.pets.current = None;
        p.row.state.pets.active = false;
    } else if kind == Kind::Demon
        && let Some(p) = map.players.get_mut(&player)
    {
        p.row.state.pets.current = None;
        p.row.state.pets.active = false;
    }
}

fn targets_unit(t: &SpellCastTargets) -> Option<Guid> {
    t.target_flags.get_unit().map(|u| u.unit_target)
}

/// The pet and stable opcodes; anything else is handed back.
pub fn handle(map: &mut Map, g: Guid, msg: C) -> Option<C> {
    match msg {
        C::CMSG_PET_ACTION(m) => action(map, g, m.pet, m.data, m.target),
        C::CMSG_PET_SET_ACTION(m) => {
            let mut slots = vec![(m.position1, m.data1)];
            if let Some(e) = m.extra {
                slots.push((e.position2, e.data2));
            }
            set_action(map, g, m.guid, &slots);
        }
        C::CMSG_PET_SPELL_AUTOCAST(m) => autocast(map, g, m.guid, m.id, m.autocast_enabled),
        C::CMSG_PET_CAST_SPELL(m) => {
            if commands(map, g, m.guid) {
                cast(map, g, m.guid, m.id, targets_unit(&m.targets));
            }
        }
        C::CMSG_PET_RENAME(m) => rename(map, g, m.pet, &m.name),
        C::CMSG_PET_ABANDON(m) => abandon(map, g, m.pet),
        C::CMSG_PET_NAME_QUERY(m) => name_query(map, g, m.pet_number, m.guid),
        C::CMSG_REQUEST_PET_INFO => send_spells(map, g),
        C::CMSG_PET_CANCEL_AURA(m) => {
            // A dead pet's master is told so (mangos's
            // `HandlePetCancelAuraOpcode`).
            if commands(map, g, m.guid) && !map.unit_view(m.guid).is_some_and(|u| u.alive) {
                feedback(map, g, PetFeedback::PetDead);
            } else if commands(map, g, m.guid) {
                let helpful = crate::game::map::auras::auras(map, m.guid).is_some_and(|a| {
                    a.holders
                        .iter()
                        .any(|h| h.spell == m.id && h.positive && !h.passive)
                });
                if helpful {
                    crate::game::map::auras::remove_spell(map, m.guid, m.id, None);
                }
            }
        }
        C::CMSG_PET_STOP_ATTACK(m) => {
            if commands(map, g, m.pet) {
                crate::game::map::combat::stop_attack(map, m.pet);
            }
        }
        C::CMSG_PET_UNLEARN(m) => hunter::untrain(map, g, m.pet),
        C::MSG_LIST_STABLED_PETS(m) => hunter::list_stable(map, g, m.npc),
        C::CMSG_STABLE_PET(m) => hunter::stable(map, g, m.stable_master),
        C::CMSG_UNSTABLE_PET(m) => hunter::unstable(map, g, m.stable_master, m.pet_number),
        C::CMSG_STABLE_SWAP_PET(m) => hunter::swap(map, g, m.npc, m.pet_slot),
        C::CMSG_BUY_STABLE_SLOT(m) => hunter::buy_slot(map, g, m.npc),
        other => return Some(other),
    }
    None
}
