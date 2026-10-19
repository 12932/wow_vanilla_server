//! Phase 3, round 6, against Thistle Tea's `spell_reception_test.exs`:
//! drains whose caster is dead or gone, and an aura's resistance to a
//! dispel, its caster's (its master's, for a pet) as it is now.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{
    self as kit, at, cast_fresh, has, health, power, set_health, tick_now,
};
use super::tests_p6_wspell::{BEAST, RANGE_30, off, sides, unit_at};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::{
    AURA_ADD_FLAT_MODIFIER, AURA_MOD_STAT, AURA_PERIODIC_DAMAGE, AURA_PERIODIC_HEALTH_FUNNEL,
    AURA_PERIODIC_LEECH, AURA_PERIODIC_MANA_LEECH,
};
use crate::game::map::creature::Life;
use crate::game::map::pets::Kind;
use crate::game::map::spells::mods;

/// Drains of 10 a second: of life, of mana, and a funnel of life.
const LEECH: u32 = 991_200;
const MANA_LEECH: u32 = 991_201;
const FUNNEL: u32 = 991_202;
/// A magic affliction and a magic blessing of the warlocks' family; a
/// talent that holds them on through any dispel; a dispel of one magic
/// aura.
const AFFLICTION: u32 = 991_203;
const BLESSING: u32 = 991_204;
const HOLD_ON: u32 = 991_205;
const DISPEL: u32 = 991_206;
/// A party's aura of strength, ranks 1 (level 1, 3) and 2 (level 60, 20);
/// one of level 60 with no rank below it.
const AURA_LOW: u32 = 991_207;
const AURA_HIGH: u32 = 991_208;
const AURA_LONE: u32 = 991_209;

fn data(d: &mut GameData) {
    sides(d);
    let drain = |id: u32, kind: u32, misc: i32| {
        let mut s = bare(id, 5);
        s.duration_index = LASTS_LONG;
        effect(
            &mut s,
            0,
            SpellEffect {
                amplitude: 1_000,
                multiple_value: 1.0,
                ..aura(kind, misc, 10, 6)
            },
        );
        s
    };
    let magic = |id: u32, e: SpellEffect| {
        let mut s = bare(id, 5);
        s.duration_index = LASTS_LONG;
        s.dispel = 1;
        s.family_name = 5;
        s.family_flags = 0x2;
        effect(&mut s, 0, e);
        s
    };
    let mut hold_on = bare(HOLD_ON, 0);
    hold_on.family_name = 5;
    hold_on.duration_index = LASTS_LONG;
    effect(
        &mut hold_on,
        0,
        SpellEffect {
            item_type: 0x2,
            ..aura(
                AURA_ADD_FLAT_MODIFIER,
                mods::OP_RESIST_DISPEL_CHANCE as i32,
                100,
                1,
            )
        },
    );
    let mut dispel = bare(DISPEL, 1);
    dispel.range_index = RANGE_30;
    effect(
        &mut dispel,
        0,
        SpellEffect {
            effect: 38,
            misc_value: 1,
            base_points: 0,
            base_dice: 1,
            target_a: 25,
            ..Default::default()
        },
    );
    let party_aura = |id: u32, level: u32, amount: i32| {
        let mut s = bare(id, 1);
        s.duration_index = 0;
        s.spell_level = level;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 35,
                radius_index: 0,
                ..aura(AURA_MOD_STAT, 0, amount, 1)
            },
        );
        s
    };
    for (id, prev, first, rank) in [
        (AURA_LOW, 0, AURA_LOW, 1),
        (AURA_HIGH, AURA_LOW, AURA_LOW, 2),
        (AURA_LONE, 0, AURA_LONE, 1),
    ] {
        d.spells.ranks.insert(
            id,
            crate::game::data::spells::SpellRank {
                prev,
                first,
                rank,
                req: 0,
            },
        );
    }
    let spells: Vec<SpellTemplate> = vec![
        party_aura(AURA_LOW, 1, 3),
        party_aura(AURA_HIGH, 60, 20),
        party_aura(AURA_LONE, 60, 5),
        drain(LEECH, AURA_PERIODIC_LEECH, 0),
        drain(MANA_LEECH, AURA_PERIODIC_MANA_LEECH, 0),
        drain(FUNNEL, AURA_PERIODIC_HEALTH_FUNNEL, 0),
        magic(AFFLICTION, aura(AURA_PERIODIC_DAMAGE, 0, 1, 6)),
        magic(
            BLESSING,
            SpellEffect {
                amplitude: 0,
                ..aura(AURA_MOD_STAT, 0, 1, 21)
            },
        ),
        hold_on,
        dispel,
    ];
    insert(d, spells);
}

