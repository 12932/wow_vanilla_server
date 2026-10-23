//! A creature's victim when one on its threat list is immune to its blows
//! (your rule; mangos-classic's `STATE_SUPPRESSED`): with anyone else on
//! its list it fights them, the immune one's threat kept, and comes back
//! once that threat moves after the immunity; alone on its list, the
//! immune one stays its victim. Divine Shield, Ice Block, and Blessing of
//! Protection against a physical creature only; a taunt holds over it;
//! among the rest the tenths still decide; a script's second on the list
//! is the second of those it may hurt. And Drek'Thar, on the real data.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, AURA_MOD_STUN, AURA_MOD_TAUNT, AURA_SCHOOL_IMMUNITY};
use crate::game::map::{Map, MapHandle, ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, HELPER, START, data as combat_data, spawn, template};

/// Divine Shield's immunities (642): physical, and every other school.
const SHIELD: u32 = 95_900;
/// Blessing of Protection's (1022): physical alone.
const PROTECTION: u32 = 95_901;
/// Ice Block's (11958): a stun, and every school.
const ICE_BLOCK: u32 = 95_902;
/// A brute whose blows are arcane.
const ARCANE_BRUTE: u32 = 3790;

fn immunity(id: u32, name: &str, effects: &[(u32, i32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: name.into(),
        range_index: 1,
        duration_index: 901,
        equipped_item_class: -1,
        ..Default::default()
    };
    for (i, (aura, misc)) in effects.iter().enumerate() {
        s.effects[i] = SpellEffect {
            effect: 6,
            aura: *aura,
            misc_value: *misc,
            target_a: 1,
            ..Default::default()
        };
    }
    s
}

fn data() -> GameData {
    let mut d = combat_data();
    d.spells.durations.insert(
        901,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    for s in [
        immunity(
            SHIELD,
            "Shield",
            &[(AURA_SCHOOL_IMMUNITY, 1), (AURA_SCHOOL_IMMUNITY, 126)],
        ),
        immunity(PROTECTION, "Protection", &[(AURA_SCHOOL_IMMUNITY, 1)]),
        immunity(
            ICE_BLOCK,
            "Ice Block",
            &[
                (AURA_MOD_STUN, 0),
                (AURA_SCHOOL_IMMUNITY, 1),
                (AURA_SCHOOL_IMMUNITY, 126),
            ],
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let mut arcane = template(ARCANE_BRUTE, 7);
    arcane.damage_school = 6;
    d.creature_templates.insert(ARCANE_BRUTE, arcane);
    d
}

/// Alice and Bob, side by side where the combat tests stand.
async fn two() -> (Client, Client, Guid, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let alice = a.enter("Alice", Race::Human, Class::Paladin).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let map = realm.registry.online(alice).unwrap().map();
    (a, b, alice, bob, map)
}

/// A creature of `entry` fighting `victim`, `list` its threat, hitting for
/// next to nothing.
fn fighting(m: &mut Map, near: Guid, entry: u32, victim: Guid, list: &[(Guid, f32)]) -> Guid {
    let g = spawn(m, near, entry, 20, 1.5);
    let c = m.creature_mut(g).unwrap();
    c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 1.0);
    c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 1.0);
    c.combat.victim = victim;
    c.combat.attacking = true;
    c.fields.set_guid(UNIT_FIELD_TARGET, victim);
    c.threat = list.to_vec();
    m.note_fighting(g);
    g
}

/// `spell` on `g`, as its own.
fn put_on(m: &mut Map, g: Guid, spell: u32) {
    crate::game::map::spells::cast_triggered(m, g, spell, Some(g));
}

fn victim(m: &Map, g: Guid) -> Guid {
    m.creature(g).map_or(Guid::zero(), |c| c.combat.victim)
}

fn threat(m: &Map, creature: Guid, g: Guid) -> Option<f32> {
    m.creature(creature)?
        .threat
        .iter()
        .find(|(t, _)| *t == g)
        .map(|(_, v)| *v)
}

/// Wait until `f` holds of the map, a few seconds at most.
async fn until(map: &MapHandle, what: &str, f: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let g = f.clone();
        if map.query(move |m| g(m)).await.unwrap_or(false) {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Alice, top of the list, casts her shield: the creature turns to Bob at
/// its next look, her threat kept; while it lasts, her threat rising
/// doesn't bring it back. Her shield gone, it stays on Bob until her threat
/// moves (mangos's `m_suppresabilityToggle`), and then, well past his, she
/// is its victim again.
#[tokio::test]
async fn a_creature_turns_from_a_shielded_top_threat_to_the_next_and_back_once_her_threat_moves() {
    let (a, _b, alice, bob, map) = two().await;
    let brute = map
        .query(move |m| {
            m.players.get_mut(&alice).unwrap().spells.push(SHIELD);
            fighting(m, alice, BRUTE, alice, &[(alice, 100.0), (bob, 50.0)])
        })
        .await
        .unwrap();
    // Before the shield it fights her.
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    assert_eq!(map.query(move |m| victim(m, brute)).await.unwrap(), alice);
    a.send(CMSG_CAST_SPELL {
        spell: SHIELD,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
    until(&map, "the creature turns to Bob", move |m| {
        victim(m, brute) == bob
            && m.creature(brute)
                .is_some_and(|c| c.fields.get_guid(UNIT_FIELD_TARGET) == bob)
    })
    .await;
    let (kept, shielded_raise) = map
        .query(move |m| {
            let kept = threat(m, brute, alice);
            ai::add_threat(m, brute, alice, 500.0);
            ai::update_creatures(m);
            (kept, victim(m, brute))
        })
        .await
        .unwrap();
    assert_eq!(kept, Some(100.0), "her threat is kept");
    assert_eq!(
        shielded_raise, bob,
        "shielded, her threat doesn't bring it back"
    );
    let (after_shield, after_threat) = map
        .query(move |m| {
            auras::remove_spell(m, alice, SHIELD, None);
            ai::update_creatures(m);
            ai::update_creatures(m);
            let after_shield = victim(m, brute);
            ai::add_threat(m, brute, alice, 1.0);
            ai::update_creatures(m);
            (after_shield, victim(m, brute))
        })
        .await
        .unwrap();
    assert_eq!(
        after_shield, bob,
        "her threat unmoved since, it stays on Bob"
    );
    assert_eq!(
        after_threat, alice,
        "her threat moved, she is its victim again"
    );
}

/// Alone on its list, the shielded one stays its victim: it neither goes
/// home nor forgets her.
#[tokio::test]
async fn a_lone_immune_player_stays_the_creatures_victim() {
    let (_a, _b, alice, _bob, map) = two().await;
    let (still, evading, kept) = map
        .query(move |m| {
            let brute = fighting(m, alice, BRUTE, alice, &[(alice, 100.0)]);
            put_on(m, alice, SHIELD);
            for _ in 0..3 {
                ai::update_creatures(m);
            }
            let c = m.creature(brute).unwrap();
            (
                c.combat.attacking && c.combat.victim == alice,
                c.evading,
                threat(m, brute, alice),
            )
        })
        .await
        .unwrap();
    assert!(still);
    assert!(!evading);
    assert_eq!(kept, Some(100.0));
}

/// Blessing of Protection keeps off physical blows: a brute turns from
/// her, one whose blows are arcane doesn't. Ice Block, as the shield.
#[tokio::test]
async fn protection_turns_a_physical_creature_away_but_not_an_arcane_one_and_ice_block_turns_both()
{
    let (_a, _b, alice, bob, map) = two().await;
    let got = map
        .query(move |m| {
            let list = [(alice, 100.0), (bob, 50.0)];
            put_on(m, alice, PROTECTION);
            let physical = fighting(m, alice, BRUTE, alice, &list);
            let arcane = fighting(m, alice, ARCANE_BRUTE, alice, &list);
            ai::update_creatures(m);
            let protected = (victim(m, physical), victim(m, arcane));
            auras::remove_spell(m, alice, PROTECTION, None);
            put_on(m, alice, ICE_BLOCK);
            let physical = fighting(m, alice, BRUTE, alice, &list);
            let arcane = fighting(m, alice, ARCANE_BRUTE, alice, &list);
            ai::update_creatures(m);
            (protected, (victim(m, physical), victim(m, arcane)))
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        (bob, alice),
        "protection: only physical blows turn away"
    );
    assert_eq!(got.1, (bob, bob), "ice block: every school");
}

/// Among those it may hurt, the tenths still decide: from a shielded top
/// it takes the next at once, and keeps them until another passes them by
/// three tenths (out of melee) — the shielded one never, however high. A
/// taunt holds over the shield (mangos sorts by taunt first).
#[tokio::test]
async fn among_the_rest_the_tenths_decide_and_a_taunt_holds_over_immunity() {
    let (_a, _b, alice, _bob, map) = two().await;
    let picks = map
        .query(move |m| {
            let near = spawn(m, alice, HELPER, 20, 1.5);
            let far = spawn(m, alice, HELPER, 20, 25.0);
            let shielded = spawn(m, alice, HELPER, 20, 1.5);
            put_on(m, shielded, SHIELD);
            let list = [(shielded, 500.0), (near, 100.0), (far, 120.0)];
            let brute = fighting(m, alice, BRUTE, shielded, &list);
            let mut picks = Vec::new();
            // Off the shielded one at once, to the top of the rest.
            picks.push(ai::overtake(m, brute, shielded) == far);
            // On `near`, `far` under three tenths more and out of reach:
            // it stays.
            picks.push(ai::overtake(m, brute, near) == near);
            // Past three tenths: `far`.
            m.creature_mut(brute).unwrap().threat[2].1 = 131.0;
            picks.push(ai::overtake(m, brute, near) == far);
            // With no victim yet, the top of those it may hurt.
            picks.push(ai::overtake(m, brute, Guid::zero()) == far);
            // A taunt by the shielded one holds over it.
            let mut taunt = SpellTemplate {
                id: 95_903,
                name: "Taunt".into(),
                duration_index: 901,
                equipped_item_class: -1,
                ..Default::default()
            };
            taunt.effects[0] = SpellEffect {
                effect: 6,
                aura: AURA_MOD_TAUNT,
                target_a: 6,
                ..Default::default()
            };
            let _ = auras::apply_spell(m, shielded, brute, &taunt, &[(0, 0)], Default::default());
            picks.push(ai::overtake(m, brute, near) == shielded);
            picks
        })
        .await
        .unwrap();
    assert_eq!(picks, vec![true; 5]);
}

/// A script's second on the list (vmangos's target type 2, mangos's
/// `ATTACKING_TARGET_TOPAGGRO` at 1) walks the list as the creature sorts
/// it, those it may hurt first: with the shielded top below the rest, the
/// second is the second of those.
#[tokio::test]
async fn a_scripts_second_on_the_list_is_the_second_of_those_it_may_hurt() {
    let (_a, _b, alice, bob, map) = two().await;
    let (before, after, third) = map
        .query(move |m| {
            let third = spawn(m, alice, HELPER, 20, 1.5);
            let brute = fighting(
                m,
                alice,
                BRUTE,
                alice,
                &[(alice, 100.0), (bob, 50.0), (third, 20.0)],
            );
            let step = ScriptStep {
                target_type: 2,
                ..Default::default()
            };
            let second =
                |m: &Map| scripts::resolve_targets(m, &step, brute, Guid::zero()).map(|t| t.1);
            let before = second(m);
            put_on(m, alice, SHIELD);
            (before, second(m), third)
        })
        .await
        .unwrap();
    assert_eq!(before, Some(bob));
    assert_eq!(
        after,
        Some(third),
        "Bob tops those it may hurt; the second is next"
    );
}
