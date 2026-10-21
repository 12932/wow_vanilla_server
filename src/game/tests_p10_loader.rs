//! Round 10's loaders (Thistle Tea's `world/loader/*_test.exs` that wvs
//! did partly or differently), on the real data: the four Blinks' ways,
//! the real charges' swing on arrival, Curse of Recklessness holding a
//! Fear back until another curse takes its place, the costumes' looks and
//! weapons, the Silithyst reward's reputation, Holly's reindeer, the
//! captures' auras and the equip-spell enchantments.

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOUNTED};
use crate::game::map::spells;
use crate::game::map::{Map, charge, combat};

/// The angle from `o` of the way from `a` to `b`, in [0, 2π), and how far.
fn way(a: Vector3d, b: Vector3d, o: f32) -> (f32, f32) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    (
        (dy.atan2(dx) - o).rem_euclid(TAU),
        (dx * dx + dy * dy).sqrt(),
    )
}

fn near_angle(a: f32, b: f32) -> bool {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d) < 0.01
}

/// Thistle Tea's "decodes all four directional Blink spells"
/// (`spell_caster_location_dbc_test.exs`): the real Blinks 29208 to 29211
/// leap twenty yards from the caster, ahead (47), behind (48), to the left
/// (49) and to the right (50), as mangos's caster-relative destinations
/// have them.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_blinks_go_the_way_their_targets_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10L1", "Blinker", Race::Human, Class::Mage).await;
    let moved = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let start = m.players[&g].position();
            let o = m.players[&g].movement.orientation;
            [29_208u32, 29_209, 29_210, 29_211].map(|id| {
                m.near_teleport(g, start, o);
                spells::cast_triggered(m, g, id, Some(g));
                way(start, m.players[&g].position(), o)
            })
        })
        .await
        .unwrap();
    for ((angle, yards), (id, want)) in moved.into_iter().zip([
        (29_208, 0.0),
        (29_209, PI),
        (29_210, FRAC_PI_2),
        (29_211, 3.0 * FRAC_PI_2),
    ]) {
        assert!((yards - 20.0).abs() < 0.1, "{id}: twenty yards, {yards}");
        assert!(near_angle(angle, want), "{id}: {angle} for {want}");
    }
}

/// Thistle Tea's "charge spells retain their automatic attack and
/// cancellation rules" (`spell_combat_control_dbc_test.exs`): the
/// warrior's Charges (100, 6178, 11578) and Intercept (20252) swing at
/// their target on arriving; the rocket helmet's charges (13711, 22641),
/// which cancel auto attack, don't.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_charges_swing_on_arrival_unless_they_cancel_the_attack() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10L2", "Charger", Race::Human, Class::Warrior).await;
    let swung = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let start = m.players[&g].position();
            let foe = dummy(m, g, 60, 15.0);
            [100u32, 6178, 11_578, 20_252, 13_711, 22_641].map(|id| {
                let p = m.players.get_mut(&g).unwrap();
                p.movement.position = start;
                p.charge = None;
                combat::stop_attack(m, g);
                spells::cast_triggered(m, g, id, Some(foe));
                let charged = m.players[&g].charge.is_some();
                m.now += Duration::from_secs(5);
                charge::update(m);
                (id, charged, m.players[&g].combat.attacking)
            })
        })
        .await
        .unwrap();
    for (id, charged, attacking) in swung {
        assert!(charged, "{id} charges");
        assert_eq!(attacking, ![13_711, 22_641].contains(&id), "{id}");
    }
}

const FLEEING: u32 = 0x0080_0000;

