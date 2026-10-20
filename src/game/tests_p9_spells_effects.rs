//! Round 9, against Thistle Tea's spell effect tests where they are
//! 1.12's (`melee_proc_test.exs`, `owned_game_object_test.exs`,
//! `passive_spells_test.exs`, `dream_dragon_test.exs`,
//! `spell_effect_test.exs`, `spell_interrupt_test.exs`,
//! `spell_magnet_test.exs`, `spell_target_test.exs` and
//! `casting_test.exs`): Sweeping Strikes on a blow blocked in part;
//! objects in their summon slots; a form's passives kept, unlearned and
//! waiting for life; the dream dragons' aura; an ability blocked whole; a
//! summon's time and place; an interrupted ritual; a passive totem's
//! spell; a friendly area about its caster; a proc's spell at its caster;
//! and a self-cast stun's heartbeat.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, has, pass};
use super::tests_p6_wspell::{face_east, off, put};
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_EX3_COMPLETELY_BLOCKED, ATTR_PASSIVE, ProcEvent, SpellDuration,
    SpellEffect, SpellTemplate,
};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::auras::*;
use crate::game::map::combat::{self, dist};
use crate::game::map::derive::SPELL_BLOCK;
use crate::game::map::procs::{self, Event};
use crate::game::map::spells::{self, Targets, mods, targets};
use crate::game::map::{Map, boss_mechanics, death, misc, objects, pets};
use crate::game::tests_p3_combat::{self as p3, OFF_HAND};

/// Sweeping Strikes, as proc_scripts knows it by its id, with five
/// charges.
const SWEEPING_STRIKES: u32 = 12_292;
/// An object of no kind's own; slot summons of it in slots 1 to 3 and in
/// slot 4, for a minute; one in slot 1 of no end.
const SLOT_OBJECT: u32 = 997_170;
const SLOTS_1_TO_3: u32 = 997_171;
const SLOT_4: u32 = 997_172;
const UNTIMED: u32 = 997_173;
/// Cat form, bear form, and a passive talent for both (Thistle Tea's
/// 16944 of stances 145, made up).
const CAT: u32 = 997_174;
const BEAR: u32 = 997_175;
const TALENT: u32 = 997_176;
/// Aura of Nature, the dragon's mark and the stun it brings, by the ids
/// boss_mechanics knows; a stealth.
const AURA_OF_NATURE: u32 = 25_044;
const MARK: u32 = 25_040;
const MARK_STUN: u32 = 25_043;
const STEALTH: u32 = 997_177;
/// An ability a block takes whole; a reaction to a block of a melee
/// ability taken, and what it casts; no dodge, no parry, and all block.
const WHOLLY_BLOCKED: u32 = 997_178;
const BLOCK_REACTION: u32 = 997_179;
const REACTION: u32 = 997_180;
const NO_DODGE: u32 = 997_181;
const NO_PARRY: u32 = 997_182;
const ALL_BLOCK: u32 = 997_183;
const SHIELD: u32 = 997_196;
/// Summons (effect 28) of a guardian for fifteen seconds, of no end, and
/// at the caster's own place (18).
const SUMMON_TIMED: u32 = 997_184;
const SUMMON_UNTIMED: u32 = 997_185;
const SUMMON_HERE: u32 = 997_186;
const GUARDIAN: u32 = 9_975;
/// A channel that raises a ritual's altar, which a silence's interrupt
/// may stop; the interrupt.
const RITUAL: u32 = 997_187;
const ALTAR: u32 = 997_188;
const KICK: u32 = 997_189;
/// A totem in the first slot, and its spell: a helpful aura of one charge
/// on itself (Grounding Totem's kind).
const TOTEM: u32 = 9_976;
const SUMMON_TOTEM: u32 = 997_190;
const MAGNET: u32 = 997_191;
/// A heal about its caster's place, of its friends within ten yards.
const CIRCLE: u32 = 997_192;
/// A proc on the owner's melee blows, and its spell at its caster.
const ON_HIT: u32 = 997_193;
const AT_CASTER: u32 = 997_194;
/// A stun cast on oneself, checking its hold every heartbeat.
const SELF_STUN: u32 = 997_195;
const LASTS_60S: u32 = 910;
const NO_END: u32 = 911;
const R10: u32 = 13;
/// `SPELL_ATTR_HEARTBEAT_RESIST_CHECK`.
const HEARTBEAT: u32 = 0x4000_0000;
const GO_GENERIC: u8 = 5;

