//! Creature combat, PvP contact and the scripted procs through the
//! protocol: rage back, thrown weapons, creatures' off hands, template
//! immunities, crowd control that damage breaks, polymorph's healing, the
//! leash, fleeing and fear, guards, and the class procs.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::map::combat::{self, Contact};
use crate::game::map::creature::Motion;
use crate::game::map::creature_combat;
use crate::game::map::procs::{self, Event};
use crate::game::map::{Map, MapHandle, MapKey};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const DUMMY: (u32, u32) = (3100, 30);
const BUDDY: (u32, u32) = (3100, 31);
const BRUTE: (u32, u32) = (3101, 32);
const STONE: (u32, u32) = (3102, 33);
const PEASANT: (u32, u32) = (3103, 34);
const GUARD: (u32, u32) = (3104, 35);

const AXE: u32 = 51_001;
const KNIVES: u32 = 51_002;
const TOMAHAWK: u32 = 51_003;
const CLEAVER: u32 = 51_004;

const STRIKE: u32 = 91_001;
const SHIV: u32 = 91_002;
const STUN: u32 = 91_003;
const POLY: u32 = 91_004;
const SAP: u32 = 91_005;
const SCARE: u32 = 91_006;
const MARK: u32 = 91_007;

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

fn spell(id: u32, name: &str, effects: [SpellEffect; 3]) -> SpellTemplate {
    SpellTemplate {
        id,
        name: name.into(),
        range_index: 1,
        equipped_item_class: -1,
        effects,
        ..Default::default()
    }
}

