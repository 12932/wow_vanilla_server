//! The server's gaps the talent audits found (`.gear`'s hands, talents'
//! effects that never took hold, the spells a quest or a book teaches),
//! each on the real data when it is here.

use std::sync::Arc;
use std::time::Duration;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::spells;
use crate::game::map::{Map, MapHandle, MapKey, auras};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

/// A character of `race` and `class` in the world, and their map.
async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let continent = if matches!(
        race,
        Race::Orc | Race::Tauren | Race::Troll | Race::NightElf
    ) {
        1
    } else {
        0
    };
    let map = realm.maps.get(realm, MapKey::world(continent));
    (c, g, map)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// `g` at `level`, knowing `learn`.
fn ready(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
    }
}

fn power(m: &Map, g: Guid, power: u16) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1 + power))
}

/// The map's clock `by` on, and its auras' ticks to then.
fn later(m: &mut Map, by: Duration) {
    m.now += by;
    auras::update(m);
}

const ANGER_MANAGEMENT: u32 = 12_296;

/// Anger Management (12296, a regeneration of rage with no amplitude in
/// the DBC) gives three fifths of its 15 a tick, every 3 s, in combat
/// only (mangos's `HandleModPowerRegen` and `PeriodicTick`): 0.9 rage.
#[tokio::test]
#[ignore = "extended"]
async fn anger_management_gives_rage_every_three_seconds_in_combat() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_w, w, map) = enter(&realm, "TSRVAM1", "Angera", Race::Orc, Class::Warrior).await;
    let rage = on(&map, move |m| {
        ready(m, w, 60, &[ANGER_MANAGEMENT]);
        let set = |m: &mut Map, combat: bool| {
            let f = m.unit_fields_mut(w).unwrap();
            f.set_u32(UNIT_FIELD_POWER1 + 1, 0);
            f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, combat);
        };
        set(m, true);
        later(m, Duration::from_millis(3_000));
        let first = power(m, w, 1);
        later(m, Duration::from_millis(1_500));
        let between = power(m, w, 1);
        later(m, Duration::from_millis(1_500));
        let second = power(m, w, 1);
        set(m, false);
        later(m, Duration::from_millis(3_000));
        (first, between, second, power(m, w, 1))
    })
    .await;
    assert_eq!(
        rage,
        (9, 9, 18, 0),
        "rage at 3, 4.5 and 6 s, then out of combat"
    );
}
