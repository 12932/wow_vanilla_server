//! Phase 6's loader round, the world (Thistle Tea's `world/loader/` tests
//! of spawns, links, groups, events, texts, quests' relations and the
//! like): what wvs reads of the real world, and what it does with it.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn copy(map: u32) -> MapKey {
    MapKey { map, instance: 3 }
}

fn concealed(m: &Map, g: Guid) -> bool {
    m.creature(g)
        .and_then(|c| c.scripted.as_ref())
        .is_some_and(|s| s.concealed)
}

const LORD_VALTHALAK: u32 = 301_240;
const HENZE_FAULK: u32 = 81_163;
const ZAETARS_SPIRIT: u32 = 55_352;
const DISABLED_OBJECT: u32 = 9496;
const SYMBOL_OF_LIFE: u32 = 8593;

/// Thistle Tea's spawn flags (`mob/spawn_flags_test.exs`,
/// `spawn_flags_vmangos_test.exs`), on the real spawns: Lord Valthalak's
/// (disabled, 0x2) isn't put out in Upper Blackrock, nor the disabled
/// object 9496 in Kalimdor; Henze Faulk (dead, 0x80) lies a settled corpse,
/// with no health, no NPC flags and no respawn of his own, comes back to
/// Symbol of Life, and comes back dead from a respawn; Zaetar's Spirit (not
/// visible, 0x40) is out in Maraudon but concealed, and stays so through a
/// respawn. No real spawn has a health percent of 0 (Thistle Tea's other
/// way to lie dead).
#[tokio::test]
#[ignore = "extended"]
async fn real_spawn_flags_hold_back_hide_and_lay_out_dead() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    let spawn = |map: u32, guid: u32| {
        d.creature_spawns[&map]
            .iter()
            .find(|s| s.guid == guid)
            .unwrap_or_else(|| panic!("spawn {guid}"))
            .clone()
    };
    assert_eq!(spawn(229, LORD_VALTHALAK).spawn_flags, 0x2);
    assert_eq!(spawn(0, HENZE_FAULK).spawn_flags, 0x80);
    assert_eq!(spawn(349, ZAETARS_SPIRIT).spawn_flags, 0x40);
    assert!(
        d.creature_spawns
            .values()
            .flatten()
            .all(|s| s.health_percent != 0.0)
    );
    let object = d.gameobject_spawns[&1]
        .iter()
        .find(|s| s.guid == DISABLED_OBJECT)
        .unwrap();
    assert_eq!(object.spawn_flags, 0x2);

    let realm = realm();
    let ubrs = realm.maps.get(&realm, copy(229));
    let valthalak = on(&ubrs, |m| {
        crate::game::map::scripts::creature_of_spawn(m, LORD_VALTHALAK)
    })
    .await;
    assert_eq!(valthalak, None, "a disabled spawn isn't out");
    let kalimdor = realm.maps.get(&realm, MapKey::world(1));
    let object = on(&kalimdor, |m| {
        crate::game::map::scripts::object_of_spawn(m, DISABLED_OBJECT)
    })
    .await;
    assert_eq!(object, None, "nor a disabled object");

    let mut c = Client::connect(&realm, "P6LFLAGS");
    let priest = c.enter("Flagga", Race::Human, Class::Priest).await;
    let eastern = realm.maps.get(&realm, MapKey::world(0));
    let henze = on(&eastern, move |m| {
        let g = crate::game::map::scripts::creature_of_spawn(m, HENZE_FAULK).expect("out");
        let c = m.creature(g).unwrap();
        let corpse = (
            c.alive(),
            c.fields.get_u32(UNIT_FIELD_HEALTH),
            c.fields.get_u32(UNIT_NPC_FLAGS),
        );
        let waits = match c.life {
            crate::game::map::creature::Life::Dead { respawn_at, .. } => {
                respawn_at.duration_since(m.now) > Duration::from_secs(365 * 86_400)
            }
            _ => false,
        };
        // Symbol of Life brings him back, cast within its range of him
        // (its script's creature is looked for there: mangos's
        // `CheckScriptTargeting`).
        let mut by = c.position;
        by.x += 3.0;
        crate::game::tests_p6_wspell::put(m, priest, by);
        crate::game::map::spells::cast_triggered(m, priest, SYMBOL_OF_LIFE, Some(g));
        let revived = m.creature(g).is_some_and(|c| c.alive());
        // A respawn lays him out again.
        crate::game::map::ai::respawn_forced(m, g, true);
        let again = crate::game::map::scripts::creature_of_spawn(m, HENZE_FAULK)
            .and_then(|g| m.creature(g))
            .map(|c| (c.alive(), c.fields.get_u32(UNIT_FIELD_HEALTH)));
        (corpse, waits, revived, again)
    })
    .await;
    assert_eq!(henze.0, (false, 0, 0), "a corpse with no flags");
    assert!(henze.1, "no respawn of his own");
    assert!(henze.2, "Symbol of Life revives him");
    assert_eq!(henze.3, Some((false, 0)), "a respawn lays him out dead");

    let maraudon = realm.maps.get(&realm, copy(349));
    let spirit = on(&maraudon, |m| {
        let g = crate::game::map::scripts::creature_of_spawn(m, ZAETARS_SPIRIT).expect("out");
        let before = concealed(m, g);
        crate::game::map::ai::respawn_forced(m, g, true);
        let g = crate::game::map::scripts::creature_of_spawn(m, ZAETARS_SPIRIT).expect("back");
        (before, concealed(m, g))
    })
    .await;
    assert_eq!(spirit, (true, true), "concealed, and through a respawn");
}

