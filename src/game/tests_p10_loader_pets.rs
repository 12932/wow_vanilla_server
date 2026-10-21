//! Round 10's pet loaders (Thistle Tea's `pet_autocast_dbc_test.exs`), on
//! the real spells and a warlock's imp: when a pet's helpful autocast
//! spells go and on whom.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::{Map, auras, combat, pets, spells};

const SUMMON_IMP: u32 = 688;

/// A warlock at 60 with an imp out: the warlock and the imp.
fn with_imp(m: &mut Map, g: Guid) -> Guid {
    ready(m, g, 60, &[]);
    spells::cast_triggered(m, g, SUMMON_IMP, None);
    pets::pet_of(m, g).expect("the imp")
}

/// The imp ready to cast `autocast`, nothing on it, then a second's turn.
fn think_with(m: &mut Map, imp: Guid, autocast: &[u32]) {
    if let Some(c) = m.caster_mut(imp) {
        c.casting = None;
        c.gcd_until = None;
        c.cooldowns.clear();
        c.category_cooldowns.clear();
    }
    // Mana and focus to spare (a hunter's pet's spells cost focus).
    if let Some(f) = m.unit_fields_mut(imp) {
        for i in 0..5 {
            f.set_u32(UNIT_FIELD_MAXPOWER1 + i, 10_000);
            f.set_u32(UNIT_FIELD_POWER1 + i, 10_000);
        }
    }
    let now = m.now;
    let s = pets::state_mut(m, imp).unwrap();
    s.autocast = autocast.to_vec();
    s.next_think = now;
    m.now += Duration::from_secs(1);
    pets::update(m);
}

/// Thistle Tea's "Dash and Dive wait for combat and do not fire within
/// melee reach": a pet's Dash (23099, 23109, 23110) and Dive (23145,
/// 23147, 23148), whose cooldown outlasts them, aren't cast idle; fighting
/// a victim twenty yards off they are, and not with it in reach. And
/// "Furious Howl and Tainted Blood wait for a victim while Prowl preserves
/// an attack command": Furious Howl (24604, more damage) and Tainted Blood
/// (19478, its icon a fight's) only in a fight; Prowl (24450) only out of
/// one, as it would end the attack (`pets/ai.rs` `pick_positive`).
#[tokio::test]
#[ignore = "extended"]
async fn a_pets_fight_buffs_wait_for_a_fight_and_prowl_for_its_end() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10P1", "Impish", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            let imp = with_imp(m, g);
            let foe = dummy(m, g, 60, 30.0);
            let mut out = Vec::new();
            for id in [
                23_099u32, 23_109, 23_110, 23_145, 23_147, 23_148, 24_604, 19_478, 24_450,
            ] {
                let step = |m: &mut Map, yards: Option<f32>| {
                    auras::remove_spell(m, imp, id, None);
                    combat::stop_attack(m, imp);
                    combat::leave_combat(m, imp);
                    combat::leave_combat(m, foe);
                    // Where it stands, its chase of the last step over.
                    pets::stop_where(m, imp);
                    if let Some(yards) = yards {
                        let at = m.position_of(imp).unwrap();
                        let there = Vector3d {
                            x: at.x + yards,
                            ..at
                        };
                        crate::game::map::scripts::place(m, foe, there, 0.0);
                        pets::attack_target(m, imp, foe);
                        m.note_fighting(imp);
                    }
                    think_with(m, imp, &[id]);
                    has(m, imp, id)
                };
                let idle = step(m, None);
                let far = step(m, Some(20.0));
                let near = step(m, Some(1.0));
                out.push((id, idle, far, near));
            }
            out
        })
        .await
        .unwrap();
    for (id, idle, far, near) in out {
        let want = match id {
            24_450 => (true, false, false),
            24_604 | 19_478 => (false, true, true),
            _ => (false, true, false),
        };
        assert_eq!(
            (idle, far, near),
            want,
            "{id}: idle, twenty yards, in reach"
        );
    }
}

