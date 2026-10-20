//! Ranged auto-repeat: Auto Shot and a wand's Shoot fire at the ranged
//! weapon's speed until cancelled, half a second after they start, as
//! Thistle Tea's `AutoRepeat` runs them. A shot can only miss or crit; it
//! spends a round of ammunition when its weapon takes ammo. Moving holds
//! Auto Shot back and cancels a wand.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::{self, AutoRepeat, Hand, Outcome};
use crate::game::map::procs;
use crate::game::map::spells::{self, Targets};

pub const SPELL_AUTO_SHOT: u32 = 75;
pub const SPELL_SHOOT: u32 = 5019;
/// The first shot waits this long, and moving holds a shot back by it.
pub const SHOT_DELAY: Duration = Duration::from_millis(500);

const SPELL_FAILED_EQUIPPED_ITEM_CLASS: u8 = 0x19;
const SPELL_FAILED_NEED_AMMO: u8 = 0x2F;
const SPELL_FAILED_NO_AMMO: u8 = 0x43;
/// Item class: projectiles.
const ITEM_CLASS_PROJECTILE: u32 = 6;
const SUBCLASS_WAND: u32 = 19;

/// `CMSG_CANCEL_AUTO_REPEAT_SPELL` and `CMSG_SET_AMMO`.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    match msg {
        ClientOpcodeMessage::CMSG_CANCEL_AUTO_REPEAT_SPELL => cancel(map, g, false),
        ClientOpcodeMessage::CMSG_SET_AMMO(m) => set_ammo(map, g, m.item),
        other => return Some(other),
    }
    None
}

/// A shot's result, laid out as any cast's: a gear failure names the gear
/// the spell wants, which the client reads after the reason.
fn cast_result(map: &Map, g: Guid, spell: u32, failure: Option<u8>) {
    spells::send_cast_result(map, g, spell, failure);
}

/// The inventory index of a carried round of the ammunition a player has
/// chosen, if the weapon takes it (mangos's and Thistle Tea's "no ammo"
/// for arrows in a gun); with none chosen, or none of it left, the first
/// the weapon takes, which becomes the choice.
fn ammo_index(map: &mut Map, g: Guid, ammo_type: u32) -> Option<usize> {
    let data = map.realm.data.clone();
    let p = map.players.get_mut(&g)?;
    let fits = |entry: u32| {
        data.item(entry)
            .is_some_and(|t| t.class == ITEM_CLASS_PROJECTILE && t.subclass == ammo_type)
    };
    let ammo = p.fields.get_u32(PLAYER_AMMO_ID);
    if ammo != 0
        && fits(ammo)
        && let Some(i) = p.items.iter().position(|i| i.entry == ammo && i.carried())
    {
        return Some(i);
    }
    let i = p.items.iter().position(|i| i.carried() && fits(i.entry))?;
    let entry = p.items[i].entry;
    p.fields.set_u32(PLAYER_AMMO_ID, entry);
    crate::game::map::auras::recalc(map, g);
    map.players
        .get(&g)?
        .items
        .iter()
        .position(|x| x.entry == entry)
}

/// Inventory type: a thrown weapon.
const INVENTORY_THROWN: u32 = 25;

/// What a ranged attack costs, as Thistle Tea's `Ammunition` works it out:
/// a round of the chosen arrows or bullets; one of a thrown stack, or a
/// point of a single thrown weapon's durability; nothing for a wand. The
/// look of what flies, for the shot's packet; or why it can't be paid.
pub fn pay_ammunition(map: &mut Map, g: Guid) -> Result<Option<(u32, u32)>, u8> {
    let (look, payment) = plan_ammunition(map, g)?;
    spend(map, g, payment);
    Ok(look)
}

/// What a ranged attack is to cost, worked out before it is paid
/// ([`spend`]), so that a shot is struck with the weapon it was loosed
/// from: the last of a stack of knives still hits as a knife (Thistle
/// Tea's `Ammunition.plan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payment {
    Nothing,
    /// One of the item (a thrown stack, or a round of ammunition).
    One(Guid),
    /// A point of the item's durability (a single thrown weapon).
    Wear(Guid),
}

