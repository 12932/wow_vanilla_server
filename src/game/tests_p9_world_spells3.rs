//! Phase 9's world spells, a third part (Thistle Tea's
//! `world/loader/*_dbc_test.exs` and `totem_test.exs` on real spells, a
//! map and a player): two slaying enchantments worn at once, a totem's
//! health from its spell, Grounding Totem's pulse, Sacrifice's shield, a
//! crit's one talent point, the engineers' trinkets' summons and where a
//! compass summon or a spell's object goes.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::enchant::{self, Enchant};
use crate::game::map::spells::{self, outcome};
use crate::game::map::{Map, auras, combat, derive, inventory, pets};
use crate::game::store::BAG_PLAYER;

const GLADIUS: u32 = 2488;
const MAIN: u8 = 15;
const OFF: u8 = 16;
const BEASTSLAYING: u32 = 853;
const ELEMENTAL_SLAYER: u32 = 854;
const BEAST_SLAYER: u32 = 13_650;
const ELEMENTAL_SLAYER_SPELL: u32 = 13_651;
const YOUNG_WOLF: u32 = 299;
const FIRE_ELEMENTAL: u32 = 575;

/// The guid of what `g` wears in `slot`.
fn worn(m: &Map, g: Guid, slot: u8) -> Option<Guid> {
    let p = &m.players[&g];
    p.item_at(BAG_PLAYER, slot).map(|i| p.items[i].guid)
}

/// `g`'s gear holders of `spell`: the item each is from.
fn from_items(m: &Map, g: Guid, spell: u32) -> Vec<Guid> {
    auras::auras(m, g).map_or_else(Vec::new, |a| {
        a.holders
            .iter()
            .filter(|h| h.spell == spell && h.equipped)
            .filter_map(|h| h.item)
            .collect()
    })
}

/// Thistle Tea's "stacks independent weapon enchants and reconciles
/// removal, breakage, and restoration" (`target_damage_dbc_test.exs`):
/// two swords enchanted with Beastslaying (853, its equip spell 13650: 6
/// more damage to beasts, aura 59) give a holder each, 12 more against a
/// beast; the off hand put in the bags leaves the main hand's; the main
/// hand broken leaves the off hand's, and mended both again; the main
/// hand's enchantment changed to Elemental Slayer (854, 13651) gives 6
/// more against a beast and 6 against an elemental; and dead, the wearer
/// keeps both.
#[tokio::test]
#[ignore = "extended"]
async fn two_slaying_enchantments_stack_and_follow_their_weapons() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for (id, spell) in [
        (BEASTSLAYING, BEAST_SLAYER),
        (ELEMENTAL_SLAYER, ELEMENTAL_SLAYER_SPELL),
    ] {
        let e = d.spells.enchantments.get(&id).expect("the enchantment");
        assert_eq!(
            (e.effects[0].0, e.effects[0].3),
            (enchant::TYPE_EQUIP_SPELL, spell)
        );
    }
    let (_c, g, map) = player(&realm, "P9WS1", "Slayer", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[674]);
            crate::game::tests_p3_combat::equip(m, g, GLADIUS, MAIN);
            crate::game::tests_p3_combat::equip(m, g, GLADIUS, OFF);
            let (main, off) = (worn(m, g, MAIN).unwrap(), worn(m, g, OFF).unwrap());
            let enchanted = |m: &mut Map, item: Guid, id: u32| {
                let p = m.players.get_mut(&g).unwrap();
                let i = p.item_by_guid(item).unwrap();
                enchant::set(
                    &mut p.items[i],
                    enchant::PERM_SLOT,
                    Enchant {
                        id,
                        ..Default::default()
                    },
                );
                p.gear_dirty = true;
            };
            let durability = |m: &mut Map, item: Guid, to: Option<u32>| {
                let p = m.players.get_mut(&g).unwrap();
                let i = p.item_by_guid(item).unwrap();
                let f = &mut p.items[i].fields;
                let v = to.unwrap_or_else(|| f.get_u32(ITEM_FIELD_MAXDURABILITY));
                f.set_u32(ITEM_FIELD_DURABILITY, v);
                p.gear_dirty = true;
            };
            let wolf = spawn(m, g, YOUNG_WOLF, 60, 5.0);
            let elemental = spawn(m, g, FIRE_ELEMENTAL, 60, 5.0);
            let state = |m: &mut Map| {
                inventory::sync_gear_all(m);
                let mut beast = from_items(m, g, BEAST_SLAYER);
                beast.sort_by_key(|x| x.guid());
                (
                    beast,
                    from_items(m, g, ELEMENTAL_SLAYER_SPELL),
                    combat::damage_versus(m, g, wolf),
                    combat::damage_versus(m, g, elemental),
                )
            };
            enchanted(m, main, BEASTSLAYING);
            enchanted(m, off, BEASTSLAYING);
            let both = state(m);
            let free = (23..39).find(|s| worn(m, g, *s).is_none()).unwrap();
            inventory::move_item(m, g, (BAG_PLAYER, OFF), (BAG_PLAYER, free));
            let bagged = (worn(m, g, OFF), state(m));
            inventory::move_item(m, g, (BAG_PLAYER, free), (BAG_PLAYER, OFF));
            let back = (worn(m, g, OFF), state(m));
            durability(m, main, Some(0));
            let broken = state(m);
            durability(m, main, None);
            let mended = state(m);
            enchanted(m, main, ELEMENTAL_SLAYER);
            let changed = state(m);
            combat::deal_damage(m, wolf, g, 1_000_000);
            let alive = m.unit_view(g).is_some_and(|u| u.alive);
            let dead = (alive, state(m));
            (main, off, both, bagged, back, broken, mended, changed, dead)
        })
        .await
        .unwrap();
    let (main, off, both, bagged, back, broken, mended, changed, dead) = out;
    let mut pair = vec![main, off];
    pair.sort_by_key(|x| x.guid());
    assert_eq!(both, (pair.clone(), vec![], 12.0, 0.0), "both swords");
    assert_eq!(
        bagged,
        (None, (vec![main], vec![], 6.0, 0.0)),
        "the off hand bagged"
    );
    assert_eq!(back, (Some(off), both.clone()), "the off hand back");
    assert_eq!(
        broken,
        (vec![off], vec![], 6.0, 0.0),
        "the main hand broken"
    );
    assert_eq!(mended, both, "the main hand mended");
    assert_eq!(
        changed,
        (vec![off], vec![main], 6.0, 6.0),
        "the main hand's changed"
    );
    assert_eq!(dead, (false, changed), "dead, both kept");
}

