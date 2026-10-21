//! Round 10's entity tests (phase 2, the rows wvs did partly or
//! differently), ported from Thistle Tea's `test/game/world/entity/`
//! (docs/parity/matrix.csv): a wild summon whose loot is its creator's,
//! and where a summon stands.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::{Map, pets};
use crate::game::tests_pets::{data, effect, enter, learn, spell, template};

/// An engineer's dummy, made up: its loot its creator's, no experience
/// (vmangos's static flags `CREATOR_LOOT` and `NO_XP`).
const DUMMY: u32 = 990_211;
/// A plain wild summon.
const PLAIN: u32 = 990_212;
const SUMMON_DUMMY: u32 = 990_213;
const SUMMON_PLAIN: u32 = 990_214;

fn wild_world() -> GameData {
    let mut d = data();
    let none = SpellEffect::default();
    for (id, entry, flags) in [
        (SUMMON_DUMMY, DUMMY, pets::CREATOR_LOOT | 0x2),
        (SUMMON_PLAIN, PLAIN, 0),
    ] {
        let mut t = template(entry, "Wild Summon", 14, 1);
        t.static_flags1 = flags;
        d.creature_templates.insert(entry, t);
        let mut e = effect(pets::EFFECT_SUMMON_WILD, 0, 1);
        e.misc_value = entry as i32;
        d.spells
            .spells
            .insert(id, spell(id, "Summon", [e, none, none]));
    }
    d
}

