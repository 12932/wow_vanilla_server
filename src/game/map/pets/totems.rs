//! Totems, guardians, companions and summoned eyes. A totem stands where
//! its shaman put it, one of each element, and works its spell: an aura it
//! holds over the shaman's party near it, a pulse it sends out, or bolts at
//! enemies in range. A guardian fights at its summoner's side for a while;
//! a companion follows; an eye is the summoner's to move.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::*;
use crate::game::data::spells::SpellTemplate;
use crate::game::map::combat::dist;

const EFFECT_APPLY_AREA_AURA_PARTY: u32 = 35;
/// How often a totem's aura reaches out again.
const AURA_EVERY: Duration = Duration::from_millis(1000);
/// How often an attacking totem casts.
const BOLT_EVERY: Duration = Duration::from_millis(1500);

/// The spell a totem of `entry` works: its `totem_spell_id`, or its spell
/// list, or its summon spell's trigger.
fn totem_spell(map: &Map, entry: u32, s: &SpellTemplate) -> u32 {
    let data = &map.realm.data;
    if let Some(spell) = data.pets.totem_spells.get(&entry) {
        return *spell;
    }
    if let Some(list) = data
        .creature_template(entry)
        .and_then(|t| data.spells.creature_spells.get(&t.spell_list_id))
        && let Some(first) = list.first()
    {
        return first.spell;
    }
    data.pets
        .creature_spell_list
        .get(&entry)
        .and_then(|l| data.pets.spell_lists.get(l))
        .and_then(|l| l.first().copied())
        .or_else(|| {
            s.effects
                .iter()
                .find(|e| e.trigger_spell != 0)
                .map(|e| e.trigger_spell)
        })
        .unwrap_or(0)
}

/// Where something summoned at a spot near `owner` stands: `d` yards
/// before them.
fn before(map: &mut Map, owner: Guid, d: f32) -> (Vector3d, f32) {
    let spot = before_spot(map, owner, d);
    clip_about(map, owner, spot)
}

fn before_spot(map: &Map, owner: Guid, d: f32) -> (Vector3d, f32) {
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
    (
        Vector3d {
            x: pos.x + d * o.cos(),
            y: pos.y + d * o.sin(),
            z: pos.z,
        },
        o,
    )
}

/// A player's guardian's level: its template's, as near its owner's as
/// that allows; one an engineering trinket calls, its owner's engineering
/// over five (mangos-classic's `EffectSummonGuardian`, Thistle Tea's
/// `GuardianLoader.level`).
fn guardian_level(map: &Map, owner: Guid, entry: u32, item: Option<Guid>) -> u8 {
    const ENGINEERING: u32 = 202;
    const TRINKET: u32 = 12;
    let data = &map.realm.data;
    let t = summon::template(data, entry);
    let own = level_of(map, owner);
    let level = own.clamp(t.level_min.max(1), t.level_max.max(t.level_min).max(1));
    let Some(p) = map.players.get(&owner) else {
        return level;
    };
    let trinket = item
        .and_then(|g| p.item_by_guid(g))
        .and_then(|i| data.item(p.items[i].entry))
        .is_some_and(|it| it.required_skill == ENGINEERING && it.inventory_type == TRINKET);
    let skill = crate::game::map::skills::value_with_bonuses(p, ENGINEERING);
    if trinket && skill > 0 {
        (skill / 5).clamp(1, 63) as u8
    } else {
        level
    }
}

fn level_of(map: &Map, g: Guid) -> u8 {
    map.unit_view(g).map(|u| u.level).unwrap_or(1).clamp(1, 63) as u8
}

/// Where a totem of `slot` stands: two yards out, each element on its own
/// side (Thistle Tea's `Totems.position`).
fn slot_spot(map: &mut Map, owner: Guid, slot: usize) -> (Vector3d, f32) {
    let spot = slot_spot_about(map, owner, slot);
    clip_about(map, owner, spot)
}

fn slot_spot_about(map: &Map, owner: Guid, slot: usize) -> (Vector3d, f32) {
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
    let a = o + std::f32::consts::FRAC_PI_4 - slot as f32 * std::f32::consts::FRAC_PI_2;
    (
        Vector3d {
            x: pos.x + 2.0 * a.cos(),
            y: pos.y + 2.0 * a.sin(),
            z: pos.z,
        },
        o,
    )
}

/// Where a totem of no slot stands: two yards in front of its owner.
fn front_spot(map: &mut Map, owner: Guid) -> (Vector3d, f32) {
    let spot = front_spot_about(map, owner);
    clip_about(map, owner, spot)
}

