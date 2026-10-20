//! Round 9, against Thistle Tea's casting tests where they are 1.12's
//! (`cast_movement_test.exs`, `cast_test.exs`, `cast_validation_test.exs`,
//! `caster_location_test.exs`, `casting_test.exs` and
//! `deferred_cooldowns_test.exs`): what moving breaks; a channel of no
//! end; Dismiss Pet's target; cooldowns that wait for their event; a
//! leap's radius and whom a caster-relative effect lands on; a duel's
//! flag; a channel at the pet cancelled; a channel's cost each second; a
//! taming rod cut short; and a ritual cancelled.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, pass};
use super::tests_p6_wspell::{face_east, off, put};
use crate::game::Realm;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_ON_NEXT_SWING, SpellDuration, SpellEffect, SpellTemplate,
};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::combat::dist;
use crate::game::map::spells::{self, Targets, checks, cooldowns, fail, mods, targets};
use crate::game::map::{Map, MapHandle, auras, objects, pets};

/// Two-second casts that moving breaks and that it doesn't (its aura and
/// channel interrupt flags naming moving, its own not); an instant that
/// moving would break; a next-swing blow.
const MOVE_BREAKS: u32 = 997_010;
const MOVE_KEEPS: u32 = 997_011;
const INSTANT_ONE: u32 = 997_012;
const NEXT_SWING: u32 = 997_013;
/// A channel of no end after a two-second cast, healing its caster each
/// second.
const NO_END: u32 = 997_014;
/// Dismiss Pet's kind, for beasts only by its data.
const DISMISS_PET: u32 = 2641;
/// An aura whose cooldown of eight seconds waits for it to go, on the
/// global cooldown.
const ON_EVENT: u32 = 997_015;
/// A leap ten yards back, and half again to its radius (a mage's).
const LEAP_BACK: u32 = 997_016;
const WIDER: u32 = 997_017;
/// Effects about the caster's back, first at the unit, first about the
/// destination.
const AT_UNIT: u32 = 997_018;
const AT_PLACE: u32 = 997_019;
/// A duel whose second target is a minion's place.
const DUEL: u32 = 997_020;
/// Eyes of the Beast's kind: a channel possessing the pet.
const EYES: u32 = 997_021;
/// A three-second channel of 10 mana and 5 a second.
const MANA_CHANNEL: u32 = 997_022;
/// The Ice Claw Bear's rod and its tame, made up.
const ROD: u32 = 19_548;
const TAME: u32 = 19_597;
/// An aura whose cooldown waits for it, of category 94; another spell of
/// that category.
const SHARED: u32 = 997_023;
const SAME_CATEGORY: u32 = 997_024;
/// A channel that raises a ritual's altar, its cooldown waiting for it.
const RITUAL: u32 = 997_025;
const ALTAR: u32 = 997_026;
/// A tameable creature of no type.
const PET_BEAST: u32 = 9_970;
const CATEGORY: u32 = 94;
const NO_END_INDEX: u32 = 902;
const LASTS_3S: u32 = 903;
const LASTS_10S: u32 = 904;
const R10: u32 = 13;
/// `SPELL_ATTR_DISABLED_WHILE_ACTIVE`: the cooldown starts at an event.
const ON_EVENT_ATTR: u32 = 0x200_0000;

fn at_caster(id: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    effect(
        &mut s,
        0,
        SpellEffect {
            effect: spells::EFFECT_DUMMY,
            target_a: 1,
            ..Default::default()
        },
    );
    s
}

fn leap(id: u32, a: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.family_name = 3;
    s.family_flags = 1;
    effect(
        &mut s,
        0,
        SpellEffect {
            effect: spells::EFFECT_LEAP,
            target_a: a,
            target_b: 48,
            radius_index: R10,
            ..Default::default()
        },
    );
    s
}

fn channel(mut s: SpellTemplate, duration: u32) -> SpellTemplate {
    s.attributes[1] |= ATTR_EX_CHANNELED;
    s.duration_index = duration;
    s
}