fn summoned(m: &Map, entry: u32) -> Option<Guid> {
    m.creatures
        .iter()
        .find(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
}

/// A player's wild summon whose loot is its creator's (an engineer's
/// Target Dummy, vmangos's `CREATOR_LOOT`) is made by them and tapped for
/// them as it comes, so a stranger's blow takes nothing; one of no
/// experience gives its creator none; a plain one is no one's until
/// struck, and gives its killer experience (Thistle Tea's "creator loot
/// claims salvage and honors no experience"; mangos's `AIM_Initialize`,
/// `SetLootRecipient(GetCreator())`). wvs tapped none for its creator.
#[tokio::test]
async fn a_creators_wild_summon_is_tapped_for_them_and_gives_no_experience() {
    let realm = crate::game::harness::realm_with(wild_world());
    let (_a, me, map) = enter(&realm, "A", "Engineer", Race::Human, Class::Warrior).await;
    let (_b, stranger, _) = enter(&realm, "B", "Stranger", Race::Human, Class::Warrior).await;
    learn(&map, me, &[SUMMON_DUMMY, SUMMON_PLAIN]);
    let out = map
        .query(move |m| {
            crate::game::map::spells::cast(m, me, SUMMON_DUMMY, Default::default(), None);
            crate::game::map::spells::cast(m, me, SUMMON_PLAIN, Default::default(), None);
            let dummy = summoned(m, DUMMY)?;
            let plain = summoned(m, PLAIN)?;
            let made = |g: Guid| {
                let c = m.creature(g).unwrap();
                (
                    c.fields.get_guid(UNIT_FIELD_CREATEDBY),
                    c.fields.get_guid(UNIT_FIELD_SUMMONEDBY),
                    c.tapped_by,
                )
            };
            let (dummy_made, plain_made) = (made(dummy), made(plain));
            // A stranger's blow takes neither the dummy's tap nor its kill.
            crate::game::map::combat::deal_damage(m, stranger, dummy, 1);
            let still = m.creature(dummy).unwrap().tapped_by;
            let xp = |m: &Map| m.players[&me].fields.get_u32(PLAYER_XP);
            let before = xp(m);
            crate::game::map::combat::deal_damage(m, me, dummy, 100_000);
            let from_dummy = xp(m) - before;
            crate::game::map::combat::deal_damage(m, me, plain, 100_000);
            let from_plain = xp(m) - before - from_dummy;
            Some((dummy_made, plain_made, still, from_dummy, from_plain))
        })
        .await
        .unwrap()
        .expect("both summoned");
    let (dummy_made, plain_made, still, from_dummy, from_plain) = out;
    assert_eq!(
        dummy_made,
        (me, Guid::zero(), Some(me)),
        "made by its caster, tapped for them"
    );
    assert_eq!(
        plain_made,
        (Guid::zero(), Guid::zero(), None),
        "no one's until struck"
    );
    assert_eq!(still, Some(me), "a stranger's blow takes no tap");
    assert_eq!(from_dummy, 0, "no experience from a no-experience summon");
    assert!(from_plain > 0, "a plain one gives its killer experience");
}

/// The entry a script turns a creature into.
const CHANGED: u32 = 990_217;

/// Script command 27 (`update_entry`) makes a creature another entry, the
/// same unit: its watcher is told the new entry, looks, level and npc
/// flags in its values, under the same GUID, and the new entry's query
/// answers its name (Thistle Tea's "publishes current identity and client
/// fields while retaining the process"). The level is the new entry's,
/// rolled in its range whatever the old one was (mangos's `SelectLevel`
/// at `UpdateEntry`): wvs kept an old level that fell in the new range.
#[tokio::test]
async fn an_update_entry_tells_its_watcher_the_new_entry_and_level() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let mut d = data();
    let mut changed = template(CHANGED, "Changed", 14, 1);
    changed.display_ids = [102, 0, 0, 0];
    changed.level_min = 15;
    changed.level_max = 15;
    changed.npc_flags = 3;
    d.creature_templates.insert(CHANGED, changed);
    // One of 10 to 20, at 12: the old level in the new range.
    let mut ranged = template(CHANGED + 1, "Ranged", 14, 1);
    ranged.level_min = 10;
    ranged.level_max = 20;
    d.creature_templates.insert(CHANGED + 1, ranged);
    let realm = crate::game::harness::realm_with(d);
    let (mut a, me, map) = enter(&realm, "A", "Watcher", Race::Human, Class::Warrior).await;
    let wolf =
        crate::game::tests_pets::place(&map, me, crate::game::tests_pets::WOLF, 0x70_0002, 5.0)
            .await;
    a.sees(wolf).await;
    map.query(move |m| {
        let step = crate::game::data::scripts::ScriptStep {
            command: 27,
            datalong: [CHANGED, 0, 0, 0],
            ..Default::default()
        };
        crate::game::map::scripts::execute(m, &step, wolf, wolf);
    })
    .await
    .unwrap();
    let told = a
        .wait_for("the new entry's values", |m| {
            crate::game::harness::values(m)
                .into_iter()
                .find(|(g, _)| *g == wolf)
                .map(|(_, mask)| crate::game::tests_bugs6::fields(&mask))
                .filter(|f| f.get(&OBJECT_FIELD_ENTRY) == Some(&CHANGED))
        })
        .await;
    assert_eq!(told.get(&UNIT_FIELD_DISPLAYID), Some(&102), "its looks");
    assert_eq!(told.get(&UNIT_FIELD_LEVEL), Some(&15), "its level");
    assert_eq!(told.get(&UNIT_NPC_FLAGS), Some(&3), "its npc flags");
    a.send(CMSG_CREATURE_QUERY {
        creature: CHANGED,
        guid: wolf,
    })
    .await;
    let name = a
        .wait_for("its query's answer", |m| match m {
            ServerOpcodeMessage::SMSG_CREATURE_QUERY_RESPONSE(r) if r.creature_entry == CHANGED => {
                Some(r.found.clone())
            }
            _ => None,
        })
        .await
        .expect("it is known")
        .name1;
    assert_eq!(name, "Changed");
    // From level 12 to an entry of 10 to 20: rolled anew, not kept.
    let levels = map
        .query(move |m| {
            let mut seen = std::collections::BTreeSet::new();
            for i in 0..40 {
                let c = m.creature_mut(wolf).unwrap();
                c.fields.set_u32(UNIT_FIELD_LEVEL, 12);
                let to = if i % 2 == 0 { CHANGED + 1 } else { CHANGED };
                crate::game::map::scripts::update_entry(m, wolf, to);
                if to == CHANGED + 1 {
                    seen.insert(m.creature(wolf).unwrap().fields.get_u32(UNIT_FIELD_LEVEL));
                }
            }
            seen
        })
        .await
        .unwrap();
    assert!(
        levels.len() > 1 && levels.iter().all(|l| (10..=20).contains(l)),
        "rolled in its range: {levels:?}"
    );
}