/// Thistle Tea's "replacing Curse of Recklessness restores an existing
/// Fear" (`fear_dbc_test.exs`): a kobold feared (6213) and then cursed
/// with Recklessness (704, which prevents fleeing) keeps the fear but
/// doesn't flee; the warlock's Curse of Weakness (702) takes the curse's
/// place, and the kobold flees again (mangos's `HandlePreventFleeing`).
#[tokio::test]
#[ignore = "extended"]
async fn a_curse_of_weakness_in_recklessness_place_lets_the_fear_take_hold() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10L3", "Cursed", Race::Human, Class::Warlock).await;
    let steps = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let foe = dummy(m, g, 60, 10.0);
            let state = |m: &Map| {
                let flags = m
                    .unit_fields(foe)
                    .map_or(0, |f| f.get_u32(UNIT_FIELD_FLAGS));
                (
                    has(m, foe, 6213),
                    has(m, foe, 704),
                    flags & FLEEING != 0,
                    m.creature(foe).unwrap().engagement.control.is_some(),
                )
            };
            apply(m, g, foe, 6213);
            let feared = state(m);
            apply(m, g, foe, 704);
            let reckless = state(m);
            apply(m, g, foe, 702);
            (feared, reckless, state(m), has(m, foe, 702))
        })
        .await
        .unwrap();
    let (feared, reckless, weakened, weakness) = steps;
    assert_eq!(feared, (true, false, true, true), "feared");
    assert_eq!(reckless, (true, true, false, false), "held back");
    assert!(weakness, "Curse of Weakness on");
    assert_eq!(weakened, (true, false, true, true), "fleeing again");
}

/// Thistle Tea's "loads the creature's display scale and equipment
/// independently of spell DBC rows" (`spell_appearance_vmangos_test.exs`):
/// the Skeleton Costume (24723) makes a kobold a Skeletal Fiend (531),
/// its display 7550 and the fiend's dagger (2711, display 7433) in its
/// hand; off again, the kobold's own look and pick (5276, display 5010)
/// are back. The Ninja Costume (24711) is a Male Dark Assassin (5946):
/// display 4617 at its scale of 1, and, the assassin holding nothing, the
/// kobold keeps its pick (mangos's `HandleAuraTransform` loads the
/// creature's equipment only where it has some).
#[tokio::test]
#[ignore = "extended"]
async fn the_costumes_take_their_creatures_look_scale_and_weapons() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10L4", "Costumer", Race::Human, Class::Mage).await;
    let looks = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let foe = dummy(m, g, 60, 5.0);
            // The kobold's pick, as its spawn holds it.
            let data = m.realm.data.clone();
            let pick = data
                .creature_template(6)
                .and_then(|t| data.creature_equipment.get(&t.equipment_id))
                .copied()
                .unwrap();
            m.creature_mut(foe)
                .unwrap()
                .set_equipment_slots(&data, pick.map(Some));
            let look = |m: &Map| {
                let f = m.unit_fields(foe).unwrap();
                (
                    f.get_u32(UNIT_FIELD_DISPLAYID),
                    f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY),
                    f.get_f32(OBJECT_FIELD_SCALE_X),
                )
            };
            let own = look(m);
            apply(m, g, foe, 24_723);
            let skeleton = look(m);
            auras::remove_spell(m, foe, 24_723, None);
            let back = look(m);
            apply(m, g, foe, 24_711);
            let ninja = look(m);
            auras::remove_spell(m, foe, 24_711, None);
            (own, skeleton, back, ninja, look(m))
        })
        .await
        .unwrap();
    let (own, skeleton, back, ninja, again) = looks;
    assert_eq!((own.0, own.1), (10_913, 5010), "the kobold and its pick");
    assert_eq!(
        (skeleton.0, skeleton.1),
        (7550, 7433),
        "the fiend and its dagger"
    );
    assert_eq!(back, own, "its own back");
    assert_eq!(
        (ninja.0, ninja.1),
        (4617, 5010),
        "the assassin, the pick kept"
    );
    assert!(
        (ninja.2 - 1.0).abs() < 1e-4,
        "at the assassin's scale: {ninja:?}"
    );
    assert_eq!(again, own, "its own back");
}

/// Thistle Tea's "normalizes the capture reward's legacy reputation
/// effect" (`spell_silithyst_dbc_test.exs`): the Silithyst capture's
/// reward (31247), whose reputation effect has a later client's number
/// (122), loads as the reputation effect and gives ten with the Cenarion
/// Circle (609). A turn-in's rewards (Traces of Silithyst 29534, which
/// casts it again, the honor 31420 and it) give twenty, mangos's
/// `REPUTATION_REWARD_SILITHYST`.
#[tokio::test]
#[ignore = "extended"]
async fn the_silithyst_reward_gives_ten_with_the_cenarion_circle() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let reward = realm.data.spells.get(31_247).unwrap();
    assert_eq!(
        (reward.effects[1].effect, reward.effects[1].misc_value),
        (crate::game::map::extra_effects::EFFECT_REPUTATION, 609)
    );
    let (_c, g, map) = player(&realm, "P10L5", "Carrier", Race::Orc, Class::Warrior).await;
    let gained = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let data = m.realm.data.clone();
            let standing =
                |m: &Map| crate::game::map::reputation::standing(&m.players[&g], &data, 609);
            let before = standing(m);
            spells::cast_triggered(m, g, 31_247, Some(g));
            let one = standing(m) - before;
            let before = standing(m);
            for r in [29_534, 31_420, 31_247] {
                spells::cast_triggered(m, g, r, Some(g));
            }
            (one, standing(m) - before)
        })
        .await
        .unwrap();
    assert_eq!(gained, (10, 20));
}

