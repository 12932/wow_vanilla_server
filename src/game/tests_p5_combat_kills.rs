//! Phase 3's fifth round of combat tests, what a kill sets off (Thistle
//! Tea's `kill_feedback_test.exs`): on-kill procs, which kills count for
//! them, and a pet's kill counting for its master.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ProcEvent, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::map::auras::{self, *};
use crate::game::map::{Map, combat, procs};
use crate::game::tests_p3_combat::{self as base, *};

/// Spirit Tap's like: on a kill, a buff.
const TAP: u32 = 90_840;
const TAPPED: u32 = 90_841;
/// A brute worth no XP.
const WORTHLESS: u32 = 3710;

fn data() -> GameData {
    let mut d = base::data();
    let mut tap = SpellTemplate {
        id: TAP,
        name: "Tap".into(),
        equipped_item_class: -1,
        proc_flags: procs::PROC_KILL,
        proc_chance: 100,
        ..Default::default()
    };
    tap.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_PROC_TRIGGER_SPELL,
        target_a: 1,
        trigger_spell: TAPPED,
        ..Default::default()
    };
    let mut tapped = SpellTemplate {
        id: TAPPED,
        name: "Tapped".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    tapped.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(TAP, tap);
    d.spells.spells.insert(TAPPED, tapped);
    // Its spell_proc_event names a school, a family and an outcome a kill
    // has none of.
    d.spells.proc_events.insert(
        TAP,
        ProcEvent {
            school_mask: 4,
            family: 3,
            family_mask: [1, 0, 0],
            ex: procs::EX_CRITICAL_HIT,
            ..Default::default()
        },
    );
    let mut worthless = template(WORTHLESS, 7);
    worthless.static_flags1 = 0x2;
    d.creature_templates.insert(WORTHLESS, worthless);
    let p60 = d.player_level_stats[&(1, 1, 60)];
    d.player_level_stats.insert((1, 9, 60), p60);
    d.player_class_stats.insert((9, 60), (1000, 1000));
    d
}

fn tapped(m: &Map, g: Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(TAPPED))
}

fn talent(m: &mut Map, g: Guid) {
    let s = m.realm.data.spells.get(TAP).cloned().unwrap();
    auras::apply(m, g, g, &s, 0, 0);
}

/// Kill `victim` by `killer`'s hand (or deal it `damage`); whether `g`'s
/// talent went off, the buff then taken away.
fn kill(m: &mut Map, killer: Guid, victim: Guid, damage: u32, g: Guid) -> bool {
    combat::deal_damage(m, killer, victim, damage);
    let went = tapped(m, g);
    auras::remove_spell(m, g, TAPPED, None);
    went
}

/// Thistle Tea's "procs for an eligible max-level kill without an XP
/// award", "rejects gray and nonrewarding victims", "ignores nonlethal ...
/// and self-inflicted damage", "retains target eligibility for ...
/// creatures without rewards", "NPC kill procs do not require a player
/// reward target", "captures one fatal blow" and "kill masks ignore hit,
/// school, and family restrictions": a level 60's kill of a level 60 sets
/// off their on-kill talent, though it gives no XP, whatever its
/// `spell_proc_event` says; a grey kill doesn't, nor one worth no XP, nor a
/// blow short of killing, nor a death of one's own doing; a creature's kill
/// of a player sets off its own; a blow on the dead, nothing more.
#[tokio::test]
async fn on_kill_talents_go_off_for_a_kill_worth_xp_or_honor() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            talent(m, ga);
            let even = spawn(m, ga, BRUTE, 60, 3.0);
            let worthy = kill(m, ga, even, 1_000_000, ga);
            let again = kill(m, ga, even, 1_000_000, ga);
            let grey = spawn(m, ga, BRUTE, 20, 3.0);
            let grey = kill(m, ga, grey, 1_000_000, ga);
            let worthless = spawn(m, ga, WORTHLESS, 60, 3.0);
            let worthless = kill(m, ga, worthless, 1_000_000, ga);
            let short = spawn(m, ga, BRUTE, 60, 3.0);
            let short = kill(m, ga, short, 10, ga);
            // A brute that kills itself.
            let fool = spawn(m, ga, BRUTE, 60, 3.0);
            talent(m, fool);
            let own = kill(m, fool, fool, 1_000_000, fool);
            // A brute's kill of Alice, at level 1.
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            talent(m, brute);
            level(m, ga, 1);
            let npc = kill(m, brute, ga, 1_000_000, brute);
            (worthy, again, grey, worthless, short, own, npc)
        })
        .await
        .unwrap();
    assert!(r.0, "a level 60's kill of a level 60");
    assert!(!r.1, "the dead die once");
    assert!(!r.2, "a grey kill");
    assert!(!r.3, "a kill worth no XP");
    assert!(!r.4, "a blow short of killing");
    assert!(!r.5, "its own death");
    assert!(r.6, "a creature's kill of a player");
}

/// mangos's `Unit::Kill`, against Thistle Tea's "does not redirect a pet's
/// killing blow to its owner": a pet's kill sets off its master's on-kill
/// talent as well as its own.
#[tokio::test]
async fn a_pets_kill_sets_off_its_masters_on_kill_talent() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warlock).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            talent(m, ga);
            let pet = spawn(m, ga, HELPER, 60, 2.0);
            m.creature_mut(pet).unwrap().owner = Some(ga);
            let victim = spawn(m, ga, BRUTE, 60, 3.0);
            kill(m, pet, victim, 1_000_000, ga)
        })
        .await
        .unwrap();
    assert!(r, "the master's talent");
}
