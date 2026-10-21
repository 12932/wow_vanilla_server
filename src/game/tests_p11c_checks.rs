//! Round 11c: a cast's checks as it ends and its targets (Thistle Tea's
//! `spell_environment_maps_test`, `gathering_test`, `teaching_dbc_test`,
//! `temporary_enchantments_test`, `spell_focus_dbc_test`,
//! `cast_validation_test`, `spell_objects_test`, `casting_test` and
//! `items_test`; mangos-classic's `Spell::cast` running `CheckCast(false)`
//! and `CheckItems`): indoors or out, tools, the item cast from and the item
//! cast at, a triggered spell's focus, an untargetable unit, game object
//! targets, an opened lock's quest credit and a death item's room.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, pass};
use crate::game::Realm;
use crate::game::data::crafts::{LOCK_KEY_SKILL, LockKey};
use crate::game::data::items::ItemSpell;
use crate::game::data::npc::Quest;
use crate::game::data::spells::{ATTR_ONLY_OUTDOORS, SpellEffect, SpellTemplate};
use crate::game::data::{GameData, GameObjectSpawn, GameObjectTemplate, ItemTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::spells::{self, Targets, checks, fail};
use crate::game::map::{Map, MapHandle, inventory, objects, professions};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const SMSG_CAST_RESULT: u16 = 0x130;
const SMSG_INVENTORY_CHANGE_FAILURE: u16 = 0x112;

/// A two-second aura only for outdoors, of ten mana.
const OUTDOOR: u32 = 998_901;
/// A two-second dig with a tool, making ore.
const DIG: u32 = 998_902;
/// A book's two-second lesson, and what it teaches.
const LESSON: u32 = 998_903;
const LEARNED: u32 = 998_904;
/// A two-second coating of a weapon, and the same wanting a reagent.
const COATING: u32 = 998_905;
const COATING_REAGENT: u32 = 998_906;
/// An aura wanting a spell focus.
const FOCUSED: u32 = 998_907;
/// A heal at any unit.
const ANY_HEAL: u32 = 998_908;
/// Two object effects of a spell of two targets; a spell at an object
/// aimed, of ten yards; an opening of a chest.
const OBJECTS: u32 = 998_909;
const AT_OBJECT: u32 = 998_910;
const OPEN: u32 = 998_911;
/// Collect Rookery Egg: its egg optional (mangos-classic).
const ROOKERY_EGG: u32 = 15_958;

const TOOL: u32 = 998_920;
const ORE: u32 = 998_921;
const BOOK: u32 = 998_922;
const VIAL: u32 = 998_923;
const WEAPON: u32 = 998_924;
const REAGENT: u32 = 998_925;
const SHARD: u32 = 998_926;
const FILLER: u32 = 998_927;
const EGG: u32 = 998_928;

const FOCUS_OBJECT: u32 = 998_940;
const ALTAR: u32 = 998_941;
const PILLAR: u32 = 998_942;
const CHEST: u32 = 998_943;
const EGG_OBJECT: u32 = 998_944;
const FOCUS: u32 = 77;
const COOKING: u32 = 185;
const QUEST: u32 = 998_950;

const RANGE_10: u32 = 953;
const RADIUS_5: u32 = 954;
const LOCK: u32 = 955;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 20,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn on_use(spell: u32, charges: i32) -> [ItemSpell; 5] {
    let mut s: [ItemSpell; 5] = Default::default();
    s[0] = ItemSpell {
        id: spell,
        charges,
        ..Default::default()
    };
    s
}

fn timed(id: u32, e: SpellEffect) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.casting_time_index = CAST_2S;
    effect(&mut s, 0, e);
    s
}

fn create(entry: u32) -> SpellEffect {
    SpellEffect {
        effect: 24,
        item_type: u64::from(entry),
        base_points: 0,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    }
}

fn activate(target_a: u32) -> SpellEffect {
    SpellEffect {
        effect: objects::EFFECT_ACTIVATE_OBJECT,
        misc_value: 7,
        target_a,
        radius_index: RADIUS_5,
        ..Default::default()
    }
}

