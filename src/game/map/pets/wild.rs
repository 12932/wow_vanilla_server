//! What a creature's spell summons (effects 41 and 42): creatures of their
//! own, as Thistle Tea's wild summons and a creature's guardians are, with
//! their own EventAI and spell lists, and their summoner told of them (and
//! of their death and going). A wild summon is of its template's faction
//! and level, and dies at the spell's duration once out of a fight; a
//! guardian is of its summoner's, at its level less the effect's offset,
//! follows it, goes at the spell's duration once out of a fight or with
//! its summoner, and drops no loot. Script command 56 takes a unit's
//! guardians away.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::*;
use crate::game::map::creature::Creature;
use crate::game::map::event_ai;

/// Thistle Tea's caster-relative spots for a summon: implicit target, and
/// its angle from the caster's facing in half turns.
const SPOTS: [(u32, f32); 9] = [
    (32, 0.25),
    (41, 1.75),
    (42, 1.25),
    (43, 0.75),
    (44, 0.25),
    (47, 0.0),
    (48, 1.0),
    (49, 0.5),
    (50, -0.5),
];

/// The despawn types of a creature's summons: a guardian's time waits out
/// its fights (vmangos's timed-or-dead); a wild summon with no time lies
/// as a corpse when it dies (dead despawn).
const GUARDIAN_DESPAWN: i32 = 1;
const DEAD_DESPAWN: i32 = 7;
/// How far behind its summoner a guardian follows.
const GUARDIAN_FOLLOW: f32 = 2.0;

/// A guardian's cap: so many of one entry for one summoner.
const MOST_GUARDIANS: usize = 16;

/// Whether `caster`'s summons are creatures of their own: it is a creature
/// no player commands.
pub fn by_creature(map: &Map, caster: Guid) -> bool {
    map.creature(caster).is_some() && !guid::is_player(master(map, caster))
}

/// Where effect `index` of `s` puts what it summons, from `pos` facing
/// `o`: its spot's distance and angle, or (a wild summon with none) its
/// radius straight ahead.
fn spot(map: &Map, s: &SpellTemplate, index: usize, pos: Vector3d, o: f32) -> Vector3d {
    let e = s.effects[index];
    let radius = map.realm.data.spells.radius(e.radius_index);
    let angle = [e.target_a, e.target_b]
        .iter()
        .find_map(|t| SPOTS.iter().find(|(id, _)| id == t).map(|(_, a)| *a))
        .or((e.effect == EFFECT_SUMMON_WILD).then_some(0.0));
    let Some(angle) = angle else {
        return pos;
    };
    let a = o + angle * std::f32::consts::PI;
    Vector3d {
        x: pos.x + radius * a.cos(),
        y: pos.y + radius * a.sin(),
        z: pos.z,
    }
}

/// Where effect `index` of `s`, cast by `caster` at `t`, summons, if at a
/// place rather than a spot about the caster: the database's (17), the
/// caster's own (18), its target unit's (53, 63), or the place it was
/// aimed at (mangos's `EffectSummonWild` with `TARGET_FLAG_DEST_LOCATION`,
/// Thistle Tea's summon destination).
pub fn destination(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    t: &Targets,
) -> Option<Vector3d> {
    let e = s.effects[index];
    match e.target_a {
        17 => map
            .realm
            .data
            .spells
            .target_positions
            .get(&s.id)
            .filter(|p| p.0 == map.key.map)
            .map(|p| Vector3d {
                x: p.1,
                y: p.2,
                z: p.3,
            }),
        18 => map.position_of(caster),
        53 | 63 => t.unit.and_then(|u| map.position_of(u)).or(t.dest),
        a if SPOTS.iter().any(|(id, _)| *id == a || *id == e.target_b) => None,
        _ => t.dest,
    }
}

