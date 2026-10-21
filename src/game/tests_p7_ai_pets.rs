//! Round 7's creature AI tests of pets, guardians and companions, ported
//! from Thistle Tea's `test/game/core/ai/bt/pet_test.exs`,
//! `bt/pet/return_test.exs`, `bt/mob/spells_test.exs`, `bt/mob_test.exs`,
//! `bt/guardian_test.exs`, `bt/creature_pet_test.exs`,
//! `bt/mini_pet_test.exs` and `bt/acquisition_test.exs`
//! (docs/parity/matrix.csv): a pet's cast before its walk, its place by a
//! master who stands, its stances' stop, its autocast, the end of its
//! fight and its death, and how long summons last.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::creature::Motion;
use crate::game::map::pets::{self, ACT_COMMAND, ACT_REACTION, Kind};
use crate::game::map::{Map, ai, combat};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{BOAR, WOLF, effect, place, spell};

/// Two self buffs that take two seconds; one for a fight (it raises
/// damage), instant.
const BUFF_A: u32 = 95_320;
const BUFF_B: u32 = 95_321;
const FURY: u32 = 95_322;
/// A guardian barred from swinging, and one not.
const STILL: u32 = 3_790;
const SWINGER: u32 = 3_791;

fn world() -> GameData {
    let mut d = crate::game::tests_pets::data();
    d.spells.cast_times.insert(98, 2_000);
    for id in [BUFF_A, BUFF_B] {
        let mut s = spell(
            id,
            "Buff",
            [
                SpellEffect {
                    aura: 29,
                    ..effect(6, 4, 1)
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        );
        s.casting_time_index = 98;
        d.spells.spells.insert(id, s);
    }
    d.spells.spells.insert(
        FURY,
        spell(
            FURY,
            "Fury",
            [
                SpellEffect {
                    aura: 13,
                    misc_value: 1,
                    ..effect(6, 9, 1)
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        ),
    );
    let mut still = crate::game::tests_pets::template(STILL, "Still", 35, 1);
    let swinger = still.clone();
    still.static_flags1 = 0x0010_0000;
    d.creature_templates.insert(STILL, still);
    d.creature_templates.insert(
        SWINGER,
        CreatureTemplate {
            entry: SWINGER,
            ..swinger
        },
    );
    d
}

use crate::game::data::spells::SpellEffect;

/// The pets take `n` turns a second apart, their casts with them.
fn think(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_secs(1);
        crate::game::map::spells::update(m);
        pets::update(m);
    }
}

fn moving(m: &Map, g: Guid) -> bool {
    matches!(m.creature(g).unwrap().motion, Motion::Moving(_))
}

fn casting(m: &Map, g: Guid) -> Option<u32> {
    m.caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| c.spell)
}

fn order(m: &mut Map, owner: Guid, pet: Guid, data: u32, target: Guid) {
    pets::handle(
        m,
        owner,
        ClientOpcodeMessage::CMSG_PET_ACTION(CMSG_PET_ACTION { pet, data, target }),
    );
}

/// The owner `g` `east` yards on from where she is, standing.
fn step_east(m: &mut Map, g: Guid, east: f32) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position.x += east;
    p.movement.flags = MovementInfo_MovementFlags::empty();
}

/// Thistle Tea's pet tree "an idle pet finishes preparing its self buff
/// before following", `PetReturn` "a newly commanded spell advances before
/// return movement", and `follow_owner` "stays put while a stationary
/// owner turns in place": a pet casting (its own buff, or a spell its
/// master told it on its way back) finishes it where it stands, then goes
/// after its master; by a master who stands it stays, however she turns.
#[tokio::test]
async fn a_pet_finishes_its_cast_before_it_follows_and_stays_by_a_master_who_turns() {
    let (_realm, _h, gh, map, pet, _wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            pets::state_mut(m, pet)
                .unwrap()
                .spells
                .extend([BUFF_A, BUFF_B]);
            think(m, 3);
            pets::state_mut(m, pet).unwrap().autocast = vec![BUFF_A];
            step_east(m, gh, 15.0);
            think(m, 1);
            let buffing = (casting(m, pet), moving(m, pet));
            think(m, 2);
            let after = (casting(m, pet), moving(m, pet));
            // Called back, told to cast on the way: the cast first.
            pets::state_mut(m, pet).unwrap().autocast.clear();
            think(m, 10);
            step_east(m, gh, 15.0);
            order(
                m,
                gh,
                pet,
                pets::button(u32::from(pets::COMMAND_FOLLOW), ACT_COMMAND),
                Guid::zero(),
            );
            order(m, gh, pet, BUFF_B, pet);
            think(m, 1);
            let recalled = (casting(m, pet), moving(m, pet));
            think(m, 10);
            // Beside her, she turns about.
            let here = m.position_of(pet).unwrap();
            let settled = moving(m, pet);
            m.players.get_mut(&gh).unwrap().movement.orientation += std::f32::consts::PI;
            think(m, 3);
            let turned = (moving(m, pet), m.position_of(pet).unwrap() == here);
            (buffing, after, recalled, settled, turned)
        })
        .await
        .unwrap();
    let (buffing, after, recalled, settled, turned) = out;
    assert_eq!(buffing, (Some(BUFF_A), false), "its buff where it stands");
    assert_eq!(after, (None, true), "then after her");
    assert_eq!(recalled, (Some(BUFF_B), false), "the cast first");
    assert!(!settled, "beside her");
    assert_eq!(turned, (false, true), "she turns, it stays");
}