fn object_template(entry: u32, kind: u8, data: [u32; 24]) -> GameObjectTemplate {
    GameObjectTemplate {
        entry,
        kind,
        display_id: 1,
        name: format!("Object {entry}"),
        size: 1.0,
        data,
        ..Default::default()
    }
}

fn data(d: &mut GameData) {
    d.spells.ranges.insert(RANGE_10, (0.0, 10.0));
    d.spells.radii.insert(RADIUS_5, 5.0);
    let mut outdoor = timed(OUTDOOR, aura(crate::game::map::auras::AURA_DUMMY, 0, 5, 1));
    outdoor.attributes[0] |= ATTR_ONLY_OUTDOORS;
    outdoor.mana_cost = 10;
    outdoor.duration_index = LASTS_LONG;
    let mut dig = timed(DIG, create(ORE));
    dig.totems[0] = TOOL;
    let lesson = timed(
        LESSON,
        SpellEffect {
            effect: 36,
            trigger_spell: LEARNED,
            target_a: 1,
            ..Default::default()
        },
    );
    let coat = SpellEffect {
        effect: professions::EFFECT_ENCHANT_ITEM_TEMPORARY,
        misc_value: 263,
        base_points: 59,
        base_dice: 1,
        ..Default::default()
    };
    let coating = timed(COATING, coat);
    let mut coating_reagent = timed(COATING_REAGENT, coat);
    coating_reagent.reagents[0] = (REAGENT, 1);
    let mut focused = bare(FOCUSED, 0);
    focused.requires_spell_focus = FOCUS;
    focused.duration_index = LASTS_LONG;
    effect(
        &mut focused,
        0,
        aura(crate::game::map::auras::AURA_DUMMY, 0, 5, 1),
    );
    let mut heal = bare(ANY_HEAL, 1);
    effect(
        &mut heal,
        0,
        SpellEffect {
            effect: 10,
            base_points: 9,
            base_dice: 1,
            target_a: 25,
            ..Default::default()
        },
    );
    let mut both = bare(OBJECTS, 0);
    both.max_affected_targets = 2;
    effect(&mut both, 0, activate(51));
    effect(&mut both, 1, activate(51));
    let mut at_object = bare(AT_OBJECT, 0);
    at_object.range_index = RANGE_10;
    effect(&mut at_object, 0, activate(23));
    let mut open = bare(OPEN, 0);
    effect(
        &mut open,
        0,
        SpellEffect {
            effect: professions::EFFECT_OPEN_LOCK,
            misc_value: 19,
            target_a: 26,
            ..Default::default()
        },
    );
    let mut egg = bare(ROOKERY_EGG, 0);
    effect(&mut egg, 0, activate(40));
    effect(&mut egg, 1, create(EGG));
    insert(
        d,
        [
            outdoor,
            dig,
            lesson,
            bare(LEARNED, 0),
            coating,
            coating_reagent,
            focused,
            heal,
            both,
            at_object,
            open,
            egg,
        ],
    );
    // The objects' script targets: the altars for the first effect, the
    // pillars for the second; the egg's.
    d.spells
        .script_targets
        .insert(OBJECTS, vec![(0, ALTAR), (0, PILLAR)]);
    d.spells
        .script_target_masks
        .insert((OBJECTS, 0, ALTAR), 0b10);
    d.spells
        .script_target_masks
        .insert((OBJECTS, 0, PILLAR), 0b01);
    d.spells
        .script_targets
        .insert(ROOKERY_EGG, vec![(0, EGG_OBJECT)]);
    for e in [TOOL, ORE, REAGENT, EGG] {
        d.items.insert(e, item(e));
    }
    d.items.insert(
        BOOK,
        ItemTemplate {
            stackable: 1,
            required_skill: COOKING,
            required_skill_rank: 10,
            spells: on_use(LESSON, -1),
            ..item(BOOK)
        },
    );
    d.items.insert(
        VIAL,
        ItemTemplate {
            spells: on_use(COATING, -1),
            ..item(VIAL)
        },
    );
    d.items.insert(
        WEAPON,
        ItemTemplate {
            class: 2,
            inventory_type: 13,
            stackable: 1,
            ..item(WEAPON)
        },
    );
    for e in [SHARD, FILLER] {
        d.items.insert(
            e,
            ItemTemplate {
                stackable: 1,
                ..item(e)
            },
        );
    }
    let mut focus = [0u32; 24];
    (focus[0], focus[1]) = (FOCUS, 5);
    d.gameobject_templates.insert(
        FOCUS_OBJECT,
        object_template(FOCUS_OBJECT, professions::GO_SPELL_FOCUS, focus),
    );
    for e in [ALTAR, PILLAR, EGG_OBJECT] {
        d.gameobject_templates
            .insert(e, object_template(e, 0, [0; 24]));
    }
    let mut chest = [0u32; 24];
    chest[0] = LOCK;
    d.gameobject_templates
        .insert(CHEST, object_template(CHEST, professions::GO_CHEST, chest));
    d.crafts.locks.insert(
        LOCK,
        vec![LockKey {
            kind: LOCK_KEY_SKILL,
            index: 19,
            skill: 0,
        }],
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 900,
            entry: CHEST,
            map: 0,
            x: START.0 + 1.0,
            y: START.1,
            z: START.2,
            state: 1,
            respawn_min: 60,
            ..Default::default()
        }],
    );
    d.npc.quests.insert(
        QUEST,
        Quest {
            id: QUEST,
            title: "Open It".into(),
            min_level: 1,
            level: 1,
            flags: 8,
            required_kills: [(-(CHEST as i32), 1), (0, 0), (0, 0), (0, 0)],
            required_spell_casts: [OPEN, 0, 0, 0],
            ..Default::default()
        },
    );
}