/// A proc aura of `kind` on `flags`, sure to go off, triggering
/// `trigger`, with `charges`.
fn proc_aura(id: u32, kind: u32, flags: u32, trigger: u32, charges: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    s.proc_flags = flags;
    s.proc_chance = 100;
    s.proc_charges = charges;
    effect(
        &mut s,
        0,
        SpellEffect {
            trigger_spell: trigger,
            ..aura(kind, 0, 1, 1)
        },
    );
    s
}

/// A spell of one aura on its caster, lasting `duration`.
fn self_aura(id: u32, kind: u32, misc: i32, amount: i32, duration: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = duration;
    effect(&mut s, 0, aura(kind, misc, amount, 1));
    s
}

fn slot_summon(slot: u32) -> SpellEffect {
    SpellEffect {
        effect: objects::EFFECT_SUMMON_OBJECT_SLOT1 + slot - 1,
        misc_value: SLOT_OBJECT as i32,
        ..Default::default()
    }
}

fn summon(id: u32, effect_id: u32, entry: u32, target: u32, duration: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = duration;
    effect(
        &mut s,
        0,
        SpellEffect {
            effect: effect_id,
            misc_value: entry as i32,
            target_a: target,
            ..Default::default()
        },
    );
    s
}

fn creature(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 60,
        level_max: 60,
        faction: 7,
        ..Default::default()
    }
}

