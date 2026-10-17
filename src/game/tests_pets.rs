//! Pets through the protocol: taming and commanding a hunter's pet, its
//! return after a logout, Call Pet and Dismiss Pet, a warlock's imp and its
//! Firebolt, totems over a party, the stable, and Enslave Demon.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::pets::{Family, PetLevel};
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::pets::{self, ACT_COMMAND, COMMAND_ATTACK, Kind};
use crate::game::map::{Map, MapHandle};

const BOAR: u32 = 3000;
const WOLF: u32 = 2000;
const IMP: u32 = 416;
const STRENGTH_TOTEM: u32 = 5874;
const STONESKIN_TOTEM: u32 = 5873;
const STABLE_MASTER: u32 = 3010;
const FELHUNTER: u32 = 3020;

const TAME_BEAST: u32 = 1515;
const CALL_PET: u32 = 883;
const DISMISS_PET: u32 = 2641;
const SUMMON_IMP: u32 = 688;
const FIREBOLT: u32 = 3110;
const STRENGTH_OF_EARTH_TOTEM: u32 = 8075;
const STRENGTH_OF_EARTH: u32 = 8076;
const STONESKIN_TOTEM_SPELL: u32 = 8071;
const STONESKIN: u32 = 8072;
const ENSLAVE_DEMON: u32 = 1098;
const GORE: u32 = 9101;
const TRAIN_GORE: u32 = 9001;

fn faction(id: u32, ours: u32, friend_mask: u32, enemy_mask: u32) -> (u32, FactionTemplate) {
    (
        id,
        FactionTemplate {
            id,
            faction: id,
            ours,
            friend_mask,
            enemy_mask,
            ..Default::default()
        },
    )
}

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

fn spell(id: u32, name: &str, effects: [SpellEffect; 3]) -> SpellTemplate {
    SpellTemplate {
        id,
        name: name.into(),
        range_index: 1,
        effects,
        equipped_item_class: -1,
        ..Default::default()
    }
}

fn template(entry: u32, name: &str, faction: u32, unit_class: u8) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 1,
        level_max: 1,
        faction,
        unit_class,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        xp_multiplier: 1.0,
        speed_run: 1.0,
        speed_walk: 1.0,
        ..Default::default()
    }
}

