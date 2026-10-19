//! Phase 8's loader round, the rogue's and the mage's real talents and
//! tricks (Thistle Tea's `world/loader/target_trigger_dbc_test.exs`,
//! `spell_dispel_resistance_dbc_test.exs`, `pickpocket_dbc_test.exs`,
//! `distraction_dbc_test.exs`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells::{self, mods};
use crate::game::map::{Map, MapHandle, auras, procs};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// The amount of `g`'s target trigger from `talent`.
fn trigger_chance(m: &Map, g: Guid, talent: u32) -> Option<i32> {
    holder(m, g, talent).and_then(|h| {
        h.effects
            .iter()
            .find(|x| x.kind == auras::AURA_ADD_TARGET_TRIGGER)
            .map(|x| x.amount)
    })
}

/// Make `g`'s target trigger from `talent` a sure one.
fn sure(m: &mut Map, g: Guid, talent: u32) {
    if let Some(a) = auras::auras_mut(m, g) {
        for h in a.holders.iter_mut().filter(|h| h.spell == talent) {
            for x in h.effects.iter_mut() {
                x.amount = 100;
            }
        }
    }
}

const RELENTLESS_STRIKES: u32 = 14_179;
const REVEALED_FLAW: u32 = 28_814;
const REVEALED_FLAW_BONUS: u32 = 28_815;
const EVISCERATE: u32 = 2098;
const RUPTURE: u32 = 1943;

/// Thistle Tea's target triggers (`target_trigger_dbc_test.exs`):
/// Relentless Strikes is no chance of its own and 20 a finisher's point,
/// so five points are sure: Eviscerate, Expose Armor, Slice and Dice,
/// Rupture and Kidney Shot give its 25 energy, none without points, and
/// Sinister Strike none. Revealed Flaw is 5 a point, for Eviscerate alone
/// (vmangos's class mask 0x20000), not Rupture.
#[tokio::test]
#[ignore = "extended"]
async fn finishers_trigger_by_their_points() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LRELENT", "Relentless", Race::Human, Class::Rogue).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        apply(m, g, g, RELENTLESS_STRIKES);
        let own = trigger_chance(m, g, RELENTLESS_STRIKES);
        let energy = |m: &mut Map, id: u32, points: u8| {
            set_power(m, g, 3, 0);
            procs::target_triggers(m, g, g, id, points);
            power(m, g, 3)
        };
        let finishers: Vec<(u32, u32, u32)> = [EVISCERATE, 8647, 5171, RUPTURE, 408, 1752]
            .into_iter()
            .map(|id| (id, energy(m, id, 5), energy(m, id, 0)))
            .collect();
        auras::remove_spell(m, g, RELENTLESS_STRIKES, None);
        apply(m, g, g, REVEALED_FLAW);
        let flaw_own = trigger_chance(m, g, REVEALED_FLAW);
        let flaw = |m: &mut Map, id: u32, points: u8| {
            auras::remove_spell(m, g, REVEALED_FLAW_BONUS, None);
            procs::target_triggers(m, g, g, id, points);
            has(m, g, REVEALED_FLAW_BONUS)
        };
        let flaws = [
            flaw(m, EVISCERATE, 20),
            flaw(m, EVISCERATE, 0),
            flaw(m, RUPTURE, 20),
        ];
        (own, finishers, flaw_own, flaws)
    })
    .await;
    let (own, finishers, flaw_own, flaws) = out;
    assert_eq!(own, Some(0));
    assert_eq!(
        finishers,
        vec![
            (EVISCERATE, 25, 0),
            (8647, 25, 0),
            (5171, 25, 0),
            (RUPTURE, 25, 0),
            (408, 25, 0),
            (1752, 0, 0),
        ]
    );
    assert_eq!(flaw_own, Some(0));
    assert_eq!(flaws, [true, false, false], "Eviscerate's points alone");
}

const CHILLED: u32 = 12_486;
const FROSTBITE: u32 = 12_494;

/// Thistle Tea's Frostbite (`target_trigger_dbc_test.exs`): its three ranks
/// are a 5, 10 and 15% chance, and (made sure) root a target that Chilled
/// lands on; not one a Blizzard's wave hits, nor one Frostbite's own root
/// does.
#[tokio::test]
#[ignore = "extended"]
async fn frostbite_comes_of_chilled_alone() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LFROST", "Frostbiter", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let mut chances = Vec::new();
        for talent in [11_071, 12_496, 12_497] {
            apply(m, g, g, talent);
            chances.push(trigger_chance(m, g, talent));
            auras::remove_spell(m, g, talent, None);
        }
        apply(m, g, g, 11_071);
        sure(m, g, 11_071);
        let rooted = |m: &mut Map, id: u32| {
            let foe = dummy(m, g, 60, 10.0);
            procs::target_triggers(m, g, foe, id, 0);
            has(m, foe, FROSTBITE)
        };
        (
            chances,
            [rooted(m, CHILLED), rooted(m, 10), rooted(m, FROSTBITE)],
        )
    })
    .await;
    assert_eq!(out.0, vec![Some(5), Some(10), Some(15)]);
    assert_eq!(out.1, [true, false, false]);
}