const BATTLE_STANDARD: u32 = 23_034;
const ALLIANCE_BATTLE_STANDARD: u32 = 14_465;

/// Thistle Tea's "anchors template stamina before applying spell-defined
/// health" (`totem_test.exs`): a totem has its summon spell's health over
/// its template's. A Battle Standard (23034, a totem of no slot, 1499 and
/// a die of 1) is 1500 health and 1500 most, not its template's class
/// health at its level; its stats worked out again, still 1500. (Thistle
/// Tea's base stamina has no counterpart: wvs's creatures have their
/// template's stats, a totem's health its spell's.)
#[tokio::test]
#[ignore = "extended"]
async fn a_totems_health_is_its_spells_and_stays_through_its_stats() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let (_c, g, map) = player(&realm, "P9WS2", "Standard", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            spells::cast_triggered(m, g, BATTLE_STANDARD, Some(g));
            let standard = *m.players[&g]
                .owned
                .loose_totems
                .last()
                .expect("the standard");
            let c = m.creature(standard).unwrap();
            let entry_level = (c.entry, c.fields.get_u32(UNIT_FIELD_LEVEL));
            let life = |m: &Map| {
                m.unit_fields(standard).map(|f| {
                    (
                        f.get_u32(UNIT_FIELD_HEALTH),
                        f.get_u32(UNIT_FIELD_MAXHEALTH),
                    )
                })
            };
            let summoned = life(m);
            derive::unit(m, standard);
            auras::recalc(m, standard);
            (entry_level, summoned, life(m))
        })
        .await
        .unwrap();
    let ((entry, level), summoned, derived) = out;
    assert_eq!(entry, ALLIANCE_BATTLE_STANDARD);
    let t = d.creature_template(entry).unwrap();
    let template = d
        .class_level_stats
        .get(&(t.unit_class.max(1), level as u8))
        .map(|s| crate::game::data::load::creature_health(s.health, t.health_multiplier));
    assert!(template.is_some_and(|h| h != 1500), "{template:?}");
    assert_eq!(summoned, Some((1500, 1500)), "the spell's");
    assert_eq!(derived, summoned, "kept");
}

const GROUNDING_TOTEM: u32 = 8177;
const GROUNDING_PASSIVE: u32 = 8179;
const GROUNDING_EFFECT: u32 = 8178;

