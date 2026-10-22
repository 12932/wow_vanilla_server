//! Round 8's pets (Thistle Tea's `control_owner_test.exs` and
//! `player_charm_test.exs`): whose side a unit's blows and spells are, and
//! how a player a creature charms keeps her foe and stands rooted.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::pets::Kind;
use crate::game::map::player::UNIT_FLAG_PVP;
use crate::game::map::{Map, MapKey, combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE, HELPER};
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{creature_at, in_combat, off};
use crate::game::tests_p8_pets_owner::{GUARD, TOTEM, hunter, summon};

/// A mage's bolt of a second and a half, at thirty yards.
const BOLT: u32 = 990_880;
const BOLT_TIME: u32 = 99;
/// Its instant.
const BLAST: u32 = 990_882;
/// A charm of many (`SPELL_AURA_AOE_CHARM`), as Chains of Kel'Thuzad.
const CHAINS: u32 = 990_883;

fn data() -> GameData {
    let mut d = crate::game::tests_p8_pets_owner::data();
    d.spells.cast_times.insert(BOLT_TIME, 1500);
    let mut bolt: SpellTemplate = crate::game::tests_pets::spell(
        BOLT,
        "Fireball",
        [
            crate::game::tests_pets::effect(2, 9, 6),
            Default::default(),
            Default::default(),
        ],
    );
    bolt.family_name = 3;
    bolt.spell_level = 1;
    bolt.mana_cost = 10;
    bolt.school = 2;
    let mut chains = crate::game::tests_pets::spell(
        CHAINS,
        "Chains",
        [
            crate::game::tests_pets::effect(6, 0, 6),
            Default::default(),
            Default::default(),
        ],
    );
    chains.effects[0].aura = crate::game::map::auras::AURA_AOE_CHARM;
    d.spells.spells.insert(CHAINS, chains);
    let mut blast = bolt.clone();
    blast.id = BLAST;
    blast.name = "Fire Blast".into();
    bolt.casting_time_index = BOLT_TIME;
    d.spells.spells.insert(BOLT, bolt);
    d.spells.spells.insert(BLAST, blast);
    d
}

/// Thistle Tea's "combat snapshots follow the current controller for every
/// companion kind" and "a creator does not imply control": a blow of the
/// hunter's pet, totem, guardian or the creature she charms is hers, its
/// foe tapped for her; a creature's own blow is no one's, and so is one's
/// that only names her its creator, which she may fight.
#[tokio::test]
async fn a_units_blow_is_its_controllers_and_not_its_creators() {
    let (_realm, _h, gh, pet, map) = hunter().await;
    let r = map
        .query(move |m| {
            let tap = |m: &mut Map, by: Guid| {
                put(m, by, off(-2.0, 0.0, 0.0));
                let b = creature_at(m, gh, BRUTE, off(-3.0, 0.0, 0.0));
                // A blow that hurts: only damage taps.
                combat::deal_damage(m, by, b, 1);
                let t = m.creature(b).unwrap().tapped_by;
                m.remove_creature(b);
                for u in [gh, by] {
                    combat::leave_combat(m, u);
                }
                t
            };
            let pet = tap(m, pet);
            let totem = summon(m, gh, TOTEM, Kind::Totem, off(-2.0, 0.0, 0.0));
            let totem = tap(m, totem);
            let guardian = summon(m, gh, GUARD, Kind::Guardian, off(-2.0, 0.0, 0.0));
            let guardian = tap(m, guardian);
            let own = creature_at(m, gh, HELPER, off(-2.0, 0.0, 0.0));
            let own = tap(m, own);
            let made = creature_at(m, gh, HELPER, off(-2.0, 0.0, 0.0));
            m.creature_mut(made)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_CREATEDBY, gh);
            let made_side = crate::game::map::pets::master(m, made) == made;
            let made = tap(m, made);
            let charmed = creature_at(m, gh, HELPER, off(-2.0, 0.0, 0.0));
            crate::game::map::pets::charmed(m, gh, charmed, false);
            let charmed = tap(m, charmed);
            ([pet, totem, guardian, charmed], [own, made], made_side, gh)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        [Some(r.3); 4],
        "the pet's, totem's, guardian's, charmed's"
    );
    assert_eq!(r.1, [None; 2], "a creature's own, and one she made");
    assert!(r.2, "a creator doesn't control");
}

