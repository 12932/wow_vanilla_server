//! Phase 3 against Thistle Tea's pet tests (`test/game/core/pet`): a
//! hunter's pet's happiness and loyalty, their ticks, their thresholds and
//! what they do to its damage and training points.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::pets::testing as hunter;
use crate::game::map::pets::{HAPPINESS_LEVEL, HAPPINESS_MAX, Kind, POWER_HAPPINESS};
use crate::game::map::{Map, MapKey};

const WOLF: u32 = 963_001;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        WOLF,
        CreatureTemplate {
            entry: WOLF,
            name: "Wolf".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 20,
            level_max: 20,
            faction: 14,
            unit_class: 1,
            creature_type: 1,
            family: 1,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            health_max: 500,
            ..Default::default()
        },
    );
    // The hunter pets' level 20.
    d.pets.levels.insert(
        (1, 20),
        crate::game::data::pets::PetLevel {
            health: 600,
            mana: 0,
            armor: 400,
            damage: (0.0, 0.0),
            stats: [40, 30, 40, 20, 20],
        },
    );
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    let key = realm
        .registry
        .online(crate::game::guid::player(1))
        .map_or(MapKey::world(0), |o| o.map().key);
    realm
        .maps
        .get(realm, key)
        .query(f)
        .await
        .expect("the map answers")
}

/// A level 20 dwarf hunter, and a wolf of hers at level 20.
async fn hunter_and_pet() -> (Arc<Realm>, Client, Guid, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let pet = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        let at = p.position();
        let pet = hunter::spawn(m, h, WOLF, Kind::Hunter, 20, at, 0.0);
        m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
        pet
    })
    .await;
    (realm, a, h, pet)
}

fn happiness(m: &Map, pet: Guid) -> u32 {
    m.creature(pet)
        .unwrap()
        .fields
        .get_u32(UNIT_FIELD_POWER1 + POWER_HAPPINESS)
}

fn set_happiness(m: &mut Map, pet: Guid, value: u32) {
    let now = happiness(m, pet);
    hunter::change_happiness(m, pet, value as i32 - now as i32);
}

fn loyalty(m: &Map, pet: Guid) -> u8 {
    m.creature(pet)
        .unwrap()
        .fields
        .get_byte(UNIT_FIELD_BYTES_1, 1)
}

fn set_loyalty(m: &mut Map, pet: Guid, rank: u8, points: i32, training: i32) {
    let c = m.creature_mut(pet).unwrap();
    c.fields.set_byte(UNIT_FIELD_BYTES_1, 1, rank);
    let s = c.pet.as_deref_mut().unwrap();
    s.loyalty_points = points;
    s.training_points = training;
    hunter::show_training_points(c);
}

fn state(m: &Map, pet: Guid) -> (i32, i32, u32) {
    let c = m.creature(pet).unwrap();
    let s = c.pet.as_deref().unwrap();
    (
        s.loyalty_points,
        s.training_points,
        c.fields.get_u32(UNIT_TRAINING_POINTS),
    )
}

