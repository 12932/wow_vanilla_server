//! Pets, guardians, totems and charmed units: creatures a player (or a
//! creature) controls. A hunter's pet and a warlock's demon have an action
//! bar, follow their master, fight on command or by their stance, and come
//! back with their master after a logout or a journey; a hunter's pet also
//! levels, grows happy or hungry, and grows loyal. Totems stand where the
//! shaman put them and cast their spell; guardians and companions follow
//! and fight (or don't) on their own; charmed and possessed units serve
//! while the aura holds. Thistle Tea's `Core.Pet` is the reference.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::spells::Targets;

mod ai;
mod control;
pub mod eyes;
mod hunter;
mod inherit;
pub(crate) mod possess;
pub mod rods;
mod summon;
mod totems;
mod wild;

/// Whether a unit is held by crowd control a pet leaves be.
#[cfg(test)]
pub(crate) use ai::breakable_cc;
/// A pet sent at a target, or stopped where it is: for scripts.
pub use ai::{attack as attack_target, stop as stop_where};
pub use control::{button, handle, learn, send_spells};
pub use hunter::{
    NPC_FLAG_STABLEMASTER, TAMER_SPELLS, beast_for, change_loyalty, give_pet, list_stable,
    pet_xp_to_level, untrain_offer,
};
/// A family's passives, learned when tamed: for the tests.
#[cfg(test)]
pub(crate) use summon::family_passives;
pub use summon::{dismiss, spellbook, teleporting, unsummon, unsummon_refunding};
pub use wild::remove_guardians;

// Spell effects that make or touch pets.
pub const EFFECT_SUMMON: u32 = 28;
pub const EFFECT_SUMMON_WILD: u32 = 41;
pub const EFFECT_SUMMON_GUARDIAN: u32 = 42;
pub const EFFECT_TAME_CREATURE: u32 = 55;
pub const EFFECT_SUMMON_PET: u32 = 56;
pub const EFFECT_LEARN_SPELL: u32 = 36;
pub const EFFECT_LEARN_PET_SPELL: u32 = 57;
pub const EFFECT_SUMMON_POSSESSED: u32 = 73;
pub const EFFECT_SUMMON_TOTEM: u32 = 74;
pub const EFFECT_SUMMON_TOTEM_SLOT1: u32 = 87;
pub const EFFECT_SUMMON_TOTEM_SLOT4: u32 = 90;
pub const EFFECT_SUMMON_PHANTASM: u32 = 93;
pub const EFFECT_SUMMON_CRITTER: u32 = 97;
pub const EFFECT_FEED_PET: u32 = 101;
pub const EFFECT_DISMISS_PET: u32 = 102;
pub const EFFECT_SUMMON_DEAD_PET: u32 = 109;
pub const EFFECT_DESTROY_ALL_TOTEMS: u32 = 110;
pub const EFFECT_SUMMON_DEMON: u32 = 112;
pub const EFFECT_APPLY_AREA_AURA_PET: u32 = 119;

/// Tame Beast's channel ticks this dummy, which tames.
pub const SPELL_TAME_BEAST: u32 = 1515;
pub const SPELL_TAME_BEAST_TICK: u32 = 13535;
pub const SPELL_TAME_BEAST_TAME: u32 = 13481;

/// Action bar button kinds (the top byte).
pub const ACT_PASSIVE: u32 = 0x01;
pub const ACT_DISABLED: u32 = 0x81;
pub const ACT_ENABLED: u32 = 0xC1;
pub const ACT_COMMAND: u32 = 0x07;
pub const ACT_REACTION: u32 = 0x06;

pub const COMMAND_STAY: u8 = 0;
pub const COMMAND_FOLLOW: u8 = 1;
pub const COMMAND_ATTACK: u8 = 2;
pub const COMMAND_DISMISS: u8 = 3;

pub const REACT_PASSIVE: u8 = 0;
pub const REACT_DEFENSIVE: u8 = 1;
pub const REACT_AGGRESSIVE: u8 = 2;

/// Power indices: focus and happiness.
pub const POWER_FOCUS: u16 = 2;
pub const POWER_HAPPINESS: u16 = 4;
pub const HAPPINESS_MAX: u32 = 1_050_000;
/// Each third of happiness: unhappy, content, happy.
pub const HAPPINESS_LEVEL: u32 = 333_000;

/// The unit is a player's: it is attackable as one is.
pub const UNIT_FLAG_PLAYER_CONTROLLED: u32 = 0x0000_0008;
pub const UNIT_FLAG_POSSESSED: u32 = 0x0100_0000;
/// `UNIT_DYNFLAG_SPECIALINFO`: Beast Lore's view of a beast.
pub const DYNFLAG_SPECIALINFO: u32 = 0x10;