/// Thistle Tea's `CreatureLink` loader (`creature_link_test.exs`): a
/// template with no search range links a slave only when its master's
/// entry has one spawn on the map (two Loathebs, no link), and a spawn's
/// own `creature_linking` row stands over its entry's template.
#[test]
fn templated_links_need_one_master_and_yield_to_a_spawns_own() {
    use crate::game::data::CreatureSpawn;
    use crate::game::data::scripts::{LINK_AGGRO_ON_AGGRO, LINK_CANT_SPAWN_IF_BOSS_DEAD};
    const RAJAXX: u32 = 900_600;
    const CAPTAIN: u32 = 900_601;
    const LOATHEB: u32 = 900_602;
    const MAGGOT: u32 = 900_603;
    let mut d = GameData::empty();
    let at = |guid, entry, map, x: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map,
        x,
        ..Default::default()
    };
    d.creature_spawns.insert(
        509,
        vec![at(1, RAJAXX, 509, 0.0), at(3, CAPTAIN, 509, 10.0)],
    );
    d.creature_spawns.insert(
        533,
        vec![
            at(5, LOATHEB, 533, 0.0),
            at(7, LOATHEB, 533, 50.0),
            at(6, MAGGOT, 533, 1.0),
        ],
    );
    d.scripts
        .link_templates
        .insert((CAPTAIN, 509), (RAJAXX, LINK_AGGRO_ON_AGGRO, 150.0));
    d.scripts
        .link_templates
        .insert((MAGGOT, 533), (LOATHEB, LINK_CANT_SPAWN_IF_BOSS_DEAD, 0.0));
    // The captain's own row: another master.
    d.scripts.links.insert(3, (9, 2));
    crate::game::data::scripts_load::link_templates(&mut d);
    assert_eq!(d.scripts.links.get(&3), Some(&(9, 2)), "its own link");
    assert_eq!(d.scripts.links.get(&6), None, "two Loathebs: none");
    assert!(!d.scripts.links.values().any(|(m, _)| *m == 1));
}

const DEN_MOTHER: u32 = 37_523;

