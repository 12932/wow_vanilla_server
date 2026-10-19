//! Phase 3's exclusive aura categories and the spell book, as Thistle
//! Tea's `consumable_test`, `slow_test`, `scripts_test` and
//! `spell_book_test` have them, on made-up spells and the fast suite.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use crate::game::data::spells::{SkillAbility, SpellEffect, SpellTemplate};
use crate::game::map::auras::{Exclusive, INTERRUPT_NOT_SEATED, exclusive};
use crate::game::map::spells;

/// A spell of no family whose effects put on `auras`, eaten seated.
fn seated(auras: &[(usize, u32)]) -> SpellTemplate {
    let mut s = bare(990_700, 0);
    s.aura_interrupt_flags = INTERRUPT_NOT_SEATED;
    for (i, a) in auras {
        s.effects[*i] = SpellEffect {
            effect: 6,
            aura: *a,
            ..Default::default()
        };
    }
    s
}

/// Food, drink and both, by their auras in any slot, flat or percent;
/// the eating aura apart from its lasting buff (Well Fed's attribute,
/// without the seated interrupt); not ordinary regeneration, nor another
/// family's; the priest family's fruit buffs; and the elixirs' masks, the
/// flask's two bits (Thistle Tea's `Consumable.category`).
#[test]
fn food_drink_and_well_fed_are_told_by_their_auras_and_flags() {
    let cat = |s: &SpellTemplate| exclusive(s, None);
    assert_eq!(cat(&seated(&[(2, 84)])), Some(Exclusive::Food));
    assert_eq!(cat(&seated(&[(0, 20)])), Some(Exclusive::Food));
    assert_eq!(cat(&seated(&[(1, 85)])), Some(Exclusive::Drink));
    assert_eq!(cat(&seated(&[(2, 21)])), Some(Exclusive::Drink));
    assert_eq!(
        cat(&seated(&[(0, 20), (1, 21)])),
        Some(Exclusive::FoodAndDrink)
    );
    let mut fed = seated(&[(0, 84)]);
    fed.attributes[2] = 0x8000_0000;
    assert_eq!(cat(&fed), Some(Exclusive::Food), "the eating aura");
    fed.aura_interrupt_flags = 0;
    assert_eq!(cat(&fed), Some(Exclusive::WellFed), "its lasting buff");
    let mut plain = seated(&[(0, 84)]);
    plain.aura_interrupt_flags = 0;
    assert_eq!(cat(&plain), None, "regeneration on its own");
    let mut other = seated(&[(0, 84)]);
    other.family_name = 13;
    assert_eq!(cat(&other), None, "another family's");
    assert_eq!(cat(&seated(&[(0, 29)])), None, "another aura");
    let mut fruit = bare(990_701, 0);
    fruit.family_name = 6;
    fruit.spell_icon = 52;
    fruit.attributes[0] = 0x0800_0000;
    fruit.interrupt_flags = 8;
    assert_eq!(cat(&fruit), Some(Exclusive::WellFed));
    fruit.spell_icon = 79;
    assert_eq!(cat(&fruit), Some(Exclusive::WellFed));
    let mut no_interrupt = fruit.clone();
    no_interrupt.interrupt_flags = 0;
    assert_eq!(cat(&no_interrupt), None);
    fruit.attributes[0] = 0;
    assert_eq!(cat(&fruit), None);
    let potion = bare(990_702, 0);
    assert_eq!(exclusive(&potion, Some(3)), Some(Exclusive::Flask));
    assert_eq!(exclusive(&potion, Some(16)), Some(Exclusive::WellFed));
    assert_eq!(exclusive(&potion, Some(19)), Some(Exclusive::Flask));
    assert_eq!(exclusive(&potion, Some(0)), None);
    assert_eq!(exclusive(&potion, Some(1)), Some(Exclusive::BattleElixir));
    assert_eq!(exclusive(&potion, Some(2)), Some(Exclusive::GuardianElixir));
}

/// Slows aren't exclusive, as Thistle Tea's `Slow.category` makes them: a
/// negative melee haste, alone or beside another aura, stacks, as
/// mangos-classic multiplies them (the user's #30); snares stack too, as
/// mangos-classic has them (#27); nor are a daze or a potion family's.
#[test]
fn slows_arent_exclusive_snares_and_attack_speed_slows_stack() {
    use crate::game::map::auras::{AURA_MOD_DECREASE_SPEED, AURA_MOD_HASTE};
    let with = |e: &[(u32, u32, i32)]| {
        let mut s = bare(990_710, 0);
        for (i, (effect, a, points)) in e.iter().enumerate() {
            s.effects[i] = SpellEffect {
                effect: *effect,
                aura: *a,
                base_points: *points,
                ..Default::default()
            };
        }
        s
    };
    assert_eq!(
        exclusive(&with(&[(6, AURA_MOD_HASTE, -11)]), None),
        None,
        "an attack-speed slow stacks"
    );
    assert_eq!(exclusive(&with(&[(6, AURA_MOD_HASTE, 20)]), None), None);
    assert_eq!(
        exclusive(&with(&[(6, AURA_MOD_HASTE, -11), (6, 65, 0)]), None),
        None
    );
    assert_eq!(
        exclusive(
            &with(&[(6, AURA_MOD_DECREASE_SPEED, -50), (2, 0, 10)]),
            None
        ),
        None,
        "a snare stacks"
    );
    let mut daze = with(&[(6, AURA_MOD_DECREASE_SPEED, -50)]);
    daze.attributes[1] = 0x4_0000;
    assert_eq!(exclusive(&daze, None), None);
    let mut potion = with(&[(6, AURA_MOD_HASTE, -11)]);
    potion.family_name = 13;
    assert_eq!(exclusive(&potion, None), None, "a potion family's");
}