fn data(d: &mut GameData) {
    d.spells.durations.insert(
        NO_END_INDEX,
        SpellDuration {
            base: -1,
            per_level: 0,
            max: -1,
        },
    );
    for (i, ms) in [(LASTS_3S, 3_000), (LASTS_10S, 10_000)] {
        d.spells.durations.insert(
            i,
            SpellDuration {
                base: ms,
                per_level: 0,
                max: ms,
            },
        );
    }
    d.spells.radii.insert(R10, 10.0);
    d.creature_templates.insert(
        PET_BEAST,
        CreatureTemplate {
            entry: PET_BEAST,
            name: "Typeless Beast".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 60,
            level_max: 60,
            faction: 7,
            type_flags: 1,
            ..Default::default()
        },
    );
    d.gameobject_templates.insert(
        ALTAR,
        crate::game::tests_p3_entity_objects::object(ALTAR, objects::GO_RITUAL, "Altar", &[(0, 2)]),
    );
    let mut breaks = at_caster(MOVE_BREAKS);
    breaks.casting_time_index = CAST_2S;
    breaks.interrupt_flags = 1;
    let mut keeps = at_caster(MOVE_KEEPS);
    keeps.casting_time_index = CAST_2S;
    keeps.aura_interrupt_flags = 8;
    keeps.channel_interrupt_flags = 8;
    let mut instant = at_caster(INSTANT_ONE);
    instant.interrupt_flags = 1;
    let mut swing = bare(NEXT_SWING, 0);
    swing.dmg_class = 2;
    swing.attributes[0] |= ATTR_ON_NEXT_SWING;
    swing.interrupt_flags = 1;
    effect(&mut swing, 0, damage(10));
    let mut no_end = channel(bare(NO_END, 0), NO_END_INDEX);
    no_end.casting_time_index = CAST_2S;
    no_end.interrupt_flags = 2;
    effect(
        &mut no_end,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(auras::AURA_PERIODIC_HEAL, 0, 1, 1)
        },
    );
    let mut dismiss = bare(DISMISS_PET, 0);
    dismiss.target_creature_type = 1;
    effect(
        &mut dismiss,
        0,
        SpellEffect {
            effect: spells::EFFECT_POWER_DRAIN,
            misc_value: 4,
            base_points: 49_999,
            base_dice: 1,
            target_a: 5,
            ..Default::default()
        },
    );
    effect(
        &mut dismiss,
        1,
        SpellEffect {
            effect: pets::EFFECT_DISMISS_PET,
            target_a: 5,
            ..Default::default()
        },
    );
    let waits = |id: u32, recovery: u32, duration: u32| {
        let mut s = bare(id, 0);
        s.attributes[0] |= ON_EVENT_ATTR;
        s.recovery_time = recovery;
        s.duration_index = duration;
        effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        s
    };
    let mut on_event = waits(ON_EVENT, 8_000, LASTS_5S);
    on_event.start_recovery_category = 133;
    on_event.start_recovery_time = 1_500;
    let mut shared = waits(SHARED, 180_000, LASTS_LONG);
    shared.category = CATEGORY;
    let mut same_category = at_caster(SAME_CATEGORY);
    same_category.category = CATEGORY;
    let mut wider = bare(WIDER, 0);
    wider.family_name = 3;
    wider.duration_index = LASTS_LONG;
    effect(
        &mut wider,
        0,
        SpellEffect {
            item_type: 1,
            ..aura(auras::AURA_ADD_PCT_MODIFIER, mods::OP_RADIUS as i32, 50, 1)
        },
    );
    let mut duel = bare(DUEL, 0);
    effect(
        &mut duel,
        0,
        SpellEffect {
            effect: spells::EFFECT_DUEL,
            target_a: 25,
            target_b: 32,
            ..Default::default()
        },
    );
    let mut eyes = channel(bare(EYES, 0), LASTS_15S);
    effect(&mut eyes, 0, aura(auras::AURA_MOD_POSSESS_PET, 0, 1, 5));
    let mut mana = channel(at_caster(MANA_CHANNEL), LASTS_3S);
    mana.mana_cost = 10;
    mana.mana_per_second = 5;
    let mut rod = channel(bare(ROD, 0), LASTS_10S);
    effect(&mut rod, 0, aura(auras::AURA_DUMMY, 0, 1, 6));
    let mut tame = bare(TAME, 0);
    tame.duration_index = LASTS_LONG;
    effect(&mut tame, 0, aura(auras::AURA_MOD_CHARM, 0, 1, 6));
    let mut ritual = channel(bare(RITUAL, 0), LASTS_15S);
    ritual.attributes[0] |= ON_EVENT_ATTR;
    ritual.recovery_time = 60_000;
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
    insert(
        d,
        [
            breaks,
            keeps,
            instant,
            swing,
            no_end,
            dismiss,
            on_event,
            shared,
            same_category,
            leap(LEAP_BACK, 1),
            wider,
            leap(AT_UNIT, 6),
            leap(AT_PLACE, 16),
            duel,
            eyes,
            mana,
            rod,
            tame,
            ritual,
        ],
    );
}

