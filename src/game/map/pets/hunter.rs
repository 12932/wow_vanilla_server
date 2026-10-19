//! A hunter's pet: taming, experience and levels, happiness and loyalty,
//! feeding, reviving, untraining, and the stable. Thistle Tea's
//! `PetHappiness`, `PetLoyalty`, `PetProgression`, `PetUntraining` and
//! `PetStable` are followed.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::summon::{self, KIND_HUNTER};
use super::*;
use crate::game::data::GameData;
use crate::game::map::Map;
use crate::game::store::PetRecord;

pub const HAPPINESS_TICK: Duration = Duration::from_millis(7_500);
pub const LOYALTY_TICK: Duration = Duration::from_secs(12);
/// Loyalty points each rank holds, and what a new rank starts with.
const LOYALTY_THRESHOLDS: [i32; 6] = [5_500, 11_500, 17_000, 23_500, 31_000, 39_500];
const LOYALTY_START: [i32; 6] = [2_000, 4_500, 7_000, 10_000, 13_500, 17_500];
/// A newly tamed beast: rebellious, and half unhappy.
const TAMED_HAPPINESS: u32 = 166_500;
const MAX_LEVEL: u8 = 60;
/// `UNIT_NPC_FLAG_STABLEMASTER`.
pub const NPC_FLAG_STABLEMASTER: u32 = 0x2000;
/// A creature type flag: tameable.
const TYPE_FLAG_TAMEABLE: u32 = 0x1;

/// The experience a hunter's pet needs for its next level: a quarter of a
/// player's.
pub fn pet_xp_to_level(data: &GameData, level: u8) -> u32 {
    data.xp_to_level(level) / 4
}

/// Happiness's hold on a pet's damage.
pub fn damage_multiplier(happiness: u32) -> f32 {
    if happiness < HAPPINESS_LEVEL {
        0.75
    } else if happiness < HAPPINESS_LEVEL * 2 {
        1.0
    } else {
        1.25
    }
}

/// Training points as the client shows them (vmangos's `GetDispTP`).
pub fn show_training_points(c: &mut crate::game::map::creature::Creature) {
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL);
    let Some(s) = c.pet.as_deref() else {
        return;
    };
    let shown = if level < 10 {
        0
    } else if s.training_points < 0 {
        (-s.training_points) as u32
    } else {
        (-(s.training_points + 1)) as u32
    };
    c.fields.set_u32(UNIT_TRAINING_POINTS, shown);
}

/// Why `hunter` can't tame `target`, if they can't.
pub fn tame_check(map: &Map, hunter: Guid, target: Option<Guid>) -> Option<PetTameFailureReason> {
    let p = map.players.get(&hunter)?;
    let Some(t) = target.and_then(|t| map.creature(t)) else {
        return Some(PetTameFailureReason::InvalidCreature);
    };
    if !t.alive() {
        return Some(PetTameFailureReason::Dead);
    }
    if t.owner.is_some() {
        return Some(PetTameFailureReason::CreatureAlreadyOwned);
    }
    let tameable = map
        .realm
        .data
        .creature_template(t.entry)
        .is_some_and(|tt| tt.type_flags & TYPE_FLAG_TAMEABLE != 0);
    if !tameable {
        return Some(PetTameFailureReason::NotTameable);
    }
    if t.fields.get_u32(UNIT_FIELD_LEVEL) > u32::from(p.level()) {
        return Some(PetTameFailureReason::TooHighLevel);
    }
    if p.owned.pet.is_some() || p.owned.charm.is_some() {
        return Some(PetTameFailureReason::AnotherSummonActive);
    }
    if p.row.state.pets.current.is_some() {
        return Some(PetTameFailureReason::TooMany);
    }
    None
}

/// The tame lands: the beast leaves its wild life and becomes the hunter's
/// pet where it stood.
pub fn tame(map: &mut Map, hunter: Guid, beast: Guid, spell: u32) {
    if let Some(reason) = tame_check(map, hunter, Some(beast)) {
        if let Some(p) = map.players.get(&hunter) {
            p.outbound.send(&SMSG_PET_TAME_FAILURE { reason });
        }
        return;
    }
    let now = map.now;
    let Some(c) = map.creature(beast) else {
        return;
    };
    let entry = c.entry;
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL).clamp(1, 60) as u8;
    let display = c.fields.get_u32(UNIT_FIELD_NATIVEDISPLAYID);
    let r = tamed(map, hunter, entry, level, display, spell);
    // The wild beast goes, to come back in its time.
    crate::game::map::combat::leave_combat(map, beast);
    crate::game::map::ai::forget(map, beast);
    if let Some(p) = map.players.get_mut(&hunter) {
        p.combat.attackers.remove(&beast);
        p.row.state.pets.current = Some(r.clone());
        p.row.state.pets.active = true;
    }
    if let Some(c) = map.creature_mut(beast) {
        c.threat.clear();
        c.combat = Default::default();
        c.fields.set_u32(UNIT_FIELD_HEALTH, 0);
        c.life = crate::game::map::creature::Life::Dead {
            corpse_until: now,
            respawn_at: now + Duration::from_secs(300),
        };
        map.dead_creatures.push(beast);
    }
    summon::from_record(map, hunter, &r);
}