/// Thistle Tea's pet tree "a guardian that cannot melee holds beside its
/// victim instead of swinging": a guardian whose template bars its swing
/// (`static_flags1` 0x00100000) stands by its victim and doesn't swing;
/// another swings.
#[tokio::test]
async fn a_guardian_barred_from_its_swing_holds_beside_its_victim() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let other = place(&map, gh, WOLF, 0x70_0020, 14.0).await;
    let out = map
        .query(move |m| {
            // The boar away: the guardians alone strike.
            pets::dismiss(m, pet);
            let mut swung = Vec::new();
            for (entry, victim) in [(STILL, wolf), (SWINGER, other)] {
                let v = m.position_of(victim).unwrap();
                let at = Vector3d { x: v.x - 1.5, ..v };
                let g = pets::testing::spawn(m, gh, entry, Kind::Guardian, 10, at, 0.0);
                let faction = m.players[&gh].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE);
                m.creature_mut(g)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_FACTIONTEMPLATE, faction);
                let f = m.unit_fields_mut(victim).unwrap();
                f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
                pets::attack_target(m, g, victim);
                think(m, 3);
                let health = m.unit_fields(victim).unwrap().get_u32(UNIT_FIELD_HEALTH);
                swung.push(health < 1_000_000);
            }
            swung
        })
        .await
        .unwrap();
    assert_eq!(out, [false, true], "the barred one holds; the other swings");
}

