//! Making controlled creatures and taking them away: a pet from its record
//! or a template, its stats for its level, its spellbook, and its going.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::*;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::guid::HighGuid;
use crate::game::map::creature::{Creature, Life, Motion};
use crate::game::map::player::Player;
use crate::game::store::PetRecord;

pub const KIND_HUNTER: u8 = 1;
pub const KIND_DEMON: u8 = 2;
/// Growl, rank 1: every hunter's pet knows it.
const GROWL: u32 = 2649;
/// The class script Demonic Sacrifice's buffs carry.
const DEMONIC_SACRIFICE_SCRIPT: i32 = 2228;

/// A template, or a stand-in for an entry the data lacks.
pub(super) fn template(data: &GameData, entry: u32) -> CreatureTemplate {
    data.creature_template(entry)
        .cloned()
        .unwrap_or_else(|| CreatureTemplate {
            entry,
            name: "Pet".into(),
            display_ids: [262, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            health_max: 100,
            base_attack_time: 2000,
            unit_class: 1,
            ..Default::default()
        })
}

/// A new controlled creature of `entry` for `owner` at `at`: its owner's
/// faction, its stats for `level`, and in the map.
pub fn spawn(
    map: &mut Map,
    owner: Guid,
    entry: u32,
    kind: Kind,
    level: u8,
    at: Vector3d,
    orientation: f32,
) -> Guid {
    let now = map.now;
    let data = map.realm.data.clone();
    let t = template(&data, entry);
    let counter = map.next_creature_counter | 0xA0_0000;
    map.next_creature_counter += 1;
    let g = if kind.controlled() {
        guid::make(HighGuid::Pet, entry, counter)
    } else {
        guid::unit(entry, counter)
    };
    let mut c = Creature::new(g, &t, level.max(1), at, orientation);
    let faction = map
        .unit_fields(owner)
        .map(|f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
        .unwrap_or(t.faction);
    // Under a player's control if its owner is: a player, or one of theirs
    // (mangos copies the caster's flag, Thistle Tea's `attach_owner` sets
    // it for a player's). A creature's keeps its template's flags. A pet
    // with a bar has that flag alone (mangos's `Pet`); a guardian, totem
    // or companion keeps its template's beside it (mangos's `SetFlag`,
    // Thistle Tea's `attach_owner`).
    let controlled = guid::is_player(owner)
        || map
            .unit_fields(owner)
            .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PLAYER_CONTROLLED));
    let f = &mut c.fields;
    f.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
    if controlled && kind.controlled() {
        f.set_u32(UNIT_FIELD_FLAGS, UNIT_FLAG_PLAYER_CONTROLLED);
    } else if controlled {
        f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PLAYER_CONTROLLED, true);
    }
    // A companion keeps its template's: some give quests, as Children's
    // Week's orphans (mangos's `EffectSummonCritter`); the others none.
    let npc_flags = if kind == Kind::Critter {
        t.npc_flags
    } else {
        0
    };
    f.set_u32(UNIT_NPC_FLAGS, npc_flags);
    f.set_guid(UNIT_FIELD_SUMMONEDBY, owner);
    f.set_guid(UNIT_FIELD_CREATEDBY, owner);
    creature_stats(&data, &mut c, &t, level);
    c.owner = Some(owner);
    c.hostile_to_players = false;
    c.home = at;
    c.pet = Some(Box::new(PetState::new(kind, owner, now)));
    map.add_creature(c.with_base());
    g
}

/// A summoned creature's stats from its class at its level, as a spawn's.
pub(super) fn creature_stats(data: &GameData, c: &mut Creature, t: &CreatureTemplate, level: u8) {
    c.fit_model(data, Some(t));
    let Some(stats) = data.class_level_stats.get(&(t.unit_class.max(1), level)) else {
        return;
    };
    let health = crate::game::data::load::creature_health(stats.health, t.health_multiplier).max(1);
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
    c.fields.set_u32(UNIT_FIELD_HEALTH, health);
    let mana = (stats.mana as f32 * t.mana_multiplier.max(0.0)).round() as u32;
    if mana > 0 {
        c.fields.set_u32(UNIT_FIELD_MAXPOWER1, mana);
        c.fields.set_u32(UNIT_FIELD_POWER1, mana);
    }
    let mult = if t.damage_multiplier > 0.0 {
        t.damage_multiplier
    } else {
        1.0
    };
    let base = stats.melee_damage * mult;
    c.fields
        .set_f32(UNIT_FIELD_MINDAMAGE, (base * 0.86).max(1.0));
    c.fields
        .set_f32(UNIT_FIELD_MAXDAMAGE, (base * 1.14).max(1.0));
    c.fields
        .set_u32(UNIT_FIELD_ATTACK_POWER, stats.attack_power);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, stats.armor);
}