/// How far a pet follows behind, and where it stops trying and jumps to
/// its master.
pub const FOLLOW_DISTANCE: f32 = 2.0;
const TELEPORT_DISTANCE: f32 = 60.0;
/// A dead hunter's pet waits this long, then goes (to be revived).
const CORPSE_STAYS: Duration = Duration::from_secs(60);
/// Other dead pets go soon.
const SUMMON_CORPSE_STAYS: Duration = Duration::from_secs(5);

/// What a controlled creature is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A hunter's tamed beast.
    Hunter,
    /// A warlock's demon, or another summoned pet with a bar.
    Demon,
    /// A summoned helper that fights on its own.
    Guardian,
    Totem,
    /// A companion (mini-pet) that follows.
    Critter,
    /// Charmed (Enslave Demon): the master's pet while it lasts.
    Charmed,
    /// Possessed (Mind Control, Eye of Kilrogg): the master moves it.
    Possessed,
}

impl Kind {
    /// Whether it has an action bar and is the master's pet.
    pub fn controlled(self) -> bool {
        matches!(self, Kind::Hunter | Kind::Demon | Kind::Charmed)
    }
}

/// A controlled creature's state.
#[derive(Debug, Clone)]
pub struct PetState {
    pub kind: Kind,
    pub owner: Guid,
    pub number: u32,
    pub name: String,
    pub renamed: bool,
    /// Every spell it knows, passives too.
    pub spells: Vec<u32>,
    /// Those its family gives (they don't come off when it is untrained).
    pub family_spells: Vec<u32>,
    pub autocast: Vec<u32>,
    pub bar: [u32; 10],
    pub react: u8,
    pub command: u8,
    pub stay_at: Option<Vector3d>,
    /// Where it stands about its master: an angle off their facing.
    pub angle: f32,
    pub summon_spell: u32,
    pub expires: Option<Instant>,
    /// A totem's slot (fire, earth, water, air), and the spell it casts.
    pub totem_slot: Option<usize>,
    pub totem_spell: u32,
    pub next_think: Instant,
    pub regen_at: Instant,
    pub happiness_at: Instant,
    pub loyalty_at: Instant,
    pub loyalty_points: i32,
    pub training_points: i32,
    pub food_mask: u32,
    pub family: u32,
    pub dead_at: Option<Instant>,
    /// When what it has of its master's is next brought in line.
    pub inherit_at: Instant,
    /// A charmed unit's own faction, to go back to.
    pub was_faction: u32,
    pub was_hostile: bool,
    /// Under Eyes of the Beast: the react and command it had before.
    pub eyes: Option<(u8, u8)>,
    /// Called back by Follow: on its way to its master, it takes no
    /// fight up until it is back (mangos's `IsRetreating`, Thistle Tea's
    /// recall).
    pub returning: bool,
    /// A charmed creature's charm spells and the cooldowns rolled for them
    /// (`creature_charm_spells`), in place of their own: milliseconds.
    pub charm_cooldowns: Vec<(u32, u32)>,
    /// The victim its master's Attack named, which a staying pet goes
    /// after (any other it fights only where it stands).
    pub commanded: Option<Guid>,
    /// When its master last sent it to attack: crowd control older than
    /// that it was sent to break (mangos's `GetPetLastAttackCommandTime`).
    pub commanded_at: Option<Instant>,
}

impl PetState {
    pub fn new(kind: Kind, owner: Guid, now: Instant) -> Self {
        Self {
            kind,
            owner,
            number: 0,
            name: String::new(),
            renamed: false,
            spells: Vec::new(),
            family_spells: Vec::new(),
            autocast: Vec::new(),
            bar: [0; 10],
            react: REACT_DEFENSIVE,
            command: COMMAND_FOLLOW,
            stay_at: None,
            angle: std::f32::consts::FRAC_PI_2 * 1.5,
            summon_spell: 0,
            expires: None,
            totem_slot: None,
            totem_spell: 0,
            next_think: now,
            regen_at: now + Duration::from_secs(2),
            happiness_at: now + hunter::HAPPINESS_TICK,
            loyalty_at: now + hunter::LOYALTY_TICK,
            loyalty_points: 0,
            training_points: 0,
            food_mask: 0,
            family: 0,
            dead_at: None,
            inherit_at: now,
            was_faction: 0,
            was_hostile: false,
            eyes: None,
            returning: false,
            charm_cooldowns: Vec::new(),
            commanded: None,
            commanded_at: None,
        }
    }
}

/// What a player controls.
#[derive(Debug, Clone, Default)]
pub struct Owned {
    /// The pet with the action bar: a hunter's pet or a demon.
    pub pet: Option<Guid>,
    /// A charmed or possessed unit, while it serves.
    pub charm: Option<Guid>,
    pub totems: [Option<Guid>; 4],
    /// Totems of no element's slot (a battleground's standard, a ward):
    /// as many as are summoned, none replacing another.
    pub loose_totems: Vec<Guid>,
    pub guardians: Vec<Guid>,
    pub critter: Option<Guid>,
    /// The pet went away for a while (mounted, dead) and comes back.
    pub unsummoned: bool,
    /// Who controls this player, if anyone.
    pub possessed_by: Option<possess::Possession>,
    /// The unit the client moves, when not its own character.
    pub mover: Option<Guid>,
}