/// The Mechanical Dragonling's trinket spell, its summon, and made-up
/// stand-ins for its dragonling (of level 47) and its trinket.
const DRAGONLING_TRINKET_SPELL: u32 = 23_076;
const DRAGONLING_SUMMON: u32 = 4073;
const DRAGONLING: u32 = 990_218;
const TRINKET: u32 = 990_219;
/// A guardian of level 5 to 8, its summon.
const HELPER: u32 = 990_220;
const SUMMON_HELPER: u32 = 990_221;

/// A player's guardian is at its template's level, as near its owner's as
/// that allows: a level-1 player's helper of 5 to 8 at 5, a dragonling
/// cast without its trinket at its 47; one an engineering trinket calls
/// is at its owner's engineering over five, 300 making a level-60
/// dragonling (mangos-classic's `EffectSummonGuardian`, `skill202 / 5` for
/// a `SKILL_ENGINEERING` trinket; Thistle Tea's "engineering trinkets
/// scale statistics without changing template defaults"). wvs gave every
/// player's guardian its owner's level, and never looked at the trinket.
#[tokio::test]
async fn a_players_guardian_is_at_its_level_and_an_engineers_by_the_skill() {
    let mut d = data();
    let none = SpellEffect::default();
    let mut dragonling = template(DRAGONLING, "Dragonling", 14, 1);
    (dragonling.level_min, dragonling.level_max) = (47, 47);
    d.creature_templates.insert(DRAGONLING, dragonling);
    let mut helper = template(HELPER, "Helper", 14, 1);
    (helper.level_min, helper.level_max) = (5, 8);
    d.creature_templates.insert(HELPER, helper);
    for level in 1..=63u8 {
        d.class_level_stats
            .entry((1, level))
            .or_insert(crate::game::data::ClassLevelStats {
                health: 40 + u32::from(level) * 10,
                melee_damage: 3.0,
                ..Default::default()
            });
    }
    let guardian = |id: u32, entry: u32| {
        let mut e = effect(pets::EFFECT_SUMMON_GUARDIAN, 0, 32);
        e.misc_value = entry as i32;
        spell(id, "Guardian", [e, none, none])
    };
    for s in [
        guardian(DRAGONLING_SUMMON, DRAGONLING),
        guardian(SUMMON_HELPER, HELPER),
        spell(
            DRAGONLING_TRINKET_SPELL,
            "Mechanical Dragonling",
            [effect(3, 0, 1), none, none],
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.items.insert(
        TRINKET,
        crate::game::data::items::ItemTemplate {
            entry: TRINKET,
            name: "Mechanical Dragonling".into(),
            display_id: 1000,
            inventory_type: 12,
            required_skill: 202,
            ..Default::default()
        },
    );
    let realm = crate::game::harness::realm_with(d);
    let (_e, me, map) = enter(&realm, "E", "Engineer", Race::Human, Class::Warrior).await;
    let levels = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, me, TRINKET, 1, false).unwrap();
            let p = m.players.get_mut(&me).unwrap();
            let item = p.items.iter().find(|i| i.entry == TRINKET).unwrap().guid;
            crate::game::map::skills::learn_skill(p, 202, 4);
            crate::game::map::skills::set_value(p, 202, 300);
            let level_of = |m: &Map, entry: u32| {
                m.players[&me]
                    .owned
                    .guardians
                    .iter()
                    .rev()
                    .filter_map(|g| m.creature(*g))
                    .find(|c| c.entry == entry && c.alive())
                    .map(|c| c.fields.get_u32(UNIT_FIELD_LEVEL))
            };
            let cast = crate::game::map::spells::cast;
            cast(m, me, SUMMON_HELPER, Default::default(), None);
            let helper = level_of(m, HELPER);
            cast(m, me, DRAGONLING_SUMMON, Default::default(), None);
            let plain = level_of(m, DRAGONLING);
            cast(
                m,
                me,
                DRAGONLING_TRINKET_SPELL,
                Default::default(),
                Some(item),
            );
            let engineered = level_of(m, DRAGONLING);
            let skill = crate::game::map::skills::value_with_bonuses(&m.players[&me], 202);
            assert_eq!(skill, 300, "the engineer's skill");
            (helper, plain, engineered)
        })
        .await
        .unwrap();
    assert_eq!(levels, (Some(5), Some(47), Some(60)));
}