/// A pet's stats for its level: a hunter's pet from the generic hunter
/// pet's table, a demon from its own; its damage by its happiness.
pub fn apply_level(data: &GameData, c: &mut Creature, level: u8) {
    let Some(s) = c.pet.as_deref() else {
        return;
    };
    let hunter = s.kind == Kind::Hunter;
    let row = if hunter { 1 } else { c.entry };
    c.fields.set_u32(UNIT_FIELD_LEVEL, u32::from(level));
    if let Some(l) = data.pets.level(row, level) {
        for (i, v) in l.stats.iter().enumerate() {
            c.fields.set_u32(UNIT_FIELD_STAT0 + i as u16, *v);
        }
        c.fields.set_u32(UNIT_FIELD_MAXHEALTH, l.health.max(1));
        c.fields.set_u32(UNIT_FIELD_RESISTANCES, l.armor);
        if l.mana > 0 && !hunter {
            c.fields.set_u32(UNIT_FIELD_MAXPOWER1, l.mana);
        }
        let strength = l.stats[0];
        c.fields
            .set_u32(UNIT_FIELD_ATTACK_POWER, (strength * 2).saturating_sub(20));
        let (min, max) = if hunter || l.damage.1 <= 0.0 {
            // Thistle Tea's hunter pet damage: by level and attack speed.
            let speed = c.fields.get_u32(UNIT_FIELD_BASEATTACKTIME).max(1000) as f32 / 1000.0;
            let lvl = f32::from(level);
            (
                lvl * 1.15 * 1.05 * speed / 2.0,
                lvl * 1.45 * 1.05 * speed / 2.0,
            )
        } else {
            l.damage
        };
        let mult = if hunter {
            hunter::damage_multiplier(c.fields.get_u32(UNIT_FIELD_POWER1 + POWER_HAPPINESS))
        } else {
            1.0
        };
        c.fields
            .set_f32(UNIT_FIELD_MINDAMAGE, (min * mult).max(1.0));
        c.fields
            .set_f32(UNIT_FIELD_MAXDAMAGE, (max * mult).max(1.0));
    }
    if hunter
        && let Some(scale) = c
            .pet
            .as_deref()
            .and_then(|s| data.pets.families.get(&s.family))
            .and_then(|f| f.scale(u32::from(level)))
    {
        c.fields.set_f32(OBJECT_FIELD_SCALE_X, scale);
    }
    if hunter {
        c.fields
            .set_u32(UNIT_FIELD_PETNEXTLEVELEXP, pet_xp_to_level(data, level));
        c.fields.set_u32(UNIT_FIELD_MAXPOWER1 + POWER_FOCUS, 100);
        c.fields
            .set_u32(UNIT_FIELD_MAXPOWER1 + POWER_HAPPINESS, HAPPINESS_MAX);
    }
    // The base the derived stats start from is the level's.
    c.base = crate::game::map::derive::UnitBase::capture(&c.fields, c.base.run_speed);
    crate::game::map::derive::creature(c);
}

