//! Round 11's entity rows of summons (docs/parity/matrix.csv): a pet
//! revived from its record, a vein that fills again, a ritual's channel, a
//! creature's guardian gone, an owned summon's time, a scripted summon's
//! attack target, and a fear taken while rooted.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::crafts::LockKey;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::{GameData, GameObjectSpawn, ItemTemplate, LootEntry};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::combat::dist;
use crate::game::map::creature::Motion;
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapHandle, MapKey, ai, creature_combat, event_ai, objects, pets};
use crate::game::tests_p3_cast_kit as cast_kit;
use crate::game::tests_p3_combat::{BRUTE, HELPER, PACK, START, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{on, spell, step};
use crate::game::tests_p3_spells_kit as spell_kit;
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_parity_motion as motion;
use crate::game::tests_pets::{self, cast, learn};

/// Revive Pet, made up to bring the pet back at fifteen percent of its
/// health (Thistle Tea's test asks for 15).
const REVIVE_PET: u32 = 982;

fn revive_data() -> GameData {
    let mut d = tests_pets::data();
    let none = SpellEffect::default();
    let revive = tests_pets::spell(
        REVIVE_PET,
        "Revive Pet",
        [
            tests_pets::effect(pets::EFFECT_SUMMON_DEAD_PET, 14, 1),
            none,
            none,
        ],
    );
    d.spells.spells.insert(REVIVE_PET, revive);
    d
}

/// The map `g` is on now.
fn map_of(realm: &Arc<Realm>, g: Guid) -> MapHandle {
    let key = realm.registry.online(g).expect("online").map().key;
    realm.maps.get(realm, key)
}

/// Log `c` out and back in, and whether a pet of theirs is out then.
async fn relogged_with_pet(realm: &Arc<Realm>, c: &mut Client) -> bool {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    let g = c.guid;
    c.login(g).await;
    crate::game::tests_p3_player2_ammo::handled(c).await;
    on(&map_of(realm, g), move |m| {
        m.players[&g]
            .owned
            .pet
            .is_some_and(|pet| m.creature(pet).is_some())
    })
    .await
}

/// Thistle Tea's "a summon-pet event starts an owned pet with the
/// requested health" (event_sink_test.exs:1058): in 1.12 only Revive Pet
/// calls a pet back at a share of its health. A hunter whose boar died
/// before she logged out has none out at login; her Revive Pet of fifteen
/// percent makes it from its record, hers, of its number, at fifteen
/// percent of its health (mangos-classic's `EffectSummonDeadPet`, its
/// health the effect's share of its maximum).
#[tokio::test]
async fn revive_pet_calls_a_pet_dead_at_logout_back_with_its_share_of_health() {
    let (realm, mut h, gh, map, pet, wolf) = tamed_with(revive_data()).await;
    h.sees(pet).await;
    let number = on(&map, move |m| {
        m.remove_creature(wolf);
        let n = m
            .creature(pet)
            .unwrap()
            .fields
            .get_u32(UNIT_FIELD_PETNUMBER);
        pets::testing::die(m, pet);
        // Out of the taming's fight, so she may log out.
        crate::game::map::combat::leave_combat(m, gh);
        n
    })
    .await;
    assert!(
        !relogged_with_pet(&realm, &mut h).await,
        "dead at logout, it stays away"
    );
    let map = map_of(&realm, gh);
    let away = on(&map, move |m| {
        let p = &m.players[&gh];
        (
            p.owned.pet,
            p.row.state.pets.current.as_ref().map(|r| r.dead),
        )
    })
    .await;
    assert_eq!(
        away,
        (None, Some(true)),
        "not in the world, its record dead"
    );
    learn(&map, gh, &[REVIVE_PET]);
    cast(&h, REVIVE_PET, None).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    let back = loop {
        let back = on(&map, move |m| {
            let g = m.players[&gh].owned.pet?;
            let c = m.creature(g)?;
            c.alive().then(|| {
                (
                    c.fields.get_u32(UNIT_FIELD_PETNUMBER),
                    c.fields.get_guid(UNIT_FIELD_SUMMONEDBY),
                    c.fields.get_u32(UNIT_FIELD_HEALTH),
                    c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
                )
            })
        })
        .await;
        if let Some(b) = back {
            break b;
        }
        assert!(tokio::time::Instant::now() < deadline, "revived");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let (n, by, health, max) = back;
    assert_eq!((n, by), (number, gh), "her pet, of its number");
    assert!(max > 0);
    assert_eq!(health, max * 15 / 100, "fifteen percent of {max}");
}

/// tests_loot's vein: Copper Ore, a pick, Mining and its lock.
const ORE: u32 = 2770;
const MINING_PICK: u32 = 2901;
const MINING: u32 = 2575;
const SKILL_MINING: u32 = 186;
const VEIN: u32 = 1731;

/// A copper vein two yards from where humans start, of two to four
/// gatherings (its template's data4 and data5), and Mining with its pick.
fn vein_data() -> GameData {
    let mut d = GameData::empty();
    for (entry, name, class, subclass) in [
        (ORE, "Copper Ore", 7, 0),
        (MINING_PICK, "Mining Pick", 2, 14),
    ] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class,
                subclass,
                name: name.into(),
                quality: 1,
                stackable: 20,
                ..Default::default()
            },
        );
    }
    let mut vein = crate::game::tests_p3_entity_objects::object(
        VEIN,
        objects::GO_CHEST,
        "Copper Vein",
        &[(0, 38), (1, 1502), (4, 2), (5, 4)],
    );
    vein.display_id = 310;
    d.gameobject_templates.insert(VEIN, vein);
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 50,
            entry: VEIN,
            map: 0,
            x: START.0 - 2.0,
            y: START.1,
            z: START.2,
            state: 1,
            respawn_min: 60,
            ..Default::default()
        }],
    );
    d.crafts.locks.insert(
        38,
        vec![LockKey {
            kind: 2,
            index: 3,
            skill: 0,
        }],
    );
    d.loot.gameobject.insert(
        1502,
        vec![LootEntry {
            item: ORE,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    let mut mining = SpellTemplate {
        id: MINING,
        name: "Mining".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    mining.effects[0] = SpellEffect {
        effect: 33,
        misc_value: 3,
        base_points: -1,
        target_a: 23,
        ..Default::default()
    };
    mining.totems = [MINING_PICK, 0];
    d.spells.spells.insert(MINING, mining);
    d
}

/// `g` mines `vein` from a fresh start until it opens (a low skill
/// sometimes fails): whether it did.
async fn mine(map: &MapHandle, g: Guid, vein: Guid) -> bool {
    for _ in 0..40 {
        let opened = on(map, move |m| {
            m.now = Instant::now();
            if let Some(p) = m.players.get_mut(&g) {
                p.caster.gcd_until = None;
                p.caster.casting = None;
            }
            crate::game::map::spells::cast(
                m,
                g,
                MINING,
                Targets {
                    object: Some(vein),
                    ..Default::default()
                },
                None,
            );
            m.players.get(&g).is_some_and(|p| p.looting == Some(vein))
        })
        .await;
        if opened {
            return true;
        }
    }
    false
}

/// Take the ore and shut the window.
async fn take_and_shut(a: &mut Client, vein: Guid) {
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("the ore", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_LOOT_RELEASE { guid: vein }).await;
    a.wait_for("the window shut", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) if r.guid == vein => Some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's "replenishes a vein, requires another cast, and clears
/// gain history on respawn" (game_object/open_lock_test.exs:86): a vein of
/// two to four gatherings mined once fills again, its use counted and its
/// miner's gain kept, its loot gone, so a look without another cast is
/// refused; cast again and mined at its fourth, it goes, its uses and
/// gains with it, and comes back on its spawn's time with none (where
/// Thistle Tea clears them at the respawn; mangos-classic's chest
/// `minRestock` and `maxRestock`).
#[tokio::test]
async fn a_vein_fills_again_until_its_last_gathering_and_goes_with_its_history() {
    let realm = harness::realm_with(vein_data());
    let mut a = Client::connect(&realm, "Miner");
    let g = a.enter("Miner", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let vein = guid::game_object(VEIN, 50);
    a.sees(vein).await;
    on(&map, move |m| {
        crate::game::tests_loot::learn(m, g, MINING, SKILL_MINING);
        let _ = crate::game::map::inventory::add_item(m, g, MINING_PICK, 1, false);
    })
    .await;
    assert!(mine(&map, g, vein).await, "the vein opens");
    take_and_shut(&mut a, vein).await;
    let refilled = on(&map, move |m| {
        (
            m.loot_state.node_uses.get(&vein).copied(),
            m.gameobject(vein).map(|o| o.loot.is_none()),
            m.loot_state
                .skilled
                .get(&vein)
                .is_some_and(|who| who.contains(&g)),
        )
    })
    .await;
    assert_eq!(
        refilled,
        (Some(1), Some(true), true),
        "a use, filled again, its gain kept"
    );
    // A look without a cast is refused.
    a.clear();
    a.send(CMSG_LOOT { guid: vein }).await;
    a.wait_for("the refusal", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) if r.guid == vein => Some(()),
        _ => None,
    })
    .await;
    let shown = a
        .gets(Duration::from_millis(100), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOOT_RESPONSE(_)).then_some(())
        })
        .await;
    assert!(shown.is_none(), "no window without a cast");
    let looting = on(&map, move |m| m.players[&g].looting).await;
    assert_eq!(looting, None);
    // Its third use was its last but one: the fourth is its last.
    on(&map, move |m| {
        m.loot_state.node_uses.insert(vein, 3);
    })
    .await;
    assert!(mine(&map, g, vein).await, "another cast opens it");
    take_and_shut(&mut a, vein).await;
    a.forgets(vein).await;
    let gone = on(&map, move |m| {
        (
            m.gameobject(vein).is_none(),
            m.loot_state.node_uses.contains_key(&vein),
            m.loot_state.skilled.contains_key(&vein),
            m.loot_state.respawns.len(),
        )
    })
    .await;
    assert_eq!(gone, (true, false, false, 1), "gone, with its history");
    let back = on(&map, move |m| {
        let now = m.now;
        for r in m.loot_state.respawns.iter_mut() {
            r.0 = now;
        }
        crate::game::map::professions::respawn_nodes(m);
        (
            m.gameobject(vein).map(|o| o.loot.is_none()),
            m.loot_state.node_uses.contains_key(&vein),
            m.loot_state.skilled.contains_key(&vein),
        )
    })
    .await;
    assert_eq!(back, (Some(true), false, false), "back, fresh");
}

/// An altar, and two channels that raise it: one of the caster's (effect
/// 50, a ritual's), one wild (76).
const ALTAR: u32 = 98_620;
const OWNED_RITUAL: u32 = 98_621;
const WILD_RITUAL: u32 = 98_622;

fn ritual_data(d: &mut GameData) {
    d.gameobject_templates.insert(
        ALTAR,
        crate::game::tests_p3_entity_objects::object(ALTAR, objects::GO_RITUAL, "Altar", &[(0, 2)]),
    );
    let ritual = |id: u32, effect: u32| {
        let mut s = cast_kit::bare(id, 0);
        s.attributes[1] |= ATTR_EX_CHANNELED;
        s.duration_index = cast_kit::LASTS_15S;
        cast_kit::effect(
            &mut s,
            0,
            SpellEffect {
                effect,
                misc_value: ALTAR as i32,
                target_a: 1,
                ..Default::default()
            },
        );
        s
    };
    cast_kit::insert(
        d,
        [
            ritual(OWNED_RITUAL, objects::EFFECT_TRANS_DOOR),
            ritual(WILD_RITUAL, objects::EFFECT_SUMMON_OBJECT_WILD),
        ],
    );
}

fn channel_object(m: &Map, g: Guid) -> Guid {
    m.unit_fields(g)
        .map_or(Guid::zero(), |f| f.get_guid(UNIT_FIELD_CHANNEL_OBJECT))
}

/// Thistle Tea's "only an owned ritual attaches to its creator's channel"
/// (game_object_summons_test.exs:105): a channel that raises an altar of
/// its caster's puts the caster's channel on it; one that raises it wild
/// raises it, of no one's, and the channel stays on its target (the
/// caster, its effect's), not on the altar (mangos-classic's
/// `EffectTransmitted` setting the channel's object,
/// `EffectSummonObjectWild` not).
#[tokio::test]
async fn only_an_owned_ritual_puts_its_casters_channel_on_its_object() {
    let realm = cast_kit::fast_realm(ritual_data);
    let (_c, a, map) = cast_kit::human(&realm, "P11S3", "Ritualist", Class::Warlock).await;
    let out = on(&map, move |m| {
        m.spells.rolls = Some(false);
        spell_kit::ready(m, a, 60, &[OWNED_RITUAL, WILD_RITUAL]);
        let mut out = Vec::new();
        for spell in [OWNED_RITUAL, WILD_RITUAL] {
            let cast = spell_kit::cast_fresh(m, a, spell, Targets::default());
            let altar = m
                .objects
                .summoned
                .iter()
                .find(|s| s.spell == spell)
                .map(|s| (s.guid, s.owner));
            out.push((cast, spell_kit::casting(m, a), altar, channel_object(m, a)));
        }
        out
    })
    .await;
    let (cast, casting, altar, channel) = out[0];
    let (owned, owner) = altar.expect("the owned altar");
    assert_eq!((cast, casting), (true, Some(OWNED_RITUAL)));
    assert_eq!(owner, Some(a), "the caster's");
    assert_eq!(channel, owned, "its channel on the altar");
    let (cast, casting, altar, channel) = out[1];
    let (wild, owner) = altar.expect("the wild altar");
    assert_eq!((cast, casting), (true, Some(WILD_RITUAL)));
    assert_eq!(owner, None, "no one's");
    assert_ne!(channel, wild, "its channel not on the wild altar");
    assert_eq!(channel, a, "but on its target, the caster herself");
}

/// A creature's guardian, and the spell that summons one for a minute.
const WHELP: u32 = 98_610;
const SUMMON_WHELP: u32 = 98_611;

fn whelp_world() -> GameData {
    let mut d = data();
    d.spells.durations.insert(
        4,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    let mut t = template(WHELP, 2);
    t.faction = PACK;
    d.creature_templates.insert(WHELP, t);
    let mut s = spell(
        SUMMON_WHELP,
        &[SpellEffect {
            effect: pets::EFFECT_SUMMON_GUARDIAN,
            misc_value: WHELP as i32,
            base_points: 0,
            base_dice: 1,
            target_a: 32,
            ..Default::default()
        }],
    );
    s.duration_index = 4;
    d.spells.spells.insert(SUMMON_WHELP, s);
    d
}

/// `owner`'s living guardians in the map, as tests_p4_world_guardians
/// lists them.
fn guardians(m: &Map, owner: Guid) -> Vec<Guid> {
    let mut v: Vec<Guid> = m
        .creatures
        .iter()
        .filter(|(_, c)| {
            c.alive()
                && (c.owner == Some(owner)
                    || c.scripted.as_deref().and_then(|s| s.summoner) == Some(owner))
        })
        .map(|(_, c)| c.guid)
        .collect();
    v.sort();
    v
}

/// The angle about its summoner a guardian follows at.
fn follow_angle(m: &Map, g: Guid) -> Option<f32> {
    m.creature(g)?.scripted.as_deref()?.follow.map(|f| f.2)
}

/// Thistle Tea's "child termination clears the slot and all child
/// projections" (mob/creature_pet_owner_test.exs:79): a creature's
/// guardian that goes leaves it none, its place behind it free, and its
/// next summoning makes a fresh one there, in the first place (wvs lists
/// a creature's guardians from the living; mangos-classic's
/// `Unit::RemoveGuardian` as one goes).
#[tokio::test]
async fn a_creatures_guardian_gone_leaves_it_none_and_the_next_takes_its_place() {
    let (_realm, _a, ga, map) = setup_with(whelp_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let owner = spawn(m, ga, BRUTE, 20, 6.0);
        let s = m.realm.data.spells.get(SUMMON_WHELP).cloned().unwrap();
        let summon = |m: &mut Map| {
            pets::effect(m, owner, owner, &s, 0, 1, false, &Default::default());
        };
        summon(m);
        let first = guardians(m, owner);
        let first_angle = first.first().and_then(|g| follow_angle(m, *g));
        if let Some(g) = first.first() {
            crate::game::map::scripts::despawn(m, *g);
        }
        let gone = first.iter().all(|g| m.creature(*g).is_none());
        let after = guardians(m, owner);
        summon(m);
        let again = guardians(m, owner);
        let again_angle = again.first().and_then(|g| follow_angle(m, *g));
        (first, first_angle, gone, after, again, again_angle)
    })
    .await;
    let (first, first_angle, gone, after, again, again_angle) = out;
    assert_eq!(first.len(), 1, "its guardian");
    assert!(gone, "out of the world");
    assert!(after.is_empty(), "it has none: {after:?}");
    assert_eq!(again.len(), 1, "a fresh one");
    assert_ne!(again[0], first[0]);
    assert!(first_angle.is_some());
    assert_eq!(again_angle, first_angle, "in the place the first had");
}

/// Thistle Tea's "does not hold owned pets that are not charmed"
/// (mob/respawn_test.exs:96): a script's summon of despawn type 3, a
/// creature's and not charmed, goes at its time though it fights, where
/// one of type 1 waits its fight out (vmangos's `TEMPSUMMON_TIMED_DESPAWN`
/// and `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN`, as mangos-classic's
/// `TemporarySummon::Update` runs them).
#[tokio::test]
async fn an_owned_summon_of_type_three_goes_at_its_time_though_it_fights() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let summoner = spawn(m, ga, BRUTE, 20, 10.0);
        let at = m.creature(summoner).unwrap().position;
        let after = Duration::from_secs(60);
        let mut summons = Vec::new();
        for kind in [3, 1] {
            let g =
                crate::game::map::scripts::summon(m, summoner, BRUTE, at, 0.0, Some((after, kind)))
                    .unwrap();
            m.creature_mut(g).unwrap().owner = Some(summoner);
            ai::attacked(m, g, ga);
            summons.push(g);
        }
        let fighting: Vec<bool> = summons
            .iter()
            .map(|g| m.combat(*g).is_some_and(|c| c.attacking))
            .collect();
        m.now += after + Duration::from_secs(1);
        event_ai::update_despawns(m);
        let there: Vec<bool> = summons.iter().map(|g| m.creature(*g).is_some()).collect();
        (fighting, there)
    })
    .await;
    let (fighting, there) = out;
    assert_eq!(fighting, [true, true], "both fight");
    assert_eq!(there, [false, true], "type 3 goes; type 1 waits");
}

