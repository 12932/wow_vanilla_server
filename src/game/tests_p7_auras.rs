//! Round 7's aura tests, ported from Thistle Tea's
//! `test/game/core/entity/appearance_test.exs` (docs/parity/matrix.csv):
//! a transform's size, reach and weapons, and what comes back after it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::DisplayInfo;
use crate::game::data::spells::{CUSTOM_POSITIVE, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;

/// Transforms into creatures 3_901 (display 100, at 0.8 of its size,
/// radius 0.5 and reach 2.0) and 3_902 (display 200), a growth by half,
/// and Ghost Wolf.
const INTO_SMALL: u32 = 97_001;
const INTO_PLAIN: u32 = 97_002;
const GROWTH: u32 = 97_010;
const GHOST_WOLF: u32 = 2645;
const SMALL: u32 = 3_901;
const PLAIN: u32 = 3_902;
/// The weapons: a sword of display 500, one of 600, and the gear the
/// small creature and the plain one hold (the plain one's empty hands).
const OWN_SWORD: u32 = 97_500;
const SMALL_SWORD: u32 = 97_600;
const SMALL_GEAR: u32 = 9_701;
const NO_GEAR: u32 = 9_702;

fn aura_spell(id: u32, aura: u32, misc: i32, points: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        // Helpful, as Thistle Tea's disguises: a player may cancel them.
        custom_flags: CUSTOM_POSITIVE,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura,
        misc_value: misc,
        base_points: points - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut small = template(SMALL, 7);
    small.display_ids = [100, 0, 0, 0];
    small.display_scales = [0.8, 0.0, 0.0, 0.0];
    small.equipment_id = SMALL_GEAR;
    d.creature_templates.insert(SMALL, small);
    let mut plain = template(PLAIN, 7);
    plain.display_ids = [200, 0, 0, 0];
    plain.equipment_id = NO_GEAR;
    d.creature_templates.insert(PLAIN, plain);
    d.display_info.insert(
        100,
        DisplayInfo {
            scale: 1.0,
            bounding_radius: 0.5,
            combat_reach: 2.0,
            ..Default::default()
        },
    );
    d.display_info.insert(
        200,
        DisplayInfo {
            scale: 1.0,
            bounding_radius: 0.389,
            combat_reach: 1.5,
            ..Default::default()
        },
    );
    for (entry, display) in [(OWN_SWORD, 500), (SMALL_SWORD, 600)] {
        let mut sword = d.items[&crate::game::tests_p3_combat::SWORD].clone();
        sword.entry = entry;
        sword.display_id = display;
        d.items.insert(entry, sword);
    }
    d.creature_equipment.insert(SMALL_GEAR, [SMALL_SWORD, 0, 0]);
    d.creature_equipment.insert(NO_GEAR, [0, 0, 0]);
    for s in [
        aura_spell(INTO_SMALL, auras::AURA_TRANSFORM, SMALL as i32, 0),
        aura_spell(INTO_PLAIN, auras::AURA_TRANSFORM, PLAIN as i32, 0),
        aura_spell(GROWTH, auras::AURA_MOD_SCALE, 0, 50),
        aura_spell(
            GHOST_WOLF,
            auras::AURA_MOD_SHAPESHIFT,
            auras::FORM_GHOST_WOLF as i32,
            0,
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn put(m: &mut Map, g: Guid, id: u32) {
    let s = m.realm.data.spells.spells[&id].clone();
    let points = crate::game::map::spells::points(&s, 0, 1);
    auras::apply_spell(m, g, g, &s, &[(0, points)], auras::Context::default()).expect("put on");
}

fn take(m: &mut Map, g: Guid, id: u32) {
    auras::remove_where(m, g, |h| h.spell == id);
}

/// Its display, size, bounding radius and combat reach.
fn looks(m: &Map, g: Guid) -> (u32, f32, f32, f32) {
    let f = m.unit_fields(g).expect("the unit");
    (
        f.get_u32(UNIT_FIELD_DISPLAYID),
        f.get_f32(OBJECT_FIELD_SCALE_X),
        f.get_f32(UNIT_FIELD_BOUNDINGRADIUS),
        f.get_f32(UNIT_FIELD_COMBATREACH),
    )
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// Thistle Tea's `Appearance` "scale and geometry are derived without
/// compounding and restore on removal": a player transformed into a
/// creature takes its size (0.8) and its model's radius and reach at that
/// size (0.4 and 1.6), a growth by half grows all three (1.2, 0.6, 2.4),
/// putting both on again changes nothing, and taking them off however
/// (removed, cancelled, by death) gives back the player's own.
#[tokio::test]
async fn a_transform_takes_its_creatures_size_and_reach_and_gives_them_back() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let own = looks(m, ga);
        let mut seen = Vec::new();
        for how in 0..3 {
            put(m, ga, INTO_SMALL);
            let small = looks(m, ga);
            put(m, ga, GROWTH);
            let grown = looks(m, ga);
            put(m, ga, INTO_SMALL);
            put(m, ga, GROWTH);
            let again = looks(m, ga);
            match how {
                0 => {
                    take(m, ga, GROWTH);
                    take(m, ga, INTO_SMALL);
                }
                1 => {
                    auras::cancel(m, ga, INTO_SMALL);
                    auras::cancel(m, ga, GROWTH);
                }
                _ => auras::clear_on_death(m, ga),
            }
            seen.push((small, grown, again, looks(m, ga)));
        }
        (own, seen)
    })
    .await;
    let (own, seen) = out;
    assert!(
        near(own.1, 1.0) && near(own.2, 0.389) && near(own.3, 1.5),
        "{own:?}"
    );
    for (how, (small, grown, again, back)) in seen.into_iter().enumerate() {
        assert_eq!(small.0, 100);
        assert!(
            near(small.1, 0.8) && near(small.2, 0.4) && near(small.3, 1.6),
            "{small:?}"
        );
        assert!(
            near(grown.1, 1.2) && near(grown.2, 0.6) && near(grown.3, 2.4),
            "{grown:?}"
        );
        assert_eq!(again, grown, "no compounding");
        assert_eq!(back, own, "taken off by {how}");
    }
}

/// Thistle Tea's `Appearance` "removing only the transform preserves
/// active growth": the transform gone, the growth stays on the player's
/// own size and reach (1.5 and 2.25), in its own look.
#[tokio::test]
async fn a_growth_outlasts_the_transform_it_was_put_on_with() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (own, after, back) = on(&map, move |m| {
        let own = looks(m, ga);
        put(m, ga, INTO_SMALL);
        put(m, ga, GROWTH);
        take(m, ga, INTO_SMALL);
        let after = looks(m, ga);
        take(m, ga, GROWTH);
        (own, after, looks(m, ga))
    })
    .await;
    assert_eq!(after.0, own.0, "its own look");
    assert!(near(after.1, 1.5) && near(after.3, 2.25), "{after:?}");
    assert!(near(after.2, 0.389 * 1.5), "{after:?}");
    assert_eq!(back, own);
}

/// Thistle Tea's `Appearance` "removing a transform restores the
/// shapeshift model and scale": a Ghost Wolf transformed shows the
/// transform, and the wolf again after it, at the size it had; out of the
/// form, the player's own. (The wolf keeps the player's size, as mangos's
/// `RestoreDisplayId` keeps a form's at the unit's own, where Thistle Tea
/// shrinks it to 0.8.)
#[tokio::test]
async fn a_transform_gone_shows_the_form_again_at_its_size() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (own, wolf, plain, again, back) = on(&map, move |m| {
        let own = looks(m, ga);
        put(m, ga, GHOST_WOLF);
        let wolf = looks(m, ga);
        put(m, ga, INTO_PLAIN);
        let plain = looks(m, ga);
        take(m, ga, INTO_PLAIN);
        let again = looks(m, ga);
        take(m, ga, GHOST_WOLF);
        (own, wolf, plain, again, looks(m, ga))
    })
    .await;
    assert_eq!(wolf.0, 4613);
    assert!(near(wolf.1, own.1), "{wolf:?}");
    assert_eq!(plain.0, 200);
    assert!(near(plain.1, 1.0), "{plain:?}");
    assert_eq!(again, wolf);
    assert_eq!(back, own);
}