const DUNGEON: u32 = 33;
const WARDEN: u32 = 990_222;

/// Script command 49 (`zone_combat_pulse`) draws everyone in its dungeon
/// into the fight, their pets too, however far off: a warlock 60 yards
/// from the boss and her imp are both on its threat list, and in the
/// fight (Thistle Tea's "captures distant dungeon members and pets while
/// excluding other copies and charms"; a copy is a map of its own in
/// wvs). mangos-classic's `SetInCombatWithZone` takes the players alone.
#[tokio::test]
async fn a_dungeon_pulse_draws_in_its_players_and_their_pets_however_far() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let mut d = data();
    d.maps.insert(
        DUNGEON,
        crate::game::data::MapInfo {
            id: DUNGEON,
            internal_name: "Dungeon".into(),
            name: "Dungeon".into(),
            instance_type: 1,
            player_limit: 5,
        },
    );
    d.creature_templates
        .insert(WARDEN, template(WARDEN, "Warden", 14, 1));
    d.creature_spawns.insert(
        DUNGEON,
        vec![crate::game::data::CreatureSpawn {
            guid: 70,
            entries: vec![WARDEN],
            map: DUNGEON,
            x: 60.0,
            ..Default::default()
        }],
    );
    let mut cfg = crate::game::harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let (mut w, me, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, me, &[crate::game::tests_pets::SUMMON_IMP]);
    map.call(move |m| {
        m.teleport(
            me,
            crate::game::map::MapKey::world(DUNGEON),
            Vector3d::default(),
            0.0,
        )
    });
    w.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    w.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    w.sees(me).await;
    let inside = realm.registry.online(me).unwrap().map();
    let warden = crate::game::guid::unit(WARDEN, 70);
    let out = inside
        .query(move |m| {
            crate::game::map::spells::cast(
                m,
                me,
                crate::game::tests_pets::SUMMON_IMP,
                Default::default(),
                None,
            );
            let imp = m.players[&me].owned.pet?;
            let step = crate::game::data::scripts::ScriptStep {
                command: 49,
                ..Default::default()
            };
            crate::game::map::scripts::execute(m, &step, warden, warden);
            let c = m.creature(warden)?;
            let listed = |g: Guid| c.threat.iter().any(|(t, _)| *t == g);
            Some((
                listed(me),
                listed(imp),
                crate::game::map::combat::dist(c.position, m.position_of(me)?),
            ))
        })
        .await
        .unwrap()
        .expect("the warden and the imp");
    let (warlock, imp, far) = out;
    assert!(far > 50.0, "far off: {far}");
    assert!(warlock && imp, "the warlock and her imp: {out:?}");
}

const STONE: u32 = 990_223;
/// No such object.
const MISSING: u32 = 990_224;

