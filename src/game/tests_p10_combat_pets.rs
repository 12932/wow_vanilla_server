//! Round 10's tests of a pet's fight, ported from Thistle Tea's
//! `test/game/core/combat/engagement/pet_combat_test.exs`: a creature holds
//! a pet in its fight only while it fights it, alive, in a fight of its
//! own, not evading, in the map; a blow between a pet and a player holds it
//! five seconds, through its orders; its own blow of any outcome holds it
//! five seconds more, and its recall stands meanwhile; once calm, it holds
//! no one and an order doesn't bring it back in.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::auras::AURA_DAMAGE_IMMUNITY;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::creature::Creature;
use crate::game::map::pets::{self, ACT_COMMAND, ACT_REACTION};
use crate::game::map::{Map, ai};
use crate::game::tests_p3_combat::{aura_of, forced_swing};
use crate::game::tests_pets::{WOLF, enter, place_in};
use crate::game::tests_pets2::{more, tamed_on};

/// A second of the map: its spells and its pets.
fn think(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += std::time::Duration::from_secs(1);
        crate::game::map::spells::update(m);
        pets::update(m);
    }
}

fn order(m: &mut Map, owner: Guid, pet: Guid, action: u8, kind: u32) {
    let data = pets::button(u32::from(action), kind);
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

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
}

/// `who`, a player, put a yard from `to`.
fn beside(m: &mut Map, who: Guid, to: Guid) {
    let at = m.position_of(to).unwrap();
    let o = m.players[&who].movement.orientation;
    let at = Vector3d {
        x: at.x + 1.0,
        ..at
    };
    crate::game::map::movement::teleport_within(m, who, at, o);
    crate::game::map::movement::teleport_ack(m, who);
}

/// A wolf near `gh` fighting `pet`, as one that struck it and took it on
/// as its victim does.
fn wolf_on(m: &mut Map, gh: Guid, pet: Guid, n: u32) -> Guid {
    let w = place_in(m, gh, WOLF, 0x70_0300 + n, 6.0);
    combat::engage(m, w, pet);
    ai::attacked(m, w, pet);
    w
}

/// How a wolf stops fighting a pet: the map, the wolf, the pet's master.
type End = fn(&mut Map, Guid, Guid);

/// Thistle Tea's `maintain/2` "prunes dead, despawned, evading, inactive,
/// moved and reincarnated enemies": a passive pet a wolf fights stays in
/// its fight past its window while the wolf does; the wolf dead, despawned,
/// evading, out of its fight though alive, gone from the map, or come back
/// anew under its guid, the pet is out of it at once.
#[tokio::test]
async fn a_pets_fight_is_held_only_while_a_creature_still_fights_it() {
    let realm = crate::game::harness::realm_with(more());
    let (_realm, _h, gh, map, pet, _wolf) = tamed_on(realm).await;
    let out = map
        .query(move |m| {
            order(m, gh, pet, pets::REACT_PASSIVE, ACT_REACTION);
            let ends: [(&str, End); 6] = [
                ("dead", |m, w, gh| ai::die(m, w, gh)),
                ("despawned", |m, w, _| ai::quiet_death(m, w)),
                ("evading", |m, w, _| ai::enter_evade(m, w)),
                ("out of its fight", |m, w, _| combat::leave_combat(m, w)),
                ("gone from the map", |m, w, _| {
                    m.remove_creature(w);
                }),
                ("come back anew", |m, w, gh| {
                    m.remove_creature(w);
                    let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
                    let at = m.position_of(gh).unwrap();
                    m.add_creature(Creature::new(w, &t, 1, at, 0.0));
                }),
            ];
            let mut seen = Vec::new();
            for (n, (what, end)) in ends.into_iter().enumerate() {
                let w = wolf_on(m, gh, pet, n as u32);
                think(m, 7);
                let held = in_combat(m, pet);
                end(m, w, gh);
                think(m, 1);
                seen.push((what, held, in_combat(m, pet)));
            }
            seen
        })
        .await
        .unwrap();
    for (what, held, after) in out {
        assert!(held, "{what}: held while the wolf fights it");
        assert!(!after, "{what}: out of its fight");
    }
}

