//! Item scripts, engineering gadgets, the spell effects and auras they
//! need, and mounting's rules, through the spell pipeline: the
//! transformation elixirs and fish, Linken's Boomerang, the transporters,
//! the Death Ray, the Explosive Sheep, jumper cables, drink, reputation,
//! taxis, pulls, items changed into others, effect immunity, attack power
//! against the marked, and where and how mounts are ridden.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::misc::FactionDef;
use crate::game::data::npc::TaxiPathNode;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::map::combat::{self, Hand};
use crate::game::map::spells::{self, Casting};
use crate::game::map::{Map, MapHandle, MapKey, extra_effects, intoxication, item_scripts, mounts};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const DUMMY: (u32, u32) = (3300, 50);

const NOGGENFOGGER: u32 = 16589;
const NOGGENFOGGER_OUTCOMES: [u32; 3] = [16591, 16593, 16595];
const DEVIATE_FISH: u32 = 8063;
const DEVIATE_OUTCOMES: [u32; 6] = [8064, 8065, 8066, 8067, 8068, 8070];
const SAVORY: u32 = 8213;
const LINKENS: u32 = 15712;
const ARRIVAL: u32 = 23448;
const ARRIVAL_OUTCOMES: [u32; 2] = [23444, 23445];
const JUMPER_CABLES: u32 = 8342;
const DEATH_RAY: u32 = 13278;
const DEATH_RAY_HURT: u32 = 13493;
const DEATH_RAY_BLAST: u32 = 13279;
const SHEEP_SUMMON: u32 = 4074;
const SHEEP: u32 = 2675;
const SHEEP_PASSIVE: u32 = 4051;
const SHEEP_BLAST: u32 = 4050;
const PARTY_TIME: u32 = 8067;

const DRINK: u32 = 93_001;
const TABARD: u32 = 93_002;
const FLIGHT: u32 = 93_003;
const PULL: u32 = 93_004;
const CHANGE: u32 = 93_005;
const NO_PULLS: u32 = 93_006;
const MARK: u32 = 93_007;
const MOUNT: u32 = 93_008;
const TEMPLE_MOUNT: u32 = 93_009;
const SHADOW: u32 = 93_010;
const INVISIBLE: u32 = 93_011;

const FACTION: u32 = 72;
const PATH: u32 = 77;
const OLD_HAT: u32 = 61_001;
const NEW_HAT: u32 = 61_002;

fn effect(effect: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        base_points,
        base_dice: 1,
        die_sides: 1,
        target_a,
        ..Default::default()
    }
}

fn aura(kind: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        aura: kind,
        ..effect(6, base_points, target_a)
    }
}

