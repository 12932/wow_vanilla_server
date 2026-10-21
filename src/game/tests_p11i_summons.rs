//! Round 11's test-only rows on creatures' summons (Thistle Tea's
//! `wild_summon_test`, `respawn_test` and `creature_pet_owner_test`): a
//! script's summon at the target it resolved, a timed death's summon
//! dying at its time, an owned summon gone at its time all the same, and
//! a creature's demon a guardian that leaves no slot behind.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::{ScriptStep, ScriptTable};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, scripts};
use crate::game::tests_p3_combat::{BRUTE, START, data, spawn};

async fn on<T: Send + 'static>(map: &MapHandle, f: impl FnOnce(&mut Map) -> T + Send + 'static) -> T {
    map.query(f).await.expect("the map answers")
}

/// A summoned brute: what the summons are.
const SUMMONED: u32 = 3_960;
const ATTACKING: u32 = 9_960;
const DYING: u32 = 9_961;
const TIMED: u32 = 9_962;

fn summon_step(id: u32, after_ms: u32, despawn_type: i32, attack: i32) -> ScriptStep {
    ScriptStep {
        id,
        command: 10,
        datalong: [SUMMONED, after_ms, 0, 0],
        dataint: [0, 0, attack, despawn_type],
        x: START.0 + 3.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = data();
    let mut t = d.creature_templates[&BRUTE].clone();
    t.entry = SUMMONED;
    d.creature_templates.insert(SUMMONED, t);
    // Attacking the source's victim (target type 1), not the target.
    d.scripts
        .generic
        .insert(ATTACKING, vec![summon_step(ATTACKING, 60_000, 1, 1)]);
    // Dying at its time, out of a fight (Thistle Tea's type 11).
    d.scripts.generic.insert(
        DYING,
        vec![summon_step(
            DYING,
            150,
            crate::game::map::event_ai::TIMED_DEATH,
            -1,
        )],
    );
    // Gone at its time (timed despawn, 3).
    d.scripts
        .generic
        .insert(TIMED, vec![summon_step(TIMED, 150, 3, -1)]);
    d
}

fn summoned_by(m: &Map, by: Guid) -> Vec<Guid> {
    m.creatures
        .iter()
        .filter(|(_, c)| {
            c.entry == SUMMONED && c.scripted.as_deref().and_then(|s| s.summoner) == Some(by)
        })
        .map(|(_, c)| c.guid)
        .collect()
}

async fn two() -> (Client, Guid, Client, Guid, MapHandle) {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    (a, ga, b, gb, map)
}

/// Thistle Tea's "attack the target their script resolved rather than the
/// script's target": a script's summons (command 10) attacks the unit its
/// `dataint3` target type finds (`scripts::summon_step`, by `find_target`
/// apart from the step's own target): its source's victim, Bob, not
/// Alice, whom the script was run at.
#[tokio::test]
async fn a_scripts_summons_attacks_the_target_it_resolved() {
    let (_a, ga, _b, gb, map) = two().await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 6.0);
        crate::game::map::ai::attacked(m, brute, gb);
        scripts::start(m, ScriptTable::Generic, ATTACKING, brute, ga);
        let summons = summoned_by(m, brute);
        let victims: Vec<Guid> = summons
            .iter()
            .filter_map(|g| m.creature(*g).map(|c| c.combat.victim))
            .collect();
        (m.creature(brute).map(|c| c.combat.victim), victims)
    })
    .await;
    assert_eq!(out.0, Some(gb), "the brute fights Bob");
    assert_eq!(out.1, vec![gb], "its summons attacks Bob, not Alice");
}

