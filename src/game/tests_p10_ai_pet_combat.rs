//! Round 10's tests of how long a pet stays in its fight, ported from
//! Thistle Tea's `test/game/core/combat/engagement/pet_combat_test.exs`
//! (docs/parity/matrix.csv): a pet's fight is over as its master's is (your
//! call on #20, mangos's `CombatManager::Update`): once no living creature
//! holds it on its threat list or attacks it, and its PvP timer has run
//! out; its own list goes with it.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::pets::{self, ACT_COMMAND, ACT_REACTION, Kind};
use crate::game::map::{Map, ai, combat, scripts};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{WOLF, effect, enter, place_in, spell};
use crate::game::tests_pets2::{bring, flag_pvp, more, tamed_on};

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
}

fn order(m: &mut Map, owner: Guid, pet: Guid, data: u32) {
    pets::handle(
        m,
        owner,
        ClientOpcodeMessage::CMSG_PET_ACTION(CMSG_PET_ACTION {
            pet,
            data,
            target: Guid::zero(),
        }),
    );
}

/// The pet told `command` and made passive.
fn stand_down(m: &mut Map, owner: Guid, pet: Guid, command: u8) {
    order(m, owner, pet, pets::button(u32::from(command), ACT_COMMAND));
    order(
        m,
        owner,
        pet,
        pets::button(u32::from(pets::REACT_PASSIVE), ACT_REACTION),
    );
}

/// The pets' steps, `ms` on in steps of a tenth of a second.
fn step(m: &mut Map, ms: u64) {
    for _ in 0..ms / 100 {
        m.now += Duration::from_millis(100);
        pets::update(m);
    }
}

/// Thistle Tea's `maintain/2` "prunes dead, despawned, evading, inactive,
/// moved and reincarnated enemies": a passive pet called back, held in its
/// fight by a wolf with it on its threat list, is out of it once the wolf
/// is dead, gone (from this map: gone to another is the same), evading,
/// out of its own fight, or come back anew under its guid.
#[tokio::test]
async fn a_pets_fight_ends_once_its_foe_is_dead_gone_evading_out_of_its_fight_or_new() {
    let (_realm, _h, gh, map, pet, _wolf) = tamed_with(crate::game::tests_pets::data()).await;
    let out = map
        .query(move |m| {
            type Ending = fn(&mut Map, Guid, Guid);
            let endings: [(&str, Ending); 5] = [
                ("dead", |m, w, gh| ai::die(m, w, gh)),
                ("gone", |m, w, _| scripts::despawn(m, w)),
                ("evading", |m, w, _| ai::enter_evade(m, w)),
                ("out of its fight", |m, w, _| {
                    m.creature_mut(w).unwrap().threat.clear();
                    combat::leave_combat(m, w);
                }),
                ("new", |m, w, gh| {
                    let old = m.remove_creature(w).unwrap();
                    let counter = w.guid() as u32 & 0x00FF_FFFF;
                    let again = place_in(m, gh, old.entry, counter, 6.0);
                    assert_eq!(again, w, "the same guid");
                }),
            ];
            let mut out = Vec::new();
            for (i, (what, end)) in endings.into_iter().enumerate() {
                let w = place_in(m, gh, WOLF, 0x70_0040 + i as u32, 6.0);
                combat::engage(m, w, pet);
                m.creature_mut(w).unwrap().threat.push((pet, 1.0));
                stand_down(m, gh, pet, pets::COMMAND_FOLLOW);
                step(m, 2_000);
                let held = in_combat(m, pet);
                end(m, w, gh);
                step(m, 2_000);
                out.push((what, held, in_combat(m, pet)));
                if m.creature(w).is_some() {
                    scripts::despawn(m, w);
                }
            }
            out
        })
        .await
        .unwrap();
    for (what, held, after) in out {
        assert!(held, "{what}: held while the wolf holds it");
        assert!(!after, "{what}: out of its fight");
    }
}

