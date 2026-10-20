//! The items and gadgets whose spells the data can't say, as Thistle Tea's
//! `Item.Consumable`, `Item.ItemSpell` and `Profession.Engineering` run
//! them, each use one weighted outcome cast as a triggered spell:
//!
//! - Noggenfogger Elixir, the Deviate Fish and the Savory Deviate Delight
//!   pick their transformation (the fish's Party Time! dances every ten
//!   seconds);
//! - Linken's Boomerang stuns one throw in thirty-one and disarms one in
//!   eleven; the Scorpid Surprise's bonus is one in eleven;
//! - the Bending Shinbone mostly breaks; the Fetid Skull resonates or turns
//!   to dust; the Six Demon Bag looses one of seven things;
//! - the Gnomish transporter (Ultrasafe Transporter: Gadgetzan) arrives,
//!   arrives badly, or drops its user from the sky; a bad arrival
//!   malfunctions, makes an evil twin or nothing; the Goblin Dimensional
//!   Ripper to Everlook may bring an evil twin or set its user alight;
//! - the Net-o-Matic, Universal Remote and Gnomish Mind Control Cap may
//!   backfire; the Goblin Bomb Dispenser may blow up in its user's hands;
//! - the dragonling and battle chicken trinkets summon their guardian; the
//!   Explosive Sheep and Goblin Bomb carry their passives and blow up when
//!   their time is up;
//! - the Gnomish Death Ray hurts its user while channelled, then lets the
//!   whole of it loose on their target;
//! - Goblin Jumper Cables (and the XL) fail more often than not.

use std::time::{Duration, Instant};

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::auras::Holder;
use crate::game::map::{Map, MapKey};

const EFFECT_DUMMY: u32 = 3;
const EFFECT_TELEPORT_UNITS: u32 = 5;
const EFFECT_RESURRECT: u32 = 18;
const EFFECT_TRIGGER_SPELL: u32 = 64;
const EFFECT_SCRIPT_EFFECT: u32 = 77;

const DEVIATE_FISH: u32 = 8063;
const SAVORY_DEVIATE_DELIGHT: u32 = 8213;
const NOGGENFOGGER: u32 = 16589;
const PARTY_TIME: u32 = 8067;
const PARTY_EVERY: Duration = Duration::from_secs(10);
/// Cheer, applause, laugh and roar; standing still, a dance too.
const PARTY_EMOTES: [u32; 4] = [21, 4, 19, 11];
const DANCE: u32 = 94;
const LINKENS_BOOMERANG: u32 = 15712;
const SCORPID_SURPRISE: u32 = 6410;
const BENDING_SHINBONE: u32 = 8856;
const FETID_SKULL: u32 = 17271;
const SIX_DEMON_BAG: u32 = 14537;
const NET_O_MATIC: u32 = 13120;
const NET_BACKFIRE: u32 = 13119;
const NET_BACKFIRE_MARKER: u32 = 13139;
const NET_BACKFIRE_SELF: u32 = 13138;
const UNIVERSAL_REMOTE: u32 = 8344;
const MIND_CONTROL_CAP: u32 = 13180;
const MIND_CONTROL: u32 = 13181;
const GOBLIN_BOMB: u32 = 23134;
const GNOMISH_TRANSPORTER: u32 = 23453;
const TRANSPORTER_ARRIVAL: u32 = 23448;
const EVERLOOK_TRANSPORTER: u32 = 23442;
const EVIL_TWIN: u32 = 23445;
const DEATH_RAY_CHANNEL: u32 = 13278;
pub const DEATH_RAY_PERIODIC: u32 = 13493;
const DEATH_RAY_DISCHARGE: u32 = 13279;
const JUMPER_CABLES: u32 = 8342;
const JUMPER_CABLES_XL: u32 = 22999;

/// Where the Gnomish transporter drops a user it fails: high over
/// Gadgetzan.
const SKY_OVER_GADGETZAN: (u32, f32, f32, f32, f32) = (1, -7341.38, -3908.11, 150.7, 0.51);

