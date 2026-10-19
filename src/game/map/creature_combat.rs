//! Creatures in a fight, beyond the swing, as Thistle Tea runs them:
//!
//! - the leash ([`should_evade`]): a creature fights on while near where
//!   the fight began, or while its victim is, or while blows still come;
//!   beyond its template's hard leash it always goes home;
//! - a victim it can't reach for 24 seconds makes it evade;
//! - fleeing: for help to the nearest of its kind, or in fear; fear and
//!   confusion move it about;
//! - regeneration: a third of its health and mana each five seconds out of
//!   a fight, health in one while polymorphed, and mana by its spirit;
//! - a dungeon's boss fighting the whole dungeon at once;
//! - its template's mechanic and school immunities.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;

/// The leash's reach around where the fight began, by default
/// (`[creatures] leash_radius`).
pub const THREAT_RADIUS: f32 = 30.0;
/// Without a hostile act this long, a creature beyond its threat radius
/// goes home, by default (`[creatures] leash_timeout_secs`).
pub const LEASH_TIMEOUT: Duration = Duration::from_secs(12);
/// A victim it can't reach this long, and it goes home, by default
/// (`[creatures] unreachable_timeout_secs`).
pub const UNREACHABLE_TIMEOUT: Duration = Duration::from_secs(24);
/// How long a creature flees, and how far it looks for help.
pub const FLEE_FOR: Duration = Duration::from_secs(7);
pub const FLEE_ASSISTANCE_RADIUS: f32 = 30.0;
/// How long it waits for help once it reaches it.
pub const ASSISTANCE_WAIT: Duration = Duration::from_millis(1500);
/// Creatures regenerate every five seconds.
pub const REGEN_EVERY: Duration = Duration::from_secs(5);

/// `flags_extra`: no leash at all; no evading an unreachable victim.
const EXTRA_NO_LEASH: u32 = 0x1;
const EXTRA_NO_UNREACHABLE_EVADE: u32 = 0x8;
/// `static_flags1`: it doesn't regenerate.
const STATIC_NO_AUTOMATIC_REGEN: u32 = 0x400;
/// `static_flags2`: it fights everyone in its dungeon.
const STATIC2_FORCE_RAID_COMBAT: u32 = 0x2;
/// `flags_extra`: it keeps no threat list.
const EXTRA_NO_THREAT_LIST: u32 = 0x800;
/// How often one fighting its whole dungeon draws in who isn't fighting.
pub const ZONE_PULSE: Duration = Duration::from_secs(3);
/// The unit flags of a creature under control: stunned, confused, fleeing.
const CONTROLLED: u32 = 0x0004_0000 | 0x0040_0000 | 0x0080_0000;

/// What a creature's fight remembers.
#[derive(Debug, Clone, Default)]
pub struct Engagement {
    /// Where the fight began.
    pub origin: Option<Vector3d>,
    /// The last hostile act on or by it.
    pub last_hostile: Option<Instant>,
    /// Since when its victim has been out of reach.
    pub unreachable_since: Option<Instant>,
    /// Set it only with `Map::rare.flee` noted (see [`RareStates`]); so
    /// with `charge` and `assist`.
    pub flee: Option<Flee>,
    pub regen_at: Option<Instant>,
    /// When fear or confusion next moves it.
    pub next_move_at: Option<Instant>,
    /// What moves it against its will, and the id of the walk it last
    /// sent it on.
    pub control: Option<Control>,
    pub control_spline: Option<u32>,
    /// When it last spent mana, for the five-second rule.
    pub mana_spent_at: Option<Instant>,
    /// When it next draws its dungeon into the fight.
    pub zone_pulse_at: Option<Instant>,
    /// A script set it drawing its dungeon in (`zone_combat_pulse`), for
    /// this fight.
    pub zone_scripted: bool,
    /// When it may next shout for the guards.
    pub guard_shout_at: Option<Instant>,
    /// When a frightened critter's escape ends.
    pub escape_at: Option<Instant>,
    /// Until when a distraction holds it.
    pub distracted_until: Option<Instant>,
    /// A charge it is on.
    pub charge: Option<crate::game::map::charge::Charge>,
    /// Damage it took from players' side, and from NPCs', this fight.
    pub damage_by_players: u64,
    pub damage_by_npcs: u64,
    /// Its calls for help on their way.
    pub assist: Vec<crate::game::map::assistance::Assist>,
    /// When it next calls its friends, fighting away from its spawn.
    pub pulse_at: Option<Instant>,
    /// The leash clock it shares with those it was drawn into the fight
    /// with ([`Leashes`]).
    pub clock: Option<u32>,
    /// When its leash is next extended while it is under control.
    pub control_extend_at: Option<Instant>,
    /// When it next looks to step about its victim, how often it has
    /// stepped aside since its victim last moved, and whether a step is
    /// under way (Thistle Tea's spreading).
    pub spread_at: Option<Instant>,
    pub spread_tries: u8,
    pub spreading: bool,
}

/// How often a creature under control has its leash extended (Thistle
/// Tea's `CombatLeash.maintain`).
const CONTROL_EXTEND_EVERY: Duration = Duration::from_secs(3);
/// How often the clocks no creature holds any more are swept away.
const LEASH_SWEEP_EVERY: Duration = Duration::from_secs(10);

/// The leash clocks creatures drawn into one fight share (Thistle Tea's
/// `CombatLeashes`): a creature called to help, a group's members pulled
/// together, a creature's summons. A hostile act on any of them extends
/// every one's leash; a clock lasts while any of them holds it.
#[derive(Debug, Default)]
pub struct Leashes {
    clocks: ahash::AHashMap<u32, Instant>,
    next: u32,
    swept_at: Option<Instant>,
}

