//! Round 9's pet tests, ported from Thistle Tea's
//! `test/game/core/ai/bt/creature_pet_test.exs`, `pet/return_test.exs`,
//! `pet/target_selection_test.exs`, `pet_test.exs`, `totem_test.exs` and
//! `test/game/core/combat/engagement/pet_combat_test.exs`
//! (docs/parity/matrix.csv): a pet kept at the edge of its master's sight
//! and gone with its master, a stay spot walked to again after a walk that
//! fell short, the foes an aggressive pet takes, a totem's last pulses, and
//! a pet's fight held by its foes and ended by its death.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOD_STAT, AURA_PERIODIC_TRIGGER_SPELL};
use crate::game::map::creature::Motion;
use crate::game::map::pets::{self, ACT_COMMAND, ACT_REACTION, Kind};
use crate::game::map::{Map, ai, combat};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{WOLF, enter, place, template};

/// A creature no one is at war with.
const NEUTRAL: u32 = 2_001;
/// A ward's pulse, and the spell it sends.
const WARD: u32 = 95_985;
const WARD_PULSE: u32 = 95_986;

fn world() -> GameData {
    let mut d = crate::game::tests_pets::data();
    d.creature_templates
        .insert(NEUTRAL, template(NEUTRAL, "Neutral", 35, 1));
    let mut pulse = SpellTemplate {
        id: WARD_PULSE,
        name: "Pulse".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    pulse.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_STAT,
        base_points: 0,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(WARD_PULSE, pulse);
    let mut ward = SpellTemplate {
        id: WARD,
        name: "Ward".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    ward.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_PERIODIC_TRIGGER_SPELL,
        amplitude: 500,
        trigger_spell: WARD_PULSE,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(WARD, ward);
    d
}

/// A second of the map: its spells and its pets.
fn think(m: &mut Map, n: u32) {
    for _ in 0..n {
        m.now += Duration::from_secs(1);
        crate::game::map::spells::update(m);
        pets::update(m);
    }
}

fn order(m: &mut Map, owner: Guid, pet: Guid, data: u32, target: Guid) {
    pets::handle(
        m,
        owner,
        ClientOpcodeMessage::CMSG_PET_ACTION(CMSG_PET_ACTION { pet, data, target }),
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

/// Creature `g` put at `p`, its grid cell with it.
fn put(m: &mut Map, g: Guid, p: Vector3d) {
    let c = m.creature_mut(g).unwrap();
    let old = c.cell;
    c.position = p;
    c.motion = Motion::Idle { until: None };
    c.cell = crate::game::map::grid::cell_of(p.x, p.y);
    let new = c.cell;
    m.grid.relocate(g, old, new);
}

fn east(p: Vector3d, by: f32) -> Vector3d {
    Vector3d { x: p.x + by, ..p }
}

fn destination(m: &Map, g: Guid) -> Option<Vector3d> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.destination()),
        _ => None,
    }
}

/// Thistle Tea's `CreaturePet.lifetime/3` "living owners retain their
/// single pet at the visibility boundary" and `TargetSelection.next/2`
/// "rejects absent owners and candidates in another world": a hunter's pet
/// and a creature's guardian are kept with their masters 120 yards off; a
/// foe gone from the map is no pet's to take on, though it struck it; a
/// creature gone from the map, its guardian goes too.
#[tokio::test]
async fn a_pet_stays_its_masters_far_off_and_goes_with_its_master() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let master = place(&map, gh, NEUTRAL, 0x70_0050, 6.0).await;
    let out = map
        .query(move |m| {
            // Her pet, her 120 yards off.
            let p = m.players.get_mut(&gh).unwrap();
            p.movement.position.x += 120.0;
            p.movement.flags = MovementInfo_MovementFlags::empty();
            think(m, 2);
            let hers = m.creature(pet).is_some();
            // A creature's guardian, the creature 120 yards off.
            let at = m.position_of(master).unwrap();
            let guardian = pets::testing::spawn(m, master, NEUTRAL, Kind::Guardian, 10, at, 0.0);
            put(m, master, east(at, 120.0));
            think(m, 2);
            let its = m.creature(guardian).is_some();
            // The wolf struck the pet, then went from the map.
            let p = m.players.get_mut(&gh).unwrap();
            p.movement.position.x -= 120.0;
            think(m, 2);
            combat::engage(m, wolf, pet);
            let struck = m.combat(pet).is_some_and(|c| c.attackers.contains(&wolf));
            m.remove_creature(wolf);
            think(m, 1);
            let after_wolf = m.combat(pet).is_some_and(|c| c.attacking);
            // The guardian's master gone from the map.
            m.remove_creature(master);
            think(m, 1);
            (
                hers,
                its,
                struck,
                after_wolf,
                m.creature(guardian).is_some(),
            )
        })
        .await
        .unwrap();
    assert!(out.0, "her pet stays hers");
    assert!(out.1, "its guardian stays its");
    assert!(out.2, "the wolf struck it");
    assert!(!out.3, "a foe gone is no one's to fight");
    assert!(!out.4, "its master gone, the guardian goes");
}

