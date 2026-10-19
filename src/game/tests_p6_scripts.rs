//! Round 6's creature script tests, ported from Thistle Tea's
//! `test/game/core/ai/creature_script/` (docs/parity/matrix.csv): the
//! imported scripts' events, timers and lines, driven on the real world
//! as a player's client sees them.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, event_ai, quests};

/// `quest` in `g`'s log, as though taken.
pub(super) fn logs(m: &mut Map, g: Guid, quest: u32) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.journal.log.push(crate::game::store::Logged {
        quest,
        ..Default::default()
    });
    quests::refresh_log(p, &data);
}

/// Where the first spawn of creature `entry` stands: its map and place.
pub(super) fn spawn_of(realm: &Realm, entry: u32) -> (u32, Vector3d) {
    let mut all: Vec<_> = realm
        .data
        .creature_spawns
        .values()
        .flatten()
        .filter(|s| s.entries.contains(&entry))
        .map(|s| {
            (
                s.guid,
                s.map,
                Vector3d {
                    x: s.x,
                    y: s.y,
                    z: s.z,
                },
            )
        })
        .collect();
    all.sort_by_key(|s| s.0);
    let s = all.first().unwrap_or_else(|| panic!("{entry} spawned"));
    (s.1, s.2)
}

/// A [`hero`] beside the spawn of `entry`, in sight of it: the client,
/// the player, the map and the creature.
pub(super) async fn hero_by(
    realm: &Arc<Realm>,
    name: &str,
    entry: u32,
) -> (Client, Guid, MapHandle, Guid) {
    let (mut c, me, _) = hero(realm, name).await;
    let (id, at) = spawn_of(realm, entry);
    let map = travel(realm, &mut c, id, off(at, 2.0, 0.0)).await;
    assert!(
        until(&map, 5_000, move |m| first(m, entry)
            .and_then(|g| m.creature(g))
            .is_some_and(|c| c.observers.contains(&me)))
        .await,
        "{entry} in sight"
    );
    let g = map.query(move |m| first(m, entry)).await.flatten().unwrap();
    (c, me, map, g)
}

/// `c` at `at` in map `id`'s world, across maps or within one: the map
/// they are in then.
pub(super) async fn travel(realm: &Arc<Realm>, c: &mut Client, id: u32, at: Vector3d) -> MapHandle {
    let from = realm.registry.online(c.guid).unwrap().map();
    if from.query(|m| m.key.map).await == Some(id) {
        relocate(&from, c, at).await;
        return from;
    }
    go(realm, c, id, at).await
}

/// The line of broadcast text `id`.
pub(super) fn line(realm: &Realm, id: u32) -> String {
    realm
        .data
        .npc
        .broadcast_line(id, false)
        .unwrap_or_else(|| panic!("text {id}"))
        .to_owned()
}

/// Wait for `c` to hear `speaker` say `id`'s line (up to its first
/// `$` placeholder), skipping others.
pub(super) async fn hears(c: &mut Client, realm: &Realm, speaker: Guid, id: u32) -> bool {
    let full = line(realm, id);
    let want = full.split('$').next().unwrap_or_default().to_owned();
    let exact = !full.contains('$');
    c.gets(Duration::from_secs(3), move |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => match &s.chat_type {
            SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. }
            | SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. }
                if *sender1 == speaker
                    && (if exact {
                        s.message == want
                    } else {
                        s.message.starts_with(&want)
                    }) =>
            {
                Some(())
            }
            _ => None,
        },
        _ => None,
    })
    .await
    .is_some()
}

/// Every script run `by` on, and what is due run.
pub(super) async fn hurry_all(map: &MapHandle, secs: f32) {
    let by = Duration::from_secs_f32(secs);
    map.query(move |m| hurry_runs(m, None, by)).await;
}

const TWIGGY: u32 = 6248;
const BIG_WILL: u32 = 6238;
const CHALLENGER: u32 = 6240;
const THE_AFFRAY: u32 = 1719;
const THE_RING: u32 = 522;

