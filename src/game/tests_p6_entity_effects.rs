//! Round 6's effect resolver tests, ported from Thistle Tea's
//! `test/game/world/entity/effect_resolver/*_test.exs`
//! (docs/parity/matrix.csv): a charge's swing delay, its refusals and its
//! one spline for all who see it; a spell's flight from its own caster; a
//! pet's kill in a battleground; Alterac Valley's bound spawns; a captured
//! animal's following; who a PvP contact flags.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::combat::{self, Contact, dist};
use crate::game::map::creature::Creature;
use crate::game::map::creature::Walk;
use crate::game::tests_parity_motion as motion;

/// Alice at the start and the wolf 20 yards east, Bob beside her to watch.
async fn charging() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Client,
    Guid,
    crate::game::map::MapHandle,
) {
    let mut d = motion::wolf_world();
    d.creature_spawns.get_mut(&0).unwrap()[0] =
        motion::spawn(80, motion::WOLF, motion::at(20.0, 0.0));
    let (realm, mut a, ga, map) = motion::enter(d).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(motion::START.0, motion::START.1 - 2.0, motion::START.2)
        .await;
    a.sees(motion::wolf()).await;
    b.sees(ga).await;
    (realm, a, b, ga, map)
}

/// A charge that doesn't attack (a helpful one, or one that ends the
/// fight) still puts its caster's swings off by the run, and swings at
/// nothing on arriving; its caster and those who see it are told the one
/// run, the same spline id, path and time (Thistle Tea's "nonattacking
/// charges still carry the swing delay without an arrival attack" and
/// "both owners receive the same movement and spline id broadcast to
/// observers").
#[tokio::test]
async fn a_charge_without_a_swing_still_waits_out_its_run_and_all_see_one_spline() {
    let (_realm, mut a, mut b, ga, map) = charging().await;
    let g = motion::wolf();
    let (attack, swing_in, spline) = map
        .query(move |m| {
            let now = m.now;
            crate::game::map::charge::start(m, ga, g, false);
            let p = &m.players[&ga];
            let c = p.charge.as_ref().expect("charging");
            (
                c.attack,
                [p.combat.next_swing, p.combat.next_offhand]
                    .map(|t| t.map(|t| t.duration_since(now))),
                c.spline_id,
            )
        })
        .await
        .unwrap();
    assert_eq!(attack, None, "no swing on arriving");
    for t in swing_in {
        assert!(
            t.is_some_and(|t| t >= Duration::from_millis(200 + 40 * 14)),
            "both hands put off by the run: {swing_in:?}"
        );
    }
    let mine = motion::next_move(&mut a, ga, |m| m.kind == 0).await;
    let seen = motion::next_move(&mut b, ga, |m| m.kind == 0).await;
    assert_eq!(mine.id, spline);
    assert_eq!(
        (seen.id, seen.duration, &seen.points),
        (mine.id, mine.duration, &mine.points),
        "Bob sees the run Alice is told"
    );
    tokio::time::sleep(Duration::from_millis(u64::from(mine.duration) + 300)).await;
    let (charging, attacking, there) = map
        .query(move |m| {
            let p = &m.players[&ga];
            (
                p.charge.is_some(),
                p.combat.attacking,
                dist(p.position(), motion::at(20.0, 0.0)),
            )
        })
        .await
        .unwrap();
    assert!(!charging && !attacking, "there, and not swinging");
    assert!(there < 6.0, "at the wolf: {there}");
}

/// A charge refuses a dead caster, a rooted one, one on a flight, one
/// aimed at itself or at no one there, and one already within reach: it
/// doesn't start, and its caster stays put (Thistle Tea's "charges reject
/// dead, rooted and taxi casters and targets in another world").
#[tokio::test]
async fn a_charge_refuses_the_dead_the_rooted_the_flying_and_no_target() {
    let (_realm, _a, _b, ga, map) = charging().await;
    let g = motion::wolf();
    let refused = map
        .query(move |m| {
            let mut refused = Vec::new();
            let mut try_it = |m: &mut Map, what: &'static str, target: Guid| {
                let from = m.players[&ga].position();
                crate::game::map::charge::start(m, ga, target, true);
                let p = &m.players[&ga];
                refused.push((what, p.charge.is_none() && dist(p.position(), from) < 0.01));
            };
            motion::apply(m, ga, ga, motion::ROOT);
            try_it(m, "rooted", g);
            crate::game::map::auras::remove_spell(m, ga, motion::ROOT, None);
            let here = m.players[&ga].position();
            let walk = Walk::new(vec![here, motion::at(0.0, 50.0)], 7.0, m.now, 1, true);
            m.players.get_mut(&ga).unwrap().flight = Some(crate::game::map::npc::Flight {
                walk,
                destination: 0,
                mount: 0,
            });
            try_it(m, "flying", g);
            m.players.get_mut(&ga).unwrap().flight = None;
            try_it(m, "itself", ga);
            try_it(m, "no one", guid::unit(motion::WOLF, 999));
            let near = motion::at(2.0, 0.0);
            let wolf_at = m.creature(g).unwrap().position;
            m.creature_mut(g).unwrap().position = near;
            try_it(m, "within reach", g);
            m.creature_mut(g).unwrap().position = wolf_at;
            crate::game::map::death::kill(m, ga);
            try_it(m, "dead", g);
            refused
        })
        .await
        .unwrap();
    for (what, it) in refused {
        assert!(it, "{what}: charged");
    }
}

