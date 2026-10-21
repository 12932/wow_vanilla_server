//! Game objects that come from spells, and what game objects do when used
//! or of their own accord, as Thistle Tea has them (its `GameObjectSummons`,
//! `Trap`, `Ritual`, `Goober` and spell caster objects, after vmangos).
//!
//! A spell's object (a mage's portal, a campfire, a hunter's trap, a
//! summoning portal) is its caster's: it bears their GUID and level, goes
//! when its time is up or its owner leaves, and a trap in a slot gives way
//! to the next in it. A trap springs its spell on an enemy that comes near
//! (or, one nobody set, on any player), as often as its charges and
//! cooldown allow. A ritual waits for its helpers, then casts its spell. A
//! quest object goes off only for a quest under way, shows its page or
//! menu, casts its spell and closes again.

use std::time::{Duration, Instant};

use ahash::AHashMap;
use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellTemplate;
use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::gameobject::GameObject;
use crate::game::map::spells::{Casting, Targets};

pub const GO_DOOR: u8 = 0;
pub const GO_BUTTON: u8 = 1;
pub const GO_CHEST: u8 = 3;
pub const GO_TRAP: u8 = 6;
pub const GO_SPELL_FOCUS: u8 = 8;
pub const GO_TEXT: u8 = 9;
pub const GO_GOOBER: u8 = 10;
pub const GO_RITUAL: u8 = 18;
pub const GO_SPELLCASTER: u8 = 22;

pub const EFFECT_TRANS_DOOR: u32 = 50;
pub const EFFECT_SUMMON_OBJECT_WILD: u32 = 76;
pub const EFFECT_SUMMON_PLAYER: u32 = 85;
pub const EFFECT_ACTIVATE_OBJECT: u32 = 86;
pub const EFFECT_SUMMON_OBJECT_SLOT1: u32 = 104;
pub const EFFECT_SUMMON_OBJECT_SLOT4: u32 = 107;

/// `GAMEOBJECT_FLAGS`: in use, locked, not to be used.
pub const GO_FLAG_IN_USE: u32 = 0x1;
pub const GO_FLAG_LOCKED: u32 = 0x2;
pub const GO_FLAG_NO_INTERACT: u32 = 0x10;
/// `GAMEOBJECT_STATE`: open (active), and closed (ready).
pub const GO_STATE_ACTIVE: u32 = 0;
pub const GO_STATE_READY: u32 = 1;
pub const GO_STATE_ACTIVE_ALTERNATIVE: u32 = 2;

/// How far a player may be from an object to use it.
const USE_YARDS: f32 = 10.0;
/// How often traps look for someone to spring on.
const TRAP_TICK: Duration = Duration::from_millis(200);
/// A trap's cooldown when its template names none.
const TRAP_COOLDOWN: Duration = Duration::from_secs(4);
/// A trans door spell's object with no time of its own lasts this long.
const DEFAULT_LIFETIME_MS: u32 = 8_000;
/// Traps that spring in a smaller circle than their template's.
const SMALL_TRAPS: [u32; 12] = [
    2561, 164_638, 164_639, 164_839, 164_872, 164_873, 164_874, 164_875, 164_876, 164_877, 164_879,
    164_880,
];
/// Quest objects that play an animation instead of opening.
const CUSTOM_DISPLAYS: [u32; 11] = [
    2570, 3071, 3072, 3073, 3074, 4392, 4472, 4491, 6785, 6747, 6871,
];

/// A map's summoned objects, traps, rituals and quest objects.
#[derive(Debug, Default)]
pub struct Objects {
    pub summoned: Vec<Summoned>,
    pub traps: AHashMap<Guid, Trap>,
    pub rituals: AHashMap<Guid, Ritual>,
    pub goobers: AHashMap<Guid, Goober>,
    /// Doors, buttons and quest objects that go back: (when, object,
    /// state).
    pub closes: Vec<(Instant, Guid, u32)>,
    /// Spawned objects used up, back at their time: (when, spawn).
    pub respawns: Vec<(Instant, u32)>,
    /// Used-up spawns a script brought out for a while (command 9), with
    /// when they were to come back: (object, spawn, when).
    pub brought_out: Vec<(Guid, u32, Instant)>,
    /// A quest object's spell with a cast time, landing, or a creature's
    /// on itself a while on (a lucky firework's Lunar Fortune): (when,
    /// object, spell, user).
    pub pending_spells: Vec<(Instant, Guid, u32, Guid)>,
    pub next_trap_look: Option<Instant>,
    /// A script's lock on an object, over its template's: unlocked
    /// (`false`, vmangos's object actions 6 and 9) or locked (`true`, 7 and
    /// 18), as Thistle Tea's `lock_override`.
    pub lock_overrides: AHashMap<Guid, bool>,
}

/// An object a spell summoned.
#[derive(Debug, Clone)]
pub struct Summoned {
    pub guid: Guid,
    pub owner: Option<Guid>,
    pub spell: u32,
    /// A summon slot (1 to 4: a hunter's traps); 0 for none.
    pub slot: u8,
    pub expires: Option<Instant>,
    /// The trap that came with it (a campfire's fire).
    pub linked: Vec<Guid>,
    /// A spell caster's uses left; `None` for no limit.
    pub charges: Option<u32>,
    /// Only its owner and their group may use it.
    pub party_only: bool,
}

/// A trap's state.
#[derive(Debug, Clone)]
pub struct Trap {
    pub owner: Option<Guid>,
    pub spell: u32,
    pub radius: f32,
    /// Springs left; 0 for no limit.
    pub charges: u32,
    pub cooldown: Duration,
    pub ready_at: Instant,
    pub stealthed: bool,
    pub depleted: bool,
}

/// A summoning ritual's state: its participants and whether it is done.
#[derive(Debug, Clone, Default)]
pub struct Ritual {
    pub owner: Option<Guid>,
    pub target: Option<Guid>,
    pub first: Option<Guid>,
    pub users: Vec<Guid>,
    pub required: usize,
    pub completion_spell: u32,
    pub animation_spell: u32,
    pub persistent: bool,
    pub caster_target_spell: u32,
    pub grouped: bool,
    pub completed: bool,
}

/// A quest object's state between uses.
#[derive(Debug, Clone, Default)]
pub struct Goober {
    pub ready_at: Option<Instant>,
    pub active_until: Option<Instant>,
    pub depleted: bool,
}

fn template(map: &Map, object: Guid) -> Option<GameObjectTemplate> {
    let entry = map.gameobject(object)?.entry;
    map.realm.data.gameobject_templates.get(&entry).cloned()
}

/// The trap a template's object springs when used or opened (vmangos'
/// `linkedTrapId`).
pub fn linked_entry(t: &GameObjectTemplate) -> u32 {
    match t.kind {
        GO_BUTTON => t.data[3],
        GO_CHEST => t.data[7],
        GO_SPELL_FOCUS => t.data[2],
        GO_GOOBER => t.data[12],
        _ => 0,
    }
}

fn trap_of(t: &GameObjectTemplate, owner: Option<Guid>, now: Instant) -> Trap {
    let radius = if SMALL_TRAPS.contains(&t.entry) {
        2.5
    } else {
        t.data[2] as f32
    };
    let cooldown = match t.data[5] {
        0 => TRAP_COOLDOWN,
        s => Duration::from_secs(u64::from(s)),
    };
    Trap {
        owner,
        spell: t.data[3],
        radius,
        charges: t.data[4],
        cooldown,
        ready_at: now + Duration::from_secs(u64::from(t.data[7])),
        stealthed: t.data[9] == 1,
        depleted: false,
    }
}