/// Thistle Tea's `Appearance` "creature equipment follows the selected
/// transform and restores its template": a creature transformed takes the
/// weapons of the creature it shows (mangos's `HandleAuraTransform`
/// loading its equipment), empty hands among them, the one before's as the
/// newer goes, and its own back, every field, at the last.
#[tokio::test]
async fn a_creatures_weapons_follow_its_transform_and_come_back() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let seen = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let data = m.realm.data.clone();
        m.creature_mut(brute)
            .unwrap()
            .set_equipment(&data, [OWN_SWORD, 0, 0]);
        let weapons = |m: &Map| -> [u32; 9] {
            let f = m.unit_fields(brute).unwrap();
            std::array::from_fn(|i| f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i as u16))
        };
        let own = weapons(m);
        put(m, brute, INTO_SMALL);
        let small = weapons(m);
        put(m, brute, INTO_PLAIN);
        let plain = weapons(m);
        take(m, brute, INTO_PLAIN);
        let back_to_small = weapons(m);
        take(m, brute, INTO_SMALL);
        (own, small, plain, back_to_small, weapons(m))
    })
    .await;
    let (own, small, plain, back_to_small, back) = seen;
    assert_eq!(own[0], 500);
    assert_eq!(small[0], 600);
    assert_eq!(plain[..3], [0, 0, 0]);
    assert_eq!(back_to_small, small);
    assert_eq!(back, own, "every field its own");
}