/// A spell's flight is timed from where its own caster stands, whoever
/// that is (a creature, a totem), at five yards' worth at least, and a
/// target not in the map is reached at once (Thistle Tea's "foreign
/// launch and delivery use the actual caster's position" and "point-blank
/// missiles use the minimum flight distance").
#[tokio::test]
async fn a_spells_flight_is_timed_from_its_own_caster() {
    let (_realm, _a, _b, ga, map) = charging().await;
    let g = motion::wolf();
    let times = map
        .query(move |m| {
            let pos = |dx: f32| motion::at(dx, 0.0);
            m.creature_mut(g).unwrap().position = pos(40.0);
            let far = crate::game::map::spells::flight_ms(m, g, ga, 20.0);
            m.creature_mut(g).unwrap().position = pos(2.0);
            let near = crate::game::map::spells::flight_ms(m, g, ga, 20.0);
            let elsewhere = crate::game::map::spells::flight_ms(m, g, guid::player(9_999), 20.0);
            (far, near, elsewhere)
        })
        .await
        .unwrap();
    assert_eq!(times, (2_000, 250, 0));
}

/// A pet's kill in Warsong Gulch is its master's: his killing blow and
/// honorable kill (Thistle Tea's "captures pet ownership and dead
/// teammates near either their body or ghost"; the dead near by body or
/// corpse are tests_p3_world_bg's).
#[tokio::test]
async fn a_pets_kill_in_a_battleground_is_its_masters() {
    use crate::game::tests_p3_world_bg as bg;
    let realm = harness::realm_with(bg::gulch());
    let mut a = bg::player(&realm, "A", "Alice", Race::Human).await;
    let mut b = bg::player(&realm, "B", "Grom", Race::Orc).await;
    let map = bg::all_in(&realm, &mut [&mut a, &mut b]).await;
    let (alice, grom) = (a.guid, b.guid);
    for c in [&mut a, &mut b] {
        bg::put(&map, c, 1200.0, 1400.0, 340.0).await;
    }
    let scores = map
        .query(move |m| {
            let t = crate::game::data::CreatureTemplate {
                entry: 9_990,
                name: "Wolf".into(),
                faction: 2,
                ..Default::default()
            };
            let at = m.players[&grom].position();
            let mut c = Creature::new(guid::unit(9_990, 1), &t, 1, at, 0.0);
            c.owner = Some(grom);
            let pet = m.add_creature(c);
            combat::kill(m, pet, alice);
            [grom, alice].map(|g| {
                let s = bg::score(m, g);
                (s.killing_blows, s.honorable_kills, s.deaths)
            })
        })
        .await
        .unwrap();
    assert_eq!(scores, [(1, 1, 0), (0, 0, 1)], "Grom, Alice");
}

