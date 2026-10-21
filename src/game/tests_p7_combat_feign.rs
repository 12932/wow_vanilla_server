//! Round 7's Feign Death (Thistle Tea's `feign_death_test.exs`): what a
//! believed one ends and keeps, what ends it, and who may still reach the
//! one lying in it.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_FEIGN_DEATH, INTERRUPT_MOVE};
use crate::game::map::combat;
use crate::game::map::spells::{Casting, Targets};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_combat::{self as p3, BRUTE};
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{WATCHER, attacking, creature_at, in_combat, look, off};

/// Feign Death, as the client's: ten seconds, and moving, turning,
/// entering a fight, talking, using and attacking end it.
const FEIGN: u32 = 5384;
/// An aura that losing one's selection ends (`IMMUNE_OR_LOST_SELECTION`).
const SENSITIVE: u32 = 95_310;
/// A three-second bolt.
const BOLT: u32 = 95_311;

fn data() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    d.spells.durations.insert(
        21,
        SpellDuration {
            base: 10_000,
            ..Default::default()
        },
    );
    let mut feign = SpellTemplate {
        id: FEIGN,
        name: "Feign Death".into(),
        equipped_item_class: -1,
        family_name: 9,
        family_flags: 256,
        duration_index: 21,
        aura_interrupt_flags: 0x3C3C,
        ..Default::default()
    };
    feign.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_FEIGN_DEATH,
        target_a: 1,
        ..Default::default()
    };
    let mut sensitive = SpellTemplate {
        id: SENSITIVE,
        name: "Sensitive".into(),
        equipped_item_class: -1,
        aura_interrupt_flags: crate::game::map::spells::interrupts::IMMUNE_OR_LOST_SELECTION,
        ..Default::default()
    };
    sensitive.effects[0] = SpellEffect {
        effect: 6,
        aura: 4,
        target_a: 1,
        ..Default::default()
    };
    let mut bolt = SpellTemplate {
        id: BOLT,
        name: "Bolt".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    bolt.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    for s in [feign, sensitive, bolt] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn put_on(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).cloned().expect("the spell");
    let _ = auras::apply_spell(m, g, g, &s, &[(0, 0)], auras::Context::default());
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// `g` three seconds into a bolt at `at`.
fn casting_at(m: &mut Map, g: Guid, at: Guid) {
    let now = m.now;
    if let Some(c) = m.caster_mut(g) {
        c.casting = Some(Casting {
            spell: BOLT,
            targets: Targets {
                unit: Some(at),
                ..Default::default()
            },
            ends: now + Duration::from_secs(3),
            item: None,
            pushbacks: 0,
            channel_until: None,
            duration: 3000,
            at: None,
        });
    }
}

fn casting(m: &Map, g: Guid) -> bool {
    m.caster(g).is_some_and(|c| c.casting.is_some())
}

fn posed(m: &Map, g: Guid) -> (bool, bool) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_u32(UNIT_DYNAMIC_FLAGS) & 0x20 != 0,
        combat::feigning(m, g),
    )
}

/// Thistle Tea's believed Feign Death (`reconcile/4`, `target_lost/3`,
/// `successful?/1`): what losing one's selection ends goes with it; every
/// cast at the one feigning is interrupted, and a player swinging at them
/// stops, in the fight still; no creature notices them or picks them from
/// its list while it lasts, though a player, a player's area spell and a
/// helper still may. Seen through, it ends none of that.
#[tokio::test]
async fn a_believed_feign_death_drops_the_fight_and_hides_from_creatures_alone() {
    let (realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(data(), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, gb, 60);
            put(m, ga, off(0.0, 0.0, 0.0));
            put(m, gb, off(3.0, 0.0, 0.0));
            put_on(m, ga, SENSITIVE);
            // A brute fights Alice and casts at her; Bob, in a fight of
            // his own, swings at her.
            let brute = creature_at(m, ga, BRUTE, off(2.0, 0.0, 0.0));
            crate::game::map::ai::attacked(m, brute, ga);
            casting_at(m, brute, ga);
            combat::enter(m, gb, false);
            if let Some(c) = m.combat_mut(gb) {
                c.attacking = true;
                c.victim = ga;
            }
            casting_at(m, gb, ga);
            put_on(m, ga, FEIGN);
            let cut = (
                !has(m, ga, SENSITIVE),
                in_combat(m, ga),
                casting(m, brute),
                casting(m, gb),
                m.combat(gb).is_some_and(|c| c.attacking),
                in_combat(m, gb),
                posed(m, ga),
            );
            // A watcher she lies by notices nothing; a brute fighting Bob,
            // given her on its list well over him (an area's blow), passes
            // her over.
            let watcher = creature_at(m, ga, WATCHER, off(-3.0, 0.0, 0.0));
            let other = creature_at(m, ga, BRUTE, off(4.0, 0.0, 0.0));
            crate::game::map::ai::attacked(m, other, gb);
            crate::game::map::ai::add_threat(m, other, gb, 10.0);
            crate::game::map::ai::add_threat(m, other, ga, 50.0);
            look(m);
            look(m);
            let hidden = (
                attacking(m, watcher, ga),
                attacking(m, other, ga),
                attacking(m, other, gb),
            );
            // A player (both free for all), a creature's area spell and a
            // helper still reach her.
            for g in [ga, gb] {
                m.players.get_mut(&g).unwrap().fields.set_flag(
                    PLAYER_FLAGS,
                    crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
                    true,
                );
            }
            let reached = (
                combat::can_attack(m, gb, ga),
                combat::can_attack_in_area(m, brute, ga),
                combat::can_assist(m, gb, ga),
            );
            (cut, hidden, reached)
        })
        .await
        .unwrap();
    let (cut, hidden, reached) = r;
    assert_eq!(
        cut,
        (true, false, false, false, false, true, (true, true)),
        "(sensitive gone, her fight, the brute's cast, Bob's cast, Bob's swing, Bob's fight, pose)"
    );
    assert_eq!(hidden, (false, false, true), "no creature takes her");
    assert_eq!(reached, (true, true, true), "players, areas and help do");
}