fn alive(m: &mut Map, g: Guid, on: bool) {
    let now = m.now;
    if let Some(c) = m.creature_mut(g) {
        c.life = if on {
            Life::Alive
        } else {
            Life::Dead {
                corpse_until: now,
                respawn_at: now,
            }
        };
    }
}

/// A drain of life, of mana, or a funnel of life does nothing at a tick
/// while its caster is dead, and again once it lives; with its caster
/// gone, nothing (Thistle Tea's aura contexts; mangos's `PeriodicTick`
/// wanting a caster alive).
#[tokio::test]
async fn a_drain_wants_its_caster_alive_and_here() {
    let realm = fast_realm(data);
    let (_c, b, map) = human(&realm, "P6XB", "Drained", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, b, 60, &[]);
            let mut out = Vec::new();
            for (spell, read) in [(LEECH, 0usize), (MANA_LEECH, 1), (FUNNEL, 2)] {
                set_health(m, b, 1_000);
                let k = unit_at(m, b, BEAST, 3.0, 0.0);
                kit::apply(m, k, b, spell);
                let value = |m: &Map| match read {
                    1 => power(m, b, 0),
                    _ => health(m, b),
                };
                let mut seen = vec![value(m)];
                tick_now(m, b, spell);
                seen.push(value(m));
                alive(m, k, false);
                tick_now(m, b, spell);
                seen.push(value(m));
                alive(m, k, true);
                tick_now(m, b, spell);
                seen.push(value(m));
                m.remove_creature(k);
                tick_now(m, b, spell);
                seen.push(value(m));
                crate::game::map::auras::remove_spell(m, b, spell, None);
                let steps: Vec<i64> = seen.windows(2).map(|w| w[1] as i64 - w[0] as i64).collect();
                out.push(steps);
            }
            out
        })
        .await
        .unwrap();
    let ticks = |n: i64| vec![n, 0, n, 0];
    assert_eq!(out[0], ticks(-10), "life drained: alive, dead, alive, gone");
    assert_eq!(out[1], ticks(-10), "mana drained");
    assert_eq!(out[2], ticks(10), "life funnelled");
}

/// A dispel's chance to fail is the aura's caster's modifiers as they are
/// at the dispel, not the dispeller's; a caster gone gives none; a pet's
/// aura its master's while the pet is here; one's own aura, one's own
/// (Thistle Tea's `DispelResistance` in `SpellReception`; mangos's
/// `EffectDispel` with the caster's `GetSpellModOwner`).
#[tokio::test]
async fn a_dispel_is_resisted_by_the_auras_casters_modifiers_now() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XC", "Dispeller", Class::Priest).await;
    let (_d, b, _) = human(&realm, "P6XD", "Afflicted", Class::Mage).await;
    let (_e, owner, _) = human(&realm, "P6XE", "Petowner", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[DISPEL]);
            let mut out = Vec::new();
            let dispel = |m: &mut Map, by: Guid, at_unit: Guid, spell: u32| {
                assert!(cast_fresh(m, by, DISPEL, at(at_unit)), "the dispel cast");
                !has(m, at_unit, spell)
            };
            // The kobold's affliction, the kobold holding it on; then not.
            let k = unit_at(m, a, BEAST, 3.0, 0.0);
            kit::apply(m, k, k, HOLD_ON);
            kit::apply(m, k, b, AFFLICTION);
            out.push(dispel(m, a, b, AFFLICTION));
            crate::game::map::auras::remove_spell(m, k, HOLD_ON, None);
            out.push(dispel(m, a, b, AFFLICTION));
            // The dispeller holding it on lends nothing.
            kit::apply(m, a, a, HOLD_ON);
            kit::apply(m, k, b, AFFLICTION);
            out.push(dispel(m, a, b, AFFLICTION));
            crate::game::map::auras::remove_spell(m, a, HOLD_ON, None);
            // Its caster gone, held on before: nothing holds it.
            kit::apply(m, k, k, HOLD_ON);
            kit::apply(m, k, b, AFFLICTION);
            m.remove_creature(k);
            out.push(dispel(m, a, b, AFFLICTION));
            // A pet's, its master's while the pet is here.
            let to = off(m, a, 2.0, 0.0);
            let pet =
                crate::game::map::pets::testing::spawn(m, owner, BEAST, Kind::Hunter, 60, to, 0.0);
            m.players.get_mut(&owner).unwrap().owned.pet = Some(pet);
            kit::apply(m, owner, owner, HOLD_ON);
            kit::apply(m, pet, b, AFFLICTION);
            out.push(dispel(m, a, b, AFFLICTION));
            m.remove_creature(pet);
            out.push(dispel(m, a, b, AFFLICTION));
            // One's own blessing, held on by one's own talent, against an
            // enemy's dispel.
            kit::apply(m, b, b, BLESSING);
            kit::apply(m, b, b, HOLD_ON);
            let enemy = unit_at(m, b, BEAST, 3.0, 0.0);
            out.push(dispel(m, enemy, b, BLESSING));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [false, true, true, true, false, true, false],
        "held by the caster's; not after; not by the dispeller's; none gone; the pet's master's; none with the pet gone; one's own"
    );
}