/// Thistle Tea's `command` "stay stops the active spline through the
/// movement transition" and `reaction` "passive stops the active spline
/// through the movement transition": told to stay, or made passive, a pet
/// on its way stops there.
#[tokio::test]
async fn stay_and_passive_stop_a_pet_on_its_way() {
    let (_realm, _h, gh, map, pet, _wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            let mut out = Vec::new();
            for data in [
                pets::button(u32::from(pets::COMMAND_STAY), ACT_COMMAND),
                pets::button(u32::from(pets::REACT_PASSIVE), ACT_REACTION),
            ] {
                let s = pets::state_mut(m, pet).unwrap();
                s.command = pets::COMMAND_FOLLOW;
                s.stay_at = None;
                step_east(m, gh, 20.0);
                think(m, 1);
                let walking = moving(m, pet);
                order(m, gh, pet, data, Guid::zero());
                out.push((walking, moving(m, pet)));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(out, [(true, false), (true, false)], "each stops it");
}

/// Thistle Tea's `drop_threat` "pets keep their auras and health when the
/// final opponent leaves combat", and `PetReturn.enter` "death clears
/// return progress and explicit Attack": a pet whose last foe goes (a
/// Vanish, a feign) leaves the fight where it stands, its health and its
/// auras as they were, its master's Attack over, and goes home to no
/// spawn; a pet's death ends its recall and the Attack it was sent on.
#[tokio::test]
async fn a_pets_last_foe_gone_leaves_it_whole_where_it_stands_and_its_death_ends_its_orders() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            order(
                m,
                gh,
                pet,
                pets::button(u32::from(pets::COMMAND_ATTACK), ACT_COMMAND),
                wolf,
            );
            crate::game::tests_p3_combat::aura_of(m, pet, 95_323, 29, 0, 5);
            let c = m.creature_mut(pet).unwrap();
            c.threat.push((wolf, 100.0));
            let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
            c.fields.set_u32(UNIT_FIELD_HEALTH, max * 2 / 5);
            c.fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                true,
            );
            let health = c.fields.get_u32(UNIT_FIELD_HEALTH);
            let here = c.position;
            ai::forget(m, wolf);
            let c = m.creature(pet).unwrap();
            let left = (
                c.combat.attacking,
                c.threat.is_empty(),
                c.fields.get_guid(UNIT_FIELD_TARGET).is_zero(),
                c.fields.get_u32(UNIT_FIELD_HEALTH) == health,
                c.auras.has_spell(95_323),
                c.fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                ),
                c.evading,
                c.position == here && !moving(m, pet),
                pets::state(m, pet).unwrap().commanded,
            );
            // Recalled, sent at the wolf, it dies.
            let now = m.now;
            let s = pets::state_mut(m, pet).unwrap();
            s.returning = true;
            s.commanded = Some(wolf);
            s.commanded_at = Some(now);
            pets::testing::die(m, pet);
            let s = pets::state(m, pet).unwrap();
            (left, (s.returning, s.commanded, s.commanded_at))
        })
        .await
        .unwrap();
    let (left, died) = out;
    assert_eq!(
        left,
        (false, true, true, true, true, false, false, true, None),
        "out of the fight, whole where it stands"
    );
    assert_eq!(died, (false, None, None), "its orders end with it");
}

/// Thistle Tea's `try_cast` "idle pets skip unavailable self buffs and
/// cast the next eligible spell" and "manual commands can cast a buff that
/// autocast reserves for combat": an idle pet passes over a buff it can't
/// cast now (its cooldown) for the next; a buff for a fight it doesn't
/// cast of its own accord out of one, but its master may tell it to.
#[tokio::test]
async fn an_idle_pet_casts_the_next_buff_it_can_and_its_master_may_order_one_for_a_fight() {
    let (_realm, _h, gh, map, pet, _wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            let s = pets::state_mut(m, pet).unwrap();
            s.spells.extend([BUFF_A, BUFF_B, FURY]);
            s.autocast = vec![BUFF_A, BUFF_B];
            let later = m.now + Duration::from_secs(600);
            m.creature_mut(pet)
                .unwrap()
                .caster
                .cooldowns
                .insert(BUFF_A, later);
            think(m, 1);
            let next = casting(m, pet);
            think(m, 3);
            // A buff for a fight: not of its own accord.
            pets::state_mut(m, pet).unwrap().autocast = vec![FURY];
            think(m, 3);
            let own = m.creature(pet).unwrap().auras.has_spell(FURY);
            order(m, gh, pet, FURY, pet);
            let told = m.creature(pet).unwrap().auras.has_spell(FURY);
            (next, own, told)
        })
        .await
        .unwrap();
    assert_eq!(out, (Some(BUFF_B), false, true));
}