fn ritual_of(t: &GameObjectTemplate, owner: Option<Guid>, target: Option<Guid>) -> Ritual {
    Ritual {
        owner,
        target,
        first: None,
        users: owner.into_iter().collect(),
        required: (t.data[0] as usize).max(1),
        completion_spell: t.data[1],
        animation_spell: t.data[2],
        persistent: t.data[3] == 1,
        caster_target_spell: t.data[4],
        grouped: t.data[6] == 1,
        completed: false,
    }
}

/// Whether `a` and `b` are one player or in one group or raid.
pub fn same_group(map: &Map, a: Guid, b: Guid) -> bool {
    a == b
        || map
            .realm
            .social
            .groups
            .lock()
            .group_of(a)
            .is_some_and(|g| g.members.iter().any(|m| m.guid == b))
}

fn present(map: &Map, g: Guid) -> bool {
    map.players.contains_key(&g) || map.creature(g).is_some()
}

fn send(map: &Map, object: Guid, m: &impl wow_world_messages::vanilla::ServerMessage) {
    if let Some(frame) = crate::game::conn::encode(m) {
        map.send_to_observers(object, &frame, false);
    }
}

/// A caster-relative place's angle off the caster's facing, in half
/// turns: the minion's place, front right, back right, back left, front
/// left, front, back, left, right.
fn angle_of(target: u32) -> Option<f32> {
    Some(match target {
        32 => 0.25,
        41 => 1.75,
        42 => 1.25,
        43 => 0.75,
        44 => 0.25,
        47 => 0.0,
        48 => 1.0,
        49 => 0.5,
        50 => -0.5,
        _ => return None,
    })
}

/// Where an effect's object goes: at its caster-relative place, at the
/// cast's destination, or where the caster stands.
fn placed(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    index: usize,
    t: &Targets,
) -> Option<(Vector3d, f32)> {
    let me = map.unit_view(caster)?;
    let e = &s.effects[index];
    let o = me.orientation;
    if let Some(a) = angle_of(e.target_a).or_else(|| angle_of(e.target_b)) {
        let reach = map.realm.data.spells.radius(e.radius_index);
        let angle = o + a * std::f32::consts::PI;
        let at = Vector3d {
            x: me.position.x + reach * angle.cos(),
            y: me.position.y + reach * angle.sin(),
            z: me.position.z,
        };
        return Some((at, o));
    }
    Some((t.dest.unwrap_or(me.position), o))
}

/// Summon `entry` at `at`: `owner`'s if it has one (it bears their GUID
/// and level), for `ms` (0 for as long as its owner stays), in `slot` (the
/// owner's last in it goes). Its linked trap comes with it.
#[allow(clippy::too_many_arguments)]
pub fn summon(
    map: &mut Map,
    owner: Option<Guid>,
    entry: u32,
    at: Vector3d,
    orientation: f32,
    ms: u32,
    spell: u32,
    slot: u8,
    ritual_target: Option<Guid>,
) -> Option<Guid> {
    let data = map.realm.data.clone();
    // The slot's object goes first, whether or not the new one can be made
    // (mangos's `EffectSummonObject` clears the slot before `Create`).
    if slot != 0
        && let Some(o) = owner
    {
        let old: Vec<Guid> = map
            .objects
            .summoned
            .iter()
            .filter(|x| x.owner == Some(o) && x.slot == slot)
            .map(|x| x.guid)
            .collect();
        for g in old {
            despawn(map, g);
        }
    }
    let t = data.gameobject_templates.get(&entry)?;
    let g = spawn_object(map, t, owner, at, orientation);
    let now = map.now;
    let expires = (ms > 0).then(|| now + Duration::from_millis(u64::from(ms)));
    let mut linked = Vec::new();
    if let Some(lt) = data.gameobject_templates.get(&linked_entry(t)) {
        let l = spawn_object(map, lt, owner, at, orientation);
        register(map, lt, l, owner, ritual_target);
        map.objects.summoned.push(Summoned {
            guid: l,
            owner,
            spell,
            slot: 0,
            expires,
            linked: Vec::new(),
            charges: None,
            party_only: false,
        });
        linked.push(l);
    }
    register(map, t, g, owner, ritual_target);
    map.objects.summoned.push(Summoned {
        guid: g,
        owner,
        spell,
        slot,
        expires,
        linked,
        charges: (t.kind == GO_SPELLCASTER && t.data[1] > 0).then_some(t.data[1]),
        party_only: t.kind == GO_SPELLCASTER && t.data[2] != 0,
    });
    // A Ghost Magnet starts calling as it is set down.
    if entry == crate::game::ghost_magnet::MAGNET {
        crate::game::ghost_magnet::start(map, g);
    }
    Some(g)
}

fn spawn_object(
    map: &mut Map,
    t: &GameObjectTemplate,
    owner: Option<Guid>,
    at: Vector3d,
    orientation: f32,
) -> Guid {
    let counter = map.object_counter();
    let mut go = GameObject::from_spawn(
        t,
        &GameObjectSpawn {
            guid: counter,
            entry: t.entry,
            map: map.key.map,
            x: at.x,
            y: at.y,
            z: at.z,
            orientation,
            rotation: [
                0.0,
                0.0,
                (orientation / 2.0).sin(),
                (orientation / 2.0).cos(),
            ],
            state: GO_STATE_READY as u8,
            anim_progress: 100,
            ..Default::default()
        },
    );
    go.spawn_guid = None;
    if let Some(o) = owner {
        go.fields.set_guid(OBJECT_FIELD_CREATED_BY, o);
        let level = map.unit_view(o).map_or(1, |u| u.level);
        go.fields.set_u32(GAMEOBJECT_LEVEL, level);
    }
    map.add_gameobject(go)
}

fn register(
    map: &mut Map,
    t: &GameObjectTemplate,
    g: Guid,
    owner: Option<Guid>,
    ritual_target: Option<Guid>,
) {
    let now = map.now;
    match t.kind {
        GO_TRAP => {
            map.objects.traps.insert(g, trap_of(t, owner, now));
        }
        GO_RITUAL => {
            map.objects
                .rituals
                .insert(g, ritual_of(t, owner, ritual_target));
        }
        _ => {}
    }
}

/// An object goes: summoned, its linked trap with it and its ritual's
/// helpers' channels ended; spawned, until its respawn.
pub fn despawn(map: &mut Map, g: Guid) {
    let respawn = map
        .gameobject(g)
        .and_then(|o| o.spawn_guid)
        .and_then(|s| {
            map.realm
                .data
                .gameobject_spawns
                .get(&map.key.map)
                .and_then(|v| v.iter().find(|x| x.guid == s))
        })
        .map_or(0, |x| x.respawn_min);
    despawn_then(
        map,
        g,
        (respawn > 0).then(|| Duration::from_secs(u64::from(respawn))),
    );
}