fn spell(id: u32, effects: [SpellEffect; 3]) -> SpellTemplate {
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 9,
        effects,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.player_level_stats.insert((1, 1, 1), [23, 20, 22, 20, 21]);
    d.player_class_stats.insert((1, 1), (60, 0));
    d.class_level_stats.insert(
        (1, 1),
        ClassLevelStats {
            health: 5000,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.creature_templates.insert(
        DUMMY.0,
        CreatureTemplate {
            entry: DUMMY.0,
            name: "Dummy".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 8,
            unit_class: 1,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            ..Default::default()
        },
    );
    let mut sheep = d.creature_templates[&DUMMY.0].clone();
    sheep.entry = SHEEP;
    sheep.name = "Explosive Sheep".into();
    d.creature_templates.insert(SHEEP, sheep);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: DUMMY.1,
            entries: vec![DUMMY.0],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        }],
    );
    d.misc.factions.insert(
        FACTION,
        FactionDef {
            id: FACTION,
            index: 0,
            name: "Stormwind".into(),
            variants: [(1, 0, 0, 0x1), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(0, FACTION);
    let node = |x: f32| TaxiPathNode {
        map: 0,
        x,
        y: START.1,
        z: START.2,
        flags: 0,
        delay: 0,
    };
    d.npc
        .taxi_path_nodes
        .insert(PATH, vec![node(START.0), node(START.0 + 50.0)]);
    for (entry, durability) in [(OLD_HAT, 100), (NEW_HAT, 40)] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class: 4,
                name: format!("Hat {entry}"),
                display_id: 1000,
                inventory_type: 1,
                max_durability: durability,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    s.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 30_000,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    let dummy_self = |id| spell(id, [effect(3, 0, 1), none, none]);
    let marker = |id| spell(id, [aura(4, 0, 1), none, none]);
    let mut all = vec![
        dummy_self(NOGGENFOGGER),
        dummy_self(DEVIATE_FISH),
        dummy_self(SAVORY),
        dummy_self(ARRIVAL),
    ];
    for id in NOGGENFOGGER_OUTCOMES
        .iter()
        .chain(&DEVIATE_OUTCOMES)
        .chain(&[8219, 8220, 8221, 8222])
        .chain(&ARRIVAL_OUTCOMES)
        .chain(&[8338])
    {
        all.push(marker(*id));
    }
    let mut boomerang = spell(LINKENS, [effect(2, 9, 6), none, none]);
    boomerang.effects[1] = SpellEffect {
        trigger_spell: 15752,
        ..effect(64, 0, 6)
    };
    boomerang.effects[2] = SpellEffect {
        trigger_spell: 15753,
        ..effect(64, 0, 6)
    };
    all.push(boomerang);
    all.push(spell(JUMPER_CABLES, [effect(18, 14, 6), none, none]));
    let mut channel = spell(DEATH_RAY, [aura(4, 0, 1), effect(3, 0, 6), none]);
    channel.duration_index = 9;
    all.push(channel);
    let mut hurt = spell(DEATH_RAY_HURT, [aura(3, 149, 1), none, none]);
    hurt.effects[0].amplitude = 1000;
    hurt.school = 2;
    all.push(hurt);
    let mut blast = spell(DEATH_RAY_BLAST, [effect(2, 599, 6), none, none]);
    blast.school = 2;
    all.push(blast);
    let mut summon = spell(SHEEP_SUMMON, [effect(42, 0, 1), none, none]);
    summon.effects[0].misc_value = SHEEP as i32;
    all.push(summon);
    all.push(marker(SHEEP_PASSIVE));
    all.push(marker(SHEEP_BLAST));
    all.push(spell(DRINK, [effect(100, 49, 1), none, none]));
    let mut tabard = spell(TABARD, [effect(103, 249, 1), none, none]);
    tabard.effects[0].misc_value = FACTION as i32;
    all.push(tabard);
    let mut flight = spell(FLIGHT, [effect(123, 0, 1), none, none]);
    flight.effects[0].misc_value = PATH as i32;
    all.push(flight);
    let mut pull = spell(PULL, [effect(124, 0, 6), none, none]);
    pull.effects[0].misc_value = 200;
    all.push(pull);
    let mut change = spell(CHANGE, [effect(34, 0, 1), none, none]);
    change.effects[0].misc_value = NEW_HAT as i32;
    all.push(change);
    let mut no_pulls = spell(NO_PULLS, [aura(37, 0, 1), none, none]);
    no_pulls.effects[0].misc_value = 124;
    all.push(no_pulls);
    all.push(spell(MARK, [aura(165, 139, 6), none, none]));
    let mut mount = spell(MOUNT, [aura(78, 0, 1), aura(32, 99, 1), none]);
    mount.effects[0].misc_value = 2402;
    all.push(mount.clone());
    mount.id = TEMPLE_MOUNT;
    all.push(mount);
    all.push(spell(25860, [effect(3, 0, 1), none, none]));
    for id in [25858, 25859, 26655, 25863] {
        let mut m = spell(id, [aura(78, 0, 1), aura(32, 59, 1), none]);
        m.effects[0].misc_value = 15665;
        all.push(m);
    }
    all.push(spell(26656, [effect(77, 0, 1), none, none]));
    let mut invisible = spell(INVISIBLE, [aura(18, 99, 1), none, none]);
    invisible.effects[0].misc_value = intoxication::DRUNK_INVISIBILITY;
    all.push(invisible);
    // A form that rides nothing.
    let mut shadow = spell(SHADOW, [aura(36, 0, 1), none, none]);
    shadow.effects[0].misc_value = 1;
    all.push(shadow);
    for t in all {
        s.spells.insert(t.id, t);
    }
    d.scripts
        .spell_areas
        .push(crate::game::data::scripts::SpellArea {
            spell: TEMPLE_MOUNT,
            area: 3428,
            gender: 2,
            ..Default::default()
        });
    d.maps.insert(
        36,
        MapInfo {
            id: 36,
            internal_name: "DeadminesInstance".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(unit()).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map)
}

fn unit() -> Guid {
    guid::unit(DUMMY.0, DUMMY.1)
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).unwrap().clone()
}

#[tokio::test]
async fn the_elixir_and_the_fish_turn_their_eater_into_one_thing_each_time() {
    let (_realm, _a, ga, map) = setup().await;
    let (elixir, fish, savory) = map
        .query(move |m| {
            let mut counts = (vec![0; 3], vec![0; 6], 0);
            for _ in 0..60 {
                spells::cast_triggered(m, ga, NOGGENFOGGER, Some(ga));
                let got: Vec<usize> = (0..3)
                    .filter(|i| has(m, ga, NOGGENFOGGER_OUTCOMES[*i]))
                    .collect();
                assert_eq!(got.len(), 1, "one outcome: {got:?}");
                counts.0[got[0]] += 1;
                for o in NOGGENFOGGER_OUTCOMES {
                    auras::remove_spell(m, ga, o, None);
                }
                spells::cast_triggered(m, ga, DEVIATE_FISH, Some(ga));
                let got: Vec<usize> = (0..6)
                    .filter(|i| has(m, ga, DEVIATE_OUTCOMES[*i]))
                    .collect();
                assert_eq!(got.len(), 1, "one outcome: {got:?}");
                counts.1[got[0]] += 1;
                for o in DEVIATE_OUTCOMES {
                    auras::remove_spell(m, ga, o, None);
                }
                // Alice's character is a woman: hers.
                spells::cast_triggered(m, ga, SAVORY, Some(ga));
                if has(m, ga, 8220) || has(m, ga, 8222) {
                    counts.2 += 1;
                }
                for o in [8219, 8220, 8221, 8222] {
                    auras::remove_spell(m, ga, o, None);
                }
            }
            counts
        })
        .await
        .unwrap();
    // The skeleton is the likeliest, three in five.
    assert!(elixir[0] > elixir[1] && elixir[0] > elixir[2], "{elixir:?}");
    assert!(fish.iter().filter(|n| **n > 0).count() >= 4, "{fish:?}");
    assert_eq!(savory, 60, "the female outcomes");
}

#[tokio::test]
async fn linkens_boomerang_rarely_stuns_and_jumper_cables_mostly_fail() {
    let (_realm, _a, ga, map) = setup().await;
    let target = unit();
    let (stuns, disarms, revived) = map
        .query(move |m| {
            let boomerang = template(m, LINKENS);
            let cables = template(m, JUMPER_CABLES);
            let (mut stuns, mut disarms, mut revived) = (0, 0, 0);
            for _ in 0..3100 {
                if !item_scripts::effect(m, ga, target, &boomerang, 1, None) {
                    stuns += 1;
                }
                if !item_scripts::effect(m, ga, target, &boomerang, 2, None) {
                    disarms += 1;
                }
            }
            for _ in 0..300 {
                auras::remove_spell(m, ga, 8338, None);
                if !item_scripts::effect(m, ga, target, &cables, 0, None) {
                    revived += 1;
                }
            }
            (stuns, disarms, revived)
        })
        .await
        .unwrap();
    assert!((40..200).contains(&stuns), "one in 31: {stuns}");
    assert!((150..450).contains(&disarms), "one in 11: {disarms}");
    assert!((50..150).contains(&revived), "a third: {revived}");
}

#[tokio::test]
async fn a_transporters_bad_arrival_malfunctions_brings_a_twin_or_nothing() {
    let (_realm, _a, ga, map) = setup().await;
    let counts = map
        .query(move |m| {
            let mut counts = [0; 3];
            for _ in 0..120 {
                spells::cast_triggered(m, ga, ARRIVAL, Some(ga));
                let i = ARRIVAL_OUTCOMES
                    .iter()
                    .position(|o| has(m, ga, *o))
                    .unwrap_or(2);
                counts[i] += 1;
                for o in ARRIVAL_OUTCOMES {
                    auras::remove_spell(m, ga, o, None);
                }
            }
            counts
        })
        .await
        .unwrap();
    assert!(
        counts[1] > counts[0] && counts[1] > counts[2],
        "the twin, four in six: {counts:?}"
    );
    assert!(counts[0] > 0 && counts[2] > 0, "{counts:?}");
}

#[tokio::test]
async fn the_death_ray_hurts_its_user_while_channelled_then_lets_it_all_loose() {
    let (_realm, _a, ga, map) = setup().await;
    let target = unit();
    let (amount, hurt, charge, lost) = map
        .query(move |m| {
            let now = m.now;
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, target);
            m.caster_mut(ga).unwrap().casting = Some(Casting {
                spell: DEATH_RAY,
                targets: Default::default(),
                ends: now + Duration::from_secs(10),
                item: None,
                pushbacks: 0,
                channel_until: Some(now + Duration::from_secs(10)),
                duration: 10_000,
            });
            let channel = template(m, DEATH_RAY);
            assert!(item_scripts::effect(m, ga, target, &channel, 1, None));
            let amount = auras::auras(m, ga)
                .and_then(|a| a.holders.iter().find(|h| h.spell == DEATH_RAY_HURT))
                .map(|h| h.effects[0].amount)
                .unwrap();
            let p = m.players.get_mut(&ga).unwrap();
            p.derived.spell_hit = 100.0;
            p.derived.spell_crit = [0.0; 7];
            p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
            p.fields.set_u32(UNIT_FIELD_HEALTH, 10_000);
            let before = health(m, ga);
            let tick = |m: &mut Map, expire: bool| {
                let now = m.now;
                let p = m.players.get_mut(&ga).unwrap();
                for h in p
                    .auras
                    .holders
                    .iter_mut()
                    .filter(|h| h.spell == DEATH_RAY_HURT)
                {
                    for x in h.effects.iter_mut() {
                        x.next_tick = Some(now);
                    }
                    if expire {
                        h.expires = Some(now);
                    }
                }
                auras::update(m);
            };
            tick(m, false);
            let hurt = before - health(m, ga);
            let charge = m.players[&ga].gadgets.death_ray;
            let full = health(m, target);
            tick(m, true);
            (amount, hurt, charge, full - health(m, target))
        })
        .await
        .unwrap();
    assert!((100..=500).contains(&amount), "{amount}");
    assert_eq!(hurt, amount as u32, "its user hurts");
    assert_eq!(charge, hurt);
    assert!(
        lost >= charge,
        "the whole of it on the target: {lost} of {charge}"
    );
}

#[tokio::test]
async fn the_explosive_sheep_carries_its_passive_and_blows_up_when_its_time_is_up() {
    let (_realm, _a, ga, map) = setup().await;
    let (passive, lasts, blew) = map
        .query(move |m| {
            spells::cast_triggered(m, ga, SHEEP_SUMMON, Some(ga));
            let sheep = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == SHEEP)
                .map(|(_, c)| c.guid)
                .unwrap();
            let now = m.now;
            let passive = has(m, sheep, SHEEP_PASSIVE);
            let lasts = m
                .creature(sheep)
                .and_then(|c| c.pet.as_deref())
                .and_then(|s| s.expires)
                .map(|t| t.duration_since(now));
            assert!(item_scripts::guardian_expired(m, sheep));
            let blew = m.creature(sheep).is_none_or(|c| !c.alive());
            (passive, lasts, blew)
        })
        .await
        .unwrap();
    assert!(passive);
    assert_eq!(lasts, Some(Duration::from_secs(180)));
    assert!(blew);
}