/// The creature that summoned `c`, or owns it.
fn summoner_of(c: &crate::game::map::creature::Creature) -> Option<Guid> {
    c.scripted
        .as_deref()
        .and_then(|s| s.summoner)
        .or(c.owner)
        .filter(|s| guid::is_unit(*s))
}

/// `g`'s leash runs on `with`'s clock from now on (`with` gets one if it
/// has none). A summon whose summoner is fighting keeps its summoner's:
/// ownership comes before a call for help (Thistle Tea's owned leash).
pub fn share_leash(map: &mut Map, g: Guid, with: Guid) {
    if g == with {
        return;
    }
    let now = map.now;
    if let Some(s) = map.creature(g).and_then(summoner_of)
        && s != with
        && map
            .creature(s)
            .is_some_and(|o| o.alive() && o.engagement.origin.is_some())
    {
        return;
    }
    let Some(w) = map.creature(with) else {
        return;
    };
    if w.engagement.origin.is_none() {
        return;
    }
    let clock = match w.engagement.clock {
        Some(id) => id,
        None => {
            let since = w.engagement.last_hostile.unwrap_or(now);
            let l = &mut map.leashes;
            l.next = l.next.wrapping_add(1);
            let id = l.next;
            l.clocks.insert(id, since);
            if let Some(w) = map.creature_mut(with) {
                w.engagement.clock = Some(id);
            }
            id
        }
    };
    if let Some(c) = map.creature_mut(g) {
        c.engagement.clock = Some(clock);
    }
}

/// A fight begins for `g`: its leash is its own, unless a creature that
/// summoned it is fighting, whose clock it runs on (Thistle Tea's summon
/// source); and its summons fighting already run on its clock from now,
/// where their fights began kept (Thistle Tea's owned leash linking a
/// summon that fought first).
pub fn leash_begins(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.engagement.clock = None;
    c.engagement.control_extend_at = Some(now + CONTROL_EXTEND_EVERY);
    let summoner = summoner_of(c);
    let summons = c
        .scripted
        .as_deref()
        .map(|s| s.summoned.clone())
        .unwrap_or_default();
    if let Some(s) = summoner {
        share_leash(map, g, s);
    }
    for s in summons {
        if map.creature(s).is_some_and(|x| {
            x.alive() && x.engagement.origin.is_some() && summoner_of(x) == Some(g)
        }) {
            share_leash(map, s, g);
        }
    }
}

/// The last time `g`'s leash was extended: by a hostile act on it, or on
/// any it shares its clock with.
pub fn leash_extended_at(map: &Map, g: Guid) -> Option<Instant> {
    map.creature(g).and_then(|c| extended_at(map, c))
}

fn extended_at(map: &Map, c: &crate::game::map::creature::Creature) -> Option<Instant> {
    let shared = c
        .engagement
        .clock
        .and_then(|id| map.leashes.clocks.get(&id).copied());
    c.engagement.last_hostile.max(shared)
}

/// A creature under control (stunned, confused, fleeing, rooted) has its
/// leash extended every three seconds, so it doesn't go home the moment it
/// is free (Thistle Tea's `CombatLeash.maintain`).
pub fn maintain_leash(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.creature(g) else {
        return;
    };
    if c.engagement.origin.is_none() || !controlled(c) {
        return;
    }
    if c.engagement.control_extend_at.is_some_and(|t| t > now) {
        return;
    }
    hostile_act(map, g);
    if let Some(c) = map.creature_mut(g) {
        c.engagement.control_extend_at = Some(now + CONTROL_EXTEND_EVERY);
    }
}

/// Stunned, confused, fleeing or rooted.
fn controlled(c: &crate::game::map::creature::Creature) -> bool {
    c.fields.get_u32(UNIT_FIELD_FLAGS) & CONTROLLED != 0 || c.auras.has(AURA_MOD_ROOT)
}

/// The clocks no creature holds any more go.
fn sweep_leashes(map: &mut Map) {
    let now = map.now;
    if map
        .leashes
        .swept_at
        .is_some_and(|t| now.duration_since(t) < LEASH_SWEEP_EVERY)
    {
        return;
    }
    map.leashes.swept_at = Some(now);
    if map.leashes.clocks.is_empty() {
        return;
    }
    let held: ahash::AHashSet<u32> = map
        .creatures
        .iter()
        .filter_map(|(_, c)| c.engagement.clock)
        .collect();
    map.leashes.clocks.retain(|id, _| held.contains(id));
}

/// Which of the rarer states some creature of the map may be in: a
/// charge, a flight, a call for help on its way, a cast, being a pet. The
/// update looks at every creature for one only while one may be in it,
/// and the look clears it when it finds none. So whatever puts a creature
/// in one notes it here first (`Map::add_creature` notes what a creature
/// arrives with, and `Map::caster_mut` notes a cast); debug builds check
/// that a look skipped would have found nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct RareStates {
    pub charge: bool,
    pub flee: bool,
    pub assist: bool,
    pub casting: bool,
    pub pet: bool,
}

impl RareStates {
    /// The states `c` is in, noted.
    pub fn note(&mut self, c: &crate::game::map::creature::Creature) {
        let e = &c.engagement;
        self.charge |= e.charge.is_some();
        self.flee |= e.flee.is_some();
        self.assist |= !e.assist.is_empty();
        self.casting |= c.caster.casting.is_some();
        self.pet |= c.pet.is_some();
    }
}

/// A creature running: from `from`, to `helper` if it found one.
#[derive(Debug, Clone, Copy)]
pub struct Flee {
    pub from: Guid,
    pub until: Instant,
    pub helper: Option<Guid>,
    /// When it reached its helper and called it.
    pub called_at: Option<Instant>,
}