/// Thistle Tea's `Capacity` "refreshing at capacity preserves the slot and
/// protects the refreshed aura": of 32 buffs, the oldest put on again
/// keeps its slot and is the newest, so a thirty-third takes the next
/// oldest's place instead.
#[tokio::test]
async fn a_buff_refreshed_at_capacity_keeps_its_slot_and_the_next_oldest_goes() {
    const FIRST: u32 = 97_301;
    const NEWCOMER: u32 = 97_400;
    let mut d = data();
    for id in (FIRST..FIRST + 32).chain([NEWCOMER]) {
        d.spells
            .spells
            .insert(id, aura_spell(id, auras::AURA_MOD_STAT, 3, 5));
    }
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let minute = auras::Context {
            duration: Some(Some(60_000)),
            ..Default::default()
        };
        let put_for = |m: &mut Map, id: u32| {
            let s = m.realm.data.spells.spells[&id].clone();
            auras::apply_spell(m, ga, ga, &s, &[(0, 5)], minute).expect("put on");
        };
        for id in FIRST..FIRST + 32 {
            put_for(m, id);
        }
        let slot = |m: &Map, id: u32| {
            auras::auras(m, ga)
                .and_then(|a| a.holders.iter().find(|h| h.spell == id))
                .and_then(|h| h.slot)
        };
        let first = slot(m, FIRST);
        crate::game::tests_p3_spells_kit::pass(m, 100);
        put_for(m, FIRST);
        crate::game::tests_p3_spells_kit::pass(m, 1);
        put_for(m, NEWCOMER);
        let shown = auras::auras(m, ga).map_or(0, |a| {
            a.holders
                .iter()
                .filter(|h| h.positive && h.slot.is_some())
                .count()
        });
        (
            first,
            slot(m, FIRST),
            slot(m, FIRST + 1),
            slot(m, NEWCOMER),
            shown,
        )
    })
    .await;
    let (first, refreshed, next, newcomer, shown) = out;
    assert_eq!(first, Some(0));
    assert_eq!(refreshed, Some(0), "the refreshed kept, in its slot");
    assert_eq!(next, None, "the next oldest gone");
    assert!(newcomer.is_some());
    assert_eq!(shown, 32);
}