/// Thistle Tea's creature groups and events (`creature_group_test.exs`,
/// `creature_event_test.exs`): the Den Mother leads her Thistle Cubs as
/// one group that aggroes and evades together (flags 6), a group of
/// Kalimdor's spawns, and no formation; 4479 walks in a formation with
/// 4480 and 4481 (led by 4481) in the Eastern Kingdoms.
/// The events' looks are the latest patch's, one per spawn and event:
/// Pyrewood's 57461 becomes 1892 in event 49, 12088 takes equipment
/// 50001 in event 27, and 45829 casts 26231 as event 2 starts.
#[test]
#[ignore = "extended"]
fn real_groups_and_event_looks_load() {
    use crate::game::data::scripts::GROUP_FORMATION;
    let Some(d) = real() else {
        return;
    };
    let s = &d.scripts;
    let group = &s.groups[&DEN_MOTHER];
    assert!(group.len() > 2, "{group:?}");
    assert!(group.iter().all(|m| m.flags == 6));
    assert!(group.iter().any(|m| m.guid == 37_567));
    assert_eq!(s.group_of.get(&37_567), Some(&DEN_MOTHER));
    assert_eq!(s.group_of.get(&DEN_MOTHER), Some(&DEN_MOTHER));
    let map_of = |guid: u32| {
        d.creature_spawns
            .iter()
            .find(|(_, v)| v.iter().any(|x| x.guid == guid))
            .map(|(m, _)| *m)
    };
    assert_eq!(map_of(DEN_MOTHER), Some(1));
    assert!(group.iter().all(|m| m.flags & GROUP_FORMATION == 0));
    // 4479's formation, led by 4481.
    let leader = s.group_of[&4479];
    assert_eq!(leader, 4481);
    let mut members: Vec<u32> = s.groups[&leader].iter().map(|m| m.guid).collect();
    members.sort_unstable();
    assert_eq!(members, vec![4479, 4480, 4481]);
    assert!(
        s.groups[&leader]
            .iter()
            .all(|m| m.flags & GROUP_FORMATION != 0)
    );
    assert_eq!(map_of(4479), Some(0));

    let looks = &d.world.event_creatures;
    for list in looks.values() {
        let mut events: Vec<u32> = list.iter().map(|e| e.event).collect();
        events.sort_unstable();
        events.dedup();
        assert_eq!(events.len(), list.len(), "one per spawn and event");
    }
    let of = |guid: u32, event: u32| {
        looks[&guid]
            .iter()
            .find(|e| e.event == event)
            .copied()
            .unwrap_or_else(|| panic!("{guid} in {event}"))
    };
    assert_eq!(of(57_461, 49).entry, 1892);
    assert_eq!(of(12_088, 27).equipment, 50_001);
    assert_eq!(of(45_829, 2).spell_start, 26_231);
}

/// Thistle Tea's `GameTele.search` (`game_tele_test.exs`) and mangos's
/// `GetGameTele`: `.tele` takes an exact name over longer ones that start
/// with it, whatever its case and spaces; then those that start with it
/// (listed when several); then one with it anywhere in its name; and says
/// so when none has it.
#[tokio::test]
async fn tele_takes_an_exact_name_then_a_prefix_then_a_part() {
    use crate::game::data::Tele;
    let mut d = GameData::empty();
    let place = |name: &str, map, x| Tele {
        name: name.into(),
        map,
        x,
        y: -132.493,
        z: 83.5312,
        orientation: 0.0,
    };
    d.tele = vec![
        place("DireMaulEast", 1, -3980.8),
        place("DireMaulNorth", 1, -3521.3),
        place("brdpathbug", 0, -8900.0),
        place("brd", 0, -8940.0),
        place("DireforgeHill", 0, -8930.0),
    ];
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "P6LTELE");
    let g = a.enter("Telia", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let landed = async |a: &mut Client| {
        a.wait_for("the teleport", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position.x),
            _ => None,
        })
        .await
    };
    a.say(".tele  BRD ").await;
    assert!((landed(&mut a).await - -8940.0).abs() < 0.1, "brd itself");
    a.say(".tele diremaul").await;
    let listed = a.system_message("Places").await;
    assert!(listed.contains("DireMaulEast") && listed.contains("DireMaulNorth"));
    a.say(".tele forge").await;
    assert!(
        (landed(&mut a).await - -8930.0).abs() < 0.1,
        "a part of a name"
    );
    a.say(".tele stormwind").await;
    a.system_message("No place").await;
    let _ = on(&map, move |m| m.players.contains_key(&g)).await;
}

