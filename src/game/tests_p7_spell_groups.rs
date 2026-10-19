//! Round 7, against Thistle Tea's `stack_rules_test.exs` and
//! `spell_group_stacking_test.exs`: vmangos's spell groups and their stack
//! rules (1, one of the group; 3, the group's highest).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, has, holder};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_HIDDEN, SpellEffect, SpellRank, SpellTemplate};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, checks, checks::fail, mods::Mods};

/// Strength buffs at a friend: a lesser and a greater of a rule-3 group
/// (and the lesser in a rule-1 group with one more); two of three groups
/// of rules 0, 1 and 3; a hidden one and another of a rule-1 group; a
/// parent and the aura it triggers, of a rule-1 group; two ranks of one
/// kind in a rule-1 group; the lesser's area and harmful kinds.
const LESSER: u32 = 997_701;
const GREATER: u32 = 997_702;
const UNRELATED: u32 = 997_703;
const FIRST: u32 = 997_704;
const SECOND: u32 = 997_705;
const HIDDEN: u32 = 997_706;
const PLAIN: u32 = 997_707;
const PARENT: u32 = 997_708;
const CHILD: u32 = 997_709;
const RANK_1: u32 = 997_710;
const RANK_2: u32 = 997_711;
const LESSER_AREA: u32 = 997_712;
const LESSER_CURSE: u32 = 997_713;
const RADIUS_10: u32 = 995;

fn buff(id: u32, amount: i32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_15S;
    effect(&mut s, 0, aura(AURA_MOD_STAT, 0, amount, 21));
    s
}

fn data(d: &mut GameData) {
    d.spells.radii.insert(RADIUS_10, 10.0);
    let mut group = |g: u32, rule: u32, members: &[(u32, u32)]| {
        d.spells.group_rules.insert(g, rule);
        for (spell, order) in members {
            d.spells.groups.entry(*spell).or_default().push(g);
            d.spells.group_orders.insert((g, *spell), *order);
        }
    };
    group(
        1,
        3,
        &[
            (LESSER, 1),
            (GREATER, 2),
            (LESSER_AREA, 1),
            (LESSER_CURSE, 1),
        ],
    );
    group(2, 1, &[(LESSER, 1), (UNRELATED, 2)]);
    group(3, 0, &[(FIRST, 1), (SECOND, 2)]);
    group(4, 1, &[(FIRST, 1), (SECOND, 2)]);
    group(5, 3, &[(FIRST, 1), (SECOND, 2)]);
    group(6, 1, &[(HIDDEN, 1), (PLAIN, 2)]);
    group(7, 1, &[(PARENT, 1), (CHILD, 2)]);
    group(8, 1, &[(RANK_1, 1), (RANK_2, 2)]);
    for (id, rank) in [(RANK_1, 1), (RANK_2, 2)] {
        d.spells.ranks.insert(
            id,
            SpellRank {
                prev: if rank == 2 { RANK_1 } else { 0 },
                first: RANK_1,
                rank,
                req: 0,
            },
        );
    }
    let mut hidden = buff(HIDDEN, 3);
    hidden.attributes[0] |= ATTR_HIDDEN;
    let mut parent = buff(PARENT, 4);
    effect(
        &mut parent,
        1,
        SpellEffect {
            trigger_spell: CHILD,
            ..aura(AURA_DUMMY, 0, 0, 21)
        },
    );
    let mut area = buff(LESSER_AREA, 10);
    area.effects[0].target_a = 30;
    area.effects[0].radius_index = RADIUS_10;
    let mut curse = buff(LESSER_CURSE, -10);
    curse.custom_flags |= crate::game::data::spells::CUSTOM_NEGATIVE;
    curse.effects[0].target_a = 6;
    insert(
        d,
        [
            buff(LESSER, 10),
            buff(GREATER, 20),
            buff(UNRELATED, 5),
            buff(FIRST, 1),
            buff(SECOND, 2),
            hidden,
            buff(PLAIN, 6),
            parent,
            buff(CHILD, 7),
            buff(RANK_1, 8),
            buff(RANK_2, 9),
            area,
            curse,
        ],
    );
}

/// What `g` wears of the made-up spells, in order.
fn worn(m: &Map, g: Guid) -> Vec<u32> {
    let mut v: Vec<u32> = auras(m, g).map_or(Vec::new(), |a| {
        a.holders
            .iter()
            .map(|h| h.spell)
            .filter(|s| (997_700..997_800).contains(s))
            .collect()
    });
    v.sort();
    v
}