/// The data the pets need: the beasts, the imp, two earth totems, a
/// stable master, and the spells.
fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        faction(1, 3, 2, 12),
        faction(3, 3, 2, 12),
        faction(2, 5, 4, 10),
        faction(5, 5, 4, 10),
        faction(14, 8, 0, 1 | 4),
        faction(32, 0, 0, 0),
        faction(35, 0, 0, 0),
    ]);
    for level in 1..=20u8 {
        for class in [1u8, 2, 8] {
            d.class_level_stats.insert(
                (class, level),
                ClassLevelStats {
                    health: 40 + u32::from(level) * 10,
                    mana: if class == 8 { 60 } else { 0 },
                    melee_damage: 3.0,
                    armor: 20,
                    ..Default::default()
                },
            );
        }
        d.pets.levels.insert(
            (1, level),
            PetLevel {
                health: 60 + u32::from(level) * 15,
                armor: 20,
                stats: [22, 20, 22, 20, 20],
                ..Default::default()
            },
        );
        d.pets.levels.insert(
            (IMP, level),
            PetLevel {
                health: 50 + u32::from(level) * 10,
                mana: 80 + u32::from(level) * 10,
                damage: (2.0, 3.0),
                stats: [20, 20, 20, 24, 23],
                ..Default::default()
            },
        );
    }
    let mut boar = template(BOAR, "Tame Boar", 32, 1);
    boar.type_flags = 0x1;
    boar.family = 5;
    d.creature_templates.insert(BOAR, boar);
    d.creature_templates
        .insert(WOLF, template(WOLF, "Wolf", 14, 1));
    let mut imp = template(IMP, "Imp", 14, 8);
    imp.family = 23;
    d.creature_templates.insert(IMP, imp);
    let mut felhunter = template(FELHUNTER, "Felhunter", 14, 2);
    felhunter.creature_type = 3;
    d.creature_templates.insert(FELHUNTER, felhunter);
    for (entry, name) in [
        (STRENGTH_TOTEM, "Strength of Earth Totem"),
        (STONESKIN_TOTEM, "Stoneskin Totem"),
    ] {
        let mut t = template(entry, name, 35, 1);
        t.creature_type = 11;
        d.creature_templates.insert(entry, t);
    }
    let mut stable = template(STABLE_MASTER, "Stable Master", 35, 1);
    stable.npc_flags = pets::NPC_FLAG_STABLEMASTER;
    d.creature_templates.insert(STABLE_MASTER, stable);
    d.pets.families.insert(
        5,
        Family {
            id: 5,
            name: "Boar".into(),
            skills: [211, 270],
            food_mask: 1,
            ..Default::default()
        },
    );
    d.pets.create_spells.insert(IMP, vec![FIREBOLT]);
    d.pets
        .totem_spells
        .insert(STRENGTH_TOTEM, STRENGTH_OF_EARTH);
    d.pets.totem_spells.insert(STONESKIN_TOTEM, STONESKIN);
    d.pets.stable_prices = vec![500, 50_000];
    // Gore: a boar's ability, taught for a training point.
    d.pets.abilities.insert(
        211,
        vec![crate::game::data::pets::PetAbility {
            spell: GORE,
            skill: 211,
            cost: 1,
            ..Default::default()
        }],
    );
    d.pets.teaching.insert(GORE, TRAIN_GORE);
    let s = &mut d.spells;
    s.cast_times.insert(2, 200);
    s.ranges.insert(1, (0.0, 30.0));
    s.radii.insert(9, 30.0);
    s.durations.insert(
        3,
        SpellDuration {
            base: 300,
            ..Default::default()
        },
    );
    s.durations.insert(
        4,
        SpellDuration {
            base: 60_000,
            ..Default::default()
        },
    );
    s.durations.insert(
        21,
        SpellDuration {
            base: -1,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    // Tame Beast: a channel whose tick tames.
    let mut tick = effect(6, 0, 6);
    tick.aura = 23;
    tick.amplitude = 300;
    tick.trigger_spell = pets::SPELL_TAME_BEAST_TICK;
    let mut tame = spell(TAME_BEAST, "Tame Beast", [effect(63, 9, 6), tick, none]);
    tame.attributes[1] = 0x4;
    tame.duration_index = 3;
    let tame_tick = spell(
        pets::SPELL_TAME_BEAST_TICK,
        "Tame Beast",
        [effect(3, 0, 6), none, none],
    );
    let tame_lands = spell(
        pets::SPELL_TAME_BEAST_TAME,
        "Tame Beast",
        [effect(55, 9, 6), none, none],
    );
    let call = spell(CALL_PET, "Call Pet", [effect(56, 0, 32), none, none]);
    let dismiss = spell(DISMISS_PET, "Dismiss Pet", [effect(102, 0, 5), none, none]);
    let mut summon_imp = effect(56, -1, 32);
    summon_imp.misc_value = IMP as i32;
    let summon = spell(SUMMON_IMP, "Summon Imp", [summon_imp, none, none]);
    let mut firebolt = spell(FIREBOLT, "Firebolt", [effect(2, 5, 6), none, none]);
    firebolt.casting_time_index = 2;
    firebolt.mana_cost = 5;
    firebolt.school = 2;
    let totem = |id, entry: u32, name: &str| {
        let mut e = effect(88, 4, 41);
        e.misc_value = entry as i32;
        let mut t = spell(id, name, [e, none, none]);
        t.duration_index = 4;
        t
    };
    let area = |id, name: &str, aura, misc| {
        let mut e = effect(35, 9, 1);
        e.aura = aura;
        e.misc_value = misc;
        e.radius_index = 9;
        let mut t = spell(id, name, [e, none, none]);
        t.duration_index = 21;
        t
    };
    let mut enslave_effect = effect(6, 0, 6);
    enslave_effect.aura = 6;
    let mut enslave = spell(ENSLAVE_DEMON, "Enslave Demon", [enslave_effect, none, none]);
    enslave.duration_index = 4;
    for t in [
        tame,
        tame_tick,
        tame_lands,
        call,
        dismiss,
        summon,
        firebolt,
        totem(
            STRENGTH_OF_EARTH_TOTEM,
            STRENGTH_TOTEM,
            "Strength of Earth Totem",
        ),
        area(STRENGTH_OF_EARTH, "Strength of Earth", 29, 0),
        totem(STONESKIN_TOTEM_SPELL, STONESKIN_TOTEM, "Stoneskin Totem"),
        area(STONESKIN, "Stoneskin", 22, 1),
        enslave,
        spell(GORE, "Gore", [effect(2, 3, 6), none, none]),
        {
            let mut e = effect(36, 0, 5);
            e.trigger_spell = GORE;
            spell(TRAIN_GORE, "Gore", [e, none, none])
        },
    ] {
        s.spells.insert(t.id, t);
    }
    d
}

/// A player in the world: their client, GUID and map.
async fn enter(
    realm: &std::sync::Arc<crate::game::Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let map = realm.registry.online(g).expect("online").map();
    (c, g, map)
}

/// Put a creature of `entry` `d` yards before `g`, with the GUID counter
/// `counter`.
async fn place(map: &MapHandle, g: Guid, entry: u32, counter: u32, d: f32) -> Guid {
    map.query(move |m| {
        let data = m.realm.data.clone();
        let t = data.creature_template(entry).cloned().unwrap();
        let p = &m.players[&g];
        let (pos, o) = (p.position(), p.movement.orientation);
        let at = Vector3d {
            x: pos.x + d * o.cos(),
            y: pos.y + d * o.sin(),
            z: pos.z,
        };
        let c = Creature::new(
            guid::unit(entry, counter),
            &t,
            1,
            at,
            o + std::f32::consts::PI,
        );
        m.add_creature(c)
    })
    .await
    .unwrap()
}

fn learn(map: &MapHandle, g: Guid, spells: &'static [u32]) {
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.spells.extend_from_slice(spells);
            for i in 0..5 {
                p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 1000);
                p.fields.set_u32(UNIT_FIELD_POWER1 + i, 1000);
            }
        }
    });
}