/// Thistle Tea's `PercentManaRecovery` "restores underlying mana while
/// another resource is active": a druid in cat form, showing energy, gets
/// a tenth of its mana back a tick, its energy left as it was.
#[tokio::test]
async fn mana_recovery_restores_a_cat_druids_mana_not_its_energy() {
    const RECOVERY: u32 = 97_410;
    const CAT: u32 = 97_411;
    let mut d = data();
    let mut recovery = aura_spell(RECOVERY, auras::AURA_OBS_MOD_MANA, 0, 10);
    recovery.effects[0].amplitude = 1000;
    d.spells.spells.insert(RECOVERY, recovery);
    d.spells.spells.insert(
        CAT,
        aura_spell(CAT, auras::AURA_MOD_SHAPESHIFT, auras::FORM_CAT as i32, 0),
    );
    let realm = crate::game::harness::realm_with(d);
    let (_c, dr, map) = crate::game::tests_p3_spells_kit::player(
        &realm,
        "P7K1",
        "Catone",
        Race::NightElf,
        Class::Druid,
    )
    .await;
    let out = on(&map, move |m| {
        put(m, dr, CAT);
        // The aura on, then the pools (its going on works out the
        // druid's maximum again).
        put(m, dr, RECOVERY);
        let f = m.players.get_mut(&dr).unwrap();
        f.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
        f.fields.set_u32(UNIT_FIELD_POWER1, 100);
        f.fields.set_u32(UNIT_FIELD_POWER1 + 3, 20);
        let max = f.fields.get_u32(UNIT_FIELD_MAXPOWER1);
        crate::game::tests_p3_spells_kit::pass(m, 1000);
        let f = &m.players[&dr].fields;
        (
            f.get_byte(UNIT_FIELD_BYTES_0, 3),
            max,
            f.get_u32(UNIT_FIELD_POWER1),
            f.get_u32(UNIT_FIELD_POWER1 + 3),
        )
    })
    .await;
    let (shown, max, mana, energy) = out;
    assert_eq!(shown, 3, "energy shown");
    assert!(max > 100);
    assert_eq!(mana, 100 + max / 10, "a tenth of its mana");
    assert_eq!(energy, 20, "its energy as it was");
}

/// Thistle Tea's `PercentStat` "death-persistent penalties survive release
/// and resurrection until their original expiry": a sickness that lasts
/// through death (Resurrection Sickness's kind) can't be cancelled, stays
/// through dying, the release and the resurrection, and gives the stats
/// back at the end it always had, the health as it was.
#[tokio::test]
async fn a_sickness_lasts_through_death_and_release_to_its_own_end() {
    use crate::game::data::spells::{ATTR_CANT_CANCEL, ATTR_EX3_DEATH_PERSISTENT};
    const SICKNESS: u32 = 97_420;
    let mut d = data();
    let mut sickness = aura_spell(SICKNESS, auras::AURA_MOD_PERCENT_STAT, -1, -75);
    sickness.custom_flags = crate::game::data::spells::CUSTOM_NEGATIVE;
    sickness.attributes[0] = ATTR_CANT_CANCEL;
    sickness.attributes[3] = ATTR_EX3_DEATH_PERSISTENT;
    d.spells.spells.insert(SICKNESS, sickness);
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, ga, 60);
        let stamina = |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_STAT0 + 2);
        let own = stamina(m);
        let s = m.realm.data.spells.spells[&SICKNESS].clone();
        let ten = auras::Context {
            duration: Some(Some(10_000)),
            ..Default::default()
        };
        auras::apply_spell(m, ga, ga, &s, &[(0, -75)], ten).expect("put on");
        let sick = stamina(m);
        auras::cancel(m, ga, SICKNESS);
        let after_cancel = has(m, ga, SICKNESS);
        crate::game::map::death::kill(m, ga);
        let dead = (has(m, ga, SICKNESS), stamina(m));
        crate::game::map::death::release(m, ga);
        let released = has(m, ga, SICKNESS);
        crate::game::map::death::resurrect(m, ga, 0.5);
        let health = m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH);
        let raised = (has(m, ga, SICKNESS), stamina(m), health > 0);
        crate::game::tests_p3_spells_kit::pass(m, 9_999);
        let before_end = has(m, ga, SICKNESS);
        crate::game::tests_p3_spells_kit::pass(m, 1);
        let ended = (
            has(m, ga, SICKNESS),
            stamina(m),
            m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH),
        );
        (
            own,
            sick,
            after_cancel,
            dead,
            released,
            raised,
            health,
            before_end,
            ended,
        )
    })
    .await;
    let (own, sick, after_cancel, dead, released, raised, health, before_end, ended) = out;
    assert_eq!(sick, own / 4, "a quarter of its stamina");
    assert!(after_cancel, "not cancelled");
    assert_eq!(dead, (true, sick), "through death");
    assert!(released, "through the release");
    assert_eq!(raised, (true, sick, true), "through the resurrection");
    assert!(before_end, "to its own end");
    assert_eq!(
        ended,
        (false, own, health),
        "the stats back, the health as it was"
    );
}