async fn setup(account: &str) -> (Arc<Realm>, Client, Guid, MapHandle) {
    let realm = fast_realm(data);
    // A name of letters: the account's, its digits as letters.
    let name: String = account
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let c = if c.is_ascii_digit() {
                char::from(b'a' + (c as u8 - b'0'))
            } else {
                c.to_ascii_lowercase()
            };
            if i == 0 { c.to_ascii_uppercase() } else { c }
        })
        .collect();
    let (c, g, map) = human(&realm, account, &name, Class::Mage).await;
    // Not a game master: they pay what a cast costs.
    map.query(move |m| {
        kit::ready(m, g, 60, &[]);
        m.players.get_mut(&g).unwrap().gm = false;
    })
    .await
    .unwrap();
    (realm, c, g, map)
}

/// The failure `c` was told of `spell`, if a failure.
async fn failure(c: &mut Client, spell: u32) -> Option<u8> {
    let f = c
        .wait_frame("SMSG_CAST_RESULT", SMSG_CAST_RESULT, |f| {
            u32::from_le_bytes(f[0..4].try_into().unwrap()) == spell
        })
        .await;
    (f[4] == 2).then(|| f[5])
}

fn give(m: &mut Map, g: Guid, entry: u32, count: u32) -> Guid {
    inventory::add_item(m, g, entry, count, false).unwrap();
    m.players[&g]
        .items
        .iter()
        .rev()
        .find(|i| i.entry == entry)
        .unwrap()
        .guid
}

fn take_all(m: &mut Map, g: Guid, entry: u32) {
    while let Some(i) = m.players[&g].items.iter().position(|i| i.entry == entry) {
        let n = m.players[&g].items[i].count();
        inventory::take_item(m, g, i, n);
    }
}

