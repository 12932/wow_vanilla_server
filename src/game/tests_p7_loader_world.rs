//! Phase 7's loader round, the world (Thistle Tea's `world/loader/` tests
//! of weather, factions, transports, fishing and trainers): what wvs reads
//! of the real world, and what it does with it.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
use crate::game::map::professions::{fishing_loot_at, fishing_skill_at};
use crate::game::map::{Map, MapHandle};

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// Thistle Tea's weather seed (`weather_test.exs`): a chance over 100, or
/// under 0 (mangos's unsigned column wraps it over), is 25, as mangos's
/// `LoadWeatherZoneChances` repairs it; one within is as it is.
#[test]
fn a_weather_chance_out_of_range_is_25() {
    use crate::game::data::load::weather_chance;
    assert_eq!(weather_chance(20), 20);
    assert_eq!(weather_chance(0), 0);
    assert_eq!(weather_chance(100), 100);
    assert_eq!(weather_chance(101), 25);
    assert_eq!(weather_chance(-1), 25);
}

/// Thistle Tea's factions and the tram (`reputation_dbc_test.exs`,
/// `transport_dbc_test.exs`): the client's 54 factions with a reputation
/// slot, the Bloodsail Buccaneers (87) in slot 0 at -6500, and faction 1
/// (a player's, of no slot) not among them; the Deeprun Tram's car
/// (176080) runs its 53 frames over 143,333 ms, its last.
#[test]
#[ignore = "extended"]
fn real_factions_and_the_tram_load() {
    let Some(d) = real() else {
        return;
    };
    assert_eq!(d.misc.factions.len(), 54);
    let bloodsail = &d.misc.factions[&87];
    assert_eq!(bloodsail.name, "Bloodsail Buccaneers");
    assert_eq!(bloodsail.index, 0);
    assert_eq!(bloodsail.variants[0].2, -6500);
    assert_eq!(d.misc.faction_by_index[&0], 87);
    assert!(!d.misc.factions.contains_key(&1));
    let tram = &d.world.animations[&176_080];
    assert_eq!(tram.len(), 53);
    assert_eq!(tram.last().unwrap().time_ms, 143_333);
    assert!(tram.windows(2).all(|w| w[0].time_ms <= w[1].time_ms));
}

/// Thistle Tea's fishing (`fishing_integration_test.exs`): the skill an
/// area's water asks for is its own, else its zone's (Jasperlode Mine's,
/// 327, is Elwynn's, 45: 130, and an unknown area's in Elwynn too); its
/// loot is the area's own table, else its zone's, each item a template.
#[test]
#[ignore = "extended"]
fn real_fishing_falls_back_from_the_area_to_its_zone() {
    use rand::SeedableRng;
    let Some(d) = real() else {
        return;
    };
    assert!(!d.crafts.fishing_skill.contains_key(&327));
    assert_eq!(fishing_skill_at(d, 327, 45), 130);
    assert_eq!(fishing_skill_at(d, 999_999, 45), 130);
    for e in d.loot.fishing.values().flatten().filter(|e| e.min > 0) {
        assert!(d.item(e.item).is_some(), "{}", e.item);
    }
    let area = fishing_loot_at(d, 327, 45);
    let zone = fishing_loot_at(d, 999_999, 45);
    assert!(std::ptr::eq(area, d.loot.fishing[&327].as_slice()));
    assert!(std::ptr::eq(zone, d.loot.fishing[&45].as_slice()));
    let mut rng = rand::rngs::StdRng::seed_from_u64(3);
    for table in [area, zone] {
        let mut out = Vec::new();
        for _ in 0..5 {
            crate::game::map::loot::roll(d, table, &[], &mut rng, &mut out, 0);
        }
        assert!(!out.is_empty());
        let items: Vec<u32> = table.iter().map(|e| e.item).collect();
        assert!(out.iter().all(|s| items.contains(&s.item)));
    }
}