fn template(map: &Map, g: Guid) -> Option<&crate::game::data::CreatureTemplate> {
    map.creature(g)
        .and_then(|c| map.realm.data.creature_template(c.entry))
}

/// A fight begins: it remembers where.
pub fn engaged(map: &mut Map, g: Guid) {
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        c.engagement.origin = Some(c.position);
        c.engagement.last_hostile = Some(now);
        c.engagement.unreachable_since = None;
    }
}

/// A creature entering a fight stands up and stops its idle emote
/// (Thistle Tea's `Emote.reset`, as vmangos's `OnEnterCombat` does).
pub fn stand_up(map: &mut Map, g: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if c.fields.get_byte(UNIT_FIELD_BYTES_1, 0) != 0 {
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 0, 0);
    }
    if c.fields.get_u32(UNIT_NPC_EMOTESTATE) != 0 {
        c.fields.set_u32(UNIT_NPC_EMOTESTATE, 0);
    }
}

/// Home from a fight, a creature takes back its resting posture: how it
/// stood, its sheath, its idle emote and its mount (Thistle Tea's
/// `Posture.restore`, as vmangos reloads the addon).
pub fn rest(map: &mut Map, g: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let (now, r) = (crate::game::map::creature::Rest::of(&c.fields), c.rest);
    if now.stand != r.stand {
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 0, r.stand);
    }
    if now.sheath != r.sheath {
        c.fields.set_byte(UNIT_FIELD_BYTES_2, 0, r.sheath);
    }
    if now.emote != r.emote {
        c.fields.set_u32(UNIT_NPC_EMOTESTATE, r.emote);
    }
    if now.mount != r.mount {
        c.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, r.mount);
    }
}

/// A hostile act on the creature, a blow landed or not, extends its leash,
/// and theirs it shares its clock with.
pub fn hostile_act(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.engagement.last_hostile = Some(now);
    if let Some(id) = c.engagement.clock
        && let Some(t) = map.leashes.clocks.get_mut(&id)
    {
        *t = now;
    }
}

/// The fight is over.
pub fn disengaged(map: &mut Map, g: Guid) {
    if let Some(c) = map.creature_mut(g) {
        c.engagement.origin = None;
        c.engagement.unreachable_since = None;
        c.engagement.flee = None;
        c.engagement.zone_pulse_at = None;
        c.engagement.zone_scripted = false;
        c.engagement.damage_by_players = 0;
        c.engagement.damage_by_npcs = 0;
        c.engagement.assist.clear();
        c.engagement.pulse_at = None;
        c.engagement.control = None;
        c.engagement.control_spline = None;
        c.engagement.clock = None;
        c.engagement.control_extend_at = None;
        c.engagement.spread_at = None;
        c.engagement.spread_tries = 0;
        c.engagement.spreading = false;
    }
}

/// `static_flags1`: its corpse is the raid's, whoever fought it.
const STATIC_CORPSE_RAID: u32 = 0x1000;

/// Damage `amount` to creature `g` from `by` (Thistle Tea's
/// `DamageOrigin.record`): a player's side's, or its own, counts as the
/// players'; anything else as the NPCs'.
pub fn damaged(map: &mut Map, g: Guid, by: Guid, amount: u32) {
    if amount == 0 || !guid::is_unit(g) {
        return;
    }
    let players = by == g || guid::is_player(crate::game::map::pets::master(map, by));
    if let Some(c) = map.creature_mut(g) {
        if players {
            c.engagement.damage_by_players += u64::from(amount);
        } else {
            c.engagement.damage_by_npcs += u64::from(amount);
        }
    }
}

/// Whether a creature's corpse is lootable, and its kill rewarded: players
/// dealt more than 35% of its damage, or its corpse is the raid's.
pub fn loot_allowed(map: &Map, g: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return true;
    };
    let (p, n) = (c.engagement.damage_by_players, c.engagement.damage_by_npcs);
    template(map, g).is_some_and(|t| t.static_flags1 & STATIC_CORPSE_RAID != 0)
        || p + n == 0
        || 65 * p > 35 * n
}

/// The share of a kill's XP its damage earns: the players' part of it.
pub fn xp_share(map: &Map, g: Guid) -> f32 {
    let Some(c) = map.creature(g) else {
        return 1.0;
    };
    let (p, n) = (c.engagement.damage_by_players, c.engagement.damage_by_npcs);
    if template(map, g).is_some_and(|t| t.static_flags1 & STATIC_CORPSE_RAID != 0) || p + n == 0 {
        1.0
    } else if loot_allowed(map, g) {
        p as f32 / (p + n) as f32
    } else {
        0.0
    }
}

/// Whether `g` fights its whole dungeon (Thistle Tea's `ZoneCombat`): its
/// template says so, it keeps a threat list, and no player owns it.
fn zone_fighter(map: &Map, g: Guid) -> bool {
    let (Some(c), Some(t)) = (map.creature(g), template(map, g)) else {
        return false;
    };
    t.static_flags2 & STATIC2_FORCE_RAID_COMBAT != 0
        && t.flags_extra & EXTRA_NO_THREAT_LIST == 0
        && c.alive()
        && !c.owner.is_some_and(guid::is_player)
        && matches!(
            crate::game::instances::instance_type(&map.realm, map.key.map),
            1 | 2
        )
}

/// A fight begins for `g` against `source`: one that fights its whole
/// dungeon draws in everyone there, and their pets, if a player's side
/// began it.
pub fn entered(map: &mut Map, g: Guid, source: Guid) {
    if !zone_fighter(map, g) {
        return;
    }
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        c.engagement.zone_pulse_at = Some(now + ZONE_PULSE);
    }
    if guid::is_player(crate::game::map::pets::master(map, source)) {
        zone_pulse(map, g, true);
    }
}