#[tokio::test]
async fn a_pets_happiness_falls_a_tick_at_a_time_by_its_loyalty_and_fights_and_its_damage_follows_its_mood()
 {
    let (realm, _a, _h, pet) = hunter_and_pet().await;
    let got = on_map(&realm, move |m| {
        let now = m.now;
        // Due now: one fall, the next in 7.5 seconds; again at once,
        // nothing; long late, one more.
        set_happiness(m, pet, 166_500);
        set_loyalty(m, pet, 1, 1_000, 0);
        let s = m.creature_mut(pet).unwrap().pet.as_deref_mut().unwrap();
        s.happiness_at = now;
        // Its loyalty's tick kept out of the way.
        s.loyalty_at = now + Duration::from_secs(3_600);
        hunter::tick(m, pet, now);
        let once = happiness(m, pet);
        hunter::tick(m, pet, now);
        let again = happiness(m, pet);
        let next = m
            .creature(pet)
            .unwrap()
            .pet
            .as_deref()
            .unwrap()
            .happiness_at
            - now;
        hunter::tick(m, pet, now + Duration::from_secs(100));
        let late = happiness(m, pet);
        // By loyalty, and half again in a fight.
        let mut falls = Vec::new();
        for rank in 1..=6 {
            for fighting in [false, true] {
                set_happiness(m, pet, 166_500);
                set_loyalty(m, pet, rank, 1_000, 0);
                m.creature_mut(pet).unwrap().fields.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    fighting,
                );
                let s = m.creature_mut(pet).unwrap().pet.as_deref_mut().unwrap();
                s.happiness_at = now;
                s.loyalty_at = now + Duration::from_secs(3_600);
                hunter::tick(m, pet, now);
                falls.push(166_500 - happiness(m, pet));
            }
        }
        m.creature_mut(pet).unwrap().fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            false,
        );
        // Its damage by the mood, at the thresholds, never compounding.
        let damage = |m: &Map| {
            m.creature(pet)
                .unwrap()
                .fields
                .get_f32(UNIT_FIELD_MINDAMAGE)
        };
        set_happiness(m, pet, 400_000);
        let content = damage(m);
        let mut moods = Vec::new();
        for value in [0, 332_999, 333_000, 665_999, 666_000, 400_000] {
            set_happiness(m, pet, value);
            moods.push(damage(m) / content);
        }
        // Clamped both ways.
        set_happiness(m, pet, 0);
        hunter::change_happiness(m, pet, 2_000_000);
        let full = happiness(m, pet);
        hunter::change_happiness(m, pet, -2_000_000);
        let empty = happiness(m, pet);
        // A death costs a tier; from little, to none.
        set_happiness(m, pet, 666_500);
        hunter::on_death(m, pet);
        let after_death = happiness(m, pet);
        set_happiness(m, pet, 100);
        hunter::on_death(m, pet);
        let from_little = happiness(m, pet);
        (
            (once, again, next, late),
            falls,
            moods,
            (full, empty),
            (after_death, from_little),
        )
    })
    .await;
    let ((once, again, next, late), falls, moods, clamps, deaths) = got;
    assert_eq!((once, again, late), (157_750, 157_750, 149_000));
    assert_eq!(next, hunter::HAPPINESS_TICK);
    let losses = [8_750, 4_375, 2_125, 1_000, 500, 250];
    let want: Vec<u32> = losses.iter().flat_map(|l| [*l, l * 3 / 2]).collect();
    assert_eq!(falls, want);
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    let want = [0.75, 0.75, 1.0, 1.0, 1.25, 1.0];
    assert!(
        moods.iter().zip(want).all(|(m, w)| near(*m, w)),
        "{moods:?}"
    );
    assert_eq!(clamps, (HAPPINESS_MAX, 0));
    assert_eq!(deaths, (666_500 - HAPPINESS_LEVEL, 0));
}