fn creature(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 20,
        level_max: 20,
        faction: 7,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        detection_range: 20.0,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.player_level_stats.insert((1, 1, 1), [23, 20, 22, 20, 21]);
    d.player_class_stats.insert((1, 1), (60, 0));
    d.player_level_stats.insert((1, 4, 1), [18, 23, 21, 20, 20]);
    d.player_class_stats.insert((4, 1), (55, 0));
    d.class_level_stats.insert(
        (1, 20),
        ClassLevelStats {
            health: 10_000,
            melee_damage: 20.0,
            mana: 300,
            ..Default::default()
        },
    );
    // The dummies are of a faction of their own, which answers calls for
    // help, and call for it.
    let mut dummy = creature(DUMMY.0, "Dummy");
    dummy.faction = 8;
    dummy.call_for_help_range = 5.0;
    d.creature_templates.insert(DUMMY.0, dummy);
    d.faction_templates.insert(
        8,
        FactionTemplate {
            id: 8,
            flags: 0x01,
            ..Default::default()
        },
    );
    d.faction_templates.insert(
        1,
        FactionTemplate {
            id: 1,
            faction: 1,
            ..Default::default()
        },
    );
    let mut brute = creature(BRUTE.0, "Brute");
    brute.equipment_id = 7;
    d.creature_templates.insert(BRUTE.0, brute);
    d.creature_equipment.insert(7, [AXE, CLEAVER, 0]);
    let mut stone = creature(STONE.0, "Stone");
    // Immune to stuns (mechanic 12) and to physical blows.
    stone.mechanic_immune_mask = 1 << 11;
    stone.school_immune_mask = 1;
    d.creature_templates.insert(STONE.0, stone);
    let mut peasant = creature(PEASANT.0, "Peasant");
    peasant.static_flags1 = 0x0800_0000;
    d.creature_templates.insert(PEASANT.0, peasant);
    let mut guard = creature(GUARD.0, "Guard");
    guard.flags_extra = 0x400;
    d.creature_templates.insert(GUARD.0, guard);
    let spawn = |guid, entry, dx: f32, dy: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
        respawn_min: 1,
        respawn_max: 1,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(DUMMY.1, DUMMY.0, 2.0, 0.0),
            spawn(BUDDY.1, DUMMY.0, 2.0, 15.0),
            spawn(BRUTE.1, BRUTE.0, 2.0, -15.0),
            spawn(STONE.1, STONE.0, -2.0, 0.0),
            spawn(PEASANT.1, PEASANT.0, -2.0, 15.0),
            spawn(GUARD.1, GUARD.0, -2.0, 30.0),
        ],
    );
    let weapon = |entry, subclass, inventory_type| ItemTemplate {
        entry,
        class: 2,
        subclass,
        name: format!("Weapon {entry}"),
        display_id: 1000 + entry % 1000,
        inventory_type,
        delay: 2000,
        damages: [
            (10.0, 20.0, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
            (0.0, 0.0, 0),
        ],
        stackable: 1,
        ..Default::default()
    };
    d.items.insert(AXE, weapon(AXE, 0, 13));
    d.items.insert(CLEAVER, weapon(CLEAVER, 0, 13));
    let mut knives = weapon(KNIVES, 16, 25);
    knives.stackable = 200;
    d.items.insert(KNIVES, knives);
    let mut tomahawk = weapon(TOMAHAWK, 16, 25);
    tomahawk.max_durability = 10;
    d.items.insert(TOMAHAWK, tomahawk);
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    s.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 10_000,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    // An ability that gives back on a miss: rage and energy.
    let mut strike = spell(STRIKE, "Strike", [effect(17, 9, 6), none, none]);
    strike.attributes[1] = crate::game::data::spells::ATTR_EX_DISCOUNT_POWER_ON_MISS;
    strike.power_type = 1;
    strike.mana_cost = 150;
    let mut shiv = spell(SHIV, "Shiv", [effect(17, 9, 6), none, none]);
    shiv.attributes[1] = crate::game::data::spells::ATTR_EX_DISCOUNT_POWER_ON_MISS;
    shiv.power_type = 3;
    shiv.mana_cost = 40;
    let mut stun = spell(STUN, "Stun", [aura(12, 0, 6), none, none]);
    stun.mechanic = 12;
    stun.duration_index = 9;
    // A mage's polymorph: confusion, family 3, prevention 1.
    let mut poly = spell(POLY, "Polymorph", [aura(5, 0, 6), none, none]);
    poly.family_name = 3;
    poly.prevention_type = 1;
    poly.duration_index = 9;
    poly.aura_interrupt_flags = 0x2;
    let mut sap = spell(SAP, "Sap", [aura(12, 0, 6), none, none]);
    sap.aura_interrupt_flags = 0x2;
    sap.duration_index = 9;
    let mut scare = spell(SCARE, "Scare", [aura(7, 0, 6), none, none]);
    scare.school = 5;
    scare.duration_index = 9;
    let mark = spell(MARK, "Mark", [aura(4, 0, 6), none, none]);
    for t in [strike, shiv, stun, poly, sap, scare, mark] {
        s.spells.insert(t.id, t);
    }
    // The class procs' spells, as vmangos has them.
    let mut ignite_talent = spell(11119, "Ignite", [aura(4, -1, 1), none, none]);
    ignite_talent.proc_flags = 0x1_0000;
    ignite_talent.proc_chance = 100;
    let mut ignite_burn = spell(creature_burn(), "Ignite", [aura(3, 0, 6), none, none]);
    ignite_burn.effects[0].amplitude = 2000;
    ignite_burn.duration_index = 9;
    ignite_burn.school = 2;
    let mut deep_wounds = spell(12834, "Deep Wounds", [aura(42, -1, 1), none, none]);
    deep_wounds.effects[0].trigger_spell = 12162;
    deep_wounds.proc_flags = 0x4;
    deep_wounds.proc_chance = 100;
    let mut bleed = spell(12721, "Deep Wound", [aura(3, 0, 6), none, none]);
    bleed.effects[0].amplitude = 3000;
    bleed.duration_index = 9;
    let mut moe = spell(29074, "Master of Elements", [aura(4, 9, 1), none, none]);
    moe.proc_flags = 0x1_0000;
    moe.proc_chance = 100;
    let mut judgement = spell(20185, "Judgement of Light", [aura(42, -1, 6), none, none]);
    judgement.effects[0].trigger_spell = 5373;
    judgement.proc_flags = 0x8;
    judgement.proc_chance = 100;
    let light = spell(20267, "Judgement of Light", [effect(10, 24, 1), none, none]);
    let mut eye = spell(9799, "Eye for an Eye", [aura(4, 99, 1), none, none]);
    eye.proc_flags = 0x8;
    eye.proc_chance = 100;
    for t in [
        ignite_talent,
        ignite_burn,
        deep_wounds,
        bleed,
        moe,
        judgement,
        light,
        eye,
    ] {
        s.spells.insert(t.id, t);
    }
    d
}