/// The spells a pet knows at `level`: its first spells (a creature's
/// list, or a demon's create spells and what the Grimoires teach, with a
/// hunter's pet's Growl), each at its highest rank by then in its skill
/// lines, as Thistle Tea's `pet_spellbook` works it out; and a hunter's
/// pet's family passives.
pub fn spellbook(data: &GameData, entry: u32, level: u8, hunter: bool, family: u32) -> Vec<u32> {
    let mut first: Vec<u32> = data
        .pets
        .creature_spell_list
        .get(&entry)
        .and_then(|l| data.pets.spell_lists.get(l))
        .cloned()
        .unwrap_or_default();
    let mut grimoires = false;
    if first.is_empty() {
        first = data
            .pets
            .create_spells
            .get(&entry)
            .cloned()
            .unwrap_or_default();
        grimoires = !hunter;
    }
    if hunter {
        first.push(GROWL);
    }
    // The skill lines the first spells are in.
    let mut skills: Vec<u32> = first
        .iter()
        .flat_map(|s| {
            data.spells
                .skill_abilities
                .get(s)
                .into_iter()
                .flatten()
                .map(|a| a.skill)
        })
        .filter(|s| data.pets.abilities.contains_key(s))
        .collect();
    skills.sort_unstable();
    skills.dedup();
    if grimoires {
        first.extend(data.pets.grimoire_spells.iter().copied());
    }
    let names: Vec<&str> = first
        .iter()
        .filter_map(|s| data.spells.get(*s))
        .map(|s| s.name.as_str())
        .collect();
    let mut best: Vec<(String, u32, u32)> = Vec::new();
    for skill in &skills {
        for a in data.pets.abilities.get(skill).into_iter().flatten() {
            let Some(s) = data.spells.get(a.spell) else {
                continue;
            };
            if s.passive() || !names.contains(&s.name.as_str()) || s.base_level > u32::from(level) {
                continue;
            }
            match best.iter_mut().find(|(n, _, _)| *n == s.name) {
                Some(b) if b.1 < s.base_level => *b = (s.name.clone(), s.base_level, s.id),
                Some(_) => {}
                None => best.push((s.name.clone(), s.base_level, s.id)),
            }
        }
    }
    let mut spells: Vec<u32> = best.into_iter().map(|(_, _, id)| id).collect();
    // First spells outside any pet skill line stay as they are.
    for s in first.iter().copied() {
        let lined = data
            .spells
            .skill_abilities
            .get(&s)
            .is_some_and(|v| v.iter().any(|a| skills.contains(&a.skill)));
        if !lined
            && data.spells.get(s).is_some()
            && !spells.contains(&s)
            && !grimoire_only(data, s, grimoires)
        {
            spells.push(s);
        }
    }
    if hunter {
        spells.extend(family_passives(data, family));
    }
    spells.sort_unstable();
    spells.dedup();
    spells
}

fn grimoire_only(data: &GameData, s: u32, grimoires: bool) -> bool {
    grimoires && data.pets.grimoire_spells.contains(&s)
}

/// A family's passives: learned when tamed.
pub fn family_passives(data: &GameData, family: u32) -> Vec<u32> {
    let Some(f) = data.pets.families.get(&family) else {
        return Vec::new();
    };
    f.skills
        .iter()
        .filter(|s| **s != 0)
        .flat_map(|s| data.pets.abilities.get(s).into_iter().flatten())
        .filter(|a| a.acquire == 2)
        .filter(|a| data.spells.get(a.spell).is_some_and(|s| s.passive()))
        .map(|a| a.spell)
        .collect()
}

/// The spells a charmed creature can be told to cast.
/// A charmed creature's abilities from `creature_charm_spells`, if it has
/// any: one roll a slot picks at most one by its availability, which
/// recovers over a time rolled from its slot's range (Thistle Tea's
/// `CharmSpells.select`); a spell that charms or possesses isn't offered.
/// (spell, cooldown in milliseconds), in slot order.
pub fn pick_charm_spells(
    data: &GameData,
    entry: u32,
    rng: &mut impl rand::Rng,
) -> Option<Vec<(u32, u32)>> {
    let options = data.pets.charm_spells.get(&entry)?;
    let controls = |id: u32| {
        data.spells.get(id).is_some_and(|s| {
            s.has_aura(crate::game::map::auras::AURA_MOD_CHARM)
                || s.has_aura(crate::game::map::auras::AURA_MOD_POSSESS)
        })
    };
    let mut out = Vec::new();
    for slot in 0..4 {
        let roll = rng.random_range(0.0f32..1.0) * 100.0;
        let mut sum = 0.0;
        let picked = options.iter().filter(|o| o.slot == slot).find(|o| {
            let hit = roll > sum && roll <= sum + o.availability;
            sum += o.availability;
            hit
        });
        if let Some(o) = picked.filter(|o| data.spells.get(o.spell).is_some() && !controls(o.spell))
        {
            let cooldown = rng.random_range(o.cooldown_min_ms..=o.cooldown_max_ms);
            out.push((o.spell, cooldown));
        }
    }
    Some(out)
}

