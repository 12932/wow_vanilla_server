//! A Subtlety rogue's damage beside a Combat rogue's, as peon's builds
//! have them (its `talents` module: Hemorrhage 0/21/30 and Combat swords
//! 15/31/5), learned as peon's bots learn them (`.talents learn` between
//! `.gear` and `.character spells`), at 60 and at 40: auto-attack and the
//! builder (Hemorrhage, Sinister Strike) at a creature for eight seconds.
//!
//! Improved Sap's auras are 1.12's proc triggers of Vanished on any
//! ability; without the narrowing to Sap that vmangos has no row for, the
//! Subtlety rogue vanished out of the fight at its first or second
//! builder (60% a hit at two ranks): the creature left the fight and
//! healed, the rogue stopped swinging, 0 damage in ten seconds where the
//! Combat rogue did about 1,950.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::auras::AURA_MOD_STEALTH;
use crate::game::map::combat::{self, *};
use crate::game::tests_p3_combat as p3;

static DB: LazyLock<std::path::PathBuf> = LazyLock::new(|| harness::real_data("db"));

/// peon's Hemorrhage build: `Talent.dbc`'s ids and ranks, in the order its
/// points are spent.
const SUBTLETY: &[(u32, u8)] = &[
    (261, 5),  // Opportunity
    (244, 5),  // Camouflage
    (245, 3),  // Initiative
    (303, 1),  // Ghostly Strike
    (263, 1),  // Improved Ambush
    (1123, 3), // Serrated Blades
    (262, 2),  // Improved Sap
    (681, 1),  // Hemorrhage
    (284, 1),  // Preparation
    (265, 2),  // Dirty Deeds
    (1701, 1), // Heightened Senses
    (1702, 5), // Deadliness
    (201, 2),  // Improved Sinister Strike
    (186, 3),  // Lightning Reflexes
    (181, 5),  // Precision
    (204, 2),  // Endurance
    (187, 3),  // Deflection
    (221, 5),  // Dual Wield Specialization
    (206, 1),  // Improved Kick
];

/// peon's Combat swords build.
const COMBAT: &[(u32, u8)] = &[
    (201, 2),  // Improved Sinister Strike
    (186, 3),  // Lightning Reflexes
    (181, 5),  // Precision
    (204, 2),  // Endurance
    (187, 3),  // Deflection
    (221, 5),  // Dual Wield Specialization
    (223, 1),  // Blade Flurry
    (242, 5),  // Sword Specialization
    (1703, 2), // Weapon Expertise
    (1122, 2), // Aggression
    (205, 1),  // Adrenaline Rush
    (276, 3),  // Improved Eviscerate
    (270, 5),  // Malice
    (274, 2),  // Murder
    (281, 1),  // Relentless Strikes
    (269, 4),  // Lethality
    (261, 5),  // Opportunity
];

const SINISTER_STRIKE: &[u32] = &[1752, 1757, 1758, 1759, 1760, 8621, 11293, 11294];
const HEMORRHAGE: &[u32] = &[16511, 17347, 17348];
/// Bonescraper, a dagger, in each hand: the same weapons for both.
const DAGGER: u32 = 13_368;
/// How long each rogue fights.
const FIGHT: Duration = Duration::from_secs(8);

/// The build's points by `level`, as peon spends them: a point a level
/// from 10.
fn learned_by(build: &[(u32, u8)], level: u8) -> String {
    let mut left = u32::from(level.saturating_sub(9));
    let mut words = Vec::new();
    for (id, ranks) in build {
        if left == 0 {
            break;
        }
        let r = u32::from(*ranks).min(left);
        left -= r;
        words.push(format!("{id}:{r}"));
    }
    words.join(" ")
}

async fn gm(a: &mut Client, line: &str) {
    a.say(line).await;
    a.system_message("").await;
}

/// What a rogue did in its fight.
#[derive(Debug, Default)]
struct Fight {
    dealt: u32,
    swings: u32,
    hits: u32,
    crits: u32,
    glancing: u32,
    misses: u32,
    dodges: u32,
    parries: u32,
    swing_damage: u32,
    /// At the end: still swinging at its creature, stealthed, and the
    /// creature in the fight.
    swinging: bool,
    stealthed: bool,
    creature_fighting: bool,
}