fn at(target: Guid) -> SpellCastTargets {
    crate::game::map::spells::wire_targets(&crate::game::map::spells::Targets {
        unit: Some(target),
        ..Default::default()
    })
}

async fn cast(c: &Client, spell: u32, target: Option<Guid>) {
    c.send(CMSG_CAST_SPELL {
        spell,
        targets: match target {
            Some(t) => at(t),
            None => SpellCastTargets {
                target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
            },
        },
    })
    .await;
}

/// The next pet bar this client gets: the pet's GUID, or zero for none.
async fn pet_bar(c: &mut Client) -> Guid {
    c.wait_for("SMSG_PET_SPELLS", |m| match m {
        ServerOpcodeMessage::SMSG_PET_SPELLS(s) => Some(s.pet),
        _ => None,
    })
    .await
}

fn pet_kind(m: &Map, g: Guid) -> Option<Kind> {
    pets::state(m, g).map(|s| s.kind)
}

/// A hunter with a tamed boar near a wolf: their client, GUID, map, the
/// pet and the wolf.
async fn tamed() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
    Guid,
) {
    let realm = harness::realm_with(data());
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, CALL_PET, DISMISS_PET]);
    let boar = place(&map, gh, BOAR, 0x70_0001, 4.0).await;
    let wolf = place(&map, gh, WOLF, 0x70_0002, 14.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    assert!(!pet.is_zero(), "the tamed boar's bar");
    (realm, h, gh, map, pet, wolf)
}