/// The record of a beast of `entry` just tamed by `hunter` with `spell`, at
/// `level`, looking as `display` (0: as its template has it).
fn tamed(
    map: &mut Map,
    hunter: Guid,
    entry: u32,
    level: u8,
    display: u32,
    spell: u32,
) -> PetRecord {
    let data = map.realm.data.clone();
    let template = data.creature_template(entry).cloned().unwrap_or_default();
    let number = summon::next_number(map, hunter);
    let spells = summon::spellbook(&data, entry, level, true, template.family);
    let autocast: Vec<u32> = spells
        .iter()
        .copied()
        .filter(|s| data.spells.get(*s).is_some_and(control::autocastable))
        .collect();
    // What it knows is paid for out of the points its loyalty will bring:
    // a fresh pet owes them (mangos's `InitPetCreateSpells`, `SetTP(-used)`;
    // Thistle Tea's spell profile).
    let owed: i32 = spells
        .iter()
        .map(|s| {
            ability_entries(&data, *s)
                .first()
                .map_or(0, |a| a.cost as i32)
        })
        .sum();
    PetRecord {
        number,
        entry,
        kind: KIND_HUNTER,
        name: template.name.clone(),
        level,
        happiness: TAMED_HAPPINESS,
        loyalty: 1,
        loyalty_points: 1_000,
        training_points: -owed,
        bar: control::default_bar(&data, &spells, &autocast, false).to_vec(),
        spells,
        autocast,
        react: REACT_DEFENSIVE,
        command: COMMAND_FOLLOW,
        summon_spell: spell,
        display_id: display,
        family: template.family,
        ..Default::default()
    }
}

/// The hunter's pet spells a tamer learns (1.12's "Taming the Beast"
/// quests'): Tame Beast, Call Pet, Dismiss Pet, Revive Pet and Feed Pet.
pub const TAMER_SPELLS: [u32; 5] = [SPELL_TAME_BEAST, 883, 2641, 982, 6991];

/// A tameable beast for a hunter of `level`: a cat or a wolf of the level,
/// a common one (the lowest entry, as the world has several); else any
/// tameable beast of the level; else any cat or wolf.
pub fn beast_for(data: &GameData, level: u8) -> Option<u32> {
    let tameable =
        |t: &&crate::game::data::CreatureTemplate| t.type_flags & TYPE_FLAG_TAMEABLE != 0;
    let of_level = |t: &&crate::game::data::CreatureTemplate| {
        t.level_min <= level && level <= t.level_max && t.rank == 0
    };
    let cat_or_wolf = |t: &&crate::game::data::CreatureTemplate| matches!(t.family, 1 | 2);
    let pick = |f: &dyn Fn(&&crate::game::data::CreatureTemplate) -> bool| {
        data.creature_templates
            .values()
            .filter(tameable)
            .filter(|t| f(t))
            .map(|t| t.entry)
            .min()
    };
    pick(&|t| of_level(t) && cat_or_wolf(t))
        .or_else(|| pick(&of_level))
        .or_else(|| pick(&cat_or_wolf))
}

/// `.pet`: `hunter` given a tamed beast of `entry` at `level`, out at their
/// side, as Tame Beast's (mangos's `.npc tame` casts its effect on a
/// selected beast; here the beast needs no finding): the pet they had, out
/// or put away, is let go. They learn the tamer's spells they lack. The
/// pet's name, or why there is none.
pub fn give_pet(map: &mut Map, hunter: Guid, entry: u32, level: u8) -> Result<String, String> {
    let data = map.realm.data.clone();
    let Some(template) = data.creature_template(entry) else {
        return Err(format!("No creature {entry}."));
    };
    if template.type_flags & TYPE_FLAG_TAMEABLE == 0 {
        return Err(format!("{} isn't tameable.", template.name));
    }
    if map.players.get(&hunter).is_none_or(|p| p.row.class != 3) {
        return Err("Only a hunter has a pet.".into());
    }
    if let Some(old) = map.players.get(&hunter).and_then(|p| p.owned.pet) {
        summon::unsummon(map, old, false);
    }
    for spell in TAMER_SPELLS {
        if map
            .players
            .get(&hunter)
            .is_some_and(|p| !p.spells.contains(&spell))
        {
            crate::game::map::spells::learn(map, hunter, spell);
        }
    }
    let r = tamed(
        map,
        hunter,
        entry,
        level.clamp(1, MAX_LEVEL),
        0,
        SPELL_TAME_BEAST_TAME,
    );
    if let Some(p) = map.players.get_mut(&hunter) {
        p.row.state.pets.current = Some(r.clone());
        p.row.state.pets.active = true;
    }
    summon::from_record(map, hunter, &r)
        .map(|_| format!("Tamed {}, level {}.", r.name, r.level))
        .ok_or_else(|| "The pet couldn't come.".into())
}