/// Each trinket's guardian summons.
const GUARDIAN_TRINKETS: [(u32, u32); 4] = [
    (23074, 19804),
    (23075, 12749),
    (23076, 4073),
    (23133, 13166),
];
/// The engineered guardians' passives.
const GUARDIAN_PASSIVES: [(u32, u32); 5] = [
    (2675, 4051),
    (2678, 23051),
    (8615, 23050),
    (8937, 13260),
    (12473, 23052),
];
/// The target dummies, wild summons (vmangos's `npc_target_dummy`, Thistle
/// Tea's `WildSummon.target_dummy`): each one's passive (threat on the
/// enemies about it every few seconds) and the taunt it comes with. A
/// dummy is of its summoner's side, stands where it is put, fights
/// nothing, and after fifteen seconds kills itself, in a fight or not
/// (mangos-classic's `npc_advanced_target_dummy`: its spawn effect,
/// passive react, and Suicide at 15 s).
pub const DUMMY_LIFETIME: Duration = Duration::from_secs(15);
const SUICIDE: u32 = 7;
/// The Explosive Sheep and the Goblin Bomb: how long each lasts, and the
/// blast it ends in.
const GUARDIAN_LIFETIMES: [(u32, Duration, u32); 2] = [
    (2675, Duration::from_secs(180), 4050),
    (8937, Duration::from_secs(60), 13259),
];

/// The engineers' Target Dummies (the plain, the Advanced and the
/// Masterwork): each one's passive and its spawn's taunt (Thistle Tea's
/// `@dummy_spells`, vmangos's `npc_target_dummy`).
pub const TARGET_DUMMIES: [(u32, u32, u32); 3] =
    [(2673, 4044, 4507), (2674, 4048, 4092), (12426, 19809, 4092)];
/// A Target Dummy lasts this long, whatever it is in.
const TARGET_DUMMY_LIFE: Duration = Duration::from_secs(15);

/// What a player's gadgets remember.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gadgets {
    /// The Death Ray's hurt to its user so far.
    pub death_ray: u32,
    /// Party Time!'s next dance.
    pub party_at: Option<Instant>,
}

fn script(s: &SpellTemplate, name: &str) -> bool {
    s.script_name == name
}

/// One of `choices` by weight; `None` when they weigh nothing.
fn pick<T: Copy>(choices: &[(u32, T)]) -> Option<T> {
    let total: u32 = choices.iter().map(|(w, _)| w).sum();
    if total == 0 {
        return None;
    }
    let mut roll = rand::rng().random_range(0..total);
    for (w, t) in choices {
        if roll < *w {
            return Some(*t);
        }
        roll -= w;
    }
    None
}

fn trigger(map: &mut Map, caster: Guid, spell: u32, target: Guid) {
    crate::game::map::spells::cast_triggered(map, caster, spell, Some(target));
}