fn has(m: &Map, g: Guid, id: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(id))
}

/// Thistle Tea's `ViewpointSync` "transfers a viewpoint between casters
/// and ignores refreshes": a bind sight from one caster that replaces
/// another's on the same unit hands the eyes over (the first looks from
/// itself again, the second from the unit), and one put on again by the
/// same caster leaves them where they are.
#[tokio::test]
async fn a_bind_sight_replaced_by_another_caster_hands_the_eyes_over() {
    const SIGHT: u32 = 97_430;
    let mut d = data();
    d.spells
        .spells
        .insert(SIGHT, aura_spell(SIGHT, auras::AURA_BIND_SIGHT, 0, 0));
    let (realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        let looks = |m: &Map| [ga, gb].map(|g| m.players[&g].fields.get_guid(PLAYER_FARSIGHT));
        let s = m.realm.data.spells.spells[&SIGHT].clone();
        let sight = |m: &mut Map, caster: Guid| {
            auras::apply_spell(m, caster, brute, &s, &[(0, 0)], auras::Context::default())
                .expect("put on");
            looks(m)
        };
        let first = sight(m, ga);
        let handed = sight(m, gb);
        let again = sight(m, gb);
        let holders = auras::auras(m, brute)
            .map_or(0, |a| a.holders.iter().filter(|h| h.spell == SIGHT).count());
        (brute, first, handed, again, holders)
    })
    .await;
    let (brute, first, handed, again, holders) = out;
    let zero = Guid::zero();
    assert_eq!(first, [brute, zero]);
    assert_eq!(handed, [zero, brute], "handed over");
    assert_eq!(again, [zero, brute], "a refresh leaves them");
    assert_eq!(holders, 1);
}

