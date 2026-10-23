//! A load test's reset between raid attempts (mangos-classic has neither):
//! `.ready save` keeps a character's ready state (where it stands, its
//! buffs, its pet), `.ready load` puts it back in it, risen, whole, out of
//! combat, its cooldowns clear and its buffs fresh; `.respawn <radius>`
//! brings every creature about the GM back as new, and the objects there
//! back as they rest.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::auras::{self, Holder};
use crate::game::map::combat::dist;
use crate::game::map::death::Death;
use crate::game::map::spells::cooldowns::SavedAura;
use crate::game::map::{Map, MapKey};
use crate::game::store::ReadyState;

pub const USAGE: &str = "Usage: .ready [save|load]";

/// `.ready`, `.ready save`, `.ready load`: the GM's ready state shown,
/// kept, or put back on the selected player (or the GM).
pub fn command(map: &mut Map, g: Guid, args: &[&str], target: Guid) -> Option<String> {
    match args {
        [] => Some(show(map, g)),
        [a] if a.eq_ignore_ascii_case("save") => Some(save(map, g)),
        [a] if a.eq_ignore_ascii_case("load") => Some(load(map, target)),
        _ => Some(USAGE.into()),
    }
}

/// The buffs a ready state keeps: helpful ones put on by a cast, the
/// player's own or another's (not passive ones, not gear's, not another's
/// lasting aura of an area, which comes back by itself, nor one another
/// aura put on and takes with it), still on.
fn kept(h: &Holder, player: Guid, now: std::time::Instant) -> bool {
    h.positive
        && !h.passive
        && !h.equipped
        && h.source.is_none()
        && !(h.caster != player && h.expires.is_none())
        && h.expires.is_none_or(|t| t > now)
}

/// The holders `.ready load` takes off before it puts the buffs back:
/// every one but the passive, the gear's, and another's lasting aura of an
/// area.
fn cleared(h: &Holder, player: Guid) -> bool {
    !h.passive && !h.equipped && !(h.caster != player && h.expires.is_none())
}

fn save(map: &mut Map, g: Guid) -> String {
    let now = map.now;
    let key = map.key;
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return String::new();
    };
    if p.death != Death::Alive {
        return "You are dead: rise before you save a ready state.".into();
    }
    let channel = p
        .caster
        .casting
        .as_ref()
        .filter(|c| c.channel_until.is_some())
        .map(|c| c.spell);
    let auras: Vec<(SavedAura, Option<u32>)> = p
        .auras
        .holders
        .iter()
        .filter(|h| kept(h, g, now) && Some(h.spell) != channel)
        .map(|h| {
            (
                SavedAura {
                    spell: h.spell,
                    caster: h.caster.guid(),
                    item: 0,
                    stacks: h.stacks,
                    charges: h.charges,
                    left: h
                        .expires
                        .map(|t| t.saturating_duration_since(now).as_millis() as u32),
                    amounts: h.effects.iter().map(|x| (x.effect, x.amount)).collect(),
                    combo: h.combo,
                    triggered: h.triggered,
                },
                // Its full time, as the cast had it; one put back at a
                // login has what was left then.
                h.expires.and(h.duration),
            )
        })
        .collect();
    let at = p.position();
    let state = ReadyState {
        map: key.map,
        at: (at.x, at.y, at.z, p.movement.orientation),
        auras,
        pet: p.owned.pet.is_some(),
    };
    let names = buff_names(&state, &data);
    let pet = state.pet;
    p.row.state.ready = Some(state);
    format!(
        "Ready state saved at {:.1}, {:.1}, {:.1} in map {}: {}{}.",
        at.x,
        at.y,
        at.z,
        key.map,
        names,
        if pet { ", and your pet" } else { "" }
    )
}

fn buff_names(state: &ReadyState, data: &crate::game::data::GameData) -> String {
    if state.auras.is_empty() {
        return "no buffs".into();
    }
    let names: Vec<String> = state
        .auras
        .iter()
        .map(|(a, _)| {
            data.spells
                .get(a.spell)
                .map_or_else(|| a.spell.to_string(), |s| s.name.to_string())
        })
        .collect();
    format!("{} buffs ({})", names.len(), names.join(", "))
}

fn show(map: &Map, g: Guid) -> String {
    let Some(p) = map.players.get(&g) else {
        return String::new();
    };
    let Some(s) = &p.row.state.ready else {
        return "No ready state saved: .ready save keeps one.".into();
    };
    format!(
        "Ready state at {:.1}, {:.1}, {:.1} in map {}: {}{}.",
        s.at.0,
        s.at.1,
        s.at.2,
        s.map,
        buff_names(s, &map.realm.data),
        if s.pet { ", and a pet" } else { "" }
    )
}