fn count(m: &Map, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

fn begin(m: &mut Map, g: Guid, spell: u32, t: Targets, item: Option<Guid>) {
    kit::fresh(m, g);
    spells::cast(m, g, spell, t, item);
    assert_eq!(kit::casting(m, g), Some(spell), "{spell} began");
}

fn me(g: Guid) -> Targets {
    Targets {
        unit: Some(g),
        ..Default::default()
    }
}

/// Thistle Tea's "rechecks current terrain when a cast started outside
/// completes indoors": a cast only for outdoors, begun outside, fails
/// indoors as it ends, costing nothing; outside, it goes.
#[tokio::test]
async fn a_cast_begun_outdoors_fails_indoors_as_it_ends() {
    let (_r, mut c, g, map) = setup("P11CEN").await;
    c.clear();
    let indoors = map
        .query(move |m| {
            m.spells.outdoors_override.insert(g, true);
            kit::set_power(m, g, 0, 1_000);
            begin(m, g, OUTDOOR, me(g), None);
            m.spells.outdoors_override.insert(g, false);
            pass(m, 2_100);
            (kit::has(m, g, OUTDOOR), kit::power(m, g, 0))
        })
        .await
        .unwrap();
    assert!(!indoors.0, "nothing cast");
    assert!(indoors.1 >= 1_000, "nothing paid");
    assert_eq!(failure(&mut c, OUTDOOR).await, Some(0x55));
    let outdoors = map
        .query(move |m| {
            m.spells.outdoors_override.insert(g, true);
            kit::set_power(m, g, 0, 1_000);
            begin(m, g, OUTDOOR, me(g), None);
            pass(m, 2_100);
            (kit::has(m, g, OUTDOOR), kit::power(m, g, 0))
        })
        .await
        .unwrap();
    assert!(outdoors.0, "cast");
    assert!(outdoors.1 < 1_000, "paid");
}

/// Thistle Tea's "rechecks range, world, skill and tools at completion":
/// a tool dropped while its cast goes fails it as it ends, making nothing.
#[tokio::test]
async fn a_tool_dropped_mid_cast_fails_it_as_it_ends() {
    let (_r, mut c, g, map) = setup("P11CTL").await;
    c.clear();
    let dropped = map
        .query(move |m| {
            give(m, g, TOOL, 1);
            begin(m, g, DIG, me(g), None);
            take_all(m, g, TOOL);
            pass(m, 2_100);
            count(m, g, ORE)
        })
        .await
        .unwrap();
    assert_eq!(dropped, 0, "nothing dug");
    assert_eq!(failure(&mut c, DIG).await, Some(fail::TOTEMS));
    let kept = map
        .query(move |m| {
            give(m, g, TOOL, 1);
            begin(m, g, DIG, me(g), None);
            pass(m, 2_100);
            count(m, g, ORE)
        })
        .await
        .unwrap();
    assert_eq!(kept, 1);
}

/// Thistle Tea's "missing, banked, and newly ineligible books cannot teach
/// at completion": a book banked or gone as its lesson ends teaches nothing
/// (mangos's `CheckItems`, `ITEM_NOT_READY`), nor one its reader's skill
/// has fallen short of (Thistle Tea's recheck); kept, it teaches and goes.
#[tokio::test]
async fn a_book_banked_gone_or_beyond_its_reader_teaches_nothing_as_it_ends() {
    let (_r, mut c, g, map) = setup("P11CBK").await;
    let skill = move |m: &mut Map, value: u32| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, COOKING, 1);
        let i = (0..128u16)
            .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == COOKING)
            .unwrap();
        p.fields
            .set_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 1, (300 << 16) | value);
    };
    for case in 0..3 {
        c.clear();
        let out = map
            .query(move |m| {
                skill(m, 10);
                take_all(m, g, BOOK);
                let book = give(m, g, BOOK, 1);
                begin(m, g, LESSON, me(g), Some(book));
                match case {
                    0 => {
                        let d = m.realm.data.clone();
                        let p = m.players.get_mut(&g).unwrap();
                        let i = p.item_by_guid(book).unwrap();
                        (p.items[i].bag, p.items[i].slot) =
                            (crate::game::store::BAG_PLAYER, inventory::BANK_START);
                        p.refresh_inventory(&d);
                    }
                    1 => take_all(m, g, BOOK),
                    _ => skill(m, 5),
                }
                pass(m, 2_100);
                (
                    m.players[&g].spells.contains(&LEARNED),
                    m.players[&g].item_by_guid(book).is_some() || case == 1,
                )
            })
            .await
            .unwrap();
        assert_eq!(
            out,
            (false, true),
            "case {case}: nothing taught, the book kept"
        );
        if case < 2 {
            assert_eq!(failure(&mut c, LESSON).await, Some(fail::ITEM_NOT_READY));
        } else {
            c.wait_frame(
                "SMSG_INVENTORY_CHANGE_FAILURE",
                SMSG_INVENTORY_CHANGE_FAILURE,
                |_| true,
            )
            .await;
        }
    }
    let taught = map
        .query(move |m| {
            skill(m, 10);
            take_all(m, g, BOOK);
            let book = give(m, g, BOOK, 1);
            begin(m, g, LESSON, me(g), Some(book));
            pass(m, 2_100);
            (m.players[&g].spells.contains(&LEARNED), count(m, g, BOOK))
        })
        .await
        .unwrap();
    assert_eq!(taught, (true, 0));
}

