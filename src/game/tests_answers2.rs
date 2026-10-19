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

// ---- #43: fears break by chance -----------------------------------------

const FEAR: u32 = 994_300;
const FLAGLESS_FEAR: u32 = 994_301;
const BOLT: u32 = 994_302;
/// Every damage taken, as Psychic Scream's and Fear's `procFlags` (0xA22A8).
const TAKEN_DAMAGE: u32 = 0x000A_22A8;

fn fear_kit(d: &mut GameData) {
    let mut fear = bare(FEAR, 5);
    fear.duration_index = LASTS_15S;
    fear.proc_flags = TAKEN_DAMAGE;
    fear.proc_chance = 100;
    effect(&mut fear, 0, aura(auras::AURA_MOD_FEAR, 0, 0, 6));
    let mut flagless = fear.clone();
    flagless.id = FLAGLESS_FEAR;
    flagless.proc_flags = 0;
    let mut bolt = bare(BOLT, 2);
    effect(&mut bolt, 0, damage(100));
    insert(d, [fear, flagless, bolt]);
}

/// mangos-classic's chance that a blow breaks a fear: the damage over 25 a
/// level less 150 past level 8, else over 50 (`HandleRemoveByDamageChanceProc`).
#[test]
fn a_fears_break_chance_is_the_blow_over_its_bearers_level() {
    use crate::game::map::procs::fear_break_chance;
    assert_eq!(fear_break_chance(60, 135), 10.0, "1,350 at 60");
    assert_eq!(fear_break_chance(10, 50), 50.0, "100 at 10");
    assert_eq!(fear_break_chance(5, 25), 50.0, "50 below 9");
    assert!(fear_break_chance(60, 1350) >= 100.0, "sure");
    assert_eq!(fear_break_chance(60, 0), 0.0);
}

/// A fear without the damage-break flag breaks by chance at a blow its
/// spell's `procFlags` name, as mangos-classic has it (the user's #43; Thistle
/// Tea keeps it): 100 damage on a level-10 unit is a sure break. One whose
/// spell names no blow keeps on.
#[tokio::test]
async fn a_fear_breaks_by_chance_at_a_blow_its_spell_names() {
    let realm = fast_realm(fear_kit);
    let (_c, mage, map) = human(&realm, "FEARS", "Fearer", Class::Mage).await;
    let got = on(&map, move |m| {
        ready(m, mage, 60, &[BOLT]);
        let feared = |m: &mut Map, spell: u32| {
            let k = dummy(m, mage, 10, 5.0);
            let s = m.realm.data.spells.get(spell).cloned().unwrap();
            auras::apply_spell(m, mage, k, &s, &[(0, 0)], auras::Context::default())
                .expect("the fear takes");
            let on_before = auras::auras(m, k).is_some_and(|a| a.has_spell(spell));
            spells::cast_triggered(m, mage, BOLT, Some(k));
            let hurt = m.unit_fields(k).unwrap().get_u32(UNIT_FIELD_HEALTH) < 1_000_000;
            (
                on_before,
                hurt,
                auras::auras(m, k).is_some_and(|a| a.has_spell(spell)),
            )
        };
        (feared(m, FEAR), feared(m, FLAGLESS_FEAR))
    })
    .await;
    assert_eq!(got.0, (true, true, false), "broken by the blow");
    assert_eq!(got.1, (true, true, true), "no blow named, it stays");
}

// ---- #42: an interrupt on a creature immune to silence -------------------

const KICK: u32 = 994_200;
const FIRE: u32 = 994_201;