/// In Alterac Valley, a general's spawn killed by a player's pet counts for
/// the pet's master, and ends the battle; a copy of the general that is
/// no spawn of the valley's (a summon) counts for nothing (Thistle Tea's
/// "captures an objective incarnation and attributes a pet kill to its
/// admitted owner" and "ordinary world kills and summons bypass
/// battleground lookups").
#[tokio::test]
async fn alterac_counts_a_pets_kill_of_a_bound_spawn_for_its_master_and_no_summon() {
    use crate::game::tests_pvp as pvp;
    let realm = harness::realm_with(pvp::alterac());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = pvp::both_into(
        &realm,
        &mut a,
        &mut b,
        wow_world_messages::vanilla::Map::AlteracValley,
    )
    .await;
    let alice = a.guid;
    let general = map
        .query(|m| {
            m.creatures
                .iter()
                .find(|(_, c)| c.entry == 11946 && c.spawn_guid.is_some())
                .map(|(_, c)| (c.guid, c.position))
        })
        .await
        .flatten()
        .expect("Drek'Thar is in the valley");
    let summoned = map
        .query(move |m| {
            let t = m.realm.data.creature_template(11946).cloned().unwrap();
            let copy = m.add_creature(Creature::new(
                guid::unit(11946, 99_999),
                &t,
                60,
                general.1,
                0.0,
            ));
            combat::kill(m, alice, copy);
            let b = m.battleground.as_ref().unwrap();
            (b.playing(), b.scores[&alice].objectives[5])
        })
        .await
        .unwrap();
    assert_eq!(summoned, (true, 0), "a summoned Drek'Thar is no general");
    map.query(move |m| {
        let t = crate::game::data::CreatureTemplate {
            entry: 9_990,
            name: "Wolf".into(),
            faction: 1,
            ..Default::default()
        };
        let mut c = Creature::new(guid::unit(9_990, 1), &t, 60, general.1, 0.0);
        c.owner = Some(alice);
        let pet = m.add_creature(c);
        combat::kill(m, pet, general.0);
    })
    .await;
    assert_eq!(pvp::winner(&mut a).await, BattlegroundWinner::Alliance);
    let leaders = map
        .query(move |m| m.battleground.as_ref().unwrap().scores[&alice].objectives[5])
        .await
        .unwrap();
    assert_eq!(leaders, 1, "the pet's kill is Alice's");
}

/// A captured animal that has stopped following is set to follow its
/// captor again at the aura's look, unless it is fighting or going home
/// (Thistle Tea's "restores lost following without interrupting combat,
/// evade or another player's request"; the aura is its captor's, so no
/// one else's look moves it).
#[tokio::test]
async fn a_captured_animal_follows_again_unless_it_fights_or_evades() {
    use crate::game::tests_p4_items_quests2 as q;
    let (_realm, _a, g, map) = q::captured().await;
    let follows = |m: &Map| {
        crate::game::map::event_ai::scripted(m, q::wolf())
            .and_then(|s| s.follow)
            .map(|(whom, _, _)| whom)
    };
    for busy in ["fighting", "evading"] {
        map.query(move |m| {
            let c = m.creature_mut(q::wolf()).unwrap();
            match busy {
                "fighting" => c.combat.attacking = true,
                _ => c.evading = true,
            }
            q::look_now(m, g);
        })
        .await;
        // Until it has looked: its next look two seconds off.
        for _ in 0..200 {
            let next = map.query(move |m| q::next_look(m, g)).await.flatten();
            if next.is_some_and(|t| t > Duration::from_millis(1000)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let following = map.query(move |m| follows(m)).await.flatten();
        assert_eq!(following, None, "{busy}: not set following");
        map.query(move |m| {
            let c = m.creature_mut(q::wolf()).unwrap();
            c.combat.attacking = false;
            c.evading = false;
        })
        .await;
    }
    map.query(move |m| q::look_now(m, g)).await;
    for _ in 0..200 {
        if map.query(move |m| follows(m)).await.flatten() == Some(g) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never set following again");
}

/// Touching an unflagged creature, or helping oneself, flags no one,
/// flagged or not: a blow on a wolf, a buff on oneself (Thistle Tea's
/// "ordinary PvE and self assistance produce no PvP notifications"; a
/// flagged creature's are tests_p3_core_pvp's).
#[tokio::test]
async fn attacking_a_plain_creature_or_helping_oneself_flags_no_one() {
    let (_realm, _a, _b, ga, map) = charging().await;
    let g = motion::wolf();
    let flagged = map
        .query(move |m| {
            let flagged = |m: &Map| {
                m.players[&ga]
                    .fields
                    .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP)
            };
            combat::engage(m, ga, g);
            combat::pvp_contact(m, ga, g, Contact::Attack);
            let after_wolf = flagged(m);
            m.players.get_mut(&ga).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                true,
            );
            combat::pvp_contact(m, ga, ga, Contact::Assist);
            (after_wolf, m.players[&ga].combat.last_pvp.is_some())
        })
        .await
        .unwrap();
    assert_eq!(flagged, (false, false), "the wolf, and herself");
}

// Who a spell's contact flags for PvP.

const SAP: u32 = 9_801;
const HEAL: u32 = 9_802;
const QUIET_BUFF: u32 = 9_803;
const QUIET_HEAL: u32 = 9_804;
const MARK: u32 = 9_805;

fn pvp_data() -> crate::game::data::GameData {
    use crate::game::data::spells::{ATTR_EX_NO_THREAT, SpellEffect};
    use crate::game::tests_pets::{data, effect, spell};
    let mut d = data();
    let none = SpellEffect::default();
    let mut stun = effect(6, 0, 6);
    stun.aura = crate::game::map::auras::AURA_MOD_STUN;
    let mut sap = spell(SAP, "Sap", [stun, none, none]);
    sap.family_name = 8;
    sap.family_flags = 0x80;
    sap.duration_index = 4;
    let heal = spell(HEAL, "Heal", [effect(10, 9, 21), none, none]);
    let mut quiet_heal = spell(QUIET_HEAL, "Quiet Heal", [effect(10, 9, 21), none, none]);
    quiet_heal.attributes[1] |= ATTR_EX_NO_THREAT;
    let mut ward = effect(6, 9, 21);
    ward.aura = 22;
    ward.misc_value = 4;
    let mut quiet_buff = spell(QUIET_BUFF, "Quiet Ward", [ward, none, none]);
    quiet_buff.attributes[1] |= ATTR_EX_NO_THREAT;
    quiet_buff.duration_index = 4;
    // A harmful mark with no damage and no threat: Hunter's Mark's kind.
    let mut mark = effect(6, 0, 6);
    mark.aura = 4;
    let mut mark = spell(MARK, "Mark", [mark, none, none]);
    mark.attributes[1] |= ATTR_EX_NO_THREAT;
    mark.duration_index = 4;
    for s in [sap, heal, quiet_heal, quiet_buff, mark] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn pvp_flagged(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP))
}

