//! Phase 3, round 5, against Thistle Tea's `shaman_test.exs`: a weapon
//! enchantment's spell on a blow that lands, by its flat chance, or its
//! rate a minute over that, or the weapon's speed with neither; on the
//! wielder when it helps; the wielder's talents for its chance (Improved
//! Poisons), for its own family alone.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_combat::equip;
use super::tests_p3_spells_kit::{self as kit, has, health};
use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{ATTR_EX2_CANT_CRIT, ATTR_EX3_ALWAYS_HIT, Enchantment};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_ADD_FLAT_MODIFIER, AURA_MOD_STAT};
use crate::game::map::procs::{self, EX_MISS, EX_NORMAL_HIT, Event};

const SWORD: u32 = 991_000;
/// Enchantments: twenty percent flat; a hundred flat but six a minute;
/// neither; a helpful one, always; a rogue's poison of twenty; a shaman's
/// of twenty.
const FLAT: u32 = 9_101;
const RATED: u32 = 9_102;
const BARE: u32 = 9_103;
const HELPFUL: u32 = 9_104;
const POISON: u32 = 9_105;
const NOT_A_POISON: u32 = 9_106;
const STRIKE: u32 = 991_001;
const RATED_STRIKE: u32 = 991_002;
const BUFF: u32 = 991_003;
const POISON_STRIKE: u32 = 991_004;
const OTHER_STRIKE: u32 = 991_005;
const IMPROVED_POISONS: u32 = 991_006;

fn enchants(d: &mut GameData) {
    d.items.insert(
        SWORD,
        ItemTemplate {
            entry: SWORD,
            class: 2,
            subclass: 7,
            name: "Sword".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 2000,
            stackable: 1,
            ..Default::default()
        },
    );
    let strike = |id: u32, family: u32, flags: u64| {
        let mut s = bare(id, 2);
        s.family_name = family;
        s.family_flags = flags;
        s.attributes[2] |= ATTR_EX2_CANT_CRIT;
        s.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
        effect(&mut s, 0, damage(1));
        s
    };
    let mut buff = bare(BUFF, 0);
    buff.duration_index = LASTS_LONG;
    effect(&mut buff, 0, aura(AURA_MOD_STAT, 0, 1, 1));
    let mut talent = bare(IMPROVED_POISONS, 0);
    talent.family_name = 8;
    talent.duration_index = LASTS_LONG;
    effect(
        &mut talent,
        0,
        crate::game::data::spells::SpellEffect {
            item_type: 0x1000,
            ..aura(AURA_ADD_FLAT_MODIFIER, 18, 10, 1)
        },
    );
    insert(
        d,
        [
            strike(STRIKE, 0, 0),
            strike(RATED_STRIKE, 0, 0),
            strike(POISON_STRIKE, 8, 0x1000),
            strike(OTHER_STRIKE, 11, 0x1000),
            buff,
            talent,
        ],
    );
    for (id, amount, spell) in [
        (FLAT, 20, STRIKE),
        (RATED, 100, RATED_STRIKE),
        (BARE, 0, STRIKE),
        (HELPFUL, 100, BUFF),
        (POISON, 20, POISON_STRIKE),
        (NOT_A_POISON, 20, OTHER_STRIKE),
    ] {
        d.spells.enchantments.insert(
            id,
            Enchantment {
                id,
                effects: [(1, amount, 0, spell), (0, 0, 0, 0), (0, 0, 0, 0)],
                name: format!("Enchantment {id}"),
            },
        );
    }
    d.spells.enchant_ppm.insert(RATED_STRIKE, 6.0);
}

fn enchant(m: &mut Map, g: Guid, id: u32) {
    if let Some(p) = m.players.get_mut(&g)
        && let Some(i) = p.items.iter().position(|i| i.entry == SWORD)
    {
        p.items[i].fields.set_u32(ITEM_FIELD_ENCHANTMENT, id);
    }
}

/// `n` blows of `ex` from `g` at `target`: the strikes their enchantment
/// sent, by the damage done.
fn strikes(m: &mut Map, g: Guid, target: Guid, ex: u32, n: u32) -> u32 {
    let before = health(m, target);
    for _ in 0..n {
        procs::trigger(
            m,
            Event {
                actor: g,
                target,
                actor_flags: procs::PROC_DONE_MELEE,
                target_flags: procs::PROC_TAKEN_MELEE,
                ex,
                school: 1,
                damage: if ex == EX_MISS { 0 } else { 10 },
                ..Default::default()
            },
        );
    }
    before - health(m, target)
}

/// Its flat chance (a fifth); its rate a minute over a flat chance (six on
/// a two-second sword, a fifth); neither, the weapon's speed's (3.6%,
/// mangos's `GetWeaponProcChance`); nothing from a miss; a helpful one on
/// the wielder (Thistle Tea's `trigger_weapon_enchant/5` tests).
#[tokio::test]
async fn an_enchantments_spell_goes_by_its_chance_or_rate_on_a_blow_that_lands() {
    let realm = fast_realm(enchants);
    let (_a, g, map) = human(&realm, "P5NA", "Wielder", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            equip(m, g, SWORD, 15);
            let kobold = kit::dummy(m, g, 60, 2.0);
            enchant(m, g, FLAT);
            let flat = strikes(m, g, kobold, EX_NORMAL_HIT, 2_000);
            let missed = strikes(m, g, kobold, EX_MISS, 500);
            enchant(m, g, RATED);
            let rated = strikes(m, g, kobold, EX_NORMAL_HIT, 2_000);
            enchant(m, g, BARE);
            let bare = strikes(m, g, kobold, EX_NORMAL_HIT, 4_000);
            enchant(m, g, HELPFUL);
            strikes(m, g, kobold, EX_NORMAL_HIT, 1);
            let helped = (has(m, g, BUFF), has(m, kobold, BUFF));
            (flat, missed, rated, bare, helped)
        })
        .await
        .unwrap();
    let (flat, missed, rated, bare, helped) = out;
    assert!((300..=500).contains(&flat), "a fifth of 2000: {flat}");
    assert_eq!(missed, 0, "nothing from a miss");
    assert!(
        (300..=500).contains(&rated),
        "six a minute, not a hundred: {rated}"
    );
    assert!((90..=200).contains(&bare), "3.6% of 4000: {bare}");
    assert_eq!(helped, (true, false), "on the wielder");
}

/// A talent for a family's spells' chance raises its poison's (twenty and
/// ten), not another family's (Thistle Tea's poison chance talents,
/// mangos's `CastItemCombatSpell`).
#[tokio::test]
async fn a_poisons_chance_takes_the_talents_of_its_family() {
    let realm = fast_realm(enchants);
    let (_a, g, map) = human(&realm, "P5NB", "Poisoner", Class::Rogue).await;
    let out = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            equip(m, g, SWORD, 15);
            let kobold = kit::dummy(m, g, 60, 2.0);
            kit::apply(m, g, g, IMPROVED_POISONS);
            enchant(m, g, POISON);
            let poison = strikes(m, g, kobold, EX_NORMAL_HIT, 2_000);
            enchant(m, g, NOT_A_POISON);
            let other = strikes(m, g, kobold, EX_NORMAL_HIT, 2_000);
            auras::remove_spell(m, g, IMPROVED_POISONS, None);
            (poison, other)
        })
        .await
        .unwrap();
    let (poison, other) = out;
    assert!(
        (500..=700).contains(&poison),
        "three tenths of 2000: {poison}"
    );
    assert!((300..=500).contains(&other), "a fifth of 2000: {other}");
}