fn creature_burn() -> u32 {
    crate::game::map::proc_scripts::IGNITE_BURN
}

async fn setup(class: Class) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, class).await;
    a.sees(guid::unit(DUMMY.0, DUMMY.1)).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map)
}

fn unit(c: (u32, u32)) -> Guid {
    guid::unit(c.0, c.1)
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn power(m: &Map, g: Guid, i: u16) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1 + i))
}

fn apply(
    m: &mut Map,
    caster: Guid,
    target: Guid,
    spell: u32,
    amount: i32,
) -> Result<(), auras::Refused> {
    let data = m.realm.data.clone();
    auras::apply_spell(
        m,
        caster,
        target,
        data.spells.get(spell).unwrap(),
        &[(0, amount)],
        Default::default(),
    )
}

#[tokio::test]
async fn avoided_blows_and_abilities_give_power_back() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (rage_back, after_miss, energy_back) = map
        .query(move |m| {
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_POWER1 + 1, 0);
            combat::ability_avoided(m, ga, STRIKE, procs::EX_DODGE, 150);
            let rage_back = power(m, ga, 1);
            // A miss refunds rage too (mangos-classic's `Spell::finish`).
            combat::ability_avoided(m, ga, STRIKE, procs::EX_MISS, 150);
            let after_miss = power(m, ga, 1);
            let p = m.players.get_mut(&ga).unwrap();
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + 3, 100);
            p.fields.set_u32(UNIT_FIELD_POWER1 + 3, 0);
            combat::ability_avoided(m, ga, SHIV, procs::EX_MISS, 40);
            (rage_back, after_miss, power(m, ga, 3))
        })
        .await
        .unwrap();
    assert_eq!(rage_back, 120, "80% of 150");
    assert_eq!(after_miss, 240, "80% of 150 again");
    assert_eq!(energy_back, 32, "80% of 40");
}

#[tokio::test]
async fn a_dodged_swing_gives_no_rage() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let target = unit(DUMMY);
    map.query(move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.derived.hit = 100.0;
        p.fields.set_f32(UNIT_FIELD_MINDAMAGE, 50.0);
        p.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 50.0);
        m.creature_mut(target).unwrap().derived.dodge = 100.0;
    })
    .await;
    a.send(CMSG_ATTACKSWING { guid: target }).await;
    a.wait_frame("a dodge", combat::SMSG_ATTACKERSTATEUPDATE, |b| {
        b.len() > 20
    })
    .await;
    // mangos-classic's `Unit::DealDamage`: rage only from damage dealt
    // (decision 98).
    let rage = map.query(move |m| power(m, ga, 1)).await.unwrap();
    assert_eq!(rage, 0);
}

#[tokio::test]
async fn thrown_weapons_spend_their_stack_or_their_wear() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (left, worn) = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let equip = |m: &mut Map, entry: u32, count: u32| {
                let p = m.players.get_mut(&ga).unwrap();
                p.items
                    .retain(|i| !(i.bag == crate::game::store::BAG_PLAYER && i.slot == 17));
                crate::game::map::inventory::add_item(m, ga, entry, count, false).unwrap();
                let p = m.players.get_mut(&ga).unwrap();
                let i = p.items.iter().rposition(|i| i.entry == entry).unwrap();
                p.items[i].slot = 17;
                p.refresh_inventory(&data);
            };
            equip(m, KNIVES, 10);
            crate::game::map::ranged::pay_ammunition(m, ga).unwrap();
            let left = m.players[&ga]
                .items
                .iter()
                .filter(|i| i.entry == KNIVES)
                .map(|i| i.count())
                .sum::<u32>();
            equip(m, TOMAHAWK, 1);
            crate::game::map::ranged::pay_ammunition(m, ga).unwrap();
            let worn = m.players[&ga]
                .items
                .iter()
                .find(|i| i.entry == TOMAHAWK)
                .map(|i| i.fields.get_u32(ITEM_FIELD_DURABILITY));
            (left, worn)
        })
        .await
        .unwrap();
    assert_eq!(left, 9);
    assert_eq!(worn, Some(9));
}