/// Whose side `g` fights on: a player's pet, totem, guardian or charmed
/// unit fights on its master's, a controlled player on its controller's;
/// anything else on its own.
pub fn master(map: &Map, g: Guid) -> Guid {
    if let Some(c) = map.creature(g) {
        return c.owner.filter(|o| guid::is_player(*o)).unwrap_or(g);
    }
    map.players
        .get(&g)
        .and_then(|p| p.owned.possessed_by)
        .map_or(g, |pos| pos.master)
}

pub use possess::{not_mover, set_mover};
pub use totems::immune;

/// The pet of `owner` (or the unit they charm), if one is out.
pub fn pet_of(map: &Map, owner: Guid) -> Option<Guid> {
    if let Some(p) = map.players.get(&owner) {
        return p
            .owned
            .charm
            .filter(|g| state(map, *g).is_some_and(|s| s.kind == Kind::Charmed))
            .or(p.owned.pet)
            .filter(|g| map.creature(*g).is_some());
    }
    map.creatures
        .iter()
        .find(|(_, c)| {
            c.owner == Some(owner) && c.pet.as_ref().is_some_and(|s| s.kind.controlled())
        })
        .map(|(_, c)| c.guid)
}

/// Who controls `unit`, if anyone.
pub fn owner_of(map: &Map, unit: Guid) -> Option<Guid> {
    map.creature(unit).and_then(|c| c.owner)
}

/// The state of controlled creature `g`.
/// What a hunter's pet's happiness makes of its melee damage (1 for any
/// other unit).
pub fn happiness_damage(map: &Map, g: Guid) -> f32 {
    match map.creature(g) {
        Some(c) if c.pet.as_deref().is_some_and(|s| s.kind == Kind::Hunter) => {
            hunter::damage_multiplier(c.fields.get_u32(UNIT_FIELD_POWER1 + POWER_HAPPINESS))
        }
        _ => 1.0,
    }
}

pub fn state(map: &Map, g: Guid) -> Option<&PetState> {
    map.creature(g).and_then(|c| c.pet.as_deref())
}

pub fn state_mut(map: &mut Map, g: Guid) -> Option<&mut PetState> {
    map.creature_mut(g).and_then(|c| c.pet.as_deref_mut())
}

/// Whether this module handles effect `index` of `s`.
pub fn handles(s: &SpellTemplate, index: usize) -> bool {
    let e = s.effects[index].effect;
    matches!(
        e,
        EFFECT_SUMMON
            | EFFECT_SUMMON_WILD
            | EFFECT_SUMMON_GUARDIAN
            | EFFECT_TAME_CREATURE
            | EFFECT_SUMMON_PET
            | EFFECT_LEARN_PET_SPELL
            | EFFECT_SUMMON_POSSESSED
            | EFFECT_SUMMON_TOTEM
            | EFFECT_SUMMON_TOTEM_SLOT1
            ..=EFFECT_SUMMON_TOTEM_SLOT4
                | EFFECT_SUMMON_PHANTASM
                | EFFECT_SUMMON_CRITTER
                | EFFECT_FEED_PET
                | EFFECT_DISMISS_PET
                | EFFECT_SUMMON_DEAD_PET
                | EFFECT_DESTROY_ALL_TOTEMS
                | EFFECT_SUMMON_DEMON
                | EFFECT_APPLY_AREA_AURA_PET
    ) || (s.id == SPELL_TAME_BEAST_TICK && e == 3)
        || (s.id == rods::SPELL_DESPAWN_CASTER && e == 3)
        || teaches_pet(s, index)
        || crate::game::map::trinkets::handles(s)
}

/// Whether effect `index` of `s` teaches the caster's pet a spell: a
/// Grimoire's `LEARN_PET_SPELL`, or a hunter's `LEARN_SPELL` at the pet.
pub fn teaches_pet(s: &SpellTemplate, index: usize) -> bool {
    let e = s.effects[index];
    e.trigger_spell != 0
        && (e.effect == EFFECT_LEARN_PET_SPELL
            || (e.effect == EFFECT_LEARN_SPELL && e.target_a == 5))
}

/// The spell `s` teaches a pet, if it teaches one.
pub fn taught(s: &SpellTemplate) -> Option<u32> {
    (0..3)
        .find(|i| teaches_pet(s, *i))
        .map(|i| s.effects[i].trigger_spell)
}