/// The hunter's kill: their pet earns the experience of it, if it is below
/// its master.
pub fn kill_xp(map: &mut Map, owner: Guid, victim: Guid, share: super::KillShare) {
    let Some(pet) = map.players.get(&owner).and_then(|p| p.owned.pet) else {
        return;
    };
    let Some(s) = state(map, pet) else {
        return;
    };
    if s.kind != Kind::Hunter {
        return;
    }
    let Some(c) = map.creature(pet).filter(|c| c.alive()) else {
        return;
    };
    let owner_level = map.players.get(&owner).map(|p| p.level()).unwrap_or(1);
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL) as u8;
    if level >= owner_level.min(MAX_LEVEL) {
        return;
    }
    let Some(v) = map.creature(victim) else {
        return;
    };
    let mob_level = v.fields.get_u32(UNIT_FIELD_LEVEL);
    let xp = match share {
        super::KillShare::Solo => crate::game::map::combat::kill_xp_with(
            u32::from(owner_level),
            u32::from(level),
            mob_level,
            &crate::game::map::combat::kill_options(map, victim),
        ),
        super::KillShare::Group { xp, max_level } => {
            if u32::from(level) <= max_level {
                xp
            } else {
                0
            }
        }
    };
    if xp == 0 {
        return;
    }
    gain_xp(map, pet, xp, owner_level);
    // A kill bonds them more.
    let rank = map
        .creature(pet)
        .map(|c| i32::from(c.fields.get_byte(UNIT_FIELD_BYTES_1, 1)))
        .unwrap_or(1);
    change_loyalty(map, pet, (100 - level as i32) / 10 + 6 - rank);
}

/// `amount` experience for a hunter's pet, levels and all.
pub fn gain_xp(map: &mut Map, pet: Guid, amount: u32, owner_level: u8) {
    if amount == 0 {
        return;
    }
    let data = map.realm.data.clone();
    let cap = owner_level.min(MAX_LEVEL);
    let Some(c) = map.creature_mut(pet) else {
        return;
    };
    let mut xp = c.fields.get_u32(UNIT_FIELD_PETEXPERIENCE) + amount;
    let mut level = c.fields.get_u32(UNIT_FIELD_LEVEL) as u8;
    let mut levels = 0;
    while level < cap {
        let need = pet_xp_to_level(&data, level);
        if need == 0 || xp < need {
            break;
        }
        xp -= need;
        level += 1;
        levels += 1;
    }
    // At its master's level, what is over is lost (mangos's and vmangos's
    // `GivePetXP`, Thistle Tea's `PetProgression.gain`).
    if level >= cap {
        xp = 0;
    }
    c.fields.set_u32(UNIT_FIELD_PETEXPERIENCE, xp);
    if levels > 0 {
        let rank = i32::from(c.fields.get_byte(UNIT_FIELD_BYTES_1, 1).max(1));
        if let Some(s) = c.pet.as_deref_mut() {
            s.training_points += (rank - 1) * levels;
        }
        summon::apply_level(&data, c, level);
        let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        c.fields.set_u32(UNIT_FIELD_HEALTH, max);
        c.fields.set_u32(UNIT_FIELD_POWER1 + POWER_FOCUS, 100);
        show_training_points(c);
        // Its spells' new ranks.
        let (entry, family) = (c.entry, c.pet.as_deref().map(|s| s.family).unwrap_or(0));
        let known = c
            .pet
            .as_deref()
            .map(|s| s.spells.clone())
            .unwrap_or_default();
        let ranks = summon::spellbook(&data, entry, level, true, family);
        for spell in ranks {
            if !known.contains(&spell) && known.iter().any(|k| data.spells.same_kind(*k, spell)) {
                control::learn(map, pet, spell);
            }
        }
    }
    if let Some(owner) = owner_of(map, pet) {
        sync_record(map, owner);
    }
}

/// Happiness and loyalty, as their times come.
pub fn tick(map: &mut Map, pet: Guid, now: Instant) {
    let Some(s) = state(map, pet) else {
        return;
    };
    let (happy_due, loyal_due) = (s.happiness_at <= now, s.loyalty_at <= now);
    if happy_due {
        let in_combat = map.unit_fields(pet).is_some_and(|f| {
            f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        });
        let rank = map
            .creature(pet)
            .map(|c| u32::from(c.fields.get_byte(UNIT_FIELD_BYTES_1, 1).clamp(1, 6)))
            .unwrap_or(1);
        let mut decay = (140u32 >> rank) * 125;
        if in_combat {
            decay = decay * 3 / 2;
        }
        change_happiness(map, pet, -(decay as i32));
        if let Some(s) = state_mut(map, pet) {
            s.happiness_at = now + HAPPINESS_TICK;
        }
    }
    if loyal_due {
        let happiness = map
            .unit_fields(pet)
            .map(|f| f.get_u32(UNIT_FIELD_POWER1 + POWER_HAPPINESS))
            .unwrap_or(0);
        let amount = if happiness >= HAPPINESS_LEVEL * 2 {
            20
        } else if happiness >= HAPPINESS_LEVEL {
            10
        } else {
            -20
        };
        if let Some(s) = state_mut(map, pet) {
            s.loyalty_at = now + LOYALTY_TICK;
        }
        change_loyalty(map, pet, amount);
    }
}

