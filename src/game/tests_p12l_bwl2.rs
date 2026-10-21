//! Blackwing Lair on the real world, as mangos-classic scripts it, its
//! talking bosses: Vaelastrasz's corruption, his plea and his fight.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_scripts::*;
use super::tests_p12l_bwl::*;
use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::bwl;
use crate::game::map::bwl::vael as va;
use crate::game::map::{Map, MapHandle, MapKey, instance_script};

/// A level 60 `class` in `map`'s copy at `to`, tough, not a GM.
pub(super) async fn joins_as(
    realm: &Arc<Realm>,
    name: &str,
    class: Class,
    map: &MapHandle,
    to: Vector3d,
) -> (Client, Guid) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, class).await;
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let from = realm.registry.online(g).unwrap().map();
    from.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        p.gm = false;
    })
    .await;
    let key: MapKey = map.query(|m| m.key).await.unwrap();
    from.call(move |m| m.teleport(g, key, to, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    toughen(map, g).await;
    (c, g)
}

/// The living creature of `entry` the copy summoned.
pub(super) fn summoned(m: &Map, entry: u32) -> Option<Guid> {
    m.creatures
        .iter()
        .find(|(_, c)| c.entry == entry && c.alive() && c.spawn_guid.is_none())
        .map(|(_, c)| c.guid)
}

fn npc_flags_of(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_NPC_FLAGS))
}

/// A line of broadcast text `text` from `speaker`, heard within three
/// seconds (others heard meanwhile passed over).
pub(super) async fn says(realm: &Arc<Realm>, c: &mut Client, speaker: Guid, text: u32) -> bool {
    let line = realm
        .data
        .npc
        .broadcast_texts
        .get(&text)
        .map(|b| b.male.clone())
        .unwrap_or_default();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        match heard(c, speaker, left).await {
            Some(l) if l.trim() == line.trim() => return true,
            Some(_) => continue,
            None => return false,
        }
    }
}

/// In Vaelastrasz's hall, inside its trigger.
const VAEL_HALL: Vector3d = Vector3d {
    x: -7529.1,
    y: -1007.19,
    z: 408.6,
};

/// Vaelastrasz: dying on the floor at a third of his health, no one to
/// speak to; a player into his hall brings Nefarius, who corrupts him;
/// then he may be spoken to, and pressed twice he speaks his three lines
/// and turns on the copy; he fights as mangos's `CombatAI` has it, Burning
/// Adrenaline on a mana user among it; his death opens his door.
#[tokio::test]
#[ignore = "extended"]
async fn vaelastrasz_is_corrupted_pleads_and_fights() {
    let Some(realm) = realm() else {
        return;
    };
    let (mut c, me, map) = hero_in(&realm, "Vaelfriend", BWL, VAEL_HALL, 0.0, &[]).await;
    not_gm(&map, me).await;
    let vael = boss(&map, bwl::NPC_VAELASTRASZ).await;
    c.sees(vael).await;
    let (state, flags, (hp, max)) = map
        .query(move |m| (stand_state(m, vael), npc_flags_of(m, vael), health(m, vael)))
        .await
        .unwrap();
    assert_eq!(state, 7, "he lies dying");
    assert_eq!(flags & 0x3, 0, "no one to speak to yet");
    assert!(hp * 100 / max <= 30, "at a third of his health: {hp}/{max}");
    c.send(CMSG_AREATRIGGER {
        trigger_id: va::AT_VAEL_INTRO,
    })
    .await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::VAELASTRASZ) == bwl::SPECIAL).await,
        "the intro begins"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(2))).await;
    let nef = {
        let mut n = None;
        for _ in 0..80 {
            n = map
                .query(|m| summoned(m, bwl::NPC_VICTOR_NEFARIUS))
                .await
                .flatten();
            if n.is_some() {
                break;
            }
            sleep_ms(25).await;
        }
        n.expect("Nefarius at the throne")
    };
    map.query(|m| hurry_copy(m, Duration::from_secs(2))).await;
    assert!(
        says(&realm, &mut c, nef, va::SAY_NEFARIUS_CORRUPT_1).await,
        "Nefarius's first line"
    );
    assert!(
        map.query(move |m| has(m, vael, va::NEFARIUS_CORRUPTION))
            .await
            .unwrap(),
        "corrupted"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(15))).await;
    assert!(says(&realm, &mut c, nef, va::SAY_NEFARIUS_CORRUPT_2).await);
    map.query(|m| hurry_copy(m, Duration::from_secs(3))).await;
    assert!(
        until(&map, 2000, move |m| npc_flags_of(m, vael) & 0x1 != 0).await,
        "he may be spoken to"
    );
    // Pressed twice, at his side.
    relocate(
        &map,
        &mut c,
        Vector3d {
            x: -7486.0,
            y: -1013.0,
            z: 408.7,
        },
    )
    .await;
    hello(&c, vael).await;
    let first_option = option(&mut c, vael, "I cannot, Vaelastrasz!").await;
    choose(&c, vael, first_option).await;
    let second = option(&mut c, vael, "Vaelastrasz, no!!!").await;
    choose(&c, vael, second).await;
    assert!(
        says(&realm, &mut c, vael, va::SAY_LINE_1).await,
        "his first line"
    );
    assert_eq!(
        map.query(move |m| stand_state(m, vael)).await,
        Some(0),
        "he stands"
    );
    map.query(|m| hurry_copy(m, Duration::from_secs(11))).await;
    assert!(says(&realm, &mut c, vael, va::SAY_LINE_2).await);
    map.query(|m| hurry_copy(m, Duration::from_secs(17))).await;
    assert!(says(&realm, &mut c, vael, va::SAY_LINE_3).await);
    let (_m, mage) = joins_as(&realm, "Vaelmage", Class::Mage, &map, VAEL_HALL).await;
    map.query(|m| hurry_copy(m, Duration::from_secs(11))).await;
    assert!(
        until(&map, 3000, move |m| faction(m, vael) == 14
            && field(m, bwl::VAELASTRASZ) == instance_script::IN_PROGRESS)
        .await,
        "he turns on them"
    );
    // His fight: some of his timed casts, and Burning Adrenaline on the mage.
    let e = |n: u32| bwl::NPC_VAELASTRASZ * 100 + n;
    map.query(move |m| tough(m, vael)).await;
    timed_cast(
        &mut c,
        &map,
        vael,
        e(3),
        va::FIRE_NOVA,
        (5.0, 5.0),
        (5.0, 5.0),
        None,
    )
    .await;
    timed_cast(
        &mut c,
        &map,
        vael,
        e(5),
        va::TAIL_SWEEP,
        (20.0, 20.0),
        (20.0, 20.0),
        None,
    )
    .await;
    map.query(move |m| {
        for (t, k, g) in m.instance.creature_timers.iter_mut() {
            if *g == vael && *k == crate::game::map::instance_adapters::TIMERS + 6220 {
                *t = m.now;
            }
        }
        crate::game::map::instance_script::update(m);
    })
    .await;
    assert!(
        until(&map, 3000, move |m| has(m, mage, va::BURNING_ADRENALINE)).await,
        "Burning Adrenaline on the mana user"
    );
    assert!(
        !map.query(move |m| has(m, me, va::BURNING_ADRENALINE))
            .await
            .unwrap(),
        "not on the warrior"
    );
    map.query(move |m| kill(m, me, vael)).await;
    assert!(
        until(&map, 2000, |m| field(m, bwl::VAELASTRASZ)
            == instance_script::DONE
            && door_state(m, bwl::DOOR_VAELASTRASZ) == Some(0))
        .await,
        "his death opens his door"
    );
}
