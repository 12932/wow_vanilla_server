//! Round 8's combat tests, defense on the attack table (Thistle Tea's
//! `defensive_capabilities_test.exs`): a defender's skill counted once at
//! each of the table's bands, a creature's avoidance with its auras and
//! its block capped, and a point of defense won shown at once.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::auras::{
    AURA_MOD_BLOCK_PERCENT, AURA_MOD_DODGE_PERCENT, AURA_MOD_PARRY_PERCENT, AURA_MOD_SKILL_TALENT,
};
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::derive::{SKILL_DEFENSE, SPELL_BLOCK, SPELL_PARRY};
use crate::game::map::{Map, MapHandle, skills, spells};
use crate::game::tests_p3_combat::{self as p3, BRUTE, OFF_HAND, SHIELD, SWORD};

type Setup = (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    MapHandle,
);

/// Alice at 60, a sword and a shield, Parry and Block known, her defense
/// at 300 and five percent of dodge from an aura (a warrior of no agility
/// dodges none of her own).
async fn defender() -> Setup {
    let (realm, a, g, map) = p3::setup_with(p3::data(), Class::Warrior).await;
    map.query(move |m| {
        p3::level(m, g, 60);
        for s in [SPELL_PARRY, SPELL_BLOCK] {
            spells::learn(m, g, s);
        }
        p3::equip(m, g, SWORD, 15);
        p3::equip(m, g, SHIELD, OFF_HAND);
        p3::aura_of(m, g, 96_700, AURA_MOD_DODGE_PERCENT, 0, 5);
        // Weapons drawn: sheathed, she wouldn't block.
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_byte(UNIT_FIELD_BYTES_2, 0, 1);
        let p = m.players.get_mut(&g).unwrap();
        p.skills_maxed = false;
        skills::learn_skill(p, SKILL_DEFENSE, 300);
        skills::set_value(p, SKILL_DEFENSE, 300);
        restat(m, g);
    })
    .await
    .unwrap();
    (realm, a, g, map)
}

fn restat(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().apply_stats(&data);
}

fn defense(m: &mut Map, g: Guid, to: u32) {
    let p = m.players.get_mut(&g).unwrap();
    skills::set_value(p, SKILL_DEFENSE, to);
    restat(m, g);
}

/// Dodge, parry and block, as her character sheet shows them.
fn sheet(m: &Map, g: Guid) -> [f32; 3] {
    let f = &m.players[&g].fields;
    [
        PLAYER_DODGE_PERCENTAGE,
        PLAYER_PARRY_PERCENTAGE,
        PLAYER_BLOCK_PERCENTAGE,
    ]
    .map(|i| f.get_f32(i))
}

/// The white table's outcome at each band's edges, for a band of `bp`
/// each of miss, dodge, parry and block.
fn edges(r: &combat::Roll, bp: i32) -> Vec<Outcome> {
    [
        bp - 1,
        bp,
        2 * bp - 1,
        2 * bp,
        3 * bp - 1,
        3 * bp,
        4 * bp - 1,
    ]
    .into_iter()
    .map(|roll| combat::roll_outcome(r, roll))
    .collect()
}