#[tokio::test]
async fn a_creature_with_two_weapons_swings_both() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let brute = unit(BRUTE);
    a.sees(brute).await;
    let dual = map
        .query(move |m| {
            // Beside Alice.
            if let Some(c) = m.creature_mut(brute) {
                c.position = Vector3d {
                    x: START.0 + 1.5,
                    y: START.1,
                    z: START.2,
                };
            }
            crate::game::map::ai::attacked(m, brute, ga);
            combat::dual_wields(m, brute)
        })
        .await
        .unwrap();
    assert!(dual);
    a.wait_frame(
        "the brute's off-hand blow",
        combat::SMSG_ATTACKERSTATEUPDATE,
        |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & combat::HITINFO_LEFTSWING != 0,
    )
    .await;
}

#[tokio::test]
async fn a_creature_template_shrugs_off_what_it_is_immune_to() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let stone = unit(STONE);
    a.sees(stone).await;
    let (stunned, frozen) = map
        .query(move |m| {
            let stunned = apply(m, ga, stone, STUN, 0);
            // A spell of another mechanic still lands.
            let frozen = apply(m, ga, stone, SCARE, 0);
            (stunned, frozen)
        })
        .await
        .unwrap();
    assert_eq!(stunned, Err(auras::Refused::Immune));
    assert!(frozen.is_ok());
    // Physical blows do nothing.
    let before = map.query(move |m| health(m, stone)).await.unwrap();
    map.query(move |m| {
        auras::remove_spell(m, stone, SCARE, None);
        // The fear may have run it a step: back home, in reach.
        if let Some(c) = m.creature_mut(stone) {
            c.position = c.home;
            c.motion = crate::game::map::creature::Motion::Idle { until: None };
            // At the warrior's level, so the blow isn't dodged or parried
            // away (a level-20 stone parries most of a level-1's swings).
            c.fields.set_u32(UNIT_FIELD_LEVEL, 1);
            crate::game::map::derive::creature(c);
            c.derived.dodge = 0.0;
            c.derived.parry = 0.0;
            c.derived.block = 0.0;
        }
        m.players.get_mut(&ga).unwrap().derived.hit = 100.0;
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.orientation = std::f32::consts::PI;
    })
    .await;
    a.send(CMSG_ATTACKSWING { guid: stone }).await;
    a.wait_frame("an immune blow", combat::SMSG_ATTACKERSTATEUPDATE, |b| {
        // Victim state follows the hit info, two packed GUIDs, the damage,
        // and one sub-damage of 20 bytes.
        b.len() >= 4 && {
            let mut at = 4;
            for _ in 0..2 {
                let mask = b[at];
                at += 1 + mask.count_ones() as usize;
            }
            at += 4 + 1 + 20;
            b.len() >= at + 4
                && u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
                    == combat::VICTIMSTATE_IS_IMMUNE
        }
    })
    .await;
    let after = map.query(move |m| health(m, stone)).await.unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn damage_breaks_polymorph_and_sap_and_a_polymorphed_creature_heals() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let target = unit(DUMMY);
    let (healed, poly_gone, sap_gone) = map
        .query(move |m| {
            apply(m, ga, target, POLY, 0).unwrap();
            let now = m.now;
            let c = m.creature_mut(target).unwrap();
            c.fields.set_u32(UNIT_FIELD_HEALTH, 100);
            c.combat.attackers.insert(ga);
            c.engagement.regen_at = Some(now);
            creature_combat::regenerate(m);
            let healed = health(m, target) > 100;
            combat::deal_damage(m, ga, target, 1);
            let poly_gone = !auras::auras(m, target).unwrap().has_spell(POLY);
            apply(m, ga, target, SAP, 0).unwrap();
            let sap = auras::auras(m, target).unwrap().holders[0].clone();
            crate::game::map::spells::periodic_damage(m, &sap, target, 1, false);
            let sap_gone = !auras::auras(m, target).unwrap().has_spell(SAP);
            (healed, poly_gone, sap_gone)
        })
        .await
        .unwrap();
    assert!(healed, "polymorph heals in a fight");
    assert!(poly_gone);
    assert!(sap_gone);
}