/// A level-60 dwarf hunter facing east, knowing `learn`, with a pet of no
/// creature type two yards ahead.
async fn hunter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    learn: &'static [u32],
) -> (Client, Guid, MapHandle, Guid) {
    let mut c = Client::connect(realm, account);
    let h = c.enter(name, Race::Dwarf, Class::Hunter).await;
    let map = world(realm);
    let pet = map
        .query(move |m| {
            kit::ready(m, h, 60, learn);
            face_east(m, h);
            pets::give_pet(m, h, PET_BEAST, 60).expect("a pet");
            let pet = pets::pet_of(m, h).expect("the pet out");
            let to = off(m, h, 2.0, 0.0);
            put(m, pet, to);
            pet
        })
        .await
        .unwrap();
    (c, h, map, pet)
}

/// Why `g` can't cast `spell` at no one now, off the global cooldown.
fn refused(m: &mut Map, g: Guid, spell: u32) -> Option<u8> {
    if let Some(c) = m.caster_mut(g) {
        c.gcd_until = None;
    }
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, g, &s);
    checks::check(m, g, &s, &Targets::default(), None, false, &mods)
}

/// Whether `g`'s cooldown of `spell` waits for its event, and whether one
/// runs.
fn waiting(m: &Map, g: Guid, spell: u32) -> (bool, bool) {
    let c = m.caster(g).unwrap();
    (
        c.pending_cooldowns.contains(&spell),
        c.cooldowns.get(&spell).is_some_and(|t| *t > m.now),
    )
}

fn channel_fields(m: &Map, g: Guid) -> (u32, Guid) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_u32(UNIT_CHANNEL_SPELL),
        f.get_guid(UNIT_FIELD_CHANNEL_OBJECT),
    )
}

fn east(p: Vector3d, dx: f32) -> Vector3d {
    Vector3d { x: p.x + dx, ..p }
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    dist(a, b) < 0.01
}

/// A cast its caster moves two yards under stops only if its own
/// interrupt flags name moving: one whose aura and channel interrupt flags
/// do, its own not, goes on. An instant, a triggered cast of two seconds
/// and a next-swing blow leave no cast to break, the blow still waiting
/// for its swing (Thistle Tea's "honors preparation flags and cast
/// exemptions"; mangos-classic's `SPELL_INTERRUPT_FLAG_MOVEMENT`).
#[tokio::test]
async fn a_cast_breaks_on_movement_only_by_its_interrupt_flag() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9C1", "Mover", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(
                m,
                a,
                60,
                &[MOVE_BREAKS, MOVE_KEEPS, INSTANT_ONE, NEXT_SWING],
            );
            face_east(m, a);
            let kobold = kit::dummy(m, a, 60, 3.0);
            let walk = |m: &mut Map| {
                let to = off(m, a, 2.0, 0.0);
                put(m, a, to);
                spells::moved(m, a);
            };
            let mut casts = Vec::new();
            for spell in [MOVE_BREAKS, MOVE_KEEPS] {
                cast_fresh(m, a, spell, Targets::default());
                let began = kit::casting(m, a);
                walk(m);
                casts.push((began, kit::casting(m, a)));
            }
            kit::fresh(m, a);
            let instant = cast_fresh(m, a, INSTANT_ONE, Targets::default());
            let after_instant = kit::casting(m, a);
            spells::cast_triggered(m, a, MOVE_BREAKS, None);
            let after_triggered = kit::casting(m, a);
            let swing = cast_fresh(m, a, NEXT_SWING, at(kobold));
            walk(m);
            let queued = m.combat(a).and_then(|c| c.queued);
            (
                casts,
                (instant, after_instant, after_triggered),
                (swing, kit::casting(m, a), queued),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        vec![
            (Some(MOVE_BREAKS), None),
            (Some(MOVE_KEEPS), Some(MOVE_KEEPS))
        ],
        "broken by its own flag, not by the others"
    );
    assert_eq!(out.1, (true, None, None), "an instant, a triggered cast");
    assert_eq!(out.2, (true, None, Some(NEXT_SWING)), "a next swing");
}