/// A rogue of `build` at `level`, made ready as peon makes its bots,
/// fights a creature of its level for [`FIGHT`]: auto-attack, and the
/// builder each second it has the energy (Hemorrhage for Subtlety).
async fn fight(map: &MapHandle, a: &mut Client, g: Guid, build: &[(u32, u8)], level: u8) -> Fight {
    gm(a, &format!(".character level {level}")).await;
    gm(a, ".gear").await;
    gm(a, &format!(".talents learn {}", learned_by(build, level))).await;
    gm(a, ".character spells").await;
    gm(a, ".god on").await;
    gm(a, ".gm off").await;
    let hemorrhage = build == SUBTLETY;
    let (c, builder) = map
        .query(move |m| {
            p3::equip(m, g, DAGGER, 15);
            p3::equip(m, g, DAGGER, 16);
            let c = p3::spawn(m, g, 6, level, 3.0);
            let ranks = if hemorrhage {
                HEMORRHAGE
            } else {
                SINISTER_STRIKE
            };
            let known = ranks
                .iter()
                .rev()
                .find(|s| m.players[&g].spells.contains(s))
                .copied();
            (c, known)
        })
        .await
        .unwrap();
    let builder = builder.expect("the builder learned");
    a.send(CMSG_SET_SELECTION { target: c }).await;
    a.send(CMSG_ATTACKSWING { guid: c }).await;
    a.clear();
    let start = Instant::now();
    while start.elapsed() < FIGHT {
        a.send(CMSG_CAST_SPELL {
            spell: builder,
            targets: SpellCastTargets {
                target_flags: SpellCastTargets_SpellCastTargetFlags::empty()
                    .set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Unit { unit_target: c }),
            },
        })
        .await;
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    let mut f = Fight::default();
    for b in a.take_frames(combat::SMSG_ATTACKERSTATEUPDATE) {
        let (info, damage, state, _) = p3::swing_log(&b);
        f.swings += 1;
        f.swing_damage += damage;
        match state {
            VICTIMSTATE_DODGE => f.dodges += 1,
            VICTIMSTATE_PARRY => f.parries += 1,
            _ if info & HITINFO_MISS != 0 => f.misses += 1,
            _ if info & HITINFO_CRITICALHIT != 0 => f.crits += 1,
            _ if info & HITINFO_GLANCING != 0 => f.glancing += 1,
            _ => f.hits += 1,
        }
    }
    let (health, swinging, stealthed, creature_fighting) = map
        .query(move |m| {
            (
                m.unit_fields(c).map_or(0, |u| u.get_u32(UNIT_FIELD_HEALTH)),
                m.combat(g).is_some_and(|x| x.attacking && x.victim == c),
                m.players[&g].auras.has(AURA_MOD_STEALTH),
                m.creature(c).is_some_and(|x| !x.evading)
                    && m.unit_fields(c).is_some_and(|u| {
                        u.has_flag(
                            UNIT_FIELD_FLAGS,
                            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                        )
                    }),
            )
        })
        .await
        .unwrap();
    f.dealt = 100_000 - health;
    f.swinging = swinging;
    f.stealthed = stealthed;
    f.creature_fighting = creature_fighting;
    f
}