/// Thistle Tea's `Guardian.lifetime` "duration expiry removes live
/// guardians but preserves corpses", `CreaturePet.lifetime` "owner death
/// retains fighting pets and corpses but removes idle pets", and the
/// companion's tree "follows behind its owner without combat actions": a
/// guardian's time up, it goes, but its corpse lies its while; a dead
/// creature's guardian fighting fights on and its corpse lies, an idle
/// one goes; a companion walks behind its owner and takes on no one.
#[tokio::test]
async fn summons_last_their_time_or_their_masters_and_a_companion_only_follows() {
    let (_realm, _h, gh, map, _pet, wolf) = tamed_with(world()).await;
    let master = place(&map, gh, WOLF, 0x70_0021, 6.0).await;
    let out = map
        .query(move |m| {
            let at = m.position_of(master).unwrap();
            let summon = |m: &mut Map, owner: Guid| {
                pets::testing::spawn(m, owner, WOLF, Kind::Guardian, 10, at, 0.0)
            };
            // Its time up: the living one goes, the corpse lies.
            let live = summon(m, gh);
            let dead = summon(m, gh);
            pets::testing::die(m, dead);
            let now = m.now;
            for g in [live, dead] {
                pets::state_mut(m, g).unwrap().expires = Some(now);
            }
            think(m, 1);
            let expiry = (m.creature(live).is_some(), m.creature(dead).is_some());
            // A creature's three: fighting, idle, dead; it dies.
            let fighting = summon(m, master);
            let idle = summon(m, master);
            let corpse = summon(m, master);
            m.creature_mut(fighting).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                true,
            );
            pets::testing::die(m, corpse);
            ai::die(m, master, gh);
            m.now += Duration::from_millis(500);
            pets::update(m);
            let orphans = [fighting, idle, corpse].map(|g| m.creature(g).is_some());
            // A companion behind her, the wolf at her.
            let here = m.players[&gh].position();
            let critter = pets::testing::spawn(m, gh, WOLF, Kind::Critter, 1, here, 0.0);
            let s = pets::state_mut(m, critter).unwrap();
            s.angle = std::f32::consts::PI;
            s.react = pets::REACT_PASSIVE;
            combat::start_attack(m, wolf, gh);
            combat::engage(m, wolf, gh);
            m.players.get_mut(&gh).unwrap().movement.orientation = 0.0;
            step_east(m, gh, 10.0);
            think(m, 1);
            let to = match &m.creature(critter).unwrap().motion {
                Motion::Moving(w) => Some(w.destination()),
                _ => None,
            };
            let p = m.players[&gh].position();
            let behind =
                to.is_some_and(|t| (t.x - (p.x - 2.0)).abs() < 0.1 && (t.y - p.y).abs() < 0.1);
            let fights = m.combat(critter).is_some_and(|c| c.attacking);
            (expiry, orphans, behind, fights)
        })
        .await
        .unwrap();
    let (expiry, orphans, behind, fights) = out;
    assert_eq!(
        expiry,
        (false, true),
        "the living one goes, the corpse lies"
    );
    assert_eq!(
        orphans,
        [true, false, true],
        "fighting and dead stay, idle goes"
    );
    assert!(behind, "the companion walks to two yards behind her");
    assert!(!fights, "and takes on no one");
}

/// Thistle Tea's `Acquisition.nearest` "breaks equal-distance ties by
/// guid": an aggressive guardian looking for a foe takes the lower guid
/// of two as near.
#[tokio::test]
async fn an_aggressive_guardian_takes_the_lower_guid_of_two_as_near() {
    let (_realm, _h, gh, map, _pet, _wolf) = tamed_with(world()).await;
    let high = place(&map, gh, WOLF, 0x70_0031, 8.0).await;
    let low = place(&map, gh, WOLF, 0x70_0030, 8.0).await;
    let out = map
        .query(move |m| {
            let p = m.players[&gh].position();
            // A boar's, which looks about it (detection range 20).
            let g = pets::testing::spawn(m, gh, BOAR, Kind::Guardian, 10, p, 0.0);
            pets::state_mut(m, g).unwrap().react = pets::REACT_AGGRESSIVE;
            think(m, 1);
            m.combat(g).filter(|c| c.attacking).map(|c| c.victim)
        })
        .await
        .unwrap();
    assert!(low.guid() < high.guid());
    assert_eq!(out, Some(low), "the lower guid");
}

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
}

/// A script's pet command (88), `datalong` the command, from `source` on
/// `target`, with the abort flag: whether its script would go on.
fn command(m: &mut Map, d1: u32, source: Guid, target: Guid) -> bool {
    use crate::game::data::scripts::{SF_ABORT_ON_FAILURE, ScriptStep};
    let st = ScriptStep {
        data_flags: SF_ABORT_ON_FAILURE,
        ..crate::game::tests_p3_entity::step(88, [d1, 0, 0, 0])
    };
    crate::game::map::scripts::execute(m, &st, source, target)
}