/// [`pay_ammunition`]'s cost and look, not yet paid.
pub fn plan_ammunition(map: &mut Map, g: Guid) -> Result<(Option<(u32, u32)>, Payment), u8> {
    let data = map.realm.data.clone();
    let Some(weapon) = combat::hand_weapon(map, g, Hand::Ranged) else {
        return Err(SPELL_FAILED_EQUIPPED_ITEM_CLASS);
    };
    let look = |entry: u32| data.item(entry).map(|t| (t.display_id, t.inventory_type));
    let Some(t) = data.item(weapon.entry) else {
        return Ok((None, Payment::Nothing));
    };
    if t.inventory_type == INVENTORY_THROWN {
        let payment = if t.stackable <= 1 {
            Payment::Wear(weapon.guid)
        } else {
            Payment::One(weapon.guid)
        };
        return Ok((look(weapon.entry), payment));
    }
    if weapon.ammo_type != 0 {
        let Some(i) = ammo_index(map, g, weapon.ammo_type) else {
            return Err(SPELL_FAILED_NO_AMMO);
        };
        let item = &map.players[&g].items[i];
        return Ok((look(item.entry), Payment::One(item.guid)));
    }
    Ok((None, Payment::Nothing))
}

/// Pay what [`plan_ammunition`] worked out.
pub fn spend(map: &mut Map, g: Guid, payment: Payment) {
    let data = map.realm.data.clone();
    match payment {
        Payment::Nothing => {}
        Payment::One(item) => {
            if let Some(i) = map.players.get(&g).and_then(|p| p.item_by_guid(item)) {
                crate::game::map::inventory::take_item(map, g, i, 1);
            }
        }
        Payment::Wear(item) => {
            let Some(p) = map.players.get_mut(&g) else {
                return;
            };
            let Some(i) = p.item_by_guid(item) else {
                return;
            };
            let f = &mut p.items[i].fields;
            let durability = f.get_u32(ITEM_FIELD_DURABILITY);
            if f.get_u32(ITEM_FIELD_MAXDURABILITY) > 0 {
                f.set_u32(ITEM_FIELD_DURABILITY, durability.saturating_sub(1));
                if durability <= 1 {
                    p.gear_dirty = true;
                    p.refresh_inventory(&data);
                }
            }
        }
    }
}

/// The projectile a player's ranged spell shows, as mangos's
/// WriteAmmoToPacket picks it: a thrown weapon itself, else the ammunition
/// set (its look and inventory type), else nothing of the weapon's kind.
/// None for a caster with no ranged weapon, or not a player.
pub fn ammo_shown(map: &Map, g: Guid) -> Option<(u32, u32)> {
    let data = &map.realm.data;
    let weapon = combat::hand_weapon(map, g, Hand::Ranged)?;
    let t = data.item(weapon.entry)?;
    if t.inventory_type == INVENTORY_THROWN {
        return Some((t.display_id, t.inventory_type));
    }
    let ammo = map.players.get(&g)?.fields.get_u32(PLAYER_AMMO_ID);
    Some(
        data.item(ammo)
            .filter(|_| ammo != 0)
            .map_or((0, t.inventory_type), |a| (a.display_id, a.inventory_type)),
    )
}

/// `CMSG_SET_AMMO`: the ammunition the ranged weapon uses.
/// Refused with mangos's `CanUseAmmo` and Thistle Tea's reasons: the dead
/// choose nothing (nor clear it); the ammunition must be carried (not in
/// the bank), be ammunition, and be of the player's class, race, level and
/// skill. The choice is kept over a logout.
pub fn set_ammo(map: &mut Map, g: Guid, item: u32) {
    use crate::game::map::inventory::{self, Fail};
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let refusal = if p.death != crate::game::map::death::Death::Alive {
        Some(Fail::YouAreDead)
    } else if item == 0 {
        None
    } else if !p.items.iter().any(|i| i.entry == item && i.carried()) {
        Some(Fail::NotFound)
    } else {
        match data.item(item) {
            None => Some(Fail::NotFound),
            Some(t) if t.inventory_type != INVENTORY_AMMO => Some(Fail::OnlyAmmo),
            Some(t) => usable_by(p, &data, t).err(),
        }
    };
    if let Some(fail) = refusal {
        inventory::send_fail(p, fail, Guid::zero());
        return;
    }
    p.fields.set_u32(PLAYER_AMMO_ID, item);
    crate::game::map::auras::recalc(map, g);
}

/// Inventory type: ammunition.
const INVENTORY_AMMO: u32 = 24;