/// Effect `index` of `s`, cast by `caster` on `target`; `triggered` if
/// another spell or an aura cast it.
pub fn effect(
    map: &mut Map,
    caster: Guid,
    target: Guid,
    s: &SpellTemplate,
    index: usize,
    amount: i32,
    triggered: bool,
) {
    let e = s.effects[index];
    // The stacking trinkets' hit scripts ride the same hook.
    if crate::game::map::trinkets::handles(s) {
        crate::game::map::trinkets::effect(map, caster, target, s);
        return;
    }
    if teaches_pet(s, index) {
        let hunter_pet = pet_of(map, caster)
            .and_then(|g| state(map, g))
            .is_some_and(|s| s.kind == Kind::Hunter);
        if hunter_pet {
            hunter::train(map, caster, s);
        } else if let Some(pet) = pet_of(map, caster) {
            // A demon's Grimoire.
            control::learn(map, pet, e.trigger_spell);
        }
        return;
    }
    match e.effect {
        3 if s.id == SPELL_TAME_BEAST_TICK => {
            crate::game::map::spells::cast_triggered(
                map,
                caster,
                SPELL_TAME_BEAST_TAME,
                Some(target),
            );
        }
        3 if s.id == rods::SPELL_DESPAWN_CASTER => rods::despawn(map, caster),
        EFFECT_TAME_CREATURE => hunter::tame(map, caster, target, s.id),
        EFFECT_SUMMON_PET => {
            if e.misc_value > 0 && wild::by_creature(map, caster) {
                // A creature's demon is a guardian of its own, beside any
                // it has (mangos-classic's `EffectSummonPet`, a
                // `GUARDIAN_PET` for a caster no player).
                wild::summon(map, caster, e.misc_value as u32, s, index, 1);
            } else if e.misc_value > 0 {
                summon::summon_demon(map, caster, e.misc_value as u32, s.id);
            } else {
                // Call Pet: the hunter's current pet.
                summon::call_pet(map, caster, s.id);
            }
        }
        EFFECT_DISMISS_PET => {
            if let Some(pet) = pet_of(map, caster) {
                dismiss(map, pet);
            }
        }
        EFFECT_SUMMON_DEAD_PET => hunter::revive(map, caster, amount),
        // Feeding takes the cast's item: see `before_effects`.
        EFFECT_FEED_PET => {}
        EFFECT_SUMMON_TOTEM | EFFECT_SUMMON_TOTEM_SLOT1..=EFFECT_SUMMON_TOTEM_SLOT4 => {
            let slot = match e.effect {
                EFFECT_SUMMON_TOTEM => None,
                n => Some((n - EFFECT_SUMMON_TOTEM_SLOT1) as usize),
            };
            totems::summon_totem(map, caster, e.misc_value.max(0) as u32, s, slot);
        }
        EFFECT_DESTROY_ALL_TOTEMS => totems::destroy_all(map, caster),
        EFFECT_SUMMON
        | EFFECT_SUMMON_WILD
        | EFFECT_SUMMON_GUARDIAN
        | EFFECT_SUMMON_PHANTASM
        | EFFECT_SUMMON_DEMON => {
            let count = amount.clamp(1, 5) as usize;
            // A creature's wild summons and guardians are creatures of
            // their own, and so is a player's wild summon: of its
            // template's faction, each cast another (mangos-classic's
            // `EffectSummonWild`, Thistle Tea's `WildSummon`). A Field
            // Repair Bot stands where it is put.
            if e.effect == EFFECT_SUMMON_WILD
                || (e.effect == EFFECT_SUMMON_GUARDIAN && wild::by_creature(map, caster))
            {
                wild::summon(map, caster, e.misc_value.max(0) as u32, s, index, count);
                return;
            }
            let entry = e.misc_value.max(0) as u32;
            // A player's guardian: a second cast of their own sends those
            // of its entry away, and summons none again unless the spell
            // has a time and a category (mangos-classic's
            // `EffectSummonGuardian`, Thistle Tea's `GuardianOwner`); one
            // another spell triggers adds to them.
            if e.effect == EFFECT_SUMMON_GUARDIAN && guid::is_player(caster) {
                if !triggered {
                    let old = wild::guardians_of(map, caster, entry);
                    let found = !old.is_empty();
                    for g in old {
                        summon::remove(map, g);
                    }
                    if found && !(s.duration_index != 0 && s.category != 0) {
                        return;
                    }
                }
                for _ in 0..count {
                    totems::summon_guardian(map, caster, entry, s, false);
                }
                return;
            }
            for i in 0..count {
                totems::summon_guardian(map, caster, entry, s, i == 0);
            }
        }
        EFFECT_SUMMON_CRITTER => totems::summon_critter(map, caster, e.misc_value.max(0) as u32, s),
        EFFECT_SUMMON_POSSESSED => {
            totems::summon_possessed(map, caster, e.misc_value.max(0) as u32, s)
        }
        EFFECT_APPLY_AREA_AURA_PET => area_aura(map, caster, s, &[(index, amount)]),
        _ => {}
    }
}