/// A group's greater replaces its lesser from another caster, and its
/// lesser after is refused without refreshing it; a lesser in a second,
/// rule-1 group with another there is refused whole, the other kept;
/// death takes the greater and brings nothing back (Thistle Tea's
/// "ordered upgrades replace and reject downgrades without refreshing",
/// "rejection is atomic when another group would remove a different
/// holder", "death clears the replacement without restoring its
/// predecessor").
#[tokio::test]
async fn a_groups_greater_replaces_its_lesser_and_refuses_it_after() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SG1", "Buffed", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let one = kit::dummy(m, a, 60, 3.0);
            let two = kit::dummy(m, a, 60, 4.0);
            kit::apply(m, one, a, LESSER);
            kit::apply(m, two, a, GREATER);
            let upgraded = worn(m, a);
            let expires = holder(m, a, GREATER).and_then(|h| h.expires);
            kit::pass(m, 1_000);
            kit::apply(m, one, a, LESSER);
            let refused = (
                worn(m, a),
                holder(m, a, GREATER).and_then(|h| h.expires) == expires,
            );
            remove_where(m, a, |h| h.spell == GREATER);
            kit::apply(m, one, a, UNRELATED);
            kit::apply(m, two, a, GREATER);
            kit::apply(m, one, a, LESSER);
            let atomic = worn(m, a);
            crate::game::map::combat::deal_damage(m, one, a, 1_000_000);
            (upgraded, refused, atomic, worn(m, a))
        })
        .await
        .unwrap();
    let (upgraded, (refused, unrefreshed), atomic, dead) = out;
    assert_eq!(upgraded, vec![GREATER]);
    assert_eq!(refused, vec![GREATER]);
    assert!(unrefreshed, "not refreshed");
    assert_eq!(atomic, vec![GREATER, UNRELATED], "refused whole");
    assert_eq!(dead, Vec::<u32>::new());
}

/// The first common group with a rule decides (rule 0 says nothing, so a
/// rule-1 group's next: each replaces the other); a hidden holder stands
/// beside its group's other, as a parent beside the aura it triggers; two
/// ranks of one kind go by their ranks whatever their group says (Thistle
/// Tea's "the first non-default common group determines exclusivity",
/// "ordinary exclusivity preserves existing passive and hidden auras",
/// "same spells refresh and direct trigger pairs preserve their parent",
/// "ordinary group membership does not override same-chain rank
/// handling").
#[tokio::test]
async fn the_first_ruled_group_decides_and_hidden_and_triggered_stand_apart() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SG2", "Grouped", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let one = kit::dummy(m, a, 60, 3.0);
            let mut out = Vec::new();
            for spells in [
                [SECOND, FIRST],
                [FIRST, SECOND],
                [HIDDEN, PLAIN],
                [PARENT, CHILD],
                [CHILD, PARENT],
                [RANK_2, RANK_1],
            ] {
                remove_where(m, a, |h| !h.passive);
                for s in spells {
                    kit::apply(m, one, a, s);
                }
                out.push(worn(m, a));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![
            vec![FIRST],
            vec![SECOND],
            vec![HIDDEN, PLAIN],
            vec![PARENT, CHILD],
            vec![PARENT, CHILD],
            vec![RANK_2],
        ]
    );
}

/// A lesser buff cast at one who has the group's greater is refused
/// before it costs anything, at oneself or another; its area kind and a
/// harmful kind aren't, and are left to each they land on (Thistle Tea's
/// "positive single-target upgrades reject weaker casts on self and
/// others" and "area buffs and harmful debuffs defer conflicts to each
/// recipient").
#[tokio::test]
async fn a_lesser_buff_at_one_with_the_greater_is_refused_before_it_costs() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P7SG3", "Caster", Class::Priest).await;
    let (_d, b, _) = human(&realm, "P7SG4", "Friend", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[LESSER, LESSER_AREA, LESSER_CURSE]);
            let check = |m: &mut Map, spell: u32, at_unit: Guid| {
                let s = m.realm.data.spells.get(spell).unwrap().clone();
                checks::check(m, a, &s, &at(at_unit), None, false, &Mods::default())
            };
            spells::cast_triggered(m, a, GREATER, Some(a));
            spells::cast_triggered(m, a, GREATER, Some(b));
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, kobold, kobold, GREATER);
            (
                check(m, LESSER, a),
                check(m, LESSER, b),
                check(m, LESSER_AREA, a),
                check(m, LESSER_CURSE, kobold),
                has(m, b, GREATER),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (
            Some(fail::AURA_BOUNCED),
            Some(fail::AURA_BOUNCED),
            None,
            None,
            true
        )
    );
}