#[tokio::test]
async fn a_pets_loyalty_rises_past_each_threshold_with_its_levels_points_falls_into_debt_and_breaks_at_the_bottom()
 {
    let (realm, mut a, h, pet) = hunter_and_pet().await;
    let got = on_map(&realm, move |m| {
        let now = m.now;
        // A loyalty tick by the mood: down unhappy, up content, more happy.
        let mut ticks = Vec::new();
        for (value, rank) in [
            (0, 2),
            (332_999, 2),
            (333_000, 2),
            (665_999, 2),
            (666_000, 2),
        ] {
            set_happiness(m, pet, value);
            set_loyalty(m, pet, rank, 1_000, 0);
            let s = m.creature_mut(pet).unwrap().pet.as_deref_mut().unwrap();
            s.loyalty_at = now;
            s.happiness_at = now + Duration::from_secs(3_600);
            hunter::tick(m, pet, now);
            ticks.push(state(m, pet).0 - 1_000);
        }
        // Past each threshold, a rank up, the next rank's start and the
        // pet's level in training points; at it, nothing.
        let mut promotions = Vec::new();
        for (rank, threshold) in [
            (1, 5_500),
            (2, 11_500),
            (3, 17_000),
            (4, 23_500),
            (5, 31_000),
        ] {
            set_loyalty(m, pet, rank, threshold, 0);
            hunter::change_loyalty(m, pet, 0);
            let held = loyalty(m, pet);
            hunter::change_loyalty(m, pet, 1);
            promotions.push((held, loyalty(m, pet), state(m, pet)));
        }
        // Below nothing, a rank down and the level's points owed.
        set_loyalty(m, pet, 3, 0, 5);
        hunter::change_loyalty(m, pet, -1);
        let demoted = (loyalty(m, pet), state(m, pet));
        // A best friend takes no more than the top; loses as any other.
        let mut best = Vec::new();
        for delta in [20, 10, -20] {
            set_loyalty(m, pet, 6, 39_490, 0);
            hunter::change_loyalty(m, pet, delta);
            best.push(state(m, pet).0);
        }
        // More loyal, a slower fall of happiness.
        set_happiness(m, pet, 166_500);
        set_loyalty(m, pet, 1, 1_000, 0);
        hunter::change_loyalty(m, pet, 4_501);
        let s = m.creature_mut(pet).unwrap().pet.as_deref_mut().unwrap();
        s.happiness_at = now;
        hunter::tick(m, pet, now);
        let slower = 166_500 - happiness(m, pet);
        // At the bottom, the bond breaks: the pet goes.
        set_loyalty(m, pet, 1, 0, 0);
        hunter::change_loyalty(m, pet, -1);
        let gone = (m.creature(pet).is_none(), m.players[&h].owned.pet.is_none());
        (ticks, promotions, demoted, best, slower, gone)
    })
    .await;
    let (ticks, promotions, demoted, best, slower, gone) = got;
    assert_eq!(ticks, vec![-20, -20, 10, 10, 20]);
    let starts = [4_500, 7_000, 10_000, 13_500, 17_500];
    for (i, (held, risen, (points, training, shown))) in promotions.iter().enumerate() {
        assert_eq!(*held as usize, i + 1);
        assert_eq!(*risen as usize, i + 2);
        assert_eq!((*points, *training), (starts[i], 20));
        // As the client reads it: minus one more than the points.
        assert_eq!(*shown as i32, -21);
    }
    assert_eq!(demoted, (2, (4_500, -15, 15)));
    assert_eq!(best, vec![39_490, 39_500, 39_470]);
    assert_eq!(slower, 4_375);
    assert_eq!(gone, (true, true));
    a.wait_for("SMSG_PET_BROKEN", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_PET_BROKEN
        )
        .then_some(())
    })
    .await;
}

/// Levels 8 to 10 of the hunter pets' table, as Thistle Tea's test has
/// them: (level, health, armor, strength, agility, stamina).
fn with_pet_levels(mut d: GameData) -> GameData {
    for (level, health, armor, strength, agility, stamina) in [
        (8, 156, 322, 29, 25, 28),
        (9, 176, 412, 30, 26, 29),
        (10, 198, 518, 31, 26, 30),
    ] {
        d.pets.levels.insert(
            (1, level),
            crate::game::data::pets::PetLevel {
                health,
                mana: 0,
                armor,
                damage: (0.0, 0.0),
                stats: [strength, agility, stamina, 20, 20],
            },
        );
    }
    let mut kobold = d.creature_templates[&WOLF].clone();
    kobold.entry = WOLF + 1;
    kobold.name = "Kobold".into();
    d.creature_templates.insert(WOLF + 1, kobold);
    d
}