/// Thistle Tea's `maintain/2` "retains PvP contact across commands until
/// the drop window expires" and "ending combat releases the pet's outgoing
/// threat references", and `receive/4` "outgoing melee refreshes contact
/// without restarting a recalled attack": a pet that struck an enemy player
/// and was told to stay or follow, passive, stays in its fight 5 s from its
/// last hostile act (one after its recall, its DoT's tick, counts: wvs has
/// no late swing result, a blow lands as it is swung), its attack not taken
/// up again; then it is out, its own threat list gone, the player no
/// longer held by it, and a Follow after puts it in no fight.
#[tokio::test]
async fn a_pets_pvp_fight_lasts_five_seconds_from_its_last_act_through_its_commands() {
    let realm = crate::game::harness::realm_with(more());
    let (realm, _h, gh, map, pet, _wolf) = tamed_on(realm).await;
    let (_b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    bring(&map, gb, gh, 8.0).await;
    flag_pvp(&map, vec![gh, gb]);
    let out = map
        .query(move |m| {
            let mut out = Vec::new();
            for command in [pets::COMMAND_STAY, pets::COMMAND_FOLLOW] {
                // Its master's Attack, its blow, and its threat on him.
                order(m, gh, pet, pets::button(u32::from(pets::REACT_DEFENSIVE), ACT_REACTION));
                pets::attack_target(m, pet, gb);
                combat::engage(m, pet, gb);
                m.creature_mut(pet).unwrap().threat.push((gb, 20.0));
                stand_down(m, gh, pet, command);
                step(m, 3_000);
                // A hostile act of its after the recall: 5 s from now.
                combat::engage(m, pet, gb);
                step(m, 4_900);
                let kept = (
                    in_combat(m, pet),
                    m.combat(pet).is_some_and(|c| c.attacking),
                );
                step(m, 200);
                let over = (
                    in_combat(m, pet),
                    m.creature(pet).unwrap().threat.is_empty(),
                );
                // He, his own timer out too, is held by no pet of hers.
                let now = m.now;
                let c = m.combat_mut(gb).unwrap();
                c.combat_until = Some(now);
                c.next_check = Some(now);
                combat::update_players(m);
                let him = in_combat(m, gb);
                order(m, gh, pet, pets::button(u32::from(pets::COMMAND_FOLLOW), ACT_COMMAND));
                step(m, 200);
                out.push((command, kept, over, him, in_combat(m, pet)));
            }
            out
        })
        .await
        .unwrap();
    for (command, kept, over, him, again) in out {
        assert_eq!(kept, (true, false), "{command}: in its fight, not attacking");
        assert_eq!(over, (false, true), "{command}: out at 5 s, its list gone");
        assert!(!him, "{command}: he is out of his");
        assert!(!again, "{command}: a Follow starts no fight");
    }
}

/// A stun that damage breaks.
const SAP: u32 = 95_395;

/// Thistle Tea's `TargetSelection.next/2` "NPC pets use threat order and
/// never restart combat while their owner evades" (mangos's `GuardianAI`
/// on its own threat list, `CanHaveThreatList` for a creature's pet): a
/// creature's demon takes the highest on its threat list, the lower guid
/// of two as high; the one under a stun that damage breaks it leaves be
/// for the next; and while its master goes home it takes no one up.
#[tokio::test]
async fn a_creatures_pet_takes_its_foes_by_threat_and_none_while_its_master_goes_home() {
    let mut d = crate::game::tests_pets::data();
    let mut sap = spell(SAP, "Sap", [effect(6, 0, 6), effect(0, 0, 0), effect(0, 0, 0)]);
    sap.effects[0].aura = crate::game::map::auras::AURA_MOD_STUN;
    sap.aura_interrupt_flags = crate::game::map::auras::INTERRUPT_DAMAGE;
    d.spells.spells.insert(SAP, sap);
    let (_realm, _h, gh, map, _pet, _wolf) = tamed_with(d).await;
    let out = map
        .query(move |m| {
            let owner = place_in(m, gh, WOLF, 0x70_0050, 30.0);
            let at = m.position_of(owner).unwrap();
            let pet = pets::testing::spawn(m, owner, WOLF, Kind::Demon, 10, at, 0.0);
            let [a, b, c] = [0u32, 1, 2].map(|i| {
                let f = place_in(m, gh, WOLF, 0x70_0051 + i, 27.0 + i as f32);
                let x = m.unit_fields_mut(f).unwrap();
                x.set_u32(UNIT_FIELD_FACTIONTEMPLATE, 1);
                x.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                x.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
                f
            });
            m.creature_mut(pet).unwrap().threat = vec![(a, 10.0), (b, 30.0), (c, 30.0)];
            let victim = |m: &mut Map| {
                m.now += Duration::from_millis(500);
                pets::update(m);
                m.combat(pet).filter(|c| c.attacking).map(|c| c.victim)
            };
            let first = victim(m);
            combat::stop_attack(m, pet);
            crate::game::map::spells::cast_triggered(m, owner, SAP, Some(b));
            let sapped = victim(m);
            combat::stop_attack(m, pet);
            m.creature_mut(owner).unwrap().evading = true;
            let home = victim(m);
            m.creature_mut(owner).unwrap().evading = false;
            (first == Some(b), sapped == Some(c), home)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, None));
}