/// Thistle Tea's "attack the target their script resolved rather than the
/// script's target" (wild_summon_test.exs:128): a summon step run at
/// Alice whose attack target type is its source's victim (1) attacks that
/// victim, the host's foe, not Alice; with the provided target's type (0),
/// her (vmangos's `SummonCreature` script command, its attack target found
/// by `GetTargetByType` from its third dataint).
#[tokio::test]
async fn a_scripted_summon_attacks_the_target_its_type_resolves_not_the_steps() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let host = spawn(m, ga, BRUTE, 20, 5.0);
        let foe = spawn(m, ga, HELPER, 20, 8.0);
        ai::attacked(m, host, foe);
        let host_victim = m.combat(host).map(|c| c.victim);
        let summon = |attack: i32| ScriptStep {
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            dataint: [0, 0, attack, 0],
            ..step(10, [BRUTE, 60_000, 0, 0])
        };
        let mut victims = Vec::new();
        for attack in [1, 0] {
            let before: Vec<Guid> = m.creatures.iter().map(|(_, c)| c.guid).collect();
            crate::game::map::scripts::execute(m, &summon(attack), host, ga);
            let new = m
                .creatures
                .iter()
                .map(|(_, c)| c)
                .find(|c| c.entry == BRUTE && !before.contains(&c.guid))
                .map(|c| c.guid)
                .expect("the summon");
            victims.push(m.combat(new).filter(|c| c.attacking).map(|c| c.victim));
        }
        (foe, host_victim, victims)
    })
    .await;
    let (foe, host_victim, victims) = out;
    assert_eq!(host_victim, Some(foe), "the host fights its foe");
    assert_eq!(victims[0], Some(foe), "its source's victim, not Alice");
    assert_eq!(victims[1], Some(ga), "the step's target");
}