/// The Affray: a warrior on the quest stepping into the ring has Twiggy
/// start it (his fighting phase, his call), and six challengers gather,
/// friendly, each turning on the player in its turn, 25 seconds apart,
/// as Twiggy calls it; Big Will comes after the last, walks into the
/// ring and squares up 15 seconds on. While it runs, the ring starts no
/// other. Twiggy calls each challenger down, and the Affray is over when
/// Big Will falls (Thistle Tea's `twiggy_flathead`, vmangos's
/// `npc_twiggy_flathead` and `at_twiggy_flathead`).
#[tokio::test]
#[ignore = "extended"]
async fn the_affray_runs_its_challengers_and_big_will_one_at_a_time() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, twiggy) = hero_by(&realm, "Brawler", TWIGGY).await;
    let t = realm.data.area_triggers[&THE_RING].clone();
    let ring = Vector3d {
        x: t.x,
        y: t.y,
        z: t.z,
    };
    map.query(move |m| {
        put(m, me, ring);
        logs(m, me, THE_AFFRAY);
    })
    .await;
    c.clear();
    c.send(CMSG_AREATRIGGER {
        trigger_id: THE_RING,
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| event_ai::phase(m, twiggy) == 1).await,
        "the Affray begun"
    );
    assert!(
        hears(&mut c, &realm, twiggy, 2301).await,
        "the Affray has begun"
    );
    // The challengers gather, friendly, then come one at a time.
    hurry_all(&map, 2.0).await;
    let gathered = map
        .query(move |m| {
            living(m, CHALLENGER)
                .iter()
                .map(|g| faction(m, *g))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(gathered, [35; 6], "six friendly challengers");
    let hostile = |m: &Map| {
        living(m, CHALLENGER)
            .iter()
            .filter(|g| faction(m, **g) == 16)
            .count()
    };
    for turn in 1..=6 {
        hurry_all(&map, if turn == 1 { 5.0 } else { 25.0 }).await;
        assert_eq!(
            map.query(move |m| hostile(m)).await.unwrap(),
            turn,
            "one more each turn"
        );
        assert!(hears(&mut c, &realm, twiggy, 2318).await, "Twiggy calls it");
        if turn == 1 {
            // A second warrior in the ring while it runs starts nothing.
            c.send(CMSG_AREATRIGGER {
                trigger_id: THE_RING,
            })
            .await;
            sleep_ms(300).await;
            assert_eq!(
                map.query(move |m| living(m, CHALLENGER).len())
                    .await
                    .unwrap(),
                6,
                "one Affray at a time"
            );
        }
    }
    // Big Will, after the last.
    assert!(map.query(|m| first(m, BIG_WILL)).await.unwrap().is_none());
    hurry_all(&map, 25.0).await;
    let will = map
        .query(|m| first(m, BIG_WILL))
        .await
        .flatten()
        .expect("Big Will comes");
    assert_eq!(map.query(move |m| faction(m, will)).await.unwrap(), 35);
    map.query(move |m| {
        if let Some(c) = m.creature_mut(will) {
            c.speeds.walk = 20.0;
            c.speeds.run = 20.0;
        }
    })
    .await;
    sleep_ms(500).await;
    let at = settled(&map, will).await;
    // Toward the ring's middle, as far as the navmesh takes him (it ends
    // at the ring's edge, 14 yards short).
    let middle = Vector3d {
        x: -1682.31,
        y: -4329.68,
        z: at.z,
    };
    let came = Vector3d {
        x: -1713.79,
        y: -4342.09,
        z: at.z,
    };
    let dist = crate::game::map::combat::dist;
    assert!(
        dist(at, middle) < 16.0 && dist(came, middle) - dist(at, middle) > 15.0,
        "into the ring: {at:?}"
    );
    hurry_all(&map, 15.0).await;
    assert_eq!(
        map.query(move |m| faction(m, will)).await.unwrap(),
        7,
        "squares up"
    );
    assert!(
        hears(&mut c, &realm, will, 2421).await,
        "ready when you are"
    );
    // Twiggy calls each down, and the end.
    let one = map.query(|m| first(m, CHALLENGER)).await.flatten().unwrap();
    map.query(move |m| kill(m, me, one)).await;
    assert!(
        hears(&mut c, &realm, twiggy, 2355).await,
        "a challenger is down"
    );
    assert_eq!(
        map.query(move |m| event_ai::phase(m, twiggy)).await,
        Some(1)
    );
    map.query(move |m| kill(m, me, will)).await;
    assert!(
        hears(&mut c, &realm, twiggy, 2320).await,
        "the Affray is over"
    );
    assert_eq!(
        map.query(move |m| event_ai::phase(m, twiggy)).await,
        Some(0)
    );
}

const RIGGLE: u32 = 15077;
const MASTER_ANGLER: u32 = 8193;
const EXTRAVAGANZA: u32 = 15;
const ANNOUNCE_BEGIN: u32 = 30022;
const ANNOUNCE_OVER: u32 = 30023;
const HAS_WINNER: u32 = 30056;

fn week() -> [u32; 3] {
    use crate::game::map::scripts::server_variable as v;
    [v(HAS_WINNER), v(ANNOUNCE_OVER), v(ANNOUNCE_BEGIN)]
}

fn set_week(has_winner: u32, over: u32, begin: u32) {
    use crate::game::map::scripts::set_server_variable as set;
    set(HAS_WINNER, has_winner);
    set(ANNOUNCE_OVER, over);
    set(ANNOUNCE_BEGIN, begin);
}

fn tournament(m: &Map, on: bool) {
    let mut e = m.realm.social.events.lock();
    if on {
        e.insert(EXTRAVAGANZA);
    } else {
        e.remove(&EXTRAVAGANZA);
    }
}

/// A Riggle, summoned beside `me` with the tournament `on` and the week
/// as `before` had it: he, and the week once his arrival has settled it
/// (read before his first judging).
async fn riggle_arrives(
    map: &MapHandle,
    me: Guid,
    on: bool,
    before: (u32, u32, u32),
) -> (Guid, [u32; 3]) {
    map.query(move |m| {
        tournament(m, on);
        set_week(before.0, before.1, before.2);
        let at = m.players[&me].position();
        let g = summon(m, RIGGLE, off(at, 3.0, 0.0), 0.0);
        // His spawned events, at once.
        event_ai::update_despawns(m);
        hurry_runs(m, Some(g), Duration::from_millis(100));
        (g, week())
    })
    .await
    .unwrap()
}

/// Riggle Bassbait's week: arriving before the tournament he starts a
/// fresh one; respawned in it, he repeats nothing; arriving on a fresh
/// server in it, he still opens it. Each second he judges it: open with
/// no winner, he takes Master Angler turn-ins and yells the start once to
/// the zone; a winner closes the turn-ins without another yell; and when
/// the tastyfish go, he yells once more. The first Master Angler wins the
/// week and is named (Thistle Tea's `riggle_bassbait`, vmangos's
/// `npc_riggle_bassbait`).
#[tokio::test]
#[ignore = "extended"]
async fn riggle_bassbait_judges_the_fishing_extravaganza_once_a_week() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero(&realm, "Angler").await;
    // His arrival.
    for (on, before, after) in [
        (false, (1, 0, 0), [0, 0, 1]),
        (true, (0, 1, 0), [0, 1, 0]),
        (true, (1, 0, 0), [1, 0, 0]),
        (true, (0, 0, 0), [0, 0, 1]),
    ] {
        let (g, week) = riggle_arrives(&map, me, on, before).await;
        assert_eq!(week, after, "arriving, tournament {on}, after {before:?}");
        clear(&map, vec![g]);
    }
    // His judging, each second.
    let (riggle, _) = riggle_arrives(&map, me, true, (0, 0, 1)).await;
    c.sees(riggle).await;
    let judge = async |c: &mut Client, on: bool, flags: u32, before: (u32, u32, u32)| {
        c.clear();
        let flags_after = map
            .query(move |m| {
                tournament(m, on);
                set_week(before.0, before.1, before.2);
                if let Some(f) = m.unit_fields_mut(riggle) {
                    f.set_u32(UNIT_NPC_FLAGS, flags);
                }
                due(m, riggle, 1_507_702);
                hurry_runs(m, Some(riggle), Duration::from_millis(100));
                (npc_flags(m, riggle), week())
            })
            .await
            .unwrap();
        let said = heard(c, riggle, Duration::from_millis(600)).await;
        (flags_after.0, flags_after.1, said)
    };
    let (flags, after, said) = judge(&mut c, true, 0x1, (0, 0, 1)).await;
    assert_eq!(flags, 0x3, "open: he takes turn-ins");
    assert_eq!(after, [0, 1, 0]);
    assert_eq!(said, Some(line(&realm, 10608)), "the start, to the zone");
    let (flags, after, said) = judge(&mut c, true, 0x3, (0, 1, 0)).await;
    assert_eq!((flags, after, said), (0x3, [0, 1, 0], None), "once");
    let (flags, after, said) = judge(&mut c, true, 0x3, (1, 0, 0)).await;
    assert_eq!(
        (flags, after, said),
        (0x1, [1, 0, 0], None),
        "a winner closes it, quietly"
    );
    let (flags, after, said) = judge(&mut c, false, 0x3, (0, 1, 0)).await;
    assert_eq!((flags, after), (0x1, [0, 0, 0]), "the tastyfish gone");
    assert_eq!(said, Some(line(&realm, 10609)), "one last yell");
    let (flags, after, said) = judge(&mut c, false, 0x1, (0, 0, 0)).await;
    assert_eq!((flags, after, said), (0x1, [0, 0, 0], None));
    // The first Master Angler.
    let quest = realm.data.npc.quests[&MASTER_ANGLER].clone();
    map.query(move |m| {
        tournament(m, true);
        set_week(0, 1, 0);
        if let Some(f) = m.unit_fields_mut(riggle) {
            f.set_u32(UNIT_NPC_FLAGS, 0x3);
        }
        logs(m, me, MASTER_ANGLER);
        for (item, count) in quest.required_items.into_iter().filter(|(i, _)| *i != 0) {
            crate::game::map::inventory::add_item(m, me, item, count, false).unwrap();
        }
    })
    .await;
    c.clear();
    map.query(move |m| quests::choose_reward(m, me, riggle, MASTER_ANGLER, 0))
        .await;
    assert!(
        until(&map, 3_000, move |m| m.players[&me]
            .journal
            .rewarded
            .contains(&MASTER_ANGLER))
        .await,
        "turned in"
    );
    map.query(move |m| hurry_runs(m, Some(riggle), Duration::from_secs(1)))
        .await;
    let said = heard(&mut c, riggle, Duration::from_secs(2)).await;
    assert_eq!(
        said.as_deref(),
        Some("We have a winner!  Angler is the Master Angler!"),
        "the winner named"
    );
    assert_eq!(week()[0], 1, "the week won");
    assert_eq!(
        map.query(move |m| npc_flags(m, riggle)).await.unwrap(),
        0x1,
        "no more turn-ins"
    );
}

/// `g`'s cast of any of `spells` that `c` sees begin or go within
/// `within`: the spell and the unit it is aimed at.
pub(super) async fn cast_among(
    c: &mut Client,
    g: Guid,
    spells: &'static [u32],
    within: Duration,
) -> Option<(u32, Guid)> {
    c.gets(within, move |m| {
        let (caster, spell, targets) = match m {
            ServerOpcodeMessage::SMSG_SPELL_START(s) => (s.caster, s.spell, &s.targets),
            ServerOpcodeMessage::SMSG_SPELL_GO(s) => (s.caster, s.spell, &s.targets),
            _ => return None,
        };
        (caster == g && spells.contains(&spell)).then(|| {
            (
                spell,
                targets
                    .target_flags
                    .get_unit()
                    .map_or(Guid::zero(), |u| u.unit_target),
            )
        })
    })
    .await
}

/// `g` out of its last cast and its cooldowns, and its event `id` due.
pub(super) fn ready(m: &mut Map, g: Guid, id: u32) {
    crate::game::map::spells::interrupt(m, g);
    if let Some(cr) = m.caster_mut(g) {
        cr.gcd_until = None;
        cr.cooldowns.clear();
        cr.category_cooldowns.clear();
    }
    due(m, g, id);
}

/// The menu `c` is shown by `npc` at its hello: its text and options.
async fn menu(c: &mut Client, npc: Guid) -> (u32, Vec<(u32, String)>) {
    c.clear();
    hello(c, npc).await;
    menu_shown(c, npc).await
}

async fn menu_shown(c: &mut Client, npc: Guid) -> (u32, Vec<(u32, String)>) {
    c.wait_for("the menu", |m| match m {
        ServerOpcodeMessage::SMSG_GOSSIP_MESSAGE(g) if g.guid == npc => Some((
            g.title_text_id,
            g.gossips
                .iter()
                .map(|o| (o.id, o.message.clone()))
                .collect(),
        )),
        _ => None,
    })
    .await
}

