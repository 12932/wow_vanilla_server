//! Phase 9's world data (Thistle Tea's `world/loader/*_dbc_test.exs` and
//! `*_vmangos_test.exs` beside them): Alterac Valley's altar and beacon
//! spells, a door's script by its spawn, Soul Link's script, the corpse,
//! party, raid and master targets, the auras that take a slot, the charms
//! of many, the spells that work objects in an area, and the world's
//! conditions, every one there and each tree ending.

use ahash::AHashSet;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::{fast_realm, human};
use super::tests_p3_spells_kit as kit;
use crate::game::data::GameData;
use crate::game::data::crafts::{LOCK_KEY_SKILL, LockKey};
use crate::game::data::scripts::SF_SWAP_INITIAL_TARGETS;
use crate::game::data::spells::{ATTR_EX2_ALLOW_DEAD_TARGET, SpellEffect, SpellTemplate};
use crate::game::data::{GameObjectTemplate, LootEntry};
use crate::game::map::spells::targets::{self, Implicit};
use crate::game::map::spells::{self, Targets, checks};
use crate::game::map::{Map, auras, objects};

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

/// Thistle Tea's Alterac Valley spells (`alterac_valley_assault_dbc_test.exs`):
/// each altar's ritual spell sends its ceremony's event (61, Call of the
/// Nether 7060 and Call to Ivus 7268), the spells of the Altar of Summoning
/// and the Circle of Calling; and disarming a beacon is Attacking (8386),
/// opening lock type 14 in its five seconds, which lock 99 takes. wvs runs
/// no event for the altars: their ritual done is `av_scripts::complete`
/// (raising the lord through the summoner, `tests_pvp2` and
/// `tests_p8_scripts_av`), and a beacon disarmed is `av_quests::use_beacon`.
#[test]
#[ignore = "extended"]
fn the_altars_send_their_ceremonies_and_attacking_opens_a_beacons_lock() {
    let Some(d) = real() else {
        return;
    };
    for (spell, event, altar) in [(21_249, 7060, 178_465), (21_648, 7268, 178_670)] {
        let e = d.spells.get(spell).unwrap().effects[0];
        assert_eq!((e.effect, e.misc_value), (61, event), "{spell}");
        let t = &d.gameobject_templates[&altar];
        assert_eq!((t.kind, t.data[1]), (objects::GO_RITUAL, spell), "{altar}");
    }
    let attacking = d.spells.get(8386).unwrap();
    let e = attacking.effects[0];
    assert_eq!((e.effect, e.misc_value), (33, 14));
    assert_eq!(attacking.casting_time_index, 6);
    assert_eq!(d.spells.cast_time(attacking), 5000);
    assert!(d.crafts.locks[&99].contains(&LockKey {
        kind: LOCK_KEY_SKILL,
        index: 14,
        skill: 0,
    }));
}

/// Thistle Tea's object and spell scripts (`spell_object_target_vmangos_test.exs`
/// "caches door-use scripts by spawn ID", `spell_script_vmangos_test.exs`):
/// spawn 34006's `gameobject_scripts` open door 33219 first (command 11),
/// and Soul Link's spell script (19028) is one step casting its aura 18814
/// (command 15) at the target it was given, swapped (`SF_SWAP_INITIAL_TARGETS`).
#[test]
#[ignore = "extended"]
fn a_doors_script_is_its_spawns_and_soul_links_casts_its_aura_swapped() {
    let Some(d) = real() else {
        return;
    };
    let door = &d.scripts.gameobject[&34_006][0];
    assert_eq!((door.command, door.datalong[0]), (11, 33_219));
    let link = &d.scripts.spell[&19_028];
    assert_eq!(link.len(), 1);
    assert_eq!(
        (link[0].command, link[0].datalong[0], link[0].target_type),
        (15, 18_814, 0)
    );
    assert_ne!(link[0].data_flags & SF_SWAP_INITIAL_TARGETS, 0);
}