/// A Subtlety rogue and a Combat rogue of `level`, side by side.
async fn subtlety_against_combat(level: u8) {
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let mut fights = Vec::new();
    for (name, build) in [("Shade", SUBTLETY), ("Blade", COMBAT)] {
        let map = map.clone();
        let mut a = Client::connect(&realm, name);
        let g = a.enter(name, Race::Human, Class::Rogue).await;
        a.heartbeat(p3::START.0, p3::START.1, p3::START.2).await;
        fights.push(tokio::spawn(async move {
            let f = fight(&map, &mut a, g, build, level).await;
            (name, f)
        }));
    }
    let mut out = Vec::new();
    for f in fights {
        out.push(f.await.unwrap());
    }
    for (name, f) in &out {
        eprintln!(
            "{name} at {level}: {:.0} dps; {f:?}",
            f.dealt as f32 / FIGHT.as_secs_f32()
        );
    }
    let (sub, com) = (&out[0].1, &out[1].1);
    assert!(
        sub.swinging && !sub.stealthed && sub.creature_fighting,
        "the Subtlety rogue fights on: {sub:?}"
    );
    // Two daggers of 1.4 s: about eleven swings in eight seconds.
    assert!(sub.swings >= 8, "the Subtlety rogue swings: {sub:?}");
    assert!(
        sub.dealt * 10 >= com.dealt * 4,
        "the Subtlety rogue's damage near the Combat rogue's: {} against {}",
        sub.dealt,
        com.dealt
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn a_subtlety_rogue_at_60_fights_as_a_combat_rogue_does() {
    subtlety_against_combat(60).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn a_subtlety_rogue_at_40_fights_as_a_combat_rogue_does() {
    subtlety_against_combat(40).await;
}

/// Improved Sap's second rank, and a proc trigger like it.
const IMPROVED_SAP: u32 = 14_094;
const LIKE_IT: u32 = 96_950;
/// What each triggers: a mark on the rogue.
const VANISHED_MARK: u32 = 96_951;
const LIKE_IT_MARK: u32 = 96_952;

/// A world with Improved Sap's second rank as 1.12 has it (a proc trigger
/// on abilities, harmful spells and area spells at 60%, here 100%), and
/// another of the same flags, each triggering a mark.
fn sap_world() -> crate::game::data::GameData {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let mut d = p3::data();
    for (id, trigger) in [(IMPROVED_SAP, VANISHED_MARK), (LIKE_IT, LIKE_IT_MARK)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Test {id}"),
            equipped_item_class: -1,
            proc_flags: 0x1_1010,
            proc_chance: 100,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: crate::game::map::auras::AURA_PROC_TRIGGER_SPELL,
            base_dice: 1,
            target_a: 1,
            trigger_spell: trigger,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    for id in [VANISHED_MARK, LIKE_IT_MARK] {
        let mut s = SpellTemplate {
            id,
            name: format!("Test {id}"),
            equipped_item_class: -1,
            duration_index: 0,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: crate::game::map::auras::AURA_DUMMY,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

/// Improved Sap procs on nothing of its own: an ability's hit leaves the
/// rogue in the fight (no Vanished), where another proc trigger of its
/// flags goes off. Its chance is Sap's, rolled as Sap is cast
/// (`class_scripts::keeps_stealth`, tests_p8_spell_stealth).
#[tokio::test]
async fn improved_sap_procs_on_no_ability() {
    use crate::game::map::{auras, spells};
    let (_realm, _a, g, map) = p3::setup_with(sap_world(), Class::Rogue).await;
    let marks = map
        .query(move |m| {
            let data = m.realm.data.clone();
            p3::level(m, g, 60);
            for id in [IMPROVED_SAP, LIKE_IT] {
                let s = data.spells.get(id).unwrap().clone();
                let _ = auras::apply_spell(m, g, g, &s, &[(0, 0)], auras::Context::default());
            }
            let c = p3::spawn(m, g, p3::BRUTE, 60, 3.0);
            m.players.get_mut(&g).unwrap().spells.push(p3::STRIKE);
            let mut marks = (0, 0);
            for _ in 0..20 {
                let at = spells::Targets {
                    unit: Some(c),
                    ..Default::default()
                };
                spells::cast_now(m, g, p3::STRIKE, at);
                let a = &m.players[&g].auras;
                marks.0 += u32::from(a.has_spell(VANISHED_MARK));
                marks.1 += u32::from(a.has_spell(LIKE_IT_MARK));
                auras::remove_spell(m, g, VANISHED_MARK, None);
                auras::remove_spell(m, g, LIKE_IT_MARK, None);
            }
            marks
        })
        .await
        .unwrap();
    assert_eq!(marks.0, 0, "Improved Sap never procs on an ability");
    assert!(
        marks.1 >= 15,
        "a trigger like it does, on each hit: {marks:?}"
    );
}