/// An object goes as [`despawn`] has it, and a spawned one comes back
/// after `back` (with none, not until its map starts again).
pub fn despawn_then(map: &mut Map, g: Guid, back: Option<Duration>) {
    let Some(o) = map.gameobject(g) else {
        forget(map, g);
        return;
    };
    let (kind, spawn) = (o.kind, o.spawn_guid);
    let state = o.fields.get_u32(GAMEOBJECT_STATE);
    let summoned = map.objects.summoned.iter().position(|s| s.guid == g);
    if summoned.is_some() && (kind != GO_RITUAL || state == GO_STATE_READY) {
        send(map, g, &SMSG_GAMEOBJECT_DESPAWN_ANIM { guid: g });
    }
    let ritual = map.objects.rituals.get(&g).cloned();
    if let Some(r) = &ritual {
        for u in &r.users {
            finish_channel(map, *u, g);
        }
    }
    // A cooldown that waited for the object (the Ritual of Doom's) starts
    // as it goes, or, its ritual never done, there is none (Thistle Tea's
    // `GameObject.release_cooldown`).
    if let Some(s) = summoned.map(|i| &map.objects.summoned[i])
        && let Some(owner) = s.owner
    {
        let spell = s.spell;
        let done = ritual.is_none_or(|r| r.completed);
        crate::game::map::spells::cooldowns::release(map, owner, spell, done);
    }
    let linked = summoned
        .map(|i| map.objects.summoned.remove(i).linked)
        .unwrap_or_default();
    forget(map, g);
    map.remove_gameobject(g);
    for l in linked {
        despawn(map, l);
    }
    if let (Some(s), Some(back)) = (spawn, back) {
        // One return a spawn: this one's.
        map.objects.respawns.retain(|(_, x)| *x != s);
        map.objects.respawns.push((map.now + back, s));
    }
}

fn forget(map: &mut Map, g: Guid) {
    map.objects.traps.remove(&g);
    map.objects.rituals.remove(&g);
    map.objects.goobers.remove(&g);
    map.objects.closes.retain(|(_, o, _)| *o != g);
}

/// The effects that summon objects (50, 76, 104 to 107) and that work on
/// them (86), for `run_untargeted`; whether `index` was one.
pub fn effect(map: &mut Map, caster: Guid, s: &SpellTemplate, index: usize, t: &Targets) -> bool {
    let e = s.effects[index];
    let entry = e.misc_value.max(0) as u32;
    let level = map.unit_view(caster).map_or(1, |u| u.level);
    let lasts = map.realm.data.spells.duration(s, level).unwrap_or(0);
    match e.effect {
        EFFECT_TRANS_DOOR if entry != 0 => {
            let Some((at, o)) = placed(map, caster, s, index, t) else {
                return true;
            };
            let ms = if lasts > 0 {
                lasts
            } else {
                DEFAULT_LIFETIME_MS
            };
            // A ritual is for the caster's target.
            let target = map
                .unit_fields(caster)
                .map(|f| f.get_guid(UNIT_FIELD_TARGET))
                .filter(|g| !g.is_zero())
                .or(t.unit.filter(|u| *u != caster));
            let Some(g) = summon(map, Some(caster), entry, at, o, ms, s.id, 0, target) else {
                return true;
            };
            // A channel that raised it is on it: the warlock's on the
            // summoning portal.
            if s.channeled()
                && map
                    .caster(caster)
                    .is_some_and(|c| c.casting.as_ref().is_some_and(|x| x.spell == s.id))
                && let Some(f) = map.unit_fields_mut(caster)
            {
                f.set_guid(UNIT_FIELD_CHANNEL_OBJECT, g);
            }
            true
        }
        EFFECT_SUMMON_OBJECT_WILD if entry != 0 => {
            if let Some((at, o)) = placed(map, caster, s, index, t) {
                summon(map, None, entry, at, o, lasts, s.id, 0, None);
            }
            true
        }
        EFFECT_SUMMON_OBJECT_SLOT1..=EFFECT_SUMMON_OBJECT_SLOT4 if entry != 0 => {
            let slot = (e.effect - EFFECT_SUMMON_OBJECT_SLOT1 + 1) as u8;
            if let Some((at, o)) = placed(map, caster, s, index, t) {
                summon(map, Some(caster), entry, at, o, lasts, s.id, slot, None);
            }
            true
        }
        EFFECT_ACTIVATE_OBJECT => {
            for o in object_targets(map, caster, s, t, Some(index)) {
                activate(map, caster, o, e.misc_value.max(0) as u32);
            }
            true
        }
        _ => false,
    }
}

/// The Ritual of Summoning's target (Thistle Tea's `validate_ritual`): a
/// player of the caster's group, online and out of combat; from a dungeon,
/// one in it; nowhere from a battleground.
pub fn ritual_check(map: &Map, caster: Guid, s: &SpellTemplate) -> Option<u8> {
    use crate::game::map::spells::checks::fail;
    if s.script_name != "spell_warlock_ritual_of_summoning" {
        return None;
    }
    let target = map
        .unit_fields(caster)
        .map(|f| f.get_guid(UNIT_FIELD_TARGET))
        .unwrap_or_default();
    let in_combat = map.unit_fields(target).is_some_and(|f| {
        f.get_u32(UNIT_FIELD_FLAGS) & crate::game::map::player::UNIT_FLAG_IN_COMBAT != 0
    });
    let valid = guid::is_player(target)
        && target != caster
        && map.realm.registry.online(target).is_some()
        && same_group(map, caster, target);
    if in_combat {
        return Some(fail::TARGET_AFFECTING_COMBAT);
    }
    if !valid {
        return Some(fail::BAD_TARGETS);
    }
    let kind = crate::game::instances::instance_type(&map.realm, map.key.map);
    if matches!(kind, 1 | 2) && !map.players.contains_key(&target) {
        return Some(TARGET_NOT_IN_INSTANCE);
    }
    if kind == 3 {
        return Some(NOT_HERE);
    }
    None
}

/// `SpellCastResult`s the ritual's checks give.
const NOT_HERE: u8 = 0x35;
const TARGET_NOT_IN_INSTANCE: u8 = 0x7f;

/// An object's lock (Thistle Tea's `GameObjectTemplate.lock_id`).
pub fn lock_of(t: &GameObjectTemplate) -> u32 {
    match t.kind {
        GO_DOOR | GO_BUTTON => t.data[1],
        25 => t.data[4],
        2 | GO_CHEST | GO_TRAP | GO_GOOBER | 12 | 13 | 24 | 26 => t.data[0],
        _ => 0,
    }
}

/// Whether `player`'s hand opens `object` if it is a door or a button, as
/// Thistle Tea's `Gathering.authorize_use` has it (the user's #7): one with
/// a lock wants one of the lock's item keys carried (worn, in the bags or
/// on the keyring), unless a script has unlocked it; one a script locked
/// that has no lock of its own opens to no hand. mangos trusts the client.
pub fn hand_opens(map: &Map, player: Guid, object: Guid) -> bool {
    let Some(t) = template(map, object) else {
        return true;
    };
    if !matches!(t.kind, GO_DOOR | GO_BUTTON) {
        return true;
    }
    let lock = lock_of(&t);
    match map.objects.lock_overrides.get(&object) {
        Some(false) => true,
        Some(true) if lock == 0 => false,
        _ if lock == 0 => true,
        _ => {
            let Some(p) = map.players.get(&player) else {
                return false;
            };
            map.realm.data.crafts.locks.get(&lock).is_some_and(|keys| {
                keys.iter().any(|k| {
                    k.kind == crate::game::data::crafts::LOCK_KEY_ITEM
                        && k.index != 0
                        && p.carried_count_of(k.index) > 0
                })
            })
        }
    }
}