#[tokio::test]
async fn a_hunter_tames_a_beast_and_it_attacks_on_command() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed().await;
    h.sees(pet).await;
    let (kind, owner, entry, number) = map
        .query(move |m| {
            let c = m.creature(pet).unwrap();
            (
                pet_kind(m, pet),
                c.owner,
                c.entry,
                c.fields.get_u32(UNIT_FIELD_PETNUMBER),
            )
        })
        .await
        .unwrap();
    assert_eq!((kind, owner, entry), (Some(Kind::Hunter), Some(gh), BOAR));
    assert_ne!(number, 0);
    // Its name, as the client asks for it.
    h.send(CMSG_PET_NAME_QUERY {
        pet_number: number,
        guid: pet,
    })
    .await;
    let name = h
        .wait_for("SMSG_PET_NAME_QUERY_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_PET_NAME_QUERY_RESPONSE(r) => Some(r.name.clone()),
            _ => None,
        })
        .await;
    assert_eq!(name, "Tame Boar");
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: wolf,
    })
    .await;
    h.wait_for("the pet's SMSG_ATTACKSTART", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTART(a) if a.attacker == pet && a.victim == wolf => {
            Some(())
        }
        _ => None,
    })
    .await;
    // It closes on the wolf and hits it.
    let mut hurt = false;
    for _ in 0..200 {
        let health = map
            .query(move |m| {
                m.creature(wolf)
                    .map(|c| c.fields.get_u32(UNIT_FIELD_HEALTH))
            })
            .await
            .flatten();
        let max = map
            .query(move |m| {
                m.creature(wolf)
                    .map(|c| c.fields.get_u32(UNIT_FIELD_MAXHEALTH))
            })
            .await
            .flatten();
        if health < max || health.is_none() {
            hurt = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(hurt, "the pet's blows land on the wolf");
}

#[tokio::test]
async fn a_pet_comes_back_after_a_logout_and_with_call_pet() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed().await;
    h.sees(pet).await;
    // Out of the fight the taming started, so the logout is at once.
    map.query(move |m| {
        m.remove_creature(wolf);
        crate::game::map::combat::leave_combat(m, gh);
        crate::game::map::combat::leave_combat(m, pet);
    })
    .await;
    h.send(CMSG_LOGOUT_REQUEST {}).await;
    h.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    h.clear();
    h.login(gh).await;
    let back = pet_bar(&mut h).await;
    assert!(!back.is_zero(), "the pet is out again after the logout");
    let (kind, entry) = map
        .query(move |m| (pet_kind(m, back), m.creature(back).map(|c| c.entry)))
        .await
        .unwrap();
    assert_eq!((kind, entry), (Some(Kind::Hunter), Some(BOAR)));
    // Dismissed, it goes; called, it comes.
    cast(&h, DISMISS_PET, None).await;
    assert_eq!(pet_bar(&mut h).await, Guid::zero(), "dismissed");
    let out = map.query(move |m| m.players[&gh].owned.pet).await.unwrap();
    assert_eq!(out, None);
    cast(&h, CALL_PET, None).await;
    let called = pet_bar(&mut h).await;
    assert!(!called.is_zero(), "called back");
    let number_kept = map
        .query(move |m| {
            m.creature(called)
                .map(|c| c.fields.get_u32(UNIT_FIELD_PETNUMBER))
                == m.players[&gh]
                    .row
                    .state
                    .pets
                    .current
                    .as_ref()
                    .map(|r| r.number)
        })
        .await
        .unwrap();
    assert!(number_kept, "the same pet");
}

#[tokio::test]
async fn a_warlocks_imp_casts_firebolt() {
    let realm = harness::realm_with(data());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    let wolf = place(&map, gw, WOLF, 0x70_0003, 18.0).await;
    w.sees(wolf).await;
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    w.sees(imp).await;
    let (kind, spells, autocast, name) = map
        .query(move |m| {
            let s = pets::state(m, imp).unwrap();
            (s.kind, s.spells.clone(), s.autocast.clone(), s.name.clone())
        })
        .await
        .unwrap();
    assert_eq!(kind, Kind::Demon);
    assert_eq!(spells, vec![FIREBOLT]);
    assert_eq!(autocast, vec![FIREBOLT], "Firebolt casts of its own accord");
    assert_eq!(name, "Imp");
    w.send(CMSG_PET_ACTION {
        pet: imp,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: wolf,
    })
    .await;
    w.wait_for("the imp's Firebolt", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_START(s) if s.caster == imp && s.spell == FIREBOLT => {
            Some(())
        }
        _ => None,
    })
    .await;
    w.wait_for("the Firebolt landing", |m| match m {
        ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d)
            if d.attacker == imp && d.target == wolf =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    // A second imp replaces the first.
    learn(&map, gw, &[]);
    cast(&w, SUMMON_IMP, None).await;
    let second = loop {
        let g = pet_bar(&mut w).await;
        if !g.is_zero() && g != imp {
            break g;
        }
    };
    let first_gone = map.query(move |m| m.creature(imp).is_none()).await.unwrap();
    assert!(first_gone && second != imp);
}