#[tokio::test]
async fn the_leash_holds_near_the_fight_and_lets_go_far_and_late() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let target = unit(DUMMY);
    let (near, recent, far_late, unreachable) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, target, ga);
            let near = creature_combat::should_evade(m, target, ga);
            // Far from where it began, Alice far too.
            let origin = m.creature(target).unwrap().engagement.origin.unwrap();
            let far = Vector3d {
                x: origin.x + 80.0,
                ..origin
            };
            m.creature_mut(target).unwrap().position = far;
            m.players.get_mut(&ga).unwrap().movement.position = Vector3d {
                x: origin.x + 85.0,
                ..origin
            };
            let recent = creature_combat::should_evade(m, target, ga);
            m.creature_mut(target).unwrap().engagement.last_hostile =
                Some(Instant::now() - Duration::from_secs(13));
            let far_late = creature_combat::should_evade(m, target, ga);
            // Back near, but out of reach for long.
            let c = m.creature_mut(target).unwrap();
            c.position = origin;
            c.engagement.last_hostile = Some(Instant::now());
            c.engagement.unreachable_since = Some(Instant::now() - Duration::from_secs(25));
            let unreachable = creature_combat::should_evade(m, target, ga);
            (near, recent, far_late, unreachable)
        })
        .await
        .unwrap();
    assert!(!near);
    assert!(!recent, "a blow in the last twelve seconds holds it");
    assert!(far_late);
    assert!(unreachable);
}

#[tokio::test]
async fn a_creature_flees_to_its_kind_who_come_to_fight() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (target, buddy) = (unit(DUMMY), unit(BUDDY));
    a.sees(buddy).await;
    let helper = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, target, ga);
            creature_combat::flee_for_assistance(m, target, true);
            m.creature(target)
                .and_then(|c| c.engagement.flee)
                .and_then(|f| f.helper)
        })
        .await
        .unwrap();
    assert_eq!(helper, Some(buddy));
    a.wait_for("the flight's emote", |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if matches!(c.chat_type, SMSG_MESSAGECHAT_ChatType::MonsterEmote { .. }) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    // Arrived: it calls, and its friend comes a moment later.
    let at_once = map
        .query(move |m| {
            let at = m.creature(buddy).unwrap().position;
            let c = m.creature_mut(target).unwrap();
            c.position = at;
            c.motion = Motion::Idle { until: None };
            c.engagement.next_move_at = None;
            creature_combat::controlled_step(m, target);
            m.creature(buddy).is_some_and(|c| c.combat.attacking)
        })
        .await
        .unwrap();
    assert!(!at_once, "not at once");
    tokio::time::sleep(Duration::from_millis(1700)).await;
    let joined = map
        .query(move |m| {
            m.creature(buddy)
                .is_some_and(|c| c.combat.attacking && c.combat.victim == ga)
        })
        .await
        .unwrap();
    assert!(joined);
}

#[tokio::test]
async fn fear_runs_a_creature_about() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let target = unit(DUMMY);
    let moving = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, target, ga);
            apply(m, ga, target, SCARE, 0).unwrap();
            let stepped = creature_combat::controlled_step(m, target);
            stepped && matches!(m.creature(target).unwrap().motion, Motion::Moving(_))
        })
        .await
        .unwrap();
    assert!(moving);
}

/// mangos-classic's call, not vmangos's: a civilian of no town the table
/// knows calls no guard, and doesn't rouse the guard near it (the calls
/// themselves are `tests_guards`').
#[tokio::test]
async fn a_civilian_of_no_known_town_calls_no_guard() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (peasant, guard) = (unit(PEASANT), unit(GUARD));
    a.sees(guard).await;
    let (roused, called) = map
        .query(move |m| {
            crate::game::map::ai::attacked(m, peasant, ga);
            (
                m.creature(guard).is_some_and(|c| c.combat.attacking),
                m.creatures.iter().any(|(_, c)| c.spawn_guid.is_none()),
            )
        })
        .await
        .unwrap();
    assert!(!roused && !called);
}