/// A channel of a duration of -1 has no end: its start tells the client
/// -1; it ticks each second, still after a minute, until it is cancelled,
/// which takes its aura and clears the channel. The cast before it is put
/// back by a blow no further than its start (Thistle Tea's "indefinite
/// channels retain their duration and schedule future ticks";
/// mangos-classic's channel with no timer, `Spell::Delayed`).
#[tokio::test]
async fn a_channel_of_no_end_runs_until_cancelled() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P9C2", "Channeller", Class::Priest).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[NO_END]);
            cast_fresh(m, a, NO_END, Targets::default());
            pass(m, 500);
            spells::pushback(m, a);
            let now = m.now;
            let left = m
                .caster(a)
                .and_then(|c| c.casting.as_ref())
                .map(|x| x.ends.saturating_duration_since(now));
            pass(m, 2_000);
            let channel = m
                .caster(a)
                .and_then(|c| c.casting.as_ref())
                .map(|x| (x.spell, x.channel_until.is_some()));
            for _ in 0..60 {
                pass(m, 1_000);
            }
            let still = (
                kit::casting(m, a),
                kit::holder(m, a, NO_END).map(|h| h.effects[0].ticks),
                channel_fields(m, a).0,
            );
            spells::cancel(m, a);
            let after = (
                kit::casting(m, a),
                kit::has(m, a, NO_END),
                channel_fields(m, a),
            );
            (left, channel, still, after)
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        Some(Duration::from_millis(2_000)),
        "put back half a second"
    );
    assert_eq!(out.1, Some((NO_END, true)), "the channel");
    assert_eq!(out.2, (Some(NO_END), Some(60), NO_END), "a minute on");
    assert_eq!(out.3, (None, false, (0, Guid::zero())), "cancelled");
    let start = c
        .wait_for("MSG_CHANNEL_START", |msg| match msg {
            ServerOpcodeMessage::MSG_CHANNEL_START(s) if s.spell == NO_END => Some(s.duration),
            _ => None,
        })
        .await;
    assert_eq!(start, u32::MAX, "-1");
}

/// Dismiss Pet, its data for beasts alone, is cast at a pet of no type:
/// its target is the pet, whose type isn't asked, and the pet goes
/// (Thistle Tea's "dismiss pet ignores the DBC creature-type mask";
/// mangos-classic's `CheckTargetCreatureType` skipping 2641).
#[tokio::test]
async fn dismiss_pet_ignores_its_creature_type() {
    let realm = fast_realm(data);
    let (_c, h, map, pet) = hunter(&realm, "P9C3", "Dismisser", &[DISMISS_PET]).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let s = m.realm.data.spells.get(DISMISS_PET).cloned().unwrap();
            let typed = checks::unit_type_allowed(m, &s, pet);
            let why = refused(m, h, DISMISS_PET);
            let cast = cast_fresh(m, h, DISMISS_PET, Targets::default());
            (typed, why, cast, pets::pet_of(m, h))
        })
        .await
        .unwrap();
    assert_eq!(out, (false, None, true, None));
}