fn data(d: &mut GameData) {
    d.spells.durations.insert(
        LASTS_60S,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    d.spells.durations.insert(
        NO_END,
        SpellDuration {
            base: -1,
            per_level: 0,
            max: -1,
        },
    );
    d.spells.radii.insert(R10, 10.0);
    d.creature_templates
        .insert(GUARDIAN, creature(GUARDIAN, "Made-up Guardian"));
    d.creature_templates
        .insert(TOTEM, creature(TOTEM, "Made-up Totem"));
    d.gameobject_templates.insert(
        SLOT_OBJECT,
        crate::game::tests_p3_entity_objects::object(SLOT_OBJECT, GO_GENERIC, "Made-up", &[]),
    );
    d.gameobject_templates.insert(
        ALTAR,
        crate::game::tests_p3_entity_objects::object(ALTAR, objects::GO_RITUAL, "Altar", &[(0, 2)]),
    );
    d.items.insert(
        SHIELD,
        ItemTemplate {
            entry: SHIELD,
            class: 4,
            subclass: 6,
            name: "Shield".into(),
            display_id: 1000,
            inventory_type: 14,
            block: 20,
            stackable: 1,
            ..Default::default()
        },
    );
    // Sweeping Strikes, on melee blows and melee abilities done.
    let sweeping = proc_aura(
        SWEEPING_STRIKES,
        AURA_DUMMY,
        procs::PROC_DONE_MELEE | procs::PROC_DONE_MELEE_SPELL,
        0,
        5,
    );
    // Slot summons, at no implicit target.
    let mut slots = bare(SLOTS_1_TO_3, 0);
    slots.duration_index = LASTS_60S;
    for i in 0..3 {
        effect(&mut slots, i, slot_summon(i as u32 + 1));
    }
    let mut slot_4 = bare(SLOT_4, 0);
    slot_4.duration_index = LASTS_60S;
    effect(&mut slot_4, 0, slot_summon(4));
    let mut untimed = bare(UNTIMED, 0);
    untimed.duration_index = NO_END;
    effect(&mut untimed, 0, slot_summon(1));
    // The forms and the talent of both.
    let cat = self_aura(CAT, AURA_MOD_SHAPESHIFT, i32::from(FORM_CAT), 0, LASTS_LONG);
    let bear = self_aura(
        BEAR,
        AURA_MOD_SHAPESHIFT,
        i32::from(FORM_BEAR),
        0,
        LASTS_LONG,
    );
    let mut talent = self_aura(TALENT, AURA_MOD_CRIT_PERCENT, 0, 6, 0);
    talent.attributes[0] |= ATTR_PASSIVE;
    talent.stances =
        spells::checks::bit(u32::from(FORM_CAT)) | spells::checks::bit(u32::from(FORM_BEAR));
    // The dream dragons' aura, its mark and stun.
    let mut nature = bare(AURA_OF_NATURE, 3);
    effect(
        &mut nature,
        0,
        SpellEffect {
            effect: spells::EFFECT_DUMMY,
            target_a: 1,
            ..Default::default()
        },
    );
    let stealth = self_aura(STEALTH, AURA_MOD_STEALTH, 0, 1, LASTS_LONG);
    let mark = self_aura(MARK, AURA_DUMMY, 0, 1, LASTS_LONG);
    let stun = self_aura(MARK_STUN, AURA_MOD_STUN, 0, 1, LASTS_5S);
    // An ability a block takes whole, and the blocker's reaction.
    let mut wholly = bare(WHOLLY_BLOCKED, 0);
    wholly.dmg_class = 2;
    wholly.attributes[3] |= ATTR_EX3_COMPLETELY_BLOCKED;
    effect(
        &mut wholly,
        0,
        SpellEffect {
            effect: spells::EFFECT_WEAPON_DAMAGE,
            target_a: 6,
            ..Default::default()
        },
    );
    let reaction = proc_aura(
        BLOCK_REACTION,
        AURA_PROC_TRIGGER_SPELL,
        procs::PROC_TAKEN_MELEE_SPELL,
        REACTION,
        0,
    );
    d.spells.proc_events.insert(
        BLOCK_REACTION,
        ProcEvent {
            ex: procs::EX_BLOCK,
            ..Default::default()
        },
    );
    let reacted = self_aura(REACTION, AURA_DUMMY, 0, 1, LASTS_LONG);
    let no_dodge = self_aura(NO_DODGE, AURA_MOD_DODGE_PERCENT, 0, -100, 0);
    let no_parry = self_aura(NO_PARRY, AURA_MOD_PARRY_PERCENT, 0, -100, 0);
    let all_block = self_aura(ALL_BLOCK, AURA_MOD_BLOCK_PERCENT, 0, 100, 0);
    // Guardians for a time, for good, and at the caster's own place.
    let timed = summon(SUMMON_TIMED, pets::EFFECT_SUMMON, GUARDIAN, 1, LASTS_15S);
    let untimed_guardian = summon(SUMMON_UNTIMED, pets::EFFECT_SUMMON, GUARDIAN, 1, NO_END);
    let here = summon(SUMMON_HERE, pets::EFFECT_SUMMON, GUARDIAN, 18, LASTS_15S);
    // The ritual, stopped as a silence stops a cast, and the interrupt.
    let mut ritual = bare(RITUAL, 0);
    ritual.attributes[1] |= ATTR_EX_CHANNELED;
    ritual.duration_index = LASTS_15S;
    ritual.interrupt_flags |= 0x8;
    ritual.prevention_type = 1;
    effect(
        &mut ritual,
        0,
        SpellEffect {
            effect: objects::EFFECT_TRANS_DOOR,
            misc_value: ALTAR as i32,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut kick = bare(KICK, 0);
    effect(
        &mut kick,
        0,
        SpellEffect {
            effect: spells::EFFECT_INTERRUPT_CAST,
            target_a: 6,
            ..Default::default()
        },
    );
    // The totem and its spell.
    let summon_totem = summon(
        SUMMON_TOTEM,
        pets::EFFECT_SUMMON_TOTEM_SLOT1,
        TOTEM,
        1,
        LASTS_LONG,
    );
    let mut magnet = self_aura(MAGNET, AURA_DUMMY, 0, 1, LASTS_LONG);
    magnet.proc_charges = 1;
    d.pets.totem_spells.insert(TOTEM, MAGNET);
    // A heal about the caster's place (22) of friends there (30).
    let mut circle = bare(CIRCLE, 2);
    effect(
        &mut circle,
        0,
        SpellEffect {
            effect: spells::EFFECT_HEAL,
            base_points: 99,
            base_dice: 1,
            target_a: 22,
            target_b: 30,
            radius_index: R10,
            ..Default::default()
        },
    );
    // A proc's spell at its caster.
    let on_hit = proc_aura(
        ON_HIT,
        AURA_PROC_TRIGGER_SPELL,
        procs::PROC_DONE_MELEE,
        AT_CASTER,
        0,
    );
    let at_caster = self_aura(AT_CASTER, AURA_DUMMY, 0, 1, LASTS_LONG);
    // A stun on oneself of fifteen seconds, its hold checked.
    let mut self_stun = self_aura(SELF_STUN, AURA_MOD_STUN, 0, 1, LASTS_15S);
    self_stun.attributes[0] |= HEARTBEAT;
    insert(
        d,
        [
            sweeping,
            slots,
            slot_4,
            untimed,
            cat,
            bear,
            talent,
            nature,
            stealth,
            mark,
            stun,
            wholly,
            reaction,
            reacted,
            no_dodge,
            no_parry,
            all_block,
            timed,
            untimed_guardian,
            here,
            ritual,
            kick,
            summon_totem,
            magnet,
            circle,
            on_hit,
            at_caster,
            self_stun,
        ],
    );
}

fn charges(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    kit::holder(m, g, spell).map(|h| h.charges)
}

fn restat(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().apply_stats(&data);
}

/// `owner`'s summoned objects: each one's slot, time left, place, maker
/// and spell, by slot.
fn slotted(m: &Map, owner: Guid) -> Vec<(u8, Option<u128>, Vector3d, Guid, u32)> {
    let now = m.now;
    let mut out: Vec<_> = m
        .objects
        .summoned
        .iter()
        .filter(|s| s.owner == Some(owner))
        .filter_map(|s| {
            let o = m.gameobject(s.guid)?;
            Some((
                s.slot,
                s.expires.map(|t| t.duration_since(now).as_millis()),
                o.position,
                o.fields.get_guid(OBJECT_FIELD_CREATED_BY),
                s.spell,
            ))
        })
        .collect();
    out.sort_by_key(|x| x.0);
    out
}

/// `owner`'s guardian of `spell`: its kind, time left and how far from
/// its owner.
fn guardian(m: &Map, owner: Guid, spell: u32) -> Option<(pets::Kind, Option<u128>, f32)> {
    let here = m.position_of(owner)?;
    m.players[&owner].owned.guardians.iter().find_map(|g| {
        let st = pets::state(m, *g).filter(|s| s.summon_spell == spell)?;
        let at = m.position_of(*g)?;
        Some((
            st.kind,
            st.expires.map(|t| t.duration_since(m.now).as_millis()),
            dist(at, here),
        ))
    })
}

fn channel_object(m: &Map, g: Guid) -> Guid {
    m.unit_fields(g)
        .map_or(Guid::zero(), |f| f.get_guid(UNIT_FIELD_CHANNEL_OBJECT))
}

/// A blow blocked in part comes off the shield with the damage after the
/// block (100 blocked by 12 is 88), as combat makes it a hit and a block;
/// Sweeping Strikes puts that on another enemy near and spends a charge,
/// five to four. A blow blocked whole, a block alone, sets off nothing
/// and spends none (Thistle Tea's "Sweeping Strikes copies a partially
/// blocked hit and spends one charge"; mangos-classic's
/// `HandleDummyAuraProc` for 12292 and `PROC_EX_BLOCK` with a normal
/// hit).
#[tokio::test]
async fn sweeping_strikes_copies_a_partly_blocked_blow_and_spends_a_charge() {
    let blocked = combat::blow(combat::Outcome::Block, 100, 100, 12, 2.0, 1.0);
    assert_eq!((blocked.2, blocked.3), (88, 12), "88 of 100 through");
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E1", "Sweeper", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let victim = kit::dummy(m, a, 60, 3.0);
            let other = kit::dummy(m, a, 60, 4.0);
            let s = m.realm.data.spells.get(SWEEPING_STRIKES).cloned().unwrap();
            kit::apply_template(m, a, a, &s);
            let start = charges(m, a, SWEEPING_STRIKES);
            let blow = |ex: u32, damage: u32| Event {
                actor: a,
                target: victim,
                actor_flags: procs::PROC_DONE_MELEE,
                target_flags: procs::PROC_TAKEN_MELEE,
                ex,
                damage,
                school: 1,
                ..Default::default()
            };
            let before = kit::health(m, other);
            procs::trigger(m, blow(procs::EX_BLOCK | procs::EX_NORMAL_HIT, 88));
            let partly = (
                before - kit::health(m, other),
                charges(m, a, SWEEPING_STRIKES),
            );
            let before = kit::health(m, other);
            procs::trigger(m, blow(procs::EX_BLOCK, 0));
            let wholly = (
                before - kit::health(m, other),
                charges(m, a, SWEEPING_STRIKES),
            );
            (start, partly, wholly)
        })
        .await
        .unwrap();
    assert_eq!(out.0, Some(5), "five charges");
    assert_eq!(out.1, (88, Some(4)), "swept, a charge spent");
    assert_eq!(out.2, (0, Some(4)), "a whole block sets off nothing");
}

/// Summons into slots 1 to 4 at a destination of (0, 0, 0) are their
/// caster's, by the spell, for its minute, each in its slot and at that
/// very place, made by the caster; cast again, each takes its slot's
/// place, still four. One of no end stands where its caster does, in slot
/// 1, with no time to it (Thistle Tea's "creates each slot on the caster
/// with an explicit zero destination" and "untimed summons use the
/// owner's position and default to the first slot"; mangos-classic's
/// `EffectSummonObject`).
#[tokio::test]
async fn slot_summons_are_their_casters_and_an_untimed_one_stands_where_they_are() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E2", "Placer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let zero = Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            };
            spells::cast_triggered_at(m, a, SLOTS_1_TO_3, zero);
            spells::cast_triggered_at(m, a, SLOT_4, zero);
            let first = slotted(m, a);
            spells::cast_triggered_at(m, a, SLOTS_1_TO_3, zero);
            let again = slotted(m, a);
            let here = m.players[&a].position();
            spells::cast_triggered(m, a, UNTIMED, None);
            let untimed: Vec<_> = slotted(m, a)
                .into_iter()
                .filter(|x| x.4 == UNTIMED)
                .collect();
            (first, again, here, untimed)
        })
        .await
        .unwrap();
    let (first, again, here, untimed) = out;
    let zero = Vector3d {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    assert_eq!(
        first.iter().map(|x| x.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "a slot each"
    );
    for (slot, left, place, maker, spell) in &first {
        assert_eq!(*left, Some(60_000), "slot {slot}: a minute");
        assert!(dist(*place, zero) < 0.01, "slot {slot}: at {place:?}");
        assert_eq!(*maker, a, "slot {slot}: the caster's");
        let by = if *slot == 4 { SLOT_4 } else { SLOTS_1_TO_3 };
        assert_eq!(*spell, by, "slot {slot}: its spell");
    }
    assert_eq!(
        again.iter().map(|x| x.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "each replaced in its slot"
    );
    assert_eq!(untimed.len(), 1, "one of no end: {untimed:?}");
    let (slot, left, place, maker, _) = untimed[0];
    assert_eq!((slot, left, maker), (1, None, a), "slot 1, no time");
    assert!(dist(place, here) < 0.01, "where its caster is: {place:?}");
}

/// A form's passive talent, its holder given charges and a proc's wait,
/// is kept as it is when the form's passives are restored: one holder,
/// its charges and wait untouched (Thistle Tea's "restoring preserves
/// holder charge and proc state").
#[tokio::test]
async fn a_form_passive_restored_keeps_its_charges() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E3A", "Keeper", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[TALENT]);
            let out_of_form = has(m, a, TALENT);
            kit::apply(m, a, a, CAT);
            let in_cat = has(m, a, TALENT);
            let ready = m.now + Duration::from_secs(5);
            if let Some(h) =
                auras_mut(m, a).and_then(|x| x.holders.iter_mut().find(|h| h.spell == TALENT))
            {
                h.charges = 2;
                h.proc_ready_at = Some(ready);
            }
            spells::restore_form_passives(m, a);
            let held: Vec<(u32, Option<std::time::Instant>)> = auras(m, a)
                .map(|x| {
                    x.holders
                        .iter()
                        .filter(|h| h.spell == TALENT)
                        .map(|h| (h.charges, h.proc_ready_at))
                        .collect()
                })
                .unwrap_or_default();
            (out_of_form, in_cat, held, ready)
        })
        .await
        .unwrap();
    let (out_of_form, in_cat, held, ready) = out;
    assert!(!out_of_form, "not out of form");
    assert!(in_cat, "in cat form");
    assert_eq!(held, vec![(2, Some(ready))], "kept as it was");
}