/// Thistle Tea's `set_command_state` "follow stops attacking and casting
/// without clearing the combat timer", "possessed stay and follow leave
/// client movement and combat in place", "explicit attacks override
/// passive reaction and reject invalid targets" and "non-creature sources
/// honor the abort flag": a script's Follow stops its pet's attack and
/// its cast and lets its foes go, its fight's flag kept; one its master
/// possesses takes no stay or follow; an Attack sends it in passive or
/// not, as its master's would, but at no one, no one dead, no one it may
/// not attack, and not while its master is pacified; and from one that
/// isn't a creature, the command fails.
#[tokio::test]
async fn a_scripts_pet_commands_stop_its_fight_send_it_in_or_fail() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let other = place(&map, gh, WOLF, 0x70_0040, 10.0).await;
    let out = map
        .query(move |m| {
            for w in [wolf, other] {
                let f = m.unit_fields_mut(w).unwrap();
                f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            }
            pets::state_mut(m, pet).unwrap().spells.push(BUFF_B);
            // Fighting, casting: Follow.
            pets::attack_target(m, pet, wolf);
            let c = m.creature_mut(pet).unwrap();
            c.threat.push((wolf, 10.0));
            c.fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                true,
            );
            crate::game::map::spells::cast(
                m,
                pet,
                BUFF_B,
                crate::game::map::spells::Targets {
                    unit: Some(pet),
                    ..Default::default()
                },
                None,
            );
            let before = (casting(m, pet), in_combat(m, pet));
            let ok = command(m, u32::from(pets::COMMAND_FOLLOW), pet, Guid::zero());
            let c = m.creature(pet).unwrap();
            let followed = (
                ok,
                c.combat.attacking,
                casting(m, pet),
                in_combat(m, pet),
                c.threat.is_empty(),
                pets::state(m, pet).unwrap().command,
            );
            // Possessed: neither stay nor follow takes.
            pets::attack_target(m, pet, wolf);
            pets::state_mut(m, pet).unwrap().kind = Kind::Possessed;
            let held = [pets::COMMAND_STAY, pets::COMMAND_FOLLOW].map(|d| {
                let ok = command(m, u32::from(d), pet, Guid::zero());
                (
                    ok,
                    m.combat(pet)
                        .is_some_and(|c| c.attacking && c.victim == wolf),
                )
            });
            pets::state_mut(m, pet).unwrap().kind = Kind::Hunter;
            // Passive, an Attack sends it in.
            command(m, u32::from(pets::COMMAND_FOLLOW), pet, Guid::zero());
            pets::state_mut(m, pet).unwrap().react = pets::REACT_PASSIVE;
            let sent = (
                command(m, u32::from(pets::COMMAND_ATTACK), pet, other),
                m.combat(pet)
                    .is_some_and(|c| c.attacking && c.victim == other),
                pets::state(m, pet).unwrap().commanded == Some(other),
            );
            // At no one; her pacified; one it may not attack; one dead.
            let attack = |m: &mut Map, t: Guid| {
                command(m, u32::from(pets::COMMAND_FOLLOW), pet, Guid::zero());
                command(m, u32::from(pets::COMMAND_ATTACK), pet, t)
            };
            let nobody = attack(m, Guid::zero());
            m.players
                .get_mut(&gh)
                .unwrap()
                .fields
                .set_flag(UNIT_FIELD_FLAGS, 0x2_0000, true);
            let pacified = attack(m, wolf);
            m.players
                .get_mut(&gh)
                .unwrap()
                .fields
                .set_flag(UNIT_FIELD_FLAGS, 0x2_0000, false);
            m.unit_fields_mut(wolf)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, 0x2, true);
            let untouchable = attack(m, wolf);
            ai::die(m, other, gh);
            let dead = attack(m, other);
            // From a player.
            let player = command(m, u32::from(pets::COMMAND_STAY), gh, Guid::zero());
            (
                before,
                followed,
                held,
                sent,
                [nobody, pacified, untouchable, dead],
                player,
            )
        })
        .await
        .unwrap();
    let (before, followed, held, sent, denied, player) = out;
    assert_eq!(before, (Some(BUFF_B), true));
    assert_eq!(
        followed,
        (true, false, None, true, true, pets::COMMAND_FOLLOW),
        "its attack and cast stop, its fight's flag stays"
    );
    assert_eq!(held, [(true, true); 2], "possessed: its fight goes on");
    assert_eq!(sent, (true, true, true), "passive, sent in");
    assert_eq!(denied, [false; 4], "no one, pacified, untouchable, dead");
    assert!(!player, "a player's fails");
}

const GROUP_SCRIPT: u32 = 9_570;

