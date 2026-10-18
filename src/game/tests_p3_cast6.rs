//! Phase 3's interrupts and cooldowns, as Thistle Tea's
//! `spell_interrupt_test`, `school_lockout_test`, `cooldowns_test` and
//! `global_cooldown_test` have them, on made-up spells and the fast suite.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{at, casting, dummy, fresh, has, health, pass, ready, set_health};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells;

const CHANNEL: u32 = 990_800;
const KICK: u32 = 990_801;
const QUICK_KICK: u32 = 990_802;
const STONE: u32 = 990_803;
const FIRE: u32 = 990_804;
const FIRE_TOO: u32 = 990_805;
const FROST: u32 = 990_806;
const STEALTH: u32 = 990_807;
const STEALTH_TOO: u32 = 990_808;
const GCD: u32 = 990_809;
const GCD_CHANNEL: u32 = 990_810;

fn kit(d: &mut GameData) {
    // A healing channel on oneself, a silence could stop.
    let mut channel = bare(CHANNEL, 2);
    channel.attributes[1] |= ATTR_EX_CHANNELED;
    channel.interrupt_flags = 0x8;
    channel.prevention_type = 1;
    channel.duration_index = LASTS_15S;
    effect(
        &mut channel,
        0,
        SpellEffect {
            amplitude: 2_000,
            ..aura(AURA_OBS_MOD_HEALTH, 0, 7, 1)
        },
    );
    // A kick: interrupt, a school locked five seconds; and one of no time.
    let mut kick = bare(KICK, 0);
    kick.dmg_class = 2;
    kick.duration_index = LASTS_5S;
    effect(
        &mut kick,
        0,
        SpellEffect {
            effect: 68,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut quick = kick.clone();
    quick.id = QUICK_KICK;
    quick.duration_index = 0;
    // A cast nothing interrupts (a Hearthstone's kind).
    let mut stone = bare(STONE, 0);
    stone.casting_time_index = CAST_10S;
    effect(&mut stone, 0, aura(AURA_DUMMY, 0, 0, 1));
    // Fire spells, one with a long cooldown of its own; a frost spell.
    let fire = |id: u32| {
        let mut s = bare(id, 2);
        s.casting_time_index = CAST_3S;
        s.prevention_type = 1;
        s.interrupt_flags = 0x8;
        effect(&mut s, 0, damage(10));
        s
    };
    let mut fire_too = fire(FIRE_TOO);
    fire_too.recovery_time = 30_000;
    let mut frost = fire(FROST);
    frost.school = 4;
    // A stealth whose cooldown waits for it to go, and another of its
    // category.
    let mut stealth = bare(STEALTH, 0);
    stealth.attributes[0] |= 0x200_0000;
    stealth.category = 38;
    stealth.category_recovery_time = 10_000;
    stealth.duration_index = LASTS_LONG;
    effect(&mut stealth, 0, aura(AURA_DUMMY, 0, 0, 1));
    let mut stealth_too = stealth.clone();
    stealth_too.id = STEALTH_TOO;
    stealth_too.attributes[0] = 0;
    // The global cooldown's spells: one instant, one a channel cast first.
    let mut gcd = bare(GCD, 2);
    gcd.start_recovery_category = 133;
    gcd.start_recovery_time = 1_500;
    effect(&mut gcd, 0, aura(AURA_DUMMY, 0, 0, 1));
    gcd.duration_index = LASTS_5S;
    let mut gcd_channel = channel.clone();
    gcd_channel.id = GCD_CHANNEL;
    gcd_channel.casting_time_index = CAST_3S;
    gcd_channel.start_recovery_category = 133;
    gcd_channel.start_recovery_time = 1_500;
    insert(
        d,
        [
            channel,
            kick,
            quick,
            stone,
            fire(FIRE),
            fire_too,
            frost,
            stealth,
            stealth_too,
            gcd,
            gcd_channel,
        ],
    );
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).cloned().unwrap()
}

fn locked(m: &Map, g: wow_world_messages::Guid, school_mask: u32) -> bool {
    let now = m.now;
    m.caster(g).is_some_and(|c| {
        c.school_lockout
            .iter()
            .any(|(mask, until)| *until > now && mask & school_mask != 0)
    })
}

/// An interrupt: a channel stops, its aura and its ticks with it; the
/// school is locked for the interrupt's time, and longer locks hold over
/// shorter; one of no time locks nothing; a cast nothing interrupts goes
/// on; the dead aren't touched (Thistle Tea's `SpellInterrupt.apply`,
/// mangos's `EffectInterruptCast`).
#[tokio::test]
async fn an_interrupt_stops_a_channel_and_locks_its_school_for_its_time() {
    let realm = fast_realm(kit);
    let (_c, mage, map) = human(&realm, "P3IA", "Kicked", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[CHANNEL, STONE, FIRE, FIRE_TOO, FROST]);
            let k = dummy(m, mage, 60, 3.0);
            set_health(m, mage, 100);
            assert!(spells::cast(m, mage, CHANNEL, Default::default(), None));
            let channelling = (casting(m, mage), has(m, mage, CHANNEL));
            spells::cast(m, k, KICK, at(mage), None);
            let kicked = (
                casting(m, mage),
                has(m, mage, CHANNEL),
                m.unit_fields(mage).unwrap().get_u32(UNIT_CHANNEL_SPELL),
            );
            let before = health(m, mage);
            pass(m, 4_000);
            let ticked = health(m, mage) > before;
            let lock = (locked(m, mage, 0x4), locked(m, mage, 0x10));
            // A shorter lock doesn't shorten it.
            spells::cooldowns::lock_school(m, mage, FIRE, 100);
            pass(m, 500);
            let still = locked(m, mage, 0x4);
            pass(m, 1_000);
            let gone = locked(m, mage, 0x4);
            // No time, no lock.
            fresh(m, mage);
            spells::cast(m, mage, FROST, at(k), None);
            fresh(m, k);
            spells::cast(m, k, QUICK_KICK, at(mage), None);
            let quick = (casting(m, mage), locked(m, mage, 0x10));
            // The stone's cast goes on.
            fresh(m, mage);
            spells::cast(m, mage, STONE, Default::default(), None);
            fresh(m, k);
            spells::cast(m, k, KICK, at(mage), None);
            let stone = casting(m, mage);
            (channelling, kicked, ticked, lock, still, gone, quick, stone)
        })
        .await
        .unwrap();
    let (channelling, kicked, ticked, lock, still, gone, quick, stone) = out;
    assert_eq!(channelling, (Some(CHANNEL), true));
    assert_eq!(
        kicked,
        (None, false, 0),
        "the channel, its aura and its field gone"
    );
    assert!(!ticked, "no tick after it");
    assert_eq!(
        lock,
        (true, false),
        "the fire channel's school locked, not frost"
    );
    assert!(still, "a shorter lock after it doesn't shorten it");
    assert!(!gone, "it runs out at its five seconds");
    assert_eq!(quick, (None, false), "no time, no lock");
    assert_eq!(stone, Some(STONE), "a cast nothing interrupts goes on");
}