/// A form's passive talent unlearned in cat form goes, and doesn't come
/// back in bear form (Thistle Tea's "unlearning prevents later form
/// restoration").
#[tokio::test]
async fn an_unlearned_form_passive_stays_gone() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E3B", "Forgetter", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[TALENT]);
            kit::apply(m, a, a, CAT);
            let in_cat = has(m, a, TALENT);
            misc::forget_spells(m, a, &[TALENT]);
            let unlearned = has(m, a, TALENT);
            kit::apply(m, a, a, BEAR);
            (in_cat, unlearned, form(m, a), has(m, a, TALENT))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false, FORM_BEAR, false));
}

/// Dying in cat form takes a form's passive talent with the form; dead in
/// the form, or a ghost, it isn't restored; risen and in cat form again,
/// it is (Thistle Tea's "death and release cannot restore talents until
/// resurrection and form entry").
#[tokio::test]
async fn a_form_passive_waits_for_life_and_the_form() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E3C", "Riser", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[TALENT]);
            kit::apply(m, a, a, CAT);
            let in_cat = has(m, a, TALENT);
            death::kill(m, a);
            let dead = has(m, a, TALENT);
            // The form put back by hand: only death keeps it off.
            let set_form = |m: &mut Map, f: u8| {
                if let Some(x) = m.unit_fields_mut(a) {
                    x.set_byte(UNIT_FIELD_BYTES_1, 2, f);
                }
            };
            set_form(m, FORM_CAT);
            spells::restore_form_passives(m, a);
            let dead_in_cat = has(m, a, TALENT);
            let now = m.now;
            death::become_ghost(m.players.get_mut(&a).unwrap(), now);
            spells::restore_form_passives(m, a);
            let ghost_in_cat = has(m, a, TALENT);
            set_form(m, 0);
            let risen = death::resurrect(m, a, 1.0);
            let alive = has(m, a, TALENT);
            kit::apply(m, a, a, CAT);
            (
                in_cat,
                dead,
                dead_in_cat,
                ghost_in_cat,
                risen,
                alive,
                has(m, a, TALENT),
            )
        })
        .await
        .unwrap();
    assert!(out.0, "in cat form");
    assert!(!out.1, "gone with death");
    assert!(!out.2, "not while dead");
    assert!(!out.3, "not while a ghost");
    assert!(out.4, "risen");
    assert!(!out.5, "not out of form");
    assert!(out.6, "back in cat form");
}