/// Thistle Tea's `remove_guardians` "removes matching guardians from
/// either unit owner without touching other companions", and the group's
/// script "selects one nested script for the whole group and preserves
/// its delays and target" and "a missed chance honors the abort flag": a
/// script's call (56) takes away a player's or a creature's guardians of
/// an entry, its other guardians and its companion left; one script, by
/// chance, runs for a creature and each of its group, its delays kept; a
/// chance missed fails the step.
#[tokio::test]
async fn a_scripts_guardians_go_by_entry_and_its_group_script_runs_for_all() {
    use crate::game::data::scripts::*;
    use crate::game::tests_p3_entity::step;
    let mut d = world();
    d.scripts.generic.insert(
        GROUP_SCRIPT,
        vec![
            step(28, [1, 0, 0, 0]),
            ScriptStep {
                delay_ms: 1_000,
                ..step(28, [8, 0, 0, 0])
            },
        ],
    );
    let (_realm, _h, gh, map, _pet, _wolf) = tamed_with(d).await;
    let master = place(&map, gh, WOLF, 0x70_0041, 6.0).await;
    let member = place(&map, gh, WOLF, 0x70_0042, 7.0).await;
    let out = map
        .query(move |m| {
            let at = m.position_of(master).unwrap();
            let summon = |m: &mut Map, owner: Guid, entry: u32, kind: Kind| {
                let g = pets::testing::spawn(m, owner, entry, kind, 10, at, 0.0);
                if kind == Kind::Guardian
                    && let Some(p) = m.players.get_mut(&owner)
                {
                    p.owned.guardians.push(g);
                }
                g
            };
            let hers = [
                summon(m, gh, STILL, Kind::Guardian),
                summon(m, gh, SWINGER, Kind::Guardian),
                summon(m, gh, STILL, Kind::Critter),
            ];
            let its = summon(m, master, STILL, Kind::Guardian);
            let remove = step(56, [STILL, 0, 0, 0]);
            crate::game::map::scripts::execute(m, &remove, gh, Guid::zero());
            let after_hers = (
                hers.map(|g| m.creature(g).is_some()),
                m.creature(its).is_some(),
            );
            crate::game::map::scripts::execute(m, &remove, master, Guid::zero());
            let after_its = m.creature(its).is_some();
            // The group's script.
            crate::game::map::creature_groups::join_with(m, member, master, 2.0, 0.0, 0);
            let stand = |m: &Map, g: Guid| {
                m.unit_fields(g)
                    .map_or(u8::MAX, |f| f.get_byte(UNIT_FIELD_BYTES_1, 0))
            };
            let group = ScriptStep {
                dataint: [100, 0, 0, 0],
                ..step(90, [GROUP_SCRIPT, 0, 0, 0])
            };
            let ran = crate::game::map::scripts::execute(m, &group, master, gh);
            let first = (stand(m, master), stand(m, member));
            for r in m.scripts.runs.iter_mut() {
                r.started -= Duration::from_millis(1_000);
            }
            crate::game::map::scripts::update(m);
            let later = (stand(m, master), stand(m, member));
            // A chance in a hundred: missed, the step fails.
            let mut honoured = true;
            let mut missed = 0;
            for _ in 0..30 {
                m.unit_fields_mut(master)
                    .unwrap()
                    .set_byte(UNIT_FIELD_BYTES_1, 0, 0);
                let slim = ScriptStep {
                    dataint: [1, 0, 0, 0],
                    data_flags: SF_ABORT_ON_FAILURE,
                    ..step(90, [GROUP_SCRIPT, 0, 0, 0])
                };
                let ok = crate::game::map::scripts::execute(m, &slim, master, gh);
                if stand(m, master) == 0 {
                    missed += 1;
                    honoured &= !ok;
                }
            }
            (after_hers, after_its, ran, first, later, missed, honoured)
        })
        .await
        .unwrap();
    let (after_hers, after_its, ran, first, later, missed, honoured) = out;
    assert_eq!(
        after_hers,
        ([false, true, true], true),
        "hers of the entry go; her other and her companion stay, its too"
    );
    assert!(!after_its, "a creature's go");
    assert!(ran);
    assert_eq!(first, (1, 1), "for both");
    assert_eq!(later, (8, 8), "its delay kept");
    assert!(missed > 0 && honoured, "a miss fails the step");
}