/// An object whose lock is opened by attacking it (a battleground's
/// banner: a skill key of lock type 17): a viewer's state starts fresh.
pub fn resets_on_sight(map: &Map, object: Guid) -> bool {
    let data = &map.realm.data;
    let Some(t) = map
        .gameobject(object)
        .and_then(|o| data.gameobject_templates.get(&o.entry))
    else {
        return false;
    };
    data.crafts.locks.get(&lock_of(t)).is_some_and(|keys| {
        keys.iter()
            .any(|k| k.kind == crate::game::data::crafts::LOCK_KEY_SKILL && k.index == 17)
    })
}

/// A spell that opens locks: no quest credit for what it opens (Thistle
/// Tea's `queue_quest_cast_credit`).
pub fn opens_lock(s: &SpellTemplate) -> bool {
    s.effects.iter().any(|e| matches!(e.effect, 33 | 59))
}

/// The objects a cast is at: the one aimed at, and those its effect 86
/// works.
pub fn cast_at(map: &Map, caster: Guid, s: &SpellTemplate, t: &Targets) -> Vec<Guid> {
    let mut out: Vec<Guid> = t
        .object
        .filter(|o| map.gameobject(*o).is_some())
        .into_iter()
        .collect();
    if s.effects.iter().any(|e| e.effect == EFFECT_ACTIVATE_OBJECT) {
        for o in object_targets(map, caster, s, t, None) {
            if !out.contains(&o) {
                out.push(o);
            }
        }
    }
    out
}

/// The objects a spell (its effect `index`, if one is named) works on: the
/// one aimed at, or the nearest its script targets name for that effect.
fn object_targets(
    map: &Map,
    caster: Guid,
    s: &SpellTemplate,
    t: &Targets,
    index: Option<usize>,
) -> Vec<Guid> {
    let named = |o: &GameObject| {
        crate::game::map::spells::targets::script_target_of(
            map,
            caster,
            s.id,
            &[0],
            o.entry,
            o.guid,
            index,
        )
    };
    // The object aimed at, unless a condition its row puts on it fails.
    if let Some(o) = t.object.and_then(|o| map.gameobject(o)) {
        let barred = map
            .realm
            .data
            .spells
            .script_target_conditions
            .contains_key(&(s.id, 0, o.entry))
            && !named(o);
        if !barred {
            return vec![o.guid];
        }
    }
    let Some(at) = map.position_of(caster) else {
        return Vec::new();
    };
    let reach = map.realm.data.spells.range(s).1.max(30.0);
    map.grid
        .around(at.x, at.y, reach)
        .filter(|g| guid::is_game_object(*g))
        .filter_map(|g| map.gameobject(g))
        .filter(|o| named(o) && dist(o.position, at) <= reach)
        .min_by(|a, b| dist(a.position, at).total_cmp(&dist(b.position, at)))
        .map(|o| o.guid)
        .into_iter()
        .collect()
}

/// Effect 86 on an object, by its action (vmangos'
/// `GameObjectActions`): used, an animation, locked or unlocked, reset,
/// gone, made usable or not.
pub fn activate(map: &mut Map, user: Guid, object: Guid, action: u32) {
    match action {
        5 | 8 => used_by_script(map, user, object),
        1..=4 => send(
            map,
            object,
            &SMSG_GAMEOBJECT_CUSTOM_ANIM {
                guid: object,
                animation_id: action - 1,
            },
        ),
        6 => script_lock(map, object, false),
        7 => script_lock(map, object, true),
        9 => {
            open(map, object, false, 0);
            script_lock(map, object, false);
        }
        10 | 13 => reset(map, object),
        12 => open(map, object, true, 0),
        15 => despawn(map, object),
        16 => set_flag(map, object, GO_FLAG_NO_INTERACT, true),
        17 => set_flag(map, object, GO_FLAG_NO_INTERACT, false),
        18 => {
            reset(map, object);
            script_lock(map, object, true);
        }
        _ => {}
    }
}

/// A script locks or unlocks `object`: its flag, and the lock a hand meets
/// ([`hand_opens`]).
fn script_lock(map: &mut Map, object: Guid, locked: bool) {
    set_flag(map, object, GO_FLAG_LOCKED, locked);
    if map.gameobject(object).is_some() {
        map.objects.lock_overrides.insert(object, locked);
    }
}

/// A script (or a spell) works an object: a trap springs on the one who
/// did, a door or button is used (its scripts run on its user), a quest
/// object goes off, anything else flips.
pub fn used_by_script(map: &mut Map, user: Guid, object: Guid) {
    let Some(t) = template(map, object) else {
        return;
    };
    match t.kind {
        GO_TRAP if map.position_of(user).is_some() => {
            ensure_trap(map, object, &t);
            spring(map, object, user);
        }
        GO_TRAP => {}
        GO_DOOR | GO_BUTTON => {
            door(map, user, object, &t);
            crate::game::map::scripts::object_scripts(map, user, object);
        }
        GO_GOOBER if map.players.contains_key(&user) => {
            use_goober(map, user, object);
        }
        GO_GOOBER => goober(map, user, object, &t, true),
        _ => {
            let state = map
                .gameobject(object)
                .map_or(GO_STATE_READY, |o| o.fields.get_u32(GAMEOBJECT_STATE));
            set_state(
                map,
                object,
                if state == GO_STATE_ACTIVE {
                    GO_STATE_READY
                } else {
                    GO_STATE_ACTIVE
                },
            );
        }
    }
}

fn set_flag(map: &mut Map, object: Guid, flag: u32, on: bool) {
    if let Some(o) = map.gameobject_mut(object) {
        o.fields.set_flag(GAMEOBJECT_FLAGS, flag, on);
    }
}

fn set_state(map: &mut Map, object: Guid, state: u32) {
    if let Some(o) = map.gameobject_mut(object) {
        o.fields.set_u32(GAMEOBJECT_STATE, state);
    }
}

/// The state an object rests in: its spawn's.
pub fn resting_state(map: &Map, object: Guid) -> u32 {
    let Some(o) = map.gameobject(object) else {
        return GO_STATE_READY;
    };
    o.spawn_guid
        .and_then(|s| {
            map.realm
                .data
                .gameobject_spawns
                .get(&map.key.map)
                .and_then(|v| v.iter().find(|x| x.guid == s))
                .map(|x| u32::from(x.state))
        })
        .unwrap_or(GO_STATE_READY)
}

/// Opened (or the alternative state), in use, and back after `close_ms`.
fn open(map: &mut Map, object: Guid, alternative: bool, close_ms: u32) {
    let Some(o) = map.gameobject(object) else {
        return;
    };
    let state = o.fields.get_u32(GAMEOBJECT_STATE);
    let back = resting_state(map, object);
    let next = if state == GO_STATE_READY {
        if alternative {
            GO_STATE_ACTIVE_ALTERNATIVE
        } else {
            GO_STATE_ACTIVE
        }
    } else {
        GO_STATE_READY
    };
    set_state(map, object, next);
    set_flag(map, object, GO_FLAG_IN_USE, true);
    // A close still to come is an older use's: this state is newer
    // (Thistle Tea's revision).
    forget_close(map, object);
    if close_ms > 0 {
        let at = map.now + Duration::from_millis(u64::from(close_ms));
        map.objects.closes.push((at, object, back));
    }
}

/// An object's close still to come forgotten: a newer state stands.
pub fn forget_close(map: &mut Map, object: Guid) {
    map.objects.closes.retain(|(_, o, _)| *o != object);
    // A script's state back after a while too; not an object's going.
    map.object_resets
        .retain(|(_, o, state)| *o != object || *state == u32::MAX);
}