/// Happiness up or down; its damage follows its mood.
pub fn change_happiness(map: &mut Map, pet: Guid, amount: i32) {
    let data = map.realm.data.clone();
    let Some(c) = map.creature_mut(pet) else {
        return;
    };
    let before = c.fields.get_u32(UNIT_FIELD_POWER1 + POWER_HAPPINESS);
    let after = (before as i64 + i64::from(amount)).clamp(0, i64::from(HAPPINESS_MAX)) as u32;
    c.fields.set_u32(UNIT_FIELD_POWER1 + POWER_HAPPINESS, after);
    if damage_multiplier(before) != damage_multiplier(after) {
        let level = c.fields.get_u32(UNIT_FIELD_LEVEL) as u8;
        let health = c.fields.get_u32(UNIT_FIELD_HEALTH);
        summon::apply_level(&data, c, level);
        let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        c.fields.set_u32(UNIT_FIELD_HEALTH, health.min(max));
    }
}

/// Loyalty points up or down: a rank more, a rank less, or the bond
/// broken and the pet gone.
pub fn change_loyalty(map: &mut Map, pet: Guid, amount: i32) {
    let Some(c) = map.creature_mut(pet) else {
        return;
    };
    let rank = c.fields.get_byte(UNIT_FIELD_BYTES_1, 1).clamp(1, 6);
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL) as i32;
    let Some(s) = c.pet.as_deref_mut() else {
        return;
    };
    let points = s.loyalty_points + amount;
    let r = usize::from(rank);
    if rank == 6 && points > LOYALTY_THRESHOLDS[5] {
        return;
    }
    if points < 0 && rank == 1 {
        // The bond is broken: the pet runs away.
        let owner = s.owner;
        if let Some(p) = map.players.get(&owner) {
            crate::game::map::chat::send_system(&p.outbound, "Your pet has run away.");
            p.outbound.send(&SMSG_PET_BROKEN {});
        }
        summon::remove(map, pet);
        if let Some(p) = map.players.get_mut(&owner) {
            p.row.state.pets.current = None;
            p.row.state.pets.active = false;
        }
        return;
    }
    if points < 0 {
        s.loyalty_points = LOYALTY_START[r - 2];
        s.training_points -= level;
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 1, rank - 1);
    } else if points > LOYALTY_THRESHOLDS[r - 1] && rank < 6 {
        s.loyalty_points = LOYALTY_START[r];
        s.training_points += level;
        c.fields.set_byte(UNIT_FIELD_BYTES_1, 1, rank + 1);
    } else {
        s.loyalty_points = points;
    }
    show_training_points(c);
}

/// A hunter's pet died: a third of its happiness goes (not in a
/// battleground).
pub fn on_death(map: &mut Map, pet: Guid) {
    if map.battleground.is_none() {
        change_happiness(map, pet, -(HAPPINESS_LEVEL as i32));
    }
}

/// Revive Pet: the dead pet up again, at `percent` of its health, beside
/// its master.
pub fn revive(map: &mut Map, hunter: Guid, percent: i32) {
    let fraction = (percent.clamp(1, 100) as f32) / 100.0;
    if let Some(pet) = map.players.get(&hunter).and_then(|p| p.owned.pet)
        && map.unit_view(pet).is_some_and(|u| !u.alive)
    {
        // A corpse two yards off or more comes to its master's side
        // (mangos's `EffectSummonDeadPet`).
        if let (Some(a), Some(b)) = (map.position_of(hunter), map.position_of(pet))
            && crate::game::map::combat::dist(a, b) >= 2.0
        {
            let (at, o) = summon::beside(map, hunter);
            crate::game::map::scripts::place(map, pet, at, o);
        }
        summon::resurrect(map, pet, fraction);
        sync_record(map, hunter);
        return;
    }
    let Some(mut r) = map
        .players
        .get(&hunter)
        .and_then(|p| p.row.state.pets.current.clone())
        .filter(|r| r.dead)
    else {
        return;
    };
    r.dead = false;
    r.health = 0;
    if let Some(p) = map.players.get_mut(&hunter) {
        p.row.state.pets.current = Some(r.clone());
    }
    if let Some(g) = summon::from_record(map, hunter, &r)
        && let Some(c) = map.creature_mut(g)
    {
        let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
        c.fields
            .set_u32(UNIT_FIELD_HEALTH, ((max as f32 * fraction) as u32).max(1));
    }
}

/// vmangos's food benefit: more for food near the pet's level.
fn food_benefit(pet_level: u32, item_level: u32) -> u32 {
    if pet_level <= item_level + 5 {
        35_000
    } else if pet_level <= item_level + 10 {
        17_000
    } else if pet_level <= item_level + 14 {
        8_000
    } else {
        0
    }
}

/// Whether `food` (an item `food_type`) is food a pet of `mask` eats.
fn eats(mask: u32, food_type: u32) -> bool {
    food_type > 0 && mask & (1 << (food_type - 1)) != 0
}