pub fn charm_spells(data: &GameData, entry: u32) -> Vec<u32> {
    if let Some(list) = data
        .pets
        .creature_spell_list
        .get(&entry)
        .and_then(|l| data.pets.spell_lists.get(l))
    {
        return list.clone();
    }
    let Some(t) = data.creature_template(entry) else {
        return Vec::new();
    };
    data.spells
        .creature_spells
        .get(&t.spell_list_id)
        .map(|l| l.iter().map(|s| s.spell).collect())
        .unwrap_or_default()
}

/// A demon's name: one start and one end from its kind's halves.
fn demon_name(data: &GameData, entry: u32) -> Option<String> {
    let halves = data.pets.names.get(&entry)?;
    let mut rng = rand::rng();
    let starts: Vec<&String> = halves
        .iter()
        .filter(|(_, h)| *h == 0)
        .map(|(w, _)| w)
        .collect();
    let ends: Vec<&String> = halves
        .iter()
        .filter(|(_, h)| *h == 1)
        .map(|(w, _)| w)
        .collect();
    if starts.is_empty() || ends.is_empty() {
        return None;
    }
    let s = starts[rng.random_range(0..starts.len())];
    let e = ends[rng.random_range(0..ends.len())];
    Some(format!("{s}{e}"))
}

/// A fresh pet number for `owner`.
pub fn next_number(map: &mut Map, owner: Guid) -> u32 {
    let Some(p) = map.players.get_mut(&owner) else {
        return 1;
    };
    p.row.state.pets.next_number = p.row.state.pets.next_number.wrapping_add(1);
    ((guid::counter(owner) & 0x7F_FFFF) << 8) | (p.row.state.pets.next_number & 0xFF)
}

/// Where a pet appears: at its master's side.
pub(super) fn beside(map: &Map, owner: Guid) -> (Vector3d, f32) {
    let (pos, o) = map
        .unit_view(owner)
        .map(|u| (u.position, u.orientation))
        .unwrap_or((
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        ));
    let a = o + std::f32::consts::FRAC_PI_2 * 1.5;
    (
        Vector3d {
            x: pos.x + FOLLOW_DISTANCE * a.cos(),
            y: pos.y + FOLLOW_DISTANCE * a.sin(),
            z: pos.z,
        },
        o,
    )
}

/// A hunter's pet's flags for its frame's Rename and Abandon.
pub const UNIT_FLAG_PET_RENAME: u32 = 0x10;
pub const UNIT_FLAG_PET_ABANDON: u32 = 0x20;