/// Thistle Tea's "rejects stale targets, dead casters, and incomplete
/// costs atomically": a coating whose weapon is gone, whose caster died or
/// whose reagent was spent as it ends coats nothing and keeps its vial.
#[tokio::test]
async fn a_coating_fails_whole_on_a_weapon_gone_a_caster_dead_or_a_reagent_spent() {
    let (_r, mut c, g, map) = setup("P11CCT").await;
    let temp = ITEM_FIELD_ENCHANTMENT + 3;
    for case in 0..3 {
        c.clear();
        let out = map
            .query(move |m| {
                if !m.unit_view(g).unwrap().alive {
                    crate::game::map::death::resurrect(m, g, 1.0);
                    kit::ready(m, g, 60, &[]);
                }
                take_all(m, g, VIAL);
                let vial = give(m, g, VIAL, 3);
                let weapon = give(m, g, WEAPON, 1);
                give(m, g, REAGENT, 1);
                let spell = if case == 2 { COATING_REAGENT } else { COATING };
                let t = Targets {
                    item: Some(weapon),
                    ..Default::default()
                };
                begin(m, g, spell, t, Some(vial));
                match case {
                    0 => take_all(m, g, WEAPON),
                    1 => crate::game::map::death::kill(m, g),
                    _ => take_all(m, g, REAGENT),
                }
                pass(m, 2_100);
                let coated = m.players[&g]
                    .item_by_guid(weapon)
                    .is_some_and(|i| m.players[&g].items[i].fields.get_u32(temp) != 0);
                (coated, count(m, g, VIAL))
            })
            .await
            .unwrap();
        assert_eq!(
            out,
            (false, 3),
            "case {case}: nothing coated, the vial kept"
        );
        if case != 1 {
            let why = if case == 0 {
                fail::ITEM_GONE
            } else {
                fail::REAGENTS
            };
            assert_eq!(
                failure(&mut c, if case == 2 { COATING_REAGENT } else { COATING }).await,
                Some(why)
            );
        }
    }
    let coated = map
        .query(move |m| {
            if !m.unit_view(g).unwrap().alive {
                crate::game::map::death::resurrect(m, g, 1.0);
            }
            take_all(m, g, VIAL);
            let vial = give(m, g, VIAL, 3);
            let weapon = give(m, g, WEAPON, 1);
            let t = Targets {
                item: Some(weapon),
                ..Default::default()
            };
            begin(m, g, COATING, t, Some(vial));
            pass(m, 2_100);
            let i = m.players[&g].item_by_guid(weapon).unwrap();
            (
                m.players[&g].items[i].fields.get_u32(temp),
                count(m, g, VIAL),
            )
        })
        .await
        .unwrap();
    assert_eq!(coated, (263, 2));
}

