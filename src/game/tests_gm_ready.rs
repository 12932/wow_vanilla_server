//! `.ready save|load` and `.respawn <radius>`, a raid attempt's reset,
//! through the protocol: the ready state kept and put back (risen, at its
//! place, whole, out of combat, its cooldowns clear, its buffs fresh from
//! their casters and its debuffs gone), on the GM or the player selected,
//! and across a relog; and every creature about the GM back as new, its
//! summons gone, one farther off untouched.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellDuration, SpellTemplate};
use crate::game::data::{CreatureSpawn, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::auras::{self, AURA_MOD_DECREASE_SPEED, AURA_MOD_STAT};
use crate::game::map::death::Death;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, MapHandle, MapKey, ai, combat};
use crate::game::tests_p3_combat::{BRUTE, START};
use crate::game::tests_p3_entity::{aura_effect, on, spell};

/// Bob's buff on Alice (ten stamina, half an hour), her own (ten
/// intellect, ten minutes), a brute's debuff (a slow), and a spell of hers
/// with a cooldown.
const FORTITUDE: u32 = 974_001;
const ARMOR: u32 = 974_002;
const SLOW: u32 = 974_003;
const BLAST: u32 = 974_004;
const HALF_HOUR: u32 = 1_800_000;
const TEN_MINUTES: u32 = 600_000;

/// Three spawns: two of them by Alice, one 80 yards off.
const DEAD_ONE: u32 = 801;
const HURT_ONE: u32 = 802;
const FAR_ONE: u32 = 803;

fn buff(id: u32, misc: i32, duration_index: u32) -> SpellTemplate {
    let mut s = spell(id, &[aura_effect(AURA_MOD_STAT, 10, misc, 21)]);
    s.duration_index = duration_index;
    s
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for (index, ms) in [(1, HALF_HOUR), (2, TEN_MINUTES)] {
        d.spells.durations.insert(
            index,
            SpellDuration {
                base: ms as i32,
                per_level: 0,
                max: ms as i32,
            },
        );
    }
    d.spells.spells.insert(FORTITUDE, buff(FORTITUDE, 2, 1));
    d.spells.spells.insert(ARMOR, buff(ARMOR, 3, 2));
    let mut slow = spell(SLOW, &[aura_effect(AURA_MOD_DECREASE_SPEED, -50, 0, 6)]);
    slow.duration_index = 2;
    d.spells.spells.insert(SLOW, slow);
    let mut blast = spell(BLAST, &[]);
    blast.recovery_time = 60_000;
    d.spells.spells.insert(BLAST, blast);
    let spawn = |guid, dx: f32| CreatureSpawn {
        guid,
        entries: vec![BRUTE],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(DEAD_ONE, 4.0),
            spawn(HURT_ONE, 8.0),
            spawn(FAR_ONE, 80.0),
        ],
    );
    d
}

/// Alice (a mage, the GM) and Bob (a priest) by her.
async fn setup() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    Client,
    Guid,
    MapHandle,
) {
    let realm = crate::game::harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Priest).await;
    b.heartbeat(START.0, START.1 + 2.0, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, b, gb, map)
}

fn apply(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let s = m.realm.data.spells.get(id).unwrap().clone();
    auras::apply_spell(m, caster, target, &s, &[(0, 10)], auras::Context::default()).unwrap();
}

/// Bob's Fortitude and Alice's own Armor on her, each with a minute left.
fn buffed(m: &mut Map, ga: Guid, gb: Guid) {
    apply(m, gb, ga, FORTITUDE);
    apply(m, ga, ga, ARMOR);
    let soon = m.now + Duration::from_secs(60);
    for h in m.players.get_mut(&ga).unwrap().auras.holders.iter_mut() {
        if h.spell == FORTITUDE || h.spell == ARMOR {
            h.expires = Some(soon);
        }
    }
}

async fn say(a: &mut Client, line: &str, reply: &str) -> String {
    a.say(line).await;
    a.system_message(reply).await
}

/// Each of `g`'s auras: its spell, caster, and the milliseconds left.
fn auras_of(m: &Map, g: Guid) -> Vec<(u32, Guid, Option<u64>)> {
    let now = m.now;
    m.players[&g]
        .auras
        .holders
        .iter()
        .filter(|h| !h.passive)
        .map(|h| {
            (
                h.spell,
                h.caster,
                h.expires
                    .map(|t| t.saturating_duration_since(now).as_millis() as u64),
            )
        })
        .collect()
}

fn health(m: &Map, g: Guid) -> (u32, u32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_MAXHEALTH),
    )
}