/// An object summoned into a slot of its owner's sends the one there away,
/// even when the new one can't be made (its template missing): the slot
/// is left empty (mangos's `EffectSummonObject` clears the slot before
/// `Create`; Thistle Tea's "failed replacement clears its slot"). wvs
/// returned before replacing and kept the old one. A summon into the slot
/// that works replaces it.
#[tokio::test]
async fn a_slots_object_goes_even_when_its_replacement_cant_be_made() {
    let mut d = data();
    d.gameobject_templates.insert(
        STONE,
        crate::game::data::GameObjectTemplate {
            entry: STONE,
            kind: 5,
            display_id: 1,
            name: "Stone".into(),
            size: 1.0,
            ..Default::default()
        },
    );
    let realm = crate::game::harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "A", "Ritualist", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            let at = m.position_of(me).unwrap();
            let summon = |m: &mut Map, entry: u32| {
                crate::game::map::objects::summon(m, Some(me), entry, at, 0.0, 0, 1, 1, None)
            };
            let in_slot = |m: &Map| {
                m.objects
                    .summoned
                    .iter()
                    .filter(|x| x.owner == Some(me) && x.slot == 1)
                    .map(|x| x.guid)
                    .collect::<Vec<_>>()
            };
            let first = summon(m, STONE).unwrap();
            let second = summon(m, STONE).unwrap();
            let replaced = (in_slot(m), m.gameobject(first).is_some());
            let failed = summon(m, MISSING);
            (
                second,
                replaced,
                failed,
                in_slot(m),
                m.gameobject(second).is_some(),
            )
        })
        .await
        .unwrap();
    let (second, replaced, failed, left, kept) = out;
    assert_eq!(
        replaced,
        (vec![second], false),
        "the second replaces the first"
    );
    assert_eq!(failed, None, "no such object");
    assert!(
        left.is_empty() && !kept,
        "the slot is empty: {left:?} {kept}"
    );
}

const FISH: u32 = 990_225;
const WALKER: u32 = 990_226;

/// A creature of the water may go after a player the terrain has in deep
/// water though their client doesn't say it swims (one wading in, or
/// standing on the bed), and a creature of the land may not; one on dry
/// land is the walker's and not the fish's (mangos's
/// `isInAccessablePlaceFor` by the terrain's `IsInWater`; Thistle Tea's
/// "a swimming creature samples water only under units it could
/// attack"). wvs went by the client's swimming flag alone.
#[tokio::test]
async fn a_creatures_reach_takes_the_water_the_terrain_has_a_player_in() {
    let mut d = data();
    for (entry, inhabit) in [(FISH, 2), (WALKER, 1)] {
        let mut t = template(entry, "Habitat", 14, 1);
        t.inhabit_type = inhabit;
        d.creature_templates.insert(entry, t);
    }
    let realm = crate::game::harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "A", "Wader", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            let fish = crate::game::tests_pets::place_in(m, me, FISH, 0x70_0003, 3.0);
            let walker = crate::game::tests_pets::place_in(m, me, WALKER, 0x70_0004, -3.0);
            let reach = |m: &Map| {
                (
                    crate::game::map::creature_motion::may_reach(m, fish, me),
                    crate::game::map::creature_motion::may_reach(m, walker, me),
                )
            };
            let dry = reach(m);
            m.players.get_mut(&me).unwrap().misc.in_water = true;
            (dry, reach(m))
        })
        .await
        .unwrap();
    assert_eq!(out.0, (false, true), "on land: the walker's");
    assert_eq!(out.1, (true, false), "in the terrain's water: the fish's");
}

const GROUND: u32 = 990_227;
const TOUCH: u32 = 990_228;

