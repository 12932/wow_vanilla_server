//! Round 8's world tests, ported from Thistle Tea's
//! `test/game/world/combat/` (docs/parity/matrix.csv): a share of damage
//! as its taker is told of it.

use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_SPLIT_DAMAGE_PCT};
use crate::game::tests_p3_combat::{BRUTE, setup, spawn};
use crate::game::tests_p3_entity::{SMSG_SPELLNONMELEEDAMAGELOG, bob, on, packed_len};

/// Soul Link's share (25228): thirty percent of all damage.
const SOUL_LINK: u32 = 25_228;

/// Thistle Tea's `SpellReception.aura_contexts/2` "projects the actual
/// sharing spell and school without offensive proc feedback": Bob bears
/// Alice's link of thirty percent; a brute's forty physical takes him
/// seventy percent, her thirty, and she is told of hers as the brute's under
/// the link's spell, of its school, none absorbed, no crit (mangos's
/// `CalculateDamageAbsorbAndResist` logging the split by its aura).
#[tokio::test]
async fn a_shared_blow_is_logged_to_its_taker_under_the_links_spell() {
    let (realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (_b, gb) = bob(&realm, ga).await;
    a.clear();
    let (brute, hurt) = on(&map, move |m| {
        let mut effects = [SpellEffect::default(); 3];
        effects[0] = SpellEffect {
            effect: 6,
            aura: AURA_SPLIT_DAMAGE_PCT,
            misc_value: 127,
            base_points: 29,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
        let s = SpellTemplate {
            id: SOUL_LINK,
            name: "Soul Link".into(),
            equipped_item_class: -1,
            effects,
            ..Default::default()
        };
        let _ = auras::apply_spell(m, ga, gb, &s, &[(0, 30)], auras::Context::default());
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        let health =
            |m: &crate::game::map::Map, g| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH);
        let before = (health(m, gb), health(m, ga));
        let left = auras::share(m, brute, gb, 40, 1);
        crate::game::map::combat::deal_damage(m, brute, gb, left);
        (brute, (before.0 - health(m, gb), before.1 - health(m, ga)))
    })
    .await;
    assert_eq!(hurt, (28, 12), "seventy percent his, thirty hers");
    let body = a
        .wait_frame("her share's log", SMSG_SPELLNONMELEEDAMAGELOG, |_| true)
        .await;
    let packed = |g| {
        let mut v = Vec::new();
        crate::game::update::write_packed_guid(&mut v, g);
        v
    };
    let target_len = packed_len(&body);
    let caster_len = packed_len(&body[target_len..]);
    assert_eq!(
        (
            &body[..target_len],
            &body[target_len..target_len + caster_len]
        ),
        (&packed(ga)[..], &packed(brute)[..]),
        "to her, from the brute"
    );
    let rest = &body[target_len + caster_len..];
    let u32_at = |at: usize| u32::from_le_bytes(rest[at..at + 4].try_into().unwrap());
    assert_eq!(u32_at(0), SOUL_LINK, "under the link's spell");
    assert_eq!(u32_at(4), 12, "her twelve");
    assert_eq!(rest[8], 0, "physical");
    assert_eq!(u32_at(9), 0, "none absorbed");
    assert_eq!(u32_at(21), 0, "no crit");
}

/// Thistle Tea's `SpellReception.aura_contexts/2` "running auras observe
/// modifier gain and removal on subsequent ticks": Alice's burn on a
/// brute ticks a hundred; while her threat is cut by thirty percent a tick
/// is seventy threat, and once the cut is gone the next is a hundred
/// (each tick's threat by her modifiers as it lands: mangos's
/// `PeriodicTick` dealing damage through `DealDamage`'s threat).
#[tokio::test]
async fn a_burns_ticks_threaten_by_the_casters_modifiers_as_they_land() {
    use crate::game::map::auras::{AURA_MOD_THREAT, AURA_PERIODIC_DAMAGE};
    use crate::game::map::spells::{self, Targets};
    use crate::game::tests_p3_cast_kit::{LASTS_15S, aura, bare, effect, insert};
    use crate::game::tests_p3_combat::{aura_of, data, setup_with};
    const BURN: u32 = 997_870;
    const CALM: u32 = 997_871;
    let mut d = data();
    d.spells.durations.insert(
        LASTS_15S,
        crate::game::data::spells::SpellDuration {
            base: 15_000,
            per_level: 0,
            max: 15_000,
        },
    );
    let mut burn = bare(BURN, 2);
    burn.duration_index = LASTS_15S;
    effect(
        &mut burn,
        0,
        SpellEffect {
            amplitude: 1000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 100, 6)
        },
    );
    insert(&mut d, [burn]);
    let (_realm, _a, ga, map) = setup_with(d, Class::Mage).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 10.0);
        let threat = |m: &crate::game::map::Map| {
            m.creature(brute)
                .unwrap()
                .threat
                .iter()
                .find(|(t, _)| *t == ga)
                .map_or(0.0, |(_, v)| *v)
        };
        aura_of(m, ga, CALM, AURA_MOD_THREAT, 127, -30);
        let cast = spells::cast(
            m,
            ga,
            BURN,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
            None,
        );
        let mut deltas = Vec::new();
        for calm in [true, false] {
            if !calm {
                crate::game::map::auras::remove_spell(m, ga, CALM, None);
            }
            let before = threat(m);
            m.now += std::time::Duration::from_millis(1001);
            crate::game::map::auras::update(m);
            deltas.push(threat(m) - before);
        }
        (cast, deltas)
    })
    .await;
    let (cast, deltas) = out;
    assert!(cast);
    assert!(
        (deltas[0] - 70.0).abs() < 0.5 && (deltas[1] - 100.0).abs() < 0.5,
        "seventy with the cut, a hundred without: {deltas:?}"
    );
}
