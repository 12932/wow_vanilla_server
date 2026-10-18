//! Phase 3's combat tests: silence and pacify (Thistle Tea's
//! `combat_control_test.exs`): their flags while any of their auras lasts,
//! what each refuses by its prevention type, the casts a silence cuts
//! short, a silenced unit's swings, and a death taking them off.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{
    self, AURA_MOD_PACIFY, AURA_MOD_PACIFY_SILENCE, AURA_MOD_SILENCE, UNIT_FLAG_PACIFIED,
    UNIT_FLAG_SILENCED,
};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::spells::checks::fail;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, aura_of, forced_swing, spawn};

/// Spells of each prevention type, cast at once and over two seconds.
const NONE: u32 = 95_900;
const SILENCEABLE: u32 = 95_901;
const PACIFIABLE: u32 = 95_902;
const SLOW_NONE: u32 = 95_910;
const SLOW_SILENCEABLE: u32 = 95_911;
const SLOW_PACIFIABLE: u32 = 95_912;

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.cast_times.insert(900, 2000);
    for (id, prevention, slow) in [
        (NONE, 0, false),
        (SILENCEABLE, 1, false),
        (PACIFIABLE, 2, false),
        (SLOW_NONE, 0, true),
        (SLOW_SILENCEABLE, 1, true),
        (SLOW_PACIFIABLE, 2, true),
    ] {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 1,
            equipped_item_class: -1,
            prevention_type: prevention,
            casting_time_index: if slow { 900 } else { 0 },
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

async fn world() -> (crate::game::harness::Client, Guid, MapHandle) {
    let realm = crate::game::harness::realm_with(data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        for s in [
            NONE,
            SILENCEABLE,
            PACIFIABLE,
            SLOW_NONE,
            SLOW_SILENCEABLE,
            SLOW_PACIFIABLE,
        ] {
            spells::learn(m, g, s);
        }
    })
    .await
    .unwrap();
    (a, g, map)
}

fn flags(m: &Map, g: Guid) -> (bool, bool) {
    let f = m.unit_fields(g).unwrap();
    (
        f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SILENCED),
        f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PACIFIED),
    )
}

fn me(g: Guid) -> Targets {
    Targets {
        unit: Some(g),
        ..Default::default()
    }
}

#[tokio::test]
async fn silence_and_pacify_flags_last_while_any_of_their_auras_does_and_end_with_a_death() {
    let (_a, g, map) = world().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // Both, then each: the both goes first.
            aura_of(m, g, 95_920, AURA_MOD_PACIFY_SILENCE, 0, 0);
            aura_of(m, g, 95_921, AURA_MOD_PACIFY, 0, 0);
            aura_of(m, g, 95_922, AURA_MOD_SILENCE, 0, 0);
            out.push(flags(m, g));
            for id in [95_920, 95_921, 95_922] {
                auras::remove_spell(m, g, id, None);
                out.push(flags(m, g));
            }
            // Each, then both: the each go first.
            aura_of(m, g, 95_921, AURA_MOD_PACIFY, 0, 0);
            aura_of(m, g, 95_922, AURA_MOD_SILENCE, 0, 0);
            aura_of(m, g, 95_920, AURA_MOD_PACIFY_SILENCE, 0, 0);
            auras::remove_spell(m, g, 95_921, None);
            auras::remove_spell(m, g, 95_922, None);
            out.push(flags(m, g));
            // Dead: neither.
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            combat::deal_damage(m, brute, g, 1_000_000);
            out.push(flags(m, g));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (true, true),
            (true, true),
            (true, false),
            (false, false),
            (true, true),
            (false, false)
        ]
    );
}

#[tokio::test]
async fn each_control_refuses_only_what_its_prevention_type_says_and_a_silence_cuts_only_those_casts_short()
 {
    let (_a, g, map) = world().await;
    let got = map
        .query(move |m| {
            let check = |m: &mut Map, s: u32| {
                let t = m.realm.data.spells.get(s).unwrap().clone();
                crate::game::tests_p3_spells_kit::fresh(m, g);
                spells::checks::check(m, g, &t, &me(g), None, false, &Default::default())
            };
            let mut refused = Vec::new();
            for (id, kind) in [
                (95_920, AURA_MOD_PACIFY_SILENCE),
                (95_921, AURA_MOD_PACIFY),
                (95_922, AURA_MOD_SILENCE),
            ] {
                aura_of(m, g, id, kind, 0, 0);
                refused.push([NONE, SILENCEABLE, PACIFIABLE].map(|s| check(&mut *m, s)));
                auras::remove_spell(m, g, id, None);
            }
            // Under way: a silence cuts short only what it prevents, a
            // pacify nothing.
            let mut cut = Vec::new();
            for (id, kind) in [
                (95_922, AURA_MOD_SILENCE),
                (95_920, AURA_MOD_PACIFY_SILENCE),
                (95_921, AURA_MOD_PACIFY),
            ] {
                for s in [SLOW_NONE, SLOW_SILENCEABLE, SLOW_PACIFIABLE] {
                    crate::game::tests_p3_spells_kit::fresh(m, g);
                    spells::cast(m, g, s, me(g), None);
                    let began = crate::game::tests_p3_spells_kit::casting(m, g) == Some(s);
                    aura_of(m, g, id, kind, 0, 0);
                    let going = crate::game::tests_p3_spells_kit::casting(m, g) == Some(s);
                    auras::remove_spell(m, g, id, None);
                    cut.push((began, going));
                }
            }
            crate::game::tests_p3_spells_kit::fresh(m, g);
            (refused, cut)
        })
        .await
        .unwrap();
    let (refused, cut) = got;
    assert_eq!(
        refused,
        [
            [None, Some(fail::SILENCED), Some(fail::PACIFIED)],
            [None, None, Some(fail::PACIFIED)],
            [None, Some(fail::SILENCED), None],
        ]
    );
    assert!(cut.iter().all(|c| c.0), "each cast began");
    assert_eq!(
        cut.iter().map(|c| c.1).collect::<Vec<_>>(),
        [true, false, true, true, false, true, true, true, true],
        "silence and both cut the silenceable short; pacify cuts nothing"
    );
}

#[tokio::test]
async fn a_silenced_unit_still_swings() {
    let (_a, g, map) = world().await;
    let hurt = map
        .query(move |m| {
            let brute = spawn(m, g, BRUTE, 20, 1.0);
            aura_of(m, brute, 95_922, AURA_MOD_SILENCE, 0, 0);
            let f = &mut m.players.get_mut(&g).unwrap().fields;
            f.set_u32(UNIT_FIELD_RESISTANCES, 0);
            f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
            f.set_u32(UNIT_FIELD_HEALTH, 100_000);
            forced_swing(m, brute, g, Outcome::Hit);
            100_000 - m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH)
        })
        .await
        .unwrap();
    assert_eq!(hurt, 100);
}
