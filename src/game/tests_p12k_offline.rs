//! Round 12, stage 12k: cooldowns and harmful auras run down while their
//! player is logged out, as mangos-classic keeps them: a cooldown's end is
//! saved (`_SaveSpellCooldowns`, `_LoadSpellCooldowns`), and a harmful
//! aura, or one that says so (`SPELL_ATTR_EX4_AURA_EXPIRES_OFFLINE`), loses
//! the time away at login (`_LoadAuras`' `timediff`); a helpful one waits.
//! A Hearthstone's hour was an hour again at each login, as was Deserter.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::Client;
use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_DEBUFF, SpellDuration};
use crate::game::guid;
use crate::game::map::auras;

/// A Hearthstone's spell (an hour's cooldown), a debuff of 15 minutes
/// (Deserter's), one of 5, a buff of 15, and a buff of 15 that runs out
/// offline.
const HEARTH: u32 = 60_970;
const DESERTER: u32 = 60_971;
const SHORT_DEBUFF: u32 = 60_972;
const BUFF: u32 = 60_973;
const OFFLINE_BUFF: u32 = 60_974;
const LASTS_15M: u32 = 960;
const LASTS_5M: u32 = 961;
const MINUTE: u64 = 60_000;

fn data(d: &mut GameData) {
    for (i, ms) in [
        (LASTS_15M, 15 * MINUTE as i32),
        (LASTS_5M, 5 * MINUTE as i32),
    ] {
        d.spells.durations.insert(
            i,
            SpellDuration {
                base: ms,
                per_level: 0,
                max: ms,
            },
        );
    }
    let mut hearth = bare(HEARTH, 0);
    hearth.recovery_time = 60 * MINUTE as u32;
    let made = |id: u32, duration: u32, debuff: bool, offline: bool| {
        let mut s = bare(id, 0);
        s.duration_index = duration;
        if debuff {
            s.attributes[0] |= ATTR_DEBUFF;
        }
        if offline {
            s.attributes[4] |= 0x4;
        }
        effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        s
    };
    insert(
        d,
        [
            hearth,
            made(DESERTER, LASTS_15M, true, false),
            made(SHORT_DEBUFF, LASTS_5M, true, false),
            made(BUFF, LASTS_15M, false, false),
            made(OFFLINE_BUFF, LASTS_15M, false, true),
        ],
    );
}

/// What is left of `spell`'s aura on `g`, in whole minutes rounded up.
fn minutes_left(m: &crate::game::map::Map, g: Guid, spell: u32) -> Option<u64> {
    kit::holder(m, g, spell).map(|h| {
        h.expires
            .map_or(u64::MAX, |t| {
                t.saturating_duration_since(m.now).as_millis() as u64
            })
            .div_ceil(MINUTE)
    })
}

/// Ten minutes logged out: the Hearthstone's 59 minutes are 49, the
/// 15-minute debuff 5 minutes, the 5-minute one gone; the buff still has
/// its 15, but the one that runs out offline 5.
#[tokio::test]
async fn cooldowns_and_debuffs_run_down_while_logged_out() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P12KOFFLINE", "Away", Class::Warrior).await;
    map.query(move |m| {
        kit::ready(m, a, 10, &[]);
        let now = m.now;
        let p = m.players.get_mut(&a).unwrap();
        p.caster
            .cooldowns
            .insert(HEARTH, now + Duration::from_millis(59 * MINUTE));
        for s in [DESERTER, SHORT_DEBUFF, BUFF, OFFLINE_BUFF] {
            kit::apply(m, a, a, s);
        }
    })
    .await
    .unwrap();
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    // Ten minutes ago, as the store has it.
    let mut row = realm.store.load(guid::counter(a)).await.expect("saved");
    assert!(row.state.saved_at_ms > 0, "the save's time is kept");
    row.state.saved_at_ms -= 10 * MINUTE as i64;
    row.logout_time -= 600;
    realm.store.save(row);
    realm.store.flush().await;
    c.clear();
    c.login(a).await;
    let cooldowns = c
        .wait_for("SMSG_INITIAL_SPELLS", |m| match m {
            ServerOpcodeMessage::SMSG_INITIAL_SPELLS(s) => Some(s.cooldowns.clone()),
            _ => None,
        })
        .await;
    let hearth = cooldowns
        .iter()
        .find(|c| u32::from(c.spell_id) == HEARTH)
        .map(|c| c.cooldown.as_millis() as u64)
        .expect("the Hearthstone's cooldown");
    assert!(
        (48 * MINUTE..=49 * MINUTE).contains(&hearth),
        "49 minutes left, not 59: {hearth} ms"
    );
    let left = map
        .query(move |m| [DESERTER, SHORT_DEBUFF, BUFF, OFFLINE_BUFF].map(|s| minutes_left(m, a, s)))
        .await
        .unwrap();
    assert_eq!(left, [Some(5), None, Some(15), Some(5)]);
}

/// A clock put back lengthens nothing: saved "in the future", a cooldown
/// keeps what it had.
#[tokio::test]
async fn a_clock_put_back_lengthens_no_cooldown() {
    let realm = fast_realm(data);
    let mut c = Client::connect(&realm, "P12KCLOCK");
    let a = c.enter("Clock", Race::Human, Class::Warrior).await;
    let map = world(&realm);
    map.query(move |m| {
        let now = m.now;
        let p = m.players.get_mut(&a).unwrap();
        p.caster
            .cooldowns
            .insert(HEARTH, now + Duration::from_millis(30 * MINUTE));
    })
    .await
    .unwrap();
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let mut row = realm.store.load(guid::counter(a)).await.expect("saved");
    row.state.saved_at_ms += 60 * MINUTE as i64;
    realm.store.save(row);
    realm.store.flush().await;
    c.clear();
    c.login(a).await;
    let cooldowns = c
        .wait_for("SMSG_INITIAL_SPELLS", |m| match m {
            ServerOpcodeMessage::SMSG_INITIAL_SPELLS(s) => Some(s.cooldowns.clone()),
            _ => None,
        })
        .await;
    let hearth = cooldowns
        .iter()
        .find(|c| u32::from(c.spell_id) == HEARTH)
        .map(|c| c.cooldown.as_millis() as u64)
        .expect("the cooldown");
    assert!((29 * MINUTE..=30 * MINUTE).contains(&hearth), "{hearth} ms");
}