/// A ground area's ticks on a creature are no spell hitting it: an EventAI
/// "hit by spell" event of the area's spell never fires as the area
/// lands on it again and again, though one of a spell cast at it does
/// (Thistle Tea's "ground aura refreshes apply without recipient or
/// caster spell-hit callbacks"; mangos's `SpellHit` is a cast's, which a
/// persistent area's dynamic object makes no more of).
#[tokio::test]
async fn a_ground_areas_ticks_fire_no_spell_hit_event() {
    use crate::game::data::scripts::{AiEvent, EF_REPEATABLE, EVENT_HIT_BY_SPELL, ScriptStep};
    let mut d = data();
    let none = SpellEffect::default();
    let mut ground = effect(27, 0, 0);
    ground.aura = crate::game::map::auras::AURA_PERIODIC_DAMAGE;
    ground.amplitude = 1000;
    ground.radius_index = 9;
    let mut area = spell(GROUND, "Ground", [ground, none, none]);
    area.duration_index = 4;
    d.spells.spells.insert(GROUND, area);
    d.spells
        .spells
        .insert(TOUCH, spell(TOUCH, "Touch", [effect(2, 0, 6), none, none]));
    let event = |id: u32, spell: u32, script: u32| AiEvent {
        id,
        creature: crate::game::tests_pets::WOLF,
        kind: EVENT_HIT_BY_SPELL,
        chance: 100,
        flags: EF_REPEATABLE,
        params: [spell as i32, 0, 0, 0],
        actions: [script, 0, 0],
        ..Default::default()
    };
    d.scripts.ai_events.insert(
        crate::game::tests_pets::WOLF,
        vec![event(1, GROUND, 951), event(2, TOUCH, 952)],
    );
    for (script, phase) in [(951, 1), (952, 2)] {
        d.scripts.ai_scripts.insert(
            script,
            vec![ScriptStep {
                command: 44,
                datalong: [phase, 0, 0, 0],
                ..Default::default()
            }],
        );
    }
    let realm = crate::game::harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "A", "Caster", Race::Human, Class::Mage).await;
    learn(&map, me, &[GROUND, TOUCH]);
    let out = map
        .query(move |m| {
            let wolf = crate::game::tests_pets::place_in(
                m,
                me,
                crate::game::tests_pets::WOLF,
                0x70_0005,
                5.0,
            );
            let at = m.position_of(wolf).unwrap();
            let max = m.unit_fields(wolf).unwrap().get_u32(UNIT_FIELD_MAXHEALTH) * 100;
            let f = m.unit_fields_mut(wolf).unwrap();
            f.set_u32(UNIT_FIELD_MAXHEALTH, max);
            f.set_u32(UNIT_FIELD_HEALTH, max);
            let there = crate::game::map::spells::Targets {
                dest: Some(at),
                ..Default::default()
            };
            crate::game::map::spells::cast(m, me, GROUND, there, None);
            let health = |m: &Map| m.unit_fields(wolf).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let before = health(m);
            for _ in 0..3 {
                let now = m.now;
                for a in &mut m.spells.areas {
                    a.next_tick = now;
                }
                m.spells.area_ticked.clear();
                crate::game::map::spells::update(m);
            }
            let ticked = before - health(m);
            let after_area = crate::game::map::event_ai::phase(m, wolf);
            crate::game::map::spells::cast(
                m,
                me,
                TOUCH,
                crate::game::map::spells::Targets {
                    unit: Some(wolf),
                    ..Default::default()
                },
                None,
            );
            (
                ticked,
                after_area,
                crate::game::map::event_ai::phase(m, wolf),
            )
        })
        .await
        .unwrap();
    let (ticked, after_area, after_touch) = out;
    assert!(ticked > 0, "the area ticked on it");
    assert_eq!(after_area, 0, "no spell-hit event from the area");
    assert_eq!(after_touch, 2, "a spell cast at it fires its event");
}

const NEAR_STONE: u32 = 990_229;
const FAR_STONE: u32 = 990_230;
const WORK_STONES: u32 = 990_231;