/// An object back as it rests, no longer in use, its close forgotten (a
/// quest object's use over, its cooldown left).
pub fn reset(map: &mut Map, object: Guid) {
    let back = resting_state(map, object);
    set_state(map, object, back);
    set_flag(map, object, GO_FLAG_IN_USE, false);
    forget_close(map, object);
    if let Some(g) = map.objects.goobers.get_mut(&object) {
        g.active_until = None;
    }
}

/// A door or button used: its linked trap springs on the user, and it
/// opens (or shuts) until its time to go back; not again while it is.
/// A door or button opened by its lock: as one used by hand ([`door`]).
pub(crate) fn door_used(map: &mut Map, user: Guid, object: Guid) {
    if let Some(t) = template(map, object) {
        door(map, user, object, &t);
    }
}

fn door(map: &mut Map, user: Guid, object: Guid, t: &GameObjectTemplate) {
    if map.objects.closes.iter().any(|(_, o, _)| *o == object) {
        return;
    }
    spring_linked(map, user, object, t);
    let close_ms = (t.data[2] / 0x10000) * 1000;
    open(map, object, false, close_ms);
    if close_ms == 0 {
        set_flag(map, object, GO_FLAG_IN_USE, false);
    }
}

/// The trap linked to an object, sprung on `user`: a summoned object's
/// own, or a spawned one's nearest of its entry.
fn spring_linked(map: &mut Map, user: Guid, object: Guid, t: &GameObjectTemplate) {
    let entry = linked_entry(t);
    if entry == 0 {
        return;
    }
    let linked = map
        .objects
        .summoned
        .iter()
        .find(|s| s.guid == object)
        .and_then(|s| s.linked.first().copied())
        .or_else(|| {
            let at = map.gameobject(object)?.position;
            map.grid
                .around(at.x, at.y, 10.0)
                .filter(|g| guid::is_game_object(*g) && *g != object)
                .filter_map(|g| map.gameobject(g))
                .filter(|o| o.entry == entry && dist(o.position, at) < 10.0)
                .min_by(|a, b| dist(a.position, at).total_cmp(&dist(b.position, at)))
                .map(|o| o.guid)
        });
    if let Some(l) = linked
        && let Some(lt) = template(map, l)
    {
        ensure_trap(map, l, &lt);
        spring(map, l, user);
    }
}

fn ensure_trap(map: &mut Map, g: Guid, t: &GameObjectTemplate) {
    let now = map.now;
    if !map.objects.traps.contains_key(&g) {
        let owner = map
            .gameobject(g)
            .map(|o| o.fields.get_guid(OBJECT_FIELD_CREATED_BY))
            .filter(|o| !o.is_zero());
        map.objects.traps.insert(g, trap_of(t, owner, now));
    }
}

/// A trap's spell on `target`, if it is armed and ready: its owner casts
/// it, or, a trap nobody set, it lands as the target's own; its charges
/// and cooldown follow.
fn spring(map: &mut Map, g: Guid, target: Guid) {
    let now = map.now;
    let Some(trap) = map.objects.traps.get(&g).cloned() else {
        return;
    };
    if trap.depleted || now < trap.ready_at {
        return;
    }
    let Some(at) = map.gameobject(g).map(|o| o.position) else {
        return;
    };
    let caster = trap.owner.filter(|o| present(map, *o)).unwrap_or(target);
    if trap.spell != 0 {
        crate::game::map::spells::cast_triggered_with(
            map,
            caster,
            trap.spell,
            Targets {
                unit: Some(target),
                dest: Some(at),
                ..Default::default()
            },
        );
    }
    let Some(t) = map.objects.traps.get_mut(&g) else {
        return;
    };
    t.ready_at = now + t.cooldown;
    match t.charges {
        0 => {}
        1 => {
            t.depleted = true;
            despawn(map, g);
        }
        n => t.charges = n - 1,
    }
}

/// A trap disarmed: it springs no more, and goes until it comes back at
/// its spawn's time.
pub fn disarm(map: &mut Map, g: Guid) {
    if let Some(t) = map.objects.traps.get_mut(&g) {
        t.depleted = true;
    }
    despawn(map, g);
}

/// Whom a trap springs on: an enemy of its owner's (one that may be
/// attacked, and is hostile or fighting), or, nobody's, any living player.
fn trap_target(map: &Map, g: Guid, trap: &Trap) -> Option<Guid> {
    if trap.radius <= 0.0 {
        return None;
    }
    let at = map.gameobject(g)?.position;
    // Living, within the radius. (Only whether it lives: a unit's view
    // would read its fields too.)
    let within = |u: &Guid| {
        map.position_of(*u)
            .is_some_and(|p| dist(p, at) <= trap.radius)
            && match map.players.get(u) {
                Some(p) => p.death == crate::game::map::death::Death::Alive,
                None => map.creature(*u).is_some_and(|c| c.alive()),
            }
    };
    let near = map.grid.around(at.x, at.y, trap.radius);
    match trap.owner {
        // Players only, so only players are looked at.
        None => near.filter(|u| guid::is_player(*u)).filter(within).min(),
        Some(owner) => near
            .filter(|u| guid::is_player(*u) || guid::is_unit(*u))
            .filter(within)
            .filter(|u| *u != owner)
            .filter(|u| {
                crate::game::map::combat::can_attack(map, owner, *u)
                    && (crate::game::map::combat::hostile(map, owner, *u)
                        || map
                            .unit_fields(*u)
                            .is_some_and(|f| f.get_u32(UNIT_FIELD_FLAGS) & 0x8_0000 != 0))
            })
            .min(),
    }
}

/// Whether `observer` sees `object`: a stealthed trap hides from those its
/// owner (or it, nobody's) is hostile to, but those who detect traps.
pub fn seen_by(map: &Map, observer: Guid, object: Guid) -> bool {
    let Some(o) = map.gameobject(object) else {
        return true;
    };
    if o.kind != GO_TRAP {
        return true;
    }
    let stealthed = map.objects.traps.get(&object).map_or_else(
        || {
            map.realm
                .data
                .gameobject_templates
                .get(&o.entry)
                .is_some_and(|t| t.data[9] == 1)
        },
        |t| t.stealthed,
    );
    if !stealthed {
        return true;
    }
    let owner = o.fields.get_guid(OBJECT_FIELD_CREATED_BY);
    let hostile = if owner.is_zero() {
        let data = &map.realm.data;
        let faction = o.fields.get_u32(GAMEOBJECT_FACTION);
        match (
            data.faction_templates.get(&faction),
            map.unit_view(observer)
                .and_then(|u| data.faction_templates.get(&u.faction)),
        ) {
            (Some(a), Some(b)) => a.hostile_to(b),
            _ => true,
        }
    } else {
        owner != observer
            && (crate::game::map::combat::hostile(map, owner, observer)
                || crate::game::map::combat::can_attack(map, owner, observer))
    };
    !hostile
        || crate::game::map::auras::auras(map, observer).is_some_and(|a| {
            a.effects().any(|(_, d)| {
                d.kind == crate::game::map::auras::AURA_MOD_INVISIBILITY_DETECTION && d.misc == 3
            })
        })
}

/// `CMSG_GAMEOBJ_USE` on a quest object (a goober): handled here, scripts
/// and all, as it runs them only when it goes off. Whether it was one.
pub fn use_goober(map: &mut Map, player: Guid, object: Guid) -> bool {
    let Some(t) = template(map, object) else {
        return false;
    };
    if t.kind != GO_GOOBER {
        return false;
    }
    if !in_reach(map, player, object)
        || !requirement_met(map, object)
        || !prepare_use(map, player, object)
    {
        return true;
    }
    goober(map, player, object, &t, false);
    true
}