/// Aura of Nature draws one unmarked out of stealth and stuns them not;
/// one with the dragon's mark it stuns (Thistle Tea's "Aura of Nature
/// only draws unmarked players out of stealth" and "stuns players
/// carrying either dragon mark"; vmangos's Emerald Dragons' script).
#[tokio::test]
async fn the_dream_dragons_aura_draws_the_unmarked_out_of_stealth_unstunned() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E4", "Sleeper", Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            let dragon = kit::dummy(m, a, 60, 5.0);
            let s = m.realm.data.spells.get(AURA_OF_NATURE).cloned().unwrap();
            kit::apply(m, a, a, STEALTH);
            let hidden = has(m, a, STEALTH);
            let handled = boss_mechanics::dummy(m, dragon, a, &s);
            let unmarked = (has(m, a, STEALTH), has(m, a, MARK_STUN));
            kit::apply(m, a, a, STEALTH);
            kit::apply(m, dragon, a, MARK);
            boss_mechanics::dummy(m, dragon, a, &s);
            let marked = (has(m, a, STEALTH), has(m, a, MARK), has(m, a, MARK_STUN));
            (hidden, handled, unmarked, marked)
        })
        .await
        .unwrap();
    assert_eq!((out.0, out.1), (true, true), "stealthed, the script's");
    assert_eq!(out.2, (false, false), "out of stealth, not stunned");
    assert_eq!(out.3, (false, true, true), "the marked stunned");
}