/// `target` back in its ready state: risen, at its place, out of combat
/// and forgotten by every creature, its cast stopped, every aura but the
/// passive and the gear's off, its cooldowns clear, its saved buffs back
/// at their full time (from their caster, if here, else its own), its
/// health and mana or energy full and its rage none; its pet up again and
/// whole.
fn load(map: &mut Map, target: Guid) -> String {
    let Some(p) = map.players.get(&target) else {
        return String::new();
    };
    let name = p.name.to_string();
    let Some(state) = p.row.state.ready.clone() else {
        return format!("{name} has no ready state: .ready save keeps one.");
    };
    let to = Vector3d {
        x: state.at.0,
        y: state.at.1,
        z: state.at.2,
    };
    if state.map != map.key.map {
        // The rest is for the map it was saved on, once there.
        map.teleport(target, MapKey::world(state.map), to, state.at.3);
        return format!(
            "{name}'s ready state is in map {}: sent there; .ready load again once in.",
            state.map
        );
    }
    if map
        .players
        .get(&target)
        .is_some_and(|p| p.death != Death::Alive)
    {
        crate::game::map::death::resurrect(map, target, 1.0);
    }
    // A spirit of redemption is a priest again, alive: its state's auras are
    // passive (27792, 27795), which the clearing below keeps, and the second
    // running out killed the priest seconds into the next attempt.
    if crate::game::map::spells::redemption::spirit(map, target) {
        crate::game::map::spells::redemption::end_without_death(map, target);
    }
    crate::game::map::spells::interrupt(map, target);
    crate::game::map::combat::leave_combat(map, target);
    crate::game::map::ai::forget(map, target);
    let key = map.key;
    map.teleport(target, key, to, state.at.3);
    auras::remove_where(map, target, |h| cleared(h, target));
    let cleared = crate::game::map::gm_dev::clear_cooldowns(map, target);
    let data = map.realm.data.clone();
    let mut back = 0;
    for (a, full) in &state.auras {
        let Some(s) = data.spells.get(a.spell) else {
            continue;
        };
        let effects: Vec<(usize, i32)> = a
            .amounts
            .iter()
            .filter(|(i, _)| (*i as usize) < 3)
            .map(|(i, v)| (*i as usize, *v))
            .collect();
        if effects.is_empty() {
            continue;
        }
        // From its caster, as it was, where they are here: a blessing and
        // a stack stay theirs.
        let caster = Guid::new(a.caster);
        let here = map.players.contains_key(&caster)
            || (guid::is_unit(caster) && map.creature(caster).is_some_and(|c| c.alive()));
        let caster = if here { caster } else { target };
        let m = crate::game::map::spells::mods::for_spell(map, caster, s);
        let ok = auras::apply_spell(
            map,
            caster,
            target,
            s,
            &effects,
            auras::Context {
                item: None,
                equipped: false,
                combo: a.combo,
                duration: Some(*full),
                charges: Some(a.charges),
                triggered: a.triggered,
                linked: false,
                activation: (
                    m.flat[crate::game::map::spells::mods::OP_ACTIVATION_TIME],
                    m.pct[crate::game::map::spells::mods::OP_ACTIVATION_TIME],
                ),
                dr: None,
            },
        );
        if ok.is_ok() {
            auras::set_stacks(map, target, a.spell, caster, None, a.stacks.max(1));
            back += 1;
        }
    }
    // The place's auras again.
    crate::game::map::spell_areas::reconcile(map, target);
    fill(map, target);
    if let Some(p) = map.players.get_mut(&target) {
        p.caster.combo_points = 0;
        p.caster.combo_target = Guid::zero();
    }
    let pet = state.pet && pet_up(map, target);
    format!(
        "{name} is ready: {back} of {} buffs back, {cleared} cooldowns cleared{}.",
        state.auras.len(),
        if pet { ", the pet whole" } else { "" }
    )
}

/// Health, mana and energy full, rage none: a fresh pull.
fn fill(map: &mut Map, g: Guid) {
    let Some(f) = map.unit_fields_mut(g) else {
        return;
    };
    let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
    f.set_u32(UNIT_FIELD_HEALTH, max.max(1));
    for power in [0u16, 2, 3] {
        let max = f.get_u32(UNIT_FIELD_MAXPOWER1 + power);
        f.set_u32(UNIT_FIELD_POWER1 + power, max);
    }
    f.set_u32(UNIT_FIELD_POWER1 + 1, 0);
}