/// Thistle Tea's `PetReturn` "a partial return path never counts as
/// arrival and is retried": a pet told to stay, 20 yards from its spot,
/// on a walk that ends 10 yards short of it, walks to its spot again once
/// that walk is over.
#[tokio::test]
async fn a_staying_pet_short_of_its_spot_walks_there_again() {
    let (_realm, _h, gh, map, pet, _wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            order(
                m,
                gh,
                pet,
                pets::button(u32::from(pets::COMMAND_STAY), ACT_COMMAND),
                Guid::zero(),
            );
            let spot = pets::state(m, pet).unwrap().stay_at.unwrap();
            put(m, pet, east(spot, 20.0));
            let now = m.now;
            let spline = m.next_spline_id;
            m.next_spline_id += 1;
            let c = m.creature_mut(pet).unwrap();
            let _ = c.walk(vec![east(spot, 20.0), east(spot, 10.0)], true, now, spline);
            // The short walk over.
            m.now += Duration::from_secs(10);
            crate::game::map::ai::advance_motion(m, pet);
            let short = m.position_of(pet).unwrap();
            pets::state_mut(m, pet).unwrap().next_think = m.now;
            m.now += Duration::from_millis(50);
            pets::update(m);
            (spot, short, destination(m, pet))
        })
        .await
        .unwrap();
    let (spot, short, to) = out;
    assert!(combat::dist(short, east(spot, 10.0)) < 0.1, "fell short");
    assert!(
        to.is_some_and(|t| combat::dist(t, spot) < 0.1),
        "on its way to its spot again: {to:?}"
    );
}

/// Thistle Tea's pet tree "aggressive pets ignore neutral and obstructed
/// units and acquire hostile players" (mangos's `PetAI::MoveInLineOfSight`,
/// `CanAttackOnSight` and `IsWithinLOSInMap`): an aggressive pet passes
/// over a neutral creature a yard off and a hostile one behind a wall two
/// yards off, and takes on a flagged enemy player three yards off.
#[tokio::test]
async fn an_aggressive_pet_takes_on_no_neutral_nor_one_behind_a_wall() {
    let (realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    // Of the Horde, on her map.
    let (_b, gb, _) = enter(&realm, "B", "Bob", Race::Undead, Class::Warrior).await;
    crate::game::tests_pets2::bring(&map, gb, gh, 3.0).await;
    crate::game::tests_pets2::flag_pvp(&map, vec![gb]);
    let neutral = place(&map, gh, NEUTRAL, 0x70_0051, 1.0).await;
    let walled = place(&map, gh, WOLF, 0x70_0052, -2.0).await;
    let out = map
        .query(move |m| {
            let p = m.players[&gh].position();
            let o = m.players[&gh].movement.orientation;
            let ahead = |d: f32| Vector3d {
                x: p.x + d * o.cos(),
                y: p.y + d * o.sin(),
                z: p.z,
            };
            put(m, pet, p);
            put(m, wolf, east(p, 200.0));
            // The neutral one a yard aside, the wolf two behind a wall.
            let side = Vector3d {
                x: p.x - o.sin(),
                y: p.y + o.cos(),
                z: p.z,
            };
            put(m, neutral, side);
            put(m, walled, ahead(-2.0));
            let wall = ahead(-1.0);
            let (dx, dy) = (-o.sin(), o.cos());
            m.spells.walls.push((
                Vector3d {
                    x: wall.x - dx,
                    y: wall.y - dy,
                    z: wall.z,
                },
                Vector3d {
                    x: wall.x + dx,
                    y: wall.y + dy,
                    z: wall.z,
                },
            ));
            let flagged = m.players[&gb]
                .fields
                .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP);
            order(
                m,
                gh,
                pet,
                pets::button(u32::from(pets::REACT_AGGRESSIVE), ACT_REACTION),
                Guid::zero(),
            );
            think(m, 1);
            (
                flagged,
                m.combat(pet).filter(|c| c.attacking).map(|c| c.victim),
            )
        })
        .await
        .unwrap();
    assert!(out.0, "Bob is flagged");
    assert_eq!(out.1, Some(gb), "Bob, not the neutral nor the walled wolf");
}

/// A totem of `owner`'s with the ward's pulse every half second, its time
/// up at `end`, its next pulse due at `next`.
fn ward(m: &mut Map, owner: Guid, end: Instant, next: Instant) -> Guid {
    let at = m.players[&owner].position();
    let t = pets::testing::spawn(m, owner, WOLF, Kind::Totem, 10, east(at, 2.0), 0.0);
    pets::state_mut(m, t).unwrap().expires = Some(end);
    let s = m.realm.data.spells.get(WARD).cloned().unwrap();
    auras::apply_spell(m, t, t, &s, &[(0, 0)], auras::Context::default()).unwrap();
    let a = auras::auras_mut(m, t).unwrap();
    for x in a.holders.iter_mut().flat_map(|h| h.effects.iter_mut()) {
        x.next_tick = Some(next);
    }
    t
}