/// A party's aura reaches each member at the rank their level may have: a
/// low one for the low, the caster's once they are high enough, none if no
/// rank may; it goes when they leave its reach, and when its caster drops
/// it (Thistle Tea's party aura ranks; mangos's area aura
/// `SelectAuraRankForLevel`).
#[tokio::test]
async fn a_party_aura_reaches_each_member_at_the_rank_of_their_level() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XF", "Paladin", Class::Paladin).await;
    let (_d, b, _) = human(&realm, "P6XG", "Novice", Class::Mage).await;
    let lo = realm.registry.online(a).unwrap();
    let bo = realm.registry.online(b).unwrap();
    assert!(crate::game::social::groups::join_directly(&realm, &lo, &bo));
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[AURA_HIGH, AURA_LONE]);
            let data = m.realm.data.clone();
            crate::game::map::stats::set_level(m.players.get_mut(&b).unwrap(), &data, 1);
            let refresh = |m: &mut Map| {
                m.spells.area_auras_at = None;
                kit::pass(m, 0);
            };
            let ranks = |m: &Map| {
                [AURA_LOW, AURA_HIGH, AURA_LONE]
                    .into_iter()
                    .filter(|s| has(m, b, *s))
                    .collect::<Vec<_>>()
            };
            let amount =
                |m: &Map, spell: u32| kit::holder(m, b, spell).map(|h| h.effects[0].amount);
            let mut out = Vec::new();
            assert!(cast_fresh(m, a, AURA_HIGH, Default::default()));
            assert!(cast_fresh(m, a, AURA_LONE, Default::default()));
            refresh(m);
            out.push((ranks(m), amount(m, AURA_LOW)));
            crate::game::map::stats::set_level(m.players.get_mut(&b).unwrap(), &data, 50);
            refresh(m);
            out.push((ranks(m), amount(m, AURA_HIGH)));
            let far = off(m, a, 60.0, 0.0);
            super::tests_p6_wspell::put(m, b, far);
            refresh(m);
            out.push((ranks(m), None));
            let near = off(m, a, 2.0, 0.0);
            super::tests_p6_wspell::put(m, b, near);
            refresh(m);
            crate::game::map::auras::remove_spell(m, a, AURA_HIGH, None);
            refresh(m);
            out.push((ranks(m), None));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out[0],
        (vec![AURA_LOW], Some(3)),
        "level 1: the low rank, none of the lone one"
    );
    assert_eq!(
        out[1],
        (vec![AURA_HIGH, AURA_LONE], Some(20)),
        "level 50: the caster's ranks"
    );
    assert_eq!(out[2].0, Vec::<u32>::new(), "out of reach");
    assert_eq!(out[3].0, vec![AURA_LONE], "the caster dropped one");
}