/// A spell whose cooldown waits for its aura to go is refused without a
/// word to the client while it waits, and while it runs once the aura is
/// gone; it may be cast again once that is done (Thistle Tea's "deferred
/// spells reject cooldowns without disturbing the client's disabled
/// state"; mangos-classic's `SPELL_ATTR_DISABLED_WHILE_ACTIVE`).
#[tokio::test]
async fn a_cooldown_on_event_spell_is_held_silently() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P9C4", "Waiter", Class::Rogue).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[ON_EVENT]);
            let cast = cast_fresh(m, a, ON_EVENT, Targets::default());
            let pending = (waiting(m, a, ON_EVENT), refused(m, a, ON_EVENT));
            let again = spells::cast(m, a, ON_EVENT, Targets::default(), None);
            auras::remove_spell(m, a, ON_EVENT, Some(a));
            let running = (waiting(m, a, ON_EVENT), refused(m, a, ON_EVENT));
            pass(m, 9_000);
            (cast, pending, again, running, refused(m, a, ON_EVENT))
        })
        .await
        .unwrap();
    assert!(out.0, "cast");
    assert_eq!(
        out.1,
        ((true, false), Some(fail::DONT_REPORT)),
        "waiting for its aura"
    );
    assert!(!out.2, "refused");
    assert_eq!(
        out.3,
        ((false, true), Some(fail::DONT_REPORT)),
        "running once it went"
    );
    assert_eq!(out.4, None, "done");
    let told = c
        .wait_frame("SMSG_CAST_RESULT", spells::SMSG_CAST_RESULT, |b| {
            b[..4] == ON_EVENT.to_le_bytes() && b[4] == 2
        })
        .await;
    assert_eq!(told[5], fail::DONT_REPORT);
}

/// A leap back of ten yards goes fifteen under half again to its radius
/// (a modifier of its family's), its caster facing as they were (Thistle
/// Tea's "uses secondary selectors and modified effect radii";
/// mangos-classic's `SPELLMOD_RADIUS` on the caster-relative place).
#[tokio::test]
async fn a_leaps_radius_takes_its_casters_modifiers() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9C5", "Leaper", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[LEAP_BACK]);
            face_east(m, a);
            let start = m.players[&a].position();
            let landed = |m: &mut Map| {
                cast_fresh(m, a, LEAP_BACK, Targets::default());
                let p = &m.players[&a];
                (p.position(), p.movement.orientation)
            };
            let plain = landed(m);
            m.near_teleport(a, start, 0.0);
            kit::apply(m, a, a, WIDER);
            let wider = landed(m);
            (start, plain, wider)
        })
        .await
        .unwrap();
    let (start, plain, wider) = out;
    assert!(near(plain.0, east(start, -10.0)), "{plain:?}");
    assert!(near(wider.0, east(start, -15.0)), "{wider:?}");
    assert_eq!((plain.1, wider.1), (0.0, 0.0), "facing kept");
}

/// An effect about the caster's back lands on whom its first target says,
/// whatever was selected: the caster's own on the caster, a unit's on the
/// unit, one about the destination on those within its ten yards of it
/// (Thistle Tea's "ignores unrelated selections while retaining explicit
/// unit and area recipients"; mangos-classic's `SetTargetMap`).
#[tokio::test]
async fn a_caster_relative_effect_lands_by_its_first_target() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9C6", "Pointer", Class::Mage).await;
    let out = map
        .query(move |m| {
            face_east(m, a);
            let k = kit::dummy(m, a, 60, 5.0);
            let near_it = off(m, a, 10.0, 0.0);
            let far_from_it = off(m, a, 30.0, 0.0);
            let land = |m: &Map, spell: u32, dest: Vector3d| {
                let s = m.realm.data.spells.get(spell).cloned().unwrap();
                let mods = mods::for_spell(m, a, &s);
                let t = Targets {
                    unit: Some(k),
                    dest: Some(dest),
                    ..Default::default()
                };
                targets::effect_targets(m, a, &s, 0, &t, &mods)
            };
            (
                k,
                land(m, LEAP_BACK, near_it),
                land(m, AT_UNIT, near_it),
                land(m, AT_PLACE, near_it),
                land(m, AT_PLACE, far_from_it),
            )
        })
        .await
        .unwrap();
    let (k, own, unit, place, far) = out;
    assert_eq!(own, vec![a], "the caster's own");
    assert_eq!(unit, vec![k], "the unit's");
    assert_eq!(place, vec![k], "within ten yards of the place");
    assert!(far.is_empty(), "none twenty-five yards off");
}