/// A quest giver, a page or a quest object about to be used (Thistle
/// Tea's `GameObjectInteraction.prepare_*_use`): not by one under a school
/// immunity (Divine Shield) if its template says so, as mangos's
/// `CannotBeUsedUnderImmunity` has it; else the user's auras a use breaks
/// (`AURA_INTERRUPT_FLAG_LOOTING`, 0x800) go. Whether it may be used.
pub fn prepare_use(map: &mut Map, user: Guid, object: Guid) -> bool {
    let Some(t) = template(map, object) else {
        return true;
    };
    let no_damage_immune = match t.kind {
        crate::game::map::world::GO_QUESTGIVER => t.data[5],
        GO_GOOBER => t.data[11],
        GO_TEXT => 0,
        _ => return true,
    };
    let immune = crate::game::map::auras::auras(map, user)
        .is_some_and(|a| a.has(crate::game::map::auras::AURA_SCHOOL_IMMUNITY));
    if no_damage_immune != 0 && immune {
        return false;
    }
    crate::game::map::auras::interrupt(map, user, crate::game::map::auras::INTERRUPT_USE);
    true
}

/// A quest object used (Thistle Tea's `Goober.use`): nothing while it is
/// in use, cooling down or not to be used; only its page or menu for one
/// not on its quest; else it opens (or plays its animation), springs its
/// trap, runs its scripts, casts its spell, shows its page or menu,
/// credits the quests it is for, and closes again, used up if it is
/// spent with a use.
fn goober(map: &mut Map, user: Guid, object: Guid, t: &GameObjectTemplate, scripted: bool) {
    let now = map.now;
    let flags = map
        .gameobject(object)
        .map_or(0, |o| o.fields.get_u32(GAMEOBJECT_FLAGS));
    let state = map
        .objects
        .goobers
        .get(&object)
        .cloned()
        .unwrap_or_default();
    if flags & GO_FLAG_NO_INTERACT != 0
        || state.depleted
        || state.active_until.is_some_and(|u| u > now)
        || state.ready_at.is_some_and(|r| now < r)
    {
        return;
    }
    let quest = t.data[1];
    let allowed = scripted
        || quest == 0
        || !map.realm.data.npc.quests.contains_key(&quest)
        || map.players.get(&user).is_some_and(|p| {
            p.journal.slot_of(quest).is_some_and(|s| {
                let q = &map.realm.data.npc.quests[&quest];
                !crate::game::map::quests::complete(p, q, &p.journal.log[s])
            })
        });
    let cooldown = Duration::from_secs(u64::from(t.data[6]));
    if !allowed {
        map.objects.goobers.entry(object).or_default().ready_at = Some(now + cooldown);
        show_readable(map, user, object, t);
        return;
    }
    let auto_close = (t.data[3] / 0x10000) * 1000;
    let custom = CUSTOM_DISPLAYS.contains(&t.display_id) || (auto_close > 0 && t.data[4] != 0);
    let back = resting_state(map, object);
    if custom {
        set_flag(map, object, GO_FLAG_IN_USE, true);
        send(
            map,
            object,
            &SMSG_GAMEOBJECT_CUSTOM_ANIM {
                guid: object,
                animation_id: 0,
            },
        );
    } else {
        open(map, object, false, 0);
    }
    let close = auto_close.max(1000);
    let g = map.objects.goobers.entry(object).or_default();
    g.ready_at = Some(now + Duration::from_millis(u64::from(auto_close)));
    g.active_until = Some(now + Duration::from_millis(u64::from(close)));
    map.objects.closes.retain(|(_, o, _)| *o != object);
    map.objects
        .closes
        .push((now + Duration::from_millis(u64::from(close)), object, back));
    spring_linked(map, user, object, t);
    if map.players.contains_key(&user) {
        crate::game::map::scripts::object_used(map, user, object);
    }
    let spell = t.data[10];
    if spell != 0
        && let Some(s) = map.realm.data.spells.get(spell)
    {
        let delay = map.realm.data.spells.cast_time(s);
        if delay > 0 {
            map.objects.pending_spells.push((
                now + Duration::from_millis(u64::from(delay)),
                object,
                spell,
                user,
            ));
        } else {
            crate::game::map::spells::cast_triggered(map, user, spell, Some(user));
        }
    }
    if map.players.contains_key(&user) {
        show_readable(map, user, object, t);
        crate::game::map::quests::object_used(map, user, object, true);
    }
}

/// A quest object's page, or its menu.
fn show_readable(map: &mut Map, user: Guid, object: Guid, t: &GameObjectTemplate) {
    if t.data[7] > 0 {
        if let Some(p) = map.players.get(&user) {
            p.outbound.send(&SMSG_GAMEOBJECT_PAGETEXT { guid: object });
        }
    } else if t.data[19] > 0 && map.realm.data.npc.gossip_menus.contains_key(&t.data[19]) {
        crate::game::map::npc::show_menu(map, user, object, t.data[19], 0);
    }
}

fn in_reach(map: &Map, player: Guid, object: Guid) -> bool {
    map.players
        .get(&player)
        .is_some_and(|p| p.death == crate::game::map::death::Death::Alive)
        && within_reach(map, player, object)
}

/// How near an object must be to use it (Thistle Tea's interaction
/// radius).
pub const INTERACT_YARDS: f32 = 5.555_56;

/// Whether `player` may reach `object` to use it: within the interaction
/// radius of its model's box, turned and scaled as it is (Thistle Tea's
/// `GameObjectInteraction.within?`), or of where it stands without one.
pub fn within_reach(map: &Map, player: Guid, object: Guid) -> bool {
    let (Some(p), Some(o)) = (map.players.get(&player), map.gameobject(object)) else {
        return false;
    };
    let mut q = [0.0f32; 4];
    for (i, v) in q.iter_mut().enumerate() {
        *v = o.fields.get_f32(GAMEOBJECT_ROTATION + i as u16);
    }
    let scale = o.fields.get_f32(OBJECT_FIELD_SCALE_X);
    let bounds = map
        .realm
        .data
        .gameobject_templates
        .get(&o.entry)
        .and_then(|t| t.bounds);
    within(
        p.position(),
        o.position,
        rotation(q, o.orientation),
        scale,
        bounds,
        INTERACT_YARDS,
    )
}

/// An object's rotation: its quaternion, or, with no z and w, one turned
/// by its facing (Thistle Tea's `GameObjectInteraction.rotation`).
pub fn rotation(q: [f32; 4], orientation: f32) -> [f32; 4] {
    if q[2] == 0.0 && q[3] == 0.0 {
        [
            q[0],
            q[1],
            (orientation / 2.0).sin(),
            (orientation / 2.0).cos(),
        ]
    } else {
        q
    }
}

/// Whether `at` is within `r` of an object at `origin`: of its model's
/// box, scaled (a scale of none is 1) and turned by `rotation`, the radius
/// unscaled; of its origin, in three dimensions, with no box or one of
/// nothing but zeroes (Thistle Tea's `GameObjectInteraction.within?`).
pub fn within(
    at: Vector3d,
    origin: Vector3d,
    rotation: [f32; 4],
    scale: f32,
    bounds: Option<([f32; 3], [f32; 3])>,
    r: f32,
) -> bool {
    let d = (at.x - origin.x, at.y - origin.y, at.z - origin.z);
    let bounds = bounds.filter(|(lo, hi)| lo.iter().chain(hi.iter()).any(|v| *v != 0.0));
    let Some((lo, hi)) = bounds else {
        return d.0 * d.0 + d.1 * d.1 + d.2 * d.2 <= r * r;
    };
    let (x, y, z) = inverse_rotate(d, rotation);
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let inside = |v: f32, l: f32, h: f32| v >= l * scale - r && v <= h * scale + r;
    inside(x, lo[0], hi[0]) && inside(y, lo[1], hi[1]) && inside(z, lo[2], hi[2])
}