#[tokio::test]
async fn touching_a_flagged_player_flags_you_and_a_threat_list_keeps_you_fighting() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Cid", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let target = unit(DUMMY);
    let (attacker_flagged, healer_flagged, unflagged, fighting) = map
        .query(move |m| {
            m.players.get_mut(&gb).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                true,
            );
            let flagged = |m: &Map, g: Guid| {
                m.players[&g]
                    .fields
                    .has_flag(UNIT_FIELD_FLAGS, crate::game::map::player::UNIT_FLAG_PVP)
            };
            combat::pvp_contact(m, ga, gb, Contact::Attack);
            let attacker_flagged = flagged(m, ga);
            combat::pvp_contact(m, gc, gb, Contact::Assist);
            let healer_flagged = flagged(m, gc);
            // Touching an unflagged player flags no one.
            m.players.get_mut(&gb).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                false,
            );
            m.players.get_mut(&ga).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_PVP,
                false,
            );
            combat::pvp_contact(m, ga, gb, Contact::Attack);
            let unflagged = !flagged(m, ga);
            // On the dummy's threat list, Alice stays in a fight past six
            // seconds.
            crate::game::map::ai::attacked(m, target, ga);
            let p = m.players.get_mut(&ga).unwrap();
            p.combat.last_action = Some(Instant::now() - Duration::from_secs(7));
            p.fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                true,
            );
            combat::update_players(m);
            let fighting = m.players[&ga].fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            );
            (attacker_flagged, healer_flagged, unflagged, fighting)
        })
        .await
        .unwrap();
    assert!(attacker_flagged);
    assert!(healer_flagged);
    assert!(unflagged);
    assert!(fighting);
}

#[tokio::test]
async fn the_class_procs_do_what_their_scripts_say() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let target = unit(DUMMY);
    let fire_crit = Event {
        actor: ga,
        target,
        actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
        target_flags: procs::PROC_TAKEN_NEGATIVE_SPELL,
        ex: procs::EX_CRITICAL_HIT,
        damage: 1000,
        school: 4,
        ..Default::default()
    };
    let (ignite_once, ignite_twice, bleed, mana, healed, eye) = map
        .query(move |m| {
            let burn = |m: &Map| {
                auras::auras(m, target).and_then(|a| {
                    a.holders
                        .iter()
                        .find(|h| h.spell == creature_burn())
                        .map(|h| h.effects[0].amount)
                })
            };
            // Ignite: 4% of the crit a tick, growing with the next.
            apply(m, ga, ga, 11119, 0).unwrap();
            procs::trigger(m, fire_crit);
            let ignite_once = burn(m);
            procs::trigger(m, fire_crit);
            let ignite_twice = burn(m);
            auras::remove_spell(m, ga, 11119, None);
            // Deep Wounds: a fifth of the weapon's average over four ticks.
            apply(m, ga, ga, 12834, 0).unwrap();
            let p = m.players.get_mut(&ga).unwrap();
            p.fields.set_f32(UNIT_FIELD_MINDAMAGE, 100.0);
            p.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 100.0);
            let blow = Event {
                actor_flags: procs::PROC_DONE_MELEE,
                target_flags: procs::PROC_TAKEN_MELEE,
                school: 1,
                ..fire_crit
            };
            procs::trigger(m, blow);
            let bleed = auras::auras(m, target).and_then(|a| {
                a.holders
                    .iter()
                    .find(|h| h.spell == 12721)
                    .map(|h| h.effects[0].amount)
            });
            auras::remove_spell(m, ga, 12834, None);
            // Master of Elements: a tenth of the spell's cost back.
            let p = m.players.get_mut(&ga).unwrap();
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
            p.fields.set_u32(UNIT_FIELD_POWER1, 0);
            apply(m, ga, ga, 29074, 10).unwrap();
            procs::trigger(
                m,
                Event {
                    spell: Some(STRIKE),
                    ..fire_crit
                },
            );
            let mana = power(m, ga, 0);
            // Judgement of Light on the dummy heals whoever strikes it.
            apply(m, ga, target, 20185, 0).unwrap();
            let p = m.players.get_mut(&ga).unwrap();
            p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
            p.fields.set_u32(UNIT_FIELD_HEALTH, 500);
            procs::trigger(m, blow);
            let healed = health(m, ga) > 500;
            // Eye for an Eye returns a crit taken, up to half one's health.
            apply(m, ga, ga, 9799, 100).unwrap();
            let p = m.players.get_mut(&ga).unwrap();
            p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
            p.fields.set_u32(UNIT_FIELD_HEALTH, 1000);
            let before = health(m, target);
            procs::trigger(
                m,
                Event {
                    actor: target,
                    target: ga,
                    actor_flags: procs::PROC_DONE_MELEE,
                    target_flags: procs::PROC_TAKEN_MELEE,
                    ex: procs::EX_CRITICAL_HIT,
                    damage: 5000,
                    school: 1,
                    ..Default::default()
                },
            );
            let eye = before - health(m, target);
            (ignite_once, ignite_twice, bleed, mana, healed, eye)
        })
        .await
        .unwrap();
    assert_eq!(ignite_once, Some(40));
    assert_eq!(ignite_twice, Some(80));
    assert_eq!(bleed, Some(5), "100 * 20% / 4");
    assert_eq!(mana, 15, "10% of Strike's 150");
    assert!(healed);
    assert_eq!(eye, 500, "capped at half of Alice's 1000");
}