/// Thistle Tea's spell targets (`spell_corpse_target_dbc_test.exs`,
/// `spell_party_target_dbc_test.exs`, `spell_pet_master_target_dbc_test.exs`):
/// the quest tools for corpses (Release Rageclaw, the Capture spells) are
/// for the dead, a dummy then their caster's reward, and Fireball and
/// Lesser Heal aren't; Plague is one school damage at the carrier's raid
/// in 5 yards, harmful (so its carrier is spared); the imp's Fire Shield is
/// a party member's, picked by the client; the Voidwalker's Sacrifice and
/// the owner talents are at the caster's master, and Infernal Fire at its
/// enemy and its master. What each comes to on the map is
/// `tests_p3_cast4`'s and `tests_p6_wspell_groups`'.
#[test]
#[ignore = "extended"]
fn corpse_raid_party_and_master_spells_target_as_thistle_tea_reads_them() {
    let Some(d) = real() else {
        return;
    };
    let s = |id: u32| d.spells.get(id).unwrap();
    for id in [10_617, 11_885, 11_886, 11_887, 11_888, 11_889] {
        let s = s(id);
        assert!(s.attr(2, ATTR_EX2_ALLOW_DEAD_TARGET) && checks::for_the_dead(s));
        assert_eq!(s.effects[0].effect, 3, "{id}");
        assert_ne!(s.effects[1].effect, 0, "{id}");
        assert_eq!(targets::implicit(s.effects[1].target_a), Implicit::Caster);
    }
    for id in [133, 2050] {
        assert!(!s(id).attr(2, ATTR_EX2_ALLOW_DEAD_TARGET) && !checks::for_the_dead(s(id)));
    }
    for id in [19_594, 26_557] {
        let s = s(id);
        let e = s.effects[0];
        assert_eq!(
            (e.effect, targets::implicit(e.target_a)),
            (2, Implicit::RaidAroundCaster)
        );
        assert_eq!(d.spells.radius(e.radius_index), 5.0);
        assert!(s.effects[1..].iter().all(|e| e.effect == 0), "{id}");
        assert!(!auras::positive(s), "{id}");
    }
    for id in [2947, 8316, 8317, 11_770, 11_771] {
        let t = s(id).effects[0].target_a;
        assert_eq!(targets::implicit(t), Implicit::PartyMember, "{id}");
        assert!(targets::wants_unit(t));
    }
    for id in [7812, 19_438, 23_784, 23_830, 23_831, 23_832, 24_592] {
        let t = s(id).effects[0].target_a;
        assert_eq!(targets::implicit(t), Implicit::CasterMaster, "{id}");
    }
    assert!(s(24_826).effects.iter().any(|e| {
        targets::implicit(e.target_a) == Implicit::Enemy
            && targets::implicit(e.target_b) == Implicit::CasterMaster
    }));
}

/// Thistle Tea's aura slots and charms (`aura_capacity_dbc_test.exs` "Blizzard
/// is exempt", `spell_combat_control_dbc_test.exs` "area charm"), as mangos
/// has them, not Thistle Tea: Blizzard's slow takes a slot as Hurricane's
/// does (mangos's `IsNeedVisibleSlot` exempts only the passive; Blizzard's
/// `SPELL_ATTR_EX_NO_AURA_ICON` only bars its cancelling), and the Gnomish
/// Mind Control Cap, AE Charm and Chains of Kel'Thuzad all carry a charm of
/// many (177), which charms whatever its spell.
#[test]
#[ignore = "extended"]
fn blizzard_takes_a_slot_and_every_charm_of_many_charms() {
    let Some(d) = real() else {
        return;
    };
    for id in [10, 16_914] {
        assert!(auras::visible(d.spells.get(id).unwrap(), false), "{id}");
    }
    assert_ne!(d.spells.get(10).unwrap().attributes[1] & 0x1000_0000, 0);
    for id in [26_740, 28_225, 28_410] {
        assert!(
            d.spells.get(id).unwrap().has_aura(auras::AURA_AOE_CHARM),
            "{id}"
        );
    }
}

/// Thistle Tea's object areas (`spell_object_actions_dbc_test.exs` "decodes
/// source and destination object areas"): Remove Bones (23479) despawns
/// (86, action 15) the objects about its source (22, then 51), and Hatch
/// Eggs (24083) works (action 8) those about its caster's place (18, then
/// 52), each in 100 yards (radius 12). Neither has `spell_script_target`
/// rows, so neither works any object, as in mangos.
#[test]
#[ignore = "extended"]
fn remove_bones_and_hatch_eggs_work_the_objects_about_their_source_and_destination() {
    let Some(d) = real() else {
        return;
    };
    for (id, action, a, b) in [(23_479, 15, 22, 51), (24_083, 8, 18, 52)] {
        let e = d.spells.get(id).unwrap().effects[0];
        assert_eq!(
            (
                e.effect,
                e.misc_value,
                e.target_a,
                e.target_b,
                e.radius_index
            ),
            (objects::EFFECT_ACTIVATE_OBJECT, action, a, b, 12),
            "{id}"
        );
        assert!(!d.spells.script_targets.contains_key(&id), "{id}");
    }
    assert_eq!(d.spells.radius(12), 100.0);
}