/// Thistle Tea's `CasterModifiers` "power drains and burns retain their
/// base resource conversion" and "periodic power burns retain resource
/// consumption without caster damage amplification": a burn, a drain and
/// a burn over time that ignore their caster's modifiers take their own
/// amount of mana, and a burn deals what it burned, however their caster's
/// spell modifiers, crit and damage done would have it (as a burn without
/// the attribute shows).
#[tokio::test]
async fn burns_and_drains_that_ignore_their_caster_take_their_own_amount() {
    use crate::game::data::spells::ATTR_EX3_IGNORE_CASTER_MODIFIERS;
    use crate::game::map::spells::{EFFECT_POWER_BURN, EFFECT_POWER_DRAIN, mods};
    const BURN: u32 = 97_440;
    const DRAIN: u32 = 97_441;
    const BURNING: u32 = 97_442;
    const PLAIN_BURN: u32 = 97_443;
    const MULTIPLIED: u32 = 97_444;
    const CRITS: u32 = 97_445;
    const STRONGER: u32 = 97_446;
    let mut d = data();
    let mage_spell = |id: u32, effect: u32, aura: u32, ignores: bool| {
        let mut s = aura_spell(id, aura, 0, 100);
        s.custom_flags = 0;
        s.family_name = 3;
        s.family_flags = 1;
        s.school = 6;
        s.range_index = 1;
        s.effects[0].effect = effect;
        s.effects[0].target_a = 6;
        s.effects[0].multiple_value = 1.0;
        s.effects[0].amplitude = 1000;
        if ignores {
            s.attributes[3] = ATTR_EX3_IGNORE_CASTER_MODIFIERS;
        }
        s
    };
    for s in [
        mage_spell(BURN, EFFECT_POWER_BURN, 0, true),
        mage_spell(DRAIN, EFFECT_POWER_DRAIN, 0, true),
        mage_spell(BURNING, 6, auras::AURA_POWER_BURN_MANA, true),
        mage_spell(PLAIN_BURN, EFFECT_POWER_BURN, 0, false),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    // The caster's: twice the multiplier on its family's spells, every
    // spell a crit, a hundred percent more damage.
    let mut multiplied = aura_spell(
        MULTIPLIED,
        auras::AURA_ADD_PCT_MODIFIER,
        mods::OP_MULTIPLE_VALUE as i32,
        100,
    );
    multiplied.family_name = 3;
    multiplied.effects[0].item_type = 1;
    d.spells.spells.insert(MULTIPLIED, multiplied);
    d.spells.spells.insert(
        CRITS,
        aura_spell(CRITS, auras::AURA_MOD_SPELL_CRIT_CHANCE, 0, 100),
    );
    d.spells.spells.insert(
        STRONGER,
        aura_spell(STRONGER, auras::AURA_MOD_DAMAGE_PERCENT_DONE, 0x7F, 100),
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Mage).await;
    let out = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, ga, 60);
        for (id, amount) in [(MULTIPLIED, 100), (CRITS, 100), (STRONGER, 100)] {
            let s = m.realm.data.spells.spells[&id].clone();
            auras::apply_spell(m, ga, ga, &s, &[(0, amount)], auras::Context::default())
                .expect("put on");
        }
        let fresh = |m: &mut Map| {
            let brute = spawn(m, ga, BRUTE, 60, 5.0);
            let f = m.unit_fields_mut(brute).unwrap();
            f.set_byte(UNIT_FIELD_BYTES_0, 3, 0);
            f.set_u32(UNIT_FIELD_MAXPOWER1, 500);
            f.set_u32(UNIT_FIELD_POWER1, 500);
            brute
        };
        let left = |m: &Map, g: Guid| {
            let f = m.unit_fields(g).unwrap();
            (
                f.get_u32(UNIT_FIELD_POWER1),
                100_000 - f.get_u32(UNIT_FIELD_HEALTH),
            )
        };
        let mut seen = Vec::new();
        for id in [BURN, DRAIN, PLAIN_BURN] {
            let brute = fresh(m);
            crate::game::map::spells::cast_triggered(m, ga, id, Some(brute));
            seen.push(left(m, brute));
        }
        let brute = fresh(m);
        let s = m.realm.data.spells.spells[&BURNING].clone();
        auras::apply_spell(m, ga, brute, &s, &[(0, 100)], auras::Context::default())
            .expect("put on");
        crate::game::tests_p3_spells_kit::pass(m, 1000);
        seen.push(left(m, brute));
        seen
    })
    .await;
    assert_eq!(out[0], (400, 100), "the burn: its own amount, as damage");
    assert_eq!(out[1], (400, 0), "the drain: its own amount, no harm");
    assert!(
        out[2].0 == 400 && out[2].1 > 100,
        "a plain burn's modified: {:?}",
        out[2]
    );
    assert_eq!(out[3], (400, 100), "the burn over time");
}