/// A duel's flag stands halfway between the two, its second target (a
/// minion's place) no place about its caster (Thistle Tea's "excludes duel
/// flags and the movement-specific forward leap"; mangos-zero's
/// `EffectDuel`).
#[tokio::test]
async fn a_duels_flag_stands_between_the_two() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9C7A", "Challenger", Class::Warrior).await;
    let (_d, b, _) = human(&realm, "P9C7B", "Challenged", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[DUEL]);
            face_east(m, a);
            let here = m.players[&a].position();
            let there = off(m, a, 4.0, 0.0);
            put(m, b, there);
            let cast = cast_fresh(m, a, DUEL, at(b));
            let flag = m.players[&a]
                .duel
                .as_ref()
                .and_then(|d| m.gameobject(d.flag))
                .map(|o| o.position);
            (cast, here, flag)
        })
        .await
        .unwrap();
    let (cast, here, flag) = out;
    assert!(cast, "challenged");
    assert!(
        flag.is_some_and(|f| near(f, east(here, 2.0))),
        "halfway: {flag:?} from {here:?}"
    );
}

/// Eyes of the Beast cancelled: its aura comes off the pet, the hunter's
/// channel fields clear and their client is told the channel is over; the
/// pet is theirs again, no longer moved by them (Thistle Tea's "removes a
/// channel aura from its recorded target after pet possession changes
/// ownership"; mangos-classic's `Spell::cancel`).
#[tokio::test]
async fn a_cancelled_eyes_of_the_beast_leaves_the_pet() {
    let realm = fast_realm(data);
    let (mut c, h, map, pet) = hunter(&realm, "P9C8", "Eyer", &[EYES]).await;
    let out = map
        .query(move |m| {
            let cast = cast_fresh(m, h, EYES, Targets::default());
            pass(m, 1_000);
            let sight = |m: &Map| m.players[&h].fields.get_guid(PLAYER_FARSIGHT);
            let possessed = |m: &Map| {
                m.creature(pet)
                    .unwrap()
                    .fields
                    .has_flag(UNIT_FIELD_FLAGS, pets::UNIT_FLAG_POSSESSED)
            };
            let on = (
                kit::has(m, pet, EYES),
                pets::eyes::controls(m, h, pet),
                sight(m) == pet,
                possessed(m),
                channel_fields(m, h).0,
            );
            spells::cancel(m, h);
            let gone = (
                kit::has(m, pet, EYES),
                pets::eyes::controls(m, h, pet),
                sight(m).is_zero(),
                possessed(m),
                channel_fields(m, h),
                pets::owner_of(m, pet),
            );
            (cast, on, gone)
        })
        .await
        .unwrap();
    assert!(out.0, "cast");
    assert_eq!(out.1, (true, true, true, true, EYES), "through its eyes");
    assert_eq!(
        out.2,
        (false, false, true, false, (0, Guid::zero()), Some(h)),
        "cancelled"
    );
    let time = c
        .wait_for("MSG_CHANNEL_UPDATE", |msg| match msg {
            ServerOpcodeMessage::MSG_CHANNEL_UPDATE(u) => Some(u.time),
            _ => None,
        })
        .await;
    assert_eq!(time, 0);
}

/// A channel of 10 mana and 5 a second costs 10 as it goes off and 5 more
/// each second after (Thistle Tea's "snapshots one resolution for launch
/// and channel ticks"; mangos-classic's `SpellAuraHolder` paying the
/// spell's `manaPerSecond`).
#[tokio::test]
async fn a_mana_channel_pays_its_cost_each_second() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9C9", "Payer", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MANA_CHANNEL]);
            kit::set_power(m, a, 0, 50);
            let cast = cast_fresh(m, a, MANA_CHANNEL, Targets::default());
            let mana = |m: &Map| kit::power(m, a, 0);
            let launched = mana(m);
            pass(m, 999);
            let early = mana(m);
            pass(m, 1);
            let one = mana(m);
            pass(m, 1_000);
            (cast, launched, early, one, mana(m))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, 40, 40, 35, 30));
}