/// A pet area aura's effects (each index and amount) on `caster`'s pet,
/// as one holder: a spell of two (Soul Link) puts both on.
/// One a pet casts (Soul Link's 25228, on itself) is on the pet and its
/// master both, from the pet, as mangos's pet area aura of a pet reaches
/// its owner: the warlock's damage then splits to the demon.
pub fn area_aura(map: &mut Map, caster: Guid, s: &SpellTemplate, effects: &[(usize, i32)]) {
    let owner = master(map, caster);
    if owner != caster {
        for target in [caster, owner] {
            if map.unit_view(target).is_some_and(|u| u.alive) {
                let _ = crate::game::map::auras::apply_spell(
                    map,
                    caster,
                    target,
                    s,
                    effects,
                    crate::game::map::auras::Context::default(),
                );
            }
        }
        return;
    }
    if let Some(pet) = pet_of(map, caster)
        && map.unit_view(pet).is_some_and(|u| u.alive)
    {
        let _ = crate::game::map::auras::apply_spell(
            map,
            caster,
            pet,
            s,
            effects,
            crate::game::map::auras::Context::default(),
        );
    }
}

/// A dummy of a spell `spell_pet_auras` names (Soul Link, 19028) at its
/// caster's pet: the pet casts the aura it names for its kind on itself
/// (mangos's `AddPetAura` from `EffectDummy`, Thistle Tea's pet auras);
/// whether it was one.
pub fn pet_aura_dummy(map: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) -> bool {
    if target == caster || master(map, target) != caster {
        return false;
    }
    let entry = map.creature(target).map_or(0, |c| c.entry);
    let aura = map
        .realm
        .data
        .pets
        .owner_auras
        .iter()
        .find(|(spell, for_pet, _)| *spell == s.id && (*for_pet == 0 || *for_pet == entry))
        .map(|(_, _, aura)| *aura);
    let Some(aura) = aura else {
        return false;
    };
    crate::game::map::spells::cast_triggered(map, target, aura, Some(target));
    true
}

/// Before a spell's effects go off: what needs the cast's targets (Feed
/// Pet's food).
pub fn before_effects(map: &mut Map, caster: Guid, s: &SpellTemplate, t: &Targets) {
    // A hunter's pet's ability, used, teaches its master to teach it.
    if state(map, caster).is_some_and(|p| p.kind == Kind::Hunter) {
        hunter::used(map, caster, s.id);
    }
    if let Some(index) = s.effects.iter().position(|e| e.effect == EFFECT_FEED_PET) {
        hunter::feed(map, caster, s, index, t.item);
    }
}

/// Why `caster` can't cast `s` for its pet's sake, if it can't: a
/// `SpellCastResult`.
pub fn check(map: &Map, caster: Guid, s: &SpellTemplate, t: &Targets) -> Option<u8> {
    use wow_world_messages::vanilla::SpellCastResult as R;
    let effects: Vec<u32> = s.effects.iter().map(|e| e.effect).collect();
    if effects.iter().all(|e| *e == 0 || !relevant(*e))
        && !s.effects.iter().any(|e| e.target_a == 5 && e.effect != 0)
        && s.id != SPELL_TAME_BEAST
    {
        return None;
    }
    let has_pet = pet_of(map, caster).is_some();
    let record = map
        .players
        .get(&caster)
        .and_then(|p| p.row.state.pets.current.as_ref());
    let call_pet = s
        .effects
        .iter()
        .any(|e| e.effect == EFFECT_SUMMON_PET && e.misc_value == 0);
    if effects.contains(&EFFECT_SUMMON_PET) {
        if map
            .players
            .get(&caster)
            .is_some_and(|p| p.owned.charm.is_some())
        {
            return Some(R::AlreadyHaveCharm.as_int());
        }
        if call_pet {
            if has_pet {
                return Some(R::AlreadyHaveSummon.as_int());
            }
            match record {
                None => return Some(R::NoPet.as_int()),
                Some(r) if r.dead => return Some(R::TargetsDead.as_int()),
                _ => {}
            }
        }
    }
    if (s.id == SPELL_TAME_BEAST || effects.contains(&EFFECT_TAME_CREATURE))
        && let Some(reason) = hunter::tame_check(map, caster, t.unit)
    {
        if let Some(p) = map.players.get(&caster) {
            p.outbound
                .send(&wow_world_messages::vanilla::SMSG_PET_TAME_FAILURE { reason });
        }
        return Some(R::DontReport.as_int());
    }
    if effects.contains(&EFFECT_SUMMON_DEAD_PET) {
        let dead_out =
            pet_of(map, caster).is_some_and(|g| map.unit_view(g).is_some_and(|u| !u.alive));
        if !dead_out && !record.is_some_and(|r| r.dead) {
            return Some(R::TargetNotDead.as_int());
        }
    }
    if effects.contains(&EFFECT_DISMISS_PET) && !has_pet {
        return Some(R::NoPet.as_int());
    }
    if effects.contains(&EFFECT_FEED_PET) {
        return hunter::feed_check(map, caster, t.item);
    }
    if taught(s).is_some()
        && pet_of(map, caster)
            .and_then(|g| state(map, g))
            .is_none_or(|s| s.kind == Kind::Hunter)
    {
        return hunter::train_check(map, caster, s);
    }
    // A spell aimed at the pet (Mend Pet) needs one, alive.
    if s.effects.iter().any(|e| e.effect != 0 && e.target_a == 5)
        && !effects.contains(&EFFECT_DISMISS_PET)
    {
        match pet_of(map, caster) {
            None => return Some(R::NoPet.as_int()),
            Some(g) if !map.unit_view(g).is_some_and(|u| u.alive) => {
                return Some(R::TargetsDead.as_int());
            }
            _ => {}
        }
    }
    None
}