#[tokio::test]
async fn a_totem_holds_its_aura_over_the_party_and_gives_way_to_another_of_its_element() {
    let realm = harness::realm_with(data());
    let (mut s, gs, map) = enter(&realm, "S", "Shaman", Race::Orc, Class::Shaman).await;
    let (mut b, gb, _) = enter(&realm, "B", "Brute", Race::Orc, Class::Warrior).await;
    learn(&map, gs, &[STRENGTH_OF_EARTH_TOTEM, STONESKIN_TOTEM_SPELL]);
    s.sees(gb).await;
    s.send(CMSG_GROUP_INVITE {
        name: "Brute".into(),
    })
    .await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    s.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
    cast(&s, STRENGTH_OF_EARTH_TOTEM, None).await;
    let has = |spell: u32| {
        let map = map.clone();
        async move {
            for _ in 0..100 {
                let got = map
                    .query(move |m| {
                        [gs, gb]
                            .map(|g| m.players.get(&g).is_some_and(|p| p.auras.has_spell(spell)))
                    })
                    .await
                    .unwrap();
                if got == [true, true] {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            false
        }
    };
    assert!(has(STRENGTH_OF_EARTH).await, "both have the totem's aura");
    let first = map
        .query(move |m| m.players[&gs].owned.totems[1])
        .await
        .flatten()
        .expect("an earth totem");
    assert_eq!(
        map.query(move |m| pet_kind(m, first)).await.unwrap(),
        Some(Kind::Totem)
    );
    cast(&s, STONESKIN_TOTEM_SPELL, None).await;
    assert!(has(STONESKIN).await, "the new totem's aura");
    let (old_gone, old_aura) = map
        .query(move |m| {
            (
                m.creature(first).is_none(),
                [gs, gb]
                    .iter()
                    .any(|g| m.players[g].auras.has_spell(STRENGTH_OF_EARTH)),
            )
        })
        .await
        .unwrap();
    assert!(old_gone, "one earth totem at a time");
    assert!(!old_aura, "the old totem's aura went with it");
}

async fn stable_result(h: &mut Client) -> StableResult {
    h.wait_for("SMSG_STABLE_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_STABLE_RESULT(r) => Some(r.result),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_pet_is_stabled_and_taken_out_again() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed().await;
    let master = place(&map, gh, STABLE_MASTER, 0x70_0004, 2.0).await;
    h.sees(master).await;
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&gh) {
            p.set_money(10_000);
        }
    });
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(
        stable_result(&mut h).await,
        StableResult::ErrStable,
        "no stall yet"
    );
    h.send(CMSG_BUY_STABLE_SLOT { npc: master }).await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessBuySlot);
    h.send(CMSG_STABLE_PET {
        stable_master: master,
    })
    .await;
    assert_eq!(
        pet_bar(&mut h).await,
        Guid::zero(),
        "the bar goes with the pet"
    );
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessStable);
    let (out, stabled, money) = map
        .query(move |m| {
            let p = &m.players[&gh];
            (p.owned.pet, p.row.state.pets.stable.len(), p.money())
        })
        .await
        .unwrap();
    assert_eq!((out, stabled, money), (None, 1, 9_500));
    assert!(map.query(move |m| m.creature(pet).is_none()).await.unwrap());
    h.send(MSG_LIST_STABLED_PETS_Client { npc: master }).await;
    let list = h
        .wait_for("MSG_LIST_STABLED_PETS", |m| match m {
            ServerOpcodeMessage::MSG_LIST_STABLED_PETS(l) => Some(l.clone()),
            _ => None,
        })
        .await;
    assert_eq!(list.stable_slots, 1);
    assert_eq!(list.pets.len(), 1);
    assert_eq!((list.pets[0].slot, list.pets[0].entry), (2, BOAR));
    h.send(CMSG_UNSTABLE_PET {
        stable_master: master,
        pet_number: list.pets[0].pet_number,
    })
    .await;
    assert_eq!(stable_result(&mut h).await, StableResult::SuccessUnstable);
    let back = pet_bar(&mut h).await;
    assert!(!back.is_zero(), "out of the stable");
}

#[tokio::test]
async fn an_enslaved_demon_serves_until_the_aura_ends() {
    let realm = harness::realm_with(data());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[ENSLAVE_DEMON]);
    let demon = place(&map, gw, FELHUNTER, 0x70_0005, 8.0).await;
    w.sees(demon).await;
    cast(&w, ENSLAVE_DEMON, Some(demon)).await;
    let bar = pet_bar(&mut w).await;
    assert_eq!(bar, demon, "the demon's bar");
    let (kind, faction, mine) = map
        .query(move |m| {
            let c = m.creature(demon).unwrap();
            (
                pet_kind(m, demon),
                c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                m.players[&gw].fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
            )
        })
        .await
        .unwrap();
    assert_eq!(kind, Some(Kind::Charmed));
    assert_eq!(faction, mine);
    // The aura comes off: the demon is its own, and angry.
    map.call(move |m| {
        crate::game::map::auras::remove_spell(m, demon, ENSLAVE_DEMON, None);
    });
    assert_eq!(pet_bar(&mut w).await, Guid::zero());
    let (kind, faction) = map
        .query(move |m| {
            (
                pet_kind(m, demon),
                m.creature(demon)
                    .map(|c| c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE)),
            )
        })
        .await
        .unwrap();
    assert_eq!((kind, faction), (None, Some(14)));
}