/// `owner`'s pet from its record, at their side.
pub fn from_record(map: &mut Map, owner: Guid, r: &PetRecord) -> Option<Guid> {
    let data = map.realm.data.clone();
    if map.players.get(&owner)?.owned.pet.is_some() {
        return None;
    }
    let kind = if r.kind == KIND_HUNTER {
        Kind::Hunter
    } else {
        Kind::Demon
    };
    let (at, o) = beside(map, owner);
    let level = r.level.max(1);
    let g = spawn(map, owner, r.entry, kind, level, at, o);
    let unix = u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
    let now = map.now;
    let c = map.creature_mut(g)?;
    if r.display_id != 0 {
        c.fields.set_u32(UNIT_FIELD_DISPLAYID, r.display_id);
        c.fields.set_u32(UNIT_FIELD_NATIVEDISPLAYID, r.display_id);
    }
    let class = if kind == Kind::Hunter {
        1
    } else {
        c.fields.get_byte(UNIT_FIELD_BYTES_0, 1).max(1)
    };
    let power = if kind == Kind::Hunter { 2 } else { 0 };
    c.fields.set_bytes(UNIT_FIELD_BYTES_0, 0, class, 0, power);
    c.fields.set_u32(UNIT_FIELD_PETNUMBER, r.number);
    c.fields.set_u32(UNIT_FIELD_PET_NAME_TIMESTAMP, unix);
    c.fields.set_u32(UNIT_CREATED_BY_SPELL, r.summon_spell);
    if kind == Kind::Hunter {
        // The pet frame's Rename (until it is named) and Abandon (mangos's
        // `UNIT_FLAG_PET_RENAME` and `UNIT_FLAG_PET_ABANDON`).
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PET_RENAME, !r.renamed);
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PET_ABANDON, true);
        c.fields.set_u32(
            UNIT_FIELD_POWER1 + POWER_HAPPINESS,
            r.happiness.min(HAPPINESS_MAX),
        );
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 1, r.loyalty.max(1));
        c.fields.set_u32(UNIT_FIELD_PETEXPERIENCE, r.xp);
    }
    {
        let s = c.pet.as_deref_mut()?;
        s.number = r.number;
        s.name = r.name.clone();
        s.renamed = r.renamed;
        s.spells = r.spells.clone();
        s.family = r.family;
        s.family_spells = family_passives(&data, r.family);
        s.autocast = r.autocast.clone();
        s.react = r.react;
        s.command = COMMAND_FOLLOW;
        s.summon_spell = r.summon_spell;
        s.loyalty_points = r.loyalty_points;
        s.training_points = r.training_points;
        s.food_mask = data.pets.families.get(&r.family).map_or(0, |f| f.food_mask);
        s.bar = if r.bar.len() == 10 {
            std::array::from_fn(|i| r.bar[i])
        } else {
            control::default_bar(&data, &s.spells, &s.autocast, false)
        };
    }
    apply_level(&data, c, level);
    // Its cooldowns ran on while it was away.
    let unix = chrono::Utc::now().timestamp_millis();
    for (spell, until) in &r.cooldowns {
        if *until > unix {
            c.caster
                .cooldowns
                .insert(*spell, now + Duration::from_millis((*until - unix) as u64));
        }
    }
    let max_health = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    let health = if r.health == 0 {
        max_health
    } else {
        r.health.min(max_health)
    };
    c.fields.set_u32(UNIT_FIELD_HEALTH, health);
    let power_index = if kind == Kind::Hunter { POWER_FOCUS } else { 0 };
    let max_power = c.fields.get_u32(UNIT_FIELD_MAXPOWER1 + power_index);
    c.fields.set_u32(
        UNIT_FIELD_POWER1 + power_index,
        if r.power == 0 {
            max_power
        } else {
            r.power.min(max_power)
        },
    );
    if kind == Kind::Hunter {
        hunter::show_training_points(c);
    }
    attach(map, owner, g);
    // Its passives take hold.
    let passives: Vec<u32> = state(map, g)
        .map(|s| s.spells.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|s| data.spells.get(*s).is_some_and(|t| t.passive()))
        .collect();
    // A hunter learns to teach the passives their pet came with (Thistle
    // Tea's `PetTraining.discover_passives`).
    if kind == Kind::Hunter {
        for spell in &passives {
            hunter::discover_passive(map, g, *spell);
        }
    }
    for spell in passives {
        apply_passive(map, g, spell);
    }
    inherit::sync(map, g);
    Some(g)
}

/// Pet `g`'s passive `spell` takes hold, its amounts by its level and its
/// master's modifiers (mangos's `CalculateSpellEffectValue` through
/// `GetSpellModOwner`: Endurance Training on the tamed pets' health
/// passive).
pub(super) fn apply_passive(map: &mut Map, g: Guid, spell: u32) {
    let data = map.realm.data.clone();
    let Some(t) = data.spells.get(spell) else {
        return;
    };
    let level = map.unit_view(g).map_or(1, |u| u.level);
    let mods = crate::game::map::spells::mods::for_spell(map, g, t);
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| t.effects[*i].effect == 6 && t.effects[*i].aura != 0)
        .map(|i| {
            (
                i,
                mods.points(t, i, crate::game::map::spells::points(t, i, level)),
            )
        })
        .collect();
    let _ = crate::game::map::auras::apply_spell(
        map,
        g,
        g,
        t,
        &effects,
        crate::game::map::auras::Context {
            duration: Some(None),
            ..Default::default()
        },
    );
}

/// `pet` becomes `owner`'s: their summon field, and its bar to their
/// client.
fn attach(map: &mut Map, owner: Guid, pet: Guid) {
    if let Some(p) = map.players.get_mut(&owner) {
        p.owned.pet = Some(pet);
        p.owned.unsummoned = false;
        p.fields.set_guid(UNIT_FIELD_SUMMON, pet);
        p.row.state.pets.active = true;
    }
    // An owner still loading the map gets it once there
    // (`movement::worldport_ack`), after the pet's create.
    if !map
        .players
        .get(&owner)
        .is_some_and(|p| p.awaiting_worldport)
    {
        send_spells(map, owner);
    }
    sync_record(map, owner);
}