/// A cooldown that waits for its aura: it holds the spell and the others
/// of its category while the aura is on, and starts its time, told by
/// `SMSG_COOLDOWN_EVENT`, as the aura goes; the client hears the longer of
/// a spell's own and its category's time (Thistle Tea's
/// `activate_on_event` and `client_cooldown_ms`).
#[tokio::test]
async fn a_cooldown_on_its_event_waits_for_the_aura_and_the_client_hears_the_longer() {
    let realm = fast_realm(|d| {
        kit(d);
        let mut both = bare(990_811, 0);
        both.recovery_time = 8_000;
        both.category = 77;
        both.category_recovery_time = 60_000;
        effect(&mut both, 0, aura(AURA_DUMMY, 0, 0, 1));
        insert(d, [both]);
    });
    let (mut c, rogue, map) = human(&realm, "P3IB", "Cooler", Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, rogue, 60, &[STEALTH, STEALTH_TOO, 990_811]);
            spells::cast(m, rogue, STEALTH, Default::default(), None);
            fresh_gcd(m, rogue);
            let held = (
                spells::cast(m, rogue, STEALTH, Default::default(), None),
                spells::cast(m, rogue, STEALTH_TOO, Default::default(), None),
            );
            m.now += Duration::from_secs(5);
            remove_spell(m, rogue, STEALTH, None);
            let at = m.now;
            let ready_at = m
                .caster(rogue)
                .and_then(|c| c.cooldowns.get(&STEALTH).copied());
            fresh_gcd(m, rogue);
            spells::cast(m, rogue, 990_811, Default::default(), None);
            (held, ready_at.map(|t| t - at))
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        (false, false),
        "the spell held, and its category's other"
    );
    assert_eq!(
        out.1,
        Some(Duration::from_secs(10)),
        "ten seconds from the aura's end"
    );
    c.wait_for("SMSG_COOLDOWN_EVENT", |m| match m {
        ServerOpcodeMessage::SMSG_COOLDOWN_EVENT(e) if e.id == STEALTH => Some(()),
        _ => None,
    })
    .await;
    let ms = c
        .wait_for("SMSG_SPELL_COOLDOWN", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(s) => s
                .cooldowns
                .iter()
                .find(|x| x.id == 990_811)
                .map(|x| x.cooldown_time),
            _ => None,
        })
        .await;
    assert_eq!(ms, Duration::from_secs(60), "the longer of the two");
}