fn fighting(m: &Map, g: Guid) -> bool {
    m.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
}

fn contested(m: &Map, g: Guid) -> bool {
    m.players[&g].fields.has_flag(
        PLAYER_FLAGS,
        crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
    )
}

/// Any spell of a player's reaching another player's side flagged for PvP
/// flags its caster, as mangos-classic's `Spell::FillTargetMap` has it:
/// Sap (which starts no fight), a harmful mark of no threat, a heal; a
/// buff of no threat on one flagged, in a fight and contested, flags the
/// caster and no more; a heal that makes threat puts the healer in the
/// fight and contests them as the one healed is (`SetInCombatWithAssisted`),
/// a totem's owner standing for the totem. A flagged creature in a fight
/// flags a helper whose help makes threat, not one whose help makes none
/// (Thistle Tea's `spell_contacts`).
#[tokio::test]
async fn a_spell_on_a_flagged_players_side_flags_its_caster_as_mangos_has_it() {
    use crate::game::map::spells::cast_triggered;
    use crate::game::tests_pets::{WOLF, enter};
    let realm = harness::realm_with(pvp_data());
    let (_a, alice, map) = enter(&realm, "A", "Alice", Race::Human, Class::Rogue).await;
    let (_b, bob, _) = enter(&realm, "B", "Bob", Race::Human, Class::Priest).await;
    let (_g, grom, _) = enter(&realm, "G", "Grom", Race::Undead, Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let reset = |m: &mut Map| {
                for g in [alice, bob, grom] {
                    let p = m.players.get_mut(&g).unwrap();
                    p.pvp = Default::default();
                    p.combat = Default::default();
                    p.fields.set_flag(
                        UNIT_FIELD_FLAGS,
                        crate::game::map::player::UNIT_FLAG_PVP,
                        false,
                    );
                    p.fields.set_flag(
                        UNIT_FIELD_FLAGS,
                        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                        false,
                    );
                    p.fields.set_flag(
                        PLAYER_FLAGS,
                        crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
                        false,
                    );
                    crate::game::map::auras::remove_spell(m, g, SAP, None);
                    crate::game::map::auras::remove_spell(m, g, QUIET_BUFF, None);
                    crate::game::map::auras::remove_spell(m, g, MARK, None);
                }
            };
            let hot = |m: &mut Map, g: Guid| {
                let f = m.unit_fields_mut(g).unwrap();
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_PVP,
                    true,
                );
                f.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    true,
                );
                if let Some(p) = m.players.get_mut(&g) {
                    p.fields.set_flag(
                        PLAYER_FLAGS,
                        crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
                        true,
                    );
                }
            };
            let mut seen = Vec::new();
            // Sap on flagged Grom: Alice flagged, and no one fights.
            reset(m);
            m.unit_fields_mut(grom).unwrap().set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                true,
            );
            cast_triggered(m, alice, SAP, Some(grom));
            seen.push((
                "sap",
                pvp_flagged(m, alice),
                fighting(m, alice) || fighting(m, grom),
                contested(m, alice),
            ));
            // A mark of no threat on him: flagged too.
            reset(m);
            m.unit_fields_mut(grom).unwrap().set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                true,
            );
            cast_triggered(m, alice, MARK, Some(grom));
            seen.push(("mark", pvp_flagged(m, alice), fighting(m, alice), false));
            // Bob flagged, fighting and contested: a quiet ward flags her,
            // no more; a heal puts her in his fight, contested.
            reset(m);
            hot(m, bob);
            cast_triggered(m, alice, QUIET_BUFF, Some(bob));
            seen.push((
                "quiet ward",
                pvp_flagged(m, alice),
                fighting(m, alice),
                contested(m, alice),
            ));
            reset(m);
            hot(m, bob);
            cast_triggered(m, alice, HEAL, Some(bob));
            seen.push((
                "heal",
                pvp_flagged(m, alice),
                fighting(m, alice),
                contested(m, alice),
            ));
            // Bob's totem, in his fight: healing it is helping him.
            reset(m);
            hot(m, bob);
            let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
            let at = m.players[&bob].position();
            let mut totem = Creature::new(guid::unit(WOLF, 77), &t, 1, at, 0.0);
            totem.owner = Some(bob);
            let totem = m.add_creature(totem);
            hot(m, totem);
            cast_triggered(m, alice, HEAL, Some(totem));
            seen.push((
                "his totem",
                pvp_flagged(m, alice),
                fighting(m, alice),
                contested(m, alice),
            ));
            // A flagged wolf in a fight: a quiet heal flags no one, a heal
            // that makes threat does.
            reset(m);
            let wolf = Creature::new(guid::unit(WOLF, 78), &t, 1, at, 0.0);
            let wolf = m.add_creature(wolf);
            hot(m, wolf);
            cast_triggered(m, alice, QUIET_HEAL, Some(wolf));
            let quiet = pvp_flagged(m, alice);
            cast_triggered(m, alice, HEAL, Some(wolf));
            seen.push(("flagged wolf", quiet, pvp_flagged(m, alice), false));
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        seen,
        vec![
            ("sap", true, false, false),
            ("mark", true, false, false),
            ("quiet ward", true, false, false),
            ("heal", true, true, true),
            ("his totem", true, true, true),
            ("flagged wolf", false, true, false),
        ]
    );
}

