//! Round 12f: the tier sets' and relics' procs mangos-classic does in code
//! (`set_procs`), on the real spells: each set's aura on its wearer, its
//! chance made sure, and the spell it answers cast.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::{aura, bare, effect, insert};
use super::tests_p3_spells_kit::{dummy, have_data, player, ready, realm_plus};
use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::map::set_procs::*;
use crate::game::map::{Map, auras, procs, spells};

/// A sure crit for every spell.
const SURE_CRIT: u32 = 995_200;
const FLASH_HEAL: u32 = 2061;
const GREATER_HEAL: u32 = 2060;
const HEALING_TOUCH: u32 = 5185;
const LESSER_HEALING_WAVE: u32 = 8004;
const LIGHTNING_BOLT: u32 = 403;
const HEALING_WAVE: u32 = 331;
const VANISH: u32 = 1856;

/// The real world, with every set aura's chance sure and a sure crit.
fn sure(d: &mut GameData) {
    for id in [ORACLE, TEN_STORMS, MANA_SURGE, TEN_STORMS_SHIELD] {
        if let Some(s) = d.spells.spells.get_mut(&id) {
            s.proc_chance = 100;
        }
    }
    let mut crit = bare(SURE_CRIT, 0);
    crit.duration_index = super::tests_p3_cast_kit::LASTS_LONG;
    effect(
        &mut crit,
        0,
        aura(auras::AURA_MOD_SPELL_CRIT_CHANCE, -1, 100, 1),
    );
    insert(d, [crit]);
}

fn real() -> Option<Arc<Realm>> {
    have_data().then(|| realm_plus(sure))
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH)
}

fn mana(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_POWER1)
}

fn set_health(m: &mut Map, g: Guid, h: u32) {
    m.unit_fields_mut(g).unwrap().set_u32(UNIT_FIELD_HEALTH, h);
}

fn wear(m: &mut Map, g: Guid, spells_: &[u32]) {
    for s in spells_ {
        spells::cast_triggered(m, g, *s, Some(g));
        assert!(has(m, g, *s), "{s} on");
    }
}

/// Garments of the Oracle: a heal heals the priest a tenth of it;
/// Vestments of Faith: a Greater Heal's crit gives its target Armor of
/// Faith.
#[tokio::test]
#[ignore = "extended"]
async fn the_priests_sets_heal_the_priest_and_armor_the_healed() {
    let Some(realm) = real() else { return };
    let (_a, priest, map) = player(&realm, "P12FSPRA", "Oracle", Race::Human, Class::Priest).await;
    let (_b, friend, _) = player(&realm, "P12FSPRB", "Faithful", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, priest, 60, &[]);
            ready(m, friend, 60, &[]);
            wear(m, priest, &[ORACLE]);
            set_health(m, priest, 100);
            set_health(m, friend, 100);
            spells::cast_triggered(m, priest, FLASH_HEAL, Some(friend));
            let healed = health(m, friend) - 100;
            let own = health(m, priest) - 100;
            auras::remove_spell(m, priest, ORACLE, None);
            wear(m, priest, &[VESTMENTS_OF_FAITH, SURE_CRIT]);
            spells::cast_triggered(m, priest, FLASH_HEAL, Some(friend));
            let armored_by_flash = has(m, friend, ARMOR_OF_FAITH);
            spells::cast_triggered(m, priest, GREATER_HEAL, Some(friend));
            (
                healed,
                own,
                armored_by_flash,
                has(m, friend, ARMOR_OF_FAITH),
            )
        })
        .await
        .unwrap();
    let (healed, own, by_flash, by_greater) = got;
    assert!(healed > 0, "healed");
    assert!(
        own >= healed / 10 - 1 && own <= healed / 10 + 1,
        "a tenth back: {own} of {healed}"
    );
    assert!(!by_flash, "not for a Flash Heal");
    assert!(by_greater, "Armor of Faith for a Greater Heal's crit");
}

/// Dreamwalker Raiment: a Healing Touch's crit gives back 30% of its cost;
/// the Idol of Longevity gives its 25 more.
#[tokio::test]
#[ignore = "extended"]
async fn the_druids_set_and_idol_give_mana_back_for_healing_touch() {
    let Some(realm) = real() else { return };
    let (_a, druid, map) =
        player(&realm, "P12FSDRA", "Dreamer", Race::NightElf, Class::Druid).await;
    let got = map
        .query(move |m| {
            ready(m, druid, 60, &[]);
            let cost = m.realm.data.spells.get(HEALING_TOUCH).unwrap().mana_cost;
            m.unit_fields_mut(druid)
                .unwrap()
                .set_u32(UNIT_FIELD_POWER1, 1000);
            wear(m, druid, &[DREAMWALKER, SURE_CRIT]);
            spells::cast_triggered(m, druid, HEALING_TOUCH, Some(druid));
            let set = mana(m, druid) as i64 - 1000;
            auras::remove_spell(m, druid, DREAMWALKER, None);
            wear(m, druid, &[IDOL_OF_LONGEVITY]);
            let before = mana(m, druid);
            spells::cast_triggered(m, druid, HEALING_TOUCH, Some(druid));
            (cost, set, mana(m, druid) as i64 - before as i64)
        })
        .await
        .unwrap();
    let (cost, set, idol) = got;
    assert_eq!(set, i64::from(cost * 30 / 100), "30% of {cost}");
    assert_eq!(idol, 25, "the idol's 25");
}