/// Ann, a level-60 mage at the start, and her friend Bob `bob_at` yards
/// east of her, both in the world of [`data`].
async fn two(
    bob_at: f32,
) -> (
    std::sync::Arc<crate::game::Realm>,
    [crate::game::harness::Client; 2],
    Guid,
    Guid,
    crate::game::map::MapHandle,
) {
    let realm = crate::game::harness::realm_with(data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let ga = a.enter("Ann", Race::Human, Class::Mage).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    map.query(move |m| {
        for g in [ga, gb] {
            p3::level(m, g, 60);
        }
        put(m, ga, off(0.0, 0.0, 0.0));
        put(m, gb, off(bob_at, 0.0, 0.0));
        crate::game::map::visibility::refresh_dirty(m);
    })
    .await
    .unwrap();
    (realm, [a, b], ga, gb, map)
}

/// Thistle Tea's "NPC-controlled players retain their controller in melee
/// and spells": a player a creature charms fights on its side, her blow
/// and her spell on her friend its, flagging neither of them for PvP nor
/// running the timer between players.
#[tokio::test]
async fn a_creature_charmed_players_blows_and_spells_are_her_charmers() {
    let (_realm, _c, ga, gb, map) = two(1.5).await;
    let r = map
        .query(move |m| {
            let npc = creature_at(m, ga, BRUTE, off(-10.0, 0.0, 0.0));
            crate::game::map::pets::charmed(m, npc, ga, false);
            let side = crate::game::map::pets::master(m, ga) == npc;
            let pvp = |m: &Map| {
                [ga, gb].map(|g| {
                    (
                        m.unit_fields(g)
                            .unwrap()
                            .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP),
                        m.combat(g).unwrap().combat_until,
                    )
                })
            };
            p3::forced_swing(m, ga, gb, combat::Outcome::Hit);
            let melee = (in_combat(m, gb), pvp(m));
            for g in [ga, gb] {
                combat::leave_combat(m, g);
            }
            let health = m.unit_fields(gb).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let cast = crate::game::map::spells::cast(
                m,
                ga,
                BLAST,
                crate::game::map::spells::Targets {
                    unit: Some(gb),
                    ..Default::default()
                },
                None,
            );
            assert!(cast, "her blast");
            let struck = m.unit_fields(gb).unwrap().get_u32(UNIT_FIELD_HEALTH) < health;
            (side, melee, struck, in_combat(m, gb), pvp(m))
        })
        .await
        .unwrap();
    let none = [(false, None); 2];
    assert!(r.0, "her charmer's side");
    assert_eq!(r.1, (true, none), "her blow");
    assert!(r.2 && r.3, "her spell lands: {r:?}");
    assert_eq!(r.4, none, "her spell");
}

/// Thistle Tea's "retains a valid victim after the controller changes
/// targets" and "roots prevent navigation while allowing a ranged spell": a
/// charmed caster keeps casting at the foe she chose though her charmer
/// turns to another; rooted, she casts from where she stands, and with no
/// mana to cast she stands there still.
#[tokio::test]
async fn a_creature_charmed_caster_keeps_her_foe_and_rooted_casts_where_she_stands() {
    let (_realm, _c, ga, gb, map) = two(20.0).await;
    let r = map
        .query(move |m| {
            let npc = creature_at(m, ga, BRUTE, off(-5.0, 0.0, 0.0));
            let other = creature_at(m, ga, HELPER, off(-20.0, 0.0, 0.0));
            let c = m.creature_mut(npc).unwrap();
            c.threat = vec![(gb, 100.0)];
            c.fields.set_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                true,
            );
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.push(BOLT);
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
            p.fields.set_u32(UNIT_FIELD_POWER1, 1000);
            crate::game::map::pets::charmed(m, npc, ga, false);
            let casting_at = |m: &Map| {
                m.caster(ga)
                    .and_then(|c| c.casting.as_ref())
                    .and_then(|c| c.targets.unit)
            };
            let think = |m: &mut Map| {
                crate::game::map::spells::interrupt(m, ga);
                let now = m.now;
                let p = m.players.get_mut(&ga).unwrap();
                let pos = p.owned.possessed_by.as_mut().unwrap();
                pos.next_think = now;
                pos.next_cast_at = now;
                crate::game::map::pets::update(m);
            };
            think(m);
            let first = casting_at(m);
            // Her charmer turns to another.
            m.creature_mut(npc).unwrap().threat = vec![(other, 100.0)];
            think(m);
            let kept = casting_at(m);
            // Rooted, she casts where she stands; with no mana, she stands.
            p3::aura_of(m, ga, 990_881, crate::game::map::auras::AURA_MOD_ROOT, 0, 0);
            let at = m.position_of(ga).unwrap();
            think(m);
            let rooted = casting_at(m);
            crate::game::map::spells::interrupt(m, ga);
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_POWER1, 0);
            let mut moved = 0.0f32;
            for _ in 0..6 {
                think(m);
                m.now += Duration::from_millis(500);
                crate::game::map::player_motion::update(m);
                let now = m.position_of(ga).unwrap();
                moved = moved.max(((now.x - at.x).powi(2) + (now.y - at.y).powi(2)).sqrt());
            }
            (first, kept, rooted, moved)
        })
        .await
        .unwrap();
    assert_eq!(r.0, Some(gb), "at her charmer's foe");
    assert_eq!(r.1, Some(gb), "kept, her charmer on another");
    assert_eq!(r.2, Some(gb), "rooted, she casts");
    assert!(r.3 < 0.01, "rooted, she stays where she is: {}", r.3);
}

/// Thistle Tea's "only Chains of Kel'Thuzad activates area charm", as
/// mangos has it: a charm of many (`SPELL_AURA_AOE_CHARM`) is a charm, of
/// whatever spell, and its end lets her go.
#[tokio::test]
async fn a_charm_of_many_charms_her_and_its_end_lets_her_go() {
    let (_realm, _c, ga, _gb, map) = two(5.0).await;
    let r = map
        .query(move |m| {
            let npc = creature_at(m, ga, BRUTE, off(-3.0, 0.0, 0.0));
            crate::game::map::spells::cast_triggered(m, npc, CHAINS, Some(ga));
            let held = |m: &Map| {
                (
                    m.players[&ga]
                        .owned
                        .possessed_by
                        .map(|p| (p.master, p.spell)),
                    m.unit_fields(ga).unwrap().get_guid(UNIT_FIELD_CHARMEDBY),
                )
            };
            let charmed = held(m);
            crate::game::map::auras::remove_spell(m, ga, CHAINS, None);
            (npc, charmed, held(m))
        })
        .await
        .unwrap();
    assert_eq!(r.1, (Some((r.0, CHAINS)), r.0), "charmed by it");
    assert_eq!(r.2, (None, Guid::zero()), "let go");
}