/// Thistle Tea's fishing trainer and lures (`fishing_integration_test.exs`,
/// `item_enchantment_integration_test.exs`): Lee Brown (3179) teaches
/// Apprentice Fishing (7733, learning 7620, to 75) and Journeyman (7734,
/// learning 7731, to 150), green to one who knows the first and has 50
/// of it. A Shiny Bauble's lure (8087) on a fishing pole lasts ten
/// minutes (vmangos's `spell_effect_mod`) and adds 25 to fishing; the
/// Aquadynamic Fish Attractor's (8089) lasts five; enchantment 265 is
/// fishing's +75.
#[tokio::test]
#[ignore = "extended"]
async fn real_fishing_is_trained_and_lured() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let learns = |s| crate::game::map::spells::learns(&d, s);
    assert_eq!((learns(7733), learns(7734)), (7620, 7731));
    let step = |s: u32| {
        d.spells.get(s).and_then(|s| {
            s.effects
                .iter()
                .find(|e| e.effect == 44)
                .map(|e| (e.misc_value, (e.base_points + 1) * 75))
        })
    };
    assert_eq!(step(7733), Some((356, 75)));
    assert_eq!(step(7734), Some((356, 150)));
    let lure = |s: u32| crate::game::map::spells::points(d.spells.get(s).unwrap(), 0, 1);
    assert_eq!((lure(8087), lure(8089)), (600, 300));
    let skill_of = |en: u32| {
        let (kind, _, _, spell) = d.spells.enchantments[&en].effects[0];
        let s = d.spells.get(spell).unwrap();
        (
            kind,
            s.effects[0].aura,
            s.effects[0].misc_value,
            crate::game::map::spells::points(s, 0, 1),
        )
    };
    assert_eq!(skill_of(263), (3, 30, 356, 25));
    assert_eq!(skill_of(265), (3, 30, 356, 75));

    let (mut c, g, map) = player(&realm, "P7LFISH", "Fisha", Race::Human, Class::Warrior).await;
    let trainer = on(&map, move |m| {
        ready(m, g, 10, &[7620]);
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::skills::learn_skill(p, 356, 1);
        crate::game::map::skills::set_value(p, 356, 50);
        spawn(m, g, 3179, 30, 2.0)
    })
    .await;
    c.clear();
    c.send(CMSG_TRAINER_LIST { guid: trainer }).await;
    let list = c
        .wait_for("SMSG_TRAINER_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_TRAINER_LIST(l) => Some(l.spells.clone()),
            _ => None,
        })
        .await;
    let state = |spell: u32| list.iter().find(|s| s.spell == spell).map(|s| s.state);
    assert_eq!(state(7733), Some(TrainerSpellState::Gray));
    assert_eq!(state(7734), Some(TrainerSpellState::Green));

    let out = on(&map, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .items
            .retain(|i| !(i.bag == crate::game::store::BAG_PLAYER && i.slot == 15));
        let pole = crate::game::tests_p4_items_gear::wear(m, g, 6256, 15);
        let before = crate::game::map::skills::value_with_bonuses(&m.players[&g], 356);
        let t = crate::game::map::spells::Targets {
            item: Some(pole),
            ..Default::default()
        };
        let began = cast_fresh(m, g, 8087, t);
        pass(m, 6000);
        crate::game::map::inventory::sync_gear(m, g);
        let p = &m.players[&g];
        let after = crate::game::map::skills::value_with_bonuses(p, 356);
        let left = p.items.iter().find(|i| i.guid == pole).map(|i| {
            crate::game::map::enchant::get(i, crate::game::map::enchant::TEMP_SLOT).duration_ms
        });
        (began, before, after, left)
    })
    .await;
    let (began, before, after, left) = out;
    assert!(began);
    assert_eq!((before, after), (50, 75));
    assert!(
        left.is_some_and(|ms| ms > 590_000 && ms <= 600_000),
        "{left:?}"
    );
}
