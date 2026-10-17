//! Phase 3's combat tests, part six (Thistle Tea's `daze_test.exs`,
//! `extra_attacks_test.exs` and `sheath_test.exs`): a creature's blow from
//! behind dazing a player, the attacks a proc or Reckoning owes, and
//! sheathing.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::{Map, auras};
use crate::game::tests_p3_combat::{self as base, *};

const CHANNEL_BROKEN: u32 = 95_500;
const CHANNEL_KEPT: u32 = 95_501;

fn data() -> GameData {
    let mut d = base::data();
    let mut dazed = SpellTemplate {
        id: combat::SPELL_DAZED,
        name: "Dazed".into(),
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 9,
        ..Default::default()
    };
    dazed.effects[0] = SpellEffect {
        effect: 6,
        aura: 33,
        base_points: -51,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(dazed.id, dazed);
    // Two channels: one sheathing breaks, one it doesn't.
    for (id, flags) in [(CHANNEL_BROKEN, 0x200), (CHANNEL_KEPT, 0)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Channel {id}"),
            range_index: 1,
            equipped_item_class: -1,
            duration_index: 9,
            channel_interrupt_flags: flags,
            ..Default::default()
        };
        s.attributes[1] = 0x4;
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: 4,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d.spells.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 4_000,
            ..Default::default()
        },
    );
    d
}

/// A creature `yards` before `g` (behind, below none) hitting for five.
fn striker(m: &mut Map, g: Guid, yards: f32) -> Guid {
    let brute = spawn(m, g, BRUTE, 20, yards);
    let f = m.unit_fields_mut(brute).unwrap();
    f.set_f32(UNIT_FIELD_MINDAMAGE, 5.0);
    f.set_f32(UNIT_FIELD_MAXDAMAGE, 5.0);
    brute
}

fn dazed(m: &Map, g: Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(combat::SPELL_DAZED))
}

fn undaze(m: &mut Map, g: Guid) {
    auras::remove_spell(m, g, combat::SPELL_DAZED, None);
}

#[test]
fn a_dazes_chance_grows_with_level_and_skill_over_defense_to_two_fifths() {
    for (level, chance) in [(1, 1.15), (10, 7.0), (29, 19.35), (30, 20.0), (60, 20.0)] {
        let got = combat::daze_chance(level, level * 5, level * 5);
        assert!((got - chance).abs() < 0.001, "{level}: {got}");
    }
    // At 30 with 150 defense: a fifth; with one point, capped at two
    // fifths; against a level-1 attacker, none; 25 more skill, a quarter.
    assert_eq!(combat::daze_chance(30, 150, 150), 20.0);
    assert_eq!(combat::daze_chance(30, 150, 1), 40.0);
    assert_eq!(combat::daze_chance(30, 5, 150), 0.0);
    assert_eq!(combat::daze_chance(30, 175, 150), 25.0);
    // Twenty more defense: 16%.
    assert_eq!(combat::daze_chance(30, 150, 170), 16.0);
}

#[tokio::test]
async fn a_creatures_blow_from_behind_that_lands_and_doesnt_kill_dazes_a_player() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            combat::FORCED_DAZE_ROLL.with(|f| f.set(Some(0.0)));
            let mut out = Vec::new();
            let hit = |m: &mut Map, from: Guid, o: Outcome| {
                undaze(m, ga);
                forced_swing(m, from, ga, o);
                dazed(m, ga)
            };
            // From behind, landing: dazed, by the creature.
            let behind = striker(m, ga, -1.0);
            out.push(hit(m, behind, Outcome::Hit));
            let by = auras::auras(m, ga)
                .unwrap()
                .holders
                .iter()
                .any(|h| h.spell == combat::SPELL_DAZED && h.caster == behind);
            // In front; a miss; a pet's blow: none.
            let front = striker(m, ga, 1.0);
            out.push(hit(m, front, Outcome::Hit));
            out.push(hit(m, behind, Outcome::Miss));
            let pet = striker(m, ga, -1.0);
            m.creature_mut(pet).unwrap().owner = Some(crate::game::guid::player(9_999));
            out.push(hit(m, pet, Outcome::Hit));
            // A blow that kills dazes no corpse.
            m.unit_fields_mut(ga).unwrap().set_u32(UNIT_FIELD_HEALTH, 1);
            out.push(hit(m, behind, Outcome::Hit));
            combat::FORCED_DAZE_ROLL.with(|f| f.set(None));
            (out, by)
        })
        .await
        .unwrap();
    assert_eq!(r.0, vec![true, false, false, false, false]);
    assert!(r.1);
}