/// How far the first pulse of a script's draws its nearest enemy in.
const ZONE_FIRST_YARDS: f32 = 533.333_3;

/// `SCRIPT_COMMAND_ZONE_COMBAT_PULSE`, as Thistle Tea's `ZoneCombat.pulse`:
/// in a dungeon, a creature that keeps a threat list and no player owns
/// draws everyone there into its fight (with `initial`, those fighting
/// already too, and first the nearest enemy if it has none), and goes on
/// every three seconds while it fights. False for a dead one.
pub fn script_zone_pulse(map: &mut Map, g: Guid, initial: bool) -> bool {
    let now = map.now;
    let Some(c) = map.creature(g) else {
        return false;
    };
    if !c.alive() {
        return false;
    }
    let eligible = template(map, g).is_none_or(|t| t.flags_extra & EXTRA_NO_THREAT_LIST == 0)
        && !c.owner.is_some_and(guid::is_player)
        && matches!(
            crate::game::instances::instance_type(&map.realm, map.key.map),
            1 | 2
        );
    if !eligible {
        return true;
    }
    let first = c.engagement.zone_pulse_at.is_none();
    let quiet = c.combat.victim.is_zero();
    let at = c.position;
    if let Some(c) = map.creature_mut(g) {
        c.engagement.zone_scripted = true;
        c.engagement.zone_pulse_at = Some(now + ZONE_PULSE);
    }
    if first && initial && quiet {
        let nearest = map
            .players
            .values()
            .filter(|p| {
                dist(p.position(), at) <= ZONE_FIRST_YARDS
                    && crate::game::map::combat::can_attack(map, g, p.guid)
                    && crate::game::map::combat::hostile(map, g, p.guid)
            })
            .min_by(|a, b| dist(a.position(), at).total_cmp(&dist(b.position(), at)))
            .map(|p| p.guid);
        if let Some(p) = nearest {
            crate::game::map::combat::engage(map, p, g);
        }
    }
    zone_pulse(map, g, initial);
    true
}

/// Everyone in the dungeon, and their pets, fight `g`: at first all of
/// them, afterwards those not already fighting.
fn zone_pulse(map: &mut Map, g: Guid, initial: bool) {
    let players: Vec<Guid> = map.players.keys().copied().collect();
    for p in players {
        let fighting = map
            .combat(p)
            .is_some_and(|c| c.attacking || !c.attackers.is_empty());
        if fighting && !initial {
            continue;
        }
        let pet = map.players.get(&p).and_then(|x| x.owned.pet);
        for t in std::iter::once(p).chain(pet) {
            if map.unit_view(t).is_some_and(|u| u.alive)
                && crate::game::map::combat::can_attack(map, g, t)
            {
                crate::game::map::combat::engage(map, t, g);
            }
        }
    }
}

/// Whether it could reach its victim on its last path.
pub fn reach(map: &mut Map, g: Guid, reachable: bool) {
    let now = map.now;
    if let Some(c) = map.creature_mut(g) {
        if reachable {
            c.engagement.unreachable_since = None;
        } else if c.engagement.unreachable_since.is_none() {
            c.engagement.unreachable_since = Some(now);
        }
    }
}

/// The hard leash: the template's, if no shorter than its detection range.
fn hard_leash(map: &Map, g: Guid) -> Option<f32> {
    template(map, g)
        .map(|t| t.leash_range)
        .filter(|r| *r > 0.0)
        .filter(|r| template(map, g).is_none_or(|t| *r >= t.detection_range))
}

/// Thistle Tea's `CombatLeash.should_evade?` and `UnreachableTarget`:
/// whether a creature fighting `victim` should give up and go home.
pub fn should_evade(map: &Map, g: Guid, victim: Guid) -> bool {
    let Some(c) = map.creature(g) else {
        return true;
    };
    let now = map.now;
    let extra = template(map, g).map_or(0, |t| t.flags_extra);
    let origin = c.engagement.origin.unwrap_or(c.home);
    let d = dist(c.position, origin);
    if hard_leash(map, g).is_some_and(|r| d > r) {
        return true;
    }
    // A victim out of reach too long.
    let controlled = controlled(c) || c.owner.is_some();
    if extra & EXTRA_NO_UNREACHABLE_EVADE == 0
        && !controlled
        && c.engagement.unreachable_since.is_some_and(|t| {
            now.duration_since(t) > Duration::from_secs(map.cfg.creatures.unreachable_timeout_secs)
        })
    {
        return true;
    }
    // In a dungeon, or without a leash, it fights on.
    if map.key.instance != 0 || extra & EXTRA_NO_LEASH != 0 || controlled {
        return false;
    }
    // The threat radius, or half again its reach of the victim if more.
    let radius = map
        .cfg
        .creatures
        .leash_radius
        .max(1.5 * crate::game::map::ai::attack_distance(map, g, victim));
    if d <= radius {
        return false;
    }
    if map
        .position_of(victim)
        .is_some_and(|p| dist(p, origin) <= radius)
    {
        return false;
    }
    let timeout = Duration::from_secs(map.cfg.creatures.leash_timeout_secs);
    extended_at(map, c).is_none_or(|t| now.duration_since(t) > timeout)
}

/// `static_flags1`: no defence against spells (the water guardians').
const STATIC_NO_SPELL_DEFENSE: u32 = 0x8000;

/// Whether a creature's template gives it no defence against spells: no
/// magic miss by its level or its mechanic resistance (`NO_SPELL_DEFENSE`,
/// which mangos-classic names and reads nowhere; Thistle Tea's).
pub fn no_spell_defense(map: &Map, g: Guid) -> bool {
    template(map, g).is_some_and(|t| t.static_flags1 & STATIC_NO_SPELL_DEFENSE != 0)
}