/// Thistle Tea's "Grounding's passive pulses immediately and then every
/// ten seconds" (`spell_magnet_dbc_test.exs`): Grounding Totem's passive
/// (8179, aura 23 triggering 8178 every 10 seconds) is due as the totem
/// is put down; its first tick works 8178 at once (the totem's shaman has
/// its magnet), and the next is ten seconds on, not before.
#[tokio::test]
#[ignore = "extended"]
async fn groundings_passive_pulses_at_once_and_every_ten_seconds() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let passive = d.spells.get(GROUNDING_PASSIVE).unwrap().effects[0];
    assert_eq!(
        (passive.aura, passive.trigger_spell, passive.amplitude),
        (auras::AURA_PERIODIC_TRIGGER_SPELL, GROUNDING_EFFECT, 10_000)
    );
    let (_c, s, map) = player(&realm, "P9WS3", "Grounder", Race::Orc, Class::Shaman).await;
    let out = map
        .query(move |m| {
            ready(m, s, 60, &[]);
            spells::cast_triggered(m, s, GROUNDING_TOTEM, None);
            let totem = m.players[&s]
                .owned
                .totems
                .iter()
                .flatten()
                .next()
                .copied()
                .expect("the totem");
            let pulse = |m: &Map| {
                holder(m, totem, GROUNDING_PASSIVE).map(|h| {
                    let x = &h.effects[0];
                    (
                        x.ticks,
                        x.next_tick.map(|t| t.duration_since(m.now).as_millis()),
                    )
                })
            };
            let put = pulse(m);
            auras::update(m);
            let first = (
                pulse(m),
                has(m, s, GROUNDING_EFFECT) || has(m, totem, GROUNDING_EFFECT),
            );
            pass(m, 9_999);
            let before = pulse(m).map(|p| p.0);
            pass(m, 1);
            (put, first, before, pulse(m).map(|p| p.0))
        })
        .await
        .unwrap();
    let (put, first, before, after) = out;
    assert_eq!(put, Some((0, Some(0))), "due as it is put down");
    assert_eq!(
        first,
        (Some((1, Some(10_000))), true),
        "8178 at once, the next in ten"
    );
    assert_eq!((before, after), (Some(1), Some(2)), "ten seconds on");
}

const SUMMON_VOIDWALKER: u32 = 697;
const SACRIFICE: u32 = 7812;
const HEROIC_STRIKE: u32 = 78;

/// Thistle Tea's "Voidwalker Sacrifice shields its owner against damage"
/// (`spell_pet_master_target_dbc_test.exs`): a Voidwalker's Sacrifice
/// (7812, its school absorb on its caster's master, target 27) shields
/// its warlock for more than 100; a 100-point physical blow at the
/// warlock is absorbed whole, no health lost, and the shield 100 less;
/// the Voidwalker dies of it.
#[tokio::test]
#[ignore = "extended"]
async fn a_voidwalkers_sacrifice_shields_its_warlock() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, w, map) = player(&realm, "P9WS4", "Sacrificer", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            spells::cast_triggered(m, w, SUMMON_VOIDWALKER, None);
            let pet = pets::pet_of(m, w).expect("the voidwalker");
            spells::cast_triggered(m, pet, SACRIFICE, None);
            let shield = |m: &Map| holder(m, w, SACRIFICE).map(|h| h.effects[0].left);
            let shielded = shield(m);
            let pet_alive = m.unit_view(pet).is_some_and(|u| u.alive);
            let kobold = dummy(m, w, 60, 5.0);
            let before = health(m, w);
            let s = m.realm.data.spells.get(HEROIC_STRIKE).cloned().unwrap();
            outcome::deal(
                m,
                kobold,
                w,
                &s,
                outcome::Damage {
                    amount: 100,
                    resisted: 0,
                    blocked: 0,
                },
                &outcome::Landing::default(),
            );
            (shielded, pet_alive, before - health(m, w), shield(m))
        })
        .await
        .unwrap();
    let (shielded, pet_alive, lost, left) = out;
    assert!(shielded.is_some_and(|a| a > 100), "{shielded:?}");
    assert!(!pet_alive, "the voidwalker's sacrificed");
    assert_eq!(lost, 0, "absorbed whole");
    assert_eq!(left, shielded.map(|a| a - 100));
}

