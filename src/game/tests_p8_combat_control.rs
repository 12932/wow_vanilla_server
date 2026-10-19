//! Round 8's combat tests, control (Thistle Tea's `combat_control_test.exs`):
//! pacify holds a fighter's blows without ending its fight, a refreshed
//! control lasts to its new end, a state immunity keeps pacify and silence
//! off, and a channel cut short by silence lets go of all it held.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, UNIT_FLAG_PACIFIED};
use crate::game::map::{Map, combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE, STRIKE};

/// Pacify and silence at once, for a second; a state immunity to it; a
/// five-second channel of a periodic harm at an enemy, kept off by
/// silence (its prevention type).
const BOTH: u32 = 96_300;
const IMMUNITY: u32 = 96_301;
const DRAIN: u32 = 96_302;
const PACIFY: u32 = 96_303;
const ONE_SECOND: u32 = 960;
const FIVE_SECONDS: u32 = 961;
const UNIT_FLAG_SILENCED: u32 = 0x2000;

fn data() -> GameData {
    let mut d = p3::data();
    for (i, ms) in [(ONE_SECOND, 1_000), (FIVE_SECONDS, 5_000)] {
        d.spells.durations.insert(
            i,
            SpellDuration {
                base: ms,
                per_level: 0,
                max: ms,
            },
        );
    }
    let control = |id: u32, aura: u32| {
        let mut s = SpellTemplate {
            id,
            name: format!("Control {id}"),
            range_index: 1,
            equipped_item_class: -1,
            dmg_class: 1,
            duration_index: ONE_SECOND,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        };
        s
    };
    let mut immunity = control(IMMUNITY, auras::AURA_STATE_IMMUNITY);
    immunity.effects[0].misc_value = auras::AURA_MOD_PACIFY_SILENCE as i32;
    immunity.effects[0].target_a = 1;
    immunity.duration_index = FIVE_SECONDS;
    let mut drain = control(DRAIN, auras::AURA_PERIODIC_DAMAGE);
    drain.attributes[1] |= ATTR_EX_CHANNELED;
    drain.duration_index = FIVE_SECONDS;
    drain.prevention_type = 1;
    drain.effects[0].amplitude = 1_000;
    drain.effects[0].base_points = 9;
    for s in [
        control(BOTH, auras::AURA_MOD_PACIFY_SILENCE),
        control(PACIFY, auras::AURA_MOD_PACIFY),
        immunity,
        drain,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn flags(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS))
}

/// `spell`'s auras from `caster` on `target`, as a cast puts them on.
fn put_on(m: &mut Map, caster: Guid, target: Guid, spell: u32) {
    let data = m.realm.data.clone();
    let s = data.spells.get(spell).unwrap().clone();
    let _ = auras::apply_spell(m, caster, target, &s, &[(0, 0)], auras::Context::default());
}

/// Thistle Tea's "pacify pauses both hands and queued abilities until
/// removed", as mangos's `AttackerStateUpdate` has it (a pacified unit's
/// swing does nothing, its fight and its queued strike left as they are):
/// under pacify, or pacify and silence, a player swinging at a creature
/// lands no blow, is still swinging, keeps her queued strike and the extra
/// attacks she is owed; once it is gone, they all land.
#[tokio::test]
async fn pacify_holds_the_blows_the_queued_strike_and_the_extra_attacks_until_it_goes() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            m.players.get_mut(&ga).unwrap().spells.push(STRIKE);
            let mut out = Vec::new();
            for control in [PACIFY, BOTH] {
                let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
                let now = m.now;
                if let Some(c) = m.combat_mut(ga) {
                    c.victim = brute;
                    c.attacking = true;
                    c.next_swing = Some(now);
                    c.next_offhand = None;
                    c.extra_attacks = 2;
                }
                combat::queue_next_swing(m, ga, STRIKE);
                put_on(m, ga, ga, control);
                let before = health(m, brute);
                for _ in 0..10 {
                    m.now += Duration::from_millis(200);
                    combat::try_swing(m, ga);
                }
                let c = m.combat(ga).unwrap();
                let held = (
                    before - health(m, brute),
                    c.attacking && c.victim == brute,
                    c.queued,
                    c.extra_attacks,
                );
                auras::remove_spell(m, ga, control, None);
                m.now += Duration::from_millis(50);
                combat::FORCED_OUTCOME.with(|f| f.set(Some(combat::Outcome::Hit)));
                combat::try_swing(m, ga);
                combat::FORCED_OUTCOME.with(|f| f.set(None));
                let c = m.combat(ga).unwrap();
                let freed = (before > health(m, brute), c.queued, c.extra_attacks);
                combat::stop_attack(m, ga);
                out.push((held, freed));
            }
            out
        })
        .await
        .unwrap();
    for (held, freed) in r {
        assert_eq!(
            held,
            (0, true, Some(STRIKE), 2),
            "(damage, swinging, queued, extra attacks) while pacified"
        );
        assert_eq!(freed, (true, None, 0), "and all of them once it goes");
    }
}