const JAINA: u32 = 4968;
const JAINAS_AUTOGRAPH: u32 = 558;
const THE_MISSING_DIPLOMAT: u32 = 1267;

/// Lady Jaina Proudmoore welcomes a stranger, and speaks of Hendel's
/// capture to one who has finished The Missing Diplomat; to a player on
/// Jaina's Autograph she offers her autograph, and it is the player who
/// signs for it, on themselves (Thistle Tea's `lady_jaina_proudmoore`,
/// vmangos's `npc_lady_jaina_proudmoore`).
#[tokio::test]
#[ignore = "extended"]
async fn lady_jaina_greets_by_the_missing_diplomat_and_signs_for_a_ward() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, jaina) = hero_by(&realm, "Ward", JAINA).await;
    let autograph = |o: &[(u32, String)]| {
        o.iter()
            .find(|(_, t)| t.contains("autograph"))
            .map(|(i, _)| *i)
    };
    let (text, options) = menu(&mut c, jaina).await;
    assert_eq!(text, 3157, "a welcome");
    assert_eq!(autograph(&options), None, "no autograph for a stranger");
    map.query(move |m| {
        m.players
            .get_mut(&me)
            .unwrap()
            .journal
            .rewarded
            .insert(THE_MISSING_DIPLOMAT);
    })
    .await;
    let (text, _) = menu(&mut c, jaina).await;
    assert_eq!(text, 3158, "Hendel's capture");
    map.query(move |m| logs(m, me, JAINAS_AUTOGRAPH)).await;
    let (_, options) = menu(&mut c, jaina).await;
    let id = autograph(&options).expect("the autograph offered");
    c.clear();
    choose(&c, jaina, id).await;
    let signed = c
        .gets(crate::game::harness::WAIT, move |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == 23122 => {
                Some((s.caster, s.hits.clone()))
            }
            _ => None,
        })
        .await;
    assert_eq!(
        signed,
        Some((me, vec![me])),
        "the player signs, on themselves"
    );
}

/// Lady Jaina in a fight: every 3 to 10 seconds, Fireball or Fire Blast
/// at her victim, or a Blizzard at a random foe; every 10 to 30, her
/// water elementals, only while none are near, else a teleport of her
/// victim, which then leaves her threat list only if she has been drawn
/// over 40 yards from her tower; and her timers wait while she casts
/// (Thistle Tea's `lady_jaina_proudmoore`).
#[tokio::test]
#[ignore = "extended"]
async fn lady_jaina_fights_with_fire_frost_and_her_elementals() {
    let Some(realm) = realm() else {
        return;
    };
    const SPELLS: &[u32] = &[20678, 20679, 20680];
    const SPECIALS: &[u32] = &[20681, 20682];
    const WATER_ELEMENTAL: u32 = 10955;
    let (mut c, me, map, jaina) = hero_by(&realm, "Theramore", JAINA).await;
    let events = &realm.data.scripts.ai_events[&JAINA];
    for id in [496802, 496803] {
        let e = events.iter().find(|e| e.id == id).unwrap();
        assert!(
            e.flags & crate::game::data::scripts::EF_NOT_CASTING != 0,
            "{id} waits on her casts"
        );
    }
    let home = map
        .query(move |m| m.position_of(jaina))
        .await
        .flatten()
        .unwrap();
    let d = map
        .query(move |m| {
            tough(m, jaina);
            if let Some(f) = m.unit_fields_mut(jaina) {
                f.set_u32(UNIT_FIELD_FACTIONTEMPLATE, 14);
                f.set_u32(UNIT_FIELD_MAXPOWER1, 1_000_000);
                f.set_u32(UNIT_FIELD_POWER1, 1_000_000);
            }
            let d = dummy(m, off(home, 4.0, 0.0));
            crate::game::map::ai::attacked(m, jaina, d);
            d
        })
        .await
        .unwrap();
    assert!(
        between(timer(&map, jaina, 496802).await, 3.0, 3.0),
        "her first spell three seconds in"
    );
    assert!(
        between(timer(&map, jaina, 496803).await, 15.0, 15.0),
        "her first special fifteen in"
    );
    // Her spells.
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..40 {
        c.clear();
        let again = map
            .query(move |m| {
                ready(m, jaina, 496802);
                left(m, jaina, 496802)
            })
            .await
            .flatten();
        assert!(between(again, 3.0, 10.0), "again in 3 to 10: {again:?}");
        let (spell, at) = cast_among(&mut c, jaina, SPELLS, Duration::from_secs(2))
            .await
            .expect("a spell");
        if spell != 20680 {
            assert_eq!(at, d, "{spell} at her victim");
        }
        seen.insert(spell);
        if seen.len() == 3 {
            break;
        }
    }
    assert_eq!(seen.len(), 3, "all three: {seen:?}");
    // While she casts, her timers wait.
    let waited = map
        .query(move |m| {
            for _ in 0..40 {
                ready(m, jaina, 496802);
                if m.creature(jaina)
                    .is_some_and(|c| c.caster.casting.is_some())
                {
                    if let Some(cr) = m.caster_mut(jaina) {
                        cr.gcd_until = None;
                    }
                    due(m, jaina, 496802);
                    return Some(left(m, jaina, 496802));
                }
            }
            None
        })
        .await
        .unwrap();
    assert_eq!(
        waited,
        Some(Some(Duration::ZERO)),
        "her spell waits on her cast"
    );
    // Her elementals, only while none are near.
    let mut called = false;
    for _ in 0..40 {
        c.clear();
        map.query(move |m| ready(m, jaina, 496803)).await;
        let (spell, _) = cast_among(&mut c, jaina, SPECIALS, Duration::from_secs(2))
            .await
            .unwrap_or_default();
        if spell == 20681 {
            called = true;
            break;
        }
    }
    assert!(called, "her elementals called");
    map.query(move |m| {
        if m.creatures
            .iter()
            .all(|(_, c)| c.entry != WATER_ELEMENTAL || !c.alive())
        {
            summon(m, WATER_ELEMENTAL, off(home, -4.0, 0.0), 0.0);
        }
    })
    .await;
    // With them near, teleports: her victim kept, her tower in reach of
    // where she stands, else let go.
    let tower = Vector3d {
        x: -4018.1,
        y: -4525.24,
        z: 12.0,
    };
    for (at, kept) in [(off(tower, 5.0, 0.0), true), (home, false)] {
        relocate(&map, &mut c, off(at, 2.0, 2.0)).await;
        map.query(move |m| {
            crate::game::map::scripts::place(m, jaina, at, 0.0);
            if let Some(cr) = m.creature_mut(jaina) {
                cr.engagement.origin = Some(at);
            }
        })
        .await;
        assert!(
            until(&map, 5_000, move |m| m
                .creature(jaina)
                .is_some_and(|c| c.observers.contains(&me)))
            .await,
            "in sight"
        );
        let mut teleported = false;
        for _ in 0..20 {
            c.clear();
            map.query(move |m| {
                crate::game::map::scripts::place(m, jaina, at, 0.0);
                if crate::game::map::combat::dist(m.position_of(d).unwrap(), at) > 5.0 {
                    crate::game::map::scripts::place(m, d, off(at, 3.0, 0.0), 0.0);
                }
                if threat(m, jaina, d) <= 0.0 {
                    crate::game::map::ai::attacked(m, jaina, d);
                }
                if let Some(cr) = m.creature_mut(jaina) {
                    for (u, t) in cr.threat.iter_mut() {
                        if *u == d {
                            *t = 100.0;
                        }
                    }
                }
                ready(m, jaina, 496803);
            })
            .await;
            let cast = cast_among(&mut c, jaina, SPECIALS, Duration::from_secs(2)).await;
            assert_ne!(
                cast.map(|c| c.0),
                Some(20681),
                "no elementals with some near"
            );
            if cast.is_some_and(|c| c.0 == 20682) {
                teleported = true;
                break;
            }
        }
        assert!(teleported, "her victim teleported");
        sleep_ms(200).await;
        let threat = map.query(move |m| threat(m, jaina, d)).await.unwrap();
        assert_eq!(threat > 0.0, kept, "her victim kept {kept}: {threat}");
    }
}

/// The map's events looked at now: run out, failed or succeeded.
pub(super) fn check_events(m: &mut Map) {
    m.map_events.next_check = None;
    crate::game::map::map_events::update(m);
}