/// Thistle Tea's `ChatChannel.resolve` (`chat_channel_test.exs`): a
/// built-in channel is known whatever the case of its name.
#[test]
fn built_in_channels_are_known_in_any_case() {
    assert_eq!(
        crate::game::social::channels::built_in("worlddefense").map(|c| c.0),
        Some(23)
    );
    assert_eq!(
        crate::game::social::channels::built_in("TRADE - CITY").map(|c| c.0),
        Some(2)
    );
}

/// Thistle Tea's texts, emotes, intros and portals on the real data
/// (`broadcast_text_vmangos_test.exs`, `emote_test.exs`,
/// `cinematic_waypoint_vmangos_test.exs`, `battleground_vmangos_test.exs`,
/// `vmangos_boundary_test.exs`): Stratholme's callbacks (11812, 11816) are
/// zone yells; `/dance` (34) and `/kneel`'s state (59, 68) are states a
/// unit stays in and `/wave` (101) plays once (3); the human intro's camera
/// starts at Northshire at 0 and runs in time order; the six battleground
/// portals are two a battleground, Warsong's 3650 the Alliance's out to
/// Kalimdor and Alterac's 2413 the Horde's out to the Eastern Kingdoms;
/// npc_text 68's first group is "Greetings, $n." for both genders, its
/// others empty, and the spirit healer's (580) the same for both; Deputy
/// Willem's quest greeting (823) is a creature's, with its emote; Marshal
/// McBride (197) gives 7 and 3100 and takes 783, and the script-spawned
/// object 35875 (a spawn time of -60) isn't out by default.
#[test]
#[ignore = "extended"]
fn real_texts_emotes_intros_portals_and_relations_load() {
    let Some(d) = real() else {
        return;
    };
    let n = &d.npc;
    let intruders = &n.broadcast_texts[&11_812];
    assert!(intruders.male.starts_with("Intruders!"));
    assert_eq!(intruders.chat_type, 6);
    // Ysida's has only a woman's line: hers, whoever is asked.
    let worry = &n.broadcast_texts[&11_816];
    assert_eq!(worry.chat_type, 6);
    for female in [true, false] {
        assert!(
            n.broadcast_line(11_816, female)
                .is_some_and(|l| l.starts_with("Don't worry about me!"))
        );
    }

    assert_eq!(d.text_emote_anims.get(&34), Some(&10));
    assert_eq!(d.text_emote_anims.get(&59), Some(&68));
    assert_eq!(d.text_emote_anims.get(&101), Some(&3));
    assert!(d.emote_states.contains(&10) && d.emote_states.contains(&68));
    assert!(!d.emote_states.contains(&3));
    assert!(
        d.text_emote_anims.len() > 100,
        "{}",
        d.text_emote_anims.len()
    );

    let camera = &d.misc.cinematic_waypoints[&81];
    let (t, x, y, z) = camera[0];
    assert_eq!(t, 0);
    assert!((x - -8960.0).abs() < 1.0 && (y - 517.0).abs() < 1.0 && (z - 86.0).abs() < 1.0);
    assert!(camera.windows(2).all(|w| w[0].0 <= w[1].0));
    assert!(!d.misc.cinematic_waypoints.contains_key(&999_999));

    let e = &d.bg_entrances;
    assert_eq!((e[&3650].bg, e[&3650].team, e[&3650].exit_map), (2, 469, 1));
    assert_eq!((e[&2413].bg, e[&2413].team, e[&2413].exit_map), (1, 67, 0));
    let mut kinds: Vec<u32> = [2412, 2413, 3650, 3654, 3953, 3954]
        .iter()
        .map(|t| e[t].bg)
        .collect();
    kinds.sort_unstable();
    assert_eq!(kinds, vec![1, 1, 2, 2, 3, 3]);

    let greetings = &n.npc_texts[&68];
    let line = |b: u32, female| n.broadcast_line(b, female).map(str::to_owned);
    assert_eq!(
        line(greetings[0].broadcast, false).as_deref(),
        Some("Greetings, $n.")
    );
    assert_eq!(
        line(greetings[0].broadcast, true).as_deref(),
        Some("Greetings, $n.")
    );
    assert_eq!(n.broadcast_texts[&greetings[0].broadcast].language, 0);
    assert!(greetings[1..].iter().all(|c| c.broadcast == 0));
    let healer = n.npc_texts[&580][0].broadcast;
    assert!(line(healer, false).is_some_and(|l| !l.is_empty()));
    assert_eq!(line(healer, false), line(healer, true));

    // Deputy Willem's (823) quest greeting is a creature's, with its emote.
    let (text, emote, _) = &n.quest_greetings[&(823, 0)];
    assert!(text.starts_with("Hello there, $c."));
    assert_eq!(*emote, 1);
    assert!(!n.quest_greetings.contains_key(&(823, 1)));
    assert!(n.creature_starts[&197].contains(&7));
    assert!(n.creature_starts[&197].contains(&3100));
    assert!(n.creature_ends[&197].contains(&783));
    assert_eq!(n.quests[&7].title, "Kobold Camp Cleanup");

    let hidden = d.gameobject_spawns[&0]
        .iter()
        .find(|s| s.guid == 35_875)
        .unwrap();
    assert!(hidden.despawned);
}

