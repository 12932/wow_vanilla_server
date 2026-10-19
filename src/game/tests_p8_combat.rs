//! Round 8's combat tests, ported from Thistle Tea's `test/game/core/combat`
//! (docs/parity/matrix.csv): a spell's procs once a target.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::{ItemSpell, ItemTemplate};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::inventory::SLOT_MAIN_HAND;
use crate::game::map::spells::Targets;
use crate::game::map::{Map, auras, enchant};
use crate::game::tests_p3_combat::{self as p3, BRUTE};

/// A point of damage, the weapon's chance on hit.
const POINT: u32 = 96_000;
/// A melee ability wanting a weapon: two effects of ten damage each; one
/// that only does a dummy; one of ten damage and a harmful aura.
const TWICE: u32 = 96_001;
const DUMMY: u32 = 96_002;
const MARKS: u32 = 96_003;
const BLADE: u32 = 63_800;

fn ability(id: u32) -> SpellTemplate {
    SpellTemplate {
        id,
        name: format!("Ability {id}"),
        range_index: 1,
        equipped_item_class: 2,
        dmg_class: 2,
        ..Default::default()
    }
}

fn effect(effect: u32, points: i32) -> SpellEffect {
    SpellEffect {
        effect,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    }
}

pub(crate) fn data() -> GameData {
    let mut d = p3::data();
    let mut point = SpellTemplate {
        id: POINT,
        name: "Point".into(),
        range_index: 1,
        equipped_item_class: -1,
        proc_chance: 50,
        ..Default::default()
    };
    point.effects[0] = effect(2, 1);
    let mut twice = ability(TWICE);
    twice.effects[0] = effect(2, 10);
    twice.effects[1] = effect(2, 10);
    let mut dummy = ability(DUMMY);
    dummy.effects[0] = effect(3, 0);
    let mut marks = ability(MARKS);
    marks.effects[0] = effect(2, 10);
    marks.effects[1] = SpellEffect {
        effect: 6,
        aura: auras::AURA_MOD_DECREASE_SPEED,
        ..effect(6, -50)
    };
    marks.duration_index = 1;
    d.spells.durations.insert(
        1,
        crate::game::data::spells::SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    for s in [point, twice, dummy, marks] {
        d.spells.spells.insert(s.id, s);
    }
    let mut blade = ItemTemplate {
        entry: BLADE,
        class: 2,
        subclass: 7,
        name: "Blade".into(),
        display_id: 1000,
        inventory_type: 13,
        delay: 2000,
        stackable: 1,
        ..Default::default()
    };
    blade.spells[0] = ItemSpell {
        id: POINT,
        trigger: 2,
        ..Default::default()
    };
    d.items.insert(BLADE, blade);
    d
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// What `spell`, cast by `g` at `victim` with the weapon's chance sure,
/// takes off `victim`'s health.
fn cast(m: &mut Map, g: Guid, victim: Guid, spell: u32) -> u32 {
    let before = health(m, victim);
    enchant::FORCED_PROC_ROLL.with(|f| f.set(Some(0.0)));
    crate::game::map::spells::cast_now(
        m,
        g,
        spell,
        Targets {
            unit: Some(victim),
            ..Default::default()
        },
    );
    enchant::FORCED_PROC_ROLL.with(|f| f.set(None));
    if let Some(c) = m.caster_mut(g) {
        c.gcd_until = None;
        c.cooldowns.clear();
    }
    before.saturating_sub(health(m, victim))
}

/// Thistle Tea's "a successful multi-effect spell reports one weapon hit
/// and a non-damaging hit still reports", and mangos's
/// `DoAllEffectOnTarget` (one `ProcDamageAndSpell` a target): an ability
/// of two effects of damage sets off the weapon's chance on hit once, as
/// does one of damage and a harmful aura; one that does no harm at all
/// sets it off too; one that the target is immune to, never.
#[tokio::test]
async fn a_spell_sets_off_its_weapons_procs_once_a_target_whatever_its_effects() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            p3::equip(m, ga, BLADE, SLOT_MAIN_HAND);
            let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
            let twice = cast(m, ga, brute, TWICE);
            let marks = cast(m, ga, brute, MARKS);
            let dummy = cast(m, ga, brute, DUMMY);
            p3::aura_of(m, brute, 96_100, auras::AURA_SCHOOL_IMMUNITY, 127, 0);
            let immune = cast(m, ga, brute, TWICE);
            (twice, marks, dummy, immune)
        })
        .await
        .unwrap();
    assert_eq!(r.0, 21, "twenty, and the weapon's point once");
    assert_eq!(r.1, 11, "ten and a slow, and the point once");
    assert_eq!(r.2, 1, "no harm, and the point");
    assert_eq!(r.3, 0, "immune: nothing");
}