/// `quest` turned in at `ender` by `me`, its items in hand, as though
/// taken: whether it was.
pub(super) fn turn_in(m: &mut Map, me: Guid, ender: Guid, quest: u32) -> bool {
    let q = m.realm.data.npc.quests[&quest].clone();
    m.players
        .get_mut(&me)
        .unwrap()
        .journal
        .rewarded
        .remove(&quest);
    logs(m, me, quest);
    for (item, count) in q.required_items.into_iter().filter(|(i, _)| *i != 0) {
        crate::game::map::inventory::add_item(m, me, item, count, false).unwrap();
    }
    // Its event's or place's part done too.
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&me).unwrap();
    if let Some(l) = p.journal.log.iter_mut().find(|l| l.quest == quest) {
        l.explored = true;
    }
    quests::refresh_log(p, &data);
    quests::choose_reward(m, me, ender, quest, 0);
    m.players[&me].journal.rewarded.contains(&quest)
}

const UNBAGWA: u32 = 1449;
const STRANGLETHORN_FEVER: u32 = 349;
const SILVERBACK: u32 = 1511;
const KONDA: u32 = 1516;
const MOKK: u32 = 1514;

/// Stranglethorn Fever: the fangs turned in, Witch Doctor Unbagwa stops
/// offering quests and stands passive while three waves of apes come at
/// him, ten seconds after each starts: three silverbacks, then four with
/// Konda, then five with Mokk. Each wave's dead count toward it, and the
/// next comes once all of it is down; the last down, he goes back to
/// work, as he does if a wave runs out of time (Thistle Tea's
/// `witch_doctor_unbagwa`, vmangos's `npc_witch_doctor_unbagwa`).
#[tokio::test]
#[ignore = "extended"]
async fn unbagwas_fever_brings_three_waves_of_apes() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map, unbagwa) = hero_by(&realm, "Feverish", UNBAGWA).await;
    let usual = map.query(move |m| faction(m, unbagwa)).await.unwrap();
    let taken = map
        .query(move |m| {
            tough(m, unbagwa);
            turn_in(m, me, unbagwa, STRANGLETHORN_FEVER)
        })
        .await
        .unwrap();
    assert!(taken, "turned in");
    let passive = map
        .query(move |m| (npc_flags(m, unbagwa) & 0x2, faction(m, unbagwa)))
        .await
        .unwrap();
    assert_eq!(passive, (0, 113), "no quests, passive");
    for (event, silverbacks, leader) in [
        (34901, 3, None),
        (34902, 4, Some(KONDA)),
        (34903, 5, Some(MOKK)),
    ] {
        assert!(
            map.query(move |m| crate::game::map::map_events::active(m, event))
                .await
                .unwrap(),
            "{event} under way"
        );
        let before = map.query(|m| living(m, SILVERBACK).len()).await.unwrap();
        hurry_all(&map, 9.5).await;
        assert_eq!(
            map.query(|m| living(m, SILVERBACK).len()).await.unwrap(),
            before,
            "{event}: ten seconds first"
        );
        hurry_all(&map, 0.6).await;
        let apes = map
            .query(move |m| {
                let mut apes = living(m, SILVERBACK);
                apes.extend(leader.into_iter().flat_map(|l| living(m, l)));
                apes
            })
            .await
            .unwrap();
        assert_eq!(
            apes.len(),
            silverbacks + leader.iter().count(),
            "{event}'s apes"
        );
        let n = apes.len() as u32;
        let counted = map
            .query(move |m| {
                tough(m, unbagwa);
                let mut counted = Vec::new();
                for g in &apes {
                    kill(m, me, *g);
                    counted.push(crate::game::map::map_events::data(m, event, 0));
                }
                check_events(m);
                counted
            })
            .await
            .unwrap();
        let want: Vec<Option<u32>> = (1..=n).map(Some).collect();
        assert_eq!(counted, want, "{event}: each fallen ape counts");
    }
    assert!(
        until(&map, 2_000, move |m| npc_flags(m, unbagwa) & 0x2 != 0
            && faction(m, unbagwa) == usual)
        .await,
        "back to work"
    );
    // A wave that runs out of time.
    let taken = map
        .query(move |m| turn_in(m, me, unbagwa, STRANGLETHORN_FEVER))
        .await
        .unwrap();
    assert!(taken, "turned in again");
    let restored = map
        .query(move |m| {
            if let Some(e) = m.map_events.active.get_mut(&34901) {
                e.expires = m.now;
            }
            check_events(m);
            hurry_runs(m, None, Duration::from_millis(100));
            (npc_flags(m, unbagwa) & 0x2, faction(m, unbagwa))
        })
        .await
        .unwrap();
    assert_eq!(restored, (0x2, usual), "a failure puts him back to work");
}

const CAULDRON: u32 = 11152;

/// The live creature of spawn `spawn` here.
pub(super) fn of_spawn(m: &Map, spawn: u32) -> Option<Guid> {
    m.creatures
        .iter()
        .find(|(_, c)| c.spawn_guid == Some(spawn) && c.alive())
        .map(|(_, c)| c.guid)
}

/// Where spawn `spawn` stands.
pub(super) fn spawn_at(realm: &Realm, spawn: u32) -> (u32, Vector3d) {
    let s = realm
        .data
        .creature_spawns
        .values()
        .flatten()
        .find(|s| s.guid == spawn)
        .unwrap_or_else(|| panic!("spawn {spawn}"));
    (
        s.map,
        Vector3d {
            x: s.x,
            y: s.y,
            z: s.z,
        },
    )
}

/// Western Plaguelands: a Scourge Cauldron calls up its own field's lord
/// for a player on that field's quest who comes in sight, and goes quiet
/// for ten minutes; for one on another field's quest, or none, it does
/// nothing (Thistle Tea's `western_plaguelands`).
#[tokio::test]
#[ignore = "extended"]
async fn a_scourge_cauldron_calls_its_lord_for_one_on_its_fields_quest() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, _) = hero(&realm, "Cauldron").await;
    let (id, felstone_at) = spawn_at(&realm, 45420);
    let map = travel(&realm, &mut c, id, off(felstone_at, 30.0, 0.0)).await;
    for (spawn, quest, lord) in [
        (45420, None, None),
        (45421, Some(5216), None),
        (45420, Some(5216), Some(11075)),
        (45421, Some(5231), Some(11077)),
    ] {
        let (_, at) = spawn_at(&realm, spawn);
        map.query(move |m| {
            let p = m.players.get_mut(&me).unwrap();
            // Seen as a player is, not a GM.
            p.gm = false;
            p.journal.log.clear();
        })
        .await;
        relocate(&map, &mut c, off(at, 15.0, 0.0)).await;
        let cauldron = map
            .query(move |m| {
                let g = of_spawn(m, spawn);
                if let Some(q) = quest {
                    logs(m, me, q);
                }
                g
            })
            .await
            .flatten()
            .unwrap_or_else(|| panic!("cauldron {spawn}"));
        assert_eq!(
            map.query(move |m| m.creature(cauldron).map(|c| c.entry))
                .await
                .flatten(),
            Some(CAULDRON)
        );
        let called = until(&map, 2_500, move |m| {
            [11075, 11076, 11077, 11078]
                .iter()
                .any(|e| !living(m, *e).is_empty())
        })
        .await;
        match lord {
            None => assert!(!called, "{spawn}, {quest:?}: quiet"),
            Some(lord) => {
                assert!(called, "{spawn}, {quest:?}: a lord");
                let (lords, gone) = map
                    .query(move |m| (living(m, lord).len(), of_spawn(m, spawn).is_none()))
                    .await
                    .unwrap();
                assert_eq!(
                    (lords, gone),
                    (1, true),
                    "{spawn}: its own lord, and it goes quiet"
                );
                let all: Vec<Guid> = map
                    .query(|m| {
                        [11075, 11076, 11077, 11078]
                            .iter()
                            .flat_map(|e| living(m, *e))
                            .collect()
                    })
                    .await
                    .unwrap();
                clear(&map, all);
            }
        }
    }
}

const DISCIPLE: u32 = 3678;
const NARALEX: u32 = 3679;
const ECTOPLASM: u32 = 3640;