/// One effect of `s` landing on `target`: whether a script took it.
pub fn effect(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    item: Option<Guid>,
) -> bool {
    match s.effects[index].effect {
        EFFECT_DUMMY if index == 0 => dummy(map, caster, target, s, item),
        EFFECT_DUMMY if s.id == DEATH_RAY_CHANNEL => {
            // The channel begins: its hurt to its user begins.
            if let Some(p) = map.players.get_mut(&caster) {
                p.gadgets.death_ray = 0;
            }
            trigger(map, caster, DEATH_RAY_PERIODIC, caster);
            true
        }
        EFFECT_SCRIPT_EFFECT if index == 0 => match s.id {
            crate::game::map::mounts::BLACK_QIRAJI => {
                crate::game::map::mounts::summon_qiraji(map, caster);
                true
            }
            BENDING_SHINBONE if target == caster => {
                if let Some(id) = pick(&[(1, 8854), (4, 8855)]) {
                    trigger(map, caster, id, caster);
                }
                true
            }
            _ => false,
        },
        EFFECT_TRIGGER_SPELL => {
            // A rare bonus that, failing its roll, isn't.
            let chance = match (index, s.id) {
                (1, LINKENS_BOOMERANG) => Some(31),
                (2, LINKENS_BOOMERANG) => Some(11),
                (1, SCORPID_SURPRISE) => Some(11),
                _ if script(s, "spell_linkens_boomerang") && index == 1 => Some(31),
                _ if script(s, "spell_linkens_boomerang") && index == 2 => Some(11),
                _ => None,
            };
            chance.is_some_and(|total| rand::rng().random_range(0..total) != 0)
        }
        EFFECT_TELEPORT_UNITS
            if s.id == EVERLOOK_TRANSPORTER || script(s, "spell_everlook_transporter") =>
        {
            // The ripper's mishaps; the trip itself goes ahead.
            if let Some(Some(id)) = pick(&[(7, None), (3, Some(EVIL_TWIN)), (2, Some(23449))]) {
                trigger(map, caster, id, caster);
            }
            false
        }
        EFFECT_RESURRECT if matches!(s.id, JUMPER_CABLES | JUMPER_CABLES_XL) => {
            let (success, failure) = if s.id == JUMPER_CABLES {
                (33, 8338)
            } else {
                (50, 23055)
            };
            if rand::rng().random_range(0..100) < success {
                return false;
            }
            trigger(map, caster, failure, caster);
            true
        }
        _ => false,
    }
}

/// A dummy effect's script: whether there was one.
fn dummy(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate, item: Option<Guid>) -> bool {
    let own = target == caster;
    let id = s.id;
    if id == NOGGENFOGGER || script(s, "spell_noggenfogger_elixir") {
        if own && let Some(o) = pick(&[(2, 16595), (2, 16593), (6, 16591)]) {
            trigger(map, caster, o, caster);
        }
        return true;
    }
    if id == DEVIATE_FISH || script(s, "spell_deviate_fish") {
        let all = [8064, 8065, 8066, 8067, 8068, 8070].map(|s| (1, s));
        if own && let Some(o) = pick(&all) {
            trigger(map, caster, o, caster);
        }
        return true;
    }
    if id == SAVORY_DEVIATE_DELIGHT || script(s, "spell_cooked_deviate_fish") {
        let male = map.players.get(&caster).is_some_and(|p| p.row.gender == 0);
        let choices = if male {
            [(1, 8219), (1, 8221)]
        } else {
            [(1, 8220), (1, 8222)]
        };
        if own && let Some(o) = pick(&choices) {
            trigger(map, caster, o, caster);
        }
        return true;
    }
    if let Some((_, summon)) = GUARDIAN_TRINKETS.iter().find(|(t, _)| *t == id) {
        if own && item.is_some() {
            trigger(map, caster, *summon, caster);
        }
        return true;
    }
    match id {
        FETID_SKULL => {
            if own && let Some(o) = pick(&[(1, 17269), (1, 17270)]) {
                trigger(map, caster, o, caster);
            }
        }
        SIX_DEMON_BAG => {
            // One of its seven tricks, two of them on its user.
            let all = [
                (25, (11921, false)),
                (25, (13322, false)),
                (20, (21179, false)),
                (7, (13323, false)),
                (3, (13323, true)),
                (15, (25189, false)),
                (5, (14642, true)),
            ];
            if let Some((o, on_self)) = pick(&all) {
                trigger(map, caster, o, if on_self { caster } else { target });
            }
        }
        NET_O_MATIC => {
            // Its user netted, a net with a backfire, or the target netted.
            if let Some(o) = pick(&[(1, 16566), (1, NET_BACKFIRE), (8, 13099)]) {
                trigger(map, caster, o, target);
            }
        }
        // The control devices do nothing to the dead (Thistle Tea's
        // `Engineering`; mangos's targeting has none dead).
        UNIVERSAL_REMOTE | MIND_CONTROL_CAP if !map.unit_view(target).is_some_and(|u| u.alive) => {}
        UNIVERSAL_REMOTE => match pick(&[(1, 8345), (1, 8346), (1, 8599)]) {
            Some(8599) => trigger(map, target, 8599, target),
            Some(o) => trigger(map, caster, o, target),
            None => {}
        },
        MIND_CONTROL_CAP => {
            let shifted = crate::game::map::auras::form(map, caster) != 0;
            match pick(&[(1, 0u8), (1, 1), (4, 2)]) {
                // It turns on its wearer, unless they're shapeshifted.
                Some(1) if !shifted => trigger(map, target, MIND_CONTROL, caster),
                Some(2) => trigger(map, caster, MIND_CONTROL, target),
                _ => {}
            }
        }
        GOBLIN_BOMB if own && item.is_some() => {
            // A malfunction's blast, or the bomb.
            if let Some(o) = pick(&[(1, 13261), (9, 13258)]) {
                trigger(map, caster, o, caster);
            }
        }
        // A transporter does nothing for the dead (Thistle Tea's dummy
        // outcomes).
        GNOMISH_TRANSPORTER | TRANSPORTER_ARRIVAL
            if !map.unit_view(caster).is_some_and(|u| u.alive) => {}
        GNOMISH_TRANSPORTER if guid::is_player(caster) => {
            match pick(&[(2, Some(23441)), (1, Some(23446)), (1, None)]) {
                Some(Some(o)) => trigger(map, caster, o, caster),
                Some(None) => {
                    let (m, x, y, z, o) = SKY_OVER_GADGETZAN;
                    map.teleport(caster, MapKey::world(m), Vector3d { x, y, z }, o);
                }
                None => {}
            }
        }
        TRANSPORTER_ARRIVAL => {
            if let Some(Some(o)) = pick(&[(1, Some(23444)), (4, Some(EVIL_TWIN)), (1, None)]) {
                trigger(map, caster, o, caster);
            }
        }
        crate::game::map::mounts::REINDEER_TRANSFORMATION if own => {
            crate::game::map::mounts::reindeer(map, caster);
        }
        _ => return false,
    }
    true
}