/// The template's mechanic immunity: `mechanic` (1 to 31) is in its mask.
pub fn mechanic_immune(map: &Map, g: Guid, mechanic: u32) -> bool {
    (1..=31).contains(&mechanic)
        && template(map, g).is_some_and(|t| t.mechanic_immune_mask & (1 << (mechanic - 1)) != 0)
}

/// The template's school immunity, for a school mask.
pub fn school_immune(map: &Map, g: Guid, school: u32) -> bool {
    school != 0 && template(map, g).is_some_and(|t| t.school_immune_mask & school != 0)
}

/// Whether a creature's template stops `spell` from `caster`: its mechanic,
/// or, if harmful, its school (mangos's `Creature::IsImmuneToSpell`,
/// Thistle Tea's `CreatureImmunity.spell?`). An effect's own mechanic stops
/// that effect alone ([`effect_immune`]).
pub fn spell_immune(map: &Map, target: Guid, caster: Guid, spell: &SpellTemplate) -> bool {
    if target == caster
        || !guid::is_unit(target)
        || spell.attr(0, crate::game::data::spells::ATTR_NO_IMMUNITIES)
    {
        return false;
    }
    mechanic_immune(map, target, spell.mechanic)
        || (!auras::positive(spell) && school_immune(map, target, spell.school_mask()))
}

/// Whether a creature's template stops one effect of `spell` from
/// `caster`, by the effect's mechanic, whatever the spell's attributes
/// (mangos's `Creature::IsImmuneToSpellEffect`, Thistle Tea's
/// `CreatureImmunity.effect?`).
pub fn effect_immune(map: &Map, target: Guid, caster: Guid, mechanic: u32) -> bool {
    target != caster && guid::is_unit(target) && mechanic_immune(map, target, mechanic)
}

/// Whether fleeing is barred: an aura that prevents it, feigned death,
/// confusion, a master's control, or a distraction holding it (Thistle
/// Tea's `Flee.blocked?`).
fn flee_prevented(map: &Map, g: Guid) -> bool {
    let now = map.now;
    map.creature(g).is_none_or(|c| {
        c.auras.has(AURA_PREVENTS_FLEEING)
            || c.auras.has(AURA_FEIGN_DEATH)
            || c.auras.has(AURA_MOD_CONFUSE)
            || c.owner.is_some()
            || c.engagement.distracted_until.is_some_and(|t| t > now)
            || !c.alive()
    })
}

/// A creature flees (script command 47): seeking help (`seek`), to the
/// nearest creature of its faction within 30 yards in sight that answers
/// calls, which it calls on arrival; else, or with none, it runs in fear
/// for seven seconds (Thistle Tea's `Flee.start`).
pub fn flee_for_assistance(map: &mut Map, g: Guid, seek: bool) {
    let now = map.now;
    if flee_prevented(map, g) {
        return;
    }
    let Some(c) = map.creature(g) else {
        return;
    };
    let from = c.combat.victim;
    if from.is_zero() {
        return;
    }
    let helper = if seek {
        crate::game::map::assistance::nearest_helper(map, g)
    } else {
        None
    };
    emote(map, g, "%s attempts to run away in fear!");
    crate::game::map::spells::interrupt(map, g);
    map.rare.flee = true;
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    c.engagement.flee = Some(Flee {
        from,
        until: now + FLEE_FOR,
        helper,
        called_at: None,
    });
    c.engagement.next_move_at = Some(now);
    c.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, true);
    // Running, it shows no target, its victim kept (mangos's
    // `RetreatMovementGenerator` and panic's `SetTarget(nullptr)`).
    c.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
}

/// A creature's text emote, its name in place of `%s`.
fn emote(map: &Map, g: Guid, text: &str) {
    let Some(c) = map.creature(g) else {
        return;
    };
    let name = map
        .realm
        .data
        .creature_template(c.entry)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    if let Some(frame) = crate::game::conn::encode(&SMSG_MESSAGECHAT {
        chat_type: SMSG_MESSAGECHAT_ChatType::MonsterEmote {
            monster: g,
            monster_name: name,
        },
        language: Language::Universal,
        message: text.to_owned(),
        tag: PlayerChatTag::None,
    }) {
        map.send_to_observers(g, &frame, false);
    }
}

/// What moves a creature against its will, as Thistle Tea's
/// `ControlMovement` has it: confusion (about where it began), fear, or a
/// flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    Confusion { anchor: Vector3d },
    Fear,
    Flee,
}

fn same_kind(a: Option<Control>, b: Option<Control>) -> bool {
    a.map(|x| std::mem::discriminant(&x)) == b.map(|x| std::mem::discriminant(&x))
}

/// What controls a creature now: confusion before fear (which an aura
/// that prevents fleeing bars), then a flight.
fn wanted_control(map: &Map, g: Guid) -> Option<Control> {
    let c = map.creature(g)?;
    if !c.alive() {
        return None;
    }
    let a = &c.auras;
    if a.has(AURA_MOD_CONFUSE) {
        let anchor = match c.engagement.control {
            Some(Control::Confusion { anchor }) => anchor,
            _ => c.position,
        };
        return Some(Control::Confusion { anchor });
    }
    if a.has(AURA_MOD_FEAR) && !a.has(AURA_PREVENTS_FLEEING) {
        return Some(Control::Fear);
    }
    // A run waits while fleeing is prevented (mangos's `SetFleeing`).
    c.engagement
        .flee
        .filter(|_| !a.has(AURA_PREVENTS_FLEEING))
        .map(|_| Control::Flee)
}