/// A warlock's demon of `entry`: the one before goes.
pub fn summon_demon(map: &mut Map, owner: Guid, entry: u32, spell: u32) {
    let data = map.realm.data.clone();
    if let Some(old) = pet_of(map, owner) {
        dismiss(map, old);
    }
    // A demon summoned ends what Demonic Sacrifice gave for the last
    // (its buffs' class script 2228), as mangos-classic's `EffectSummonPet`.
    if guid::is_player(owner) {
        crate::game::map::auras::remove_where(map, owner, |h| {
            h.effects.iter().any(|x| {
                x.kind == crate::game::map::auras::AURA_OVERRIDE_CLASS_SCRIPTS
                    && x.misc == DEMONIC_SACRIFICE_SCRIPT
            })
        });
    }
    let level = map
        .unit_view(owner)
        .map(|u| u.level)
        .unwrap_or(1)
        .clamp(1, 60) as u8;
    let t = template(&data, entry);
    let number = next_number(map, owner);
    let spells = spellbook(&data, entry, level, false, t.family);
    let autocast: Vec<u32> = spells
        .iter()
        .copied()
        .filter(|s| data.spells.get(*s).is_some_and(control::autocastable))
        .collect();
    // The name this demon drew before, else a new one, kept.
    let kept = map.players.get(&owner).and_then(|p| {
        p.row
            .state
            .pets
            .demon_names
            .iter()
            .find(|(e, _)| *e == entry)
            .map(|(_, n)| n.clone())
    });
    let name = match kept {
        Some(n) => n,
        None => {
            let drawn = demon_name(&data, entry);
            if let (Some(n), Some(p)) = (&drawn, map.players.get_mut(&owner)) {
                p.row.state.pets.demon_names.push((entry, n.clone()));
            }
            drawn.unwrap_or_else(|| t.name.clone())
        }
    };
    let r = PetRecord {
        number,
        entry,
        kind: KIND_DEMON,
        name,
        level,
        bar: control::default_bar(&data, &spells, &autocast, false).to_vec(),
        spells,
        autocast,
        react: REACT_DEFENSIVE,
        command: COMMAND_FOLLOW,
        summon_spell: spell,
        family: t.family,
        ..Default::default()
    };
    if let Some(p) = map.players.get_mut(&owner) {
        p.row.state.pets.current = Some(r.clone());
        p.row.state.pets.active = true;
    }
    from_record(map, owner, &r);
}

/// Call Pet: the hunter's current pet, from the stable of one.
pub fn call_pet(map: &mut Map, owner: Guid, spell: u32) {
    let _ = spell;
    let Some(r) = map
        .players
        .get(&owner)
        .and_then(|p| p.row.state.pets.current.clone())
        .filter(|r| !r.dead)
    else {
        return;
    };
    from_record(map, owner, &r);
}

/// The record of a pet as it now is.
pub fn record_of(map: &Map, pet: Guid) -> Option<PetRecord> {
    let c = map.creature(pet)?;
    let s = c.pet.as_deref()?;
    if !matches!(s.kind, Kind::Hunter | Kind::Demon) {
        return None;
    }
    let hunter = s.kind == Kind::Hunter;
    let f = &c.fields;
    Some(PetRecord {
        number: s.number,
        entry: c.entry,
        kind: if hunter { KIND_HUNTER } else { KIND_DEMON },
        name: s.name.clone(),
        renamed: s.renamed,
        level: f.get_u32(UNIT_FIELD_LEVEL) as u8,
        xp: f.get_u32(UNIT_FIELD_PETEXPERIENCE),
        health: f.get_u32(UNIT_FIELD_HEALTH),
        power: f.get_u32(UNIT_FIELD_POWER1 + if hunter { POWER_FOCUS } else { 0 }),
        happiness: f.get_u32(UNIT_FIELD_POWER1 + POWER_HAPPINESS),
        loyalty: f.get_byte(UNIT_FIELD_BYTES_1, 1),
        loyalty_points: s.loyalty_points,
        training_points: s.training_points,
        spells: s.spells.clone(),
        autocast: s.autocast.clone(),
        bar: s.bar.to_vec(),
        // Under Eyes of the Beast, the stance it goes back to.
        react: s.eyes.map_or(s.react, |(react, _)| react),
        command: s.eyes.map_or(s.command, |(_, command)| command),
        dead: !c.alive(),
        summon_spell: s.summon_spell,
        display_id: f.get_u32(UNIT_FIELD_NATIVEDISPLAYID),
        family: s.family,
        cooldowns: {
            let now = map.now;
            let unix = chrono::Utc::now().timestamp_millis();
            c.caster
                .cooldowns
                .iter()
                .filter(|(_, t)| **t > now)
                .map(|(spell, t)| (*spell, unix + t.duration_since(now).as_millis() as i64))
                .collect()
        },
    })
}