/// Thistle Tea's "scripted timed-death summons initialize the same
/// lifetime": a script's summons of the timed-death type (11, its
/// `dataint4`) dies at its time out of a fight
/// (`event_ai::when_due`'s `TIMED_DEATH`), leaving its corpse, with no
/// respawn of its own.
#[tokio::test]
async fn a_timed_death_summons_dies_at_its_time_and_leaves_a_corpse() {
    let (_a, ga, _b, _gb, map) = two().await;
    let (summoner, summons) = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 30.0);
        scripts::start(m, ScriptTable::Generic, DYING, brute, ga);
        (brute, summoned_by(m, brute))
    })
    .await;
    assert_eq!(summons.len(), 1);
    let s = summons[0];
    assert!(on(&map, move |m| m.creature(s).is_some_and(|c| c.alive())).await, "alive at first");
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let state = on(&map, move |m| m.creature(s).map(|c| c.alive())).await;
        if state == Some(false) {
            break;
        }
        assert_eq!(state, Some(true), "a corpse left, not gone");
        assert!(tokio::time::Instant::now() < deadline, "it never died");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = on(&map, move |m| {
        (
            m.creature(s).map(|c| (c.alive(), c.fields.get_u32(UNIT_FIELD_HEALTH))),
            summoned_by(m, summoner).len(),
        )
    })
    .await;
    assert_eq!(after.0, Some((false, 0)), "its corpse stays");
}

/// Thistle Tea's "does not hold owned pets that are not charmed": a
/// summons given an owner (a player's, not charmed) goes at its time all
/// the same, as one of no owner: its despawn is by its type, its life and
/// its fight (`event_ai::when_due`), not its ownership.
#[tokio::test]
async fn an_owned_summons_goes_at_its_time() {
    let (_a, ga, _b, _gb, map) = two().await;
    let summons = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 30.0);
        scripts::start(m, ScriptTable::Generic, TIMED, brute, ga);
        scripts::start(m, ScriptTable::Generic, TIMED, brute, ga);
        let summons = summoned_by(m, brute);
        if let Some(c) = m.creature_mut(summons[0]) {
            c.owner = Some(ga);
        }
        summons
    })
    .await;
    assert_eq!(summons.len(), 2);
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let left = on(&map, {
            let summons = summons.clone();
            move |m| summons.iter().filter(|g| m.creature(**g).is_some()).count()
        })
        .await;
        if left == 0 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{left} still there");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Thistle Tea's "child termination clears the slot and all child
/// projections": a creature's demon (Summon Imp, `SPELL_EFFECT_SUMMON_PET`)
/// is a guardian of its own, as mangos-classic's `EffectSummonPet` makes a
/// caster no player's (`GUARDIAN_PET`, `AddGuardian`, not `SetPet`), so
/// no slot (`UNIT_FIELD_SUMMON`) of the summoner's holds it; gone, it is
/// out of the world, and the summoner summons another.
#[tokio::test]
async fn a_creatures_demon_leaves_no_slot_and_another_comes_once_it_goes() {
    use crate::game::tests_pets::{IMP, SUMMON_IMP, WOLF, enter, place};
    let realm = harness::realm_with(crate::game::tests_pets::data());
    let (_w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let summoner = place(&map, gw, WOLF, 0x70_0200, 10.0).await;
    let imps = move |m: &Map| {
        m.creatures
            .iter()
            .filter(|(_, c)| c.entry == IMP && c.scripted.as_deref().and_then(|s| s.summoner) == Some(summoner))
            .map(|(_, c)| c.guid)
            .collect::<Vec<Guid>>()
    };
    let (first, slot) = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, summoner, SUMMON_IMP, None);
        let imp = imps(m);
        let slot = m.creature(summoner).unwrap().fields.get_guid(UNIT_FIELD_SUMMON);
        (imp, slot)
    })
    .await;
    assert_eq!(first.len(), 1, "its imp");
    assert_eq!(slot, Guid::zero(), "a guardian, in no slot");
    let imp = first[0];
    let after = on(&map, move |m| {
        scripts::despawn(m, imp);
        let gone = m.creature(imp).is_none();
        crate::game::map::spells::cast_triggered(m, summoner, SUMMON_IMP, None);
        (gone, imps(m), m.creature(summoner).unwrap().fields.get_guid(UNIT_FIELD_SUMMON))
    })
    .await;
    assert!(after.0, "the imp gone from the world");
    assert_eq!(after.1.len(), 1, "another in its place: {:?}", after.1);
    assert_ne!(after.1[0], imp);
    assert_eq!(after.2, Guid::zero());
}