/// Thistle Tea's "counts defense skill once at each attack-table
/// boundary", as mangos's `RollMeleeOutcomeAgainst` has it: a creature of
/// 300 weapon skill on a defender of 305 misses 5.2% and is dodged,
/// parried and blocked 5.2% each, her character sheet's chances, the five
/// points not counted again; at 295, 4.8% each; and a creature of 310 on
/// her 305, 4.8% each again. An ability's table blocks in the same band.
#[tokio::test]
async fn a_defenders_skill_counts_once_at_each_band_of_the_table() {
    let (_realm, _a, g, map) = defender().await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            for (defended, level, bp) in [(305, 60, 520), (295, 60, 480), (305, 62, 480)] {
                // Past her cap of 300 by a talent's five.
                crate::game::map::auras::remove_spell(m, g, 96_704, None);
                if defended > 300 {
                    p3::aura_of(m, g, 96_704, AURA_MOD_SKILL_TALENT, SKILL_DEFENSE as i32, 5);
                }
                defense(m, g, defended.min(300));
                let brute = p3::spawn(m, g, BRUTE, level, 3.0);
                let r = combat::roll_for(m, brute, g, Hand::Main).expect("a roll");
                let rest = combat::roll_outcome(&r, 4 * bp);
                let special = combat::roll_special(&r, 3 * bp, 9_999);
                out.push((bp, sheet(m, g), edges(&r, bp), rest, special));
            }
            out
        })
        .await
        .unwrap();
    use Outcome::*;
    for (bp, sheet, edges, rest, special) in r {
        assert_eq!(
            edges,
            [Miss, Dodge, Dodge, Parry, Parry, Block, Block],
            "bands of {bp}"
        );
        assert!(matches!(rest, Hit | Crit), "{bp}: past them, {rest:?}");
        assert_eq!(special, Block, "{bp}: an ability's block");
        // Against a creature of her level, her sheet's own chances.
        if bp == 520 {
            assert!(sheet.iter().all(|c| (c - 5.2).abs() < 1e-4), "{sheet:?}");
        }
    }
}

/// Thistle Tea's "creature avoidance includes aura modifiers and the block
/// cap": a creature of two more dodge, three more parry and fifty more
/// block, struck by one of its level, dodges 7%, parries 8% and blocks
/// only 5%.
#[tokio::test]
async fn a_creatures_avoidance_takes_its_auras_and_its_block_is_capped() {
    let (_realm, _a, g, map) = p3::setup_with(p3::data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, g, 60);
            let brute = p3::spawn(m, g, BRUTE, 60, 3.0);
            // Before it: from behind, it would neither parry nor block.
            let other = p3::spawn(m, g, BRUTE, 60, 2.0);
            for (id, kind, amount) in [
                (96_701, AURA_MOD_DODGE_PERCENT, 2),
                (96_702, AURA_MOD_PARRY_PERCENT, 3),
                (96_703, AURA_MOD_BLOCK_PERCENT, 50),
            ] {
                p3::aura_of(m, brute, id, kind, 0, amount);
            }
            let r = combat::roll_for(m, other, brute, Hand::Main).expect("a roll");
            [1_199, 1_200, 1_999, 2_000, 2_499, 2_500].map(|roll| combat::roll_outcome(&r, roll))
        })
        .await
        .unwrap();
    use Outcome::*;
    assert_eq!(&r[..5], &[Dodge, Parry, Parry, Block, Block]);
    assert!(
        matches!(r[5], Hit | Crit),
        "past the capped block: {:?}",
        r[5]
    );
}

/// Thistle Tea's "defense skill gains immediately refresh displayed
/// avoidance": a blow taken that teaches her a point of defense, 295 to
/// 296, shows her four hundredths of a percent more dodge, parry and block
/// at once.
#[tokio::test]
async fn a_point_of_defense_won_shows_on_her_sheet_at_once() {
    let (_realm, _a, g, map) = defender().await;
    let r = map
        .query(move |m| {
            defense(m, g, 295);
            let before = sheet(m, g);
            let brute = p3::spawn(m, g, BRUTE, 60, 3.0);
            let mut blows = 0;
            while skills::value(&m.players[&g], SKILL_DEFENSE) == Some(295) && blows < 5_000 {
                p3::forced_swing(m, brute, g, Outcome::Hit);
                m.players
                    .get_mut(&g)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_HEALTH, 1_000);
                blows += 1;
            }
            (
                skills::value(&m.players[&g], SKILL_DEFENSE),
                before,
                sheet(m, g),
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(296), "a point won");
    for i in 0..3 {
        assert!(
            (r.2[i] - r.1[i] - 0.04).abs() < 1e-4,
            "{i}: {} to {}",
            r.1[i],
            r.2[i]
        );
    }
}