/// Take a controlled creature out of the map, and out of its master's
/// hands.
pub fn remove(map: &mut Map, g: Guid) {
    let owner = owner_of(map, g);
    // Seen through, its master sees through their own eyes again
    // (Thistle Tea's `Companion.suspend` clears the possession).
    if let Some(owner) = owner
        && state(map, g).is_some_and(|s| s.eyes.is_some())
    {
        super::eyes::look_away(map, owner, g);
    }
    let totem = state(map, g).map(|s| s.kind) == Some(Kind::Totem);
    if totem {
        totems::lift_auras(map, g);
        // Those who see it watch it sink, as mangos-classic's
        // `Totem::UnSummon` and Thistle Tea's totem show it go.
        if let Some(frame) =
            crate::game::conn::encode(&wow_world_messages::vanilla::SMSG_GAMEOBJECT_DESPAWN_ANIM {
                guid: g,
            })
        {
            map.send_to_observers(g, &frame, false);
        }
    }
    if let Some(owner) = owner
        && let Some(p) = map.players.get_mut(&owner)
    {
        let o = &mut p.owned;
        let was_pet = o.pet == Some(g);
        if was_pet {
            o.pet = None;
            p.fields.set_guid(UNIT_FIELD_SUMMON, Guid::zero());
        }
        if o.charm == Some(g) {
            o.charm = None;
        }
        if o.critter == Some(g) {
            o.critter = None;
        }
        o.guardians.retain(|x| *x != g);
        o.loose_totems.retain(|x| *x != g);
        for t in o.totems.iter_mut() {
            if *t == Some(g) {
                *t = None;
            }
        }
        if was_pet {
            p.outbound
                .send(&wow_world_messages::vanilla::SMSG_PET_SPELLS {
                    pet: Guid::zero(),
                    action_bars: None,
                });
        }
    }
    crate::game::map::combat::leave_combat(map, g);
    crate::game::map::ai::forget(map, g);
    map.dead_creatures.retain(|x| *x != g);
    map.remove_creature(g);
}

/// Put a pet away: a hunter's waits to be called again, a demon is gone,
/// and anything else just goes.
pub fn unsummon(map: &mut Map, pet: Guid, keep: bool) {
    let owner = owner_of(map, pet);
    let kind = state(map, pet).map(|s| s.kind);
    if let Some(owner) = owner {
        sync_record(map, owner);
        if let Some(p) = map.players.get_mut(&owner) {
            let pets = &mut p.row.state.pets;
            match kind {
                Some(Kind::Hunter) => pets.active = keep,
                Some(Kind::Demon) if !keep => {
                    pets.current = None;
                    pets.active = false;
                }
                _ => {}
            }
        }
    }
    remove(map, pet);
}

/// A pet put away with its reagents given back, as mangos's
/// `Unsummon(PET_SAVE_REAGENTS)` has it (at a talent reset): a hunter's
/// waits to be called again, a demon goes and its summoning's reagents
/// come back to its master (but in a battleground, where it comes back by
/// itself), a charmed unit is let go.
pub fn unsummon_refunding(map: &mut Map, pet: Guid) {
    let Some((kind, owner, spell)) = state(map, pet).map(|s| (s.kind, s.owner, s.summon_spell))
    else {
        return;
    };
    if matches!(kind, Kind::Charmed | Kind::Possessed) {
        release(map, owner, pet);
        return;
    }
    unsummon(map, pet, false);
    if kind != Kind::Demon || map.battleground.is_some() {
        return;
    }
    let data = map.realm.data.clone();
    let Some(s) = data.spells.get(spell) else {
        return;
    };
    for (item, count) in s.reagents.iter().filter(|(i, n)| *i != 0 && *n > 0) {
        let _ = crate::game::map::inventory::add_item(map, owner, *item, *count, true);
    }
}

