//! Phase 3, round 6, against Thistle Tea's `casting_combat_test.exs`: what
//! a cast does to its caster's melee: the swing started over at its
//! launch, and attacks a spell cancels.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{
    ATTR_CANCELS_AUTO_ATTACK, ATTR_EX_CHANNELED, ATTR_RANGED, SpellTemplate,
};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::combat::{self, AutoRepeat};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::spells;

/// Instants that start the swing over (interrupt flag 0x8), one exempt
/// (`SPELL_ATTR_EX2_DO_NOT_RESET_COMBAT_TIMERS`), one without the flag; a
/// two-second cast and a channel that start it over.
const RESET: u32 = 990_920;
const EXEMPT: u32 = 990_921;
const NO_FLAG: u32 = 990_922;
const SLOW_RESET: u32 = 990_923;
const CHANNEL_RESET: u32 = 990_924;
/// Spells that cancel auto attack: an instant, a ranged one, a slow one; a
/// spell whose auras damage breaks, which doesn't.
const STOP: u32 = 990_925;
const RANGED_STOP: u32 = 990_926;
const SLOW_STOP: u32 = 990_927;
const FRAGILE: u32 = 990_928;

fn data(d: &mut GameData) {
    let with = |id: u32, f: &dyn Fn(&mut SpellTemplate)| {
        let mut s = bare(id, 0);
        f(&mut s);
        s
    };
    let spells: Vec<SpellTemplate> = vec![
        with(RESET, &|s| s.interrupt_flags = 0x8),
        with(EXEMPT, &|s| {
            s.interrupt_flags = 0x8;
            s.attributes[2] |= 0x2_0000;
        }),
        with(NO_FLAG, &|s| s.interrupt_flags = 0x7),
        with(SLOW_RESET, &|s| {
            s.interrupt_flags = 0x8;
            s.casting_time_index = CAST_2S;
        }),
        with(CHANNEL_RESET, &|s| {
            s.interrupt_flags = 0x8;
            s.attributes[1] |= ATTR_EX_CHANNELED;
            s.duration_index = LASTS_5S;
        }),
        with(STOP, &|s| s.attributes[0] |= ATTR_CANCELS_AUTO_ATTACK),
        with(RANGED_STOP, &|s| {
            s.attributes[0] |= ATTR_CANCELS_AUTO_ATTACK | ATTR_RANGED;
        }),
        with(SLOW_STOP, &|s| {
            s.attributes[0] |= ATTR_CANCELS_AUTO_ATTACK;
            s.casting_time_index = CAST_2S;
        }),
        with(FRAGILE, &|s| s.aura_interrupt_flags = 0x2),
    ];
    insert(d, spells);
}

/// `g`'s swing timers, as offsets from now.
fn timers(m: &Map, g: Guid) -> (Option<u64>, Option<u64>) {
    let now = m.now;
    let off = |t: Option<std::time::Instant>| {
        t.map(|t| t.saturating_duration_since(now).as_millis() as u64)
    };
    m.combat(g)
        .map_or((None, None), |c| (off(c.next_swing), off(c.next_offhand)))
}

/// `g` swinging at `at`, its timers due in half a second and 0.6; a
/// player's weapons 1.8 and 2.6 seconds, dual wielding or not.
fn swinging_with(m: &mut Map, g: Guid, at: Guid, dual: bool) {
    let now = m.now;
    combat::start_attack(m, g, at);
    if let Some(p) = m.players.get_mut(&g) {
        p.fields.set_u32(UNIT_FIELD_BASEATTACKTIME, 1_800);
        p.fields.set_u32(UNIT_FIELD_BASEATTACKTIME + 1, 2_600);
        p.derived.dual_wield = dual;
    }
    let c = m.combat_mut(g).unwrap();
    c.attacking = true;
    c.next_swing = Some(now + Duration::from_millis(500));
    c.next_offhand = Some(now + Duration::from_millis(600));
}

fn swinging(m: &mut Map, g: Guid, at: Guid) {
    swinging_with(m, g, at, true);
}

/// A cast that interrupts the swing starts both hands over at their
/// weapons' periods as it goes off, unless it is triggered, exempt or
/// lacks the flag; the off hand only when it can swing; a cast's begun
/// and interrupted leave the timers, its completion starts them over; a
/// channel starts them over at its start, not again at its end (Thistle
/// Tea's `CastingCombat.launch/3` and the spell lifecycle; mangos's
/// `IsMeleeAttackResetSpell` in `_handle_immediate_phase`).
#[tokio::test]
async fn a_cast_starts_the_swing_over_as_it_goes_off() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6CA", "Swinger", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(
                m,
                a,
                60,
                &[RESET, EXEMPT, NO_FLAG, SLOW_RESET, CHANNEL_RESET],
            );
            let kobold = kit::dummy(m, a, 60, 2.0);
            swinging(m, a, kobold);
            cast_fresh(m, a, RESET, at(kobold));
            let reset = timers(m, a);
            let mut kept = Vec::new();
            for spell in [EXEMPT, NO_FLAG] {
                swinging(m, a, kobold);
                cast_fresh(m, a, spell, at(kobold));
                kept.push(timers(m, a));
            }
            swinging(m, a, kobold);
            spells::cast_triggered(m, a, RESET, Some(kobold));
            kept.push(timers(m, a));
            swinging_with(m, a, kobold, false);
            cast_fresh(m, a, RESET, at(kobold));
            let one_hand = timers(m, a);
            swinging(m, a, kobold);
            cast_fresh(m, a, SLOW_RESET, at(kobold));
            let begun = timers(m, a);
            spells::interrupt(m, a);
            let interrupted = timers(m, a);
            cast_fresh(m, a, SLOW_RESET, at(kobold));
            m.now += Duration::from_millis(2_000);
            let now = m.now;
            m.combat_mut(a).unwrap().next_swing = Some(now + Duration::from_millis(100));
            spells::update(m);
            let completed = timers(m, a);
            swinging(m, a, kobold);
            cast_fresh(m, a, CHANNEL_RESET, at(a));
            let channelled = timers(m, a);
            pass(m, 4_000);
            let now = m.now;
            m.combat_mut(a).unwrap().next_swing = Some(now + Duration::from_millis(100));
            pass(m, 1_100);
            let after_channel = timers(m, a);
            (
                reset,
                kept,
                one_hand,
                begun,
                interrupted,
                completed,
                channelled,
                after_channel,
            )
        })
        .await
        .unwrap();
    let (reset, kept, one_hand, begun, interrupted, completed, channelled, after) = out;
    let fresh = (Some(1_800), Some(2_600));
    let left = (Some(500), Some(600));
    assert_eq!(reset, fresh, "both hands started over");
    assert_eq!(kept, [left, left, left], "exempt, no flag, triggered");
    assert_eq!(one_hand, (Some(1_800), Some(600)), "no off hand to swing");
    assert_eq!((begun, interrupted), (left, left), "begun, interrupted");
    assert_eq!(completed.0, Some(1_800), "completed");
    assert_eq!(channelled, fresh, "a channel at its start");
    assert_eq!(
        after.0,
        Some(0),
        "not again at its end: the old timer ran out"
    );
}