/// A taming rod's channel cut short tames nothing: its beast is no one's,
/// with no tame on it, after the channel's time; run its course, the rod
/// tames (Thistle Tea's "an interrupted taming rod channel leaves the
/// beast wild"; vmangos's `HandleAuraDummy` on the channel's end).
#[tokio::test]
async fn an_interrupted_taming_rod_leaves_the_beast_wild() {
    assert_eq!(pets::rods::tame_of(ROD), Some(TAME));
    let realm = fast_realm(data);
    let mut c = Client::connect(&realm, "P9C10");
    let h = c.enter("Rodder", Race::Dwarf, Class::Hunter).await;
    let map = world(&realm);
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, h, 60, &[ROD]);
            let beast = kit::dummy(m, h, 60, 4.0);
            let cast = cast_fresh(m, h, ROD, at(beast));
            pass(m, 2_000);
            let channelling = kit::casting(m, h);
            spells::cancel(m, h);
            pass(m, 12_000);
            let wild = (kit::has(m, beast, TAME), pets::owner_of(m, beast));
            let other = kit::dummy(m, h, 60, 4.0);
            cast_fresh(m, h, ROD, at(other));
            pass(m, 10_000);
            let tamed = (kit::has(m, other, TAME), pets::owner_of(m, other));
            (cast, channelling, wild, tamed)
        })
        .await
        .unwrap();
    assert!(out.0, "cast");
    assert_eq!(out.1, Some(ROD), "channelling");
    assert_eq!(out.2, (false, None), "cut short: wild");
    assert_eq!(out.3, (true, Some(h)), "run its course: tamed");
}

/// A cooldown waiting for its event holds its category; let go undone, it
/// leaves no cooldown, its button cleared, and both the spell and its
/// category may be cast (Thistle Tea's "canceling a pending object
/// releases its shared category"; mangos-classic's
/// `SPELL_ATTR_DISABLED_WHILE_ACTIVE` and `SMSG_CLEAR_COOLDOWN`).
#[tokio::test]
async fn releasing_a_pending_cooldown_frees_its_category() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P9C11", "Releaser", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[SHARED, SAME_CATEGORY]);
            let cast = cast_fresh(m, a, SHARED, Targets::default());
            let held = (
                waiting(m, a, SHARED),
                refused(m, a, SHARED),
                refused(m, a, SAME_CATEGORY),
            );
            cooldowns::release(m, a, SHARED, false);
            let categories = m.caster(a).unwrap().category_cooldowns.len();
            let free = (
                waiting(m, a, SHARED),
                categories,
                refused(m, a, SHARED),
                refused(m, a, SAME_CATEGORY),
            );
            (cast, held, free)
        })
        .await
        .unwrap();
    assert!(out.0, "cast");
    assert_eq!(
        out.1,
        (
            (true, false),
            Some(fail::DONT_REPORT),
            Some(fail::NOT_READY)
        ),
        "the category held"
    );
    assert_eq!(out.2, ((false, false), 0, None, None), "let go");
    let target = c
        .wait_for("SMSG_CLEAR_COOLDOWN", |msg| match msg {
            ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(x) if x.id == SHARED => Some(x.target),
            _ => None,
        })
        .await;
    assert_eq!(target, a);
}

/// A ritual's channel cancelled before it is done: its altar goes, and
/// the cooldown that waited for it is let go, none running and the button
/// cleared (Thistle Tea's "canceled rituals clear the pending timer before
/// asynchronous object cleanup"; mangos-classic's ritual object removed
/// with its spell's cooldown).
#[tokio::test]
async fn a_cancelled_ritual_starts_no_cooldown() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P9C12", "Ritualist", Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[RITUAL]);
            let cast = cast_fresh(m, a, RITUAL, Targets::default());
            let altar = m
                .objects
                .summoned
                .iter()
                .find(|s| s.owner == Some(a) && s.spell == RITUAL)
                .map(|s| s.guid);
            let raised = (altar.is_some(), kit::casting(m, a), waiting(m, a, RITUAL));
            spells::cancel(m, a);
            let gone = altar.is_some_and(|g| m.gameobject(g).is_none());
            (cast, raised, gone, waiting(m, a, RITUAL))
        })
        .await
        .unwrap();
    assert!(out.0, "cast");
    assert_eq!(
        out.1,
        (true, Some(RITUAL), (true, false)),
        "raised, waiting"
    );
    assert!(out.2, "the altar went");
    assert_eq!(out.3, (false, false), "no cooldown");
    let target = c
        .wait_for("SMSG_CLEAR_COOLDOWN", |msg| match msg {
            ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(x) if x.id == RITUAL => Some(x.target),
            _ => None,
        })
        .await;
    assert_eq!(target, a);
}