/// What `.ready load` must leave: alive at `at`, whole, full of mana,
/// out of combat and forgotten by `brute`, no cooldowns, Bob's Fortitude
/// and her Armor at their full time, nothing else.
fn assert_ready(m: &Map, ga: Guid, gb: Guid, at: Vector3d, brute: Guid) {
    let p = &m.players[&ga];
    assert_eq!(p.death, Death::Alive);
    let here = p.position();
    assert!(combat::dist(here, at) < 0.01, "{here:?}, saved {at:?}");
    let (hp, max) = health(m, ga);
    assert_eq!(hp, max, "whole");
    let mana = p.fields.get_u32(UNIT_FIELD_MAXPOWER1);
    assert!(mana > 0);
    assert_eq!(p.fields.get_u32(UNIT_FIELD_POWER1), mana, "full of mana");
    assert!(!p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT));
    assert!(p.caster.cooldowns.is_empty() && p.caster.gcd_until.is_none());
    if let Some(c) = m.creature(brute) {
        assert!(!c.threat.iter().any(|(t, _)| *t == ga), "{:?}", c.threat);
    }
    let mut on = auras_of(m, ga);
    on.sort_by_key(|a| a.0);
    assert_eq!(on.len(), 2, "{on:?}");
    let (fort, armor) = (on[0], on[1]);
    assert_eq!((fort.0, fort.1), (FORTITUDE, gb), "Bob's, as before");
    assert!(fort.2.unwrap() > u64::from(HALF_HOUR) - 5_000, "{fort:?}");
    assert_eq!((armor.0, armor.1), (ARMOR, ga));
    assert!(
        armor.2.unwrap() > u64::from(TEN_MINUTES) - 5_000,
        "{armor:?}"
    );
}

/// Saved buffed, then moved, hurt, out of mana, slowed, fighting a brute
/// that holds her on its list, a minute's cooldown on a spell and the
/// global one, Bob's buff run out: `.ready load` puts her back.
#[tokio::test]
async fn ready_load_puts_a_hurt_fighting_player_back_as_she_was_saved() {
    let (_realm, mut a, ga, _b, gb, map) = setup().await;
    on(&map, move |m| buffed(m, ga, gb)).await;
    let saved = say(&mut a, ".ready save", "Ready state saved").await;
    assert!(saved.contains("2 buffs"), "{saved}");
    let at = Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    };
    a.heartbeat(START.0 + 20.0, START.1, START.2).await;
    let brute = guid::unit(BRUTE, HURT_ONE);
    let fighting = on(&map, move |m| {
        auras::remove_spell(m, ga, FORTITUDE, None);
        apply(m, brute, ga, SLOW);
        ai::attacked(m, brute, ga);
        ai::add_threat(m, brute, ga, 100.0);
        combat::deal_damage(m, brute, ga, 50);
        let now = m.now;
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_POWER1, 0);
        p.caster
            .cooldowns
            .insert(BLAST, now + Duration::from_secs(60));
        p.caster.gcd_until = Some(now + Duration::from_millis(1500));
        let in_combat = p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
        let (hp, max) = health(m, ga);
        (in_combat, hp < max, auras_of(m, ga).len())
    })
    .await;
    assert_eq!(
        fighting,
        (true, true, 2),
        "fighting, hurt, Armor and the slow"
    );
    a.clear();
    let loaded = say(&mut a, ".ready load", "is ready").await;
    assert!(loaded.contains("2 of 2 buffs back"), "{loaded}");
    a.wait_for("the blast's cooldown cleared", |m| match m {
        ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(c) if c.id == BLAST => Some(()),
        _ => None,
    })
    .await;
    on(&map, move |m| assert_ready(m, ga, gb, at, brute)).await;
}

