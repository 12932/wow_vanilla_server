//! Round 12f on the real world: Frost Nova breaking on damage, Winter's
//! Chill's five stacks, Dominate Mind dispelled off a friend, and Shadow
//! Flame against the Onyxia Scale Cloak, as vmangos's data has them.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::{bare, damage, effect, insert};
use super::tests_p3_spells_kit::{dummy, have_data, player, ready, realm, realm_plus};
use crate::game::fields::*;
use crate::game::map::{Map, auras, spells};

const FROST_NOVA: u32 = 122;
const WINTERS_CHILL: u32 = 12579;
const DOMINATE_MIND: u32 = 20604;
const DISPEL_MAGIC: u32 = 527;
const SHADOW_FLAME: u32 = 22539;
const SHADOW_FLAME_BURN: u32 = 22682;
const ONYXIA_SCALE_CLOAK: u32 = 22683;
const BOLT: u32 = 995_190;

fn on(m: &Map, g: wow_world_messages::Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Frost Nova's root breaks on a blow by the fear's chance: 100 damage on a
/// level-10 kobold is sure.
#[tokio::test]
#[ignore = "extended"]
async fn frost_nova_breaks_on_a_blow() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut bolt = bare(BOLT, 2);
        effect(&mut bolt, 0, damage(100));
        insert(d, [bolt]);
    });
    let (_c, mage, map) = player(&realm, "P12FRNOVA", "Novaer", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 10, 5.0);
            spells::cast_triggered(m, mage, FROST_NOVA, None);
            let before = on(m, k, FROST_NOVA);
            spells::cast_triggered(m, mage, BOLT, Some(k));
            (before, on(m, k, FROST_NOVA))
        })
        .await
        .unwrap();
    assert_eq!(got, (true, false));
}

/// Winter's Chill's five stacks are 10% more crit for frost at its bearer,
/// and none for fire.
#[tokio::test]
#[ignore = "extended"]
async fn winters_chill_five_stacks_are_ten_percent_for_frost() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P12FRCHILL", "Chillier", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let k = dummy(m, mage, 60, 5.0);
            for _ in 0..6 {
                spells::cast_triggered(m, mage, WINTERS_CHILL, Some(k));
            }
            let a = auras::auras(m, k).unwrap();
            (
                a.sum_mask(auras::AURA_MOD_ATTACKER_SPELL_CRIT_CHANCE, 1 << 4),
                a.sum_mask(auras::AURA_MOD_ATTACKER_SPELL_CRIT_CHANCE, 1 << 2),
            )
        })
        .await
        .unwrap();
    assert_eq!(got, (10, 0));
}

/// A priest's Dispel Magic takes Lucifron's Dominate Mind off a friend it
/// made an enemy.
#[tokio::test]
#[ignore = "extended"]
async fn dispel_magic_takes_dominate_mind_off_a_friend() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_a, priest, map) =
        player(&realm, "P12FRDISA", "Dispelly", Race::Human, Class::Priest).await;
    let (_b, friend, _) = player(&realm, "P12FRDISB", "Dominee", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, priest, 60, &[]);
            ready(m, friend, 60, &[]);
            let k = dummy(m, friend, 60, 5.0);
            let s = m.realm.data.spells.get(DOMINATE_MIND).cloned().unwrap();
            auras::apply_spell(m, k, friend, &s, &[(0, 0)], auras::Context::default()).ok();
            let before = on(m, friend, DOMINATE_MIND);
            spells::cast_triggered(m, priest, DISPEL_MAGIC, Some(friend));
            (before, on(m, friend, DOMINATE_MIND))
        })
        .await
        .unwrap();
    assert_eq!(got, (true, false));
}

/// Shadow Flame burns the bare and not the cloaked.
#[tokio::test]
#[ignore = "extended"]
async fn shadow_flame_burns_only_those_without_the_cloak() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_a, bare_one, map) =
        player(&realm, "P12FRFLA", "Unclady", Race::Human, Class::Warrior).await;
    let (_b, cloaked, _) = player(&realm, "P12FRFLB", "Cloakey", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, bare_one, 60, &[]);
            ready(m, cloaked, 60, &[]);
            spells::cast_triggered(m, cloaked, ONYXIA_SCALE_CLOAK, Some(cloaked));
            let k = dummy(m, bare_one, 63, 5.0);
            // Hardy enough to live through the flame.
            for g in [bare_one, cloaked] {
                let f = &mut m.players.get_mut(&g).unwrap().fields;
                f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            }
            let burnt = |m: &mut Map, g| {
                spells::cast_triggered(m, k, SHADOW_FLAME, Some(g));
                on(m, g, SHADOW_FLAME_BURN)
            };
            (
                on(m, cloaked, ONYXIA_SCALE_CLOAK),
                burnt(m, bare_one),
                burnt(m, cloaked),
            )
        })
        .await
        .unwrap();
    assert!(got.0, "the cloak's aura on");
    assert!(got.1, "the bare burnt");
    assert!(!got.2, "the cloaked not");
}
