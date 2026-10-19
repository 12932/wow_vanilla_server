//! Round 7's aura tests on the real data, ported from Thistle Tea's
//! `breakable_control_dbc_test.exs`, `engineering_dbc_test.exs`,
//! `form_links_dbc_test.exs`, `form_links_test.exs` and
//! `periodic_trigger_test.exs` (docs/parity/matrix.csv): the crowd
//! control a pet leaves be, the Net-o-Matic's nets, a druid's form boosts
//! and Leader of the Pack, and Azuregos's pulse.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::{Map, auras, combat, spells};

/// Thistle Tea's `breakable_crowd_control?` "protects Polymorph, Gouge,
/// Freezing Trap, Sap, and Repentance" and "permits attacks through Frost
/// Nova, Hammer of Justice, and Fear": the crowd control damage breaks
/// (a confuse, stun or transform that damage ends) a pet leaves be of its
/// own accord; a root, a stun damage doesn't end, and a fear it attacks
/// through.
#[tokio::test]
#[ignore = "extended"]
async fn a_pet_leaves_be_only_the_crowd_control_damage_breaks() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P7KR1", "Holdone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let held = |m: &mut Map, id: u32| {
                let kobold = dummy(m, mage, 60, 8.0);
                apply(m, mage, kobold, id);
                (
                    id,
                    has(m, kobold, id),
                    crate::game::map::pets::breakable_cc(m, kobold),
                )
            };
            let protected: Vec<_> = [118, 1776, 3355, 6770, 20066]
                .into_iter()
                .map(|id| held(m, id))
                .collect();
            let open: Vec<_> = [122, 853, 5782].into_iter().map(|id| held(m, id)).collect();
            (protected, open)
        })
        .await
        .unwrap();
    let (protected, open) = out;
    for (id, on, breakable) in protected {
        assert!(on && breakable, "{id} on and left be");
    }
    for (id, on, breakable) in open {
        assert!(on && !breakable, "{id} on and attacked through");
    }
}

const NET: u32 = 13099;
const LONG_BACKFIRE: u32 = 16566;
const BACKFIRE: u32 = 13119;
const MARKER: u32 = 13139;
const ON_THROWER: u32 = 13138;

/// The holder of `spell` on `g`: its full duration.
fn lasts(m: &Map, g: Guid, spell: u32) -> Option<Option<u32>> {
    holder(m, g, spell).map(|h| h.duration)
}

/// Thistle Tea's `Engineering` "normal nets root the target for ten
/// seconds and expire through ordinary aura cleanup", "the long backfire
/// roots only the caster for thirty seconds" and "the charging net
/// triggers one transient marker and roots its caster despite an enemy
/// selection": the Net-o-Matic's net holds its target ten seconds; its
/// long backfire holds the thrower alone thirty; its backfire holds the
/// target and the thrower twenty seconds each, no marker left; each runs
/// out at its end, and the thrower's goes with its death.
#[tokio::test]
#[ignore = "extended"]
async fn the_net_o_matics_nets_hold_whom_they_net_for_their_time() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, gnome, map) = player(&realm, "P7KR2", "Netone", Race::Gnome, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, gnome, 60, &[]);
            let rooted =
                |m: &Map, g: Guid| auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOD_ROOT));
            // The net.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, NET, Some(kobold));
            let net = (lasts(m, kobold, NET), rooted(m, kobold), rooted(m, gnome));
            pass(m, 9_999);
            let held = rooted(m, kobold);
            pass(m, 1);
            let net_after = (has(m, kobold, NET), rooted(m, kobold));
            // The long backfire.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, LONG_BACKFIRE, Some(kobold));
            let long = (
                lasts(m, gnome, LONG_BACKFIRE),
                rooted(m, gnome),
                rooted(m, kobold),
            );
            pass(m, 30_000);
            let long_after = rooted(m, gnome);
            // The backfire.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, BACKFIRE, Some(kobold));
            let backfire = (
                lasts(m, kobold, BACKFIRE),
                lasts(m, gnome, ON_THROWER),
                has(m, gnome, MARKER),
            );
            pass(m, 20_000);
            let backfire_after = (rooted(m, kobold), rooted(m, gnome));
            // Again, and the thrower dies netted.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, BACKFIRE, Some(kobold));
            combat::kill(m, kobold, gnome);
            let dead = has(m, gnome, ON_THROWER);
            (
                net,
                held,
                net_after,
                long,
                long_after,
                backfire,
                backfire_after,
                dead,
            )
        })
        .await
        .unwrap();
    let (net, held, net_after, long, long_after, backfire, backfire_after, dead) = out;
    assert_eq!(
        net,
        (Some(Some(10_000)), true, false),
        "the net: its target, ten seconds"
    );
    assert!(held, "held to its end");
    assert_eq!(net_after, (false, false), "and free at it");
    assert_eq!(
        long,
        (Some(Some(30_000)), true, false),
        "the thrower alone, thirty"
    );
    assert!(!long_after);
    assert_eq!(
        backfire,
        (Some(Some(20_000)), Some(Some(20_000)), false),
        "both twenty seconds, no marker"
    );
    assert_eq!(backfire_after, (false, false));
    assert!(!dead, "gone with the thrower's death");
}