#[tokio::test]
async fn a_creatures_attack_power_moves_its_damage_by_vmangos_s_ratio() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let target = unit(DUMMY);
    let (before, after, ap) = map
        .query(move |m| {
            let data = m.realm.data.clone();
            // A base of 100 attack power, then a hundred more.
            let c = m.creature_mut(target).unwrap();
            c.base.attack_power = 100;
            crate::game::map::derive::creature(c);
            let before = c.fields.get_f32(UNIT_FIELD_MINDAMAGE);
            let mut roar = data.spells.get(MARK).cloned().unwrap();
            roar.effects[0].aura = auras::AURA_MOD_ATTACK_POWER;
            let _ = auras::apply_spell(m, ga, target, &roar, &[(0, 100)], Default::default());
            let c = m.creature(target).unwrap();
            (
                before,
                c.fields.get_f32(UNIT_FIELD_MINDAMAGE),
                c.derived.attack_power,
            )
        })
        .await
        .unwrap();
    assert_eq!(ap, 200);
    // (7 * 100 + 3 * 200) / (10 * 100) = 1.3.
    assert!((after / before - 1.3).abs() < 0.01, "{before} {after}");
}

/// A GM's `.spawn` is built as a world spawn is: the user's Drek'Thar came
/// with nothing in his hands, and a bare template's health.
#[tokio::test]
async fn a_spawned_creature_holds_its_weapons_and_has_its_stats() {
    let (_realm, mut a, _ga, map) = setup(Class::Warrior).await;
    a.sees(unit(BRUTE)).await;
    a.say(&format!(".spawn {}", BRUTE.0)).await;
    let spawned = a
        .wait_for("the spawned brute", |m| {
            harness::creates(m)
                .into_iter()
                .find(|g| guid::is_unit(*g) && guid::entry(*g) == BRUTE.0 && *g != unit(BRUTE))
        })
        .await;
    let look = move |m: &Map, g: Guid| {
        m.unit_fields(g).map(|f| {
            (
                f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY),
                f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + 1),
                f.get_u32(UNIT_VIRTUAL_ITEM_INFO),
                f.get_u32(UNIT_FIELD_MAXHEALTH),
            )
        })
    };
    let (world, ours) = map
        .query(move |m| (look(m, unit(BRUTE)), look(m, spawned)))
        .await
        .unwrap();
    let ours = ours.unwrap();
    assert_eq!(ours.0, 1000 + AXE % 1000, "the axe in its main hand");
    assert_eq!(ours.1, 1000 + CLEAVER % 1000, "the cleaver in its off hand");
    assert_eq!(Some(ours), world, "as the world's brute");
}