/// Thistle Tea's "Holly preserves both speed tiers through resolved
/// triggered spells" (`mount_transformation_dbc_test.exs`): Holly's
/// Reindeer Transformation (25860) on a Brown Horse (458) is the reindeer
/// (25858) at 11.2 yards a second, on a Swift White Steed (23228) the
/// swift one (25859) at 14; the horse gone, the Mounted Reindeer's look
/// (15665's display, 15902 in vmangos; Thistle Tea's test caches 15960)
/// the one mount. Cancelled, under water or dead, the reindeer goes: no
/// mount, a run of 7.
#[tokio::test]
#[ignore = "extended"]
async fn holly_makes_a_reindeer_of_either_speed_and_it_goes_as_a_mount_goes() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10L6", "Reindeer", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let mount_of = |m: &Map| {
                let p = &m.players[&g];
                (
                    p.fields.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
                    p.speeds.run,
                    auras::auras(m, g).map_or(0, |a| {
                        a.holders.iter().filter(|h| h.has(AURA_MOUNTED)).count()
                    }),
                )
            };
            let mut out = Vec::new();
            for (mount, reindeer) in [(458u32, 25_858u32), (23_228, 25_859)] {
                for end in ["cancelled", "under water", "dead"] {
                    if !m.unit_view(g).is_some_and(|u| u.alive) {
                        crate::game::map::death::resurrect(m, g, 1.0);
                    }
                    apply(m, g, g, mount);
                    spells::cast_triggered(m, g, 25_860, Some(g));
                    let on = (has(m, g, reindeer), has(m, g, mount), mount_of(m));
                    match end {
                        "cancelled" => auras::cancel(m, g, reindeer),
                        "under water" => auras::interrupt(m, g, auras::INTERRUPT_UNDER_WATER),
                        _ => crate::game::map::death::kill(m, g),
                    }
                    out.push((mount, end, on, has(m, g, reindeer), mount_of(m)));
                }
            }
            out
        })
        .await
        .unwrap();
    for (mount, end, on, still, off) in out {
        let speed = if mount == 458 { 11.2 } else { 14.0 };
        assert!(on.0 && !on.1, "{mount}: the reindeer for the mount: {on:?}");
        assert_eq!(
            (on.2.0, on.2.2),
            (15_902, 1),
            "{mount}: its look, one mount"
        );
        assert!((on.2.1 - speed).abs() < 1e-3, "{mount}: {speed}: {on:?}");
        assert!(!still, "{mount}, {end}: gone");
        assert_eq!(off.0, 0, "{mount}, {end}: no mount");
        assert!(
            (off.1 - 7.0).abs() < 1e-3,
            "{mount}, {end}: a run of 7: {off:?}"
        );
    }
}