/// Dead, and a ghost: `.ready load` raises her where she was saved, as
/// she was.
#[tokio::test]
async fn ready_load_raises_the_dead_and_ghosts_at_the_saved_place() {
    let (_realm, mut a, ga, _b, gb, map) = setup().await;
    on(&map, move |m| buffed(m, ga, gb)).await;
    say(&mut a, ".ready save", "Ready state saved").await;
    let at = Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    };
    let brute = guid::unit(BRUTE, HURT_ONE);
    let dead = on(&map, move |m| {
        crate::game::map::death::kill(m, ga);
        m.players[&ga].death != Death::Alive
    })
    .await;
    assert!(dead);
    say(&mut a, ".ready load", "is ready").await;
    on(&map, move |m| assert_ready(m, ga, gb, at, brute)).await;
    // Released, a ghost elsewhere.
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    a.send(CMSG_REPOP_REQUEST {}).await;
    let ghost = {
        let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
        loop {
            let d = on(&map, move |m| m.players[&ga].death).await;
            if matches!(d, Death::Ghost { .. }) || tokio::time::Instant::now() > deadline {
                break d;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    assert!(matches!(ghost, Death::Ghost { .. }), "{ghost:?}");
    say(&mut a, ".ready load", "is ready").await;
    on(&map, move |m| assert_ready(m, ga, gb, at, brute)).await;
}

/// Bob, selected, is put back in his own ready state by Alice's
/// `.ready load`; with none, she is told so. A player may not.
#[tokio::test]
async fn ready_load_puts_back_the_selected_player_and_a_player_may_not() {
    let (_realm, mut a, _ga, mut b, gb, map) = setup().await;
    a.send(CMSG_SET_SELECTION { target: gb }).await;
    say(&mut a, ".ready load", "Bob has no ready state").await;
    say(&mut b, ".ready save", "Ready state saved").await;
    on(&map, move |m| {
        combat::deal_damage(m, guid::unit(BRUTE, HURT_ONE), gb, 50);
    })
    .await;
    say(&mut a, ".ready load", "Bob is ready").await;
    let (hp, max) = on(&map, move |m| health(m, gb)).await;
    assert_eq!(hp, max);
    on(&map, move |m| m.players.get_mut(&gb).unwrap().gm_level = 0).await;
    say(&mut b, ".ready load", "There is no such command").await;
}

/// The ready state is kept on the character: after a relog `.ready` shows
/// it and `.ready load` puts the buffs back.
#[tokio::test]
async fn the_ready_state_survives_a_relog() {
    let (_realm, mut a, ga, _b, gb, map) = setup().await;
    on(&map, move |m| buffed(m, ga, gb)).await;
    say(&mut a, ".ready save", "Ready state saved").await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(ga).await;
    let shown = say(&mut a, ".ready", "Ready state at").await;
    assert!(shown.contains("2 buffs"), "{shown}");
    on(&map, move |m| auras::remove_spell(m, ga, ARMOR, None)).await;
    say(&mut a, ".ready load", "2 of 2 buffs back").await;
    let on_her = on(&map, move |m| auras_of(m, ga)).await;
    assert!(
        on_her.iter().any(|a| a.0 == ARMOR && a.1 == ga),
        "{on_her:?}"
    );
}

/// `.respawn 50`: a killed brute back, a hurt one fighting Alice and drawn
/// off its spawn back there whole and out of the fight, its summons gone;
/// the one 80 yards off, hurt, left as it is.
#[tokio::test]
async fn respawn_around_brings_back_and_resets_whats_near_and_leaves_whats_far() {
    let (_realm, mut a, ga, _b, _gb, map) = setup().await;
    let (dead, hurt, far) = (
        guid::unit(BRUTE, DEAD_ONE),
        guid::unit(BRUTE, HURT_ONE),
        guid::unit(BRUTE, FAR_ONE),
    );
    let (summon, far_health) = on(&map, move |m| {
        combat::kill(m, ga, dead);
        ai::attacked(m, hurt, ga);
        combat::deal_damage(m, ga, hurt, 100);
        if let Some(c) = m.creature_mut(hurt) {
            c.position.y += 10.0;
        }
        combat::deal_damage(m, ga, far, 100);
        // A creature the hurt one summoned.
        let summon = crate::game::tests_p3_combat::spawn(m, ga, BRUTE, 20, 2.0);
        crate::game::map::event_ai::scripted_mut(m, summon)
            .unwrap()
            .summoner = Some(hurt);
        let far_health = health(m, far).0;
        assert!(!m.creature(dead).unwrap().alive());
        assert!(m.creature(hurt).unwrap().combat.attacking);
        (summon, far_health)
    })
    .await;
    let reply = say(&mut a, ".respawn 50", "Respawned").await;
    assert!(reply.contains("2 creatures"), "{reply}");
    assert!(reply.contains("1 summons gone"), "{reply}");
    let (back, home, gone, far_now) = on(&map, move |m| {
        let d = m.creature(dead).unwrap();
        let h = m.creature(hurt).unwrap();
        let spawn = Vector3d {
            x: START.0 + 8.0,
            y: START.1,
            z: START.2,
        };
        (
            d.alive() && health(m, dead).0 == health(m, dead).1,
            (
                combat::dist(h.position, spawn) < 0.01,
                health(m, hurt).0 == health(m, hurt).1,
                h.combat.attacking,
                h.threat.is_empty(),
            ),
            m.creature(summon).is_none(),
            health(m, far).0,
        )
    })
    .await;
    assert!(back, "the dead one back whole");
    assert_eq!(
        home,
        (true, true, false, true),
        "home, whole, out of the fight"
    );
    assert!(gone, "its summons gone");
    assert_eq!(far_now, far_health, "the far one untouched");
    for bad in [
        ".respawn 0",
        ".respawn 501",
        ".respawn lots",
        ".respawn 5 5",
    ] {
        say(&mut a, bad, "Usage: .respawn").await;
    }
}