#[tokio::test]
async fn drink_makes_a_player_drunk_time_sobers_them_and_death_does_at_once() {
    let (_realm, _a, ga, map) = setup().await;
    let (drunk, field, sobered, seen, dead) = map
        .query(move |m| {
            spells::cast_triggered(m, ga, DRINK, Some(ga));
            let drunk = intoxication::value(m, ga);
            let field = m.players[&ga].fields.get_byte(PLAYER_BYTES_3, 1);
            let now = m.now;
            m.players.get_mut(&ga).unwrap().drunk.sober_at = Some(now);
            intoxication::update(m);
            let sobered = intoxication::value(m, ga);
            // Drunk, Alice sees what hides in drunken invisibility.
            let target = unit();
            spells::cast_triggered(m, target, INVISIBLE, Some(target));
            let seen = spells::stealth::sees(m, ga, target);
            crate::game::map::death::kill(m, ga);
            (drunk, field, sobered, seen, intoxication::value(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(drunk, 50 * 256);
    assert_eq!(field, ((50 * 256) >> 8) as u8);
    assert_eq!(sobered, 50 * 256 - 256);
    assert!(seen);
    assert_eq!(dead, 0);
}

#[tokio::test]
async fn reputation_taxi_pull_and_item_change_effects_do_what_they_say() {
    let (realm, mut a, ga, map) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let (standing, flying, hat) = map
        .query(move |m| {
            let data = m.realm.data.clone();
            spells::cast_triggered(m, ga, TABARD, Some(ga));
            let standing = crate::game::map::reputation::standing(&m.players[&ga], &data, FACTION);
            crate::game::map::inventory::add_item(m, ga, OLD_HAT, 1, false).unwrap();
            let p = m.players.get_mut(&ga).unwrap();
            let it = p.items.iter_mut().find(|i| i.entry == OLD_HAT).unwrap();
            it.fields.set_u32(ITEM_FIELD_DURABILITY, 50);
            let (item, at) = (it.guid, (it.bag, it.slot));
            let change = template(m, CHANGE);
            assert!(extra_effects::effect(m, ga, ga, &change, 0, 0, Some(item)));
            let hat = m.players[&ga]
                .items
                .iter()
                .find(|i| i.entry == NEW_HAT)
                .map(|i| {
                    (
                        (i.bag, i.slot) == at,
                        i.fields.get_u32(ITEM_FIELD_DURABILITY),
                    )
                });
            // Bob pulls Alice to him from ten yards.
            m.players.get_mut(&gb).unwrap().movement.position = Vector3d {
                x: START.0 + 10.0,
                y: START.1,
                z: START.2,
            };
            spells::cast_triggered(m, gb, PULL, Some(ga));
            spells::cast_triggered(m, ga, FLIGHT, Some(ga));
            (standing, m.players[&ga].flight.is_some(), hat)
        })
        .await
        .unwrap();
    assert_eq!(standing, 250);
    assert!(flying, "on the path");
    assert_eq!(hat, Some((true, 20)), "where it was, half worn");
    let body = a.wait_frame("the pull", 0x0EF, |b| b.len() >= 18).await;
    let cos = f32::from_le_bytes(body[body.len() - 16..body.len() - 12].try_into().unwrap());
    let speed = f32::from_le_bytes(body[body.len() - 8..body.len() - 4].try_into().unwrap());
    assert!(cos > 0.99, "towards Bob: {cos}");
    assert_eq!(speed, 20.0);
}

#[tokio::test]
async fn an_effect_immunity_stops_its_kind_and_a_mark_adds_attack_power() {
    let (realm, _a, ga, map) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let target = unit();
    let (immune, other, plain, marked) = map
        .query(move |m| {
            spells::cast_triggered(m, ga, NO_PULLS, Some(ga));
            let pull = template(m, PULL);
            let drink = template(m, DRINK);
            let immune = extra_effects::effect_immune(m, ga, &pull, 0);
            let other = extra_effects::effect_immune(m, ga, &drink, 0);
            let p = m.players.get_mut(&gb).unwrap();
            p.fields.set_f32(UNIT_FIELD_MINDAMAGE, 100.0);
            p.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 100.0);
            p.fields.set_u32(UNIT_FIELD_BASEATTACKTIME, 2800);
            let mut rng = rand::rng();
            let plain = combat::weapon_damage(m, gb, target, Hand::Main, &mut rng).0;
            spells::cast_triggered(m, gb, MARK, Some(target));
            let marked = combat::weapon_damage(m, gb, target, Hand::Main, &mut rng).0;
            (immune, other, plain, marked)
        })
        .await
        .unwrap();
    assert!(immune, "no pulls");
    assert!(!other);
    // 140 attack power, for a 2.8 second weapon: 28 more.
    assert_eq!(marked, plain + 28);
}

#[tokio::test]
async fn mounts_go_where_they_may_and_the_reindeer_and_the_tank_have_their_rules() {
    let (realm, mut a, ga, map) = setup().await;
    let (no_reindeer, shifted, swift, dismissed, tank) = map
        .query(move |m| {
            let reindeer = template(m, 25860);
            let mount = template(m, MOUNT);
            let no_reindeer = mounts::check(m, ga, &reindeer, false);
            // Shapeshifted into a form that doesn't ride.
            spells::cast_triggered(m, ga, SHADOW, Some(ga));
            let shifted = mounts::check(m, ga, &mount, false);
            auras::remove_spell(m, ga, SHADOW, None);
            assert_eq!(mounts::check(m, ga, &mount, false), None);
            // Mounted at full speed, the reindeer is a swift one.
            spells::cast_triggered(m, ga, MOUNT, Some(ga));
            m.players.get_mut(&ga).unwrap().speeds.run = 14.0;
            spells::cast_triggered(m, ga, 25860, Some(ga));
            let swift = has(m, ga, 25859) && !has(m, ga, MOUNT);
            // The tank cast mounted only dismounts.
            let qiraji = template(m, 26656);
            let dismissed = mounts::dismissed(m, ga, &qiraji) && !has(m, ga, 25859);
            // Out in the world, the tank is the one ridden anywhere.
            spells::cast_triggered(m, ga, 26656, Some(ga));
            let tank = has(m, ga, 26655);
            (no_reindeer, shifted, swift, dismissed, tank)
        })
        .await
        .unwrap();
    assert_eq!(no_reindeer, Some(mounts::ONLY_MOUNTED));
    assert_eq!(shifted, Some(0x3d), "not in that form");
    assert!(swift);
    assert!(dismissed);
    assert!(tank);
    // In the Deadmines, no mount but one that names its area; the tank is
    // the Temple's kind.
    let g = ga;
    map.call(move |m| {
        m.teleport(
            g,
            MapKey::world(36),
            Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
        )
    });
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(ga).await;
    let dungeon = realm.registry.online(ga).unwrap().map();
    let (refused, temple, tank) = dungeon
        .query(move |m| {
            let mount = template(m, MOUNT);
            let temple_mount = template(m, TEMPLE_MOUNT);
            let refused = mounts::check(m, ga, &mount, false);
            let temple = mounts::check(m, ga, &temple_mount, false);
            spells::cast_triggered(m, ga, 26656, Some(ga));
            (refused, temple, has(m, ga, 25863))
        })
        .await
        .unwrap();
    assert_eq!(refused, Some(mounts::NO_MOUNTS_ALLOWED));
    assert_eq!(temple, None, "its area says");
    assert!(tank);
}

#[tokio::test]
async fn the_deviate_fishs_party_time_dances() {
    let (_realm, mut a, ga, map) = setup().await;
    map.query(move |m| {
        spells::cast_triggered(m, ga, PARTY_TIME, Some(ga));
        let now = m.now;
        item_scripts::update(m);
        m.players.get_mut(&ga).unwrap().gadgets.party_at = Some(now);
        item_scripts::update(m);
    })
    .await;
    a.wait_for("an emote", |m| match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == ga => Some(()),
        _ => None,
    })
    .await;
}