fn pet_level(m: &Map, pet: Guid) -> (u32, u32) {
    let f = &m.creature(pet).unwrap().fields;
    (
        f.get_u32(UNIT_FIELD_LEVEL),
        f.get_u32(UNIT_FIELD_PETEXPERIENCE),
    )
}

#[tokio::test]
async fn a_pet_levels_on_its_experience_to_its_masters_level_losing_what_is_over_there_and_its_kills_by_its_level()
 {
    let realm = harness::realm_with(with_pet_levels(data()));
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        let at = p.position();
        let fresh = |m: &mut Map| {
            let pet = hunter::spawn(m, h, WOLF, Kind::Hunter, 8, at, 0.0);
            m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
            set_happiness(m, pet, 500_000);
            set_loyalty(m, pet, 1, 1_000, 0);
            pet
        };
        // A part of a level is kept.
        let pet = fresh(m);
        hunter::gain_xp(m, pet, 73, 12);
        let part = pet_level(m, pet);
        // Far more, its master at 10: level 10, and nothing over; more
        // still, nothing.
        hunter::gain_xp(m, pet, 100_000, 10);
        let capped = pet_level(m, pet);
        let c = m.creature(pet).unwrap();
        let full = (
            c.fields.get_u32(UNIT_FIELD_HEALTH),
            c.fields
                .get_u32(UNIT_FIELD_POWER1 + crate::game::map::pets::POWER_FOCUS),
        );
        hunter::gain_xp(m, pet, 1_000, 10);
        let still = pet_level(m, pet);
        // Below the cap, what is over a level stays; at it, it goes; the
        // master a level up, it goes on.
        crate::game::map::pets::dismiss(m, pet);
        let pet = fresh(m);
        hunter::gain_xp(m, pet, 1_500, 10);
        let nine = pet_level(m, pet);
        hunter::gain_xp(m, pet, 2_000, 10);
        let ten = pet_level(m, pet);
        hunter::gain_xp(m, pet, 42, 11);
        let resumed = pet_level(m, pet);
        // A kill a level up gives the level's points (two a level at the
        // third loyalty), then the kill's loyalty, which a rank up gives
        // the new level's points too.
        crate::game::map::pets::dismiss(m, pet);
        let pet = fresh(m);
        set_loyalty(m, pet, 3, 16_999, 7);
        let need = crate::game::map::pets::pet_xp_to_level(&data, 8);
        hunter::gain_xp(m, pet, need - 1, 20);
        let victim = {
            let counter = m.next_creature_counter | 0x90_0000;
            m.next_creature_counter += 1;
            let t = data.creature_template(WOLF + 1).unwrap().clone();
            let c = crate::game::map::creature::Creature::new(
                crate::game::guid::unit(WOLF + 1, counter),
                &t,
                10,
                at,
                0.0,
            );
            m.add_creature(c.with_base())
        };
        hunter::kill_xp(m, h, victim, crate::game::map::pets::KillShare::Solo);
        let c = m.creature(pet).unwrap();
        let s = c.pet.as_deref().unwrap();
        let bonded = (
            c.fields.get_u32(UNIT_FIELD_LEVEL),
            c.fields.get_byte(UNIT_FIELD_BYTES_1, 1),
            s.training_points,
            s.loyalty_points,
        );
        // A group's share, for a pet no higher than the kill's best level.
        let before = pet_level(m, pet).1;
        hunter::kill_xp(
            m,
            h,
            victim,
            crate::game::map::pets::KillShare::Group {
                xp: 17,
                max_level: 9,
            },
        );
        let shared = pet_level(m, pet).1 - before;
        hunter::kill_xp(
            m,
            h,
            victim,
            crate::game::map::pets::KillShare::Group {
                xp: 17,
                max_level: 8,
            },
        );
        let above = pet_level(m, pet).1 - before - shared;
        (
            part,
            capped,
            full,
            still,
            (nine, ten, resumed),
            bonded,
            (shared, above),
        )
    })
    .await;
    let (part, capped, full, still, (nine, ten, resumed), bonded, (shared, above)) = got;
    assert_eq!(part, (8, 73));
    assert_eq!(capped, (10, 0));
    assert_eq!(full, (198, 100));
    assert_eq!(still, (10, 0));
    assert_eq!((nine, ten, resumed), ((9, 150), (10, 0), (10, 42)));
    assert_eq!(bonded, (9, 4, 7 + 2 + 9, 10_000));
    assert_eq!((shared, above), (17, 0));
}