#[tokio::test]
async fn an_extra_attack_batch_doesnt_stack_and_reckonings_bank_arms_on_a_swing() {
    let (_realm, _a, ga, map) = setup(Class::Paladin).await;
    let r = map
        .query(move |m| {
            let g = ga;
            combat::grant_extra_attacks(m, g, 1, 2);
            combat::grant_extra_attacks(m, g, 1, 3);
            let batch = m.combat(g).unwrap().extra_attacks;
            // None from inside an extra attack.
            m.combat_mut(g).unwrap().extra_attacks = 0;
            m.combat_mut(g).unwrap().in_extra_attack = true;
            combat::grant_extra_attacks(m, g, 1, 2);
            let recursive = m.combat(g).unwrap().extra_attacks;
            m.combat_mut(g).unwrap().in_extra_attack = false;
            // Reckoning banks four at most, and holds off other grants.
            for _ in 0..6 {
                combat::grant_extra_attacks(m, g, combat::SPELL_RECKONING, 1);
            }
            let held = m.combat(g).unwrap().held_extra_attacks;
            combat::grant_extra_attacks(m, g, 1, 1);
            let blocked = m.combat(g).unwrap().extra_attacks;
            // A regular swing arms them.
            let brute = spawn(m, g, BRUTE, 20, 1.0);
            forced_swing(m, g, brute, Outcome::Miss);
            let c = m.combat(g).unwrap();
            (
                batch,
                recursive,
                held,
                blocked,
                c.extra_attacks,
                c.held_extra_attacks,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (2, 0, 4, 0, 4, 0));
}

#[tokio::test]
async fn extra_attacks_are_main_hand_swings_that_keep_a_queued_strike_and_wait_for_reach() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let g = ga;
            let brute = spawn(m, g, BRUTE, 20, 1.0);
            let far = spawn(m, g, BRUTE, 20, 20.0);
            m.players.get_mut(&g).unwrap().spells.push(STRIKE);
            let now = m.now;
            let health = |m: &Map, b: Guid| m.unit_fields(b).unwrap().get_u32(UNIT_FIELD_HEALTH);
            // Two owed at a victim out of reach: they wait.
            {
                let c = m.combat_mut(g).unwrap();
                c.victim = far;
                c.attacking = true;
                c.next_swing = Some(now + Duration::from_secs(5));
                c.next_offhand = Some(now + Duration::from_secs(9));
                c.extra_attacks = 2;
            }
            combat::try_swing(m, g);
            let waiting = m.combat(g).unwrap().extra_attacks;
            // In reach: two white swings, the strike and timers kept.
            combat::queue_next_swing(m, g, STRIKE);
            m.combat_mut(g).unwrap().victim = brute;
            let before = health(m, brute);
            combat::FORCED_OUTCOME.with(|f| f.set(Some(Outcome::Hit)));
            if let Some(f) = m.unit_fields_mut(g) {
                f.set_f32(UNIT_FIELD_MINDAMAGE, 7.0);
                f.set_f32(UNIT_FIELD_MAXDAMAGE, 7.0);
            }
            combat::try_swing(m, g);
            combat::FORCED_OUTCOME.with(|f| f.set(None));
            let c = m.combat(g).unwrap();
            (
                waiting,
                before - health(m, brute),
                c.extra_attacks,
                c.queued,
                c.next_swing == Some(now + Duration::from_secs(5)),
                c.next_offhand == Some(now + Duration::from_secs(9)),
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, 2);
    // Two blows of seven (before armor, of which it has none).
    assert_eq!(r.1, 14);
    assert_eq!(r.2, 0);
    assert_eq!(r.3, Some(STRIKE));
    assert!(r.4 && r.5, "the timers aren't touched");
}

#[tokio::test]
async fn sheathing_ignores_unknown_states_and_breaks_only_the_channels_it_should() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let state = |m: &Map| m.players[&ga].fields.get_byte(UNIT_FIELD_BYTES_2, 0);
            let before = state(m);
            combat::sheathe(m, ga, 3);
            let unknown = state(m) == before;
            combat::sheathe(m, ga, 2);
            let ranged = state(m);
            let mut channels = Vec::new();
            for id in [CHANNEL_BROKEN, CHANNEL_KEPT] {
                m.players.get_mut(&ga).unwrap().spells.push(id);
                let started = crate::game::map::spells::cast_now(
                    m,
                    ga,
                    id,
                    crate::game::map::spells::Targets {
                        unit: Some(ga),
                        ..Default::default()
                    },
                );
                let channeling = |m: &Map| {
                    m.caster(ga)
                        .and_then(|c| c.casting.as_ref())
                        .is_some_and(|c| c.spell == id && c.channel_until.is_some())
                };
                let was = channeling(m);
                combat::sheathe(m, ga, 1);
                channels.push((started, was, channeling(m)));
                crate::game::map::spells::interrupt(m, ga);
            }
            (unknown, ranged, channels)
        })
        .await
        .unwrap();
    assert!(r.0);
    assert_eq!(r.1, 2);
    assert_eq!(r.2, vec![(true, true, false), (true, true, true)]);
}