/// Whether `p` may use an item of `t` at all (mangos's `CanUseItem`): its
/// class, race, level and skill.
fn usable_by(
    p: &crate::game::map::player::Player,
    data: &crate::game::data::GameData,
    t: &crate::game::data::items::ItemTemplate,
) -> Result<(), crate::game::map::inventory::Fail> {
    use crate::game::map::inventory::{self, Fail};
    let fits = |mask: i32, of: u8| mask == -1 || mask == 0 || mask & (1 << (of - 1)) != 0;
    if !fits(t.allowable_class, p.row.class) || !fits(t.allowable_race, p.row.race) {
        return Err(Fail::NeverUse);
    }
    if u32::from(p.level()) < t.required_level {
        return Err(Fail::Level(t.required_level as u8));
    }
    // The skill it asks for, even where every weapon and armor is allowed.
    if t.required_skill != 0 {
        match crate::game::map::skills::value(p, t.required_skill) {
            None | Some(0) => return Err(Fail::NoProficiency),
            Some(v) if v < t.required_skill_rank => return Err(Fail::Skill),
            Some(_) => {}
        }
    }
    inventory::proficient(p, &inventory::Proficiency::of(p, data), t)
}

/// Start auto-repeating `spell` at the targets' unit.
pub fn start(map: &mut Map, g: Guid, spell: u32, targets: Targets) {
    let now = map.now;
    let Some(target) = targets.unit.filter(|t| *t != g) else {
        return;
    };
    let Some(weapon) = combat::hand_weapon(map, g, Hand::Ranged) else {
        cast_result(map, g, spell, Some(SPELL_FAILED_EQUIPPED_ITEM_CLASS));
        return;
    };
    if weapon.ammo_type != 0 && ammo_index(map, g, weapon.ammo_type).is_none() {
        cast_result(map, g, spell, Some(SPELL_FAILED_NEED_AMMO));
        return;
    }
    let Some(c) = map.combat_mut(g) else {
        return;
    };
    // Stopped and started again, it waits for the weapon as it would have
    // (Thistle Tea's toggled Shoot keeps its last launch's cooldown).
    let ready = c
        .auto_repeat
        .map(|a| a.next_at)
        .or(c.ranged_ready_at)
        .unwrap_or(now);
    c.auto_repeat = Some(AutoRepeat {
        spell,
        target,
        next_at: ready.max(now + SHOT_DELAY),
    });
    cast_result(map, g, spell, None);
}

/// Stop auto-repeating; with `tell`, the client is told.
pub fn cancel(map: &mut Map, g: Guid, tell: bool) {
    let stopped = map
        .combat_mut(g)
        .and_then(|c| c.auto_repeat.take())
        .is_some();
    if stopped
        && tell
        && let Some(p) = map.players.get(&g)
    {
        p.outbound.send(&SMSG_CANCEL_AUTO_REPEAT {});
    }
}

/// Whether `g` is shooting a wand (Shoot, of the magic damage class), not
/// Auto Shot.
fn wanding(map: &Map, g: Guid) -> bool {
    map.combat(g)
        .and_then(|c| c.auto_repeat)
        .and_then(|ar| map.realm.data.spells.get(ar.spell))
        .is_some_and(|s| s.dmg_class == 1)
}

/// A player begins casting another spell: a wand stops, Auto Shot goes on.
pub fn cast_begun(map: &mut Map, g: Guid) {
    if wanding(map, g) {
        cancel(map, g, true);
    }
}

/// A player moved: a wand stops; Auto Shot waits until they stand.
pub fn moved(map: &mut Map, g: Guid) {
    let now = map.now;
    if map.combat(g).and_then(|c| c.auto_repeat).is_none() {
        return;
    }
    if wanding(map, g) {
        cancel(map, g, true);
        return;
    }
    if let Some(c) = map.combat_mut(g)
        && let Some(a) = c.auto_repeat.as_mut()
    {
        a.next_at = a.next_at.max(now + SHOT_DELAY);
    }
}

/// Fire the shots that are due.
pub fn update(map: &mut Map) {
    let now = map.now;
    let due: Vec<Guid> = map
        .players
        .values()
        .filter(|p| p.combat.auto_repeat.is_some_and(|a| a.next_at <= now))
        .map(|p| p.guid)
        .collect();
    for g in due {
        shoot(map, g);
    }
}

