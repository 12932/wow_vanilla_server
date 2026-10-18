//! Phase 3, round 5, against Thistle Tea's `spell_effect_test.exs`: whom a
//! pet's effects at its master land on, a helpful ability that can't be
//! dodged, and a spell whose every effect is resisted.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, has, health, holder, missed};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{
    AURA_MOD_DAMAGE_DONE, AURA_MOD_MECHANIC_RESISTANCE, AURA_MOD_STAT, AURA_MOD_STUN,
};
use crate::game::map::spells;

const MASTER_AND_SELF: u32 = 991_300;
const ENEMY_AND_MASTER: u32 = 991_301;
const PARTY_BUFF: u32 = 991_302;
const STUN: u32 = 991_303;
const STUN_PROOF: u32 = 991_304;

fn kit_data(d: &mut GameData) {
    let lasting = |id: u32, effects: &[SpellEffect]| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_15S;
        for (i, e) in effects.iter().enumerate() {
            effect(&mut s, i, *e);
        }
        s
    };
    let mut enemy_and_master = lasting(
        ENEMY_AND_MASTER,
        &[
            damage(50),
            SpellEffect {
                target_b: 27,
                ..aura(AURA_MOD_STAT, 3, 10, 6)
            },
        ],
    );
    enemy_and_master.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    enemy_and_master.school = 2;
    let mut party = lasting(PARTY_BUFF, &[aura(AURA_MOD_DAMAGE_DONE, 1, 8, 20)]);
    party.dmg_class = 2;
    let mut stun = lasting(
        STUN,
        &[SpellEffect {
            mechanic: 12,
            ..aura(AURA_MOD_STUN, 0, 0, 6)
        }],
    );
    stun.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    let spells: Vec<SpellTemplate> = vec![
        lasting(
            MASTER_AND_SELF,
            &[
                aura(AURA_MOD_STAT, 3, 10, 27),
                aura(AURA_MOD_DAMAGE_DONE, 1, 5, 1),
            ],
        ),
        enemy_and_master,
        party,
        stun,
        lasting(
            STUN_PROOF,
            &[aura(AURA_MOD_MECHANIC_RESISTANCE, 12, 100, 1)],
        ),
    ];
    insert(d, spells);
}

fn kinds(m: &Map, g: Guid, spell: u32) -> Vec<u32> {
    holder(m, g, spell).map_or(Vec::new(), |h| h.effects.iter().map(|x| x.kind).collect())
}

/// A pet's spell: its effect at its master on the master alone, its effect
/// at itself on itself; one at an enemy that names the master second hurts
/// only the enemy, and its aura goes on both (Thistle Tea's master-targeted
/// tests).
#[tokio::test]
async fn a_pets_effects_at_its_master_and_at_itself_land_apart() {
    let realm = fast_realm(kit_data);
    let (_a, a, map) = human(&realm, "P5FA", "Master", Class::Warrior).await;
    let (_b, b, _) = human(&realm, "P5FB", "Enemy", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            kit::ready(m, b, 60, &[]);
            let pet = kit::dummy(m, a, 60, 2.0);
            if let Some(c) = m.creature_mut(pet) {
                c.owner = Some(a);
            }
            spells::cast_triggered(m, pet, MASTER_AND_SELF, Some(pet));
            let apart = (kinds(m, a, MASTER_AND_SELF), kinds(m, pet, MASTER_AND_SELF));
            kit::set_health(m, a, 1_000);
            kit::set_health(m, b, 1_000);
            spells::cast_triggered(m, pet, ENEMY_AND_MASTER, Some(b));
            let enemy = (health(m, b) < 1_000, has(m, b, ENEMY_AND_MASTER));
            let master = (health(m, a), has(m, a, ENEMY_AND_MASTER));
            (apart, enemy, master)
        })
        .await
        .unwrap();
    let ((on_master, on_pet), enemy, master) = out;
    assert_eq!(
        on_master,
        [AURA_MOD_STAT],
        "the master's effect on the master"
    );
    assert_eq!(on_pet, [AURA_MOD_DAMAGE_DONE], "its own on the pet");
    assert_eq!(enemy, (true, true), "the enemy hurt and affected");
    assert_eq!(master, (1_000, true), "the master unhurt, affected");
}

/// A helpful ability of the melee class lands on a party member who would
/// dodge anything (Thistle Tea's "positive melee-class buffs cannot be
/// dodged"); a spell whose every effect is resisted is told as a resist
/// and puts nothing on (its "reports resistance").
#[tokio::test]
async fn a_helpful_ability_isnt_dodged_and_a_wholly_resisted_spell_is_a_resist() {
    let realm = fast_realm(kit_data);
    let (mut ca, a, map) = human(&realm, "P5FC", "Shouter", Class::Warrior).await;
    let (_b, b, _) = human(&realm, "P5FD", "Dodger", Class::Warrior).await;
    let lo = realm.registry.online(a).unwrap();
    let bo = realm.registry.online(b).unwrap();
    assert!(crate::game::social::groups::join_directly(&realm, &lo, &bo));
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            kit::ready(m, b, 60, &[]);
            if let Some(p) = m.players.get_mut(&b) {
                p.derived.dodge = 100.0;
                p.derived.parry = 100.0;
            }
            let landed = (0..20)
                .filter(|_| {
                    crate::game::map::auras::remove_spell(m, b, PARTY_BUFF, None);
                    kit::fresh(m, a);
                    spells::cast_triggered(m, a, PARTY_BUFF, None);
                    has(m, b, PARTY_BUFF)
                })
                .count();
            let kobold = kit::dummy(m, a, 60, 3.0);
            kit::apply(m, kobold, kobold, STUN_PROOF);
            spells::cast_triggered(m, a, STUN, Some(kobold));
            (landed, has(m, kobold, STUN))
        })
        .await
        .unwrap();
    let (landed, stunned) = out;
    assert_eq!(landed, 20, "never dodged");
    assert!(!stunned, "nothing on");
    assert_eq!(missed(&mut ca, STUN).await, 2, "told a resist");
}