/// The master dismisses `pet`: a hunter's goes to wait, a demon goes, a
/// charmed unit is let go.
pub fn dismiss(map: &mut Map, pet: Guid) {
    match state(map, pet).map(|s| (s.kind, s.owner)) {
        Some((Kind::Charmed | Kind::Possessed, owner)) => release(map, owner, pet),
        Some(_) => unsummon(map, pet, false),
        None => {}
    }
}

/// A controlled creature dies: it falls and stays a while.
pub fn die(map: &mut Map, g: Guid) {
    let now = map.now;
    crate::game::map::auras::clear_on_death(map, g);
    crate::game::map::combat::leave_combat(map, g);
    let spline = map.next_spline_id;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let moving = matches!(c.motion, Motion::Moving(_));
    c.motion = Motion::Idle { until: None };
    c.life = Life::Dead {
        corpse_until: now + Duration::from_secs(3600),
        respawn_at: now + Duration::from_secs(3600),
    };
    c.threat.clear();
    c.combat = Default::default();
    c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
    c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
    let pos = c.position;
    let kind = c.pet.as_deref().map(|s| s.kind);
    if let Some(s) = c.pet.as_deref_mut() {
        s.dead_at = Some(now);
    }
    if moving {
        map.next_spline_id = map.next_spline_id.wrapping_add(1);
        let frame = crate::game::map::ai::stop_packet(g, pos, spline);
        map.send_to_observers(g, &frame, false);
    }
    if kind == Some(Kind::Hunter) {
        hunter::on_death(map, g);
        if let Some(owner) = owner_of(map, g) {
            sync_record(map, owner);
        }
    } else if kind == Some(Kind::Demon)
        && let Some(owner) = owner_of(map, g)
        && let Some(p) = map.players.get_mut(&owner)
    {
        p.row.state.pets.current = None;
        p.row.state.pets.active = false;
    }
    if kind == Some(Kind::Totem) {
        totems::lift_auras(map, g);
    }
}

/// A dead pet's corpse goes: a hunter's waits, dead, to be revived.
pub fn corpse_gone(map: &mut Map, g: Guid) {
    if let Some(owner) = owner_of(map, g) {
        sync_record(map, owner);
    }
    remove(map, g);
}

/// Whether a player's pet must be away for now: mounted, dead, flying.
pub fn must_go(p: &Player) -> bool {
    p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID) != 0
        || p.death != crate::game::map::death::Death::Alive
        || p.flight.is_some()
}

/// A player's pet goes for a while, or comes back.
pub fn temporary(map: &mut Map, owner: Guid) {
    let Some(p) = map.players.get(&owner) else {
        return;
    };
    let away = must_go(p);
    match (p.owned.pet, p.owned.unsummoned) {
        (Some(pet), _) if away => {
            let alive = map.unit_view(pet).is_some_and(|u| u.alive);
            if !alive {
                return;
            }
            sync_record(map, owner);
            remove(map, pet);
            if let Some(p) = map.players.get_mut(&owner) {
                p.owned.unsummoned = true;
                p.row.state.pets.active = true;
            }
        }
        (None, true) if !away => {
            if let Some(p) = map.players.get_mut(&owner) {
                p.owned.unsummoned = false;
            }
            let r = map
                .players
                .get(&owner)
                .and_then(|p| p.row.state.pets.current.clone())
                .filter(|r| !r.dead);
            if let Some(r) = r {
                from_record(map, owner, &r);
            }
        }
        _ => {}
    }
}

/// Bring a dead pet up again where it is, at `fraction` of its health.
pub fn resurrect(map: &mut Map, g: Guid, fraction: f32) {
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.life = Life::Alive;
    let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    c.fields
        .set_u32(UNIT_FIELD_HEALTH, ((max as f32 * fraction) as u32).max(1));
    // Its regeneration, hunger and loyalty start their intervals afresh:
    // none fell due while it lay dead (Thistle Tea's `Regen.initialize`,
    // `PetLoyalty` and `PetHappiness` after a revival).
    if let Some(s) = c.pet.as_deref_mut() {
        s.dead_at = None;
        s.regen_at = now + Duration::from_secs(2);
        s.happiness_at = now + hunter::HAPPINESS_TICK;
        s.loyalty_at = now + hunter::LOYALTY_TICK;
    }
}