/// Why Feed Pet can't take `item`, if it can't.
pub fn feed_check(map: &Map, hunter: Guid, item: Option<Guid>) -> Option<u8> {
    use wow_world_messages::vanilla::SpellCastResult as R;
    let Some(pet) = pet_of(map, hunter) else {
        return Some(R::NoPet.as_int());
    };
    let Some(c) = map.creature(pet) else {
        return Some(R::NoPet.as_int());
    };
    if !c.alive() {
        return Some(R::TargetsDead.as_int());
    }
    if c.fields.has_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    ) {
        return Some(R::AffectingCombat.as_int());
    }
    let p = map.players.get(&hunter)?;
    let Some(t) = item
        .and_then(|g| p.item_by_guid(g))
        .and_then(|i| map.realm.data.item(p.items[i].entry))
    else {
        return Some(R::ItemGone.as_int());
    };
    let mask = c.pet.as_deref().map(|s| s.food_mask).unwrap_or(0);
    if !eats(mask, t.food_type) {
        return Some(R::WrongPetFood.as_int());
    }
    if food_benefit(c.fields.get_u32(UNIT_FIELD_LEVEL), t.item_level) == 0 {
        return Some(R::FoodLowlevel.as_int());
    }
    None
}

/// Feed Pet: the food is eaten, and its happiness comes in over the
/// spell's time.
pub fn feed(map: &mut Map, hunter: Guid, s: &SpellTemplate, index: usize, item: Option<Guid>) {
    let data = map.realm.data.clone();
    if feed_check(map, hunter, item).is_some() {
        return;
    }
    let Some(pet) = pet_of(map, hunter) else {
        return;
    };
    let Some((i, level)) = map.players.get(&hunter).and_then(|p| {
        let i = p.item_by_guid(item?)?;
        Some((i, data.item(p.items[i].entry)?.item_level))
    }) else {
        return;
    };
    let pet_level = map
        .creature(pet)
        .map(|c| c.fields.get_u32(UNIT_FIELD_LEVEL))
        .unwrap_or(1);
    let benefit = food_benefit(pet_level, level);
    crate::game::map::inventory::take_item(map, hunter, i, 1);
    let trigger = s.effects[index].trigger_spell;
    if let Some(t) = data.spells.get(trigger) {
        let effects: Vec<(usize, i32)> = (0..3)
            .filter(|i| t.effects[*i].effect == 6 && t.effects[*i].aura != 0)
            .map(|i| (i, benefit as i32))
            .collect();
        let _ = crate::game::map::auras::apply_spell(
            map,
            hunter,
            pet,
            t,
            &effects,
            crate::game::map::auras::Context::default(),
        );
    } else {
        change_happiness(map, pet, benefit as i32 * 10);
    }
}

/// Thistle Tea's untraining price: 10 silver, then 50, then a gold, and a
/// gold more each time to ten, back to 10 silver after a day.
fn untrain_cost(last: u32, at: i64, now: i64) -> u32 {
    if at == 0 || now - at >= 86_400 || last < 1_000 {
        1_000
    } else if last < 5_000 {
        5_000
    } else if last < 10_000 {
        10_000
    } else {
        (last + 10_000).min(100_000)
    }
}

/// Whether `pet` is `hunter`'s own tamed beast with more than one spell to
/// forget (mangos's `HandlePetUnlearnOpcode`, Thistle Tea's
/// `PetUntraining.quote`).
fn untrainable(map: &Map, hunter: Guid, pet: Guid) -> bool {
    state(map, pet)
        .is_some_and(|s| s.owner == hunter && s.kind == Kind::Hunter && s.spells.len() > 1)
}

/// The stable master's gossip: what untraining would cost now.
pub fn untrain_offer(map: &mut Map, hunter: Guid) {
    let Some(pet) = map.players.get(&hunter).and_then(|p| p.owned.pet) else {
        return;
    };
    if !untrainable(map, hunter, pet) {
        return;
    }
    let Some(p) = map.players.get(&hunter) else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    let cost = untrain_cost(
        p.row.state.pets.untrain_cost,
        p.row.state.pets.untrain_at,
        now,
    );
    p.outbound.send(&SMSG_PET_UNLEARN_CONFIRM {
        pet,
        talent_reset_cost: cost,
    });
}

/// `CMSG_PET_UNLEARN`: the pet forgets its trained spells, and gets its
/// training points back.
pub fn untrain(map: &mut Map, hunter: Guid, pet: Guid) {
    let data = map.realm.data.clone();
    if !untrainable(map, hunter, pet) {
        return;
    }
    let now = chrono::Utc::now().timestamp();
    let Some(p) = map.players.get_mut(&hunter) else {
        return;
    };
    let cost = untrain_cost(
        p.row.state.pets.untrain_cost,
        p.row.state.pets.untrain_at,
        now,
    );
    if p.money() < cost {
        p.outbound.send(&SMSG_BUY_FAILED {
            guid: pet,
            item: 0,
            result: BuyResult::NotEnoughtMoney,
        });
        return;
    }
    p.set_money(p.money() - cost);
    p.row.state.pets.untrain_cost = cost;
    p.row.state.pets.untrain_at = now;
    let Some(c) = map.creature_mut(pet) else {
        return;
    };
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL) as i32;
    let rank = i32::from(c.fields.get_byte(UNIT_FIELD_BYTES_1, 1).max(1));
    let entry = c.entry;
    let Some(s) = c.pet.as_deref_mut() else {
        return;
    };
    let keep: Vec<u32> = s.family_spells.clone();
    let gone: Vec<u32> = s
        .spells
        .iter()
        .copied()
        .filter(|x| !keep.contains(x))
        .collect();
    s.spells = keep;
    s.autocast.clear();
    s.bar = control::default_bar(&data, &s.spells, &[], false);
    s.training_points = level * (rank - 1);
    let _ = entry;
    show_training_points(c);
    for spell in gone {
        crate::game::map::auras::remove_spell(map, pet, spell, None);
    }
    send_spells(map, hunter);
    sync_record(map, hunter);
}

