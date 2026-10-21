//! Leaving the world dead before release (a logout, a disconnect, a spirit
//! of redemption's logout): released first, as mangos-classic's
//! `WorldSession::LogoutPlayer` has them (`BuildPlayerRepop`,
//! `RepopAtGraveyard`), so they come back a ghost at the graveyard, their
//! corpse where they fell, not risen there at full health.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage as S;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{GameData, Graveyard};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::death::{self, Death};
use crate::game::map::spells::redemption;
use crate::game::map::{Map, MapHandle, auras, combat};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
/// The graveyard, 50 yards east.
const GRAVEYARD_X: f32 = START.0 + 50.0;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.graveyards.push(Graveyard {
        id: 1,
        map: 0,
        x: GRAVEYARD_X,
        y: START.1,
        z: START.2,
        orientation: 0.0,
        zone: 0,
        faction: 0,
    });
    for id in std::iter::once(redemption::FORM).chain(redemption::STATE) {
        let mut s = SpellTemplate {
            id,
            name: "Spirit of Redemption".into(),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: auras::AURA_DUMMY,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn map_of(realm: &Arc<Realm>, g: Guid) -> MapHandle {
    realm.registry.online(g).expect("online").map()
}

/// Log out through the protocol, waiting for the character screen.
async fn log_out(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, S::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
}

/// How `g` is on their map: their death, health, x, and whether their
/// corpse lies at the start, where they fell.
async fn state(realm: &Arc<Realm>, g: Guid) -> (Death, u32, f32, bool) {
    let map = map_of(realm, g);
    on(&map, move |m| {
        let p = &m.players[&g];
        let corpse = m
            .corpses
            .get(&guid::corpse(guid::counter(g)))
            .is_some_and(|c| (c.position.x - START.0).abs() < 1.0);
        (
            p.death,
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.position().x,
            corpse,
        )
    })
    .await
}

fn is_ghost(d: Death) -> bool {
    matches!(d, Death::Ghost { .. })
}

/// A player who logs out dead, before release, logs back in a ghost at
/// the graveyard with one health point, their corpse where they fell.
/// Before, they were saved dead and unreleased, and rose at their next
/// login at full health where they fell: no corpse run.
#[tokio::test]
async fn logging_out_dead_before_release_comes_back_a_ghost_at_the_graveyard() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "DEADOUT");
    let g = a.enter("Deadout", Race::Human, Class::Warrior).await;
    on(&map_of(&realm, g), move |m| death::kill(m, g)).await;
    log_out(&mut a).await;
    a.login(g).await;
    let (d, health, x, corpse) = state(&realm, g).await;
    assert!(is_ghost(d), "a ghost: {d:?}");
    assert_eq!(health, 1, "a ghost's one health point");
    assert!((x - GRAVEYARD_X).abs() < 1.0, "at the graveyard: {x}");
    assert!(corpse, "their corpse where they fell");
}

/// So too a client that goes dead, before release: at its leaving it is
/// released, and comes back a ghost at the graveyard.
#[tokio::test]
async fn disconnecting_dead_before_release_comes_back_a_ghost() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "DEADDC");
    let g = a.enter("Deaddc", Race::Human, Class::Warrior).await;
    on(&map_of(&realm, g), move |m| death::kill(m, g)).await;
    a.disconnect();
    harness::offline(&realm, g).await;
    let mut b = Client::connect(&realm, "DEADDC");
    b.login(g).await;
    let (d, health, x, corpse) = state(&realm, g).await;
    assert!(is_ghost(d), "a ghost: {d:?}");
    assert_eq!(health, 1);
    assert!((x - GRAVEYARD_X).abs() < 1.0, "at the graveyard: {x}");
    assert!(corpse, "their corpse where they fell");
}

/// A spirit of redemption who logs out dies, as mangos's `LogoutPlayer`
/// kills one, and is released: a ghost at the graveyard at their next
/// login. Before, they were saved a spirit at full health and came back
/// alive.
#[tokio::test]
async fn a_spirit_of_redemption_logging_out_comes_back_a_ghost() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "SPIRITOUT");
    let g = a.enter("Spiritout", Race::Human, Class::Priest).await;
    let spirit = on(&map_of(&realm, g), move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .spells
            .push(redemption::TALENT);
        let max = m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
        combat::deal_damage(m, g, g, max + 100);
        redemption::spirit(m, g)
    })
    .await;
    assert!(spirit, "a spirit of redemption");
    log_out(&mut a).await;
    a.login(g).await;
    let (d, _, x, corpse) = state(&realm, g).await;
    assert!(is_ghost(d), "a ghost: {d:?}");
    assert!((x - GRAVEYARD_X).abs() < 1.0, "at the graveyard: {x}");
    assert!(corpse);
}

/// One who logged out alive comes back alive, where they were: nothing
/// released.
#[tokio::test]
async fn logging_out_alive_releases_nothing() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "ALIVEOUT");
    let g = a.enter("Aliveout", Race::Human, Class::Warrior).await;
    log_out(&mut a).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    a.login(g).await;
    let (d, health, x, corpse) = state(&realm, g).await;
    assert_eq!(d, Death::Alive);
    assert!(health > 1);
    assert!((x - START.0).abs() < 1.0, "where they were: {x}");
    assert!(!corpse);
}