/// A pet's blow on a flagged player is its master's: he is flagged,
/// contested and in the fight; the one struck is flagged by it, not
/// contested. A possessed player's blow flags the one who holds them
/// (Thistle Tea's "resolves a pet attack to both players and applies the
/// owner's flags" and "a possessed player's combat flags the controller";
/// mangos's `SetInCombatWithVictim` by the beneficiary player).
#[tokio::test]
async fn a_pets_or_a_puppets_blow_flags_its_master() {
    use crate::game::tests_pets::{WOLF, enter};
    let realm = harness::realm_with(pvp_data());
    let (_a, alice, map) = enter(&realm, "A", "Alice", Race::Human, Class::Warrior).await;
    let (_b, bob, _) = enter(&realm, "B", "Bob", Race::Human, Class::Priest).await;
    let (_g, grom, _) = enter(&realm, "G", "Grom", Race::Undead, Class::Warrior).await;
    let (pet_blow, puppet_blow) = map
        .query(move |m| {
            let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
            let at = m.players[&alice].position();
            let mut pet = Creature::new(guid::unit(WOLF, 79), &t, 1, at, 0.0);
            pet.owner = Some(alice);
            let pet = m.add_creature(pet);
            m.unit_fields_mut(grom).unwrap().set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                true,
            );
            combat::engage(m, pet, grom);
            combat::deal_damage(m, pet, grom, 1);
            let pet_blow = (
                pvp_flagged(m, alice),
                contested(m, alice),
                fighting(m, alice),
                pvp_flagged(m, grom),
                contested(m, grom),
            );
            // Bob, held by Grom, strikes flagged Alice: Grom's blow.
            for g in [grom, alice] {
                let p = m.players.get_mut(&g).unwrap();
                p.combat = Default::default();
                p.pvp = Default::default();
                p.fields.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_PVP,
                    g == alice,
                );
            }
            let now = m.now;
            m.players.get_mut(&bob).unwrap().owned.possessed_by =
                Some(crate::game::map::pets::possess::Possession {
                    master: grom,
                    spell: 605,
                    was_faction: 1,
                    manual: true,
                    next_think: now,
                    next_cast_at: now,
                    melee_fallback: false,
                });
            combat::engage(m, bob, alice);
            combat::deal_damage(m, bob, alice, 1);
            (pet_blow, (pvp_flagged(m, grom), fighting(m, grom)))
        })
        .await
        .unwrap();
    assert_eq!(pet_blow, (true, true, true, true, false), "Alice's pet");
    assert_eq!(puppet_blow, (true, true), "Grom's puppet");
}