const HEART_OF_THE_WILD: [(u32, u32); 5] = [
    (17003, 4),
    (17004, 8),
    (17005, 12),
    (17006, 16),
    (24894, 20),
];
const BEAR: u32 = 5487;
const DIRE_BEAR: u32 = 9634;
const CAT: u32 = 768;
const BEAR_BOOST: u32 = 24899;
const CAT_BOOST: u32 = 24900;

fn stat(m: &Map, g: Guid, i: u16) -> u32 {
    m.players[&g].fields.get_u32(UNIT_FIELD_STAT0 + i)
}

fn near(got: u32, want: f32) -> bool {
    (got as f32 - want).abs() <= 1.0
}

/// Thistle Tea's `FormLinks` "all Heart of the Wild ranks apply their
/// exact form-dependent percentage": each rank gives its share more
/// intellect (4, 8, 12, 16, 20), and as much more stamina in bear and dire
/// bear form, strength in cat, each with its form's boost, the bear's
/// health share kept; out of form, neither boost nor its stat.
#[tokio::test]
#[ignore = "extended"]
async fn every_rank_of_heart_of_the_wild_gives_its_share_in_each_form() {
    if !have_data() {
        return;
    }
    let realm = realm();
    for (n, (rank, share)) in HEART_OF_THE_WILD.into_iter().enumerate() {
        let (_c, dr, map) = player(
            &realm,
            &format!("P7KH{n}"),
            &format!("Heartone{}", ["a", "b", "c", "d", "e"][n]),
            Race::NightElf,
            Class::Druid,
        )
        .await;
        let out = map
            .query(move |m| {
                ready(m, dr, 60, &[]);
                let own = [0, 2, 3].map(|i| stat(m, dr, i));
                spells::learn(m, dr, rank);
                let intellect = stat(m, dr, 3);
                let mut forms = Vec::new();
                for (form, stat_index, boost) in [
                    (CAT, 0, CAT_BOOST),
                    (BEAR, 2, BEAR_BOOST),
                    (DIRE_BEAR, 2, BEAR_BOOST),
                ] {
                    let max = m.players[&dr].fields.get_u32(UNIT_FIELD_MAXHEALTH);
                    set_health(m, dr, max / 2);
                    spells::cast_triggered(m, dr, form, None);
                    let f = &m.players[&dr].fields;
                    let share = f.get_u32(UNIT_FIELD_HEALTH) as f32
                        / f.get_u32(UNIT_FIELD_MAXHEALTH).max(1) as f32;
                    let shifted = (stat(m, dr, stat_index), has(m, dr, boost), share);
                    auras::remove_spell(m, dr, form, None);
                    let normal = (
                        [0, 2, 3].map(|i| stat(m, dr, i)),
                        has(m, dr, BEAR_BOOST) || has(m, dr, CAT_BOOST),
                    );
                    forms.push((form, stat_index, shifted, normal));
                }
                (own, intellect, forms)
            })
            .await
            .unwrap();
        let (own, intellect, forms) = out;
        let grown = |base: u32| base as f32 * (100 + share) as f32 / 100.0;
        assert!(
            near(intellect, grown(own[2])),
            "{rank}: intellect {intellect} of {own:?}"
        );
        for (form, stat_index, (value, boosted, health), (normal, any)) in forms {
            let base = own[if stat_index == 0 { 0 } else { 1 }];
            assert!(
                near(value, grown(base)),
                "{rank} in {form}: {value} of {base}"
            );
            assert!(boosted, "{rank} in {form}: its boost");
            assert!(
                (health - 0.5).abs() < 0.01,
                "{rank} in {form}: health share {health}"
            );
            assert_eq!(
                [normal[0], normal[1]],
                [own[0], own[1]],
                "{rank} out of {form}"
            );
            assert!(near(normal[2], grown(own[2])));
            assert!(!any, "{rank} out of {form}: no boost");
        }
    }
}