fn front_spot_about(map: &Map, owner: Guid) -> (Vector3d, f32) {
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
    (
        Vector3d {
            x: pos.x + 2.0 * o.cos(),
            y: pos.y + 2.0 * o.sin(),
            z: pos.z,
        },
        o,
    )
}

/// Effects a totem shrugs off (Thistle Tea's `Totems.immune_effect?`): heals
/// and power, taunts, and harmful or healing auras, but its shaman's own
/// and the shaman's totem spells.
pub fn immune(map: &Map, caster: Guid, target: Guid, s: &SpellTemplate, index: usize) -> bool {
    if caster == target || !state(map, target).is_some_and(|t| t.kind == Kind::Totem) {
        return false;
    }
    if s.family_name == 11 && s.family_flags as u32 & 0x0400_6000 != 0 {
        return false;
    }
    let e = s.effects[index];
    const REGEN_AURAS: [u32; 6] = [8, 24, 20, 21, 84, 85];
    matches!(e.effect, 114 | 10 | 67 | 75 | 30)
        || (matches!(e.effect, 6 | 27 | 35)
            && (!crate::game::map::auras::positive(s) || REGEN_AURAS.contains(&e.aura)))
}

/// A totem of `entry` for `owner`, in `slot` (the one there goes); one of
/// no slot (`SUMMON_TOTEM`: a standard, a ward) stands in front and takes
/// no element's place, as mangos's `TOTEM_SLOT_NONE`.
pub fn summon_totem(
    map: &mut Map,
    owner: Guid,
    entry: u32,
    s: &SpellTemplate,
    slot: Option<usize>,
) {
    let now = map.now;
    let slot = slot.map(|s| s.min(3));
    if let Some(old) = slot.and_then(|slot| map.players.get(&owner)?.owned.totems[slot]) {
        summon::remove(map, old);
    }
    let (at, o) = match slot {
        Some(slot) => slot_spot(map, owner, slot),
        None => front_spot(map, owner),
    };
    let level = level_of(map, owner);
    let g = summon::spawn(map, owner, entry, Kind::Totem, level, at, o);
    let spell = totem_spell(map, entry, s);
    let data = map.realm.data.clone();
    // Its time, with its shaman's modifiers (mangos's
    // `CalculateSpellDuration` in `EffectSummonTotem`: Improved Fire Nova
    // Totem's sooner blast).
    let mods = crate::game::map::spells::mods::for_spell(map, owner, s);
    let duration = data
        .spells
        .duration(s, u32::from(level))
        .map(|ms| mods.apply_ms(crate::game::map::spells::mods::OP_DURATION, ms));
    // Its party aura's reach with its shaman's radius modifiers, as they
    // are now: mangos's `AreaAura` takes `SPELLMOD_RADIUS` from its
    // caster's modifier owner once, as the totem casts it (Thistle Tea's
    // snapshot at `Totems.prepare`).
    let radius = data.spells.get(spell).map_or(0.0, |t| {
        let m = crate::game::map::spells::mods::for_spell(map, owner, t);
        (0..3)
            .filter(|i| t.effects[*i].effect == EFFECT_APPLY_AREA_AURA_PARTY)
            .map(|i| crate::game::map::spells::targets::radius(map, t, i, &m))
            .fold(0.0f32, f32::max)
    });
    if let Some(c) = map.creature_mut(g) {
        c.speeds.run = 0.0;
        c.speeds.walk = 0.0;
        c.base.run_speed = 0.0;
        c.fields.set_u32(UNIT_CREATED_BY_SPELL, s.id);
        // A totem has the few points of health its summon spell gives.
        let health = (s
            .effects
            .iter()
            .find(|e| {
                (EFFECT_SUMMON_TOTEM_SLOT1..=EFFECT_SUMMON_TOTEM_SLOT4).contains(&e.effect)
                    || e.effect == EFFECT_SUMMON_TOTEM
            })
            .map_or(5, |e| e.base_points + e.base_dice))
        .max(1) as u32;
        c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
        c.fields.set_u32(UNIT_FIELD_HEALTH, health);
        c.base.max_health = health;
        if let Some(st) = c.pet.as_deref_mut() {
            st.totem_slot = slot;
            st.totem_spell = spell;
            st.totem_radius = radius;
            st.summon_spell = s.id;
            st.react = REACT_PASSIVE;
            st.expires = duration.map(|ms| now + Duration::from_millis(u64::from(ms)));
            st.next_think = now;
        }
    }
    if let Some(p) = map.players.get_mut(&owner) {
        match slot {
            Some(slot) => p.owned.totems[slot] = Some(g),
            None => p.owned.loose_totems.push(g),
        }
    }
    // A creature's totem is its summon: it hears of it as it comes, dies
    // and goes, as of its guardians (mangos-classic's `Totem`, a
    // `TemporarySummon`: `JustSummoned`, `SummonedCreatureJustDied` and
    // `SummonedCreatureDespawn`; Thistle Tea's summon events), where it
    // heard of none.
    if guid::is_unit(owner) && map.creature(owner).is_some() {
        if let Some(c) = map.creature_mut(g) {
            c.scripted.get_or_insert_with(Default::default).summoner = Some(owner);
        }
        if let Some(s) = crate::game::map::event_ai::scripted_mut(map, owner) {
            s.summoned.push(g);
        }
        if !map.ai_spawned.contains(&g) {
            crate::game::map::event_ai::spawned(map, g);
        }
    }
    // Its shaman's spell modifiers from the start, for the spell it works
    // at once (Improved Fire Nova Totem's quicker pulse), as Thistle Tea's
    // totem takes `owner_spell_modifiers` as it is made.
    inherit::sync(map, g);
    // A totem whose spell lasts on it (a pulse, a ward) has it from the
    // start.
    if let Some(t) = data.spells.get(spell)
        && t.has_effect(6)
        && !t.has_effect(EFFECT_APPLY_AREA_AURA_PARTY)
        && crate::game::map::auras::positive(t)
    {
        crate::game::map::spells::cast_triggered(map, g, spell, Some(g));
    }
}