fn shoot(map: &mut Map, g: Guid) {
    let now = map.now;
    let data = map.realm.data.clone();
    let Some(ar) = map.combat(g).and_then(|c| c.auto_repeat) else {
        return;
    };
    let Some(s) = data.spells.get(ar.spell) else {
        cancel(map, g, true);
        return;
    };
    let target = ar.target;
    if !map.unit_view(target).is_some_and(|u| u.alive) || !combat::can_attack(map, g, target) {
        cancel(map, g, true);
        return;
    }
    // Pacified (or silenced, for what silence stops), it stops without a
    // shot: each repeat is cast again, and fails (mangos's
    // `_UpdateAutoRepeatSpell` interrupting it, Thistle Tea's `blocked?`).
    if spells::checks::prevented(map, g, s) {
        cancel(map, g, true);
        return;
    }
    // A cast under way, or a stun, holds the shot.
    if map.caster(g).is_some_and(|c| c.casting.is_some())
        || crate::game::map::auras::held(map, g, false)
    {
        return;
    }
    let (Some(me), Some(them)) = (map.unit_view(g), map.unit_view(target)) else {
        return;
    };
    // Its range as its cast's: the maximum with its modifiers (Hawk Eye),
    // the dead zone beyond both reaches as the maximum is (mangos's
    // `GetMinMaxRange`, Thistle Tea's `combat_distance`).
    let mods = spells::mods::for_spell(map, g, s);
    let (min, max) = data.spells.range(s);
    let max = mods.apply(spells::mods::OP_RANGE, max).max(0.0);
    let d = combat::dist(me.position, them.position);
    let reach = me.combat_reach + them.combat_reach;
    let out_of_range = d > max + reach + 1.0 || (min > 0.0 && d < min + reach);
    // Out of range or sight, or a player's side gone from it (into
    // stealth), the repeat stops: its cast fails, and mangos interrupts it
    // (Thistle Tea's `outside_range?` and `combat_distance`).
    let unseen = !s.attr(2, crate::game::data::spells::ATTR_EX2_IGNORE_LOS)
        && !spells::checks::line_of_sight(map, me.position, them.position);
    let concealed =
        spells::checks::player_side(map, target) && !spells::stealth::sees(map, g, target);
    if out_of_range || unseen || concealed {
        cancel(map, g, true);
        return;
    }
    if !combat::facing(&me, &them) {
        return;
    }
    // A broken weapon, or one the spell doesn't take (a bow for a wand's
    // Shoot), stops it: mangos checks each repeat's cast again.
    let weapon = combat::hand_weapon(map, g, Hand::Ranged)
        .filter(|_| spells::checks::equipped_ok(map, g, s));
    let Some(weapon) = weapon else {
        cancel(map, g, true);
        return;
    };
    // A round of ammunition, paid once the shot is struck.
    let (ammo_display, payment) = match plan_ammunition(map, g) {
        Ok(planned) => planned,
        Err(reason) => {
            cast_result(map, g, ar.spell, Some(reason));
            cancel(map, g, true);
            return;
        }
    };
    let period = map
        .unit_fields(g)
        .map(|f| f.get_u32(UNIT_FIELD_RANGEDATTACKTIME))
        .unwrap_or(2000)
        .max(100);
    if let Some(c) = map.combat_mut(g)
        && let Some(a) = c.auto_repeat.as_mut()
    {
        a.next_at = now + Duration::from_millis(u64::from(period));
    }
    if let Some(c) = map.combat_mut(g) {
        c.ranged_ready_at = Some(now + Duration::from_millis(u64::from(period)));
    }
    resolve(map, g, target, s, weapon, ammo_display);
    spend(map, g, payment);
}