/// A script target's row spares the effects its `inverseEffectMask` names:
/// a spell of two "activate object" effects works the near stone with its
/// first (the near one's row spares its second) and the far stone with
/// its second (the far one's spares its first), where both went to the
/// nearest (mangos's `CanNotHitWithSpellEffect`, vmangos's 16 rows: Clean
/// Up Stink Bomb, Fire Cannon, Lunar Invitation; Thistle Tea's "retains
/// different recipients and applies only their effects").
#[tokio::test]
async fn a_script_targets_row_spares_the_effects_its_mask_names() {
    let mut d = data();
    for entry in [NEAR_STONE, FAR_STONE] {
        d.gameobject_templates.insert(
            entry,
            crate::game::data::GameObjectTemplate {
                entry,
                kind: 5,
                display_id: 1,
                name: "Stone".into(),
                size: 1.0,
                ..Default::default()
            },
        );
    }
    // Each effect sets the object's no-interact flag (action 16).
    let mut work = effect(crate::game::map::objects::EFFECT_ACTIVATE_OBJECT, 0, 40);
    work.misc_value = 16;
    let none = SpellEffect::default();
    d.spells.spells.insert(
        WORK_STONES,
        spell(WORK_STONES, "Work Stones", [work, work, none]),
    );
    d.spells
        .script_targets
        .insert(WORK_STONES, vec![(0, NEAR_STONE), (0, FAR_STONE)]);
    d.spells
        .script_target_spared
        .insert((WORK_STONES, 0, NEAR_STONE), 0b10);
    d.spells
        .script_target_spared
        .insert((WORK_STONES, 0, FAR_STONE), 0b01);
    let realm = crate::game::harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "A", "Worker", Race::Human, Class::Warrior).await;
    learn(&map, me, &[WORK_STONES]);
    let out = map
        .query(move |m| {
            let p = m.position_of(me).unwrap();
            let at = |dx: f32| Vector3d { x: p.x + dx, ..p };
            let summon = |m: &mut Map, entry: u32, dx: f32| {
                crate::game::map::objects::summon(m, None, entry, at(dx), 0.0, 0, 1, 0, None)
                    .unwrap()
            };
            let near = summon(m, NEAR_STONE, 3.0);
            let far = summon(m, FAR_STONE, 6.0);
            crate::game::map::spells::cast(m, me, WORK_STONES, Default::default(), None);
            let worked = |m: &Map, g: Guid| {
                m.gameobject(g)
                    .is_some_and(|o| o.fields.get_u32(GAMEOBJECT_FLAGS) & 0x10 != 0)
            };
            (worked(m, near), worked(m, far))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true), "each stone by the effect its row allows");
}

/// In a battleground a kill's honor is shared by the killer's whole side,
/// its battleground raid, near the victim and alive, at the raid's rate:
/// Alice's blow on Grim shares it with Bob and Carol, of no group of
/// theirs, a third each at three's rate, and Dave, far off, has none
/// (mangos's battleground raid, `SetBattleGroundRaid`, rewarded as a
/// group; Thistle Tea's "shares battleground team damage with the nearby
/// living roster using raid scaling"). wvs shared it by social group
/// alone, so Alice had it all.
#[tokio::test]
async fn a_battleground_kills_honor_is_shared_by_the_side_near() {
    let realm = crate::game::harness::realm();
    let mut clients = Vec::new();
    let mut guids = Vec::new();
    for (name, race) in [
        ("Alice", Race::Human),
        ("Bob", Race::Human),
        ("Carol", Race::Human),
        ("Dave", Race::Human),
        ("Grim", Race::Undead),
    ] {
        let mut c = crate::game::harness::Client::connect(&realm, name);
        guids.push(c.enter(name, race, Class::Warrior).await);
        clients.push(c);
    }
    let [alice, bob, carol, dave, grim] = guids[..] else {
        unreachable!()
    };
    let map = realm.registry.online(alice).unwrap().map();
    map.query(move |m| {
        let data = m.realm.data.clone();
        let here = m.position_of(alice).unwrap();
        for g in [alice, bob, carol, dave, grim] {
            let p = m.players.get_mut(&g).unwrap();
            crate::game::map::stats::set_level(p, &data, 60);
            p.movement.position = if g == dave {
                Vector3d {
                    x: here.x + 300.0,
                    ..here
                }
            } else {
                here
            };
        }
        let mut bg = crate::game::map::battleground::Match::new(2, m.now);
        for (g, team) in [(alice, 0), (bob, 0), (carol, 0), (dave, 0), (grim, 1)] {
            bg.scores.insert(
                g,
                crate::game::map::battleground::Score {
                    team,
                    ..Default::default()
                },
            );
        }
        m.battleground = Some(bg);
        crate::game::map::pvp::damaged(m, alice, grim, 100);
        crate::game::map::pvp::player_killed(m, alice, grim);
        m.battleground = None;
    })
    .await
    .unwrap();
    let ledger = realm.social.honor.lock();
    let today: Vec<u32> = [alice, bob, carol, dave]
        .iter()
        .map(|g| {
            ledger
                .entries
                .get(&crate::game::guid::counter(*g))
                .and_then(|e| e.days.get(&ledger.day))
                .map_or(0, |d| d.contribution as u32)
        })
        .collect();
    let each = (crate::game::honor::kill_points(60, 60, 0, 0) * crate::game::honor::group_rate(3)
        / 3.0)
        .trunc() as u32;
    assert!(each > 0);
    assert_eq!(today, vec![each, each, each, 0]);
}