/// Whether a pulse of unit `g`'s auras due at `due` goes: a totem's goes
/// only while its shaman is here and alive, and not one due after its time
/// (mangos's `Totem::Update` unsummons it before its auras tick; Thistle
/// Tea's totem lifetime keeps a pulse due by its end). Any other unit's
/// goes.
pub fn pulses(map: &Map, g: Guid, due: Option<Instant>) -> bool {
    let Some(s) = state(map, g).filter(|s| s.kind == Kind::Totem) else {
        return true;
    };
    map.unit_view(s.owner).is_some_and(|u| u.alive)
        && match (s.expires, due) {
            (Some(end), Some(t)) => t <= end,
            _ => true,
        }
}

/// Destroy Totems: every totem of the shaman's goes.
pub fn destroy_all(map: &mut Map, owner: Guid) {
    let totems: Vec<Guid> = map
        .players
        .get(&owner)
        .map(|p| p.owned.totems.iter().flatten().copied().collect())
        .unwrap_or_default();
    for t in totems {
        summon::remove(map, t);
    }
}

/// The party around a totem: its shaman and their group, in the map and
/// within `radius`.
fn party_near(map: &Map, owner: Guid, at: Vector3d, radius: f32) -> Vec<Guid> {
    let members: Vec<Guid> = {
        let groups = map.realm.social.groups.lock();
        match groups.group_of(owner) {
            Some(g) => g.members.iter().map(|m| m.guid).collect(),
            None => vec![owner],
        }
    };
    members
        .into_iter()
        .filter(|g| {
            map.players.get(g).is_some_and(|p| {
                p.death == crate::game::map::death::Death::Alive && dist(p.position(), at) <= radius
            })
        })
        .collect()
}