#[test]
fn a_pets_kill_is_its_masters_base_by_its_own_levels_factor() {
    use crate::game::map::combat::{KillXp, kill_xp_with};
    let plain = KillXp::default();
    let elite = KillXp {
        elite: true,
        ..Default::default()
    };
    let none = KillXp {
        no_xp: true,
        ..Default::default()
    };
    assert_eq!(kill_xp_with(20, 8, 8, &plain), 145);
    assert_eq!(kill_xp_with(20, 8, 12, &elite), 348);
    assert_eq!(kill_xp_with(20, 8, 1, &plain), 0);
    assert_eq!(kill_xp_with(20, 8, 8, &none), 0);
    assert_eq!(kill_xp_with(60, 8, 52, &plain), 414);
}

#[test]
fn a_pets_name_is_two_to_twelve_letters_of_one_script() {
    for good in [
        "Fang",
        "Éclair",
        "Волк",
        "白狼",
        "シロオオカミ",
        "늑대",
        "Abcdefghijkl",
    ] {
        assert!(hunter::good_name(good), "{good}");
    }
    for bad in [
        "",
        "A",
        "Abcdefghijklm",
        "Wolf123",
        "White Wolf",
        "Fang!",
        "Woлf",
    ] {
        assert!(!hunter::good_name(bad), "{bad}");
    }
}

#[tokio::test]
async fn a_hunters_pet_may_be_renamed_once_by_its_master_with_a_good_name_and_keeps_it() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage as S;
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let mut b = Client::connect(&realm, "O");
    b.enter("Other", Race::Dwarf, Class::Hunter).await;
    let record = crate::game::store::PetRecord {
        number: 1,
        entry: WOLF,
        kind: 1,
        name: "Wolf".into(),
        level: 20,
        happiness: 500_000,
        loyalty: 1,
        loyalty_points: 1_000,
        ..Default::default()
    };
    let pet = on_map(&realm, move |m| {
        m.players.get_mut(&h).unwrap().row.state.pets.current = Some(record.clone());
        let pet = hunter::from_record(m, h, &record).unwrap();
        m.creature_mut(pet)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_PET_NAME_TIMESTAMP, u32::MAX - 10);
        pet
    })
    .await;
    let look = move |m: &mut Map| {
        let c = m.creature(pet).unwrap();
        (
            c.pet.as_deref().unwrap().name.clone(),
            c.fields
                .has_flag(UNIT_FIELD_FLAGS, hunter::UNIT_FLAG_PET_RENAME),
            c.fields
                .has_flag(UNIT_FIELD_FLAGS, hunter::UNIT_FLAG_PET_ABANDON),
            c.fields.get_u32(UNIT_FIELD_PET_NAME_TIMESTAMP),
        )
    };
    assert_eq!(
        on_map(&realm, look).await,
        ("Wolf".to_owned(), true, true, u32::MAX - 10)
    );
    let rename = |name: &str| CMSG_PET_RENAME {
        pet,
        name: name.into(),
    };
    // A bad name is refused, the rename kept for a good one.
    a.send(rename("Bad Name")).await;
    a.wait_for("SMSG_PET_NAME_INVALID", |m| {
        matches!(m, S::SMSG_PET_NAME_INVALID).then_some(())
    })
    .await;
    // Another hunter can't name it.
    b.send(rename("Mine")).await;
    a.send(rename("Fang")).await;
    a.send(CMSG_PLAYED_TIME {}).await;
    a.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, S::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
    // Named, a new stamp past the last, though the clock is behind it; no
    // Rename, still Abandon.
    assert_eq!(
        on_map(&realm, look).await,
        ("Fang".to_owned(), false, true, u32::MAX - 9)
    );
    a.send(rename("Claw")).await;
    a.send(CMSG_PLAYED_TIME {}).await;
    a.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, S::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
    assert_eq!(on_map(&realm, look).await.0, "Fang");
    // Dismissed and called again: still Fang, and not to be renamed.
    let back = on_map(&realm, move |m| {
        crate::game::map::pets::dismiss(m, pet);
        let record = m.players[&h].row.state.pets.current.clone().unwrap();
        let again = hunter::from_record(m, h, &record).unwrap();
        let c = m.creature(again).unwrap();
        (
            c.pet.as_deref().unwrap().name.clone(),
            c.fields
                .has_flag(UNIT_FIELD_FLAGS, hunter::UNIT_FLAG_PET_RENAME),
        )
    })
    .await;
    assert_eq!(back, ("Fang".to_owned(), false));
}