/// Thistle Tea's "real muzzle and collar pulses trigger capture and
/// indefinite dummy auras acquire lifetime checks"
/// (`captured_follower_dbc_test.exs`): the Frostwolf Muzzle (21794) and
/// the Alterac Ram Collar (21866) pulse their capture (21795, 21867) every
/// fifteen seconds; the aura a Frostwolf (10981) or a ram (10990) puts on
/// its captor (21827, 21863), a dummy, looks at its animal every two
/// seconds.
#[tokio::test]
#[ignore = "extended"]
async fn the_muzzle_and_collar_pulse_and_their_captures_look_every_two_seconds() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10L7", "Captor", Race::Orc, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let data = m.realm.data.clone();
            [
                (21_794u32, 21_795u32, 21_827u32, 10_981u32),
                (21_866, 21_867, 21_863, 10_990),
            ]
            .map(|(capture, pulse, aura, entry)| {
                let s = data.spells.get(capture).unwrap();
                let pulses = s.effects.iter().any(|e| {
                    e.aura == auras::AURA_PERIODIC_TRIGGER_SPELL
                        && e.trigger_spell == pulse
                        && e.amplitude == 15_000
                });
                let dummy_aura = data.spells.get(aura).unwrap().has_aura(auras::AURA_DUMMY);
                let animal = spawn(m, g, entry, 55, 4.0);
                apply(m, animal, g, aura);
                let look = holder(m, g, aura).and_then(|h| h.effects[0].next_tick);
                let look = look.map(|t| t.saturating_duration_since(m.now));
                (pulses, dummy_aura, has(m, g, aura), look)
            })
        })
        .await
        .unwrap();
    for (pulses, dummy_aura, held, look) in out {
        assert!(pulses && dummy_aura && held, "{pulses} {dummy_aura} {held}");
        assert_eq!(look, Some(Duration::from_secs(2)));
    }
}

/// Put an item of `entry` on `g` in the equipment slot `slot`.
fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    use crate::game::store::BAG_PLAYER;
    if let Some(p) = m.players.get_mut(&g) {
        p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    }
    crate::game::map::inventory::add_item(m, g, entry, 1, false).expect("room for it");
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).expect("the player");
    let i = p
        .items
        .iter()
        .rposition(|i| i.entry == entry)
        .expect("the item");
    p.items[i].bag = BAG_PLAYER;
    p.items[i].slot = slot;
    p.refresh_inventory(&data);
    p.gear_dirty = true;
    crate::game::map::inventory::sync_gear(m, g);
}

/// `g`'s item of `entry` enchanted (permanently) with `enchantment`, its
/// gear's auras put in line.
fn enchant(m: &mut Map, g: Guid, entry: u32, enchantment: u32) {
    if let Some(p) = m.players.get_mut(&g)
        && let Some(i) = p.items.iter_mut().find(|i| i.entry == entry)
    {
        i.fields.set_u32(ITEM_FIELD_ENCHANTMENT, enchantment);
        p.gear_dirty = true;
    }
    crate::game::map::inventory::sync_gear(m, g);
}

const FRAYED_BRACERS: u32 = 3365;

/// Thistle Tea's "real Minor Health and Greater Intellect equip spells
/// derive player stats" (`enchantments_dbc_test.exs`): bracers enchanted
/// with Health +5 (41, its equip spell Minor Health 7419) raise the
/// wearer's health by five, with Intellect +7 (1883, Greater Intellect
/// 13825) their intellect by seven; the enchantment gone, so are they.
#[tokio::test]
#[ignore = "extended"]
async fn an_equip_spell_enchantment_raises_its_stat_while_it_is_on() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10L8", "Enchanted", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            let data = m.realm.data.clone();
            crate::game::map::stats::set_level(m.players.get_mut(&g).unwrap(), &data, 60);
            equip(m, g, FRAYED_BRACERS, 8);
            let stats = |m: &Map| {
                let f = &m.players[&g].fields;
                (
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                    f.get_u32(UNIT_FIELD_STAT0 + 3),
                )
            };
            let bare = stats(m);
            enchant(m, g, FRAYED_BRACERS, 41);
            let health = (stats(m), has(m, g, 7419));
            enchant(m, g, FRAYED_BRACERS, 1883);
            let intellect = (stats(m), has(m, g, 7419), has(m, g, 13_825));
            enchant(m, g, FRAYED_BRACERS, 0);
            (bare, health, intellect, stats(m), has(m, g, 13_825))
        })
        .await
        .unwrap();
    let (bare, health, intellect, after, still) = out;
    assert!(health.1, "Minor Health on");
    assert_eq!(health.0, (bare.0 + 5, bare.1), "five health");
    assert!(
        !intellect.1 && intellect.2,
        "Greater Intellect in its place"
    );
    assert_eq!(intellect.0.1, bare.1 + 7, "seven intellect");
    assert!(!still, "gone with the enchantment");
    assert_eq!(after, bare);
}

