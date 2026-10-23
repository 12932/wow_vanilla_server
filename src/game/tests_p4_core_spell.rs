//! Phase 3, round 4, against Thistle Tea's `spell_test.exs`: which spells
//! are hostile (instant kills among them), a finisher's duration by its
//! combo points, and a triggered spell's points fixed by its script.

use wow_world_messages::vanilla::*;

use crate::game::data::spells::{CUSTOM_NEGATIVE, CUSTOM_POSITIVE, SpellEffect, SpellTemplate};
use crate::game::map::auras::{self, AURA_MOD_STAT, positive};
use crate::game::map::spells;
use crate::game::tests_p3_combat::*;

fn one(effect: u32, aura: u32, target_a: u32, target_b: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id: 97_000,
        name: "Spell".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect,
        aura,
        target_a,
        target_b,
        base_points: 9,
        base_dice: 1,
        ..Default::default()
    };
    s
}

#[test]
fn instant_kills_are_hostile_but_on_the_caster_alone_and_vmangos_s_polarity_comes_first() {
    // Death Touch at any unit (25), at an enemy; one at the caster alone.
    assert!(!positive(&one(1, 0, 25, 0)));
    assert!(!positive(&one(1, 0, 6, 0)));
    assert!(positive(&one(1, 0, 1, 0)));
    // Demonic Sacrifice at the pet is vmangos's positive.
    let mut sacrifice = one(1, 0, 5, 0);
    sacrifice.custom_flags = CUSTOM_POSITIVE;
    assert!(positive(&sacrifice));
    // A Voidwalker's Sacrifice by its family.
    let mut voidwalker = one(1, 0, 5, 0);
    (voidwalker.family_name, voidwalker.family_flags) = (5, 0x0200_0000);
    assert!(positive(&voidwalker));
    // Negative at the caster; positive over negative.
    let mut marked = one(1, 0, 1, 0);
    marked.custom_flags = CUSTOM_NEGATIVE;
    assert!(!positive(&marked));
    marked.custom_flags = CUSTOM_NEGATIVE | CUSTOM_POSITIVE;
    assert!(positive(&marked));
}

#[test]
fn debuffs_at_enemies_are_hostile_and_heals_and_buffs_at_allies_are_not() {
    // An aura at an enemy, at the enemies about the caster or a point.
    assert!(!positive(&one(6, 4, 6, 0)));
    assert!(!positive(&one(6, 4, 15, 0)));
    assert!(!positive(&one(6, 4, 22, 15)));
    // School damage.
    assert!(!positive(&one(2, 0, 6, 0)));
    // A heal and a buff at an ally (21).
    assert!(positive(&one(10, 0, 21, 0)));
    assert!(positive(&one(6, AURA_MOD_STAT, 21, 0)));
}

/// An aura is judged by its target first, as mangos-classic's
/// `IsPositiveEffect` does: a pacify (25) or a stun (12) at a raid member
/// (57), a friend (21) or the caster (1) helps (Blessing of Protection,
/// Divine Intervention, Ice Block), unless the spell is marked a debuff or
/// vmangos's negative (Forbearance); at an enemy it harms, and so does
/// damage at a friend.
#[test]
fn an_aura_at_a_friend_helps_unless_marked_harmful() {
    for t in [1, 21, 57] {
        assert!(positive(&one(6, 25, t, 0)), "pacify at {t}");
        assert!(positive(&one(6, 12, t, 0)), "stun at {t}");
    }
    let mut debuff = one(6, 25, 21, 0);
    debuff.attributes[0] |= crate::game::data::spells::ATTR_DEBUFF;
    assert!(!positive(&debuff));
    let mut negative = one(6, 12, 1, 0);
    negative.custom_flags = CUSTOM_NEGATIVE;
    assert!(!positive(&negative));
    assert!(!positive(&one(6, 25, 6, 0)));
    // A friend's target and an enemy's: harmful.
    assert!(!positive(&one(6, 25, 21, 6)));
    assert!(!positive(&one(2, 0, 21, 0)));
}

#[test]
fn a_finishers_duration_climbs_a_fifth_a_point_to_its_most_at_five() {
    let d = |combo| spells::combo_duration(6_000, 16_000, combo);
    assert_eq!([1, 3, 5, 8].map(d), [8_000, 12_000, 16_000, 16_000]);
    assert_eq!(d(0), 6_000);
    // A spell whose most is no more than its base stays at its base.
    assert_eq!(spells::combo_duration(6_000, 0, 3), 6_000);
}

#[tokio::test]
async fn a_scripts_fixed_points_replace_the_triggered_spells_own() {
    let (_realm, _a, g, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let mut s = one(6, AURA_MOD_STAT, 1, 0);
            s.id = 97_001;
            s.effects[0].misc_value = 0;
            spells::scripts::cast_template_with_points(m, g, s, g, [Some(-27), None, None]);
            auras::auras(m, g).and_then(|a| {
                a.holders
                    .iter()
                    .find(|h| h.spell == 97_001)
                    .map(|h| h.effects.iter().map(|e| e.amount).collect::<Vec<_>>())
            })
        })
        .await
        .unwrap();
    assert_eq!(r, Some(vec![-27]));
}