/// A point turned back by a rotation's quaternion.
fn inverse_rotate((x, y, z): (f32, f32, f32), [qx, qy, qz, qw]: [f32; 4]) -> (f32, f32, f32) {
    let norm = qx * qx + qy * qy + qz * qz + qw * qw;
    if norm <= 0.0 {
        return (x, y, z);
    }
    let tx = 2.0 * (qz * y - qy * z) / norm;
    let ty = 2.0 * (qx * z - qz * x) / norm;
    let tz = 2.0 * (qy * x - qx * y) / norm;
    (
        x + qw * tx + qz * ty - qy * tz,
        y + qw * ty + qx * tz - qz * tx,
        z + qw * tz + qy * tx - qx * ty,
    )
}

/// Whether `player` is near enough to use `object`: the interaction reach
/// for the objects Thistle Tea measures (doors, buttons, quest givers,
/// pages, quest objects, rituals, spell casters, meeting stones), ten
/// yards for the rest.
pub fn in_use_range(map: &Map, player: Guid, object: Guid) -> bool {
    let Some(o) = map.gameobject(object) else {
        return false;
    };
    if matches!(
        o.kind,
        GO_DOOR | GO_BUTTON | 2 | GO_TEXT | GO_GOOBER | GO_RITUAL | GO_SPELLCASTER | 23
    ) {
        return within_reach(map, player, object);
    }
    map.players
        .get(&player)
        .is_some_and(|p| dist(p.position(), o.position) <= USE_YARDS)
}

/// vmangos' `gameobject_requirement`: a spawn used only once another is
/// dead (a creature) or open (an object). A spawn not loaded doesn't
/// block.
pub fn requirement_met(map: &Map, object: Guid) -> bool {
    let Some(s) = map.gameobject(object).and_then(|o| o.spawn_guid) else {
        return true;
    };
    let Some(&(kind, other)) = map.realm.data.object_requirements.get(&s) else {
        return true;
    };
    match kind {
        0 => map
            .by_spawn
            .get(&other)
            .and_then(|g| map.creature(*g))
            .is_none_or(|c| !c.alive()),
        1 => map
            .gameobjects
            .iter()
            .find(|(_, o)| o.spawn_guid == Some(other))
            .is_none_or(|(_, o)| o.fields.get_u32(GAMEOBJECT_STATE) == GO_STATE_ACTIVE),
        _ => true,
    }
}

/// `CMSG_GAMEOBJ_USE` on the objects this module runs: doors and buttons,
/// traps, pages, rituals and spell casters. Whether it was handled; for
/// the rest, the quests asking for its use are credited.
pub fn use_object(map: &mut Map, player: Guid, object: Guid) -> bool {
    let Some(t) = template(map, object) else {
        return false;
    };
    if !requirement_met(map, object) {
        return true;
    }
    if t.kind != crate::game::map::world::GO_QUESTGIVER {
        crate::game::map::quests::object_used(map, player, object, false);
    }
    if map
        .gameobject(object)
        .is_some_and(|o| o.fields.get_u32(GAMEOBJECT_FLAGS) & GO_FLAG_NO_INTERACT != 0)
        && matches!(t.kind, GO_DOOR | GO_BUTTON | GO_TRAP | GO_RITUAL)
    {
        return true;
    }
    match t.kind {
        GO_DOOR | GO_BUTTON => door(map, player, object, &t),
        GO_TRAP => {
            ensure_trap(map, object, &t);
            spring(map, object, player);
        }
        GO_TEXT => {
            if t.data[0] > 0
                && let Some(p) = map.players.get(&player)
            {
                p.outbound.send(&SMSG_GAMEOBJECT_PAGETEXT { guid: object });
            }
        }
        GO_RITUAL => use_ritual(map, player, object, &t),
        GO_SPELLCASTER => use_spellcaster(map, player, object, &t),
        _ => return false,
    }
    true
}

/// A spell caster used (a mage's portal): its spell on the user, if they
/// may (a party's portal is its owner's group's), a charge spent.
fn use_spellcaster(map: &mut Map, user: Guid, object: Guid, t: &GameObjectTemplate) {
    let summoned = map
        .objects
        .summoned
        .iter()
        .find(|s| s.guid == object)
        .cloned();
    if let Some(s) = &summoned
        && s.party_only
        && let Some(owner) = s.owner
        && !same_group(map, owner, user)
    {
        return;
    }
    if t.data[0] != 0 {
        crate::game::map::spells::cast_triggered(map, user, t.data[0], Some(user));
    }
    let Some(i) = map.objects.summoned.iter().position(|s| s.guid == object) else {
        return;
    };
    if let Some(n) = map.objects.summoned[i].charges {
        if n <= 1 {
            despawn(map, object);
        } else {
            map.objects.summoned[i].charges = Some(n - 1);
        }
    }
}

/// A ritual used (Thistle Tea's `Ritual.use`): once each, not its owner,
/// and, a grouped one, only by its owner's group (or its first user's). A
/// helper channels its animation; with enough, it is done.
fn use_ritual(map: &mut Map, user: Guid, object: Guid, t: &GameObjectTemplate) {
    let r = map
        .objects
        .rituals
        .entry(object)
        .or_insert_with(|| ritual_of(t, None, None))
        .clone();
    if r.completed || r.owner == Some(user) || r.users.contains(&user) {
        return;
    }
    let anchor = r.owner.or(r.first);
    if r.grouped
        && let Some(a) = anchor
        && !same_group(map, a, user)
    {
        return;
    }
    let done = {
        let r = map.objects.rituals.get_mut(&object).expect("just made");
        r.users.push(user);
        r.first = r.first.or(Some(user));
        r.completed = r.users.len() >= r.required;
        r.completed
    };
    if r.animation_spell != 0 {
        let now = map.now;
        let ms = map
            .objects
            .summoned
            .iter()
            .find(|s| s.guid == object)
            .and_then(|s| s.expires)
            .map(|e| e.saturating_duration_since(now).as_millis() as u32)
            .filter(|ms| *ms > 0)
            .or_else(|| {
                let s = map.realm.data.spells.get(r.animation_spell)?;
                map.realm.data.spells.duration(s, 1)
            })
            .unwrap_or(0);
        if ms > 0 {
            start_channel(map, user, object, r.animation_spell, ms);
        }
    }
    if done {
        complete_ritual(map, object);
    }
}