/// Thistle Tea's "retains resolved conditions and per-effect masks"
/// (`spell_object_target_vmangos_test.exs`) and "retains conditions and
/// effect exclusions" (`spell_unit_target_vmangos_test.exs`): Collect
/// Rookery Egg (15958) keeps the condition its object row names, Dominion
/// of Soul (16053) its creature row's; the rows' `inverseEffectMask` keeps
/// a target off the effects it names: Clean Up Stink Bomb's (24973) first
/// bomb (180449) isn't for its second effect, its second (180450) isn't
/// for its third; Fire Cannon's (24731) creature (15218) isn't for its
/// second effect, its objects (180452, 180515) not for its first (mangos's
/// `CanNotHitWithSpellEffect`).
#[tokio::test]
#[ignore = "extended"]
async fn a_script_targets_rows_keep_their_conditions_and_the_effects_they_are_for() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let spells = &d.spells;
    assert_eq!(
        spells.script_target_conditions.get(&(15_958, 0, 175_124)),
        Some(&15_958)
    );
    assert_eq!(
        spells.script_target_conditions.get(&(16_053, 1, 10_321)),
        Some(&16_053)
    );
    for (row, mask) in [
        ((24_973, 0, 180_449), 2),
        ((24_973, 0, 180_450), 4),
        ((24_731, 1, 15_218), 2),
        ((24_731, 0, 180_452), 1),
    ] {
        assert_eq!(spells.script_target_masks.get(&row), Some(&mask), "{row:?}");
    }
    let (_c, g, map) = player(&realm, "P10L9", "Cleaner", Race::Human, Class::Warrior).await;
    let named = map
        .query(move |m| {
            use crate::game::map::spells::targets::script_target;
            let any = Guid::new(0xF110_0000_0000_0001);
            let rows = [
                (24_973u32, 0u32, 180_449u32),
                (24_973, 0, 180_450),
                (24_731, 1, 15_218),
                (24_731, 0, 180_452),
            ];
            rows.map(|(spell, kind, entry)| {
                [0usize, 1, 2].map(|i| script_target(m, g, spell, i, &[kind], entry, any))
            })
        })
        .await
        .unwrap();
    assert_eq!(named[0], [true, false, true], "the first bomb");
    assert_eq!(named[1], [true, true, false], "the second bomb");
    assert_eq!(named[2], [true, false, true], "the cannon's creature");
    assert_eq!(named[3], [false, true, true], "the cannon's object");
}

const STRIKE_ENCHANTMENT: u32 = 998_207;
const STRUCK: u32 = 998_208;
const SHORT_STAFF: u32 = 2132;

/// An enchantment whose combat spell, every blow, is a buff of its own.
fn striking(d: &mut crate::game::data::GameData) {
    let mut buff = d.spells.get(20_007).cloned().unwrap();
    buff.id = STRUCK;
    d.spells.spells.insert(STRUCK, buff);
    d.spells.enchantments.insert(
        STRIKE_ENCHANTMENT,
        crate::game::data::spells::Enchantment {
            id: STRIKE_ENCHANTMENT,
            effects: [
                (
                    crate::game::map::enchant::TYPE_COMBAT_SPELL,
                    100,
                    100,
                    STRUCK,
                ),
                (0, 0, 0, 0),
                (0, 0, 0, 0),
            ],
            name: "Striking".into(),
        },
    );
}