#[tokio::test]
async fn a_pet_earns_nothing_of_a_grey_kill_dead_or_put_away_and_three_quarters_of_an_npcs_pet() {
    let realm = harness::realm_with(with_pet_levels(data()));
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        let at = p.position();
        let pet = hunter::spawn(m, h, WOLF, Kind::Hunter, 8, at, 0.0);
        m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
        set_happiness(m, pet, 500_000);
        set_loyalty(m, pet, 1, 1_000, 0);
        let victim = |m: &mut Map, level: u8| {
            let counter = m.next_creature_counter | 0x91_0000;
            m.next_creature_counter += 1;
            let t = data.creature_template(WOLF + 1).unwrap().clone();
            let c = crate::game::map::creature::Creature::new(
                crate::game::guid::unit(WOLF + 1, counter),
                &t,
                level,
                at,
                0.0,
            );
            m.add_creature(c.with_base())
        };
        let xp = |m: &Map| pet_level(m, pet).1;
        let solo = crate::game::map::pets::KillShare::Solo;
        // A grey kill (level 1 to the pet's 8): nothing.
        let grey = victim(m, 1);
        hunter::kill_xp(m, h, grey, solo);
        let from_grey = xp(m);
        // An NPC's pet: three quarters.
        let npc_pet = victim(m, 8);
        let npc = victim(m, 8);
        let now = m.now;
        let c = m.creature_mut(npc_pet).unwrap();
        c.owner = Some(npc);
        c.pet = Some(Box::new(crate::game::map::pets::PetState::new(
            Kind::Demon,
            npc,
            now,
        )));
        hunter::kill_xp(m, h, npc_pet, solo);
        let from_npc_pet = xp(m) - from_grey;
        // Dead, nothing; put away, nothing.
        let ordinary = victim(m, 8);
        m.creature_mut(pet)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 0);
        if let Some(c) = m.creature_mut(pet) {
            c.life = crate::game::map::creature::Life::Dead {
                corpse_until: now,
                respawn_at: now,
            };
        }
        let before = xp(m);
        hunter::kill_xp(m, h, ordinary, solo);
        let dead = xp(m) - before;
        m.players.get_mut(&h).unwrap().owned.pet = None;
        hunter::kill_xp(m, h, ordinary, solo);
        let away = xp(m) - before;
        (from_grey, from_npc_pet, dead, away)
    })
    .await;
    use crate::game::map::combat::{KillXp, kill_xp_with};
    let pet_victim = KillXp {
        pet_victim: true,
        ..Default::default()
    };
    assert_eq!(got, (0, kill_xp_with(20, 8, 8, &pet_victim), 0, 0));
}