/// Wailing Caverns: once Mutanus is dead, the disciple, waiting on him,
/// wakes Naralex, fighting or not, and only then (his last phase, his
/// event ended well); the disciple's death ends the ritual's event a
/// failure (Thistle Tea's `wailing_caverns`, vmangos's
/// `npc_disciple_of_naralex`).
#[tokio::test]
#[ignore = "extended"]
async fn naralex_wakes_once_mutanus_is_dead_and_the_ritual_fails_with_the_disciple() {
    use super::tests_p4_scripts_dungeons::{inside, set};
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = inside(&realm, "Dreamer", 43, DISCIPLE).await;
    let d = map.query(|m| first(m, DISCIPLE)).await.flatten().unwrap();
    c.sees(d).await;
    let (naralex, by) = map
        .query(|m| {
            let n = first(m, NARALEX)?;
            Some((n, m.position_of(n)?))
        })
        .await
        .flatten()
        .expect("Naralex");
    // At the ritual's place, by Naralex.
    relocate(&map, &mut c, off(by, 4.0, 4.0)).await;
    map.query(move |m| crate::game::map::scripts::place(m, d, off(by, 5.0, 0.0), 0.0))
        .await;
    c.sees(naralex).await;
    // Not in his waiting phase, Mutanus dead: nothing.
    let early = map
        .query(move |m| {
            set(m, 5, 3);
            event_ai::set_phase(m, d, 1);
            due(m, d, 367807);
            event_ai::phase(m, d)
        })
        .await
        .unwrap();
    assert_eq!(early, 1, "only while waiting on Mutanus");
    for fighting in [false, true] {
        c.clear();
        let phase = map
            .query(move |m| {
                crate::game::map::map_events::start(m, DISCIPLE, d, me, Duration::from_secs(600));
                event_ai::set_phase(m, d, 6);
                set(m, 4, 1);
                if fighting {
                    tough(m, d);
                    let dd = dummy(m, off(m.position_of(d).unwrap(), 3.0, 0.0));
                    crate::game::map::ai::attacked(m, d, dd);
                    due(m, d, 367808);
                } else {
                    due(m, d, 367807);
                }
                hurry_runs(m, Some(d), Duration::from_millis(100));
                (
                    event_ai::phase(m, d),
                    super::tests_p4_scripts_dungeons::data(m, 4),
                    crate::game::map::map_events::active(m, DISCIPLE),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            phase,
            (7, 3, false),
            "fighting {fighting}: the finale, done, the event over"
        );
        assert!(
            hears(&mut c, &realm, naralex, 1271).await,
            "fighting {fighting}: Naralex awakens"
        );
    }
    // The disciple's death fails the ritual.
    let ended = map
        .query(move |m| {
            crate::game::map::map_events::start(m, DISCIPLE, d, me, Duration::from_secs(600));
            kill(m, me, d);
            crate::game::map::map_events::active(m, DISCIPLE)
        })
        .await
        .unwrap();
    assert!(!ended, "the ritual's event ended with him");
}

/// An evolving ectoplasm takes on the colour of the school that strikes
/// it, frost, fire, nature or shadow, its form and its immunity; while it
/// wears one, another school changes nothing; ten seconds on, it is
/// itself again (Thistle Tea's `wailing_caverns`, vmangos's
/// `npc_evolving_ectoplasm`).
#[tokio::test]
#[ignore = "extended"]
async fn an_evolving_ectoplasm_takes_the_colour_of_the_school_that_strikes_it() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map) = hero(&realm, "Colourist").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    for (school, form, immunity, other, other_form) in [
        (16, 7944, 7940, 4, 7943),
        (4, 7943, 7942, 8, 7945),
        (8, 7945, 7941, 32, 7946),
        (32, 7946, 7743, 16, 7944),
    ] {
        let g = summon_near(&map, ECTOPLASM, off(at, 3.0, 0.0)).await;
        let worn = map
            .query(move |m| {
                event_ai::update_despawns(m);
                tough(m, g);
                event_ai::spell_hit(m, g, me, 133, school);
                let worn = (has(m, g, form), has(m, g, immunity), event_ai::phase(m, g));
                event_ai::spell_hit(m, g, me, 133, other);
                (worn, has(m, g, other_form))
            })
            .await
            .unwrap();
        assert_eq!(
            worn,
            ((true, true, 1), false),
            "school {school}: its form and immunity, and no other while it wears them"
        );
        let after = map
            .query(move |m| {
                hurry_runs(m, Some(g), Duration::from_secs(10));
                (has(m, g, form), event_ai::phase(m, g))
            })
            .await
            .unwrap();
        assert_eq!(after, (false, 0), "school {school}: itself again");
        clear(&map, vec![g]);
    }
}

const WARD_OF_ZUMRAH: u32 = 7785;

/// A Ward of Zum'rah raises a skeleton every five seconds, fighting or
/// not (Thistle Tea's `zul_farrak`).
#[tokio::test]
#[ignore = "extended"]
async fn a_ward_of_zumrah_raises_a_skeleton_every_five_seconds() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero(&realm, "Warded").await;
    let at = map.query(move |m| m.players[&me].position()).await.unwrap();
    let ward = summon_near(&map, WARD_OF_ZUMRAH, off(at, 3.0, 0.0)).await;
    c.sees(ward).await;
    assert!(
        until(&map, 2_000, move |m| left(m, ward, 778501).is_some()).await,
        "its timer"
    );
    timed_cast(
        &mut c,
        &map,
        ward,
        778501,
        11088,
        (0.0, 5.0),
        (5.0, 5.0),
        None,
    )
    .await;
    let d = map
        .query(move |m| {
            tough(m, ward);
            let d = dummy(m, off(at, 5.0, 0.0));
            crate::game::map::ai::attacked(m, ward, d);
            d
        })
        .await
        .unwrap();
    assert!(
        until(&map, 2_000, move |m| left(m, ward, 778502).is_some()).await,
        "its fight's timer"
    );
    timed_cast(
        &mut c,
        &map,
        ward,
        778502,
        11088,
        (0.0, 5.0),
        (5.0, 5.0),
        None,
    )
    .await;
    clear(&map, vec![ward, d]);
}

/// The kills `g` has toward `quest`.
pub(super) fn kills_of(m: &Map, g: Guid, quest: u32) -> u32 {
    m.players[&g]
        .journal
        .log
        .iter()
        .find(|l| l.quest == quest)
        .map_or(0, |l| l.kills.iter().sum())
}

/// Object `entry` summoned where `by` stands, for `secs`.
pub(super) fn object_by(m: &mut Map, by: Guid, entry: u32, secs: u32) {
    let step = crate::game::data::scripts::ScriptStep {
        command: 76,
        datalong: [entry, secs, 0, 0],
        ..Default::default()
    };
    crate::game::map::scripts::execute(m, &step, by, by);
}

/// An Andorhal watchtower credits a player on its quest who comes in
/// sight, but only while a beacon torch burns by them (Thistle Tea's
/// `western_plaguelands`).
#[tokio::test]
#[ignore = "extended"]
async fn an_andorhal_watchtower_credits_a_scout_by_a_beacon_torch() {
    const TOWER: u32 = 10902;
    const TORCH: u32 = 176093;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map, tower) = hero_by(&realm, "Scout", TOWER).await;
    map.query(move |m| {
        let p = m.players.get_mut(&me).unwrap();
        p.gm = false;
        logs(m, me, 5097);
    })
    .await;
    sleep_ms(1_500).await;
    assert_eq!(
        map.query(move |m| kills_of(m, me, 5097)).await.unwrap(),
        0,
        "no torch, no credit"
    );
    map.query(move |m| object_by(m, me, TORCH, 60)).await;
    assert!(
        until(&map, 3_000, move |m| kills_of(m, me, 5097) > 0).await,
        "credited by the torch's light"
    );
    assert!(
        map.query(move |m| m.creature(tower).is_some())
            .await
            .unwrap()
    );
}

const TRIAGE: u32 = 6624;
const ALLIANCE_DOCTOR: u32 = 12939;
const PATIENTS: [u32; 3] = [12938, 12936, 12937];

/// The patients here, alive.
fn patients(m: &Map) -> Vec<Guid> {
    PATIENTS.iter().flat_map(|e| living(m, *e)).collect()
}

/// Creature `g`'s out-of-combat timers held off an hour (a patient's
/// bleeding).
fn stanch(m: &mut Map, g: Guid) {
    let later = m.now + Duration::from_secs(3600);
    let Some(entry) = m.creature(g).map(|c| c.entry) else {
        return;
    };
    let bleeds: Vec<usize> = m.realm.data.scripts.ai_events[&entry]
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == crate::game::data::scripts::EVENT_TIMER_OOC)
        .map(|(i, _)| i)
        .collect();
    if let Some(s) = event_ai::scripted_mut(m, g) {
        for i in bleeds {
            s.timers[i] = Some(later);
        }
    }
}