fn relevant(effect: u32) -> bool {
    matches!(
        effect,
        EFFECT_SUMMON_PET
            | EFFECT_TAME_CREATURE
            | EFFECT_SUMMON_DEAD_PET
            | EFFECT_DISMISS_PET
            | EFFECT_FEED_PET
            | EFFECT_LEARN_PET_SPELL
    )
}

/// The player arrived in the map (logging in, or from another): their pet
/// comes with them.
pub fn owner_arrived(map: &mut Map, owner: Guid) {
    let Some(p) = map.players.get(&owner) else {
        return;
    };
    let pets = &p.row.state.pets;
    let Some(r) = pets.current.clone().filter(|r| pets.active && !r.dead) else {
        return;
    };
    if p.death != crate::game::map::death::Death::Alive || summon::must_go(p) {
        if let Some(p) = map.players.get_mut(&owner) {
            p.owned.unsummoned = true;
        }
        return;
    }
    summon::from_record(map, owner, &r);
}

/// The player leaves the map (logging out, or for another): what they
/// control is kept (their pet) or let go (totems, guardians, charms).
pub fn owner_leaving(map: &mut Map, owner: Guid) {
    sync_record(map, owner);
    possess::release(map, owner);
    if let Some(c) = map
        .players
        .get(&owner)
        .and_then(|p| p.owned.charm)
        .filter(|c| map.players.contains_key(c))
    {
        possess::release(map, c);
    }
    let Some(p) = map.players.get(&owner) else {
        return;
    };
    let owned = p.owned.clone();
    if let Some(c) = owned.charm {
        release(map, owner, c);
    }
    for g in owned
        .totems
        .iter()
        .flatten()
        .chain(owned.loose_totems.iter())
        .chain(owned.guardians.iter())
        .chain(owned.critter.iter())
    {
        summon::remove(map, *g);
    }
    if let Some(pet) = owned.pet {
        summon::remove(map, pet);
    }
    if let Some(p) = map.players.get_mut(&owner) {
        let unsummoned = p.owned.unsummoned;
        p.owned = Owned {
            unsummoned,
            ..Default::default()
        };
    }
}

/// The player's pet, as it is now, into their stored character.
pub fn sync_record(map: &mut Map, owner: Guid) {
    let Some(pet) = map.players.get(&owner).and_then(|p| p.owned.pet) else {
        return;
    };
    let Some(r) = summon::record_of(map, pet) else {
        return;
    };
    if let Some(p) = map.players.get_mut(&owner) {
        p.row.state.pets.current = Some(r);
        p.row.state.pets.active = true;
    }
}

/// `creature` was attacked by `attacker`. A controlled creature answers by
/// its stance; a creature attacked by another's pet or totem also counts
/// the master. True when it is handled here.
pub fn on_attacked(map: &mut Map, creature: Guid, attacker: Guid) -> bool {
    if let Some(s) = state(map, creature) {
        if !matches!(s.kind, Kind::Totem | Kind::Critter | Kind::Possessed) {
            ai::defend(map, creature, attacker);
        }
        return true;
    }
    // Not a master who can't join it: dead, or lying in a believed Feign
    // Death (mangos's `CanJoinInAttacking`).
    if let Some(master) = owner_of(map, attacker).filter(|m| {
        guid::is_player(*m)
            && map.unit_view(*m).is_some_and(|u| u.alive)
            && !crate::game::map::combat::feigning(map, *m)
    }) {
        let realm = map.realm.clone();
        if let Some(c) = map.creature_mut(creature) {
            if c.tapped_by.is_none() {
                crate::game::map::tap::take(&realm, c, master);
            }
            if c.threat.iter().all(|(g, _)| *g != master) {
                c.threat.push((master, 0.0));
            }
        }
        // The master is in the fight too.
        if let Some(p) = map.players.get_mut(&master) {
            p.combat.attackers.insert(creature);
        }
        crate::game::map::combat::enter(map, master, false);
    }
    false
}