/// Thistle Tea's "refresh keeps control until the refreshed deadline": a
/// pacify and silence of a second put on again half a second in holds on
/// until a second after the second, and then its flags go.
#[tokio::test]
async fn a_refreshed_control_lasts_to_its_new_end() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
            let both = UNIT_FLAG_PACIFIED | UNIT_FLAG_SILENCED;
            put_on(m, ga, brute, BOTH);
            m.now += Duration::from_millis(500);
            auras::update(m);
            put_on(m, ga, brute, BOTH);
            m.now += Duration::from_millis(500);
            auras::update(m);
            let at_first_end = flags(m, brute) & both;
            m.now += Duration::from_millis(510);
            auras::update(m);
            (at_first_end, flags(m, brute) & both, both)
        })
        .await
        .unwrap();
    assert_eq!(r.0, r.2, "held past the first end");
    assert_eq!(r.1, 0, "and gone at the new one");
}

/// Thistle Tea's "state immunity blocks the combined effect with immune
/// feedback", as mangos's `IsImmuneToSpellEffect` has it: a creature
/// immune to pacify and silence (`SPELL_AURA_STATE_IMMUNITY` of its kind)
/// is neither, and its caster is told it was immune.
#[tokio::test]
async fn a_state_immunity_keeps_pacify_and_silence_off_and_says_so() {
    let (_realm, mut a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
            put_on(m, brute, brute, IMMUNITY);
            crate::game::map::visibility::refresh_dirty(m);
            brute
        })
        .await
        .unwrap();
    a.sees(brute).await;
    let pacified = map
        .query(move |m| {
            crate::game::map::spells::cast_triggered(m, ga, BOTH, Some(brute));
            flags(m, brute) & (UNIT_FLAG_PACIFIED | UNIT_FLAG_SILENCED)
        })
        .await
        .unwrap();
    assert_eq!(pacified, 0, "neither pacified nor silenced");
    assert_eq!(
        crate::game::tests_p3_spells_kit::missed(&mut a, BOTH).await,
        7,
        "immune"
    );
}

/// Thistle Tea's "interrupted channels release objects, remote auras, and
/// client timers", as mangos's `Spell::cancel` has it: a channel cut short
/// by a silence takes its aura off its target, clears the channel's fields
/// and tells its caster's client the channel is over.
#[tokio::test]
async fn a_channel_cut_short_by_silence_lets_go_of_its_target_and_its_bar() {
    let (_realm, mut a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            p3::level(m, ga, 60);
            m.players.get_mut(&ga).unwrap().spells.push(DRAIN);
            p3::spawn(m, ga, BRUTE, 60, 3.0)
        })
        .await
        .unwrap();
    a.sees(brute).await;
    let r = map
        .query(move |m| {
            crate::game::map::spells::cast(
                m,
                ga,
                DRAIN,
                crate::game::map::spells::Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
                None,
            );
            let channel = |m: &Map| {
                let f = m.unit_fields(ga).unwrap();
                (
                    f.get_u32(UNIT_CHANNEL_SPELL),
                    f.get_guid(UNIT_FIELD_CHANNEL_OBJECT),
                    auras::auras(m, brute).is_some_and(|x| x.has_spell(DRAIN)),
                )
            };
            let during = channel(m);
            put_on(m, brute, ga, BOTH);
            let casting = m.caster(ga).and_then(|c| c.casting.as_ref()).is_some();
            (during, casting, channel(m))
        })
        .await
        .unwrap();
    assert_eq!(r.0, (DRAIN, brute, true), "channelling at the brute");
    assert!(!r.1, "cut short");
    assert_eq!(
        r.2,
        (0, Guid::zero(), false),
        "its fields and its aura gone"
    );
    let ended = a
        .wait_for("the channel's end", |m| match m {
            ServerOpcodeMessage::MSG_CHANNEL_UPDATE(u) => Some(u.time),
            _ => None,
        })
        .await;
    assert_eq!(ended, 0);
}