/// A totem's turn: its aura over the party near, or a bolt at an enemy.
pub fn step(map: &mut Map, g: Guid, now: Instant) {
    let Some(s) = state(map, g) else {
        return;
    };
    if s.next_think > now {
        return;
    }
    let (owner, spell, kept) = (s.owner, s.totem_spell, s.totem_radius);
    let data = map.realm.data.clone();
    let Some(t) = data.spells.get(spell) else {
        if let Some(s) = state_mut(map, g) {
            s.next_think = now + Duration::from_secs(3600);
        }
        return;
    };
    let Some(at) = map.position_of(g) else {
        return;
    };
    if t.has_effect(EFFECT_APPLY_AREA_AURA_PARTY) {
        if let Some(s) = state_mut(map, g) {
            s.next_think = now + AURA_EVERY;
        }
        let mut radius = kept;
        if radius <= 0.0 {
            for e in t
                .effects
                .iter()
                .filter(|e| e.effect == EFFECT_APPLY_AREA_AURA_PARTY)
            {
                radius = radius.max(data.spells.radius(e.radius_index));
            }
        }
        let radius = if radius > 0.0 { radius } else { 30.0 };
        let near = party_near(map, owner, at, radius);
        // Those near have it; those gone off don't.
        let had: Vec<Guid> = map
            .players
            .values()
            .filter(|p| {
                p.auras
                    .holders
                    .iter()
                    .any(|h| h.spell == spell && h.caster == g)
            })
            .map(|p| p.guid)
            .collect();
        for p in had.iter().filter(|p| !near.contains(p)) {
            crate::game::map::auras::remove_spell(map, *p, spell, Some(g));
        }
        let level = level_of(map, g);
        let effects: Vec<(usize, i32)> = (0..3)
            .filter(|i| {
                t.effects[*i].effect == EFFECT_APPLY_AREA_AURA_PARTY && t.effects[*i].aura != 0
            })
            .map(|i| (i, crate::game::map::spells::points(t, i, u32::from(level))))
            .collect();
        for p in near.into_iter().filter(|p| !had.contains(p)) {
            let _ = crate::game::map::auras::apply_spell(
                map,
                g,
                p,
                t,
                &effects,
                crate::game::map::auras::Context {
                    duration: Some(None),
                    ..Default::default()
                },
            );
        }
        return;
    }
    if crate::game::map::auras::positive(t) {
        if let Some(s) = state_mut(map, g) {
            s.next_think = now + Duration::from_secs(3600);
        }
        return;
    }
    // An attacking totem: a bolt at its shaman's foe, or the nearest.
    if let Some(s) = state_mut(map, g) {
        s.next_think = now + BOLT_EVERY;
    }
    if map.caster(g).is_some_and(|c| c.casting.is_some()) {
        return;
    }
    let (_, range) = data.spells.range(t);
    let range = if range > 0.0 { range } else { 20.0 };
    let target = map
        .combat(owner)
        .filter(|c| c.attacking)
        .map(|c| c.victim)
        .into_iter()
        .chain(
            map.combat(owner)
                .map(|c| c.attackers.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default(),
        )
        .find(|v| {
            map.unit_view(*v)
                .is_some_and(|u| u.alive && dist(u.position, at) <= range)
                && crate::game::map::combat::can_attack(map, g, *v)
        });
    if let Some(v) = target {
        crate::game::map::spells::cast(
            map,
            g,
            spell,
            Targets {
                unit: Some(v),
                ..Default::default()
            },
            None,
        );
    }
}

/// A totem's auras come off those it gave them to.
pub fn lift_auras(map: &mut Map, g: Guid) {
    let holding: Vec<(Guid, u32)> = map
        .players
        .values()
        .flat_map(|p| {
            p.auras
                .holders
                .iter()
                .filter(|h| h.caster == g)
                .map(|h| (p.guid, h.spell))
                .collect::<Vec<_>>()
        })
        .collect();
    for (p, spell) in holding {
        crate::game::map::auras::remove_spell(map, p, spell, Some(g));
    }
}

/// A guardian of `entry` for `owner`, for the spell's time, at `at` (else
/// before its owner): it fights what its summoner fights.
pub fn summon_guardian(
    map: &mut Map,
    owner: Guid,
    entry: u32,
    s: &SpellTemplate,
    replace: bool,
    at: Option<Vector3d>,
    item: Option<Guid>,
) -> Option<Guid> {
    if entry == 0 {
        return None;
    }
    // A player's cast replaces their guardians of its kind (Thistle Tea's
    // `Guardians.prepare`); a creature's add up, to sixteen.
    let same: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.owner == Some(owner)
                && c.entry == entry
                && c.pet.as_deref().is_some_and(|p| p.kind == Kind::Guardian)
        })
        .map(|(_, c)| c.guid)
        .collect();
    if guid::is_player(owner) {
        if replace {
            for g in same {
                summon::remove(map, g);
            }
        }
    } else if same.len() >= 16 {
        return None;
    }
    let now = map.now;
    let data = map.realm.data.clone();
    let level = if guid::is_player(owner) {
        guardian_level(map, owner, entry, item)
    } else {
        data.creature_template(entry)
            .map_or(1, |t| t.level_max.max(1))
    };
    let (beside, o) = before(map, owner, 2.0);
    let g = summon::spawn(
        map,
        owner,
        entry,
        Kind::Guardian,
        level,
        at.unwrap_or(beside),
        o,
    );
    let duration = data.spells.duration(s, u32::from(level));
    if let Some(c) = map.creature_mut(g) {
        c.fields.set_u32(UNIT_CREATED_BY_SPELL, s.id);
        if let Some(st) = c.pet.as_deref_mut() {
            st.summon_spell = s.id;
            st.react = REACT_AGGRESSIVE;
            st.expires = duration.map(|ms| now + Duration::from_millis(u64::from(ms)));
            st.angle = std::f32::consts::PI * (0.5 + rand::random::<f32>());
        }
    }
    if let Some(p) = map.players.get_mut(&owner) {
        p.owned.guardians.push(g);
    }
    crate::game::map::item_scripts::guardian_summoned(map, g, entry);
    Some(g)
}