const BONES: u32 = 990_900;
const LEVER: u32 = 990_901;
const FISSURE: u32 = 990_902;
const REMOVE: u32 = 990_910;
const HATCH: u32 = 990_911;
const ROAR: u32 = 990_912;
/// A made-up radius index: 10 yards.
const TEN_YARDS: u32 = 990;

/// A spell working objects (86, `action`) in ten yards about `a` and `b`.
fn works_objects(id: u32, action: i32, a: u32, b: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Made-up {id}"),
        equipped_item_class: -1,
        range_index: 1,
        casting_time_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: objects::EFFECT_ACTIVATE_OBJECT,
        misc_value: action,
        target_a: a,
        target_b: b,
        radius_index: TEN_YARDS,
        ..Default::default()
    };
    s
}

fn object_data(d: &mut GameData) {
    d.spells.radii.insert(TEN_YARDS, 10.0);
    for (entry, kind, data) in [
        (BONES, 5, [0; 24]),
        (LEVER, 5, [0; 24]),
        (FISSURE, 6, {
            let mut data = [0; 24];
            data[3] = objects::ERUPTION;
            data[4] = 100;
            data
        }),
    ] {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind,
                display_id: 1,
                size: 1.0,
                name: format!("Object {entry}"),
                data,
                ..Default::default()
            },
        );
    }
    for (id, action, a, b, named) in [
        (REMOVE, 15, 22, 51, BONES),
        (HATCH, 15, 18, 52, BONES),
        (ROAR, 5, 18, 52, FISSURE),
        (objects::ERUPTION, 5, 18, 52, FISSURE),
    ] {
        d.spells.spells.insert(id, works_objects(id, action, a, b));
        d.spells.script_targets.insert(id, vec![(0, named)]);
    }
}

/// `entry` at `(dx, dy)` from `at`.
fn put(m: &mut Map, entry: u32, at: Vector3d, dx: f32, dy: f32) -> Guid {
    let at = Vector3d {
        x: at.x + dx,
        y: at.y + dy,
        z: at.z,
    };
    objects::summon(m, None, entry, at, 0.0, 0, 0, 0, None).unwrap()
}

/// An area of objects works every one its script targets name in its
/// radius, not the nearest alone (mangos's
/// `TARGET_ENUM_GAMEOBJECTS_SCRIPT_AOE_AT_SRC_LOC` and `_DEST_LOC`): Remove
/// Bones' shape (22, 51) despawns the three bones about its caster, not
/// those 15 yards off nor a lever beside them; Hatch Eggs' (18, 52), cast
/// by a creature 30 yards on, the two about it.
#[tokio::test]
async fn an_object_area_works_every_object_its_script_names_in_its_radius() {
    let realm = fast_realm(object_data);
    let (_c, g, map) = human(&realm, "P9WD1", "Bonepicker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let here = m.position_of(g).unwrap();
            let near = [
                put(m, BONES, here, 2.0, 0.0),
                put(m, BONES, here, 0.0, -3.0),
                put(m, BONES, here, -6.0, 6.0),
            ];
            let kept = [
                put(m, BONES, here, 0.0, 15.0),
                put(m, LEVER, here, 1.0, 1.0),
            ];
            let gone =
                |m: &Map, v: &[Guid]| v.iter().filter(|o| m.gameobject(**o).is_none()).count();
            spells::cast_triggered(m, g, REMOVE, None);
            let removed = (gone(m, &near), gone(m, &kept));
            let k = kit::dummy(m, g, 60, 30.0);
            let there = m.position_of(k).unwrap();
            let eggs = [
                put(m, BONES, there, 1.0, 0.0),
                put(m, BONES, there, 0.0, -2.0),
            ];
            spells::cast_triggered(m, k, HATCH, None);
            (removed, gone(m, &eggs), gone(m, &kept))
        })
        .await
        .unwrap();
    assert_eq!(out, ((3, 0), 2, 0));
}