/// Against the real data, when it is here (it isn't in CI): a level 20 imp
/// knows each of its spells at its highest rank by then, a tamed level 9
/// wolf its Bite and Growl, and the families, levels and totems load.
#[test]
fn the_real_pets_know_their_spells() {
    let dir = std::path::Path::new("/local/home/wallenx/runs/tt-data/db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::data::load::load(dir);
    let imp = pets::spellbook(&data, IMP, 20, false, 23);
    assert!(
        imp.contains(&7800) && !imp.contains(&3110) && !imp.contains(&7801),
        "Firebolt rank 3 at 20: {imp:?}"
    );
    assert!(
        imp.contains(&7804),
        "Blood Pact rank 2, from the Grimoire: {imp:?}"
    );
    let wolf = pets::spellbook(&data, 118, 9, true, 1);
    assert!(
        wolf.contains(&17255) && wolf.contains(&2649),
        "Bite 2 and Growl: {wolf:?}"
    );
    let f = &data.pets.families[&1];
    assert_eq!((f.skills, f.food_mask), ([208, 270], 1));
    assert!(data.pets.level(1, 60).is_some_and(|l| l.health > 1000));
    assert!(data.pets.level(IMP, 20).is_some_and(|l| l.mana > 0));
    assert_eq!(data.pets.totem_spells.get(&2523), Some(&22048));
    assert_eq!(data.pets.create_spells.get(&IMP), Some(&vec![FIREBOLT]));
    assert_eq!(data.pets.stable_prices.len(), 2);
}

#[tokio::test]
async fn a_hunter_trains_their_pet_and_learns_what_it_uses() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed().await;
    h.sees(pet).await;
    map.query(move |m| {
        if let Some(s) = pets::state_mut(m, pet) {
            s.training_points = 5;
        }
        if let Some(p) = m.players.get_mut(&gh) {
            p.spells.push(TRAIN_GORE);
        }
    })
    .await;
    cast(&h, TRAIN_GORE, None).await;
    let mut learned = None;
    for _ in 0..100 {
        learned = map
            .query(move |m| {
                pets::state(m, pet).map(|s| (s.spells.contains(&GORE), s.training_points))
            })
            .await
            .flatten();
        if learned.is_some_and(|l| l.0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(learned, Some((true, 4)), "Gore, for a point");
    // Trained again: it knows it already.
    cast(&h, TRAIN_GORE, None).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let points = map
        .query(move |m| pets::state(m, pet).map(|s| s.training_points))
        .await
        .flatten();
    assert_eq!(points, Some(4), "nothing spent on what it knows");
    // Forgotten by the hunter, and taught again when the pet uses it.
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&gh) {
            p.spells.retain(|s| *s != TRAIN_GORE);
        }
    })
    .await;
    h.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(GORE, pets::ACT_ENABLED),
        target: wolf,
    })
    .await;
    h.wait_for("SMSG_LEARNED_SPELL", |m| match m {
        ServerOpcodeMessage::SMSG_LEARNED_SPELL(l) if l.id == TRAIN_GORE => Some(()),
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_pet_goes_while_its_master_rides_and_comes_back() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed().await;
    h.sees(pet).await;
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&gh) {
            p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 2402);
        }
    });
    assert_eq!(pet_bar(&mut h).await, Guid::zero(), "away while mounted");
    let away = map
        .query(move |m| (m.creature(pet).is_none(), m.players[&gh].owned.unsummoned))
        .await
        .unwrap();
    assert_eq!(away, (true, true));
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&gh) {
            p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
        }
    });
    let back = pet_bar(&mut h).await;
    assert!(!back.is_zero(), "back on dismounting");
    let same = map
        .query(move |m| {
            m.creature(back)
                .map(|c| c.fields.get_u32(UNIT_FIELD_PETNUMBER))
                == m.players[&gh]
                    .row
                    .state
                    .pets
                    .current
                    .as_ref()
                    .map(|r| r.number)
        })
        .await
        .unwrap();
    assert!(same);
}