/// A pet ability's skill-line entries.
fn ability_entries(data: &GameData, spell: u32) -> Vec<crate::game::data::pets::PetAbility> {
    data.pets
        .abilities
        .values()
        .flatten()
        .filter(|a| a.spell == spell)
        .copied()
        .collect()
}

/// Whether `a` and `b` are ranks of one pet ability.
fn same_ability(data: &GameData, a: u32, b: u32) -> bool {
    data.spells.same_kind(a, b)
        || matches!((data.spells.get(a), data.spells.get(b)), (Some(x), Some(y)) if x.name == y.name)
}

/// Whether a pet's use of an ability teaches its master, on a roll of
/// 0 to 100: ten of the 101 do (mangos's `Pet::CheckLearning`,
/// `urand(0, 100) < 10`; Thistle Tea's `PetLearning`).
pub fn teaches(roll: u32) -> bool {
    roll < 10
}

/// A hunter's living pet used `spell`: by [`teaches`]' chance, its master
/// learns to teach it to a pet. With the rolls fixed (the tests), it
/// doesn't.
pub fn used(map: &mut Map, pet: Guid, spell: u32) {
    if !map.unit_view(pet).is_some_and(|u| u.alive)
        || map.realm.data.spells.get(spell).is_none_or(|t| t.passive())
    {
        return;
    }
    let roll = if crate::game::map::spells::outcome::rolling(map) {
        use rand::RngExt;
        rand::rng().random_range(0..=100)
    } else {
        100
    };
    if teaches(roll) {
        teach_master(map, pet, spell);
    }
}

/// A hunter's pet came with passive `spell`: its master learns to teach
/// it, at once (mangos's `InitPetCreateSpells`, "learn passive skills
/// when tamed"; Thistle Tea's `PetTraining.discover_passives`).
pub fn discover_passive(map: &mut Map, pet: Guid, spell: u32) {
    teach_master(map, pet, spell);
}

/// The master of hunter's pet `pet` learns to teach `spell`, if there is a
/// teaching of it they lack.
fn teach_master(map: &mut Map, pet: Guid, spell: u32) {
    let Some(s) = state(map, pet).filter(|s| s.kind == Kind::Hunter) else {
        return;
    };
    let owner = s.owner;
    let Some(teaching) = map.realm.data.pets.teaching.get(&spell).copied() else {
        return;
    };
    if map
        .players
        .get(&owner)
        .is_some_and(|p| !p.spells.contains(&teaching))
    {
        crate::game::map::spells::learn(map, owner, teaching);
    }
}

/// What training a pet in an ability costs: its points, less those spent
/// on the rank it knows.
fn training_cost(data: &GameData, s: &PetState, spell: u32) -> i32 {
    let total = ability_entries(data, spell).first().map_or(0, |a| a.cost) as i32;
    let spent = s
        .spells
        .iter()
        .filter(|k| same_ability(data, **k, spell))
        .filter_map(|k| ability_entries(data, *k).first().map(|a| a.cost as i32))
        .max()
        .unwrap_or(0);
    total - spent
}

/// Why a teaching spell can't train the hunter's pet, if it can't
/// (Thistle Tea's `PetTraining.validate`).
pub fn train_check(map: &Map, hunter: Guid, teaching: &SpellTemplate) -> Option<u8> {
    use wow_world_messages::vanilla::SpellCastResult as R;
    let data = &map.realm.data;
    let spell = taught(teaching)?;
    let Some(pet) = map.players.get(&hunter).and_then(|p| p.owned.pet) else {
        return Some(R::NoPet.as_int());
    };
    let Some((c, s)) = map
        .creature(pet)
        .and_then(|c| c.pet.as_deref().map(|s| (c, s)))
        .filter(|(_, s)| s.kind == Kind::Hunter)
    else {
        return Some(R::NoPet.as_int());
    };
    if !c.alive() {
        return Some(R::TargetsDead.as_int());
    }
    let Some(t) = data.spells.get(spell) else {
        return Some(R::NotKnown.as_int());
    };
    let skills: Vec<u32> = ability_entries(data, spell)
        .iter()
        .map(|a| a.skill)
        .collect();
    let family_skill = data.pets.families.get(&s.family).map_or(0, |f| f.skills[0]);
    if !skills.contains(&270) && !skills.contains(&family_skill) {
        return Some(R::BadTargets.as_int());
    }
    let known = s
        .spells
        .iter()
        .filter(|k| same_ability(data, **k, spell))
        .filter_map(|k| data.spells.get(*k))
        .any(|k| k.base_level >= t.base_level);
    if known {
        return Some(R::SpellLearned.as_int());
    }
    let level = c.fields.get_u32(UNIT_FIELD_LEVEL);
    if teaching.spell_level.max(t.spell_level) > level {
        return Some(R::Lowlevel.as_int());
    }
    if !t.passive() {
        let mut active: Vec<&str> = s
            .spells
            .iter()
            .filter_map(|k| data.spells.get(*k))
            .filter(|k| !k.passive())
            .map(|k| k.name.as_str())
            .collect();
        active.push(t.name.as_str());
        active.sort_unstable();
        active.dedup();
        if active.len() > 4 {
            return Some(R::TooManySkills.as_int());
        }
    }
    let cost = training_cost(data, s, spell);
    if cost > 0 && s.training_points < cost {
        return Some(R::TrainingPoints.as_int());
    }
    None
}