fn pulses(m: &Map, t: Guid) -> u32 {
    auras::auras(m, t).map_or(0, |a| {
        a.holders
            .iter()
            .filter(|h| h.spell == WARD)
            .flat_map(|h| h.effects.iter())
            .map(|x| x.ticks)
            .sum()
    })
}

/// Thistle Tea's totem tree "preserves final due pulses but rejects pulses
/// after expiry or owner loss" (mangos's `Totem::Update` unsummons before
/// its auras tick): a totem's pulse due before its time is up goes, an
/// update late; one due after it doesn't, nor one due once its shaman is
/// dead; and the totem goes.
#[tokio::test]
async fn a_totems_last_pulse_goes_but_none_after_its_time_or_its_shaman() {
    let (_realm, _h, gh, map, _pet, _wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            let t0 = m.now;
            let ms = |n: u64| t0 + Duration::from_millis(n);
            let due = ward(m, gh, ms(1000), ms(500));
            let late = ward(m, gh, ms(1000), ms(1500));
            m.now = ms(1500);
            auras::update(m);
            let at_end = (pulses(m, due), pulses(m, late));
            pets::update(m);
            let gone = (m.creature(due).is_none(), m.creature(late).is_none());
            // Its shaman dead, nothing more.
            let lasting = ward(m, gh, ms(60_000), ms(1500));
            crate::game::map::death::kill(m, gh);
            auras::update(m);
            let orphan = pulses(m, lasting);
            pets::update(m);
            (at_end, gone, orphan, m.creature(lasting).is_none())
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        (1, 0),
        "the pulse due by its end goes; one after, no"
    );
    assert_eq!(out.1, (true, true), "both go");
    assert_eq!(out.2, 0, "none once its shaman is dead");
    assert!(out.3, "and it goes");
}

/// Thistle Tea's `maintain/2` "incoming threat prevents health
/// regeneration without a selected victim": a pet a wolf struck, called
/// back and passive, has no victim but stays in its fight, its health not
/// coming back, while the wolf lives; the wolf dead, its fight ends and its
/// health comes back.
#[tokio::test]
async fn a_pet_held_in_its_fight_by_a_foe_has_no_health_back_until_it_ends() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            let f = m.unit_fields_mut(wolf).unwrap();
            f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
            f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            combat::engage(m, wolf, pet);
            let f = m.unit_fields_mut(pet).unwrap();
            let half = f.get_u32(UNIT_FIELD_MAXHEALTH) / 2;
            f.set_u32(UNIT_FIELD_HEALTH, half);
            order(
                m,
                gh,
                pet,
                pets::button(u32::from(pets::REACT_PASSIVE), ACT_REACTION),
                Guid::zero(),
            );
            order(
                m,
                gh,
                pet,
                pets::button(u32::from(pets::COMMAND_FOLLOW), ACT_COMMAND),
                Guid::zero(),
            );
            think(m, 10);
            let health = |m: &Map| m.unit_fields(pet).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let held = (
                health(m) == half,
                in_combat(m, pet),
                m.unit_fields(pet)
                    .unwrap()
                    .get_guid(UNIT_FIELD_TARGET)
                    .is_zero(),
            );
            ai::die(m, wolf, gh);
            think(m, 10);
            (held, (in_combat(m, pet), health(m) > half))
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true, true), "no victim, no health back");
    assert_eq!(out.1, (false, true), "its fight over, its health back");
}

/// Thistle Tea's `die/1` "death clears references and rejects delayed
/// combat messages": a pet in a fight that dies is out of it, its foes and
/// its target gone; a blow that comes after doesn't put its corpse back
/// in a fight.
#[tokio::test]
async fn a_dead_pets_fight_is_over_and_a_later_blow_starts_none() {
    let (_realm, _h, _gh, map, pet, wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            combat::engage(m, wolf, pet);
            pets::attack_target(m, pet, wolf);
            m.creature_mut(pet).unwrap().threat.push((wolf, 10.0));
            pets::testing::die(m, pet);
            let state = |m: &Map| {
                let c = m.creature(pet).unwrap();
                (
                    in_combat(m, pet),
                    c.combat.attackers.is_empty(),
                    c.combat.attacking,
                    c.threat.is_empty(),
                    c.fields.get_guid(UNIT_FIELD_TARGET).is_zero(),
                )
            };
            let died = state(m);
            combat::engage(m, wolf, pet);
            (died, state(m))
        })
        .await
        .unwrap();
    assert_eq!(out.0, (false, true, false, true, true), "its fight over");
    assert_eq!(out.1, (false, true, false, true, true), "and none starts");
}