/// tests_parity_motion's fear.
const FEAR: u32 = 92_001;

/// The wolf's run under way: its spline id.
fn run_of(m: &Map, g: Guid) -> Option<u32> {
    match &m.creature(g)?.motion {
        Motion::Moving(w) => Some(w.spline_id),
        Motion::Idle { .. } => None,
    }
}

/// Thistle Tea's "observes the fear caster beyond ordinary perception
/// range" (ai_environment_test.exs:323): a wolf rooted, then feared by
/// Alice sixty yards off, stands where it is and runs nowhere, its
/// watcher sent no run; freed, it runs as from a frightener sixty yards
/// off, four to ten yards, not the eleven or more of one at its feet
/// (Thistle Tea's `Fear.destination_request`; mangos-classic's
/// `FleeingMovementGenerator` waiting while the unit can't move).
#[tokio::test]
async fn a_creature_rooted_then_feared_from_afar_stands_until_free() {
    let mut d = motion::wolf_world();
    d.creature_spawns.insert(
        0,
        vec![motion::spawn(80, motion::WOLF, motion::at(60.0, 0.0))],
    );
    let (_realm, mut a, ga, map) = motion::enter(d).await;
    let g = motion::wolf();
    a.sees(g).await;
    a.clear();
    let (far, held) = on(&map, move |m| {
        let here = m.creature(g).unwrap().position;
        let far = dist(here, m.players[&ga].position());
        motion::apply(m, ga, g, motion::ROOT);
        motion::apply(m, ga, g, FEAR);
        let mut held = Vec::new();
        for _ in 0..3 {
            let stepped = creature_combat::controlled_step(m, g);
            let stays = dist(m.creature(g).unwrap().position, here) < 0.01;
            held.push((stepped, run_of(m, g), stays));
            m.now += Duration::from_millis(1500);
        }
        (far, held)
    })
    .await;
    assert!((59.0..61.0).contains(&far), "sixty yards off: {far}");
    assert!(
        held.iter().all(|h| *h == (true, None, true)),
        "held where it is: {held:?}"
    );
    let ran = a
        .gets(Duration::from_millis(300), |msg| match msg {
            ServerOpcodeMessage::SMSG_MONSTER_MOVE(x)
                if x.guid == g
                    && matches!(
                        x.move_type,
                        SMSG_MONSTER_MOVE_MonsterMoveType::Normal { .. }
                    ) =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(ran.is_none(), "its watcher sent no run");
    let run = on(&map, move |m| {
        crate::game::map::auras::remove_spell(m, g, motion::ROOT, None);
        creature_combat::controlled_step(m, g);
        let c = m.creature(g).unwrap();
        match &c.motion {
            Motion::Moving(w) => Some(dist(w.destination(), c.position)),
            Motion::Idle { .. } => None,
        }
    })
    .await;
    let run = run.expect("free, it runs in fear");
    assert!(
        (3.99..=10.01).contains(&run),
        "as from a frightener sixty yards off: {run}"
    );
    // And now its watcher is sent the run.
    a.wait_for("the run", |msg| match msg {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(x)
            if x.guid == g
                && matches!(
                    x.move_type,
                    SMSG_MONSTER_MOVE_MonsterMoveType::Normal { .. }
                ) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
}

/// A lamp raised for a minute whose spell's minute of cooldown waits for
/// it to go (`SPELL_ATTR_COOLDOWN_ON_EVENT`).
const LAMP: u32 = 98_630;
const RAISE_LAMP: u32 = 98_631;
const LASTS_A_MINUTE: u32 = 98;

fn lamp_data(d: &mut GameData) {
    d.gameobject_templates.insert(
        LAMP,
        crate::game::tests_p3_entity_objects::object(LAMP, objects::GO_GOOBER, "Lamp", &[]),
    );
    d.spells.durations.insert(
        LASTS_A_MINUTE,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    let mut s = cast_kit::bare(RAISE_LAMP, 0);
    s.attributes[0] |= 0x200_0000;
    s.recovery_time = 60_000;
    s.duration_index = LASTS_A_MINUTE;
    cast_kit::effect(
        &mut s,
        0,
        SpellEffect {
            effect: objects::EFFECT_TRANS_DOOR,
            misc_value: LAMP as i32,
            target_a: 1,
            ..Default::default()
        },
    );
    cast_kit::insert(d, [s]);
}

/// Thistle Tea's "logout saves an activated object cooldown"
/// (game_object_summons_test.exs:288): a lamp raised by a spell whose
/// cooldown waits for it, up as its raiser logs out, goes with her, its
/// cooldown started, so she comes back with it running (mangos-classic's
/// `RemoveAllGameObjects` as she leaves the world, `AddCooldown` for
/// `SPELL_ATTR_COOLDOWN_ON_EVENT`), where the lamp went at the next
/// update, after her save, and the cooldown with it.
#[tokio::test]
async fn a_cooldown_waiting_for_its_object_starts_as_its_raiser_logs_out_and_is_kept() {
    let realm = cast_kit::fast_realm(lamp_data);
    let (mut c, g, map) = cast_kit::human(&realm, "P11S8", "Lamplighter", Class::Mage).await;
    let raised = on(&map, move |m| {
        spell_kit::ready(m, g, 60, &[RAISE_LAMP]);
        spell_kit::cast_fresh(m, g, RAISE_LAMP, Targets::default());
        let up = m
            .objects
            .summoned
            .iter()
            .any(|s| s.owner == Some(g) && s.spell == RAISE_LAMP);
        let c = m.caster(g).unwrap();
        (
            up,
            c.pending_cooldowns.contains(&RAISE_LAMP),
            c.cooldowns.contains_key(&RAISE_LAMP),
        )
    })
    .await;
    assert_eq!(raised, (true, true, false), "up, its cooldown waiting");
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    c.login(g).await;
    crate::game::tests_p3_player2_ammo::handled(&mut c).await;
    let map = map_of(&realm, g);
    let back = on(&map, move |m| {
        let now = m.now;
        let left = m
            .caster(g)
            .and_then(|c| c.cooldowns.get(&RAISE_LAMP))
            .map(|t| t.saturating_duration_since(now));
        let lamp = m.objects.summoned.iter().any(|s| s.spell == RAISE_LAMP);
        (left, lamp)
    })
    .await;
    let (left, lamp) = back;
    assert!(!lamp, "the lamp went with her");
    assert!(
        left.is_some_and(|l| l > Duration::from_secs(50)),
        "its cooldown running: {left:?}"
    );
}
