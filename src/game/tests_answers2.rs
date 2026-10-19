//! The user's second batch of answers (2026-10-19), each as it is now: a
//! second login takes the character over (#35), fears break by chance
//! (#43), an interrupt locks out a silence-immune creature's school (#42),
//! attack-speed slows multiply (#30), cancelling Stealth takes Vanish's too
//! (#41), the quest log's slots are the group's (#47), the grey "!" shows
//! to seven levels short and is a setting (#21), locked doors and buttons
//! want their key by hand (#7); and a talent's trained ranks want the
//! talent at a trainer. Fade's threat (#46) is in `tests_p3_combat2`.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::{LASTS_15S, aura, bare, damage, effect, fast_realm, human, insert};
use super::tests_p3_spells_kit::{at, casting, dummy, ready};
use crate::game::Realm;
use crate::game::data::npc::Quest;
use crate::game::data::spells::{SkillAbility, SpellTemplate, TalentInfo};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client, WAIT};
use crate::game::map::{Map, MapHandle, auras, objects, quests, spells};
use crate::game::store::Logged;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map")
}

/// The map `g` is on.
fn map_of(realm: &Arc<Realm>, g: Guid) -> MapHandle {
    realm.registry.online(g).expect("in the world").map()
}

/// Wait for what `c` has sent so far to have been handled: a query of the
/// map answered after it.
async fn handled(c: &mut Client) {
    c.send(CMSG_QUESTGIVER_STATUS_QUERY {
        guid: guid::unit(1, 1),
    })
    .await;
    c.wait_for("SMSG_QUESTGIVER_STATUS", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(_)).then_some(())
    })
    .await;
}

fn npc(entry: u32, flags: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Npc {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 10,
        level_max: 10,
        faction: 35,
        unit_class: 1,
        health_max: 1000,
        npc_flags: flags,
        ..Default::default()
    }
}

fn beside(guid: u32, entry: u32, dx: f32) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    }
}

// ---- #30: attack-speed slows multiply ------------------------------------

const SLOW_20: u32 = 993_000;
const SLOW_10: u32 = 993_001;

fn slow_kit(d: &mut GameData) {
    // Lone negative melee hastes, the kind Thistle Tea keeps one of.
    let slow = |id: u32, amount: i32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_15S;
        effect(&mut s, 0, aura(auras::AURA_MOD_HASTE, 0, amount, 6));
        s
    };
    insert(d, [slow(SLOW_20, -20), slow(SLOW_10, -10)]);
}

/// Two attack-speed slows from two casters both stay and multiply on the
/// swing, as mangos-classic's `ApplyAttackTimePercentMod` has them (the
/// user's #30): 2,000 ms × 1.2 × 1.1. Before, the weaker was passed over
/// and only the stronger counted (Thistle Tea's, after vmangos).
#[tokio::test]
async fn attack_speed_slows_from_two_casters_stand_together_and_multiply() {
    let realm = fast_realm(slow_kit);
    let (_c, w, map) = human(&realm, "SLOWS", "Slowed", Class::Warrior).await;
    let got = on(&map, move |m| {
        ready(m, w, 60, &[]);
        let (a, b) = (dummy(m, w, 60, 3.0), dummy(m, w, 60, 4.0));
        let period = |m: &Map| m.players[&w].fields.get_u32(UNIT_FIELD_BASEATTACKTIME);
        let bare_period = period(m);
        for (by, id) in [(a, SLOW_20), (b, SLOW_10)] {
            let s = m.realm.data.spells.get(id).cloned().unwrap();
            let points = s.effects[0].base_points + 1;
            let _ = auras::apply_spell(m, by, w, &s, &[(0, points)], auras::Context::default());
        }
        crate::game::map::derive::unit(m, w);
        let on = [SLOW_20, SLOW_10].map(|id| auras::auras(m, w).is_some_and(|x| x.has_spell(id)));
        (bare_period, on, period(m))
    })
    .await;
    assert_eq!(got.0, 2000);
    assert_eq!(got.1, [true, true], "both slows on");
    assert_eq!(got.2, 2640, "and multiplied");
}

// ---- #41: cancelling Stealth with Vanish on ------------------------------

const STEALTH: u32 = 994_100;
const VANISHED: u32 = 994_101;

fn stealth_kit(d: &mut GameData) {
    // Stealth: the form and the aura; Vanish's own: the aura alone, for a
    // while.
    let mut stealth = bare(STEALTH, 0);
    stealth.duration_index = super::tests_p3_cast_kit::LASTS_LONG;
    effect(
        &mut stealth,
        0,
        aura(
            auras::AURA_MOD_SHAPESHIFT,
            i32::from(auras::FORM_STEALTH),
            0,
            1,
        ),
    );
    effect(&mut stealth, 1, aura(auras::AURA_MOD_STEALTH, 0, 0, 1));
    let mut vanished = bare(VANISHED, 0);
    vanished.duration_index = LASTS_15S;
    effect(&mut vanished, 0, aura(auras::AURA_MOD_STEALTH, 0, 0, 1));
    insert(d, [stealth, vanished]);
}