/// Thistle Tea's Vile Poisons (`spell_dispel_resistance_dbc_test.exs`): its
/// fifth rank (16720) gives the rogue's poisons (Deadly, Crippling and
/// Wound Poison) a 40% chance to resist a dispel, and Garrote none.
#[tokio::test]
#[ignore = "extended"]
async fn vile_poisons_guard_poisons_from_dispels_not_garrote() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LVILE", "Vilepoison", Race::Human, Class::Rogue).await;
    let out = on(&map, move |m| {
        ready(m, g, 50, &[]);
        apply(m, g, g, 16_720);
        [2818, 3409, 13_218, 703].map(|id| {
            let s = m.realm.data.spells.get(id).cloned().unwrap();
            (id, mods::for_spell(m, g, &s).dispel_resistance())
        })
    })
    .await;
    assert_eq!(
        out,
        [(2818, 40.0), (3409, 40.0), (13_218, 40.0), (703, 0.0)]
    );
}

const PICK_POCKET: u32 = 921;
const STEALTH: u32 = 1784;
const DISTRACT: u32 = 1725;
const DEFIAS_SMUGGLER: u32 = 95;

fn stealthed(m: &Map, g: Guid) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOD_STEALTH))
}

fn fighting(m: &Map, k: Guid, g: Guid) -> bool {
    m.creature(k)
        .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == g))
}

/// Thistle Tea's Pick Pocket (`pickpocket_dbc_test.exs`): a harmful spell
/// of five yards a rogue may cast stealthed; a pocket picked leaves the
/// rogue stealthed and the Defias Smuggler none the wiser, and one that
/// fails (its spell resisted) brings the rogue out of stealth and the
/// smuggler down on them (mangos's `CasterHitTargetWithSpell`:
/// `FAILURE_BREAKS_STEALTH`, threat only on a miss).
#[tokio::test]
#[ignore = "extended"]
async fn pick_pocket_keeps_stealth_and_a_failed_one_is_noticed() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let s = d.spells.get(PICK_POCKET).unwrap();
    assert!(s.attr(1, crate::game::data::spells::ATTR_EX_ALLOW_WHILE_STEALTHED));
    assert!(
        s.attr(1, 0x1000) && s.attr(1, 0x20_0000),
        "breaks stealth, threat on a miss"
    );
    assert!(!auras::positive(s));
    assert_eq!(d.spells.range(s).1, 5.0);
    let (_c, g, map) = player(&realm, "P8LPOCKET", "Pocketer", Race::Human, Class::Rogue).await;
    let out = on(&map, move |m| {
        ready(m, g, 20, &[PICK_POCKET, STEALTH]);
        let mut seen = Vec::new();
        for resisted in [false, true] {
            let k = spawn(m, g, DEFIAS_SMUGGLER, 11, 3.0);
            if resisted {
                // Sure to resist a physical spell: the least chance to land,
                // which the fixed rolls take for a miss.
                let mut warded = made_up(990_861, 6, auras::AURA_MOD_RESIST_CHANCE, 1, 200);
                warded.duration_index = 21;
                apply_template(m, k, k, &warded);
            }
            fresh(m, g);
            spells::cast_triggered(m, g, STEALTH, Some(g));
            let cast = cast_fresh(m, g, PICK_POCKET, at(k));
            let picked = m.creature(k).unwrap().loot.is_some();
            seen.push((cast, picked, stealthed(m, g), fighting(m, k, g)));
            auras::remove_spell(m, g, STEALTH, None);
        }
        seen
    })
    .await;
    assert_eq!(out[0], (true, true, true, false), "picked unseen");
    assert_eq!(out[1], (true, false, false, true), "a failure is noticed");
}

/// Thistle Tea's Distract (`distraction_dbc_test.exs`): a harmful ground
/// spell (10 yards about where it lands, for 10 seconds) a rogue may cast
/// stealthed: the kobold near the spot turns to it and stands, the rogue
/// still stealthed and not in a fight with it.
#[tokio::test]
#[ignore = "extended"]
async fn distract_turns_a_creature_and_keeps_its_rogue_hidden() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let s = d.spells.get(DISTRACT).unwrap();
    assert!(s.attr(1, crate::game::data::spells::ATTR_EX_ALLOW_WHILE_STEALTHED));
    assert!(!auras::positive(s));
    assert_eq!(spells::points(s, 0, 60), 10);
    assert_eq!(d.spells.radius(s.effects[0].radius_index), 10.0);
    let (_c, g, map) = player(
        &realm,
        "P8LDISTRACT",
        "Distracter",
        Race::Human,
        Class::Rogue,
    )
    .await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[DISTRACT, STEALTH]);
        let k = dummy(m, g, 60, 15.0);
        spells::cast_triggered(m, g, STEALTH, Some(g));
        let at = m.creature(k).unwrap().position;
        let spot = Vector3d {
            x: at.x + 3.0,
            ..at
        };
        fresh(m, g);
        let cast = spells::cast(
            m,
            g,
            DISTRACT,
            spells::Targets {
                dest: Some(spot),
                ..Default::default()
            },
            None,
        );
        let c = m.creature(k).unwrap();
        let held = c
            .engagement
            .distracted_until
            .map(|t| t.saturating_duration_since(m.now).as_secs());
        (
            cast,
            held,
            c.orientation.cos() > 0.99,
            stealthed(m, g),
            fighting(m, k, g),
        )
    })
    .await;
    assert_eq!(out, (true, Some(10), true, true, false));
}
