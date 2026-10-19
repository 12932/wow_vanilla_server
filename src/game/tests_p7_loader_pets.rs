//! Phase 7's loader round, pets and spells learned with spells (Thistle
//! Tea's `world/loader/` tests of demons' names, pet training and passive
//! spells): what wvs reads of them, and what they do.

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
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

/// Thistle Tea's demon names (`pet_name_test.exs`,
/// `pet_name_vmangos_test.exs`): every warlock demon (the imp 416, the
/// voidwalker 1860, the succubus 1863, the felhunter 417) has more than
/// 20 openings and closings; a summoned imp's name is one opening and one
/// closing joined, a capital and lower case.
#[tokio::test]
#[ignore = "extended"]
async fn a_demons_name_joins_an_opening_and_a_closing() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let halves = |entry: u32, half: u8| -> Vec<String> {
        d.pets.names[&entry]
            .iter()
            .filter(|(_, h)| *h == half)
            .map(|(w, _)| w.clone())
            .collect()
    };
    for entry in [416, 417, 1860, 1863] {
        assert!(halves(entry, 0).len() > 20, "{entry}'s openings");
        assert!(halves(entry, 1).len() > 20, "{entry}'s closings");
    }
    let (starts, ends) = (halves(416, 0), halves(416, 1));
    let (_c, g, map) = player(&realm, "P7PNAME", "Namer", Race::Human, Class::Warlock).await;
    let name = on(&map, move |m| {
        ready(m, g, 10, &[]);
        crate::game::map::spells::cast_triggered(m, g, 688, Some(g));
        let pet = m.players[&g].owned.pet?;
        crate::game::map::pets::state(m, pet).map(|s| s.name.clone())
    })
    .await
    .expect("an imp, named");
    let joined = starts
        .iter()
        .any(|s| name.starts_with(s.as_str()) && ends.iter().any(|e| name[s.len()..] == **e));
    assert!(joined, "{name}");
    let mut chars = name.chars();
    assert!(
        chars.next().is_some_and(|c| c.is_ascii_uppercase()),
        "{name}"
    );
    assert!(chars.all(|c| c.is_ascii_lowercase()), "{name}");
}

/// Thistle Tea's pet training (`pet_training_dbc_test.exs`): a wolf
/// (family 1) comes with its family's passives (8875, 17223, the
/// 19580s, 20782, 20784), Great Stamina (4187) not among them, and Great
/// Stamina costs 5 training points.
#[test]
#[ignore = "extended"]
fn a_wolfs_passives_are_its_familys_and_great_stamina_is_trained() {
    let Some(d) = real() else {
        return;
    };
    let mut passives = crate::game::map::pets::family_passives(d, 1);
    passives.sort_unstable();
    passives.dedup();
    assert_eq!(
        passives,
        vec![
            8875, 17_223, 19_580, 19_581, 19_582, 19_589, 19_591, 20_782, 20_784
        ]
    );
    let stamina = d
        .pets
        .abilities
        .values()
        .flatten()
        .find(|a| a.spell == 4187)
        .unwrap();
    assert_eq!((stamina.cost, stamina.superseded_by), (5, 4188));
}

/// Thistle Tea's hidden passives (`passive_spell_vmangos_test.exs`) and
/// mangos's `spell_learn_spell`: Feline Swiftness's ranks (17002, 24866)
/// bring their hidden passives (24867, 24864) as they are learned; a
/// spell that brings another to the spellbook (Poisons, 2842, Instant
/// Poison 8681) brings it too.
#[tokio::test]
#[ignore = "extended"]
async fn feline_swiftness_brings_its_hidden_passive() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    assert_eq!(d.spells.learn_with[&17_002], vec![24_867]);
    assert_eq!(d.spells.learn_with[&24_866], vec![24_864]);
    let (_c, g, map) = player(&realm, "P7PFELINE", "Felina", Race::NightElf, Class::Druid).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[17_002, 24_866, 2842]);
        let p = &m.players[&g];
        [24_867, 24_864, 8681].map(|s| p.spells.contains(&s))
    })
    .await;
    assert_eq!(out, [true, true, true]);
}

/// Thistle Tea's controlled summons (`controlled_pet_dbc_test.exs`): the
/// creatures' pet summons (3612's 698, 3621's 756, 11939's 12922) stay
/// until dismissed, and the elementals (513's 329, 895's 575) a minute;
/// Summon Doomguard (22865, 14385) is a wild summon by vmangos's
/// `spell_effect_mod`, as wvs applies it.
#[test]
#[ignore = "extended"]
fn creature_pets_stay_and_elementals_last_a_minute() {
    let Some(d) = real() else {
        return;
    };
    for (id, entry, duration, effect) in [
        (3612, 698, -1, 28),
        (3621, 756, -1, 28),
        (11_939, 12_922, -1, 28),
        (22_865, 14_385, -1, 41),
        (513, 329, 60_000, 28),
        (895, 575, 60_000, 28),
    ] {
        let s = d.spells.get(id).unwrap();
        let e = &s.effects[0];
        assert_eq!((e.effect, e.misc_value), (effect, entry), "{id}");
        assert_eq!(
            d.spells
                .durations
                .get(&s.duration_index)
                .map_or(-1, |x| x.base),
            duration,
            "{id}"
        );
    }
}
