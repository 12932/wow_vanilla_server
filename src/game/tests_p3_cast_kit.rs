//! What Phase 3's casting tests share beyond the spell kit
//! (`tests_p3_spells_kit`): a realm on made-up data for the fast suite, with
//! a creature to cast at and the spells a test makes up.

use std::sync::Arc;

use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};

/// The made-up creature, a Kobold Vermin's entry so that the spell kit's
/// `dummy` finds it.
pub const KOBOLD: u32 = 6;

/// Cast time indexes: none, 2 seconds, 3, 3.5 and 10.
pub const INSTANT: u32 = 1;
pub const CAST_2S: u32 = 900;
pub const CAST_3S: u32 = 901;
pub const CAST_3500: u32 = 902;
pub const CAST_10S: u32 = 903;
/// Duration indexes: 5 seconds, 15, and two hours.
pub const LASTS_5S: u32 = 900;
pub const LASTS_15S: u32 = 901;
pub const LASTS_LONG: u32 = 21;

/// Made-up data: a creature to cast at, cast times and durations, and what
/// `f` adds.
pub fn data_with(f: impl FnOnce(&mut GameData)) -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        KOBOLD,
        CreatureTemplate {
            entry: KOBOLD,
            name: "Kobold Vermin".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 60,
            level_max: 60,
            faction: 7,
            ..Default::default()
        },
    );
    for (i, ms) in [
        (INSTANT, 0),
        (CAST_2S, 2_000),
        (CAST_3S, 3_000),
        (CAST_3500, 3_500),
        (CAST_10S, 10_000),
    ] {
        d.spells.cast_times.insert(i, ms);
    }
    for (i, ms) in [
        (LASTS_5S, 5_000),
        (LASTS_15S, 15_000),
        (LASTS_LONG, 7_200_000),
    ] {
        d.spells.durations.insert(
            i,
            SpellDuration {
                base: ms,
                per_level: 0,
                max: ms,
            },
        );
    }
    f(&mut d);
    d
}

/// A realm on [`data_with`]'s data.
pub fn fast_realm(f: impl FnOnce(&mut GameData)) -> Arc<Realm> {
    harness::realm_with(data_with(f))
}

/// The eastern kingdoms' map of `realm`, where a human starts.
pub fn world(realm: &Arc<Realm>) -> MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

/// `f` on a map of made-up data, with no one in it.
pub async fn on_map<T: Send + 'static>(
    f_data: impl FnOnce(&mut GameData),
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    let realm = fast_realm(f_data);
    world(&realm).query(f).await.expect("the map")
}

/// A human of `class` in a realm of made-up data, with their map.
pub async fn human(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    class: Class,
) -> (Client, wow_world_messages::Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Human, class).await;
    (c, g, world(realm))
}

/// A made-up spell with no effects: instant, at a unit, of `school`.
pub fn bare(id: u32, school: u32) -> SpellTemplate {
    SpellTemplate {
        id,
        name: format!("Made-up {id}"),
        school,
        dmg_class: 1,
        equipped_item_class: -1,
        range_index: 1,
        casting_time_index: INSTANT,
        effects: [SpellEffect {
            bonus_coefficient: -1.0,
            ..Default::default()
        }; 3],
        ..Default::default()
    }
}

/// Effect `i` of `s` as `e`, with no coefficient of its own (the formula's).
pub fn effect(s: &mut SpellTemplate, i: usize, e: SpellEffect) {
    s.effects[i] = SpellEffect {
        bonus_coefficient: if e.bonus_coefficient == 0.0 {
            -1.0
        } else {
            e.bonus_coefficient
        },
        ..e
    };
}

/// A damage effect (2) of `amount` at the unit target.
pub fn damage(amount: i32) -> SpellEffect {
    SpellEffect {
        effect: 2,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    }
}

/// An aura effect (6) of `aura` with `misc` and `amount`, at `target`.
pub fn aura(aura: u32, misc: i32, amount: i32, target: u32) -> SpellEffect {
    SpellEffect {
        effect: 6,
        aura,
        misc_value: misc,
        base_points: amount - 1,
        base_dice: 1,
        target_a: target,
        ..Default::default()
    }
}

/// Put the made-up spells in `d`.
pub fn insert(d: &mut GameData, spells: impl IntoIterator<Item = SpellTemplate>) {
    for s in spells {
        d.spells.spells.insert(s.id, s);
    }
}