/// A controlled creature died at `killer`'s hand. True when it is handled
/// here (no loot, no reward).
pub fn died(map: &mut Map, killer: Guid, victim: Guid) -> bool {
    let Some(s) = state(map, victim) else {
        return false;
    };
    let (kind, owner) = (s.kind, s.owner);
    let _ = killer;
    match kind {
        Kind::Charmed | Kind::Possessed => {
            // Death lets it go, and it dies as itself.
            release(map, owner, victim);
            false
        }
        _ => {
            summon::die(map, victim);
            true
        }
    }
}

/// How a hunter's pet shares its master's kill (Thistle Tea's
/// `PetExperience`): alone, from its master's base and its own level; in a
/// group, its master's share, while no higher than the kill's best level.
#[derive(Debug, Clone, Copy)]
pub enum KillShare {
    Solo,
    Group { xp: u32, max_level: u32 },
}

/// The master's kill: their hunter's pet shares the experience.
pub fn owner_killed(map: &mut Map, owner: Guid, victim: Guid, share: KillShare) {
    hunter::kill_xp(map, owner, victim, share);
}

/// `master` takes control of `unit` (charm or possession).
pub fn charmed(map: &mut Map, master: Guid, unit: Guid, possess: bool) {
    if map.players.contains_key(&unit) {
        possess::take(map, master, unit);
        return;
    }
    if !guid::is_player(master) || map.creature(unit).is_none() {
        return;
    }
    let now = map.now;
    // One pet at a time: a demon steps aside.
    if !possess && let Some(pet) = map.players.get(&master).and_then(|p| p.owned.pet) {
        dismiss(map, pet);
    }
    crate::game::map::combat::leave_combat(map, unit);
    crate::game::map::spells::interrupt(map, unit);
    // Its own way ends where it is (Thistle Tea's `ControlSync` stops the
    // path; mangos's charm puts it to follow its charmer).
    crate::game::map::ai::halt(map, unit);
    let faction = map
        .players
        .get(&master)
        .map(|p| p.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE))
        .unwrap_or(0);
    let data = map.realm.data.clone();
    map.rare.pet = true;
    let Some(c) = map.creature_mut(unit) else {
        return;
    };
    let kind = if possess {
        Kind::Possessed
    } else {
        Kind::Charmed
    };
    let mut s = PetState::new(kind, master, now);
    s.was_faction = c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
    s.was_hostile = c.hostile_to_players;
    s.name = data
        .creature_template(c.entry)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    match summon::pick_charm_spells(&data, c.entry, &mut rand::rng()) {
        Some(picked) => {
            s.spells = picked.iter().map(|(spell, _)| *spell).collect();
            s.charm_cooldowns = picked;
        }
        None => s.spells = summon::charm_spells(&data, c.entry),
    }
    s.bar = control::default_bar(&data, &s.spells, &[], possess);
    s.react = if possess {
        REACT_PASSIVE
    } else {
        REACT_DEFENSIVE
    };
    c.owner = Some(master);
    c.pet = Some(Box::new(s));
    c.threat.clear();
    crate::game::map::tap::clear(c);
    c.evading = false;
    c.hostile_to_players = false;
    c.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
    c.fields.set_guid(UNIT_FIELD_CHARMEDBY, master);
    c.fields
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PLAYER_CONTROLLED, true);
    if possess {
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_POSSESSED, true);
    }
    // Whoever fought it forgets it.
    crate::game::map::ai::forget(map, unit);
    if let Some(p) = map.players.get_mut(&master) {
        p.owned.charm = Some(unit);
        p.fields.set_guid(UNIT_FIELD_CHARM, unit);
        if possess {
            p.fields.set_guid(PLAYER_FARSIGHT, unit);
            p.outbound
                .send(&wow_world_messages::vanilla::SMSG_CLIENT_CONTROL_UPDATE {
                    guid: unit,
                    allow_movement: true,
                });
        }
    }
    send_spells(map, master);
}

/// The charm or possession of `unit` ended.
pub fn uncharmed(map: &mut Map, master: Guid, unit: Guid) {
    if map
        .players
        .get(&unit)
        .and_then(|p| p.owned.possessed_by)
        .is_some_and(|pos| pos.master == master)
    {
        possess::release(map, unit);
        return;
    }
    let charmed = state(map, unit)
        .is_some_and(|s| matches!(s.kind, Kind::Charmed | Kind::Possessed) && s.owner == master);
    if charmed {
        release(map, master, unit);
    }
}

/// Let a charmed or possessed unit go: its own faction again, and (alive)
/// it turns on its master.
fn release(map: &mut Map, master: Guid, unit: Guid) {
    let_go(map, master, unit, true);
}