/// The class categories: the mage's armors by their family flags, the
/// warlock's by visual and icon, Dampen and Amplify Magic the mage's
/// alone; anything else none (Thistle Tea's `exclusive_category`).
#[test]
fn the_class_armors_and_magics_are_told_by_their_family() {
    let of = |family: u32, flags: u64| {
        let mut s = bare(990_720, 0);
        s.family_name = family;
        s.family_flags = flags;
        s
    };
    assert_eq!(
        exclusive(&of(3, 0x0200_0000), None),
        Some(Exclusive::MageArmor)
    );
    let mut warlock = of(5, 0);
    warlock.spell_visual = 130;
    warlock.spell_icon = 89;
    assert_eq!(exclusive(&warlock, None), Some(Exclusive::WarlockArmor));
    for mask in [0x2000, 0x8000_2000] {
        assert_eq!(exclusive(&of(3, mask), None), Some(Exclusive::MageMagic));
        assert_eq!(exclusive(&of(6, mask), None), None);
    }
    assert_eq!(exclusive(&bare(990_721, 0), None), None);
    assert_eq!(exclusive(&of(3, 1), None), None);
}

const RANK_1: u32 = 990_730;
const RANK_2: u32 = 990_731;
const RANK_3: u32 = 990_732;
const OTHER: u32 = 990_733;

/// Heroic Strike's kind: three ranks, each superseded by the next on its
/// skill line, and a spell of its own beside them.
fn ranks(d: &mut crate::game::data::GameData) {
    insert(d, [RANK_1, RANK_2, RANK_3, OTHER].map(|id| bare(id, 0)));
    for (spell, next) in [(RANK_1, RANK_2), (RANK_2, RANK_3)] {
        d.spells.skill_abilities.insert(
            spell,
            vec![SkillAbility {
                spell,
                superseded_by: next,
                ..Default::default()
            }],
        );
    }
}

/// What `c` heard of the spell book: learned spells and supersessions.
async fn heard(c: &mut crate::game::harness::Client) -> (Vec<u32>, Vec<(u16, u16)>) {
    let mut learned = Vec::new();
    while let Some(id) = c
        .gets(Duration::from_millis(50), |m| match m {
            ServerOpcodeMessage::SMSG_LEARNED_SPELL(l) if l.id > 990_000 => Some(l.id),
            _ => None,
        })
        .await
    {
        learned.push(id);
    }
    let mut superseded = Vec::new();
    loop {
        let Ok(b) = tokio::time::timeout(
            Duration::from_millis(50),
            c.wait_frame(
                "SMSG_SUPERCEDED_SPELL",
                spells::log::SMSG_SUPERCEDED_SPELL,
                |_| true,
            ),
        )
        .await
        else {
            break;
        };
        superseded.push((
            u16::from_le_bytes([b[0], b[1]]),
            u16::from_le_bytes([b[2], b[3]]),
        ));
    }
    (learned, superseded)
}

/// The spell book: a spell known isn't learned again; a rank supersedes
/// the rank just below it alone, the lower kept known; learned in order,
/// each rank supersedes the one before; a spell of its own sits beside
/// the others (Thistle Tea's `SpellBook.learn`, mangos's `addSpell`).
#[tokio::test]
async fn a_rank_supersedes_the_one_below_it_and_a_known_spell_isnt_learned_again() {
    let realm = fast_realm(ranks);
    let (mut c, g, map) = human(&realm, "P3SB", "Booker", Class::Warrior).await;
    c.clear();
    let known = |ids: &'static [u32]| {
        let ids = ids.to_vec();
        move |m: &mut crate::game::map::Map| {
            for id in ids {
                spells::learn(m, g, id);
            }
            m.players[&g]
                .spells
                .iter()
                .copied()
                .filter(|s| *s > 990_000)
                .collect::<Vec<_>>()
        }
    };
    let first = map.query(known(&[RANK_1, RANK_1])).await.unwrap();
    assert_eq!(first, [RANK_1]);
    assert_eq!(heard(&mut c).await, (vec![RANK_1], vec![]), "once");
    let skip = map.query(known(&[RANK_3])).await.unwrap();
    assert_eq!(skip, [RANK_1, RANK_3]);
    assert_eq!(
        heard(&mut c).await,
        (vec![RANK_3], vec![]),
        "the third rank doesn't supersede the first"
    );
    let realm = fast_realm(ranks);
    let (mut c, g, map) = human(&realm, "P3SC", "Bookertwo", Class::Warrior).await;
    c.clear();
    let known = move |m: &mut crate::game::map::Map| {
        for id in [RANK_1, RANK_2, RANK_3, OTHER] {
            spells::learn(m, g, id);
        }
        m.players[&g]
            .spells
            .iter()
            .copied()
            .filter(|s| *s > 990_000)
            .collect::<Vec<_>>()
    };
    let all = map.query(known).await.unwrap();
    assert_eq!(all, [RANK_1, RANK_2, RANK_3, OTHER], "the lower ranks kept");
    let (learned, superseded) = heard(&mut c).await;
    assert_eq!(learned, [RANK_1, RANK_2, RANK_3, OTHER]);
    assert_eq!(
        superseded,
        [
            (RANK_1 as u16, RANK_2 as u16),
            (RANK_2 as u16, RANK_3 as u16)
        ],
        "each by the next, and the spell of its own by none"
    );
}