/// Fear, confusion or a flight began or ended: the walk under way stops
/// where the creature is, whatever it was, and the new mode moves it from
/// there at once (Thistle Tea's `ControlMovement.transition`). Its mode now.
pub fn control_changed(map: &mut Map, g: Guid) -> Option<Control> {
    let now = map.now;
    // Its walk home is its own.
    if map.creature(g).is_none_or(|c| c.evading) {
        return None;
    }
    let want = wanted_control(map, g);
    let had = map.creature(g).and_then(|c| c.engagement.control);
    if same_kind(had, want) {
        return want;
    }
    crate::game::map::ai::halt(map, g);
    if let Some(c) = map.creature_mut(g) {
        c.engagement.control = want;
        c.engagement.control_spline = None;
        c.engagement.next_move_at = want.map(|_| now);
    }
    want
}

/// `MOVEFLAG_ROOT`: a creature held where it stands, in its movement as a
/// newcomer is told of it (mangos's `SendMoveRoot` keeps it there).
pub const MOVEFLAG_ROOT: u32 = 0x0800_0000;

/// A creature rooted or stunned: whatever walk it was on (a wander, its
/// path, a chase, a run in fear) stops where it is, and those who see it
/// are told it is held (mangos's `SetImmobilizedState`: `StopMoving`, then
/// `SMSG_SPLINE_MOVE_ROOT`). Its own walk goes on once it's free; its fight
/// goes on where it stands.
pub fn immobilized(map: &mut Map, g: Guid) {
    let now = map.now;
    let Some(was) = map.creature(g).map(|c| c.move_flags & MOVEFLAG_ROOT != 0) else {
        return;
    };
    crate::game::map::ai::halt(map, g);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if c.movement_type != crate::game::map::creature::MOVEMENT_IDLE
        && matches!(c.motion, Motion::Idle { until: None })
    {
        c.motion = Motion::Idle { until: Some(now) };
    }
    c.move_flags |= MOVEFLAG_ROOT;
    if !was && let Some(frame) = crate::game::conn::encode(&SMSG_SPLINE_MOVE_ROOT { guid: g }) {
        map.send_to_observers(g, &frame, false);
    }
}

/// A creature's last root or stun gone: it may move again, and those who
/// see it are told (mangos's `SMSG_SPLINE_MOVE_UNROOT`).
pub fn freed(map: &mut Map, g: Guid) {
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    if c.move_flags & MOVEFLAG_ROOT == 0 {
        return;
    }
    c.move_flags &= !MOVEFLAG_ROOT;
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPLINE_MOVE_UNROOT { guid: g }) {
        map.send_to_observers(g, &frame, false);
    }
}

/// Rooted or stunned: it may not move.
pub fn immobile(map: &Map, g: Guid) -> bool {
    map.creature(g)
        .is_some_and(|c| c.auras.has(AURA_MOD_ROOT) || c.auras.has(AURA_MOD_STUN))
}