/// An aura `s` from `caster` that, landing, does something else instead
/// (Thistle Tea's `Aura.Script.instant_application`): the Net-o-Matic's
/// backfire marker (13139), which the backfiring net (13119) puts on its
/// thrower, nets the thrower (13138) and leaves no aura. Whether it did.
pub fn instead_of_aura(map: &mut Map, caster: Guid, s: &SpellTemplate) -> bool {
    if s.id != NET_BACKFIRE_MARKER {
        return false;
    }
    trigger(map, caster, NET_BACKFIRE_SELF, caster);
    true
}

/// An aura's amount the data can't say: the Death Ray's hurt a second.
pub fn aura_amount(s: &SpellTemplate) -> Option<i32> {
    (s.id == DEATH_RAY_PERIODIC || script(s, "spell_gdr_periodic"))
        .then(|| 99 + rand::rng().random_range(1..=401))
}

/// A periodic hurt of `h` on `target`: the Death Ray's counts while its
/// user channels, and is nothing once they've stopped.
pub fn periodic_damage(map: &mut Map, target: Guid, h: &Holder, amount: u32) -> u32 {
    if h.spell != DEATH_RAY_PERIODIC {
        return amount;
    }
    let channelling = map
        .caster(target)
        .and_then(|c| c.casting.as_ref())
        .is_some_and(|c| c.spell == DEATH_RAY_CHANNEL && c.channel_until.is_some());
    if !channelling {
        return 0;
    }
    if let Some(p) = map.players.get_mut(&target) {
        p.gadgets.death_ray = p.gadgets.death_ray.saturating_add(amount);
    }
    amount
}