/// An ability a block takes whole, at a warrior with a shield who can
/// neither dodge nor parry and blocks all: it does nothing, the client is
/// told it was blocked, and the warrior's reaction to a block of a melee
/// ability taken goes off (Thistle Tea's "blocked melee abilities reach
/// victim proc reactions"; mangos-classic's
/// `SPELL_ATTR_EX3_COMPLETELY_BLOCKED` in `MeleeSpellHitResult` and the
/// victim's `PROC_FLAG_TAKEN_MELEE_SPELL_HIT` with `PROC_EX_BLOCK`).
#[tokio::test]
async fn a_wholly_blocked_ability_procs_the_blockers_reaction() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P9E5", "Blocker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[SPELL_BLOCK]);
            p3::equip(m, a, SHIELD, OFF_HAND);
            // Weapons drawn: sheathed, a player doesn't block.
            m.players
                .get_mut(&a)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_2, 0, 1);
            for s in [NO_DODGE, NO_PARRY, ALL_BLOCK, BLOCK_REACTION] {
                kit::apply(m, a, a, s);
            }
            restat(m, a);
            let block = m.players[&a].fields.get_f32(PLAYER_BLOCK_PERCENTAGE);
            let kobold = kit::dummy(m, a, 60, 3.0);
            let before = kit::health(m, a);
            spells::cast_triggered(m, kobold, WHOLLY_BLOCKED, Some(a));
            (block, before, kit::health(m, a), has(m, a, REACTION))
        })
        .await
        .unwrap();
    let (block, before, after, reacted) = out;
    assert!(block >= 100.0, "blocks all: {block}");
    assert_eq!(after, before, "nothing done");
    assert!(reacted, "the reaction");
    assert_eq!(kit::missed(&mut c, WHOLLY_BLOCKED).await, 5, "blocked");
}