/// Thistle Tea's "uses only the striking hand and excludes feral weapon
/// procs" (`enchantments_dbc_test.exs`): a druid's enchanted main-hand
/// staff procs its enchantment's spell on a main-hand blow, not on an
/// off-hand or ranged one (no weapon there), nor in cat or bear form, nor
/// disarmed (mangos's `GetWeaponForAttack` of a usable weapon:
/// `CanUseEquippedWeapon`).
#[tokio::test]
#[ignore = "extended"]
async fn a_weapon_enchantment_procs_from_its_own_hand_and_not_in_a_feral_form() {
    if !have_data() {
        return;
    }
    use crate::game::map::procs::{self, Event};
    let realm = realm_plus(striking);
    let (_c, g, map) = player(&realm, "P10LA", "Feral", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            equip(m, g, SHORT_STAFF, 15);
            enchant(m, g, SHORT_STAFF, STRIKE_ENCHANTMENT);
            let foe = dummy(m, g, 60, 2.0);
            let blow = |m: &mut Map, offhand: bool, ranged: bool| {
                auras::remove_spell(m, g, STRUCK, None);
                procs::trigger(
                    m,
                    Event {
                        actor: g,
                        target: foe,
                        actor_flags: if ranged {
                            procs::PROC_DONE_RANGED
                        } else {
                            procs::PROC_DONE_MELEE
                        },
                        target_flags: procs::PROC_TAKEN_MELEE,
                        ex: procs::EX_NORMAL_HIT,
                        school: 1,
                        offhand,
                        ranged,
                        ..Default::default()
                    },
                );
                has(m, g, STRUCK)
            };
            let hands = [
                blow(m, false, false),
                blow(m, true, false),
                blow(m, false, true),
            ];
            let form = |m: &mut Map, f: u8| {
                m.players
                    .get_mut(&g)
                    .unwrap()
                    .fields
                    .set_byte(UNIT_FIELD_BYTES_1, 2, f);
            };
            let mut forms = Vec::new();
            for f in [auras::FORM_CAT, auras::FORM_BEAR, auras::FORM_DIREBEAR] {
                form(m, f);
                forms.push(blow(m, false, false));
            }
            form(m, 0);
            apply(m, foe, g, 676);
            let disarmed = (has(m, g, 676), blow(m, false, false));
            auras::remove_spell(m, g, 676, None);
            (hands, forms, disarmed, blow(m, false, false))
        })
        .await
        .unwrap();
    let (hands, forms, disarmed, again) = out;
    assert_eq!(hands, [true, false, false], "the main hand's alone");
    assert_eq!(forms, [false; 3], "none in a feral form");
    assert_eq!(disarmed, (true, false), "none disarmed");
    assert!(again, "and again with the staff in hand");
}

const ASCENDANCE: u32 = 28_200;
const ASCENDANCE_BONUS: u32 = 28_204;

/// Thistle Tea's "Ascendance has six scripted charges and a permanent
/// five-stack bonus" (`stacking_proc_dbc_test.exs`), its area spells: a
/// Renew's cast spends a charge and gives a stack, an Arcane Explosion's
/// (a place's, 22 and 15), a Blizzard's (28) and a Prayer of Healing's (a
/// party's, 20) do nothing (Thistle Tea's `area_target?` on either of an
/// effect's targets).
#[tokio::test]
#[ignore = "extended"]
async fn ascendance_spends_nothing_on_an_area_spell_a_partys_heal_too() {
    if !have_data() {
        return;
    }
    use crate::game::map::procs::{self, Event};
    let realm = realm();
    let (_c, g, map) = player(&realm, "P10LB", "Ascended", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let count = |m: &Map| {
                (
                    holder(m, g, ASCENDANCE).map(|h| h.charges),
                    holder(m, g, ASCENDANCE_BONUS).map(|h| h.stacks),
                )
            };
            let cast_end = |m: &mut Map, spell: u32| {
                let s = m.realm.data.spells.get(spell).cloned().unwrap();
                let (done, _) = spells::outcome::proc_flags(&s);
                procs::trigger(
                    m,
                    Event {
                        actor: g,
                        target: g,
                        actor_flags: done,
                        ex: procs::EX_CAST_END,
                        spell: Some(spell),
                        school: s.school_mask(),
                        ..Default::default()
                    },
                );
                count(m)
            };
            apply(m, g, g, ASCENDANCE);
            let renewed = cast_end(m, 139);
            let areas = [1449u32, 10, 596].map(|id| cast_end(m, id));
            (renewed, areas)
        })
        .await
        .unwrap();
    let (renewed, areas) = out;
    assert_eq!(renewed, (Some(5), Some(1)), "a Renew's stack");
    assert_eq!(areas, [renewed; 3], "the area spells', nothing");
}

