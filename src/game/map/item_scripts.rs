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
/// The Explosive Sheep and the Goblin Bomb: how long each lasts, and the
/// blast it ends in.
const GUARDIAN_LIFETIMES: [(u32, Duration, u32); 2] = [
    (2675, Duration::from_secs(180), 4050),
    (8937, Duration::from_secs(60), 13259),
];

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

/// An engineered guardian came: its passive, and the Sheep's and the
/// Bomb's own time.
pub fn guardian_summoned(map: &mut Map, g: Guid, entry: u32) {
    if let Some((_, passive)) = GUARDIAN_PASSIVES.iter().find(|(e, _)| *e == entry) {
        trigger(map, g, *passive, g);
    }
    let now = map.now;
    if let Some((_, life, _)) = GUARDIAN_LIFETIMES.iter().find(|(e, _, _)| *e == entry)
        && let Some(st) = map.creature_mut(g).and_then(|c| c.pet.as_deref_mut())
    {
        st.expires = Some(now + *life);
    }
}

/// A guardian's time is up: the Sheep and the Bomb go off, and lie dead
/// a moment. Whether it was one.
pub fn guardian_expired(map: &mut Map, g: Guid) -> bool {
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
                    let moving = crate::game::map::spells::checks::moving(&p.movement.flags);
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