/// Thistle Tea's "checks the original player before projecting triggered
/// effects" (mangos-classic's `CheckCast` for a triggered spell too): a
/// player's triggered spell wanting a focus fails without one by them,
/// told so, and goes with one; a creature's goes without.
#[tokio::test]
async fn a_triggered_spell_wants_its_players_focus() {
    let (_r, mut c, g, map) = setup("P11CFC").await;
    c.clear();
    let without = map
        .query(move |m| {
            spells::cast_triggered(m, g, FOCUSED, Some(g));
            kit::has(m, g, FOCUSED)
        })
        .await
        .unwrap();
    assert!(!without);
    assert_eq!(
        failure(&mut c, FOCUSED).await,
        Some(fail::REQUIRES_SPELL_FOCUS)
    );
    let out = map
        .query(move |m| {
            let k = kit::dummy(m, g, 60, 30.0);
            spells::cast_triggered(m, k, FOCUSED, Some(k));
            let creature = kit::has(m, k, FOCUSED);
            let at = m.players[&g].position();
            objects::summon(m, None, FOCUS_OBJECT, at, 0.0, 60_000, 0, 0, None);
            spells::cast_triggered(m, g, FOCUSED, Some(g));
            (creature, kit::has(m, g, FOCUSED))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true));
}

/// Thistle Tea's "any-unit healing respects player immunity without
/// inheriting harmful-only flags", as mangos-classic has it: a heal at a
/// unit that can't be targeted (`UNIT_FLAG_UNTARGETABLE`) is refused; one
/// at a unit of other flags (not attackable, pacified) goes.
#[tokio::test]
async fn a_heal_at_an_untargetable_unit_is_refused() {
    let (realm, _c, g, map) = setup("P11CUT").await;
    let (_d, b, _) = human(&realm, "P11CUT2", "Patient", Class::Mage).await;
    let out = map
        .query(move |m| {
            let mut seen = Vec::new();
            for flags in [checks::UNIT_FLAG_UNTARGETABLE, 0x0200_0200, 0] {
                m.players
                    .get_mut(&b)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_FLAGS, flags);
                kit::fresh(m, g);
                let s = m.realm.data.spells.get(ANY_HEAL).cloned().unwrap();
                let mods = spells::mods::for_spell(m, g, &s);
                seen.push(checks::check(m, g, &s, &kit::at(b), None, false, &mods));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(out, vec![Some(fail::BAD_TARGETS), None, None]);
}

/// Thistle Tea's "caps each object's effect independently after selector
/// filtering" (mangos's `m_affectedTargetCount` on each effect's object
/// list): two effects of a spell of two targets, one at the altars and
/// one at the pillars about the caster, each work the two nearest of
/// theirs.
#[tokio::test]
async fn each_object_effect_works_at_most_its_spells_count() {
    let (_r, _c, g, map) = setup("P11COB").await;
    let out = map
        .query(move |m| {
            let at = m.players[&g].position();
            let mut placed = Vec::new();
            for entry in [ALTAR, PILLAR] {
                for d in [1.0, 2.0, 3.0] {
                    let p = wow_world_messages::vanilla::Vector3d { x: at.x + d, ..at };
                    placed
                        .push(objects::summon(m, None, entry, p, 0.0, 60_000, 0, 0, None).unwrap());
                }
            }
            kit::fresh(m, g);
            spells::cast(m, g, OBJECTS, me(g), None);
            placed
                .iter()
                .map(|o| m.objects.lock_overrides.get(o) == Some(&true))
                .collect::<Vec<bool>>()
        })
        .await
        .unwrap();
    assert_eq!(out, vec![true, true, false, true, true, false]);
}

/// Thistle Tea's "validates explicit object presence, range and
/// instance": an object aimed at is worked within the spell's range, not
/// beyond it (mangos's `CheckRange`), and not if it isn't in this copy.
#[tokio::test]
async fn an_object_aimed_at_is_here_and_in_range() {
    let (_r, _c, g, map) = setup("P11CAO").await;
    let out = map
        .query(move |m| {
            let at = m.players[&g].position();
            let o = m.players[&g].movement.orientation;
            let put = |m: &mut Map, d: f32| {
                let p = wow_world_messages::vanilla::Vector3d {
                    x: at.x + d * o.cos(),
                    y: at.y + d * o.sin(),
                    z: at.z,
                };
                objects::summon(m, None, ALTAR, p, 0.0, 60_000, 0, 0, None).unwrap()
            };
            let near = put(m, 9.0);
            let far = put(m, 11.0);
            let elsewhere = crate::game::guid::game_object(ALTAR, 999_999);
            let s = m.realm.data.spells.get(AT_OBJECT).cloned().unwrap();
            let mods = spells::mods::for_spell(m, g, &s);
            let mut seen = Vec::new();
            for o in [near, far, elsewhere] {
                let t = Targets {
                    object: Some(o),
                    ..Default::default()
                };
                seen.push(checks::check(m, g, &s, &t, None, false, &mods));
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![None, Some(fail::OUT_OF_RANGE), Some(fail::BAD_TARGETS)]
    );
}

/// Thistle Tea's "defers object cast credit until live activation
/// succeeds", as mangos-classic gives it (`RewardPlayerAndGroupAtCast`
/// after the effects at an object): a lock opened by a cast credits the
/// quest asking for that cast at that object.
#[tokio::test]
async fn an_opened_lock_credits_its_cast() {
    let (_r, mut c, g, map) = setup("P11COL").await;
    let chest = crate::game::guid::game_object(CHEST, 900);
    c.sees(chest).await;
    let credited = map
        .query(move |m| {
            let d = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            p.journal.log.push(crate::game::store::Logged {
                quest: QUEST,
                ..Default::default()
            });
            crate::game::map::quests::refresh_log(p, &d);
            kit::fresh(m, g);
            spells::cast(
                m,
                g,
                OPEN,
                Targets {
                    object: Some(chest),
                    ..Default::default()
                },
                None,
            );
            m.players[&g]
                .journal
                .log
                .iter()
                .find(|l| l.quest == QUEST)
                .map_or(0, |l| l.kills[0])
        })
        .await
        .unwrap();
    assert_eq!(credited, 1);
}

/// Thistle Tea's "delivers a partial reward through the owner and commits
/// only the instances that fit" (mangos's `HandleChannelDeathItem`: `count
/// - noSpaceForCount`): three shards of a death with room for two give
/// the two, and say the rest didn't fit.
#[tokio::test]
async fn a_death_item_gives_what_fits() {
    let (_r, mut c, g, map) = setup("P11CDI").await;
    c.clear();
    let out = map
        .query(move |m| {
            let data = m.realm.data.clone();
            while inventory::fitting(&m.players[&g], &data, SHARD, 3) == Ok(3) {
                give(m, g, FILLER, 1);
            }
            let room = inventory::fitting(&m.players[&g], &data, SHARD, 3);
            let k = kit::dummy(m, g, 1, 5.0);
            crate::game::map::combat::kill(m, g, k);
            spells::outcome::give_death_items(m, k, vec![(g, SHARD, 3)]);
            (room, count(m, g, SHARD))
        })
        .await
        .unwrap();
    assert_eq!(out, (Ok(2), 2));
    c.wait_frame(
        "SMSG_INVENTORY_CHANGE_FAILURE",
        SMSG_INVENTORY_CHANGE_FAILURE,
        |_| true,
    )
    .await;
}

/// Collect Rookery Egg without its egg near goes, as mangos-classic's
/// `CheckScriptTargeting` makes a script's object optional (the user's
/// call, 2026-10-21; Thistle Tea requires it for 15958, 16447 and 24973):
/// its egg is made.
#[tokio::test]
async fn collect_rookery_egg_goes_without_its_egg() {
    let (_r, mut c, g, map) = setup("P11CRE").await;
    c.clear();
    let made = map
        .query(move |m| {
            kit::fresh(m, g);
            spells::cast(m, g, ROOKERY_EGG, me(g), None);
            count(m, g, EGG)
        })
        .await
        .unwrap();
    assert_eq!(made, 1);
    assert_eq!(failure(&mut c, ROOKERY_EGG).await, None);
}
