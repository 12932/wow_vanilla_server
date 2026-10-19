//! Round 8's combat tests, disarm (Thistle Tea's `disarm_test.exs`): a
//! druid's paws are no weapon to lose, but leaving the form finds her
//! disarmed; a weapon's cast disarmed before it ends fails there; and an
//! ability free of a weapon strikes with her fists and her unarmed skill.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOD_DISARM, FORM_CAT};
use crate::game::map::combat::{self, Hand};
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{self as p3, BRUTE};

const MACE: u32 = 63_950;
const DISARM: u32 = 97_000;
/// A weapon's strike of a second and a half's cast (Slam's kind); a blow
/// of the weapon's damage that wants none (a fist's).
const SLAM: u32 = 97_001;
const PUNCH: u32 = 97_002;
const SKILL_UNARMED: u32 = 162;
const SKILL_MACES: u32 = 54;
const CAST_1500: u32 = 970;

fn data() -> GameData {
    let mut d = p3::data();
    let mut mace = ItemTemplate {
        entry: MACE,
        class: 2,
        subclass: 4,
        name: "Mace".into(),
        display_id: 1000,
        inventory_type: 21,
        delay: 3000,
        stackable: 1,
        ..Default::default()
    };
    mace.damages[0] = (70.0, 90.0, 0);
    d.items.insert(MACE, mace);
    // A druid's stats, for a warrior read as one.
    d.player_level_stats
        .insert((1, 11, 60), [140, 0, 20, 20, 20]);
    d.player_class_stats.insert((11, 60), (1000, 0));
    d.spells.cast_times.insert(CAST_1500, 1_500);
    let strike = |id: u32, class: i32| {
        let mut s = SpellTemplate {
            id,
            name: format!("Strike {id}"),
            range_index: 1,
            equipped_item_class: class,
            dmg_class: 2,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 58,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        };
        s
    };
    let mut slam = strike(SLAM, 2);
    slam.casting_time_index = CAST_1500;
    for s in [slam, strike(PUNCH, -1)] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

type Setup = (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    MapHandle,
);

async fn armed() -> Setup {
    let (realm, a, g, map) = p3::setup_with(data(), Class::Warrior).await;
    map.query(move |m| {
        p3::level(m, g, 60);
        p3::equip(m, g, MACE, 15);
        let p = m.players.get_mut(&g).unwrap();
        p.spells.extend([SLAM, PUNCH]);
        for s in [SKILL_UNARMED, SKILL_MACES] {
            crate::game::map::skills::learn_skill(p, s, 300);
        }
        p.skills_maxed = false;
        crate::game::map::skills::set_value(p, SKILL_MACES, 300);
        crate::game::map::skills::set_value(p, SKILL_UNARMED, 250);
        restat(m, g);
    })
    .await
    .unwrap();
    (realm, a, g, map)
}

fn restat(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().apply_stats(&data);
}

/// The main hand's damage and speed.
fn main_hand(m: &Map, g: Guid) -> (u32, u32, u32) {
    let f = &m.players[&g].fields;
    (
        f.get_f32(UNIT_FIELD_MINDAMAGE) as u32,
        f.get_f32(UNIT_FIELD_MAXDAMAGE) as u32,
        f.get_u32(UNIT_FIELD_BASEATTACKTIME),
    )
}

fn disarm(m: &mut Map, g: Guid) {
    p3::aura_of(m, g, DISARM, AURA_MOD_DISARM, 0, 0);
    restat(m, g);
}

/// Thistle Tea's "preserves feral damage and rechecks disarm when leaving
/// form": a druid in cat form keeps her paws' damage and second's swing
/// under a disarm; leaving the form, she fights with her fists, not her
/// mace.
#[tokio::test]
async fn a_cats_paws_are_kept_under_disarm_and_leaving_the_form_finds_her_fists() {
    let (_realm, _a, g, map) = armed().await;
    let r = map
        .query(move |m| {
            let mace = main_hand(m, g);
            // A druid, as derive reads her class, in cat form.
            m.players.get_mut(&g).unwrap().row.class = 11;
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, FORM_CAT);
            restat(m, g);
            let cat = main_hand(m, g);
            disarm(m, g);
            let cat_disarmed = main_hand(m, g);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            restat(m, g);
            let left = main_hand(m, g);
            auras::remove_spell(m, g, DISARM, None);
            restat(m, g);
            (mace, cat, cat_disarmed, left, main_hand(m, g))
        })
        .await
        .unwrap();
    let (mace, cat, cat_disarmed, left, rearmed) = r;
    assert_eq!(cat.2, 1_000, "a cat's second");
    assert_eq!(cat_disarmed, cat, "the paws kept");
    assert_eq!(left.2, 2_000, "fists' two seconds, not the mace's three");
    assert!(left.1 < mace.0, "fists' damage: {left:?} against {mace:?}");
    assert_eq!(rearmed.2, 3_000, "the mace back with the disarm gone");
}

/// Thistle Tea's "rejects a weapon cast disarmed during preparation", as
/// mangos's `Spell::cast` checks the items again at its end: a weapon's
/// strike of a second and a half, begun armed and disarmed before it ends,
/// fails there, and the brute takes nothing.
#[tokio::test]
async fn a_weapons_cast_disarmed_before_it_ends_fails_there() {
    let (_realm, _a, g, map) = armed().await;
    let r = map
        .query(move |m| {
            let brute = p3::spawn(m, g, BRUTE, 60, 3.0);
            let before = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            spells::cast(
                m,
                g,
                SLAM,
                Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
                None,
            );
            let began = m
                .caster(g)
                .and_then(|c| c.casting.as_ref())
                .map(|c| c.spell);
            m.now += Duration::from_millis(500);
            spells::update(m);
            disarm(m, g);
            m.now += Duration::from_millis(1_100);
            spells::update(m);
            let after = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            (
                began,
                m.caster(g).and_then(|c| c.casting.as_ref()).is_some(),
                before - after,
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(SLAM), "begun armed");
    assert_eq!((r.1, r.2), (false, 0), "over, and nothing dealt");
}

/// Thistle Tea's "snapshots unarmed damage and skill for allowed melee
/// abilities": a disarmed player's blow that wants no weapon strikes with
/// her fists' damage, her unarmed skill rolling it, not the mace's.
#[tokio::test]
async fn a_disarmed_players_free_blow_is_her_fists_and_her_unarmed_skill() {
    let (_realm, _a, g, map) = armed().await;
    let r = map
        .query(move |m| {
            let brute = p3::spawn(m, g, BRUTE, 60, 3.0);
            let armed_skill = combat::roll_for(m, g, brute, Hand::Main)
                .unwrap()
                .attacker_skill;
            disarm(m, g);
            let fists = main_hand(m, g);
            let skill = combat::roll_for(m, g, brute, Hand::Main)
                .unwrap()
                .attacker_skill;
            let before = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            spells::cast_now(
                m,
                g,
                PUNCH,
                Targets {
                    unit: Some(brute),
                    ..Default::default()
                },
            );
            let dealt = before - m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            (armed_skill, skill, fists, dealt)
        })
        .await
        .unwrap();
    let (armed_skill, skill, fists, dealt) = r;
    assert_eq!(
        (armed_skill, skill),
        (300, 250),
        "(the mace's skill, unarmed)"
    );
    assert_eq!(fists.2, 2_000, "fists' two seconds");
    // The fists' damage and the effect's own point.
    assert!(
        (fists.0 + 1..=fists.1 + 1).contains(&dealt),
        "{dealt} within the fists' {fists:?} and a point"
    );
}