const SINISTER_STRIKE: u32 = 1752;
const SEAL_FATE: u32 = 14_195;
const CLAW: u32 = 1082;
const BLOOD_FRENZY: u32 = 16_954;
const CAT_FORM: u32 = 768;
/// Every blow a crit.
const SURE_CRIT: u32 = 990_970;

/// Thistle Tea's "critical builders produce one talent proc through one
/// melee outcome" (`spell_combo_points_dbc_test.exs`): with Seal Fate
/// (14195, triggering 14189's point) a Sinister Strike that crits gives
/// two points, its own and the talent's once; one that doesn't, its own
/// alone. So for Blood Frenzy (16954, 16953) and a cat's Claw.
#[tokio::test]
#[ignore = "extended"]
async fn a_critical_builder_gives_its_talents_one_point() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut s = made_up(SURE_CRIT, 6, auras::AURA_MOD_CRIT_PERCENT, 0, 200);
        s.duration_index = 21;
        d.spells.spells.insert(SURE_CRIT, s);
    });
    let (_r, rogue, rmap) = player(&realm, "P9WS5", "Sealer", Race::Human, Class::Rogue).await;
    let (_d, druid, dmap) = player(&realm, "P9WS6", "Frenzier", Race::NightElf, Class::Druid).await;
    let points = |m: &mut Map, g: Guid, builder: u32, crit: bool| {
        m.spells.rolls = Some(false);
        if crit {
            apply(m, g, g, SURE_CRIT);
        } else {
            auras::remove_spell(m, g, SURE_CRIT, None);
        }
        let foe = dummy(m, g, 60, 3.0);
        assert!(cast_fresh(m, g, builder, at(foe)), "{builder} cast");
        let c = &m.players[&g].caster;
        (c.combo_target == foe, c.combo_points)
    };
    let rogue_points = rmap
        .query(move |m| {
            ready(m, rogue, 60, &[SINISTER_STRIKE]);
            apply(m, rogue, rogue, SEAL_FATE);
            [true, false].map(|crit| points(m, rogue, SINISTER_STRIKE, crit))
        })
        .await
        .unwrap();
    let druid_points = dmap
        .query(move |m| {
            ready(m, druid, 60, &[CLAW]);
            apply(m, druid, druid, BLOOD_FRENZY);
            apply(m, druid, druid, CAT_FORM);
            set_power(m, druid, 3, 100);
            [true, false].map(|crit| points(m, druid, CLAW, crit))
        })
        .await
        .unwrap();
    for out in [rogue_points, druid_points] {
        assert_eq!(out, [(true, 2), (true, 1)], "a crit's two, a hit's one");
    }
}

/// The engineers' trinkets: each one's item, its spell, the summon it
/// triggers and the creature that comes.
const TRINKETS: [(u32, u32, u32, u32); 4] = [
    (16_022, 23_074, 19_804, 12_473),
    (10_576, 23_075, 12_749, 8615),
    (4396, 23_076, 4073, 2678),
    (10_725, 23_133, 13_166, 8836),
];

/// Thistle Tea's "engineering trinket wrappers retain their item through
/// triggered delivery" (`spell_guardian_dbc_test.exs`): each engineer's
/// trinket's spell (Arcanite, Mithril Mechanical and Mechanical
/// Dragonling, Gnomish Battle Chicken) used from its item triggers its
/// summon with the item, which its go names (mangos's
/// `CastSpell(m_caster, .., m_CastItem)`), and its creature comes (the
/// chicken a guardian too, by vmangos's `spell_effect_mod`, where Thistle
/// Tea's test takes the client's totem); cast with no item, it summons
/// nothing.
#[tokio::test]
#[ignore = "extended"]
async fn an_engineers_trinkets_summon_keeps_its_item() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, g, map) = player(&realm, "P9WS7", "Tinkerer", Race::Gnome, Class::Warrior).await;
    for (item, wrapper, summon, entry) in TRINKETS {
        let out = map
            .query(move |m| {
                ready(m, g, 60, &[]);
                let summons = |m: &Map| {
                    m.creatures
                        .iter()
                        .filter(|(_, c)| c.entry == entry && c.owner == Some(g))
                        .count()
                };
                spells::cast_triggered(m, g, wrapper, Some(g));
                let without = summons(m);
                inventory::add_item(m, g, item, 1, false).expect("room");
                let p = &m.players[&g];
                let guid = p.items.iter().rfind(|i| i.entry == item).unwrap().guid;
                fresh(m, g);
                spells::cast(m, g, wrapper, at(g), Some(guid));
                (guid, without, summons(m))
            })
            .await
            .unwrap();
        let (guid, without, with) = out;
        assert_eq!(
            (without, with),
            (0, 1),
            "{wrapper}: none without its item, one with"
        );
        let named = c
            .gets(std::time::Duration::from_millis(500), |msg| match msg {
                ServerOpcodeMessage::SMSG_SPELL_GO(go) if go.spell == summon => Some(go.cast_item),
                _ => None,
            })
            .await;
        assert_eq!(named, Some(guid), "{summon}'s go names its item");
    }
}