/// `g`'s channel runs its course now: the Death Ray's hurt, ending with
/// it, has its last tick first, and that counts (mangos's
/// `GDRPeriodicDamage` counts every tick; Thistle Tea's delivers the last
/// before the channel ends).
pub fn channel_running_out(map: &mut Map, g: Guid) {
    let death_ray = map
        .caster(g)
        .and_then(|c| c.casting.as_ref())
        .is_some_and(|c| c.spell == DEATH_RAY_CHANNEL && c.channel_until.is_some());
    if death_ray {
        crate::game::map::auras::run_due(map, g, DEATH_RAY_PERIODIC, g);
    }
}

/// A holder came off `target`: the Death Ray's run out lets loose what it
/// cost on its user's target.
pub fn aura_gone(map: &mut Map, target: Guid, h: &Holder, expired: bool) {
    if h.spell != DEATH_RAY_PERIODIC {
        return;
    }
    let Some(p) = map.players.get_mut(&target) else {
        return;
    };
    let charge = std::mem::take(&mut p.gadgets.death_ray);
    let aim = p.fields.get_guid(UNIT_FIELD_TARGET);
    let alive = p.death == crate::game::map::death::Death::Alive;
    if !expired || !alive || charge == 0 || aim.is_zero() || aim == target {
        return;
    }
    if !map.unit_view(aim).is_some_and(|u| u.alive) {
        return;
    }
    crate::game::map::spells::scripts::cast_with_points(
        map,
        target,
        DEATH_RAY_DISCHARGE,
        aim,
        [Some(charge as i32), None, None],
    );
}

/// An engineered guardian's own passive, if it has one.
pub fn guardian_passive(entry: u32) -> Option<u32> {
    GUARDIAN_PASSIVES
        .iter()
        .find(|(e, _)| *e == entry)
        .map(|(_, p)| *p)
}

/// Whether `g` is an engineer's Target Dummy ([`TARGET_DUMMIES`]): it
/// stands where it came, fights no one and answers no blow (vmangos's
/// `npc_target_dummy`, a no-movement AI; Thistle Tea's passive tree).
pub fn target_dummy(map: &Map, g: Guid) -> bool {
    map.creature(g)
        .is_some_and(|c| TARGET_DUMMIES.iter().any(|(e, _, _)| *e == c.entry))
}

/// A creature's wild summon `g` came, of `caster`'s: a target dummy takes
/// their side, fights nothing, puts its passive on, taunts those about it,
/// and is to kill itself in fifteen seconds.
pub fn wild_summoned(map: &mut Map, g: Guid, caster: Guid) {
    let Some(entry) = map.creature(g).map(|c| c.entry) else {
        return;
    };
    let Some((_, passive, taunt)) = TARGET_DUMMIES.iter().find(|(e, ..)| *e == entry) else {
        return;
    };
    let data = map.realm.data.clone();
    let faction = map
        .unit_fields(caster)
        .map(|f| f.get_u32(UNIT_FIELD_FACTIONTEMPLATE));
    if let Some(c) = map.creature_mut(g) {
        if let Some(faction) = faction {
            c.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
            c.hostile_to_players = crate::game::map::ai::hostile_to_players(&data, faction, false);
        }
        let s = c.scripted.get_or_insert_with(Default::default);
        s.passive = true;
        s.reaction = Some(crate::game::map::assistance::Reaction::Passive);
    }
    trigger(map, g, *passive, g);
    trigger(map, g, *taunt, g);
    let when = map.now + DUMMY_LIFETIME;
    map.objects
        .pending_spells
        .push((when, Guid::zero(), SUICIDE, g));
}