/// Fear, confusion and fleeing move a creature themselves: the fight's own
/// step waits. Whether it did.
pub fn controlled_step(map: &mut Map, g: Guid) -> bool {
    let now = map.now;
    let Some(c) = map.creature(g) else {
        return false;
    };
    let feared = c.auras.has(AURA_MOD_FEAR);
    let confused = c.auras.has(AURA_MOD_CONFUSE);
    // Its victim changed (a taunt), dead or gone: the run is for nothing
    // (Thistle Tea's `SeekAssistance.maintain`).
    let lost =
        |f: &Flee| c.combat.victim != f.from || map.unit_view(f.from).is_none_or(|u| !u.alive);
    if let Some(f) = c.engagement.flee
        && (f.until <= now || feared || confused || lost(&f))
    {
        end_flee(map, g);
    }
    let Some(mode) = control_changed(map, g) else {
        return false;
    };
    // Where its run has taken it.
    crate::game::map::ai::advance_motion(map, g);
    let Some(c) = map.creature(g) else {
        return false;
    };
    // Stunned or rooted: it stands, held. A run for help goes on the
    // moment it is free, the rest of its way asked for anew (Thistle Tea's
    // `SeekAssistance`, mangos's retreat reset); fear and confusion keep
    // their own pauses.
    // One that never moves stands under fear and confusion too: they take,
    // but its AI holds it rooted (mangos's `SetAIImmobilizedState` for
    // `SESSILE`, which every movement but a fall waits out).
    if c.auras.has(AURA_MOD_STUN)
        || c.auras.has(AURA_MOD_ROOT)
        || crate::game::map::creature_motion::sessile(map, g)
    {
        let seeking = c.engagement.flee.is_some_and(|f| f.helper.is_some());
        crate::game::map::ai::halt(map, g);
        if seeking && let Some(c) = map.creature_mut(g) {
            c.engagement.next_move_at = Some(now);
        }
        return true;
    }
    match &c.motion {
        Motion::Moving(w) if Some(w.spline_id) == c.engagement.control_spline => return true,
        // A walk of something else's: it stops.
        Motion::Moving(_) => crate::game::map::ai::halt(map, g),
        Motion::Idle { .. } => {}
    }
    let Some(c) = map.creature(g) else {
        return false;
    };
    if c.engagement.next_move_at.is_some_and(|t| t > now) {
        return true;
    }
    let flee = c.engagement.flee;
    let pos = c.position;
    let mut rng = rand::rng();
    let between = |rng: &mut rand::rngs::ThreadRng, lo: u64, hi: u64| {
        Duration::from_millis(rng.random_range(lo..hi))
    };
    let planned = if let Some(f) = flee
        && let Some(helper) = f.helper
    {
        match map.position_of(helper) {
            Some(at) if dist(at, pos) > 3.0 => {
                let path = match map.nav.as_mut() {
                    Some(nav) => nav.path(pos, at).unwrap_or_else(|| vec![pos, at]),
                    None => vec![pos, at],
                };
                Ok((path, true, Duration::from_millis(500)))
            }
            Some(_) => {
                // Arrived: help is called, and given a moment.
                if f.called_at.is_none() {
                    if !f.from.is_zero() {
                        crate::game::map::assistance::drawn_in(map, g, f.from);
                    }
                    if let Some(c) = map.creature_mut(g)
                        && let Some(fl) = c.engagement.flee.as_mut()
                    {
                        fl.called_at = Some(now);
                        fl.until = fl.until.min(now + ASSISTANCE_WAIT);
                    }
                }
                return true;
            }
            None => {
                end_flee(map, g);
                return false;
            }
        }
    } else if let Control::Confusion { anchor } = mode {
        // Confusion: a walk to a point of the navmesh within four yards of
        // where it began, no further than eight yards, or a pause.
        use crate::game::map::player_motion::{CONFUSION_PATH, CONFUSION_RADIUS};
        let point = crate::game::map::player_motion::confusion_point(map, anchor, &mut rng);
        match point.and_then(|to| {
            crate::game::map::player_motion::control_path(map, pos, to, CONFUSION_PATH).filter(
                |p| {
                    p.iter()
                        .all(|x| dist(*x, anchor) <= CONFUSION_RADIUS + 0.01)
                },
            )
        }) {
            Some(path) => Ok((path, false, between(&mut rng, 500, 1500))),
            None => Err(between(&mut rng, 500, 1500)),
        }
    } else {
        // Fear: Thistle Tea's run at a random angle, longer the nearer
        // the one it fears, to a point the navmesh has near where the
        // run would end.
        let source = flee.map(|f| f.from).or_else(|| {
            c.auras
                .holders
                .iter()
                .find(|h| h.has(AURA_MOD_FEAR))
                .map(|h| h.caster)
        });
        let distance = source
            .and_then(|s| map.position_of(s))
            .map_or(0.0, |s| dist(s, pos));
        match crate::game::map::player_motion::fear_point(map, pos, distance, &mut rng) {
            None => Err(between(&mut rng, 1000, 1500)),
            Some(to) => match crate::game::map::player_motion::control_path(
                map,
                pos,
                to,
                crate::game::map::player_motion::FEAR_PATH,
            ) {
                Some(path) => Ok((path, true, between(&mut rng, 800, 1500))),
                None => Err(between(&mut rng, 800, 1500)),
            },
        }
    };
    let (path, run, pause) = match planned {
        Ok(p) => p,
        Err(pause) => {
            if let Some(c) = map.creature_mut(g) {
                c.engagement.next_move_at = Some(now + pause);
            }
            return true;
        }
    };
    let spline_id = map.next_spline_id;
    map.next_spline_id = map.next_spline_id.wrapping_add(1);
    let Some(c) = map.creature_mut(g) else {
        return true;
    };
    let frame = c.walk(path, run, now, spline_id);
    // The pause comes after the run.
    let run_for = match &c.motion {
        Motion::Moving(w) => w.duration,
        Motion::Idle { .. } => Duration::ZERO,
    };
    c.engagement.control_spline = Some(spline_id);
    c.engagement.next_move_at = Some(now + run_for + pause);
    if let Some(frame) = frame {
        map.send_to_observers(g, &frame, false);
    }
    true
}

fn end_flee(map: &mut Map, g: Guid) {
    // A critter's escape selects no one.
    let critter = crate::game::map::creature_motion::critter(map, g);
    let Some(c) = map.creature_mut(g) else {
        return;
    };
    let fled = c.engagement.flee.take().is_some();
    if fled && !c.auras.has(AURA_MOD_FEAR) {
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING, false);
    }
    // Its target shown again, if it fights on.
    if fled && !critter && c.alive() && c.combat.attacking {
        let victim = c.combat.victim;
        c.fields.set_guid(UNIT_FIELD_TARGET, victim);
    }
    c.chase_target_at = None;
}

/// Whether a creature is polymorphed: a mage's confusion that transforms.
pub fn polymorphed(map: &Map, g: Guid) -> bool {
    let data = &map.realm.data;
    crate::game::map::auras::auras(map, g).is_some_and(|a| {
        a.holders.iter().any(|h| {
            h.has(AURA_MOD_CONFUSE)
                && data
                    .spells
                    .get(h.spell)
                    .is_some_and(|s| s.family_name == 3 && s.prevention_type == 1)
        })
    })
}

/// A creature's mana each tick in a fight, from its class's spirit and
/// intellect (Thistle Tea's `creature_combat_mana_per_tick`).
pub fn combat_mana_per_tick(map: &Map, g: Guid) -> u32 {
    let Some(c) = map.creature(g) else {
        return 0;
    };
    let data = &map.realm.data;
    let Some(t) = data.creature_template(c.entry) else {
        return 0;
    };
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL).min(255) as u8;
    let Some(stats) = data.class_level_stats.get(&(t.unit_class, level)) else {
        return 0;
    };
    let spirit = stats.spirit as f32;
    let per_tick = match t.unit_class {
        8 | 5 => spirit / 4.0 + 12.5,
        7 => spirit / 5.0 + 17.0,
        2 | 3 | 9 | 11 => spirit / 5.0 + 15.0,
        _ => 0.0,
    };
    let intellect = (stats.intellect as f32).max(1.0);
    ((per_tick / 2.0 + 0.6 * intellect.sqrt() / 5.0) * 5.0).max(0.0) as u32
}

/// Whether a creature's health comes back by itself (Thistle Tea's
/// `Regen.regenerates_health?`).
pub fn regenerates_health(map: &Map, g: Guid) -> bool {
    regenerated_stats(map, g) & REGEN_HEALTH != 0
}