/// A spell that cancels auto attack ends its caster's swinging and the
/// strike queued for it, not the fight; its client is told the attack
/// stopped. A ranged one ends Auto Shot too. Triggered, it does as much;
/// it needn't hit anyone. A creature casting one stops swinging but keeps
/// its fight and its threat. A spell whose auras damage breaks doesn't
/// stop attacks, and one cancelled before it goes off doesn't either
/// (Thistle Tea's `CastingCombat.finish`; mangos's `Spell::cast`, with
/// `SPELL_ATTR_CANCELS_AUTO_ATTACK_COMBAT`, triggered or not).
#[tokio::test]
async fn a_spell_that_cancels_auto_attack_stops_it_not_the_fight() {
    let realm = fast_realm(data);
    let (mut c, a, map) = kit::player(&realm, "P6CB", "Stopper", Race::Dwarf, Class::Hunter).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[STOP, RANGED_STOP, SLOW_STOP, FRAGILE]);
            let kobold = kit::dummy(m, a, 60, 2.0);
            let attacking = |m: &Map, g: Guid| m.combat(g).is_some_and(|c| c.attacking);
            let in_fight = |m: &Map, g: Guid| {
                m.unit_fields(g)
                    .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
            };
            swinging(m, a, kobold);
            m.combat_mut(a).unwrap().queued = Some(STOP);
            cast_fresh(m, a, STOP, Default::default());
            let stopped = (attacking(m, a), m.combat(a).unwrap().queued, in_fight(m, a));
            let now = m.now;
            let shooting = |m: &mut Map| {
                m.combat_mut(a).unwrap().auto_repeat = Some(AutoRepeat {
                    spell: 75,
                    target: kobold,
                    next_at: now,
                });
            };
            swinging(m, a, kobold);
            shooting(m);
            cast_fresh(m, a, STOP, Default::default());
            let melee_only = m.combat(a).unwrap().auto_repeat.is_some();
            swinging(m, a, kobold);
            cast_fresh(m, a, RANGED_STOP, Default::default());
            let ranged = (attacking(m, a), m.combat(a).unwrap().auto_repeat.is_some());
            swinging(m, a, kobold);
            spells::cast_triggered(m, a, STOP, None);
            let triggered = attacking(m, a);
            swinging(m, a, kobold);
            cast_fresh(m, a, FRAGILE, Default::default());
            let fragile = attacking(m, a);
            cast_fresh(m, a, SLOW_STOP, Default::default());
            pass(m, 1_000);
            spells::interrupt(m, a);
            let cancelled = attacking(m, a);
            // A creature in a fight with the hunter.
            swinging(m, kobold, a);
            m.creature_mut(kobold).unwrap().threat = vec![(a, 100.0)];
            spells::cast_triggered(m, kobold, STOP, None);
            let creature = (
                attacking(m, kobold),
                in_fight(m, kobold),
                m.creature(kobold).unwrap().threat.clone(),
            );
            (
                stopped, melee_only, ranged, triggered, fragile, cancelled, creature,
            )
        })
        .await
        .unwrap();
    let (stopped, melee_only, ranged, triggered, fragile, cancelled, creature) = out;
    assert_eq!(
        stopped,
        (false, None, true),
        "swing and strike end, the fight goes on"
    );
    assert!(melee_only, "Auto Shot goes on after a melee spell");
    assert_eq!(ranged, (false, false), "a ranged one ends it too");
    assert!(!triggered, "triggered");
    assert!(fragile, "damage-breakable auras alone");
    assert!(cancelled, "cancelled");
    assert_eq!(creature, (false, true, vec![(a, 100.0)]));
    let stop = c
        .gets(Duration::from_millis(500), |m| match m {
            ServerOpcodeMessage::SMSG_ATTACKSTOP(s) if s.player == a => Some(s.enemy),
            _ => None,
        })
        .await;
    assert!(stop.is_some(), "told");
    let cancel = c
        .gets(Duration::from_millis(500), |m| match m {
            ServerOpcodeMessage::SMSG_CANCEL_AUTO_REPEAT => Some(()),
            _ => None,
        })
        .await;
    assert!(cancel.is_some(), "told of Auto Shot");
}