/// Where a summon of a creature's at each of the compass's eight points
/// goes: its spell (target 47, 50, 48, 49, 41, 42, 43, 44) and its angle
/// off the caster's facing in half turns (front, right, back, left, front
/// right, back right, back left, front left).
const COMPASS: [(u32, f32); 8] = [
    (26_144, 0.0),
    (26_145, -0.5),
    (26_146, 1.0),
    (26_147, 0.5),
    (26_148, -0.25),
    (26_149, -0.75),
    (26_150, 0.75),
    (26_151, 0.25),
];
const EYE_TENTACLE: u32 = 15_726;
/// A player's spells that put an object down before them: Portal:
/// Stormwind (a door, 3 yards), Lightwell (a door, its second target the
/// front, 2 yards) and Freezing Trap (an object of a slot, 2 yards).
const PUT_DOWN: [(u32, u32, f32); 3] = [
    (10_059, 176_296, 3.0),
    (724, 181_102, 2.0),
    (1499, 2561, 2.0),
];

/// `from` and `o` moved `yards` at `half_turns` off `o`.
fn off(from: Vector3d, o: f32, yards: f32, half_turns: f32) -> Vector3d {
    let a = o + half_turns * std::f32::consts::PI;
    Vector3d {
        x: from.x + yards * a.cos(),
        y: from.y + yards * a.sin(),
        z: from.z,
    }
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.z - b.z).abs() < 0.01
}

/// Thistle Tea's "decodes compass summons and secondary destinations"
/// (`spell_caster_location_dbc_test.exs`): the eight Eye Tentacle summons
/// (26144 to 26151, a wild summon at 30 yards) a creature casts each put
/// one at its own point of the compass about it, 30 yards off; a
/// player's Portal: Stormwind puts its portal 3 yards before them, a
/// Lightwell (its source the caster, its second target the front) its
/// well 2 yards before them, and a Freezing Trap its trap 2 yards before
/// them.
#[tokio::test]
#[ignore = "extended"]
async fn compass_summons_and_objects_go_where_their_targets_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let well = d.spells.get(724).unwrap().effects[0];
    assert_eq!(
        (
            well.effect,
            well.target_a,
            well.target_b,
            d.spells.radius(well.radius_index)
        ),
        (50, 22, 47, 2.0)
    );
    let (_c, g, map) = player(&realm, "P9WS8", "Compass", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let caster = dummy(m, g, 60, 10.0);
            let (from, o) = {
                let c = m.creature(caster).unwrap();
                (c.position, c.orientation)
            };
            let summons: Vec<(u32, bool)> = COMPASS
                .iter()
                .map(|&(id, a)| {
                    spells::cast_triggered(m, caster, id, None);
                    let at = m
                        .creatures
                        .iter()
                        .find(|(_, c)| {
                            c.entry == EYE_TENTACLE && c.fields.get_u32(UNIT_CREATED_BY_SPELL) == id
                        })
                        .map(|(_, c)| c.position);
                    (id, at.is_some_and(|at| near(at, off(from, o, 30.0, a))))
                })
                .collect();
            let (from, o) = (m.players[&g].position(), m.players[&g].movement.orientation);
            let objects: Vec<(u32, bool)> = PUT_DOWN
                .iter()
                .map(|&(id, entry, yards)| {
                    spells::cast_triggered(m, g, id, None);
                    let at = m
                        .gameobjects
                        .iter()
                        .find(|(_, x)| x.entry == entry)
                        .map(|(_, x)| x.position);
                    (id, at.is_some_and(|at| near(at, off(from, o, yards, 0.0))))
                })
                .collect();
            (summons, objects)
        })
        .await
        .unwrap();
    let (summons, objects) = out;
    assert_eq!(summons, COMPASS.map(|(id, _)| (id, true)));
    assert_eq!(objects, PUT_DOWN.map(|(id, _, _)| (id, true)));
}