const ONYXIA: u32 = 10_184;
const KING_MUKLA: u32 = 1559;
const SPIRIT_HEALER: u32 = 6491;

/// Thistle Tea's mob builder and model geometry (`mob/builder_test.exs`,
/// `model_geometry_test.exs`, `model_geometry_dbc_test.exs`,
/// `creature_template_vmangos_test.exs`, `elemental_creature_vmangos_test.exs`):
/// Onyxia, drawn at 2.0 where her model's own size is 1.8, reaches 26
/// yards (her addon's 23.4 by 2.0 / 1.8) with a radius of 2, at a scale of
/// 2; King Mukla, at his display's own size, keeps his addon's reach and
/// radius. Display 50's model is 1.913 high (CreatureModelData's height,
/// mangos's `GetCollisionHeight`); 4617 and 6921 may be mounted in, 7550
/// may not. Unending Breath breathes water, the Forsaken's Underwater
/// Breathing holds it four times as long. The spirit healer's query says
/// it is seen by ghosts (type flag 0x2) and no more. The Fire, Water and
/// Shadow elementals strike with their schools and are immune to them,
/// the Rock Elemental strikes with its fists and is immune to nature.
#[tokio::test]
#[ignore = "extended"]
async fn real_models_reach_breathe_and_show_as_they_should() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    assert!((d.display_info[&50].height - 1.913).abs() < 0.001);
    assert!(d.display_info[&4617].can_mount);
    assert!(d.display_info[&6921].can_mount);
    assert!(!d.display_info[&7550].can_mount);
    let aura = |spell: u32| {
        let s = d.spells.get(spell).unwrap();
        s.effects
            .iter()
            .find(|e| e.aura != 0)
            .map(|e| (e.aura, e.base_points + e.base_dice))
            .unwrap()
    };
    use crate::game::map::auras::{AURA_MOD_WATER_BREATHING, AURA_WATER_BREATHING};
    assert_eq!(aura(5697).0, AURA_WATER_BREATHING);
    assert_eq!(aura(5227), (AURA_MOD_WATER_BREATHING, 300));
    for (entry, school, immune) in [(575, 2, 4), (691, 4, 16), (703, 5, 32), (92, 0, 8)] {
        let t = d.creature_template(entry).unwrap();
        assert_eq!(
            (t.damage_school, t.school_immune_mask),
            (school, immune),
            "{entry}"
        );
    }

    let realm = realm();
    let mut c = Client::connect(&realm, "P6LMODEL");
    let g = c.enter("Modela", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let sizes = on(&map, move |m| {
        let at = m.players[&g].position();
        [ONYXIA, KING_MUKLA].map(|entry| {
            let c = crate::game::map::scripts::summon(m, Guid::zero(), entry, at, 0.0, None)
                .expect("the creature");
            let f = &m.creature(c).unwrap().fields;
            (
                f.get_f32(OBJECT_FIELD_SCALE_X),
                f.get_f32(UNIT_FIELD_COMBATREACH),
                f.get_f32(UNIT_FIELD_BOUNDINGRADIUS),
            )
        })
    })
    .await;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    let (scale, reach, radius) = sizes[0];
    assert!(
        near(scale, 2.0) && near(reach, 26.0) && near(radius, 2.0),
        "{sizes:?}"
    );
    let (_, reach, radius) = sizes[1];
    assert!(near(reach, 5.625) && near(radius, 4.725), "{sizes:?}");
    c.send(CMSG_CREATURE_QUERY {
        creature: SPIRIT_HEALER,
        guid: Guid::zero(),
    })
    .await;
    let flags = c
        .wait_for("SMSG_CREATURE_QUERY_RESPONSE", |m| match m {
            ServerOpcodeMessage::SMSG_CREATURE_QUERY_RESPONSE(r) => {
                r.found.as_ref().map(|f| f.type_flags)
            }
            _ => None,
        })
        .await;
    assert_eq!(flags, 0x2);
}