/// A shared meteor splits its base among those it falls on before the
/// caster's spell power is added to each share, and each target's own
/// mitigation then takes its part of its share: one fallen on alone
/// takes 202 more than each of three, with the bonus or without; one
/// taking half fire damage takes half its share (Thistle Tea's "splits
/// the base before outgoing bonuses and target mitigation").
#[tokio::test]
async fn a_shared_meteors_base_is_split_before_bonuses_and_mitigation() {
    use crate::game::map::auras::{self, AURA_MOD_DAMAGE_DONE, AURA_MOD_DAMAGE_PERCENT_TAKEN};
    use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
    use crate::game::tests_p3_entity::{aura_effect, spell};
    use crate::game::tests_p3_entity_effects as fx;
    const FIRE_POWER: u32 = 99_810;
    const HALF_FIRE: u32 = 99_811;
    let mut d = fx::world();
    // A magic spell, as a meteor is: spell power counts for it.
    let meteor = d.spells.spells.get_mut(&fx::METEOR).unwrap();
    meteor.dmg_class = 1;
    meteor.effects[0].bonus_coefficient = 1.0;
    d.spells.spells.insert(
        FIRE_POWER,
        spell(
            FIRE_POWER,
            &[aura_effect(AURA_MOD_DAMAGE_DONE, 300, 1 << 2, 1)],
        ),
    );
    d.spells.spells.insert(
        HALF_FIRE,
        spell(
            HALF_FIRE,
            &[aura_effect(AURA_MOD_DAMAGE_PERCENT_TAKEN, -50, 1 << 2, 1)],
        ),
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Mage).await;
    let out = map
        .query(move |m| {
            let trio: Vec<Guid> = (0..3)
                .map(|i| spawn(m, ga, BRUTE, 20, 10.0 + i as f32))
                .collect();
            let lone = spawn(m, ga, BRUTE, 20, 40.0);
            let (at_trio, at_lone) = (
                m.position_of(trio[1]).unwrap(),
                m.position_of(lone).unwrap(),
            );
            let fall = |m: &mut Map, at| {
                let mut all = trio.clone();
                all.push(lone);
                fx::heal(m, &all);
                crate::game::map::spells::cast_triggered_with(
                    m,
                    ga,
                    fx::METEOR,
                    crate::game::map::spells::Targets {
                        dest: Some(at),
                        ..Default::default()
                    },
                );
                (
                    trio.iter().map(|g| fx::lost(m, *g)).collect::<Vec<_>>(),
                    fx::lost(m, lone),
                )
            };
            let plain = (fall(m, at_trio).0, fall(m, at_lone).1);
            let power = m.realm.data.spells.get(FIRE_POWER).cloned().unwrap();
            auras::apply(m, ga, ga, &power, 0, 300);
            let powered = (fall(m, at_trio).0, fall(m, at_lone).1);
            let half = m.realm.data.spells.get(HALF_FIRE).cloned().unwrap();
            auras::apply(m, trio[0], trio[0], &half, 0, -50);
            let halved = fall(m, at_trio).0;
            (plain, powered, halved)
        })
        .await
        .unwrap();
    let ((plain3, plain1), (pow3, pow1), halved) = out;
    assert_eq!((plain3.clone(), plain1), (vec![100, 100, 100], 302));
    assert_eq!(
        pow3[0], 400,
        "a third of the base, and the bonus whole: {pow3:?}"
    );
    assert!(pow3.iter().all(|d| *d == pow3[0]), "{pow3:?}");
    assert_eq!(pow1 - pow3[0], 202, "the bonus added to each share whole");
    assert_eq!(halved[1], pow3[0]);
    assert!(
        halved[0].abs_diff(pow3[0] / 2) <= 1,
        "half the share: {halved:?} of {pow3:?}"
    );
}