fn fresh_gcd(m: &mut Map, g: wow_world_messages::Guid) {
    if let Some(c) = m.caster_mut(g) {
        c.gcd_until = None;
    }
}

/// The global cooldown: once begun its time holds when the modifier that
/// shortened it goes; a channel interrupted while it is still being cast
/// gives it back (Thistle Tea's `trigger_gcd` and `cancel`).
#[tokio::test]
async fn the_global_cooldown_keeps_its_time_and_a_cancelled_cast_gives_it_back() {
    let realm = fast_realm(|d| {
        kit(d);
        let mut quicker = bare(990_812, 0);
        quicker.family_name = 3;
        quicker.duration_index = LASTS_LONG;
        effect(
            &mut quicker,
            0,
            SpellEffect {
                item_type: 1,
                ..aura(
                    AURA_ADD_FLAT_MODIFIER,
                    spells::mods::OP_GLOBAL_COOLDOWN as i32,
                    -500,
                    1,
                )
            },
        );
        for id in [GCD, GCD_CHANNEL] {
            if let Some(s) = d.spells.spells.get_mut(&id) {
                s.family_name = 3;
                s.family_flags = 1;
            }
        }
        insert(d, [quicker]);
    });
    let (_c, mage, map) = human(&realm, "P3IC", "Global", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[GCD, GCD_CHANNEL]);
            let t = template(m, 990_812);
            super::tests_p3_spells_kit::apply_template(m, mage, mage, &t);
            let at = m.now;
            spells::cast(m, mage, GCD, Default::default(), None);
            remove_spell(m, mage, 990_812, None);
            let held = m.caster(mage).and_then(|c| c.gcd_until).map(|t| t - at);
            fresh(m, mage);
            spells::cast(m, mage, GCD_CHANNEL, Default::default(), None);
            let channel_gcd = m.caster(mage).and_then(|c| c.gcd_until).is_some();
            m.now += Duration::from_millis(100);
            spells::cancel(m, mage);
            let after = m.caster(mage).and_then(|c| c.gcd_until);
            (held, channel_gcd, after)
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        Some(Duration::from_millis(1_000)),
        "a second, as it began"
    );
    assert!(out.1, "the channel's cast holds the global cooldown");
    assert_eq!(out.2, None, "and gives it back when cancelled");
}