/// The player's pet up again if it fell, and whole, out of combat and
/// forgotten, its harmful auras gone. Whether there is one.
fn pet_up(map: &mut Map, owner: Guid) -> bool {
    let dead = crate::game::map::pets::pet_of(map, owner)
        .is_none_or(|pet| map.unit_view(pet).is_none_or(|u| !u.alive));
    if dead {
        crate::game::map::pets::revive_pet(map, owner, 100);
    }
    let Some(pet) = crate::game::map::pets::pet_of(map, owner) else {
        return false;
    };
    if map.unit_view(pet).is_none_or(|u| !u.alive) {
        return false;
    }
    crate::game::map::spells::interrupt(map, pet);
    crate::game::map::combat::leave_combat(map, pet);
    crate::game::map::ai::forget(map, pet);
    auras::remove_where(map, pet, |h| !h.passive && !h.positive);
    if let Some(c) = map.caster_mut(pet) {
        c.cooldowns.clear();
        c.category_cooldowns.clear();
        c.gcd_until = None;
    }
    fill(map, pet);
    true
}

/// The farthest `.respawn <radius>` reaches.
pub const MAX_RADIUS: f32 = 500.0;

/// `.respawn <radius>`: every creature within `radius` yards of the GM (where
/// it is, or where it is from) back as new: the dead risen, the living
/// home, whole, out of combat, their scripts begun again; what they
/// summoned gone. Players' pets, totems and charmed units stay. The objects
/// there in use come back as they rest, and those gone come back now.
pub fn respawn_around(map: &mut Map, g: Guid, radius: f32) -> String {
    let Some(at) = map.position_of(g) else {
        return String::new();
    };
    let near = |p: Vector3d| dist(p, at) <= radius;
    let by_player = |o: Option<Guid>| o.is_some_and(guid::is_player);
    let spawned: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.spawn_guid.is_some() && (near(c.position) || near(c.home)))
        .filter(|(_, c)| !by_player(c.owner))
        .map(|(_, c)| c.guid)
        .collect();
    // Their summons, and the summons' own, gone first: their summoners,
    // new, hear nothing of them.
    let mut owners: ahash::AHashSet<Guid> = spawned.iter().copied().collect();
    let mut summons: Vec<Guid> = Vec::new();
    loop {
        let more: Vec<Guid> = map
            .creatures
            .iter()
            .filter(|(_, c)| c.spawn_guid.is_none() && !summons.contains(&c.guid))
            .filter(|(_, c)| {
                let summoner = c.scripted.as_deref().and_then(|s| s.summoner);
                summoner.is_some_and(|s| owners.contains(&s))
                    || c.owner.is_some_and(|o| owners.contains(&o))
            })
            .map(|(_, c)| c.guid)
            .collect();
        if more.is_empty() {
            break;
        }
        owners.extend(more.iter().copied());
        summons.extend(more);
    }
    for s in &summons {
        crate::game::map::scripts::remove_for_good(map, *s);
    }
    for c in &spawned {
        crate::game::map::ai::respawn_forced(map, *c, true);
    }
    crate::game::map::battleground::av_scripts::respawned(map, &spawned);
    // Objects: one in use back as it rests now, one gone back now.
    let data = map.realm.data.clone();
    let used: Vec<Guid> = map
        .objects
        .closes
        .iter()
        .map(|(_, o, _)| *o)
        .filter(|o| map.gameobject(*o).is_some_and(|x| near(x.position)))
        .collect();
    for o in &used {
        crate::game::map::objects::reset(map, *o);
    }
    let now = map.now;
    let spawns = data.gameobject_spawns.get(&map.key.map);
    let mut objects = used.len();
    let close = |s: u32| {
        spawns
            .and_then(|v| v.iter().find(|x| x.guid == s))
            .is_some_and(|x| {
                near(Vector3d {
                    x: x.x,
                    y: x.y,
                    z: x.z,
                })
            })
    };
    // A used object's, and a looted chest's or node's.
    for (when, s) in map
        .objects
        .respawns
        .iter_mut()
        .chain(map.loot_state.respawns.iter_mut())
    {
        if *when > now && close(*s) {
            *when = now;
            objects += 1;
        }
    }
    format!(
        "Respawned {} creatures within {radius:.0} yards, {} summons gone, {objects} objects reset.",
        spawned.len(),
        summons.len()
    )
}