fn kick_kit(d: &mut GameData) {
    use super::tests_p3_cast_kit::{CAST_3S, LASTS_5S};
    // Immune to silence (mechanic 9, the mask's 256), as 467 of vmangos's
    // creatures are.
    d.creature_templates
        .get_mut(&super::tests_p3_cast_kit::KOBOLD)
        .unwrap()
        .mechanic_immune_mask = 1 << 8;
    let mut kick = bare(KICK, 0);
    kick.dmg_class = 2;
    kick.duration_index = LASTS_5S;
    effect(
        &mut kick,
        0,
        crate::game::data::spells::SpellEffect {
            effect: 68,
            target_a: 6,
            mechanic: 26,
            ..Default::default()
        },
    );
    let mut fire = bare(FIRE, 2);
    fire.casting_time_index = CAST_3S;
    fire.prevention_type = 1;
    fire.interrupt_flags = 0x8;
    effect(&mut fire, 0, damage(10));
    insert(d, [kick, fire]);
}

/// An interrupt on a creature immune to silence stops its cast and locks
/// its school out all the same, as mangos-classic's `EffectInterruptCast`
/// has it: the lockout isn't a silence (the user's #42; Thistle Tea, after
/// vmangos, spares the creature). wvs did so already; this keeps it.
#[tokio::test]
async fn an_interrupt_locks_out_the_school_of_a_creature_immune_to_silence() {
    let realm = fast_realm(kick_kit);
    let (_c, rogue, map) = human(&realm, "KICKS", "Kicker", Class::Rogue).await;
    let got = on(&map, move |m| {
        ready(m, rogue, 60, &[KICK]);
        let k = dummy(m, rogue, 60, 3.0);
        assert!(
            spells::cast(m, k, FIRE, at(rogue), None),
            "the creature casts"
        );
        let casting_before = casting(m, k);
        spells::cast_triggered(m, rogue, KICK, Some(k));
        let after = casting(m, k);
        let now = m.now;
        let locked = m.caster(k).is_some_and(|c| {
            c.school_lockout
                .iter()
                .any(|(mask, until)| *until > now && mask & 4 != 0)
        });
        let again = spells::cast(m, k, FIRE, at(rogue), None);
        let silenced = auras::auras(m, k).is_some_and(|a| a.has(auras::AURA_MOD_SILENCE));
        (casting_before, after, locked, again, silenced)
    })
    .await;
    assert_eq!(got.0, Some(FIRE));
    assert_eq!(got.1, None, "interrupted");
    assert!(got.2, "its fire locked out");
    assert!(!got.3, "and no fire cast meanwhile");
    assert!(!got.4, "by no silence");
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

// ---- #10: a talent's trained ranks at a trainer ---------------------------

const WARRIOR_TRAINER: u32 = 9600;
/// Mortal Strike's talent spell, and the rank a trainer teaches after it.
const MORTAL_STRIKE: u32 = 996_000;
const MORTAL_STRIKE_2: u32 = 996_001;
const MS_TALENT: u32 = 135;

fn trainer_data() -> GameData {
    let mut d = GameData::empty();
    let mut t = npc(WARRIOR_TRAINER, 0x10);
    t.trainer_type = 0;
    t.trainer_class = 1;
    d.creature_templates.insert(WARRIOR_TRAINER, t);
    d.creature_spawns
        .insert(0, vec![beside(960, WARRIOR_TRAINER, 2.0)]);
    let spell = |id: u32| SpellTemplate {
        id,
        name: format!("Mortal Strike {id}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    insert(&mut d, [spell(MORTAL_STRIKE), spell(MORTAL_STRIKE_2)]);
    d.spells.talents.insert(
        MS_TALENT,
        TalentInfo {
            id: MS_TALENT,
            ranks: [MORTAL_STRIKE, 0, 0, 0, 0],
            ..Default::default()
        },
    );
    // The skill line chains the talent's spell to the trained rank, as the
    // client's SkillLineAbility does; vmangos's spell_chain has no row.
    d.spells.skill_abilities.insert(
        MORTAL_STRIKE,
        vec![SkillAbility {
            skill: 26,
            spell: MORTAL_STRIKE,
            superseded_by: MORTAL_STRIKE_2,
            ..Default::default()
        }],
    );
    d.spells.index_talent_lines();
    d.npc.trainers.insert(
        WARRIOR_TRAINER,
        vec![crate::game::data::npc::TrainerSpell {
            spell: MORTAL_STRIKE_2,
            cost: 0,
            required_level: 1,
            ..Default::default()
        }],
    );
    d
}

async fn trainer_list(a: &mut Client, trainer: Guid) -> Vec<TrainerSpell> {
    a.clear();
    a.send(CMSG_TRAINER_LIST { guid: trainer }).await;
    a.wait_for("SMSG_TRAINER_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_TRAINER_LIST(l) => Some(l.spells.clone()),
        _ => None,
    })
    .await
}

/// A talent's trained rank (Mortal Strike's second) is red at a trainer to
/// a warrior without the talent, and asks for it, as mangos-classic's
/// `GetTrainerSpellState` wants the rank before by its chain; with the
/// talent it is green. Before, vmangos's spell_chain left the rank with no
/// rank before, so it was green to anyone, and could be bought.
#[tokio::test]
async fn a_trainer_offers_a_talents_trained_rank_only_with_the_talent() {
    let realm = harness::realm_with(trainer_data());
    let mut a = Client::connect(&realm, "TRAINS");
    let g = a.enter("Trainee", Race::Human, Class::Warrior).await;
    let trainer = guid::unit(WARRIOR_TRAINER, 960);
    a.sees(trainer).await;
    let row = |list: Vec<TrainerSpell>| {
        list.into_iter()
            .find(|s| s.spell == MORTAL_STRIKE_2)
            .map(|s| (s.state, s.required_spells[0]))
    };
    let without = row(trainer_list(&mut a, trainer).await);
    assert_eq!(
        without,
        Some((TrainerSpellState::Red, MORTAL_STRIKE)),
        "red, wanting the talent"
    );
    a.send(CMSG_TRAINER_BUY_SPELL {
        guid: trainer,
        id: MORTAL_STRIKE_2,
    })
    .await;
    a.wait_for("SMSG_TRAINER_BUY_FAILED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_TRAINER_BUY_FAILED(_)).then_some(())
    })
    .await;
    let map = map_of(&realm, g);
    on(&map, move |m| {
        m.players.get_mut(&g).unwrap().spells.push(MORTAL_STRIKE)
    })
    .await;
    let with = row(trainer_list(&mut a, trainer).await);
    assert_eq!(with, Some((TrainerSpellState::Green, MORTAL_STRIKE)));
}

/// The real warrior trainers: Mortal Strike's second rank (21551) is red,
/// asking for the talent (12294), to a level-48 warrior without it, and
/// green with it.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_trainers_offer_mortal_strikes_ranks_only_with_the_talent() {
    use super::tests_p3_spells_kit::{have_data, player, realm as real_realm, spawn};
    if !have_data() {
        return;
    }
    let realm = real_realm();
    let (mut a, g, map) = player(
        &realm,
        "TRAINREAL",
        "Realtrainee",
        Race::Human,
        Class::Warrior,
    )
    .await;
    let trainer = on(&map, move |m| {
        ready(m, g, 48, &[]);
        // Ander Germaine, Stormwind's warrior trainer.
        spawn(m, g, 914, 60, 3.0)
    })
    .await;
    let row = |list: Vec<TrainerSpell>| {
        list.into_iter()
            .find(|s| crate::game::map::spells::learns(&realm.data, s.spell) == 21_551)
            .map(|s| (s.state, s.required_spells[0]))
    };
    let without = row(trainer_list(&mut a, trainer).await);
    assert_eq!(
        without,
        Some((TrainerSpellState::Red, 12_294)),
        "red, wanting the talent"
    );
    on(&map, move |m| {
        m.players.get_mut(&g).unwrap().spells.push(12_294)
    })
    .await;
    let with = row(trainer_list(&mut a, trainer).await);
    assert_eq!(with.map(|w| w.0), Some(TrainerSpellState::Green));
}