const AHEAD: u32 = 990_215;
const AT_PLACE: u32 = 990_216;

/// A summon put at a spot about its summoner stands short of what hides
/// the spot from them: a wild summon five yards ahead, a totem two yards
/// out and a pet at its master's side, each behind a wall a yard off,
/// come this side of it; one a spell puts at a place it names stands
/// there, wall or no (Thistle Tea's "clips implicit minion positions at
/// terrain while retaining explicit destinations"; mangos's
/// `GetClosePoint`, a spot in sight). wvs put each at its spot.
#[tokio::test]
async fn a_summons_spot_is_short_of_what_hides_it_and_a_named_place_is_kept() {
    let mut d = wild_world();
    d.spells.radii.insert(13, 5.0);
    let none = SpellEffect::default();
    let mut ahead = effect(pets::EFFECT_SUMMON_WILD, 0, 0);
    ahead.misc_value = PLAIN as i32;
    ahead.radius_index = 13;
    d.spells
        .spells
        .insert(AHEAD, spell(AHEAD, "Ahead", [ahead, none, none]));
    let mut placed = effect(pets::EFFECT_SUMMON_WILD, 0, 0);
    placed.misc_value = DUMMY as i32;
    d.spells
        .spells
        .insert(AT_PLACE, spell(AT_PLACE, "At Place", [placed, none, none]));
    let realm = crate::game::harness::realm_with(d);
    let (_w, me, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(
        &map,
        me,
        &[
            AHEAD,
            AT_PLACE,
            crate::game::tests_pets::SUMMON_IMP,
            crate::game::tests_pets::STRENGTH_OF_EARTH_TOTEM,
        ],
    );
    let out = map
        .query(move |m| {
            let p = m.players[&me].position();
            let o = m.players[&me].movement.orientation;
            // Along and across the warlock's facing.
            let at = |along: f32, across: f32| Vector3d {
                x: p.x + along * o.cos() - across * o.sin(),
                y: p.y + along * o.sin() + across * o.cos(),
                z: p.z,
            };
            let ahead_of = |q: Vector3d| (q.x - p.x) * o.cos() + (q.y - p.y) * o.sin();
            // A wall a yard ahead and one a yard behind, ten yards wide.
            m.spells.walls.push((at(1.0, -5.0), at(1.0, 5.0)));
            m.spells.walls.push((at(-1.0, -5.0), at(-1.0, 5.0)));
            for s in [
                AHEAD,
                crate::game::tests_pets::SUMMON_IMP,
                crate::game::tests_pets::STRENGTH_OF_EARTH_TOTEM,
            ] {
                crate::game::map::spells::cast(m, me, s, Default::default(), None);
            }
            // The place the cast names, eight yards ahead, beyond the wall.
            let beyond = crate::game::map::spells::Targets {
                dest: Some(at(8.0, 0.0)),
                ..Default::default()
            };
            crate::game::map::spells::cast(m, me, AT_PLACE, beyond, None);
            let pos = |g: Option<Guid>| g.and_then(|g| m.position_of(g)).map(ahead_of);
            let owned = &m.players[&me].owned;
            Some((
                pos(summoned(m, PLAIN)).expect("ahead"),
                pos(summoned(m, DUMMY)).expect("at the place"),
                pos(owned.totems.iter().flatten().next().copied()).expect("the totem"),
                pos(owned.pet).expect("the imp"),
            ))
        })
        .await
        .unwrap()
        .expect("each summoned");
    let (wild, named, totem, pet) = out;
    assert!(
        (0.0..1.0).contains(&wild),
        "the wild summon this side of the wall ahead: {wild}"
    );
    assert!((named - 8.0).abs() < 0.01, "the named place kept: {named}");
    assert!(
        (0.0..1.0).contains(&totem),
        "the totem this side of the wall: {totem}"
    );
    assert!(
        (-1.0..=0.0).contains(&pet),
        "the pet this side of the wall behind: {pet}"
    );
}