/// The teaching spell lands: the pet learns the ability, for its points.
pub fn train(map: &mut Map, hunter: Guid, teaching: &SpellTemplate) {
    if train_check(map, hunter, teaching).is_some() {
        return;
    }
    let data = map.realm.data.clone();
    let Some(spell) = taught(teaching) else {
        return;
    };
    let Some(pet) = map.players.get(&hunter).and_then(|p| p.owned.pet) else {
        return;
    };
    let cost = state(map, pet).map_or(0, |s| training_cost(&data, s, spell));
    // The lower rank it knew goes for the new one.
    let lower: Vec<u32> = state(map, pet)
        .map(|s| {
            s.spells
                .iter()
                .copied()
                .filter(|k| *k != spell && same_ability(&data, *k, spell))
                .collect()
        })
        .unwrap_or_default();
    if let Some(s) = state_mut(map, pet) {
        s.spells.retain(|k| !lower.contains(k));
        for b in s.bar.iter_mut() {
            let (action, kind) = control::unpack(*b);
            if lower.contains(&action) {
                *b = control::button(spell, kind);
            }
        }
        for a in s.autocast.iter_mut() {
            if lower.contains(a) {
                *a = spell;
            }
        }
        s.training_points -= cost.max(0);
    }
    for l in &lower {
        crate::game::map::auras::remove_spell(map, pet, *l, None);
    }
    control::learn(map, pet, spell);
    // A passive takes hold at once.
    if data.spells.get(spell).is_some_and(|t| t.passive()) {
        summon::apply_passive(map, pet, spell);
    }
    if let Some(c) = map.creature_mut(pet) {
        show_training_points(c);
    }
    sync_record(map, hunter);
}

fn stable_master(map: &Map, player: Guid, npc: Guid) -> bool {
    crate::game::map::npc::npc_entry(map, player, npc, NPC_FLAG_STABLEMASTER).is_some()
}

fn stable_result(map: &Map, player: Guid, r: StableResult) {
    if let Some(p) = map.players.get(&player) {
        p.outbound.send(&SMSG_STABLE_RESULT { result: r });
    }
}

/// `MSG_LIST_STABLED_PETS`: the current pet (alive) and those stabled.
pub fn list_stable(map: &mut Map, player: Guid, npc: Guid) {
    if !stable_master(map, player, npc) {
        // The client asks for the stable at a right-click on a stable
        // master with no gossip of its own (Wesley's NPC flags are the
        // stable's alone): one not a hunter gets its gossip instead, its
        // greeting with no stable in it, as mangos's would show them. It
        // got nothing at all.
        let not_hunter = map.players.get(&player).is_some_and(|p| p.row.class != 3);
        let master = map.creature(npc).is_some_and(|c| {
            c.fields.get_u32(crate::game::fields::UNIT_NPC_FLAGS) & NPC_FLAG_STABLEMASTER != 0
        });
        if not_hunter && master {
            crate::game::map::npc::hello(map, player, npc);
        }
        return;
    }
    sync_record(map, player);
    let Some(p) = map.players.get(&player) else {
        return;
    };
    let pets = &p.row.state.pets;
    let entry = |r: &PetRecord, slot: u8| StabledPet {
        pet_number: r.number,
        entry: r.entry,
        level: Level::new(r.level.max(1)),
        name: r.name.clone(),
        loyalty: u32::from(r.loyalty),
        slot,
    };
    let mut list = Vec::new();
    if let Some(r) = pets
        .current
        .as_ref()
        .filter(|r| r.kind == KIND_HUNTER && !r.dead)
    {
        list.push(entry(r, 1));
    }
    let mut stabled = pets.stable.clone();
    stabled.sort_by_key(|(slot, _)| *slot);
    for (slot, r) in &stabled {
        list.push(entry(r, slot + 1));
    }
    p.outbound.send(&MSG_LIST_STABLED_PETS_Server {
        npc,
        stable_slots: pets.stable_slots,
        pets: list,
    });
}