/// `g` at 60 with `skill` at one of its 300.
fn untrained(m: &mut Map, g: Guid, skill: u32) {
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::skills::learn_skill(p, skill, 1);
    crate::game::map::skills::set_value(p, skill, 1);
}

fn skill(m: &Map, g: Guid, skill: u32) -> u32 {
    crate::game::map::skills::value(&m.players[&g], skill).unwrap_or(0)
}

#[tokio::test]
async fn a_white_swing_trains_its_weapon_once_except_on_the_immune_or_a_players_side() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            equip(m, ga, SWORD, 15);
            untrained(m, ga, 43);
            let brute = spawn(m, ga, BRUTE, 60, 1.0);
            let mut out = Vec::new();
            // Any outcome but immunity trains, once a swing.
            for o in [Outcome::Hit, Outcome::Miss, Outcome::Dodge, Outcome::Parry] {
                let before = skill(m, ga, 43);
                forced_swing(m, ga, brute, o);
                out.push(skill(m, ga, 43) - before);
            }
            // Immune to its school: nothing.
            aura_of(
                m,
                brute,
                90_200,
                crate::game::map::auras::AURA_SCHOOL_IMMUNITY,
                1,
                1,
            );
            let before = skill(m, ga, 43);
            forced_swing(m, ga, brute, Outcome::Hit);
            out.push(skill(m, ga, 43) - before);
            // A player's pet trains no one.
            let pet = spawn(m, ga, BRUTE, 60, 1.0);
            m.creature_mut(pet).unwrap().owner = Some(crate::game::guid::player(9_999));
            let before = skill(m, ga, 43);
            forced_swing(m, ga, pet, Outcome::Hit);
            out.push(skill(m, ga, 43) - before);
            out
        })
        .await
        .unwrap();
    assert_eq!(r, vec![1, 1, 1, 1, 0, 0]);
}