/// Thistle Tea's Stuck (`spell_stuck_dbc_test.exs`) and mangos-classic's
/// `HandleStartCommand`: `.start` casts the client's Stuck (7355), ten
/// seconds of it, at its caster; done, its effect takes them to where
/// they last stood on the ground (Thistle Tea's; see the matrix's note).
/// Refused in a fight.
#[tokio::test]
#[ignore = "extended"]
async fn real_start_casts_stuck_to_where_its_caster_last_stood() {
    use crate::game::map::gm_dev::STUCK;
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    let stuck = d.spells.get(STUCK).unwrap();
    assert_eq!(
        stuck.effects[0].effect,
        crate::game::map::spells::EFFECT_STUCK
    );
    assert_eq!(d.spells.cast_time(stuck), 10_000);
    let realm = realm();
    let mut c = Client::connect(&realm, "P6LSTUCK");
    let g = c.enter("Stucka", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let safe = on(&map, move |m| {
        let at = m.players[&g].position();
        let safe = Vector3d {
            x: at.x + 20.0,
            ..at
        };
        m.players.get_mut(&g).unwrap().safe_position = Some((0, safe, 1.0));
        safe
    })
    .await;
    c.say(".start").await;
    let mut began = None;
    for _ in 0..100 {
        began = on(&map, move |m| {
            m.caster(g)
                .and_then(|c| c.casting.as_ref())
                .map(|c| (c.spell, c.duration))
        })
        .await;
        if began.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(began, Some((STUCK, 10_000)));
    on(&map, |m| {
        m.now += Duration::from_millis(10_001);
        crate::game::map::spells::update(m);
    })
    .await;
    let landed = c
        .wait_for("the teleport", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
            _ => None,
        })
        .await;
    assert!((landed.x - safe.x).abs() < 0.1, "{landed:?} {safe:?}");
    // Not in a fight.
    on(&map, move |m| {
        crate::game::map::combat::enter(m, g, false);
    })
    .await;
    c.say(".start").await;
    c.system_message("in combat").await;
}

const THRESHWACKONATOR: u32 = 6669;
const GYROMASTS_REVENGE: u32 = 2078;

/// What `c` is told of `npc`'s gossip: its text, and its options' texts.
async fn gossip(c: &mut Client, npc: Guid) -> (u32, Vec<String>) {
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.wait_for("SMSG_GOSSIP_MESSAGE", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => Some((
            g.title_text_id,
            g.gossips.iter().map(|o| o.message.clone()).collect(),
        )),
        _ => None,
    })
    .await
}