/// Cancelling Stealth while Vanish's stealth is on takes both, so the
/// rogue is seen (the user's #41, as Thistle Tea's `cancel_spell` takes
/// every stealth holder); before, Vanish's kept them hidden (mangos's
/// `CMSG_CANCEL_AURA` takes the spell named alone). A buff of another kind
/// cancelled takes nothing more.
#[tokio::test]
async fn cancelling_stealth_with_vanish_on_takes_both() {
    let realm = fast_realm(stealth_kit);
    let (mut c, rogue, map) = human(&realm, "VANISH", "Vanisher", Class::Rogue).await;
    on(&map, move |m| {
        for id in [STEALTH, VANISHED] {
            let s = m.realm.data.spells.get(id).cloned().unwrap();
            let effects: Vec<(usize, i32)> = (0..3)
                .filter(|i| s.effects[*i].effect == 6)
                .map(|i| (i, 0))
                .collect();
            auras::apply_spell(m, rogue, rogue, &s, &effects, auras::Context::default())
                .expect("the stealth takes");
        }
    })
    .await;
    let both = on(&map, move |m| {
        [STEALTH, VANISHED].map(|id| auras::auras(m, rogue).is_some_and(|a| a.has_spell(id)))
    })
    .await;
    assert_eq!(both, [true, true]);
    c.send(CMSG_CANCEL_AURA { id: STEALTH }).await;
    handled(&mut c).await;
    let after = on(&map, move |m| {
        (
            [STEALTH, VANISHED].map(|id| auras::auras(m, rogue).is_some_and(|a| a.has_spell(id))),
            auras::auras(m, rogue).is_some_and(|a| a.has(auras::AURA_MOD_STEALTH)),
            auras::form(m, rogue),
        )
    })
    .await;
    assert_eq!(
        after,
        ([false, false], false, 0),
        "both gone, the rogue seen"
    );
}

// ---- #21: the grey "!" -------------------------------------------------

/// Givers of quests 7, 8 and 29 levels above a new character's.
const SEVEN_UP: u32 = 9210;
const EIGHT_UP: u32 = 9211;
const FAR_UP: u32 = 9212;

fn grey_data() -> GameData {
    let mut d = GameData::empty();
    for (giver, min_level) in [(SEVEN_UP, 8), (EIGHT_UP, 9), (FAR_UP, 30)] {
        let id = giver + 100;
        d.npc.quests.insert(
            id,
            Quest {
                id,
                title: format!("Quest {id}"),
                min_level,
                level: i32::from(min_level),
                ..Default::default()
            },
        );
        d.npc.creature_starts.insert(giver, vec![id]);
    }
    d
}

async fn giver_status(c: &mut Client, giver: u32) -> QuestGiverStatus {
    let g = guid::unit(giver, 1);
    c.send(CMSG_QUESTGIVER_STATUS_QUERY { guid: g }).await;
    c.wait_for("SMSG_QUESTGIVER_STATUS", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTGIVER_STATUS(s) if s.guid == g => Some(s.status),
        _ => None,
    })
    .await
}

/// A giver shows a level-1 character the grey "!" for a quest seven levels
/// up, and nothing for one eight up, as mangos's `Quests.HighLevelHideDiff`
/// does by default (the user's #21). Before, only four levels up showed it.
#[tokio::test]
async fn the_grey_exclamation_shows_to_seven_levels_short() {
    let realm = harness::realm_with(grey_data());
    let mut a = Client::connect(&realm, "GREY");
    a.enter("Greyone", Race::Human, Class::Warrior).await;
    assert_eq!(
        giver_status(&mut a, SEVEN_UP).await,
        QuestGiverStatus::Unavailable
    );
    assert_eq!(giver_status(&mut a, EIGHT_UP).await, QuestGiverStatus::None);
    assert_eq!(giver_status(&mut a, FAR_UP).await, QuestGiverStatus::None);
}

/// `[quests] high_level_hide_diff` is the gap, live: -1 shows the grey "!"
/// at any gap, 0 only for quests of the player's own level.
#[tokio::test]
async fn the_grey_exclamations_gap_is_a_setting() {
    let realm = harness::realm_with(grey_data());
    let mut a = Client::connect(&realm, "GREYS");
    let g = a.enter("Greytwo", Race::Human, Class::Warrior).await;
    let map = map_of(&realm, g);
    let shown = on(&map, move |m| {
        let mut shown = Vec::new();
        for gap in [-1, 0, 8] {
            let mut cfg = (*m.cfg).clone();
            cfg.quests.high_level_hide_diff = gap;
            m.cfg = Arc::new(cfg);
            shown.push(
                [SEVEN_UP, EIGHT_UP, FAR_UP]
                    .map(|giver| quests::status(m, g, guid::unit(giver, 1))),
            );
        }
        shown
    })
    .await;
    use QuestGiverStatus::{None as Nothing, Unavailable as Grey};
    assert_eq!(shown[0], [Grey, Grey, Grey], "any gap");
    assert_eq!(shown[1], [Nothing, Nothing, Nothing], "none");
    assert_eq!(shown[2], [Grey, Grey, Nothing], "eight");
}