/// Let a charmed or possessed unit go, turning on its master if `turn`.
fn let_go(map: &mut Map, master: Guid, unit: Guid, turn: bool) {
    let Some(s) = map.creature_mut(unit).and_then(|c| c.pet.take()) else {
        return;
    };
    let possess = s.kind == Kind::Possessed;
    let summoned = s.summon_spell != 0;
    if let Some(c) = map.creature_mut(unit) {
        c.owner = None;
        c.fields.set_u32(UNIT_FIELD_FACTIONTEMPLATE, s.was_faction);
        c.hostile_to_players = s.was_hostile;
        c.fields.set_guid(UNIT_FIELD_CHARMEDBY, Guid::zero());
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PLAYER_CONTROLLED, false);
        c.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_POSSESSED, false);
    }
    crate::game::map::combat::leave_combat(map, unit);
    if let Some(p) = map.players.get_mut(&master) {
        if p.owned.charm == Some(unit) {
            p.owned.charm = None;
        }
        if p.owned.mover == Some(unit) {
            p.owned.mover = None;
        }
        p.fields.set_guid(UNIT_FIELD_CHARM, Guid::zero());
        if possess {
            p.fields.set_guid(PLAYER_FARSIGHT, Guid::zero());
            p.outbound
                .send(&wow_world_messages::vanilla::SMSG_CLIENT_CONTROL_UPDATE {
                    guid: master,
                    allow_movement: true,
                });
        }
    }
    // The pet the master had, back again; or no bar.
    if map
        .players
        .get(&master)
        .is_some_and(|p| p.owned.pet.is_some())
    {
        send_spells(map, master);
    } else if let Some(p) = map.players.get(&master) {
        p.outbound
            .send(&wow_world_messages::vanilla::SMSG_PET_SPELLS {
                pet: Guid::zero(),
                action_bars: None,
            });
    }
    // The aura that held it comes off, if it is still on.
    let holding = crate::game::map::auras::auras(map, unit).and_then(|a| {
        a.holders
            .iter()
            .find(|h| {
                h.caster == master
                    && (h.has(crate::game::map::auras::AURA_MOD_CHARM)
                        || h.has(crate::game::map::auras::AURA_MOD_POSSESS))
            })
            .map(|h| h.spell)
    });
    if let Some(spell) = holding {
        crate::game::map::auras::remove_spell(map, unit, spell, Some(master));
    }
    if summoned {
        // A summoned eye goes when its master lets it go.
        summon::remove(map, unit);
        return;
    }
    let alive = map.unit_view(unit).is_some_and(|u| u.alive);
    if turn && alive && crate::game::map::combat::can_attack(map, unit, master) {
        crate::game::map::ai::attacked(map, unit, master);
        // Its health's worth of threat, as for being charmed (mangos's
        // `Uncharm`, Thistle Tea's `turn_on_controller`).
        let max = map
            .unit_fields(unit)
            .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXHEALTH));
        crate::game::map::ai::add_threat(map, unit, master, max as f32);
    }
}

/// Movement from a player's client: a possessed unit's when the client
/// has made it its active mover. True when it is handled here.
pub fn move_possessed(
    map: &mut Map,
    player: Guid,
    info: &wow_world_messages::vanilla::MovementInfo,
    relay: crate::game::map::movement::Relay,
) -> bool {
    possess::moved(map, player, info, relay)
}

/// Run what every player controls.
pub fn update(map: &mut Map) {
    let now = map.now;
    let mut controlled: Vec<Guid> = Vec::new();
    if map.rare.pet {
        controlled = map
            .creatures
            .iter()
            .filter(|(_, c)| c.pet.is_some())
            .map(|(_, c)| c.guid)
            .collect();
        map.rare.pet = !controlled.is_empty();
    } else {
        debug_assert!(
            map.creatures.iter().all(|(_, c)| c.pet.is_none()),
            "a pet unnoted (RareStates)"
        );
    }
    for g in controlled {
        ai::step(map, g, now);
    }
    possess::update(map);
    // Pets away for a while come back when their master can have them;
    // those whose master mounted or died go away for a while.
    let waiting: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.owned.unsummoned || (p.owned.pet.is_some() && summon::must_go(p)))
        .map(|p| p.guid)
        .collect();
    for g in waiting {
        summon::temporary(map, g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_pack_the_action_in_the_low_bytes() {
        assert_eq!(control::button(2, ACT_COMMAND), 0x0700_0002);
        assert_eq!(control::button(3110, ACT_ENABLED), 0xC100_0C26);
        assert_eq!(control::unpack(0xC100_0C26), (3110, ACT_ENABLED));
    }
}

/// What the tests reach of a hunter's pet and the summons.
#[cfg(test)]
pub mod testing {
    pub use super::ai::regen;
    pub use super::control::good_name;
    pub use super::hunter::{
        HAPPINESS_TICK, change_happiness, change_loyalty, gain_xp, kill_xp, on_death,
        pet_xp_to_level, revive, show_training_points, tame, tick, train_check, untrain,
    };
    pub use super::summon::{
        UNIT_FLAG_PET_ABANDON, UNIT_FLAG_PET_RENAME, die, from_record, record_of, spawn,
        summon_demon,
    };
}