/// An engineered guardian came: its passive, at the full health it gives
/// (a dragonling's is nearly all its passive's; Thistle Tea's "dragonlings
/// start with their passive health"), and the Sheep's and the Bomb's own
/// time; a Target Dummy summoned as a guardian, its passive (its pull every
/// three seconds), its spawn's taunt and its fifteen seconds.
pub fn guardian_summoned(map: &mut Map, g: Guid, entry: u32) {
    if let Some((_, passive)) = GUARDIAN_PASSIVES.iter().find(|(e, _)| *e == entry) {
        trigger(map, g, *passive, g);
        if let Some(c) = map.creature_mut(g) {
            let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
            c.fields.set_u32(UNIT_FIELD_HEALTH, max);
        }
    }
    let now = map.now;
    if let Some((_, passive, spawn)) = TARGET_DUMMIES.iter().find(|(e, _, _)| *e == entry) {
        let at = map.position_of(g);
        if let Some(st) = map.creature_mut(g).and_then(|c| c.pet.as_deref_mut()) {
            st.expires = Some(now + TARGET_DUMMY_LIFE);
            st.react = crate::game::map::pets::REACT_PASSIVE;
            st.command = crate::game::map::pets::COMMAND_STAY;
            st.stay_at = at;
        }
        trigger(map, g, *passive, g);
        trigger(map, g, *spawn, g);
        return;
    }
    if let Some((_, life, _)) = GUARDIAN_LIFETIMES.iter().find(|(e, _, _)| *e == entry)
        && let Some(st) = map.creature_mut(g).and_then(|c| c.pet.as_deref_mut())
    {
        st.expires = Some(now + *life);
    }
}

/// A guardian's time is up: the Sheep and the Bomb go off, and lie dead
/// a moment; a Target Dummy falls, in a fight or not (vmangos's
/// `npc_target_dummy`, Thistle Tea's `death_in_combat?`). Whether it was
/// one.
pub fn guardian_expired(map: &mut Map, g: Guid) -> bool {
    if target_dummy(map, g) {
        if let Some(st) = map.creature_mut(g).and_then(|c| c.pet.as_deref_mut()) {
            st.expires = None;
        }
        if map.creature(g).is_some_and(|c| c.alive()) {
            crate::game::map::combat::kill(map, g, g);
        }
        return true;
    }
    let Some(c) = map.creature(g) else {
        return false;
    };
    let Some((_, _, blast)) = GUARDIAN_LIFETIMES.iter().find(|(e, _, _)| *e == c.entry) else {
        return false;
    };
    if !c.alive() {
        return false;
    }
    trigger(map, g, *blast, g);
    if let Some(st) = map.creature_mut(g).and_then(|c| c.pet.as_deref_mut()) {
        st.expires = None;
    }
    if map.creature(g).is_some_and(|c| c.alive()) {
        crate::game::map::combat::kill(map, g, g);
    }
    true
}

/// Party Time!'s dancers dance.
pub fn update(map: &mut Map) {
    let now = map.now;
    let dancers: Vec<(Guid, bool)> = map
        .players
        .values_mut()
        .filter_map(|p| {
            if !p.auras.has_spell(PARTY_TIME) {
                p.gadgets.party_at = None;
                return None;
            }
            match p.gadgets.party_at {
                None => {
                    p.gadgets.party_at = Some(now + PARTY_EVERY);
                    None
                }
                Some(t) if t <= now => {
                    p.gadgets.party_at = Some(now + PARTY_EVERY);
                    let alive = p.death == crate::game::map::death::Death::Alive;
                    // Pitching and a spline's elevation are moving too
                    // (mangos's `MOVEFLAG_MASK_MOVING`, Thistle Tea's
                    // `MovementBlock.moving?`).
                    let f = &p.movement.flags;
                    let moving = crate::game::map::spells::checks::moving(f)
                        || f.get_pitch_up()
                        || f.get_pitch_down()
                        || f.get_spline_elevation().is_some();
                    alive.then_some((p.guid, moving))
                }
                Some(_) => None,
            }
        })
        .collect();
    for (g, moving) in dancers {
        let mut all: Vec<(u32, u32)> = PARTY_EMOTES.iter().map(|e| (1, *e)).collect();
        if !moving {
            all.push((1, DANCE));
        }
        let Some(emote) = pick(&all).and_then(|e| Emote::try_from(e).ok()) else {
            continue;
        };
        if let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE { emote, guid: g }) {
            map.send_to_observers(g, &frame, true);
        }
    }
}