/// Thistle Tea's `maintain/2` "retains PvP contact across commands until
/// the drop window expires" and "ending combat releases the pet's outgoing
/// threat references": a player's blow holds a pet in its fight five
/// seconds, told to stay or follow and made passive the while, though the
/// player lives on beside it (mangos's PvP timer); then it is out, holding
/// no one (a pet keeps no threat list, mangos's `CanHaveThreatList`) and
/// fighting no one, and an order doesn't put it back in.
#[tokio::test]
async fn a_players_blow_holds_a_pet_five_seconds_through_its_orders_and_then_nothing_does() {
    let realm = crate::game::harness::realm_with(more());
    let (realm, _h, gh, map, pet, _wolf) = tamed_on(realm).await;
    let (_b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    let out = map
        .query(move |m| {
            beside(m, gb, pet);
            let mut seen = Vec::new();
            for command in [pets::COMMAND_STAY, pets::COMMAND_FOLLOW] {
                // A blow, as a swing deals it.
                combat::engage(m, gb, pet);
                combat::deal_damage(m, gb, pet, 1);
                order(m, gh, pet, command, ACT_COMMAND);
                order(m, gh, pet, pets::REACT_PASSIVE, ACT_REACTION);
                let target = m
                    .unit_fields(pet)
                    .unwrap()
                    .get_guid(UNIT_FIELD_TARGET)
                    .is_zero();
                think(m, 4);
                let held = in_combat(m, pet);
                think(m, 1);
                let out = in_combat(m, pet);
                let c = m.creature(pet).unwrap();
                let holds_none =
                    c.combat.attackers.is_empty() && c.threat.is_empty() && !c.combat.attacking;
                order(m, gh, pet, pets::COMMAND_FOLLOW, ACT_COMMAND);
                think(m, 1);
                seen.push((
                    target,
                    held,
                    out,
                    holds_none,
                    in_combat(m, pet),
                    m.unit_view(gb).is_some_and(|u| u.alive),
                ));
            }
            seen
        })
        .await
        .unwrap();
    for (n, (target, held, out, holds_none, ordered, alive)) in out.into_iter().enumerate() {
        assert!(target && held, "{n}: no target, held 4 s");
        assert!(!out && holds_none, "{n}: out at 5 s, holding no one");
        assert!(!ordered && alive, "{n}: an order after keeps it out");
    }
}

/// Thistle Tea's `receive/4` "outgoing melee refreshes contact without
/// restarting a recalled attack": a pet's own blow at a player, landed,
/// missed, dodged, parried or on the immune, holds it in its fight five
/// seconds from that blow; called back, it stays called back, with no
/// target, through the rest of that window, then is out. A swing at a
/// creature on its way home is no blow, and holds it nothing. (wvs
/// resolves a swing at once, so none comes after the recall: a recalled
/// pet's attack has stopped.)
#[tokio::test]
async fn a_pets_own_blow_of_any_outcome_holds_it_five_seconds_more_and_its_recall_stands() {
    let realm = crate::game::harness::realm_with(more());
    let (realm, _h, gh, map, pet, _wolf) = tamed_on(realm).await;
    let (_b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    let out = map
        .query(move |m| {
            order(m, gh, pet, pets::REACT_PASSIVE, ACT_REACTION);
            let mut held = Vec::new();
            for outcome in [Outcome::Hit, Outcome::Miss, Outcome::Dodge, Outcome::Parry] {
                beside(m, gb, pet);
                forced_swing(m, pet, gb, outcome);
                combat::stop_attack(m, pet);
                think(m, 3);
                held.push(in_combat(m, pet));
            }
            aura_of(m, gb, 997_700, AURA_DAMAGE_IMMUNITY, 127, 0);
            beside(m, gb, pet);
            forced_swing(m, pet, gb, Outcome::Hit);
            order(m, gh, pet, pets::COMMAND_FOLLOW, ACT_COMMAND);
            let recalled = |m: &Map| {
                (
                    pets::state(m, pet).is_some_and(|s| s.returning),
                    m.unit_fields(pet)
                        .unwrap()
                        .get_guid(UNIT_FIELD_TARGET)
                        .is_zero(),
                    !m.combat(pet).unwrap().attacking,
                )
            };
            let after = recalled(m);
            think(m, 4);
            let window = (in_combat(m, pet), !m.combat(pet).unwrap().attacking);
            think(m, 1);
            let over = in_combat(m, pet);
            // A wolf on its way home: the swing at it isn't made.
            let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
            let at = m.position_of(pet).unwrap();
            let w = m.add_creature(Creature::new(
                guid::unit(WOLF, 0x70_0310),
                &t,
                1,
                Vector3d {
                    x: at.x + 1.0,
                    ..at
                },
                0.0,
            ));
            m.creature_mut(w).unwrap().evading = true;
            forced_swing(m, pet, w, Outcome::Hit);
            let evaded = (
                in_combat(m, pet),
                m.combat(pet).unwrap().last_action.is_none(),
            );
            (held, after, window, over, evaded)
        })
        .await
        .unwrap();
    let (held, after, window, over, evaded) = out;
    assert_eq!(held, [true; 4], "each blow holds it on: 12 s in all");
    assert_eq!(after, (true, true, true), "called back, no target");
    assert_eq!(window, (true, true), "held 4 s after its last, called back");
    assert!(!over, "out 5 s after it");
    assert_eq!(
        evaded,
        (false, true),
        "a swing at the evading holds nothing"
    );
}