/// A summon of fifteen seconds gives a guardian of its caster's that
/// lasts its fifteen seconds, two yards before them; one of no end lasts
/// for good; one at the caster's own place stands there (Thistle Tea's
/// "controlled summons execute on the caster with the requested lifetime
/// and destination"; mangos-classic's `EffectSummonGuardian`).
#[tokio::test]
async fn a_summon_lasts_its_spells_time_beside_its_caster() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E6", "Summoner", Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            face_east(m, a);
            let mut out = Vec::new();
            for spell in [SUMMON_TIMED, SUMMON_UNTIMED, SUMMON_HERE] {
                spells::cast_triggered(m, a, spell, None);
                out.push(guardian(m, a, spell));
            }
            out
        })
        .await
        .unwrap();
    let timed = out[0].expect("a guardian for a time");
    assert_eq!((timed.0, timed.1), (pets::Kind::Guardian, Some(15_000)));
    assert!(timed.2 > 1.5 && timed.2 < 2.5, "beside: {}", timed.2);
    let untimed = out[1].expect("a guardian for good");
    assert_eq!((untimed.0, untimed.1), (pets::Kind::Guardian, None));
    let here = out[2].expect("a guardian at the caster's place");
    assert!(here.2 < 0.01, "where the caster stands: {}", here.2);
}

/// A ritual's channel interrupted as a silence stops a cast: the channel
/// is over, its object field cleared, and the altar it raised is gone
/// (Thistle Tea's "releases owned objects through channel cancellation";
/// mangos-classic's `EffectInterruptCast` and the channel's object
/// removed with it).
#[tokio::test]
async fn an_interrupt_ends_an_owned_rituals_channel_and_takes_its_object() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E7", "Ritualist", Class::Warlock).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[RITUAL]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let cast = cast_fresh(m, a, RITUAL, Targets::default());
            let altar = m
                .objects
                .summoned
                .iter()
                .find(|s| s.owner == Some(a) && s.spell == RITUAL)
                .map(|s| s.guid);
            let raised = (kit::casting(m, a), channel_object(m, a) == altar.unwrap());
            spells::cast_triggered(m, kobold, KICK, Some(a));
            let gone = (
                kit::casting(m, a),
                channel_object(m, a),
                altar.is_some_and(|g| m.gameobject(g).is_none()),
            );
            (cast, altar.is_some(), raised, gone)
        })
        .await
        .unwrap();
    assert_eq!((out.0, out.1), (true, true), "cast, raised");
    assert_eq!(out.2, (Some(RITUAL), true), "channelled at the altar");
    assert_eq!(out.3, (None, Guid::zero(), true), "interrupted, gone");
}

/// A totem whose spell is a helpful aura on itself has it from its
/// summoning; spent (gone), its turns after don't cast it again (Thistle
/// Tea's "a spent passive totem does not recreate its aura";
/// mangos-classic's `Totem::Summon` casting a passive totem's spell
/// once).
#[tokio::test]
async fn a_passive_totems_spent_aura_stays_gone() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E8", "Shaman", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            spells::cast_triggered(m, a, SUMMON_TOTEM, None);
            let totem = m.players[&a].owned.totems[0].expect("a totem");
            let magnet = (
                has(m, totem, MAGNET),
                charges(m, totem, MAGNET),
                pets::state(m, totem).map(|s| s.totem_spell),
            );
            remove_spell(m, totem, MAGNET, None);
            for _ in 0..3 {
                pass(m, 5_000);
                pets::update(m);
            }
            let now = m.now;
            let next = pets::state(m, totem).map(|s| s.next_think > now + Duration::from_secs(60));
            (magnet, has(m, totem, MAGNET), next)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, Some(1), Some(MAGNET)), "its spell on it");
    assert!(!out.1, "spent, not cast again");
    assert_eq!(out.2, Some(true), "its turns put off");
}