/// Thistle Tea's "every pet Fire Shield and Devour Magic rank retains its
/// selectable recipients": each Fire Shield (2947, 8316, 8317, 11770,
/// 11771) is helpful, of the imp's look (289) and family (5, 0x800000),
/// thirty yards, and an imp autocasts it on its master once under attack;
/// each Devour Magic (19505, 19731, 19734, 19736) is a dispel of magic (1)
/// at any unit (25), not harmful.
#[tokio::test]
#[ignore = "extended"]
async fn every_fire_shield_goes_on_the_attacked_master_and_devour_magic_is_no_harm() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for id in [19_505u32, 19_731, 19_734, 19_736] {
        let s = d.spells.get(id).unwrap();
        let e = s.effects[0];
        assert_eq!((e.effect, e.target_a, e.misc_value), (38, 25, 1), "{id}");
        assert!(auras::positive(s), "{id}: no harm");
    }
    let (_c, g, map) = player(&realm, "P10P2", "Shielded", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            let imp = with_imp(m, g);
            let data = m.realm.data.clone();
            let kobold = dummy(m, g, 60, 4.0);
            combat::start_attack(m, kobold, g);
            combat::engage(m, kobold, g);
            [2947u32, 8316, 8317, 11_770, 11_771].map(|id| {
                let s = data.spells.get(id).unwrap();
                let shape = (
                    auras::positive(s),
                    s.spell_visual,
                    s.family_name,
                    s.family_flags & 0x80_0000 != 0,
                    data.spells.range(s).1,
                );
                auras::remove_spell(m, g, id, None);
                think_with(m, imp, &[id]);
                (id, shape, has(m, g, id))
            })
        })
        .await
        .unwrap();
    for (id, shape, on_master) in out {
        assert_eq!(shape, (true, 289, 5, true, 30.0), "{id}");
        assert!(on_master, "{id}: on the master under attack");
    }
}

/// Prowl (24450) from the pet's bar, nothing targeted and no fight: it
/// goes on the pet, its slow its own (mangos's `HandlePetAction`, a spell
/// that needs no target cast at none), not "nothing to attack".
#[tokio::test]
#[ignore = "extended"]
async fn prowl_from_the_pets_bar_goes_on_the_pet_with_nothing_to_attack() {
    if !have_data() {
        return;
    }
    use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
    const PROWL: u32 = 24_450;
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10P3", "Prowler", Race::Human, Class::Warlock).await;
    let prowled = map
        .query(move |m| {
            let imp = with_imp(m, g);
            think_with(m, imp, &[]);
            pets::state_mut(m, imp).unwrap().spells.push(PROWL);
            pets::handle(
                m,
                g,
                ClientOpcodeMessage::CMSG_PET_ACTION(CMSG_PET_ACTION {
                    pet: imp,
                    data: pets::button(PROWL, pets::ACT_ENABLED),
                    target: Guid::zero(),
                }),
            );
            has(m, imp, PROWL)
        })
        .await
        .unwrap();
    assert!(prowled);
}

/// Thistle Tea's "engineering guardian passives derive health and damage
/// from DBC" (`spell_guardian_dbc_test.exs`): the Mechanical Dragonling
/// (2678, from 4073), the Mithril one (8615, from 12749) and the Arcanite
/// one (12473, from 19804) come with their passives (23051, 23050, 23052),
/// worth 300, 500 and 700 more health and 50, 90 and 115 more damage a
/// blow (a creature's flat physical damage done is added as it swings,
/// `combat::weapon_damage`); the passive taken off, they are their own. A
/// guardian of no passive (the imp, 416) has none.
#[tokio::test]
#[ignore = "extended"]
async fn the_dragonlings_passives_give_their_health_and_damage() {
    if !have_data() {
        return;
    }
    assert_eq!(crate::game::map::item_scripts::guardian_passive(416), None);
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10P4", "Engineer", Race::Gnome, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let foe = dummy(m, g, 60, 3.0);
            [
                (4073u32, 2678u32, 23_051u32),
                (12_749, 8615, 23_050),
                (19_804, 12_473, 23_052),
            ]
            .map(|(summon, entry, passive)| {
                spells::cast_triggered(m, g, summon, Some(g));
                let it = m
                    .creatures
                    .iter()
                    .find(|(_, c)| c.entry == entry && c.alive())
                    .map(|(_, c)| c.guid)
                    .expect("the dragonling");
                let stats = |m: &Map| {
                    use rand::SeedableRng;
                    let blow = |seed: u64| {
                        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
                        let hand = combat::Hand::Main;
                        combat::weapon_damage(m, it, foe, hand, &mut rng).0 as f32
                    };
                    let f = m.unit_fields(it).unwrap();
                    (f.get_u32(UNIT_FIELD_MAXHEALTH), blow(1), blow(2))
                };
                let with = (has(m, it, passive), stats(m));
                auras::remove_spell(m, it, passive, None);
                let own = stats(m);
                m.remove_creature(it);
                (
                    entry,
                    with.0,
                    with.1.0 - own.0,
                    with.1.1 - own.1,
                    with.1.2 - own.2,
                )
            })
        })
        .await
        .unwrap();
    for ((entry, on, health, min, max), want) in
        out.into_iter()
            .zip([(300u32, 50.0f32), (500, 90.0), (700, 115.0)])
    {
        assert!(on, "{entry}: its passive");
        assert_eq!(health, want.0, "{entry}: health");
        assert!(
            (min - want.1).abs() < 0.01 && (max - want.1).abs() < 0.01,
            "{entry}: two blows {min} {max} for {}",
            want.1
        );
    }
}