/// A shot's roll, damage and logs.
fn resolve(
    map: &mut Map,
    g: Guid,
    target: Guid,
    s: &crate::game::data::spells::SpellTemplate,
    weapon: crate::game::map::derive::Weapon,
    ammo_display: Option<(u32, u32)>,
) {
    let data = map.realm.data.clone();
    let Some(roll) = combat::roll_for(map, g, target, Hand::Ranged) else {
        return;
    };
    let mut rng = rand::rng();
    let wand = s.dmg_class == 1 || weapon.subclass == SUBCLASS_WAND;
    let mut outcome = combat::roll_outcome(&roll, rng.random_range(0..10_000));
    let (raw, school) = if wand {
        // A wand deals its own school, without attack power.
        let school = data.item(weapon.entry).map_or(0, |t| t.damages[0].2).min(6);
        let (lo, hi) = (weapon.min.max(1.0), weapon.max.max(weapon.min.max(1.0)));
        let rolled = if hi > lo {
            rng.random_range(lo..=hi)
        } else {
            lo
        };
        // Its school's damage done that fits it (Wand Specialization).
        let rolled = rolled
            * crate::game::map::auras::auras(map, g).map_or(1.0, |a| {
                crate::game::map::derive::wand_pct(a, 1 << school, &weapon)
            });
        // A wand crits as a spell does.
        if outcome != Outcome::Miss {
            let crit = combat::derived(map, g).spell_crit[school as usize];
            outcome = if rng.random_range(0.0f32..100.0) < crit {
                Outcome::Crit
            } else {
                Outcome::Hit
            };
        }
        (rolled as u32, 1 << school)
    } else {
        let (d, school) = combat::weapon_damage(map, g, target, Hand::Ranged, &mut rng);
        let points: i32 = (0..3)
            .filter(|i| matches!(s.effects[*i].effect, 17 | 58 | 121))
            .map(|i| spells::points(s, i, 1))
            .sum();
        ((d as i32 + points).max(1) as u32, school)
    };
    let mitigated = if school & 1 != 0 {
        combat::armor_mitigate(map, g, target, raw)
    } else {
        raw
    };
    // A wand's school the target is immune to: nothing, and it says so.
    let immune = wand && outcome != Outcome::Miss && combat::school_immune(map, target, school);
    let damage = match outcome {
        _ if immune => 0,
        Outcome::Miss => 0,
        Outcome::Crit if wand => mitigated * 3 / 2,
        Outcome::Crit => (mitigated as f32 * combat::crit_multiplier(map, g, target)) as u32,
        _ => mitigated,
    };
    // A wand's magic is partly resisted as a spell of its school is.
    let resisted = if wand && school & 1 == 0 && damage > 0 {
        let as_spell = crate::game::data::spells::SpellTemplate {
            school: school.trailing_zeros(),
            ..s.clone()
        };
        spells::outcome::partial_resist(map, g, target, &as_spell, damage, false)
    } else {
        0
    };
    let damage = damage - resisted;
    let (left, absorbed) = if damage > 0 {
        crate::game::map::auras::absorb_and_share(map, g, target, damage, school)
    } else {
        (0, 0)
    };
    let mut flags = SMSG_SPELL_GO_CastFlags::empty();
    if let Some((display, inventory_type)) = ammo_display.filter(|_| !wand) {
        flags = flags.set_ammo(SMSG_SPELL_GO_CastFlags_Ammo {
            ammo_display_id: display,
            ammo_inventory_type: inventory_type,
        });
    }
    let missed = outcome == Outcome::Miss || immune;
    if let Some(frame) = crate::game::conn::encode(&SMSG_SPELL_GO {
        cast_item: g,
        caster: g,
        spell: s.id,
        flags,
        hits: if missed { Vec::new() } else { vec![target] },
        misses: if missed {
            vec![SpellMiss {
                target,
                miss_info: if immune {
                    SpellMiss_SpellMissInfo::Immune
                } else {
                    SpellMiss_SpellMissInfo::Miss
                },
            }]
        } else {
            Vec::new()
        },
        targets: spells::wire_targets(&Targets {
            unit: Some(target),
            ..Default::default()
        }),
    }) {
        map.send_to_observers(g, &frame, true);
    }
    if !missed
        && let Some(frame) = crate::game::conn::encode(&SMSG_SPELLNONMELEEDAMAGELOG {
            target,
            attacker: g,
            spell: s.id,
            damage: left,
            school: SpellSchool::try_from(school.trailing_zeros() as u8)
                .unwrap_or(SpellSchool::Normal),
            absorbed_damage: absorbed,
            resisted,
            periodic_log: school & 1 != 0,
            unused: 0,
            blocked: 0,
            hit_info: if outcome == Outcome::Crit {
                HitInfo::new_affects_victim()
            } else {
                HitInfo::empty()
            },
            extend_flag: 0,
        })
    {
        map.send_to_observers(g, &frame, true);
        if let Some(p) = map.players.get(&target)
            && !p.known.contains(&g)
        {
            p.outbound.send_frame(frame);
        }
    }
    combat::engage(map, g, target);
    if left > 0 {
        crate::game::map::spells::pushback_by(map, target, g);
        combat::deal_damage_school(map, g, target, left, school);
    }
    let mut ex = combat::outcome_ex(outcome);
    if absorbed > 0 {
        ex |= procs::EX_ABSORB;
    }
    procs::trigger(
        map,
        procs::Event {
            actor: g,
            target,
            actor_flags: procs::PROC_DONE_RANGED,
            target_flags: procs::PROC_TAKEN_RANGED,
            ex,
            spell: Some(s.id),
            damage: left,
            school,
            ranged: true,
            ..Default::default()
        },
    );
}