/// Effect `index` of `s`, cast by creature `caster`: `count` of `entry`,
/// the first at `at` (else at its spot about the caster) and the rest
/// about it within the effect's radius.
pub fn summon(
    map: &mut Map,
    caster: Guid,
    entry: u32,
    s: &SpellTemplate,
    index: usize,
    count: usize,
    at: Option<Vector3d>,
) {
    let data = map.realm.data.clone();
    let Some(t) = data.creature_template(entry) else {
        return;
    };
    let guardian = matches!(
        s.effects[index].effect,
        EFFECT_SUMMON_GUARDIAN | EFFECT_SUMMON_PET
    );
    if guardian && guardians_of(map, caster, entry).len() >= MOST_GUARDIANS {
        return;
    }
    let Some(view) = map.unit_view(caster) else {
        return;
    };
    let faction = map
        .unit_fields(caster)
        .map_or(t.faction, |f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE));
    let offset = s.effects[index].multiple_value;
    let level =
        if guardian && offset <= 0.0 && (1..=63).contains(&((view.level as f32 + offset) as i32)) {
            (view.level as f32 + offset) as u8
        } else if t.level_max > t.level_min {
            rand::rng().random_range(t.level_min..=t.level_max)
        } else {
            t.level_min
        }
        .max(1);
    let despawn = data
        .spells
        .duration(s, view.level)
        .filter(|ms| *ms > 0)
        .map(|ms| map.now + Duration::from_millis(u64::from(ms)));
    let base = at.unwrap_or_else(|| spot(map, s, index, view.position, view.orientation));
    let radius = data.spells.radius(s.effects[index].radius_index);
    for i in 0..count {
        // The rest about the first, within the effect's radius.
        let at = if i > 0 && radius > 0.0 {
            let (a, d) = {
                let mut rng = rand::rng();
                (
                    rng.random_range(0.0..std::f32::consts::TAU),
                    radius * rng.random_range(0.0f32..=1.0).sqrt(),
                )
            };
            Vector3d {
                x: base.x + d * a.cos(),
                y: base.y + d * a.sin(),
                z: base.z,
            }
        } else {
            base
        };
        let counter = map.next_creature_counter;
        map.next_creature_counter += 1;
        let mut c = Creature::new(
            guid::unit(entry, counter | 0x80_0000),
            t,
            level,
            at,
            view.orientation,
        );
        summon::creature_stats(&data, &mut c, t, level);
        // Its template's weapons, as mangos's `InitEntry` gives every
        // creature.
        if let Some(items) = data.creature_equipment.get(&t.equipment_id) {
            c.set_equipment(&data, *items);
        }
        c.fields.set_u32(UNIT_CREATED_BY_SPELL, s.id);
        let faction = if guardian {
            c.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
            c.fields.set_guid(UNIT_FIELD_SUMMONEDBY, caster);
            c.fields.set_guid(UNIT_FIELD_CREATEDBY, caster);
            faction
        } else {
            t.faction
        };
        c.hostile_to_players = crate::game::map::ai::hostile_to_players(&data, faction, t.civilian);
        let mut c = c.with_base();
        let scripted = c.scripted.get_or_insert_with(Default::default);
        scripted.summoner = Some(caster);
        scripted.despawn_at = despawn;
        if guardian {
            // It follows its summoner, as Thistle Tea's guardians do: two
            // yards off, each a twelfth of a turn on from the one before.
            let others = guardians_of(map, caster, 0).len() + i;
            let angle = (std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_6 * others as f32)
                % std::f32::consts::TAU;
            scripted.guardian = true;
            scripted.despawn_type = GUARDIAN_DESPAWN;
            scripted.follow = Some((caster, GUARDIAN_FOLLOW, angle));
        } else {
            // A wild summon's time ends in its death, as Thistle Tea's
            // `WildSummon` has it.
            scripted.despawn_type = if despawn.is_some() {
                event_ai::TIMED_DEATH
            } else {
                DEAD_DESPAWN
            };
        }
        let g = map.add_creature(c);
        if let Some(s) = event_ai::scripted_mut(map, caster) {
            s.summoned.push(g);
        }
    }
}

/// Whether `s` summons a guardian.
fn guardian_spell(map: &Map, spell: u32) -> bool {
    map.realm.data.spells.get(spell).is_some_and(|s| {
        s.effects
            .iter()
            .any(|e| matches!(e.effect, EFFECT_SUMMON_GUARDIAN | EFFECT_SUMMON_PET))
    })
}

/// The guardians of `owner` (of `entry`, or all for 0): a player's, a
/// creature's that follow it, and those its guardian spells made.
pub(super) fn guardians_of(map: &Map, owner: Guid, entry: u32) -> Vec<Guid> {
    let of_entry = |g: &Guid| entry == 0 || map.creature(*g).is_some_and(|c| c.entry == entry);
    if let Some(p) = map.players.get(&owner) {
        return p.owned.guardians.iter().copied().filter(of_entry).collect();
    }
    let mut out: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.owner == Some(owner) && c.pet.as_deref().is_some_and(|p| p.kind == Kind::Guardian)
        })
        .map(|(_, c)| c.guid)
        .filter(of_entry)
        .collect();
    let summoned = event_ai::scripted(map, owner)
        .map(|s| s.summoned.clone())
        .unwrap_or_default();
    out.extend(summoned.into_iter().filter(of_entry).filter(|g| {
        map.creature(*g).is_some_and(|c| {
            c.scripted.as_deref().and_then(|s| s.summoner) == Some(owner)
                && guardian_spell(map, c.fields.get_u32(UNIT_CREATED_BY_SPELL))
        })
    }));
    out
}

/// Script command 56: `owner`'s guardians (of `entry`, or all for 0) go.
pub fn remove_guardians(map: &mut Map, owner: Guid, entry: u32) {
    for g in guardians_of(map, owner, entry) {
        if map.creature(g).is_some_and(|c| c.pet.is_some()) {
            summon::remove(map, g);
        } else {
            crate::game::map::scripts::despawn(map, g);
        }
    }
}