/// What a creature regenerates (Thistle Tea's `regenerate_stats`): health
/// and its power, by a script's say (`C_SET_REGENERATION`) over its
/// template's (none with the no-regeneration flag, else both).
pub const REGEN_HEALTH: u32 = 0x1;
pub const REGEN_POWER: u32 = 0x2;

fn regenerated_stats(map: &Map, g: Guid) -> u32 {
    crate::game::map::event_ai::scripted(map, g)
        .and_then(|s| s.regenerate)
        .unwrap_or_else(|| {
            if template(map, g).is_none_or(|t| t.static_flags1 & STATIC_NO_AUTOMATIC_REGEN == 0) {
                REGEN_HEALTH | REGEN_POWER
            } else {
                0
            }
        })
}

/// Creatures regenerate: a third of health and mana each tick out of a
/// fight, health in one while polymorphed, and mana in one by spirit and
/// intellect outside the five-second rule.
pub fn regenerate(map: &mut Map) {
    let now = map.now;
    // Health or mana short first: few are, and it is answered from the
    // creature itself, not its fields' values.
    let due: Vec<Guid> = map
        .creatures
        .iter()
        .filter(|(_, c)| c.fields.below_max())
        .filter(|(_, c)| c.alive() && !c.evading && c.owner.is_none_or(|o| !guid::is_player(o)))
        .filter(|(_, c)| c.engagement.regen_at.is_none_or(|t| t <= now))
        .map(|(_, c)| c.guid)
        .filter(|&g| !crate::game::map::event_ai::feigning(map, g))
        .collect();
    for &g in &due {
        let stats = regenerated_stats(map, g);
        let poly = polymorphed(map, g);
        let combat_mana = combat_mana_per_tick(map, g);
        let Some(c) = map.creature_mut(g) else {
            continue;
        };
        if c.engagement.regen_at.is_none() {
            c.engagement.regen_at = Some(now + REGEN_EVERY);
            continue;
        }
        c.engagement.regen_at = Some(now + REGEN_EVERY);
        let fighting = c.combat.attacking || !c.combat.attackers.is_empty();
        let f = &mut c.fields;
        if stats & REGEN_HEALTH != 0 && (!fighting || poly) {
            let (health, max) = (
                f.get_u32(UNIT_FIELD_HEALTH),
                f.get_u32(UNIT_FIELD_MAXHEALTH),
            );
            f.set_u32(UNIT_FIELD_HEALTH, (health + (max / 3).max(1)).min(max));
        }
        let (mana, max) = (
            f.get_u32(UNIT_FIELD_POWER1),
            f.get_u32(UNIT_FIELD_MAXPOWER1),
        );
        // Mana spent within five seconds holds it back.
        let cast_lately = c
            .engagement
            .mana_spent_at
            .is_some_and(|t| now.duration_since(t) < crate::game::map::stats::FIVE_SECOND_RULE);
        let gain = if max == 0 || stats & REGEN_POWER == 0 {
            0
        } else if !fighting {
            (max / 3).max(1)
        } else if cast_lately {
            0
        } else {
            combat_mana
        };
        c.fields.set_u32(UNIT_FIELD_POWER1, (mana + gain).min(max));
    }
    // Health back: wounds slow it no more.
    for g in due {
        crate::game::map::combat::sync_aura_state(map, g);
    }
}

/// The creatures' flight timers, and their regeneration.
pub fn update(map: &mut Map) {
    {
        let _z = crate::zone!("regeneration");
        regenerate(map);
    }
    {
        let _z = crate::zone!("leashes");
        sweep_leashes(map);
    }
    {
        let _z = crate::zone!("guards and Jaina");
        crate::game::jaina::update(map);
    }
    {
        let _z = crate::zone!("assistance");
        crate::game::map::assistance::update(map);
    }
    let now = map.now;
    // Only in a dungeon or a raid does a creature pulse its map (as
    // `zone_fighter` has it), so only there is every creature looked at.
    let dungeon = matches!(
        crate::game::instances::instance_type(&map.realm, map.key.map),
        1 | 2
    );
    let pulses: Vec<(Guid, Guid)> = if dungeon {
        map.creatures
            .iter()
            .filter(|(_, c)| c.engagement.zone_pulse_at.is_some_and(|t| t <= now))
            .map(|(_, c)| (c.guid, c.combat.victim))
            .collect()
    } else {
        debug_assert!(
            map.creatures
                .iter()
                .all(|(_, c)| c.engagement.zone_pulse_at.is_none()),
            "{}: a creature pulses a map that is no dungeon",
            map.key
        );
        Vec::new()
    };
    for (g, victim) in pulses {
        let fighting = map.creature(g).is_some_and(|c| c.combat.attacking);
        if let Some(c) = map.creature_mut(g) {
            c.engagement.zone_pulse_at = fighting.then_some(now + ZONE_PULSE);
        }
        let scripted = map.creature(g).is_some_and(|c| c.engagement.zone_scripted);
        if fighting && (zone_fighter(map, g) || scripted) && !should_evade(map, g, victim) {
            zone_pulse(map, g, false);
        }
    }
    let mut over: Vec<Guid> = Vec::new();
    if map.rare.flee {
        let mut any = false;
        over = map
            .creatures
            .iter()
            .filter(|(_, c)| {
                any |= c.engagement.flee.is_some();
                c.engagement.flee.is_some_and(|f| f.until <= now)
            })
            .map(|(_, c)| c.guid)
            .collect();
        map.rare.flee = any;
    } else {
        debug_assert!(
            map.creatures
                .iter()
                .all(|(_, c)| c.engagement.flee.is_none()),
            "a creature flees unnoted (RareStates)"
        );
    }
    for g in over {
        end_flee(map, g);
    }
}
