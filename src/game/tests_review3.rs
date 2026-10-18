//! The third code review's findings (combat, spells, auras, threat and
//! creature AI), each shown on made-up data in the fast suite.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{at, dummy, health, pass, ready};
use crate::game::data::ItemTemplate;
use crate::game::map::inventory::BANK_START;
use crate::game::map::spells;
use crate::game::store::BAG_PLAYER;

const REAGENT: u32 = 990_390;

fn reagent() -> ItemTemplate {
    ItemTemplate {
        entry: REAGENT,
        name: "Made-up Reagent".into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 20,
        buy_count: 1,
        ..Default::default()
    }
}

/// A cast's reagents are checked again as it goes off where it takes them
/// from, the bags: put in the bank meanwhile, they fail the cast, rather
/// than it going off for nothing.
#[tokio::test]
async fn reagents_banked_during_a_cast_fail_it() {
    const SPELL: u32 = 990_301;
    let realm = fast_realm(|d| {
        d.items.insert(REAGENT, reagent());
        let mut s = bare(SPELL, 2);
        s.casting_time_index = CAST_2S;
        s.reagents[0] = (REAGENT, 1);
        effect(&mut s, 0, damage(100));
        insert(d, [s]);
    });
    let (_c, mage, map) = human(&realm, "R3RG", "Reagents", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[SPELL]);
            let k = dummy(m, mage, 60, 3.0);
            crate::game::map::inventory::add_item(m, mage, REAGENT, 1, false).unwrap();
            // A GM pays no reagents.
            m.players.get_mut(&mage).unwrap().gm = false;
            let before = health(m, k);
            let began = spells::cast(m, mage, SPELL, at(k), None);
            // Into the bank while the cast goes on.
            let p = m.players.get_mut(&mage).unwrap();
            let i = p.items.iter().position(|i| i.entry == REAGENT).unwrap();
            p.items[i].bag = BAG_PLAYER;
            p.items[i].slot = BANK_START;
            pass(m, 2_100);
            let p = &m.players[&mage];
            (began, before - health(m, k), p.count_of(REAGENT))
        })
        .await
        .unwrap();
    assert!(out.0, "the cast began with the reagent in the bags");
    assert_eq!(out.1, 0, "it doesn't go off with the reagent in the bank");
    assert_eq!(out.2, 1, "and the reagent is still there");
}

fn threat_on(
    m: &crate::game::map::Map,
    creature: wow_world_messages::Guid,
    of: wow_world_messages::Guid,
) -> f32 {
    m.creature(creature)
        .and_then(|c| c.threat.iter().find(|(g, _)| *g == of).map(|(_, t)| *t))
        .unwrap_or(0.0)
}

/// A spell's damage makes threat by the caster's threat auras once, their
/// product, as a swing's does (Thistle Tea's `SpellThreat`, mangos's
/// `ThreatCalcHelper`): an all-school +30% makes 100 damage 130 threat,
/// and a holy-only +60% makes a holy spell's 160 and a fire spell's 100.
#[tokio::test]
async fn a_spells_threat_takes_the_threat_auras_once() {
    const ALL: u32 = 990_310;
    const HOLY_ONLY: u32 = 990_311;
    const FIRE: u32 = 990_312;
    const HOLY: u32 = 990_313;
    let realm = fast_realm(|d| {
        let mut all = bare(ALL, 0);
        all.duration_index = LASTS_LONG;
        effect(
            &mut all,
            0,
            aura(crate::game::map::auras::AURA_MOD_THREAT, 127, 30, 1),
        );
        let mut holy_only = bare(HOLY_ONLY, 0);
        holy_only.duration_index = LASTS_LONG;
        effect(
            &mut holy_only,
            0,
            aura(crate::game::map::auras::AURA_MOD_THREAT, 2, 60, 1),
        );
        let mut fire = bare(FIRE, 2);
        effect(&mut fire, 0, damage(100));
        let mut holy = bare(HOLY, 1);
        effect(&mut holy, 0, damage(100));
        insert(d, [all, holy_only, fire, holy]);
    });
    let (_c, paladin, map) = human(&realm, "R3TH", "Threatened", Class::Paladin).await;
    let out = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{apply_template, cast_fresh};
            ready(m, paladin, 60, &[FIRE, HOLY]);
            let template =
                |m: &crate::game::map::Map, id| m.realm.data.spells.get(id).unwrap().clone();
            let mut seen = Vec::new();
            for (aura, spell) in [(ALL, FIRE), (HOLY_ONLY, HOLY), (HOLY_ONLY, FIRE)] {
                let k = dummy(m, paladin, 60, 3.0);
                let t = template(m, aura);
                apply_template(m, paladin, paladin, &t);
                cast_fresh(m, paladin, spell, at(k));
                seen.push((1_000_000 - health(m, k), threat_on(m, k, paladin)));
                crate::game::map::auras::remove_spell(m, paladin, aura, None);
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(out[0].0, 100);
    assert!((out[0].1 - 130.0).abs() < 0.01, "all schools: {:?}", out[0]);
    assert!(
        (out[1].1 - 160.0).abs() < 0.01,
        "holy on holy: {:?}",
        out[1]
    );
    assert!(
        (out[2].1 - 100.0).abs() < 0.01,
        "holy on fire: {:?}",
        out[2]
    );
}

/// A cast interrupted is told its caster once (mangos's `Spell::cancel`
/// sends one `SMSG_CAST_RESULT`).
#[tokio::test]
async fn an_interrupted_cast_is_told_once() {
    const SPELL: u32 = 990_330;
    let realm = fast_realm(|d| {
        let mut s = bare(SPELL, 2);
        s.casting_time_index = CAST_2S;
        effect(&mut s, 0, damage(100));
        insert(d, [s]);
    });
    let (mut c, mage, map) = human(&realm, "R3IN", "Interrupted", Class::Mage).await;
    map.query(move |m| {
        ready(m, mage, 60, &[SPELL]);
        let k = dummy(m, mage, 60, 3.0);
        assert!(spells::cast(m, mage, SPELL, at(k), None));
        spells::interrupt(m, mage);
    })
    .await
    .unwrap();
    let interrupted = |b: &[u8]| {
        b.len() >= 6 && b[..4] == SPELL.to_le_bytes() && b[4] == 2 && b[5] == 0x23
    };
    c.wait_frame("the cast interrupted", 0x130, interrupted).await;
    let again = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        c.wait_frame("told again", 0x130, interrupted),
    )
    .await;
    assert!(again.is_err(), "told twice");
}
