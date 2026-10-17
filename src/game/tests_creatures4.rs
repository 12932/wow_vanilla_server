//! Creatures as Thistle Tea has them past the fight: where they may go
//! (water and air), wounded ones slowing, pausing while a player talks to
//! them, critters running; the War Effort's events, LocalDefense's
//! alerts, and a pet's failed casts told to its master.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn template(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_max: 1000,
        ..Default::default()
    }
}

fn spawn(guid: u32, entry: u32, at: (f32, f32, f32)) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: at.0,
        y: at.1,
        z: at.2,
        ..Default::default()
    }
}

async fn enter(
    realm: &std::sync::Arc<crate::game::Realm>,
    account: &str,
    name: &str,
    race: Race,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

/// The War Effort is over in this patch: only its post-war watch runs.
#[test]
fn the_war_effort_is_over() {
    let mut d = GameData::empty();
    for id in [84, 85, 86] {
        d.events.insert(
            id,
            GameEvent {
                id,
                hardcoded: true,
                ..Default::default()
            },
        );
    }
    let now = chrono::Utc::now().timestamp();
    let on = crate::game::events::active(&d, now);
    assert!(on.contains(&86), "the post-war watch");
    assert!(!on.contains(&84) && !on.contains(&85), "no war, no gong");
}

const GUARD: u32 = 68;

/// A town guard felled by an Alliance player warns the Horde on the map
/// that its area is under attack, and not again for ten seconds.
#[tokio::test]
async fn a_fallen_guard_warns_the_other_side() {
    let mut d = GameData::empty();
    let mut guard = template(GUARD, "Stormwind City Guard", 11);
    guard.flags_extra = 0x400;
    d.creature_templates.insert(GUARD, guard);
    d.creature_spawns.insert(
        0,
        vec![
            spawn(1, GUARD, (START.0 + 3.0, START.1, START.2)),
            spawn(2, GUARD, (START.0 - 3.0, START.1, START.2)),
        ],
    );
    let realm = harness::realm_with(d);
    let (_a, me, map) = enter(&realm, "A", "Alliance", Race::Human).await;
    let (mut h, horde, _) = enter(&realm, "H", "Horde", Race::Undead).await;
    let (mut b, _, _) = enter(&realm, "B", "Ally", Race::Human).await;
    assert_eq!(
        realm.registry.online(horde).unwrap().map().key,
        map.key,
        "all on one map"
    );
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&me) {
            p.area = 12;
        }
    });
    let guards = map
        .query(|m| {
            m.creatures
                .iter()
                .filter(|(_, c)| c.entry == GUARD)
                .map(|(_, c)| c.guid)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    let first = guards[0];
    map.call(move |m| crate::game::map::combat::deal_damage(m, me, first, 1_000_000));
    let area = h
        .wait_frame("SMSG_ZONE_UNDER_ATTACK", 0x254, |_| true)
        .await;
    assert_eq!(area, 12u32.to_le_bytes(), "Elwynn is under attack");
    h.clear();
    let told_own = b
        .gets(Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ZONE_UNDER_ATTACK(_)).then_some(())
        })
        .await;
    assert!(told_own.is_none(), "the attackers' side isn't warned");
    // The second guard, at once: quiet.
    let second = guards[1];
    map.call(move |m| crate::game::map::combat::deal_damage(m, me, second, 1_000_000));
    let again = h
        .gets(Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ZONE_UNDER_ATTACK(_)).then_some(())
        })
        .await;
    assert!(again.is_none(), "not again within ten seconds");
}

/// A pet told to cast at a target out of its reach tells its master why.
#[tokio::test]
async fn a_pets_failed_cast_is_told_to_its_master() {
    use crate::game::tests_pets as tp;
    let realm = harness::realm_with(tp::data());
    let (mut w, gw, map) = tp::enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    tp::learn(&map, gw, &[tp::SUMMON_IMP]);
    let wolf = tp::place(&map, gw, tp::WOLF, 0x70_0009, 80.0).await;
    w.sees(wolf).await;
    tp::cast(&w, tp::SUMMON_IMP, None).await;
    let imp = tp::pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    w.sees(imp).await;
    w.send(CMSG_PET_CAST_SPELL {
        guid: imp,
        id: tp::FIREBOLT,
        targets: tp::at(wolf),
    })
    .await;
    let failed = w.wait_frame("SMSG_PET_CAST_FAILED", 0x138, |_| true).await;
    let mut want = tp::FIREBOLT.to_le_bytes().to_vec();
    want.extend([2, crate::game::map::spells::fail::OUT_OF_RANGE]);
    assert_eq!(failed, want, "out of range");
    w.wait_for("the cooldown cleared on the bar", |m| match m {
        ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(c) if c.id == tp::FIREBOLT && c.target == imp => {
            Some(())
        }
        _ => None,
    })
    .await;
}