/// Thistle Tea's "distinguishes queued combat-range attacks from other
/// short-range spells" (`spell_melee_range_dbc_test.exs`): Heroic Strike
/// (78, 11566), Cleave (845) and Maul (6807) are of "Combat Range" and on
/// the next swing; Lacerate (5422) is on the next swing at five yards of
/// "Interact Range", not a swing's reach; Fireball (133) neither. So a
/// warrior's Taunt (355, "Combat Range", of no melee damage class) reaches
/// as far as a swing (mangos's `SPELL_RANGE_FLAG_MELEE`): refused at seven
/// yards, cast at four; and a Backhand (6253, a melee blow of "Interact
/// Range") reaches its five yards past both units' reach: seven yards.
#[tokio::test]
#[ignore = "extended"]
async fn a_combat_range_spell_reaches_as_far_as_a_swing_and_no_further() {
    if !have_data() {
        return;
    }
    use crate::game::map::spells::checks;
    let realm = realm();
    let d = realm.data.clone();
    for id in [78u32, 11_566, 845, 6807] {
        let s = d.spells.get(id).unwrap();
        assert!(checks::melee_range(s) && s.on_next_swing(), "{id}");
    }
    let lacerate = d.spells.get(5422).unwrap();
    assert_eq!(d.spells.range(lacerate).1, 5.0);
    assert!(lacerate.on_next_swing() && !checks::melee_range(lacerate));
    assert!(!checks::melee_range(d.spells.get(133).unwrap()));
    let (_c, g, map) = player(&realm, "P10LC", "Taunter", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[355]);
            // In Defensive Stance, as Taunt asks.
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, 18);
            let data = m.realm.data.clone();
            [(355u32, 7.0f32), (355, 4.0), (6253, 7.0)].map(|(id, yards)| {
                let foe = dummy(m, g, 60, yards);
                let s = data.spells.get(id).unwrap();
                let mods = spells::mods::for_spell(m, g, s);
                let r = checks::check(m, g, s, &at(foe), None, false, &mods);
                m.remove_creature(foe);
                (id, yards, r)
            })
        })
        .await
        .unwrap();
    assert_eq!(
        out[0],
        (355, 7.0, Some(checks::fail::OUT_OF_RANGE)),
        "Taunt at seven"
    );
    assert_eq!(out[1], (355, 4.0, None), "Taunt at four");
    assert_eq!(out[2], (6253, 7.0, None), "Backhand at seven");
}

/// Thistle Tea's "Beast Lore grants temporary caster access and only
/// targets beasts" (`empathy_dbc_test.exs`): Beast Lore (1462) lasts thirty
/// seconds and is for beasts alone (a wolf, 299, not a kobold, 6); on the
/// wolf it raises `UNIT_DYNFLAG_SPECIALINFO` (0x10), gone when it ends.
/// (Thistle Tea shows the flag to the caster alone; wvs, as mangos's
/// `HandleAuraEmpathy`, sets the field, which every viewer sees.)
#[tokio::test]
#[ignore = "extended"]
async fn beast_lore_is_for_beasts_and_its_flag_goes_with_its_thirty_seconds() {
    if !have_data() {
        return;
    }
    use crate::game::map::spells::checks;
    const BEAST_LORE: u32 = 1462;
    const SPECIAL_INFO: u32 = 0x10;
    let realm = realm();
    let lore = realm.data.spells.get(BEAST_LORE).unwrap();
    assert_eq!(realm.data.spells.duration(lore, 60), Some(30_000));
    let (_c, g, map) = player(&realm, "P10LD", "Lorist", Race::Dwarf, Class::Hunter).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[BEAST_LORE]);
            let data = m.realm.data.clone();
            let s = data.spells.get(BEAST_LORE).unwrap();
            let wolf = spawn(m, g, 299, 60, 10.0);
            let kobold = dummy(m, g, 60, 10.0);
            let mods = spells::mods::for_spell(m, g, s);
            let on_kobold = checks::check(m, g, s, &at(kobold), None, false, &mods);
            let flag = |m: &Map| {
                m.unit_fields(wolf)
                    .map_or(0, |f| f.get_u32(UNIT_DYNAMIC_FLAGS) & SPECIAL_INFO)
            };
            let cast = cast_fresh(m, g, BEAST_LORE, at(wolf));
            pass(m, 3000);
            let on = (has(m, wolf, BEAST_LORE), flag(m));
            pass(m, 26_000);
            let still = has(m, wolf, BEAST_LORE);
            pass(m, 2000);
            (
                on_kobold,
                cast,
                on,
                still,
                has(m, wolf, BEAST_LORE),
                flag(m),
            )
        })
        .await
        .unwrap();
    let (on_kobold, cast, on, still, after, flag) = out;
    assert!(on_kobold.is_some(), "not at a kobold");
    assert!(cast, "at the wolf");
    assert_eq!(on, (true, SPECIAL_INFO), "its flag on");
    assert!(still && !after, "for thirty seconds");
    assert_eq!(flag, 0, "and its flag with it");
}