/// Onyxia's roar works every lava fissure about her, each erupting once:
/// the Eruption a fissure casts only shows the fissures about it erupting
/// (mangos's `EffectActivateObject` for 17731), else each would be used
/// again and cast it again, without end.
#[tokio::test]
async fn a_roar_erupts_each_fissure_in_its_radius_once() {
    let realm = fast_realm(object_data);
    let (_c, g, map) = human(&realm, "P9WD2", "Roarer", Class::Warrior).await;
    let springs = map
        .query(move |m| {
            let here = m.position_of(g).unwrap();
            let fissures = [
                put(m, FISSURE, here, 3.0, 0.0),
                put(m, FISSURE, here, -3.0, 2.0),
                put(m, FISSURE, here, 0.0, 6.0),
                put(m, FISSURE, here, 20.0, 0.0),
            ];
            spells::cast_triggered_with(m, g, ROAR, Targets::default());
            fissures.map(|f| 100 - m.objects.traps.get(&f).map_or(100, |t| t.charges))
        })
        .await
        .unwrap();
    assert_eq!(springs, [1, 1, 1, 0]);
}

/// Thistle Tea's condition audit (`condition_coverage_test.exs`): every
/// condition the world names (loot, vendors, gossip menus and options,
/// EventAI events, every script table's steps, quests' `RequiredCondition`,
/// area trigger teleports and `spell_script_target`) is there, every
/// child of an and, or and not (-1, -2, -3) is too, and no tree comes back
/// to itself (Thistle Tea counts 8388 references to 977 conditions; wvs,
/// with its code-built scripts' and conditions, 8752 to 1341, 2333 in
/// their trees).
#[test]
#[ignore = "extended"]
fn every_condition_the_world_names_is_there_and_each_tree_ends() {
    let Some(d) = real() else {
        return;
    };
    let s = &d.scripts;
    let mut named: Vec<u32> = Vec::new();
    let loot = &d.loot;
    for t in [
        &loot.creature,
        &loot.gameobject,
        &loot.reference,
        &loot.pickpocket,
        &loot.skinning,
        &loot.fishing,
        &loot.item,
        &loot.disenchant,
    ] {
        named.extend(t.values().flatten().map(|e: &LootEntry| e.condition));
    }
    for t in [&d.vendors, &d.vendor_templates] {
        named.extend(t.values().flatten().map(|v| v.condition));
    }
    for menu in d.npc.gossip_menus.values() {
        named.extend(menu.texts.iter().map(|(_, c)| *c));
        named.extend(menu.options.iter().map(|o| o.condition));
    }
    named.extend(s.ai_events.values().flatten().map(|e| e.condition));
    for t in [
        &s.ai_scripts,
        &s.generic,
        &s.event,
        &s.spell,
        &s.quest_start,
        &s.quest_end,
        &s.gossip,
        &s.movement,
        &s.gameobject,
        &s.areatrigger,
        &s.creature_spells,
    ] {
        named.extend(t.values().flatten().map(|step| step.condition));
    }
    named.extend(s.quest_condition.values());
    named.extend(s.teleport_condition.values());
    named.extend(d.spells.script_target_conditions.values());
    named.retain(|c| *c != 0);
    let roots: AHashSet<u32> = named.iter().copied().collect();
    assert!(
        named.len() > 8000 && roots.len() > 900,
        "{} {}",
        named.len(),
        roots.len()
    );
    // Down each tree, with the conditions on the way to each.
    let mut reached: AHashSet<u32> = AHashSet::new();
    let mut missing: Vec<(u32, Option<u32>)> = Vec::new();
    let mut cycles: Vec<u32> = Vec::new();
    let mut todo: Vec<(u32, Vec<u32>)> = roots.iter().map(|r| (*r, Vec::new())).collect();
    while let Some((id, path)) = todo.pop() {
        if path.contains(&id) {
            cycles.push(id);
            continue;
        }
        let Some(c) = s.conditions.get(&id) else {
            missing.push((id, path.last().copied()));
            continue;
        };
        reached.insert(id);
        let children: Vec<u32> = match c.kind {
            -3 => vec![c.values[0]],
            -2 | -1 => c.values.to_vec(),
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|v| *v > 0)
        .map(|v| v as u32)
        .collect();
        for child in children {
            let mut p = path.clone();
            p.push(id);
            todo.push((child, p));
        }
    }
    assert!(
        missing.is_empty(),
        "missing (and who names them): {missing:?}"
    );
    assert!(cycles.is_empty(), "come back to: {cycles:?}");
    assert!(reached.len() > 1700, "{}", reached.len());
}