/// Triage: the doctor lays a patient in a free bunk every ten seconds,
/// and none while every bunk is taken; a patient arrives lying down at
/// its injury's health (75, 50 or 25 percent), doesn't regenerate, and is
/// one of the quest's event's; it bleeds 50 a second, and its death
/// counts toward the six that fail it (Thistle Tea's `triage`).
#[tokio::test]
#[ignore = "extended"]
async fn triages_doctor_fills_his_bunks_and_his_patients_bleed() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map, doctor) = hero_by(&realm, "Medic", ALLIANCE_DOCTOR).await;
    map.query(move |m| {
        let p = m.players.get_mut(&me).unwrap();
        crate::game::map::skills::learn_skill(p, 129, 4);
        crate::game::map::skills::set_value(p, 129, 225);
        quests::accept(m, me, doctor, TRIAGE);
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| m.players[&me].journal.has(TRIAGE)
            && event_ai::phase(m, doctor) == 1)
        .await,
        "taken, the doctor at work"
    );
    assert!(
        map.query(|m| crate::game::map::map_events::active(m, TRIAGE))
            .await
            .unwrap()
    );
    // A patient each time, until his seven bunks are full; then none.
    let mut counts = Vec::new();
    for _ in 0..10 {
        let n = map
            .query(move |m| {
                for p in patients(m) {
                    stanch(m, p);
                }
                due(m, doctor, 1293901);
                event_ai::update_despawns(m);
                for p in patients(m) {
                    stanch(m, p);
                }
                patients(m).len()
            })
            .await
            .unwrap();
        counts.push(n);
    }
    assert_eq!(
        counts,
        [1, 2, 3, 4, 5, 6, 7, 7, 7, 7],
        "a bunk each, while one is free"
    );
    // As each lies.
    let lying = map
        .query(move |m| {
            let targets = crate::game::map::map_events::targets(m, TRIAGE);
            patients(m)
                .into_iter()
                .map(|p| {
                    let entry = m.creature(p).unwrap().entry;
                    let (hp, max) = health(m, p);
                    (
                        entry,
                        hp * 100 / max,
                        stand_state(m, p),
                        crate::game::map::creature_combat::regenerates_health(m, p),
                        targets.contains(&p),
                    )
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    for (entry, pct, stand, regenerates, target) in lying {
        let want = match entry {
            12938 => 75,
            12936 => 50,
            _ => 25,
        };
        assert!(pct.abs_diff(want) <= 1, "{entry} at {pct}, not {want}");
        assert_eq!((stand, regenerates, target), (7, false, true), "{entry}");
    }
    // One bleeds out, and counts.
    let p = map.query(|m| patients(m)[0]).await.unwrap();
    let (bled, died) = map
        .query(move |m| {
            if let Some(f) = m.unit_fields_mut(p) {
                f.set_u32(UNIT_FIELD_HEALTH, 120);
            }
            let id = m.creature(p).unwrap().entry * 100 + 2;
            due(m, p, id);
            let bled = health(m, p).0;
            due(m, p, id);
            due(m, p, id);
            hurry_runs(m, Some(p), Duration::from_millis(100));
            (bled, crate::game::map::map_events::data(m, TRIAGE, 1))
        })
        .await
        .unwrap();
    assert_eq!(bled, 70, "50 a second");
    assert_eq!(died, Some(1), "its death counted");
    assert!(
        until(&map, 2_000, move |m| m
            .creature(p)
            .is_none_or(|c| !c.alive()))
        .await
    );
}

const GRENKA: u32 = 4490;
const HARPY: u32 = 4100;

/// Test of Endurance: the foodstuffs (event 747) call Grenka, hidden,
/// unless one is near already; her flock comes in three waves, one harpy
/// five seconds on, two at twenty, one at thirty-five, when she shows
/// herself, may be fought and joins the fight (Thistle Tea's
/// `test_of_endurance`).
#[tokio::test]
#[ignore = "extended"]
async fn the_foodstuffs_call_grenka_and_her_flock() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, _) = hero(&realm, "Enduring").await;
    // By the ledge in Thousand Needles, where the foodstuffs are set out.
    let ledge = Vector3d {
        x: -5589.63,
        y: -1575.89,
        z: 11.75,
    };
    let map = travel(&realm, &mut c, 1, off(ledge, 10.0, 0.0)).await;
    let call = move |m: &mut Map| {
        crate::game::map::scripts::start(
            m,
            crate::game::data::scripts::ScriptTable::Event,
            747,
            me,
            me,
        );
        hurry_runs(m, None, Duration::from_millis(100));
        event_ai::update_despawns(m);
        living(m, GRENKA)
    };
    let grenka = map.query(call).await.unwrap();
    assert_eq!(grenka.len(), 1, "Grenka called");
    let g = grenka[0];
    let hidden = map
        .query(move |m| {
            (
                event_ai::scripted(m, g).is_some_and(|s| s.concealed),
                unit_flags(m, g) & 0x300,
            )
        })
        .await
        .unwrap();
    assert_eq!(hidden, (true, 0x300), "hidden, not to be fought");
    assert_eq!(
        map.query(call).await.unwrap().len(),
        1,
        "not again while she is near"
    );
    let mut harpies = Vec::new();
    for secs in [5.0, 15.0, 15.0] {
        hurry_all(&map, secs).await;
        harpies.push(
            map.query(move |m| {
                living(m, HARPY)
                    .into_iter()
                    .filter(|h| event_ai::scripted(m, *h).is_some_and(|s| s.summoner == Some(g)))
                    .count()
            })
            .await
            .unwrap(),
        );
    }
    assert_eq!(harpies, [1, 3, 4], "one, two and one");
    let shown = map
        .query(move |m| {
            (
                event_ai::scripted(m, g).is_some_and(|s| s.concealed),
                unit_flags(m, g) & 0x300,
                fighting(m, g),
            )
        })
        .await
        .unwrap();
    assert_eq!(shown, (false, 0, true), "she shows herself and fights");
}

const TERENTHIS: u32 = 3693;
const SELARIN: u32 = 3694;

/// Either of Terenthis's escapes brings Sentinel Selarin, but not a
/// second while she is still about (Thistle Tea's `terenthis`).
#[tokio::test]
#[ignore = "extended"]
async fn terenthis_calls_selarin_once_while_she_is_about() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map, terenthis) = hero_by(&realm, "Escapee", TERENTHIS).await;
    let before = map.query(|m| living(m, SELARIN).len()).await.unwrap();
    let mut seen = Vec::new();
    for quest in [994, 995, 994] {
        let n = map
            .query(move |m| {
                assert!(turn_in(m, me, terenthis, quest), "{quest} turned in");
                hurry_runs(m, None, Duration::from_millis(100));
                event_ai::update_despawns(m);
                living(m, SELARIN).len()
            })
            .await
            .unwrap();
        seen.push(n - before);
    }
    assert_eq!(seen, [1, 1, 1], "called once, not again while she is about");
}

const SLIM: u32 = 4962;
const SLIMS_FRIEND: u32 = 4971;
const THE_MISSING_DIPLOMAT_SLIM: u32 = 1249;

