//! Phase 8's loader round, the one-target limits and helpful threat
//! (Thistle Tea's `world/loader/single_target_aura_dbc_test.exs` and
//! `spell_threat_dbc_test.exs`), on the real spells.

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::spells as sd;
use crate::game::map::{Map, MapHandle};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// Thistle Tea's single-target classes (`single_target_aura_dbc_test.exs`),
/// with vmangos's single-target flag on every one: one caster's Polymorph
/// of any model (118, rank 4, Turtle, Pig) on a second target ends the
/// first's, as one judgement does another (Justice, Light, Wisdom); a
/// spell's ranks share their limit (Hunter's Mark, Fear, Banish,
/// Entangling Roots, Hibernate), and different controls don't (Fear and
/// Banish, Roots and Hibernate, Polymorph and Hunter's Mark).
#[tokio::test]
#[ignore = "extended"]
async fn one_casters_single_target_auras_share_their_kinds_limit() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let mut pairs: Vec<(u32, u32, bool)> = Vec::new();
    for ids in [
        &[118, 12_826, 28_271, 28_272][..],
        &[20_184, 20_185, 20_186],
    ] {
        for a in ids {
            for b in ids {
                pairs.push((*a, *b, true));
            }
        }
    }
    for (a, b) in [
        (1130, 14_325),
        (5782, 6215),
        (710, 18_647),
        (339, 9853),
        (2637, 18_658),
    ] {
        pairs.push((a, b, true));
    }
    for (a, b) in [(5782, 710), (339, 2637), (118, 1130)] {
        pairs.push((a, b, false));
    }
    for (a, b, _) in &pairs {
        for id in [a, b] {
            assert!(
                d.spells
                    .get(*id)
                    .unwrap()
                    .custom(sd::CUSTOM_SINGLE_TARGET_AURA),
                "{id}"
            );
        }
    }
    let (_c, g, map) = player(&realm, "P8LONE", "Onetarget", Race::Human, Class::Mage).await;
    let asked = pairs.clone();
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        asked
            .iter()
            .map(|(a, b, _)| {
                let first = dummy(m, g, 60, 10.0);
                let second = dummy(m, g, 60, 12.0);
                apply(m, g, first, *a);
                let on_first = has(m, first, *a);
                apply(m, g, second, *b);
                (*a, *b, on_first && has(m, second, *b) && !has(m, first, *a))
            })
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(out, pairs);
}

/// Thistle Tea's "loads helpful-threat suppression independently from
/// initial threat": Rupture is marked to make no helpful threat
/// (`SPELL_ATTR_EX4_NO_HELPFUL_THREAT`, read where a helpful spell's
/// threat is had), Holy Light's second rank isn't; neither is marked to
/// make no threat as it lands.
#[test]
#[ignore = "extended"]
fn rupture_makes_no_helpful_threat_and_holy_light_does() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let rupture = d.spells.get(1943).unwrap();
    let light = d.spells.get(635).unwrap();
    assert!(rupture.attr(4, sd::ATTR_EX4_NO_HELPFUL_THREAT));
    assert!(!light.attr(4, sd::ATTR_EX4_NO_HELPFUL_THREAT));
    for s in [rupture, light] {
        assert!(!s.attr(2, sd::ATTR_EX2_NO_INITIAL_THREAT), "{}", s.id);
    }
}