/// A heal about its caster's place (22) of the friends there (30), aimed
/// at an enemy: it lands on the caster and a friend five yards off, not
/// on the enemy aimed at, nor on a friend twenty yards off (Thistle Tea's
/// "friendly source areas ignore an enemy selection"; mangos-classic's
/// `TARGET_ALL_FRIENDLY_UNITS_AROUND_CASTER`).
#[tokio::test]
async fn a_friendly_area_about_the_caster_ignores_the_unit_aimed_at() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E9A", "Healer", Class::Priest).await;
    let (_d, b, _) = human(&realm, "P9E9B", "Near", Class::Warrior).await;
    let (_e, far, _) = human(&realm, "P9E9C", "Far", Class::Warrior).await;
    let out = map
        .query(move |m| {
            face_east(m, a);
            let kobold = kit::dummy(m, a, 60, 3.0);
            let to = off(m, a, 0.0, 5.0);
            put(m, b, to);
            let to = off(m, a, 0.0, 20.0);
            put(m, far, to);
            let s = m.realm.data.spells.get(CIRCLE).cloned().unwrap();
            let mods = mods::for_spell(m, a, &s);
            let hit = targets::effect_targets(m, a, &s, 0, &at(kobold), &mods);
            (kobold, hit)
        })
        .await
        .unwrap();
    let (kobold, hit) = out;
    assert!(hit.contains(&a) && hit.contains(&b), "{hit:?}");
    assert!(!hit.contains(&kobold), "not the enemy aimed at");
    assert!(!hit.contains(&far), "not twenty yards off");
}

/// A proc on its owner's melee blow whose spell is at its caster lands
/// on the owner, not on the victim the blow proposed (Thistle Tea's
/// "redirects a victim-proposed proc to its DBC caster target";
/// mangos-classic's `TriggerSpell` with the spell's own targets).
#[tokio::test]
async fn a_procs_spell_at_its_caster_lands_on_its_owner() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E10", "Procer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            kit::apply(m, a, a, ON_HIT);
            procs::trigger(
                m,
                Event {
                    actor: a,
                    target: kobold,
                    actor_flags: procs::PROC_DONE_MELEE,
                    ex: procs::EX_NORMAL_HIT,
                    damage: 10,
                    school: 1,
                    ..Default::default()
                },
            );
            (has(m, a, AT_CASTER), has(m, kobold, AT_CASTER))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false));
}

/// A stun of fifteen seconds that checks its hold, cast on oneself, is
/// harmful still and so takes a heartbeat: a player's breaks at a time
/// drawn (twelve seconds at the spread's middle), a creature's rolls
/// again every five seconds (Thistle Tea's "routes self heartbeat auras
/// through the owner boundary for a sampled break"; vmangos's
/// `SPELL_ATTR_HEARTBEAT_RESIST_CHECK`).
#[tokio::test]
async fn a_stun_cast_on_oneself_samples_its_heartbeat() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9E11", "Stunned", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let s = m.realm.data.spells.get(SELF_STUN).cloned().unwrap();
            let harmful = !positive(&s);
            let now = m.now;
            let beat = |m: &Map, g: Guid| {
                kit::holder(m, g, SELF_STUN)
                    .and_then(|h| h.heartbeat)
                    .map(|b| {
                        (
                            b.break_at.map(|t| t.duration_since(now).as_millis()),
                            b.next_check.map(|t| t.duration_since(now).as_millis()),
                        )
                    })
            };
            spells::cast_triggered(m, a, SELF_STUN, None);
            spells::cast_triggered(m, kobold, SELF_STUN, None);
            (harmful, beat(m, a), beat(m, kobold))
        })
        .await
        .unwrap();
    assert!(out.0, "harmful");
    assert_eq!(out.1, Some((Some(12_000), None)), "a player's break");
    assert_eq!(out.2, Some((None, Some(5_000))), "a creature's roll");
}