/// Tapoke "Slim" Jahn, fought: with no friend of his near he calls one,
/// once, boasting only while he is on the run (the quest's event); at a
/// fifth of his health he gives up, but only on the run: he and his
/// friend stand down, friendly; he begs and tells what he knows; and
/// twelve seconds on the player has the quest's credit, its event ends
/// well, and he goes (Thistle Tea's `tapoke_slim_jahn`).
#[tokio::test]
#[ignore = "extended"]
async fn slim_calls_a_friend_once_and_gives_up_only_on_the_run() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, slim) = hero_by(&realm, "Diplomat", SLIM).await;
    let friends = |m: &Map| living(m, SLIMS_FRIEND);
    // Not on the run.
    c.clear();
    map.query(move |m| {
        for f in friends(m) {
            crate::game::map::scripts::despawn(m, f);
        }
        tough(m, slim);
        crate::game::map::scripts::set_faction(m, slim, 14);
        crate::game::map::ai::attacked(m, slim, me);
    })
    .await;
    assert!(
        cast_any_of(&mut c, slim, 16457, Duration::from_secs(2))
            .await
            .is_some(),
        "a friend called"
    );
    assert_eq!(
        heard(&mut c, slim, Duration::from_millis(500)).await,
        None,
        "no boast, not on the run"
    );
    let kept = map
        .query(move |m| {
            set_pct(m, slim, 15);
            due(m, slim, 496201);
            faction(m, slim)
        })
        .await
        .unwrap();
    assert_eq!(kept, 14, "no giving up, not on the run");
    // On the run.
    c.clear();
    map.query(move |m| {
        for f in friends(m) {
            crate::game::map::scripts::despawn(m, f);
        }
        logs(m, me, THE_MISSING_DIPLOMAT_SLIM);
        crate::game::map::map_events::start(
            m,
            THE_MISSING_DIPLOMAT_SLIM,
            slim,
            me,
            Duration::from_secs(600),
        );
        if let Some(cr) = m.caster_mut(slim) {
            cr.gcd_until = None;
            cr.cooldowns.clear();
        }
        event_ai::aggro(m, slim, me);
    })
    .await;
    assert!(
        cast_any_of(&mut c, slim, 16457, Duration::from_secs(2))
            .await
            .is_some(),
        "a friend called"
    );
    assert!(
        hears(&mut c, &realm, slim, 5827).await,
        "a boast, on the run"
    );
    // A friend near: none called.
    let friend = map
        .query(move |m| {
            let at = m.position_of(slim).unwrap();
            let f = friends(m)
                .into_iter()
                .next()
                .unwrap_or_else(|| summon(m, SLIMS_FRIEND, off(at, 2.0, 0.0), 0.0));
            crate::game::map::scripts::place(m, f, off(at, 2.0, 0.0), 0.0);
            tough(m, f);
            f
        })
        .await
        .unwrap();
    c.clear();
    map.query(move |m| {
        if let Some(cr) = m.caster_mut(slim) {
            cr.gcd_until = None;
            cr.cooldowns.clear();
        }
        event_ai::aggro(m, slim, me);
    })
    .await;
    assert!(
        cast_any_of(&mut c, slim, 16457, Duration::from_millis(800))
            .await
            .is_none(),
        "once"
    );
    // He gives up.
    c.clear();
    let gave_up = map
        .query(move |m| {
            set_pct(m, slim, 15);
            due(m, slim, 496201);
            hurry_runs(m, None, Duration::from_millis(100));
            (faction(m, slim), fighting(m, slim), faction(m, friend))
        })
        .await
        .unwrap();
    assert_eq!(gave_up, (35, false, 35), "he and his friend stand down");
    assert!(
        hears(&mut c, &realm, friend, 5828).await,
        "more than he bargained for"
    );
    hurry_all(&map, 2.0).await;
    assert!(hears(&mut c, &realm, slim, 1743).await, "he begs");
    hurry_all(&map, 4.0).await;
    assert!(hears(&mut c, &realm, slim, 1744).await, "what he knows");
    let done = map
        .query(move |m| {
            hurry_runs(m, None, Duration::from_secs(6));
            let explored = m.players[&me]
                .journal
                .log
                .iter()
                .find(|l| l.quest == THE_MISSING_DIPLOMAT_SLIM)
                .is_some_and(|l| l.explored);
            (
                explored,
                crate::game::map::map_events::active(m, THE_MISSING_DIPLOMAT_SLIM),
            )
        })
        .await
        .unwrap();
    assert_eq!(done, (true, false), "the credit, the event over");
    assert!(
        until(&map, 3_000, move |m| m
            .creature(slim)
            .is_none_or(|c| !c.alive() || c.despawned))
        .await,
        "he goes"
    );
}

/// Mikhail sends Slim out of the inn on The Missing Diplomat; left to run,
/// Slim reaches the gate, and that fails the quest (Thistle Tea's
/// `QuestEscort.Catalog` for 1249).
#[tokio::test]
#[ignore = "extended"]
async fn slim_reaching_the_gate_fails_the_missing_diplomat() {
    const MIKHAIL: u32 = 4963;
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map, mikhail) = hero_by(&realm, "Pursuer", MIKHAIL).await;
    let slim = map.query(|m| first(m, SLIM)).await.flatten().expect("Slim");
    map.query(move |m| {
        m.players
            .get_mut(&me)
            .unwrap()
            .journal
            .rewarded
            .extend([1248, 1247, 1246, 1244, 1243, 1242, 1241]);
        quests::accept(m, me, mikhail, THE_MISSING_DIPLOMAT_SLIM);
        for e in m.escorts.iter_mut() {
            e.skip_start();
        }
    })
    .await;
    assert!(
        until(&map, 3_000, move |m| m.players[&me]
            .journal
            .has(THE_MISSING_DIPLOMAT_SLIM))
        .await,
        "taken"
    );
    let mut failed = false;
    for _ in 0..1500 {
        let (f, over) = map
            .query(move |m| {
                if let Some(c) = m.creature_mut(slim) {
                    c.speeds.walk = 60.0;
                    c.speeds.run = 60.0;
                }
                if let Some(at) = m.position_of(slim) {
                    put(m, me, off(at, 1.0, 0.0));
                }
                let log = m.players[&me]
                    .journal
                    .log
                    .iter()
                    .find(|l| l.quest == THE_MISSING_DIPLOMAT_SLIM);
                (
                    log.is_some_and(|l| l.failed),
                    !m.escorts
                        .iter()
                        .any(|e| e.quest == THE_MISSING_DIPLOMAT_SLIM),
                )
            })
            .await
            .unwrap();
        if f {
            failed = true;
            break;
        }
        assert!(!over, "the escort ended without failing the quest");
        sleep_ms(20).await;
    }
    assert!(failed, "the gate reached fails the quest");
}

/// Whether `me` has `quest` failed in their log.
pub(super) fn failed(m: &Map, me: Guid, quest: u32) -> bool {
    m.players[&me]
        .journal
        .log
        .iter()
        .find(|l| l.quest == quest)
        .is_some_and(|l| l.failed)
}

/// Whether `me` has `quest`'s event or place done in their log.
pub(super) fn explored(m: &Map, me: Guid, quest: u32) -> bool {
    m.players[&me]
        .journal
        .log
        .iter()
        .find(|l| l.quest == quest)
        .is_some_and(|l| l.explored)
}

/// `quest` taken from `giver` again, as though new: whether it was.
fn take_again(m: &mut Map, me: Guid, giver: Guid, quest: u32, before: &[u32]) -> bool {
    let p = m.players.get_mut(&me).unwrap();
    p.journal.log.retain(|l| l.quest != quest);
    p.journal.rewarded.remove(&quest);
    p.journal.rewarded.extend(before.iter().copied());
    let data = m.realm.data.clone();
    quests::refresh_log(m.players.get_mut(&me).unwrap(), &data);
    quests::accept(m, me, giver, quest);
    let taken = m.players[&me].journal.has(quest);
    if !taken {
        let q = data.npc.quests.get(&quest).unwrap();
        eprintln!(
            "{quest} not taken: can_take {} condition {} npc {:?} flags {:x} ranks {:?} {:?} combat {:?} at war {}",
            quests::can_take(&m.players[&me], q, &data),
            quests::condition_ok(m, me, giver, quest),
            crate::game::map::npc::npc_entry(m, me, giver, 0),
            npc_flags(m, giver),
            crate::game::map::reputation::reaction_rank(m, giver, me),
            crate::game::map::reputation::reaction_rank(m, me, giver),
            m.combat(giver).map(|f| (f.attacking, f.victim)),
            crate::game::map::reputation::faction_of(m, giver)
                .is_some_and(|f| crate::game::map::reputation::at_war(&m.players[&me], &data, f)),
        );
    }
    taken
}

const PIZNIK: u32 = 4276;
const GERENZOS_ORDERS: u32 = 1090;
const WINDSHEAR: [u32; 3] = [3998, 4001, 4003];