#[tokio::test]
async fn a_creatures_blow_trains_a_players_defense_and_shows_at_once_in_their_avoidance() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            untrained(m, ga, crate::game::map::derive::SKILL_DEFENSE);
            let data = m.realm.data.clone();
            m.players.get_mut(&ga).unwrap().apply_stats(&data);
            let brute = spawn(m, ga, BRUTE, 60, 1.0);
            let f = m.unit_fields_mut(brute).unwrap();
            f.set_f32(UNIT_FIELD_MINDAMAGE, 1.0);
            f.set_f32(UNIT_FIELD_MAXDAMAGE, 1.0);
            // The defense the table and the avoidance derive from.
            let dodge = |m: &Map| m.players[&ga].derived.defense;
            let before = (skill(m, ga, 95), dodge(m));
            forced_swing(m, brute, ga, Outcome::Parry);
            let after = (skill(m, ga, 95), dodge(m));
            // A pet's blows train nothing.
            let pet = spawn(m, ga, BRUTE, 60, 1.0);
            m.creature_mut(pet).unwrap().owner = Some(crate::game::guid::player(9_999));
            forced_swing(m, pet, ga, Outcome::Hit);
            (before, after, skill(m, ga, 95))
        })
        .await
        .unwrap();
    assert_eq!(r.0.0 + 1, r.1.0);
    assert!(r.1.1 > r.0.1, "{r:?}");
    assert_eq!(r.2, r.1.0);
}

#[tokio::test]
async fn combo_points_go_with_a_new_target_with_their_targets_death_and_their_holders() {
    let (_realm, _a, ga, map) = setup(Class::Rogue).await;
    let r = map
        .query(move |m| {
            use crate::game::map::spells::add_combo_points;
            let one = spawn(m, ga, BRUTE, 20, 1.0);
            let two = spawn(m, ga, BRUTE, 20, 1.0);
            let points = |m: &Map| {
                let p = &m.players[&ga];
                (
                    p.caster.combo_points,
                    p.caster.combo_target,
                    p.fields.get_byte(PLAYER_FIELD_BYTES, 1),
                )
            };
            add_combo_points(m, ga, one, 2);
            let first = points(m);
            // Another target: the old points go.
            add_combo_points(m, ga, two, 1);
            let moved = points(m);
            // The target dies: they go.
            combat::kill(m, ga, two);
            let target_died = points(m);
            // The rogue dies: theirs go.
            add_combo_points(m, ga, one, 3);
            combat::kill(m, one, ga);
            let own = points(m);
            (first, moved, target_died, own, one, two)
        })
        .await
        .unwrap();
    let (first, moved, target_died, own, one, two) = r;
    assert_eq!(first, (2, one, 2));
    assert_eq!(moved, (1, two, 1));
    assert_eq!(target_died, (0, Guid::zero(), 0));
    assert_eq!(own, (0, Guid::zero(), 0));
}

#[tokio::test]
async fn death_closes_a_players_reactive_windows_and_a_resurrection_doesnt_bring_them_back() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            // A hunter, for both windows.
            m.players.get_mut(&ga).unwrap().row.class = 3;
            let one = spawn(m, ga, BRUTE, 20, 1.0);
            let two = spawn(m, ga, BRUTE, 20, 1.0);
            combat::after_outcome(m, one, ga, Outcome::Dodge);
            combat::after_outcome(m, two, ga, Outcome::Parry);
            let state = |m: &Map| {
                m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_AURASTATE)
                    & (combat::AURA_STATE_DEFENSE | combat::AURA_STATE_HUNTER_PARRY)
            };
            let open = state(m);
            combat::kill(m, one, ga);
            let c = m.combat(ga).unwrap();
            let dead = (state(m), c.defense_window, c.hunter_parry_window);
            crate::game::map::death::resurrect(m, ga, 1.0);
            combat::sync_aura_state(m, ga);
            (open, dead, state(m))
        })
        .await
        .unwrap();
    let both = combat::AURA_STATE_DEFENSE | combat::AURA_STATE_HUNTER_PARRY;
    assert_eq!(r.0, both);
    assert_eq!(r.1, (0, None, None));
    assert_eq!(r.2, 0);
}