/// `CMSG_STABLE_PET`: the current pet into a free stall.
pub fn stable(map: &mut Map, player: Guid, npc: Guid) {
    // No master to serve them: told so (mangos's `CheckStableMaster`).
    if !stable_master(map, player, npc) {
        return stable_result(map, player, StableResult::ErrStable);
    }
    if let Some(pet) = map.players.get(&player).and_then(|p| p.owned.pet) {
        if !map.unit_view(pet).is_some_and(|u| u.alive) {
            return stable_result(map, player, StableResult::ErrStable);
        }
        sync_record(map, player);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let pets = &mut p.row.state.pets;
    let free = (1..=pets.stable_slots).find(|s| pets.stable.iter().all(|(x, _)| x != s));
    let current = pets
        .current
        .clone()
        .filter(|r| r.kind == KIND_HUNTER && !r.dead);
    let (Some(slot), Some(r)) = (free, current) else {
        return stable_result(map, player, StableResult::ErrStable);
    };
    pets.stable.push((slot, r));
    pets.current = None;
    pets.active = false;
    let out = p.owned.pet;
    if let Some(pet) = out {
        summon::remove(map, pet);
    }
    stable_result(map, player, StableResult::SuccessStable);
}

/// `CMSG_UNSTABLE_PET`: a stabled pet out, with no current one.
pub fn unstable(map: &mut Map, player: Guid, npc: Guid, number: u32) {
    // No master to serve them: told so (mangos's `CheckStableMaster`).
    if !stable_master(map, player, npc) {
        return stable_result(map, player, StableResult::ErrStable);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let pets = &mut p.row.state.pets;
    if pets.current.is_some() {
        return stable_result(map, player, StableResult::ErrStable);
    }
    // One that can't come out (its creature not in the world's data)
    // stays in, as mangos's `HandleUnstablePet` checks the template first.
    let Some(i) = pets.stable.iter().position(|(_, r)| {
        r.number == number && map.realm.data.creature_template(r.entry).is_some()
    }) else {
        return stable_result(map, player, StableResult::ErrStable);
    };
    let (_, r) = pets.stable.remove(i);
    pets.current = Some(r.clone());
    pets.active = true;
    summon::from_record(map, player, &r);
    stable_result(map, player, StableResult::SuccessUnstable);
}

/// `CMSG_STABLE_SWAP_PET`: the current pet and a stabled one change places.
pub fn swap(map: &mut Map, player: Guid, npc: Guid, number: u32) {
    // No master to serve them: told so (mangos's `CheckStableMaster`).
    if !stable_master(map, player, npc) {
        return stable_result(map, player, StableResult::ErrStable);
    }
    if let Some(pet) = map.players.get(&player).and_then(|p| p.owned.pet) {
        if !map.unit_view(pet).is_some_and(|u| u.alive) {
            return stable_result(map, player, StableResult::ErrStable);
        }
        sync_record(map, player);
    }
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let pets = &mut p.row.state.pets;
    // Nothing moves for one that can't come out (its creature not in the
    // world's data), as mangos's `HandleStableSwapPet` checks the template
    // before the pet out goes in.
    let Some(i) = pets.stable.iter().position(|(_, r)| {
        r.number == number && map.realm.data.creature_template(r.entry).is_some()
    }) else {
        return stable_result(map, player, StableResult::ErrStable);
    };
    let Some(current) = pets
        .current
        .clone()
        .filter(|r| r.kind == KIND_HUNTER && !r.dead)
    else {
        return stable_result(map, player, StableResult::ErrStable);
    };
    let (slot, r) = pets.stable.remove(i);
    pets.stable.push((slot, current));
    pets.current = Some(r.clone());
    pets.active = true;
    let out = p.owned.pet;
    if let Some(pet) = out {
        summon::remove(map, pet);
    }
    summon::from_record(map, player, &r);
    stable_result(map, player, StableResult::SuccessUnstable);
}

/// `CMSG_BUY_STABLE_SLOT`: a stall more, for its price.
pub fn buy_slot(map: &mut Map, player: Guid, npc: Guid) {
    // No master to serve them: told so (mangos's `CheckStableMaster`).
    if !stable_master(map, player, npc) {
        return stable_result(map, player, StableResult::ErrStable);
    }
    let prices = map.realm.data.pets.stable_prices.clone();
    let Some(p) = map.players.get_mut(&player) else {
        return;
    };
    let slots = p.row.state.pets.stable_slots;
    let Some(price) = prices.get(usize::from(slots)).copied().or(match slots {
        0 => Some(500),
        1 => Some(50_000),
        _ => None,
    }) else {
        return stable_result(map, player, StableResult::ErrStable);
    };
    if slots >= 2 {
        return stable_result(map, player, StableResult::ErrStable);
    }
    if p.money() < price {
        return stable_result(map, player, StableResult::ErrMoney);
    }
    p.set_money(p.money() - price);
    p.row.state.pets.stable_slots = slots + 1;
    stable_result(map, player, StableResult::SuccessBuySlot);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happiness_damage_food_and_untraining_follow_thistle_tea() {
        assert_eq!(damage_multiplier(100_000), 0.75);
        assert_eq!(damage_multiplier(400_000), 1.0);
        assert_eq!(damage_multiplier(800_000), 1.25);
        assert_eq!(food_benefit(20, 18), 35_000);
        assert_eq!(food_benefit(30, 18), 8_000);
        assert_eq!(food_benefit(40, 18), 0);
        assert!(eats(1, 1) && !eats(1, 2));
        assert_eq!(untrain_cost(0, 0, 100), 1_000);
        assert_eq!(untrain_cost(1_000, 50, 100), 5_000);
        assert_eq!(untrain_cost(5_000, 50, 100), 10_000);
        assert_eq!(untrain_cost(100_000, 50, 100), 100_000);
        assert_eq!(untrain_cost(100_000, 50, 100_000), 1_000);
    }
}