/// Thistle Tea's resisted Feign Death keeps what a believed one ends:
/// what losing one's selection ends stays, she stays in the fight and on
/// its list, and creatures may take her (only her own swing stops).
#[tokio::test]
async fn a_feign_death_seen_through_keeps_the_fight_the_list_and_her_auras() {
    let realm = crate::game::harness::realm_with(data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            p3::level(m, ga, 1);
            m.spells.rolls = Some(true);
            put(m, ga, off(0.0, 0.0, 0.0));
            put_on(m, ga, SENSITIVE);
            // Six level-60 brutes against a level-1 player: one sees
            // through it.
            let brutes: Vec<Guid> = (0..6)
                .map(|i| creature_at(m, ga, BRUTE, off(2.0, i as f32, 0.0)))
                .collect();
            for b in &brutes {
                crate::game::map::ai::attacked(m, *b, ga);
            }
            // She swings at one and shoots at it.
            let now = m.now;
            if let Some(c) = m.combat_mut(ga) {
                c.attacking = true;
                c.victim = brutes[0];
                c.auto_repeat = Some(combat::AutoRepeat {
                    spell: 75,
                    target: brutes[0],
                    next_at: now + Duration::from_secs(2),
                });
            }
            put_on(m, ga, FEIGN);
            m.spells.rolls = None;
            (
                has(m, ga, SENSITIVE),
                in_combat(m, ga),
                brutes
                    .iter()
                    .all(|b| m.creature(*b).unwrap().threat.iter().any(|(t, _)| *t == ga)),
                posed(m, ga),
                m.combat(ga).map(|c| (c.attacking, c.auto_repeat.is_some())),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (true, true, true, (true, false), Some((false, false))),
        "(her aura, her fight, the lists, the pose, her swing and shot stopped)"
    );
    a.wait_for("SMSG_CANCEL_AUTO_REPEAT", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CANCEL_AUTO_REPEAT).then_some(())
    })
    .await;
}

/// mangos's believed Feign Death with a pet fighting (where Thistle Tea
/// holds the hunter in the fight six seconds): the hunter's fight ends,
/// and the pet's blows bring them back into it only once they are up.
#[tokio::test]
async fn a_fighting_pets_blows_bring_a_feigning_hunter_back_only_once_up() {
    let realm = crate::game::harness::realm_with(data());
    let mut h = crate::game::harness::Client::connect(&realm, "H");
    let gh = h.enter("Hana", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            p3::level(m, gh, 60);
            put(m, gh, off(0.0, 0.0, 0.0));
            crate::game::map::pets::give_pet(
                m,
                gh,
                crate::game::tests_p7_combat::BEAST_WATCHER,
                60,
            )
            .expect("a pet");
            let pet = crate::game::map::pets::pet_of(m, gh).unwrap();
            put(m, pet, off(2.0, 0.0, 0.0));
            let brute = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
            let blow = |m: &mut Map| p3::forced_swing(m, pet, brute, combat::Outcome::Hit);
            let state = |m: &Map| {
                (
                    in_combat(m, gh),
                    m.creature(brute)
                        .unwrap()
                        .threat
                        .iter()
                        .any(|(t, _)| *t == gh),
                )
            };
            blow(m);
            let pulled = state(m);
            put_on(m, gh, FEIGN);
            let feigned = state(m);
            blow(m);
            let lying = state(m);
            auras::remove_spell(m, gh, FEIGN, None);
            blow(m);
            (pulled, feigned, lying, state(m))
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        ((true, true), (false, false), (false, false), (true, true)),
        "(in the fight, on the list): pulled, feigned, the pet's blow, up"
    );
}

/// Thistle Tea's "movement, expiry and death clear both pose and target
/// protection": moving, the ten seconds running out, or dying, and the
/// pose and the protection go together.
#[tokio::test]
async fn moving_running_out_or_dying_ends_the_pose_and_the_protection() {
    let (_realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            put(m, ga, off(0.0, 0.0, 0.0));
            let mut seen = Vec::new();
            for how in 0..3 {
                put_on(m, ga, FEIGN);
                let on = posed(m, ga);
                match how {
                    0 => auras::interrupt(m, ga, INTERRUPT_MOVE),
                    1 => {
                        m.now += Duration::from_secs(11);
                        auras::update(m);
                    }
                    _ => crate::game::map::death::kill(m, ga),
                }
                let flags = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_FLAGS);
                seen.push((on, posed(m, ga), flags & auras::UNIT_FLAG_FEIGN_DEATH != 0));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(r, vec![((true, true), (false, false), false); 3]);
}