/// Gerenzo's Orders: Piznik's waves and the quest's credit come only
/// while the defense is under way, so a defense ended early brings
/// neither; and his death in it fails the quest and ends it (Thistle
/// Tea's `piznik`).
#[tokio::test]
#[ignore = "extended"]
async fn piznik_s_waves_and_credit_need_the_defense_and_his_death_fails_it() {
    let Some(realm) = realm() else {
        return;
    };
    let (_c, me, map, piznik) = hero_by(&realm, "Miner", PIZNIK).await;
    let goblins = |m: &Map| -> usize { WINDSHEAR.iter().map(|e| living(m, *e).len()).sum() };
    // Ended at once: nothing comes, and no credit.
    let (taken, before) = map
        .query(move |m| {
            tough(m, piznik);
            (take_again(m, me, piznik, GERENZOS_ORDERS, &[]), goblins(m))
        })
        .await
        .unwrap();
    assert!(taken);
    assert!(
        until(&map, 2_000, move |m| event_ai::phase(m, piznik) == 1).await,
        "defending"
    );
    let after = map
        .query(move |m| {
            crate::game::map::map_events::end(m, GERENZOS_ORDERS, false);
            for _ in 0..40 {
                hurry_runs(m, None, Duration::from_secs(5));
            }
            (goblins(m), explored(m, me, GERENZOS_ORDERS))
        })
        .await
        .unwrap();
    assert_eq!(
        after,
        (before, false),
        "no waves, no credit, the defense over"
    );
    // His death in it.
    let taken = map
        .query(move |m| {
            if let Some(c) = m.creature_mut(piznik) {
                c.fields.set_u32(UNIT_FIELD_HEALTH, 1);
            }
            event_ai::set_phase(m, piznik, 0);
            take_again(m, me, piznik, GERENZOS_ORDERS, &[])
        })
        .await
        .unwrap();
    assert!(taken);
    assert!(
        until(&map, 2_000, move |m| event_ai::phase(m, piznik) == 1).await,
        "defending"
    );
    let ended = map
        .query(move |m| {
            kill(m, me, piznik);
            hurry_runs(m, None, Duration::from_millis(100));
            (
                failed(m, me, GERENZOS_ORDERS),
                crate::game::map::map_events::active(m, GERENZOS_ORDERS),
            )
        })
        .await
        .unwrap();
    assert_eq!(ended, (true, false), "failed with him, and over");
}

const SHAKES: u32 = 2610;
const DEATH_FROM_BELOW: u32 = 667;

/// Death From Below: taking it, Shakes O'Breen sounds the alarm, stops
/// offering quests and stands to his defense; if he dies, or the player
/// strays over 150 yards, the quest fails and he is back (Thistle Tea's
/// `shakes_o_breen`).
#[tokio::test]
#[ignore = "extended"]
async fn shakes_sounds_the_alarm_and_fails_if_he_dies_or_the_player_strays() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, shakes) = hero_by(&realm, "Sailor", SHAKES).await;
    let home = map
        .query(move |m| m.position_of(shakes))
        .await
        .flatten()
        .unwrap();
    for stray in [false, true] {
        c.clear();
        let shakes = map
            .query(move |m| first(m, SHAKES))
            .await
            .flatten()
            .expect("Shakes");
        let taken = map
            .query(move |m| {
                put(m, me, off(home, 2.0, 0.0));
                // On speaking terms with his cove, whatever this hero's
                // standing there.
                crate::game::map::scripts::set_faction(m, shakes, 35);
                take_again(m, me, shakes, DEATH_FROM_BELOW, &[670])
            })
            .await
            .unwrap();
        assert!(taken);
        assert!(hears(&mut c, &realm, shakes, 6372).await, "battle stations");
        let standing = map
            .query(move |m| (event_ai::phase(m, shakes), npc_flags(m, shakes) & 0x2))
            .await
            .unwrap();
        assert_eq!(standing, (1, 0), "to his defense, no quests");
        let failed_now = map
            .query(move |m| {
                if stray {
                    put(m, me, off(home, 200.0, 0.0));
                } else {
                    kill(m, me, shakes);
                }
                check_events(m);
                hurry_runs(m, None, Duration::from_millis(100));
                (
                    failed(m, me, DEATH_FROM_BELOW),
                    crate::game::map::map_events::active(m, DEATH_FROM_BELOW),
                )
            })
            .await
            .unwrap();
        assert_eq!(failed_now, (true, false), "stray {stray}: failed");
        assert!(
            until(&map, 3_000, move |m| first(m, SHAKES).is_some_and(|g| {
                event_ai::phase(m, g) == 0 && npc_flags(m, g) & 0x2 != 0
            }))
            .await,
            "stray {stray}: he is back"
        );
    }
}

const DASHEL: u32 = 4961;
const THUG: u32 = 4969;
const DASHELS_BRAWL: u32 = 1447;

/// The Missing Diplomat's brawl: Dashel Stonefist gives up at a fifth of
/// his health only in the quest's fight; giving up, he and his thugs
/// turn friendly and he goes home, where he sends them off, each
/// answering him in its turn, and the quest is done for its player;
/// left alone, he settles it himself (Thistle Tea's `dashel_stonefist`).
#[tokio::test]
#[ignore = "extended"]
async fn dashel_gives_up_in_the_quest_fight_and_sends_his_thugs_off() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map, dashel) = hero_by(&realm, "Brawled", DASHEL).await;
    let usual = map.query(move |m| faction(m, dashel)).await.unwrap();
    // Not in the quest's fight: he fights on.
    let kept = map
        .query(move |m| {
            tough(m, dashel);
            crate::game::map::scripts::set_faction(m, dashel, 14);
            let d = dummy(m, off(m.position_of(dashel).unwrap(), 3.0, 0.0));
            crate::game::map::ai::attacked(m, dashel, d);
            set_pct(m, dashel, 15);
            due(m, dashel, 496101);
            let kept = (event_ai::phase(m, dashel), faction(m, dashel));
            crate::game::map::scripts::set_faction(m, dashel, usual);
            crate::game::map::scripts::despawn(m, d);
            event_ai::evade(m, dashel);
            kept
        })
        .await
        .unwrap();
    assert_eq!(kept, (0, 14), "no giving up outside the quest's fight");
    assert!(
        until(&map, 5_000, move |m| at_peace(m, dashel)).await,
        "home again"
    );
    // The quest's fight.
    let taken = map
        .query(move |m| {
            if let Some(f) = m.unit_fields_mut(dashel) {
                let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
                f.set_u32(UNIT_FIELD_HEALTH, max);
            }
            take_again(m, me, dashel, DASHELS_BRAWL, &[1246])
        })
        .await
        .unwrap();
    assert!(taken, "taken");
    assert!(
        until(&map, 2_000, move |m| event_ai::phase(m, dashel) == 1
            && living(m, THUG).len() == 2)
        .await,
        "the fight, with two thugs"
    );
    let thugs: [Guid; 2] = map
        .query(|m| living(m, THUG))
        .await
        .unwrap()
        .try_into()
        .unwrap();
    let phases: Vec<u32> = map
        .query(move |m| {
            let mut p: Vec<u32> = thugs.iter().map(|t| event_ai::phase(m, *t)).collect();
            p.sort();
            p
        })
        .await
        .unwrap();
    assert_eq!(phases, [1, 2], "each thug its turn");
    c.clear();
    let gave_up = map
        .query(move |m| {
            for t in &thugs {
                tough(m, *t);
            }
            set_pct(m, dashel, 15);
            due(m, dashel, 496101);
            hurry_runs(m, None, Duration::from_millis(100));
            (
                event_ai::phase(m, dashel),
                faction(m, dashel),
                thugs.iter().map(|t| faction(m, *t)).collect::<Vec<_>>(),
                m.creature(dashel).is_some_and(|c| c.evading) || at_peace(m, dashel),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        gave_up,
        (2, 35, vec![35, 35], true),
        "yielded, everyone friendly, he going home"
    );
    assert!(hears(&mut c, &realm, dashel, 1712).await, "enough fighting");
    // Home, he sends them off.
    let settling = map
        .query(move |m| {
            for t in &thugs {
                if let Some(at) = m.position_of(dashel) {
                    crate::game::map::scripts::place(m, *t, off(at, 2.0, 0.0), 0.0);
                }
            }
            event_ai::reached_home(m, dashel);
            event_ai::phase(m, dashel)
        })
        .await
        .unwrap();
    assert_eq!(settling, 3, "settling");
    hurry_all(&map, 3.0).await;
    assert!(hears(&mut c, &realm, dashel, 1713).await, "back off");
    let (first, second) = map
        .query(move |m| {
            let first = thugs
                .iter()
                .copied()
                .find(|t| event_ai::phase(m, *t) == 1)
                .unwrap();
            let second = thugs.iter().copied().find(|t| *t != first).unwrap();
            (first, second)
        })
        .await
        .unwrap();
    hurry_all(&map, 3.0).await;
    assert!(
        hears(&mut c, &realm, first, 1716).await,
        "the first answers"
    );
    hurry_all(&map, 1.5).await;
    assert!(hears(&mut c, &realm, second, 1715).await, "then the second");
    let done = map
        .query(move |m| {
            for _ in 0..12 {
                hurry_runs(m, None, Duration::from_secs(1));
            }
            (
                explored(m, me, DASHELS_BRAWL),
                faction(m, dashel),
                npc_flags(m, dashel) & 0x2,
                event_ai::phase(m, dashel),
            )
        })
        .await
        .unwrap();
    assert_eq!(done, (true, usual, 0x2, 0), "credited, and himself again");
}