/// A ritual done: its owner casts its spell on its target (the summoning
/// portal's summons), or, nobody's, its first user casts it; one of its
/// helpers takes its spell for them (the Ritual of Doom's sacrifice); the
/// channels end; and it goes unless it stays for the next.
fn complete_ritual(map: &mut Map, object: Guid) {
    let Some(r) = map.objects.rituals.get(&object).cloned() else {
        return;
    };
    let at = map.gameobject(object).map(|o| o.position);
    if r.completion_spell != 0 {
        match r.owner {
            Some(owner) => crate::game::map::spells::cast_triggered_with(
                map,
                owner,
                r.completion_spell,
                Targets {
                    unit: r.target.or(Some(owner)),
                    dest: at,
                    ..Default::default()
                },
            ),
            None => {
                if let Some(first) = r.first {
                    crate::game::map::spells::cast_triggered_with(
                        map,
                        first,
                        r.completion_spell,
                        Targets {
                            unit: Some(first),
                            object: Some(object),
                            dest: at,
                            ..Default::default()
                        },
                    );
                }
            }
        }
    }
    if r.caster_target_spell != 0 && !r.users.is_empty() {
        let who = r.users[rand::rng().random_range(0..r.users.len())];
        crate::game::map::spells::cast_triggered(map, who, r.caster_target_spell, Some(who));
    }
    for u in &r.users {
        finish_channel(map, *u, object);
    }
    if r.persistent {
        if r.owner.is_none()
            && let Some(x) = map.objects.rituals.get_mut(&object)
        {
            x.users.clear();
            x.first = None;
            x.completed = false;
        }
    } else {
        despawn(map, object);
    }
}

/// A helper channels a ritual's animation at it.
fn start_channel(map: &mut Map, g: Guid, object: Guid, spell: u32, ms: u32) {
    let now = map.now;
    let until = now + Duration::from_millis(u64::from(ms));
    if let Some(c) = map.caster_mut(g) {
        c.casting = Some(Casting {
            spell,
            targets: Targets {
                object: Some(object),
                ..Default::default()
            },
            ends: until,
            item: None,
            pushbacks: 0,
            channel_until: Some(until),
            duration: ms,
        });
    }
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_u32(UNIT_CHANNEL_SPELL, spell);
        f.set_guid(UNIT_FIELD_CHANNEL_OBJECT, object);
    }
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&MSG_CHANNEL_START_Server {
            spell,
            duration: ms,
        });
    }
}

/// A channel at `object` ends as it should: done, not cut short.
fn finish_channel(map: &mut Map, g: Guid, object: Guid) {
    let on = map
        .unit_fields(g)
        .is_some_and(|f| f.get_guid(UNIT_FIELD_CHANNEL_OBJECT) == object);
    if !on {
        return;
    }
    if let Some(c) = map.caster_mut(g)
        && c.casting
            .as_ref()
            .is_some_and(|x| x.channel_until.is_some())
    {
        c.casting = None;
    }
    if let Some(f) = map.unit_fields_mut(g) {
        f.set_u32(UNIT_CHANNEL_SPELL, 0);
        f.set_guid(UNIT_FIELD_CHANNEL_OBJECT, Guid::zero());
    }
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&MSG_CHANNEL_UPDATE_Server { time: 0 });
    }
}

/// A unit's channel ended before its ritual was done: an owner's portal
/// goes; a helper leaves (a persistent ritual short of helpers may be
/// done again), and a placed ritual everyone has left is led by whoever
/// comes next (Thistle Tea's `Ritual.leave`).
pub fn channel_ended(map: &mut Map, g: Guid, spell: u32) {
    let owned: Vec<Guid> = map
        .objects
        .summoned
        .iter()
        .filter(|s| s.owner == Some(g) && s.spell == spell)
        .map(|s| s.guid)
        .filter(|o| map.objects.rituals.get(o).is_some_and(|r| !r.completed))
        .collect();
    for o in owned {
        despawn(map, o);
    }
    for r in map.objects.rituals.values_mut() {
        if r.animation_spell == spell && r.owner != Some(g) && r.users.contains(&g) {
            r.users.retain(|u| *u != g);
            if r.completed && r.persistent && r.users.len() < r.required {
                r.completed = false;
            }
            if r.owner.is_none() && r.users.is_empty() {
                r.first = None;
            }
        }
    }
}

/// Every update: summoned objects whose time is up or whose owner left
/// go; doors and quest objects close; used-up spawns come back; quest
/// objects' spells land; and, every so often, traps look about them.
pub fn update(map: &mut Map) {
    let now = map.now;
    let gone: Vec<Guid> = map
        .objects
        .summoned
        .iter()
        .filter(|s| {
            s.expires.is_some_and(|e| e <= now) || s.owner.is_some_and(|o| !present(map, o))
        })
        .map(|s| s.guid)
        .collect();
    for g in gone {
        despawn(map, g);
    }
    if !map.objects.closes.is_empty() {
        let due: Vec<(Instant, Guid, u32)> = map
            .objects
            .closes
            .iter()
            .copied()
            .filter(|(t, _, _)| *t <= now)
            .collect();
        map.objects.closes.retain(|(t, _, _)| *t > now);
        for (_, g, state) in due {
            set_state(map, g, state);
            set_flag(map, g, GO_FLAG_IN_USE, false);
            // A quest object spent with a use, or a summoned one, goes.
            let spent = template(map, g).is_some_and(|t| {
                t.kind == GO_GOOBER
                    && (t.data[5] != 0
                        || map
                            .gameobject(g)
                            .is_some_and(|o| !o.fields.get_guid(OBJECT_FIELD_CREATED_BY).is_zero()))
            });
            if spent {
                if let Some(x) = map.objects.goobers.get_mut(&g) {
                    x.depleted = true;
                }
                despawn(map, g);
            }
        }
    }
    if !map.objects.pending_spells.is_empty() {
        let due: Vec<(Instant, Guid, u32, Guid)> = map
            .objects
            .pending_spells
            .iter()
            .copied()
            .filter(|(t, ..)| *t <= now)
            .collect();
        map.objects.pending_spells.retain(|(t, ..)| *t > now);
        for (_, _, spell, user) in due {
            if present(map, user) {
                crate::game::map::spells::cast_triggered(map, user, spell, Some(user));
            }
        }
    }
    if !map.objects.respawns.is_empty() {
        let due: Vec<u32> = map
            .objects
            .respawns
            .iter()
            .filter(|(t, _)| *t <= now)
            .map(|(_, s)| *s)
            .collect();
        map.objects.respawns.retain(|(t, _)| *t > now);
        let data = map.realm.data.clone();
        for s in due {
            // Out already (an event's, a pool's).
            if crate::game::map::scripts::object_of_spawn(map, s).is_some() {
                continue;
            }
            if let Some(spawn) = data
                .gameobject_spawns
                .get(&map.key.map)
                .and_then(|v| v.iter().find(|x| x.guid == s))
                && let Some(t) = data.gameobject_templates.get(&spawn.entry)
            {
                map.add_gameobject(GameObject::from_spawn(t, spawn));
            }
        }
    }
    if map.objects.next_trap_look.is_some_and(|t| t > now) || map.players.is_empty() {
        return;
    }
    map.objects.next_trap_look = Some(now + TRAP_TICK);
    let data = map.realm.data.clone();
    let traps: Vec<(Guid, u32)> = map
        .gameobjects
        .iter()
        .filter(|(_, o)| o.kind == GO_TRAP)
        .map(|(_, o)| (o.guid, o.entry))
        .collect();
    for (g, entry) in traps {
        if !map.objects.traps.contains_key(&g)
            && let Some(t) = data.gameobject_templates.get(&entry)
        {
            ensure_trap(map, g, t);
        }
        let Some(trap) = map.objects.traps.get(&g).cloned() else {
            continue;
        };
        if trap.depleted || now < trap.ready_at {
            continue;
        }
        if let Some(target) = trap_target(map, g, &trap) {
            spring(map, g, target);
        }
    }
}
