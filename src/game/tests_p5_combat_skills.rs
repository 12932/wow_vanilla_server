//! Phase 3's fifth round of combat tests, weapon skills (Thistle Tea's
//! `combat_skills_test.exs`): abilities train the weapon they are made
//! with, as swings do.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, skills};
use crate::game::tests_p3_combat::{self as base, *};

const STRIKE_OF: u32 = 90_860;
const SHOT: u32 = 90_861;
const BOLT: u32 = 90_862;
const BOW: u32 = 63_541;
const CRITTER: u32 = 3730;

const SWORDS: u32 = 43;
const BOWS: u32 = 45;
const UNARMED: u32 = 162;

fn attack(id: u32, class: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Attack {id}"),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: class,
        school: if class == 1 { 4 } else { 0 },
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 0,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = base::data();
    for s in [attack(STRIKE_OF, 2), attack(SHOT, 3), attack(BOLT, 1)] {
        d.spells.spells.insert(s.id, s);
    }
    d.items.insert(
        BOW,
        ItemTemplate {
            entry: BOW,
            class: 2,
            subclass: 2,
            name: "Bow".into(),
            display_id: 1000,
            inventory_type: 15,
            delay: 3000,
            stackable: 1,
            ..Default::default()
        },
    );
    d.creature_templates.insert(CRITTER, template(CRITTER, 8));
    d
}

fn skill(m: &Map, g: Guid, skill: u32) -> u32 {
    skills::value(&m.players[&g], skill).unwrap_or(0)
}

fn cast(m: &mut Map, g: Guid, spell: u32, at: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    p.caster.casting = None;
    p.caster.gcd_until = None;
    p.caster.cooldowns.clear();
    spells::cast(
        m,
        g,
        spell,
        Targets {
            unit: Some(at),
            ..Default::default()
        },
        None,
    );
}

/// Thistle Tea's "melee and ranged weapon abilities train once, non-weapon
/// abilities do not", "spell-damage shots and ranged debuffs train once"
/// and "ranged spell damage uses weapon accuracy and trains on misses", as
/// mangos's `ProcSkillsAndReactives` has them for a melee-based proc: a
/// melee ability trains the sword in hand, once a cast, and does so though
/// it is dodged; a ranged one the bow; a magic spell neither; a critter
/// trains nothing; and Thistle Tea's "disarm trains unarmed", as mangos's
/// `CanUseEquippedWeapon` has it.
#[tokio::test]
async fn melee_and_ranged_abilities_train_their_weapon_once_a_cast() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            equip(m, ga, BOW, 17);
            let p = m.players.get_mut(&ga).unwrap();
            for s in [SWORDS, BOWS] {
                skills::learn_skill(p, s, 1);
                skills::set_value(p, s, 10);
            }
            p.spells.extend([STRIKE_OF, SHOT, BOLT]);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let critter = spawn(m, ga, CRITTER, 60, 3.0);
            cast(m, ga, STRIKE_OF, brute);
            let struck = skill(m, ga, SWORDS);
            m.creature_mut(brute).unwrap().derived.dodge = 100.0;
            cast(m, ga, STRIKE_OF, brute);
            let dodged = skill(m, ga, SWORDS);
            m.creature_mut(brute).unwrap().derived.dodge = 0.0;
            cast(m, ga, SHOT, brute);
            let shot = (skill(m, ga, BOWS), skill(m, ga, SWORDS));
            cast(m, ga, BOLT, brute);
            let bolt = (skill(m, ga, BOWS), skill(m, ga, SWORDS));
            cast(m, ga, STRIKE_OF, critter);
            let critter = skill(m, ga, SWORDS);
            // Disarmed: unarmed.
            let p = m.players.get_mut(&ga).unwrap();
            skills::learn_skill(p, UNARMED, 1);
            skills::set_value(p, UNARMED, 10);
            aura_of(m, ga, 90_863, crate::game::map::auras::AURA_MOD_DISARM, 0, 0);
            cast(m, ga, STRIKE_OF, brute);
            let disarmed = (skill(m, ga, UNARMED), skill(m, ga, SWORDS));
            (struck, dodged, shot, bolt, critter, disarmed)
        })
        .await
        .unwrap();
    let (struck, dodged, shot, bolt, critter, disarmed) = r;
    assert_eq!(disarmed, (11, 12), "disarmed, unarmed trains");
    assert_eq!(struck, 11, "a melee ability trains the sword, once");
    assert_eq!(dodged, 12, "a dodged one too");
    assert_eq!(shot, (11, 12), "a ranged one the bow");
    assert_eq!(bolt, (11, 12), "a magic spell nothing");
    assert_eq!(critter, 12, "a critter nothing");
}