/// The Totem of Flowing Water gives mana back for Lesser Healing Wave; the
/// Earthfury's Mana Surge gives back 35% of a Healing Wave's cost (its
/// `spell_proc_event` names Healing Wave and Lesser Healing Wave, not
/// Lightning Bolt); The Ten Storms puts Lightning Shield on
/// the one they heal, whose charge strikes their attacker.
#[tokio::test]
#[ignore = "extended"]
async fn the_shamans_sets_and_totem_answer_their_spells() {
    let Some(realm) = real() else { return };
    let (_a, shaman, map) = player(&realm, "P12FSSHA", "Stormy", Race::Orc, Class::Shaman).await;
    let (_b, friend, _) = player(&realm, "P12FSSHB", "Tanky", Race::Orc, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, shaman, 60, &[]);
            m.unit_fields_mut(shaman)
                .unwrap()
                .set_u32(UNIT_FIELD_POWER1, 1000);
            wear(m, shaman, &[TOTEM_OF_FLOWING_WATER]);
            spells::cast_triggered(m, shaman, LESSER_HEALING_WAVE, Some(shaman));
            let totem = mana(m, shaman) as i64 - 1000;
            auras::remove_spell(m, shaman, TOTEM_OF_FLOWING_WATER, None);
            let k = dummy(m, shaman, 60, 5.0);
            let cost = m.realm.data.spells.get(HEALING_WAVE).unwrap().mana_cost;
            wear(m, shaman, &[MANA_SURGE]);
            let before = mana(m, shaman);
            spells::cast_triggered(m, shaman, LIGHTNING_BOLT, Some(k));
            let by_bolt = mana(m, shaman) as i64 - before as i64;
            let before = mana(m, shaman);
            spells::cast_triggered(m, shaman, HEALING_WAVE, Some(shaman));
            let surge = mana(m, shaman) as i64 - before as i64;
            auras::remove_spell(m, shaman, MANA_SURGE, None);
            wear(m, shaman, &[TEN_STORMS]);
            spells::cast_triggered(m, shaman, HEALING_WAVE, Some(friend));
            let shielded = (
                has(m, friend, TEN_STORMS_SHIELD),
                has(m, shaman, TEN_STORMS_SHIELD),
            );
            // The kobold's blow on the shaman.
            let kobold = health(m, k);
            procs::trigger(
                m,
                procs::Event {
                    actor: k,
                    target: friend,
                    actor_flags: procs::PROC_DONE_MELEE,
                    target_flags: procs::PROC_TAKEN_MELEE,
                    ex: procs::EX_NORMAL_HIT,
                    damage: 10,
                    school: 1,
                    ..Default::default()
                },
            );
            (totem, cost, by_bolt, surge, shielded, kobold - health(m, k))
        })
        .await
        .unwrap();
    let (totem, cost, by_bolt, surge, shielded, struck) = got;
    assert_eq!(totem, 10, "the totem's 10");
    assert_eq!(by_bolt, 0, "nothing for a Lightning Bolt");
    assert_eq!(surge, i64::from(cost * 35 / 100), "35% of {cost}");
    assert_eq!(
        shielded,
        (true, false),
        "the Ten Storms' shield on the healed"
    );
    assert!(struck > 0, "its charge strikes the attacker");
}

/// Nightslayer's Clean Escape heals the rogue as they vanish.
#[tokio::test]
#[ignore = "extended"]
async fn nightslayers_clean_escape_heals_on_vanish() {
    let Some(realm) = real() else { return };
    let (_a, rogue, map) = player(&realm, "P12FSROA", "Escapee", Race::Human, Class::Rogue).await;
    let got = map
        .query(move |m| {
            ready(m, rogue, 60, &[]);
            wear(m, rogue, &[CLEAN_ESCAPE]);
            set_health(m, rogue, 100);
            spells::cast_triggered(m, rogue, VANISH, Some(rogue));
            health(m, rogue)
        })
        .await
        .unwrap();
    assert_eq!(got, 600, "500 more");
}

/// Cheat Death goes off only for the blow that takes its bearer from 20%
/// or more to below it.
#[tokio::test]
#[ignore = "extended"]
async fn cheat_death_goes_off_only_as_health_drops_below_a_fifth() {
    let Some(realm) = real() else { return };
    let (_a, warrior, map) =
        player(&realm, "P12FSCDA", "Cheater", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, warrior, 60, &[]);
            let k = dummy(m, warrior, 60, 5.0);
            wear(m, warrior, &[CHEAT_DEATH]);
            let max = m
                .unit_fields(warrior)
                .unwrap()
                .get_u32(UNIT_FIELD_MAXHEALTH);
            // A blow of `damage`, the bearer left at `after`.
            let blow = |m: &mut Map, after: u32, damage: u32| {
                auras::remove_spell(m, warrior, 28846, None);
                set_health(m, warrior, after);
                procs::trigger(
                    m,
                    procs::Event {
                        actor: k,
                        target: warrior,
                        target_flags: procs::PROC_TAKEN_MELEE,
                        ex: procs::EX_NORMAL_HIT,
                        damage,
                        school: 1,
                        ..Default::default()
                    },
                );
                has(m, warrior, 28846)
            };
            let fifth = max / 5;
            (
                blow(m, fifth + 10, 20),
                blow(m, fifth - 10, 20),
                blow(m, fifth - 30, 20),
            )
        })
        .await
        .unwrap();
    assert_eq!(got, (false, true, false), "above, across, already below");
}