/// Summon `g` lasts `time` from now.
pub fn lasts(map: &mut Map, g: Guid, time: Duration) {
    let now = map.now;
    if let Some(st) = map.creature_mut(g).and_then(|c| c.pet.as_deref_mut()) {
        st.expires = Some(now + time);
    }
}

/// A companion: it follows, and fights nothing. Summoning the one out
/// sends it away. Its level is its template's, not its owner's, and it is
/// immune to players and creatures both, over its template's flags
/// (mangos's `EffectSummonCritter`, Thistle Tea's `build_mini_pet`).
pub fn summon_critter(map: &mut Map, owner: Guid, entry: u32, s: &SpellTemplate) {
    let old = map.players.get(&owner).and_then(|p| p.owned.critter);
    if let Some(old) = old {
        let same = map.creature(old).is_some_and(|c| c.entry == entry);
        summon::remove(map, old);
        if same {
            return;
        }
    }
    let t = summon::template(&map.realm.data, entry);
    let level = if t.level_max > t.level_min {
        rand::RngExt::random_range(&mut rand::rng(), t.level_min..=t.level_max)
    } else {
        t.level_min
    };
    // Two yards off its owner's front left, facing them, whatever the
    // spell's destination (Thistle Tea's `MiniPet`; mangos-classic's
    // minion position has no radius, and puts it on its owner).
    let (at, o) = {
        let (pos, o) = map
            .unit_view(owner)
            .map_or((Vector3d::default(), 0.0), |u| (u.position, u.orientation));
        let a = o + std::f32::consts::FRAC_PI_4;
        (
            Vector3d {
                x: pos.x + 2.0 * a.cos(),
                y: pos.y + 2.0 * a.sin(),
                z: pos.z,
            },
            (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU),
        )
    };
    let (at, o) = clip_about(map, owner, (at, o));
    let now = map.now;
    let duration = map
        .realm
        .data
        .spells
        .duration(s, u32::from(level_of(map, owner)));
    let g = summon::spawn(map, owner, entry, Kind::Critter, level.max(1), at, o);
    if let Some(c) = map.creature_mut(g) {
        c.fields.set_u32(UNIT_CREATED_BY_SPELL, s.id);
        let flags = c.fields.get_u32(UNIT_FIELD_FLAGS)
            | t.unit_flags
            | crate::game::map::combat::UNIT_FLAG_IMMUNE_TO_PLAYER
            | crate::game::map::combat::UNIT_FLAG_IMMUNE_TO_NPC;
        c.fields.set_u32(UNIT_FIELD_FLAGS, flags);
        if let Some(st) = c.pet.as_deref_mut() {
            st.summon_spell = s.id;
            st.react = REACT_PASSIVE;
            st.angle = std::f32::consts::PI;
            // A spell with a time calls it for that long (mangos's
            // `SetDuration(m_duration)`).
            st.expires = duration.map(|ms| now + Duration::from_millis(u64::from(ms)));
        }
    }
    if let Some(p) = map.players.get_mut(&owner) {
        p.owned.critter = Some(g);
    }
}

/// A summoned eye (Eye of Kilrogg): its summoner sees and moves through it
/// while the channel lasts.
pub fn summon_possessed(
    map: &mut Map,
    owner: Guid,
    entry: u32,
    s: &SpellTemplate,
    at: Option<Vector3d>,
) {
    if !guid::is_player(owner) || entry == 0 {
        return;
    }
    let level = level_of(map, owner);
    let (beside, o) = before(map, owner, 2.0);
    let at = at.unwrap_or(beside);
    let g = summon::spawn(map, owner, entry, Kind::Guardian, level, at, o);
    if let Some(c) = map.creature_mut(g) {
        c.fields.set_u32(UNIT_CREATED_BY_SPELL, s.id);
        c.pet = None;
        c.owner = None;
    }
    charmed(map, owner, g, true);
    if let Some(st) = state_mut(map, g) {
        st.summon_spell = s.id;
    }
}
