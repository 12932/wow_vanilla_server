//! Phase 3's chains, as Thistle Tea's `chain_test` has them: each jump's
//! share by the effect's multiplier, a miss that takes a place without
//! taking a share, the modifiers to the jumps and the share, and heals.
//! On made-up spells and the fast suite.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{apply_template, at, dummy, fresh, health, ready, set_health};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, mods};

const BOLT: u32 = 990_900;
const HEAL: u32 = 990_901;
const JUMPS: u32 = 990_902;
const IMMUNE: u32 = 990_903;
const UNCHAINED: u32 = 990_904;

fn chains(d: &mut GameData) {
    let mut bolt = bare(BOLT, 3);
    bolt.family_name = 11;
    bolt.family_flags = 2;
    bolt.range_index = 0;
    effect(
        &mut bolt,
        0,
        SpellEffect {
            chain_target: 3,
            ..damage(100)
        },
    );
    bolt.dmg_multiplier = [0.5, 1.0, 1.0];
    let mut heal = bare(HEAL, 3);
    effect(
        &mut heal,
        0,
        SpellEffect {
            effect: 10,
            chain_target: 3,
            target_a: 45,
            ..damage(100)
        },
    );
    heal.dmg_multiplier = [0.5, 1.0, 1.0];
    // One more jump, and a fifth more of each share.
    let mut jumps = bare(JUMPS, 0);
    jumps.family_name = 11;
    jumps.duration_index = LASTS_LONG;
    for (i, (op, amount)) in [(mods::OP_JUMP_TARGETS, 1), (mods::OP_EFFECT_PAST_FIRST, 20)]
        .into_iter()
        .enumerate()
    {
        let kind = if i == 0 {
            AURA_ADD_FLAT_MODIFIER
        } else {
            AURA_ADD_PCT_MODIFIER
        };
        effect(
            &mut jumps,
            i,
            SpellEffect {
                item_type: 2,
                ..aura(kind, op as i32, amount, 1)
            },
        );
    }
    let mut immune = bare(IMMUNE, 0);
    immune.duration_index = LASTS_LONG;
    effect(&mut immune, 0, aura(AURA_SCHOOL_IMMUNITY, 0x8, 0, 1));
    // A bolt whose multiplier no chain uses.
    let mut unchained = bare(UNCHAINED, 3);
    effect(&mut unchained, 0, damage(100));
    unchained.dmg_multiplier = [0.2, 1.0, 1.0];
    insert(d, [bolt, heal, jumps, immune, unchained]);
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).cloned().unwrap()
}

/// Kobolds in a line before `g`, three yards apart.
fn line(m: &mut Map, g: Guid, n: usize) -> Vec<Guid> {
    (0..n)
        .map(|i| dummy(m, g, 60, 3.0 + 3.0 * i as f32))
        .collect()
}

fn lost(m: &Map, ks: &[Guid]) -> Vec<u32> {
    ks.iter().map(|k| 1_000_000 - health(m, *k)).collect()
}

/// A chain's damage: the first whole, each jump the multiplier of the one
/// before; a target that resists or is immune takes a place and no share;
/// a jump more and a fifth more of each share from the caster's
/// modifiers; a bolt that doesn't chain keeps its damage whatever its
/// multiplier (Thistle Tea's `Chain.plan` and `scale`).
#[tokio::test]
async fn a_chain_takes_its_share_at_each_jump_and_a_miss_takes_none() {
    let realm = fast_realm(chains);
    let (_c, shaman, map) = human(&realm, "P3CH", "Chainer", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, shaman, 60, &[]);
            let ks = line(m, shaman, 4);
            fresh(m, shaman);
            spells::cast(m, shaman, BOLT, at(ks[0]), None);
            let plain = lost(m, &ks);
            for k in &ks {
                set_health(m, *k, 1_000_000);
            }
            // The second immune.
            let t = template(m, IMMUNE);
            apply_template(m, ks[1], ks[1], &t);
            fresh(m, shaman);
            spells::cast(m, shaman, BOLT, at(ks[0]), None);
            let missed = lost(m, &ks);
            remove_spell(m, ks[1], IMMUNE, None);
            for k in &ks {
                set_health(m, *k, 1_000_000);
            }
            let t = template(m, JUMPS);
            apply_template(m, shaman, shaman, &t);
            fresh(m, shaman);
            spells::cast(m, shaman, BOLT, at(ks[0]), None);
            let modified = lost(m, &ks);
            for k in &ks {
                set_health(m, *k, 1_000_000);
            }
            fresh(m, shaman);
            spells::cast(m, shaman, UNCHAINED, at(ks[0]), None);
            let unchained = lost(m, &ks);
            (plain, missed, modified, unchained)
        })
        .await
        .unwrap();
    let (plain, missed, modified, unchained) = out;
    assert_eq!(plain, [100, 50, 25, 0], "a half at each jump, three in all");
    assert_eq!(
        missed,
        [100, 0, 50, 0],
        "the immune takes a place, not a share"
    );
    assert_eq!(modified, [100, 60, 36, 21], "a jump more, and 60% of each");
    assert_eq!(unchained, [100, 0, 0, 0]);
}

/// A chain heal goes to the most hurt friend in reach each jump, back to
/// its caster too, with its share at each (Thistle Tea's chain heal).
#[tokio::test]
async fn a_chain_heal_jumps_to_the_most_hurt_and_back_to_its_caster() {
    let realm = fast_realm(chains);
    let (_c, shaman, map) = human(&realm, "P3CI", "Healer", Class::Mage).await;
    let (_d, friend, _) = human(&realm, "P3CJ", "Friendone", Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, shaman, 60, &[]);
            let at_me = m.players[&shaman].position();
            if let Some(p) = m.players.get_mut(&friend) {
                p.movement.position = Vector3d {
                    x: at_me.x + 3.0,
                    ..at_me
                };
            }
            for g in [shaman, friend] {
                if let Some(f) = m.unit_fields_mut(g) {
                    f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
                }
                set_health(m, g, 100);
            }
            fresh(m, shaman);
            spells::cast(m, shaman, HEAL, at(friend), None);
            (health(m, friend) - 100, health(m, shaman) - 100)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (100, 50),
        "the friend first, then back to the caster at half"
    );
}