/// Thistle Tea's gossip followers (`quest_follower_test.exs`,
/// `quest_follower_vmangos_test.exs`, `gossip_test.exs`) and
/// mangos-classic's `npc_threshwackonator`: the Threshwackonator, with no
/// menu of its own, shows its idle text (718) and nothing to choose; with
/// Gyromast's Revenge taken (which starts nothing: Gyromast is far off),
/// its key text (758) and "[PH] Insert key", which closes the gossip and
/// has it follow, once. The real followers' quests exist and their lines
/// are broadcast texts.
#[tokio::test]
#[ignore = "extended"]
async fn real_threshwackonator_follows_from_its_key_gossip() {
    use crate::game::map::escorts::{self, Act};
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    for f in crate::game::map::escorts::catalog::FOLLOWERS {
        // One that starts on accepting waits on its event (an
        // exploration); the Threshwackonator's on its death.
        let gossip = escorts::GOSSIP_STARTS.iter().any(|g| g.quest == f.quest);
        assert!(
            gossip || d.npc.quests[&f.quest].needs_event(),
            "{}",
            f.quest
        );
        for a in f.accept.iter().chain(f.arrive) {
            if let Act::Say(t) | Act::SayBy(_, t) = a {
                assert!(
                    d.npc
                        .broadcast_line(*t, false)
                        .is_some_and(|l| !l.is_empty()),
                    "{t}"
                );
            }
        }
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P6LTHRESH");
    let g = c.enter("Thresha", Race::NightElf, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    let robot = on(&map, move |m| {
        let p = m.players[&g].position();
        let o = m.players[&g].movement.orientation;
        let at = Vector3d {
            x: p.x + 2.0 * o.cos(),
            y: p.y + 2.0 * o.sin(),
            z: p.z,
        };
        crate::game::map::scripts::summon(m, Guid::zero(), THRESHWACKONATOR, at, 0.0, None)
            .expect("the robot")
    })
    .await;
    c.sees(robot).await;
    assert_eq!(gossip(&mut c, robot).await, (718, vec![]));
    c.say(&format!(".addquest {GYROMASTS_REVENGE}")).await;
    let mut taken = false;
    for _ in 0..50 {
        taken = on(&map, move |m| m.players[&g].journal.has(GYROMASTS_REVENGE)).await;
        if taken {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(taken);
    let following = move |m: &mut Map| m.escorts.iter().any(|a| a.creature == robot);
    assert!(!on(&map, following).await, "not on accepting");
    assert_eq!(
        gossip(&mut c, robot).await,
        (758, vec!["[PH] Insert key".to_owned()])
    );
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: robot,
        gossip_list_id: 1,
        unknown: None,
    })
    .await;
    c.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    assert!(on(&map, following).await, "it follows");
    // Chosen again, nothing more: one follow.
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: robot,
        gossip_list_id: 1,
        unknown: None,
    })
    .await;
    c.wait_for("SMSG_GOSSIP_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
    })
    .await;
    let follows = on(&map, move |m| {
        m.escorts.iter().filter(|a| a.creature == robot).count()
    })
    .await;
    assert_eq!(follows, 1);
    assert_eq!(escorts::GOSSIP_STARTS.len(), 1);
}
