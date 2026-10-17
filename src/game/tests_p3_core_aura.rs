//! Phase 3's auras, Thistle Tea's `AuraTest` ported: what a player's client
//! is told of an aura (its duration, at login too, its spell modifiers),
//! the slots and the fields an aura's effects change (resistances, stats
//! and pools, speeds, flags), roots coming and going, absorbs, cancelling,
//! and Polymorph's and the forms' looks.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{self as sd, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::tests_p3_combat::{self as base, BRUTE, setup_with, spawn};

/// Frost Armor's armor, for ten minutes.
const FROST_ARMOR: u32 = 168;
/// A helpful aura with no end.
const LASTING: u32 = 95_900;
/// A ward of thirty seconds, and one of five.
const WARD: u32 = 95_901;
const BRIEF: u32 = 95_902;
/// A passive's spell modifier: a tenth of a second off the casts of
/// family flag 5.
const QUICKEN: u32 = 11_070;
/// The same, shown (a passive with a visual).
const QUICKEN_SHOWN: u32 = 95_903;
/// Frost Nova's root, eight seconds.
const NOVA: u32 = 122;
const SLOW_FALL: u32 = 130;
/// Spirit of Redemption's unattackable state.
const UNTOUCHABLE: u32 = 27_792;
/// Slows of half and seven tenths, eight seconds; Chilled's three tenths,
/// five.
const CHILL: u32 = 120;
const COLD_SNAP: u32 = 11_113;
const CHILLED: u32 = 6136;
const ARCANE_INTELLECT: u32 = 1459;
const VIGOR: u32 = 21_369;
const SURVIVALIST: u32 = 20_579;
const FIRE_WARD: u32 = 543;
const POLYMORPH: u32 = 118;
/// Polymorph's sheep, and its look.
const SHEEP: u32 = 16_372;
const SHEEP_DISPLAY: u32 = 856;
/// Frost Armor that can't be cancelled, and one that is passive.
const STUCK_ARMOR: u32 = 95_904;
const PASSIVE_ARMOR: u32 = 95_905;

const SCHOOL_FIRE: u32 = 0x4;
const SCHOOL_FROST: u32 = 0x10;

/// A spell of aura effects, each an aura type and its misc value, lasting
/// `ms` (none for no end).
fn aura_spell(id: u32, ms: Option<u32>, effects: &[(u32, i32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Aura {id}"),
        equipped_item_class: -1,
        duration_index: ms.unwrap_or(0),
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
    let mut d = base::data();
    // A level-1 mage, rogue and druid of 20 in each stat: 80 health and
    // 80 mana before their stats (Thistle Tea's fixtures).
    for class in [4u8, 8, 11] {
        for race in [1u8, 4] {
            d.player_level_stats
                .insert((race, class, 1), [20, 20, 20, 20, 20]);
        }
        d.player_class_stats
            .insert((class, 1), (60, if class == 4 { 0 } else { 80 }));
    }
    let mut sheep = base::template(SHEEP, 1);
    sheep.display_ids = [SHEEP_DISPLAY, 0, 0, 0];
    d.creature_templates.insert(SHEEP, sheep);
    let ten_minutes = Some(600_000);
    let mut all = vec![
        aura_spell(FROST_ARMOR, ten_minutes, &[(AURA_MOD_RESISTANCE, 1)]),
        aura_spell(LASTING, None, &[(AURA_MOD_STAT, 0)]),
        aura_spell(WARD, Some(30_000), &[(AURA_MOD_RESISTANCE, 4)]),
        aura_spell(BRIEF, Some(5_000), &[(AURA_MOD_RESISTANCE, 8)]),
        aura_spell(NOVA, Some(8_000), &[(AURA_MOD_ROOT, 0)]),
        aura_spell(SLOW_FALL, Some(30_000), &[(AURA_FEATHER_FALL, 0)]),
        aura_spell(UNTOUCHABLE, Some(15_000), &[(AURA_MOD_UNATTACKABLE, 0)]),
        aura_spell(CHILL, Some(8_000), &[(AURA_MOD_DECREASE_SPEED, 0)]),
        aura_spell(COLD_SNAP, Some(8_000), &[(AURA_MOD_DECREASE_SPEED, 0)]),
        aura_spell(CHILLED, Some(5_000), &[(AURA_MOD_DECREASE_SPEED, 0)]),
        aura_spell(ARCANE_INTELLECT, Some(1_800_000), &[(AURA_MOD_STAT, 3)]),
        aura_spell(VIGOR, None, &[(AURA_MOD_INCREASE_ENERGY, 3)]),
        aura_spell(SURVIVALIST, None, &[(AURA_MOD_INCREASE_HEALTH_PERCENT, 0)]),
        aura_spell(
            FIRE_WARD,
            Some(30_000),
            &[(AURA_SCHOOL_ABSORB, SCHOOL_FIRE as i32)],
        ),
        aura_spell(
            POLYMORPH,
            Some(20_000),
            &[(AURA_MOD_CONFUSE, 0), (AURA_TRANSFORM, SHEEP as i32)],
        ),
    ];
    for id in [QUICKEN, QUICKEN_SHOWN] {
        let mut s = aura_spell(
            id,
            None,
            &[(
                AURA_ADD_FLAT_MODIFIER,
                crate::game::map::spells::mods::OP_CASTING_TIME as i32,
            )],
        );
        s.effects[0].item_type = 1 << 5;
        s.attributes[0] |= sd::ATTR_PASSIVE;
        s.family_name = 3;
        s.spell_visual = u32::from(id == QUICKEN_SHOWN);
        all.push(s);
    }
    let mut stuck = aura_spell(STUCK_ARMOR, ten_minutes, &[(AURA_MOD_RESISTANCE, 1)]);
    stuck.attributes[0] |= sd::ATTR_CANT_CANCEL;
    let mut passive = aura_spell(PASSIVE_ARMOR, ten_minutes, &[(AURA_MOD_RESISTANCE, 1)]);
    passive.attributes[0] |= sd::ATTR_PASSIVE;
    all.extend([stuck, passive]);
    for s in all {
        if s.duration_index != 0 {
            d.spells.durations.insert(
                s.duration_index,
                SpellDuration {
                    base: s.duration_index as i32,
                    ..Default::default()
                },
            );
        }
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Put `id` on `target` from `caster`, each of its effects with its amount.
fn put(m: &mut Map, caster: Guid, target: Guid, id: u32, amounts: &[i32]) -> Result<(), Refused> {
    let s = m.realm.data.spells.get(id).cloned().expect("the spell");
    let effects: Vec<(usize, i32)> = amounts.iter().copied().enumerate().collect();
    auras::apply_spell(m, caster, target, &s, &effects, Context::default())
}

/// `g`'s holders: each spell, its caster and its slot.
fn holders(m: &Map, g: Guid) -> Vec<(u32, Guid, Option<usize>)> {
    auras::auras(m, g).map_or_else(Vec::new, |a| {
        a.holders
            .iter()
            .map(|h| (h.spell, h.caster, h.slot))
            .collect()
    })
}

fn field(m: &Map, g: Guid, index: u16) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(index))
}

/// Time passes on the map: its auras run out and tick.
fn pass(m: &mut Map, by: Duration) {
    m.now += by;
    auras::update(m);
}

fn durations(m: &ServerOpcodeMessage) -> Option<(u8, u32)> {
    match m {
        ServerOpcodeMessage::SMSG_UPDATE_AURA_DURATION(d) => Some((d.aura_slot, d.aura_duration)),
        _ => None,
    }
}

/// Thistle Tea's `apply_spell/5` (the duration for player targets, none
/// for mobs) and `self_duration_events/2` (none without an end): an aura
/// with an end on a player tells its client the slot and the whole
/// duration; one on a creature and one with no end tell no one.
#[tokio::test]
async fn a_players_timed_aura_tells_its_client_its_slot_and_duration() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    a.clear();
    map.query(move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        put(m, ga, brute, FROST_ARMOR, &[29]).unwrap();
        put(m, ga, ga, LASTING, &[5]).unwrap();
        put(m, ga, ga, FROST_ARMOR, &[29]).unwrap();
        assert_eq!(holders(m, ga)[1].2, Some(1));
    })
    .await
    .unwrap();
    let told = a.wait_for("SMSG_UPDATE_AURA_DURATION", durations).await;
    assert_eq!(told, (1, 600_000));
    assert_eq!(a.gets(Duration::from_millis(200), durations).await, None);
}

/// Thistle Tea's `self_duration_events/2`: at login a player is told what
/// is left of each aura, not its whole duration. One that ran out before
/// the save isn't kept (as mangos's `_SaveAuras`), so nothing is told of it.
#[tokio::test]
async fn the_duration_told_at_login_is_what_was_left() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    a.clear();
    let back = map
        .query(move |m| {
            put(m, ga, ga, WARD, &[10]).unwrap();
            put(m, ga, ga, BRIEF, &[10]).unwrap();
            put(m, ga, ga, LASTING, &[5]).unwrap();
            m.now += Duration::from_secs(10);
            let now = m.now;
            let row = m.players.get_mut(&ga).unwrap().to_row(now);
            for s in [WARD, BRIEF, LASTING] {
                auras::remove_spell(m, ga, s, None);
            }
            m.players.get_mut(&ga).unwrap().row.state.auras = row.state.auras;
            crate::game::map::spells::cooldowns::restore(m, ga);
            holders(m, ga)
                .into_iter()
                .map(|h| (h.0, h.2))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert_eq!(back, [(WARD, Some(0)), (LASTING, Some(1))]);
    let mut told = Vec::new();
    while let Some(d) = a.gets(Duration::from_millis(200), durations).await {
        told.push(d);
    }
    assert_eq!(told, [(0, 30_000), (1, 5_000), (0, 20_000)]);
}

/// Thistle Tea's `apply_spell/5` and `expire_due/2`: Frost Armor's armor
/// goes on the armor field and no other, on top of the unit's own, and
/// comes off as the aura runs out, the unit's own kept.
#[tokio::test]
async fn mod_resistance_goes_on_its_schools_field_over_the_units_own() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            let fields = |m: &Map| {
                (
                    field(m, brute, UNIT_FIELD_RESISTANCES),
                    field(m, brute, UNIT_FIELD_RESISTANCES + 1),
                    m.creature(brute).unwrap().base.resistances[0],
                )
            };
            let mut out = Vec::new();
            put(m, ga, brute, FROST_ARMOR, &[29]).unwrap();
            out.push(fields(m));
            auras::remove_spell(m, brute, FROST_ARMOR, None);
            m.creature_mut(brute).unwrap().base.resistances[0] = 7;
            auras::recalc(m, brute);
            put(m, ga, brute, FROST_ARMOR, &[29]).unwrap();
            out.push(fields(m));
            pass(m, Duration::from_secs(601));
            out.push(fields(m));
            (out, holders(m, brute).len())
        })
        .await
        .unwrap();
    assert_eq!(got.0, [(29, 0, 0), (36, 0, 7), (7, 0, 7)]);
    assert_eq!(got.1, 0);
}

/// Thistle Tea's `apply_spell/5`: a spell modifier aura tells the client
/// the modifier's new sum as it comes (`SMSG_SET_FLAT_SPELL_MODIFIER`: the
/// family bit, the operation, the amount) and as it goes (nothing).
#[tokio::test]
async fn a_spell_modifier_is_told_as_it_comes_and_as_it_goes() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    a.clear();
    let op = crate::game::map::spells::mods::OP_CASTING_TIME as u8;
    let told = |amount: i32| [&[5u8, op][..], &amount.to_le_bytes()].concat();
    map.query(move |m| put(m, ga, ga, QUICKEN, &[-100]).unwrap())
        .await
        .unwrap();
    let on = a
        .wait_frame("SMSG_SET_FLAT_SPELL_MODIFIER", 0x266, |_| true)
        .await;
    assert_eq!(on, told(-100));
    map.query(move |m| auras::remove_spell(m, ga, QUICKEN, None))
        .await
        .unwrap();
    let off = a
        .wait_frame("SMSG_SET_FLAT_SPELL_MODIFIER", 0x266, |_| true)
        .await;
    assert_eq!(off, told(0));
}

/// Thistle Tea's `apply_spell/5`: a passive with no visual holds no slot
/// and stays off the aura fields, so the next aura takes the first slot; a
/// passive with a visual is shown.
#[tokio::test]
async fn a_passive_is_shown_only_with_a_visual() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            put(m, ga, ga, QUICKEN, &[-100]).unwrap();
            put(m, ga, ga, FROST_ARMOR, &[29]).unwrap();
            put(m, ga, ga, QUICKEN_SHOWN, &[-100]).unwrap();
            (
                holders(m, ga),
                field(m, ga, UNIT_FIELD_AURA),
                field(m, ga, UNIT_FIELD_AURA + 1),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        [
            (QUICKEN, ga, None),
            (FROST_ARMOR, ga, Some(0)),
            (QUICKEN_SHOWN, ga, Some(1))
        ]
    );
    assert_eq!((got.1, got.2), (FROST_ARMOR, QUICKEN_SHOWN));
}

/// Thistle Tea's `apply_spell/5` and `expire_due/2` on roots: a player
/// rooted is told so; as the root runs out they are told they're free,
/// whatever their client's movement flags said in between.
#[tokio::test]
async fn a_root_running_out_frees_the_player_whatever_their_flags() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    a.clear();
    map.query(move |m| put(m, ga, ga, NOVA, &[0]).unwrap())
        .await
        .unwrap();
    a.wait_frame("SMSG_FORCE_MOVE_ROOT", 0x0E8, |_| true).await;
    // The client's own flags, rooted no more.
    let (x, y, z) = base::START;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_forward(),
            timestamp: 0,
            position: Vector3d { x, y, z },
            orientation: 0.0,
            fall_time: 0.0,
        },
    })
    .await;
    let early = a
        .gets(Duration::from_millis(100), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(_)).then_some(())
        })
        .await;
    assert!(early.is_none(), "still rooted");
    let gone = map
        .query(move |m| {
            pass(m, Duration::from_secs(9));
            holders(m, ga).is_empty()
        })
        .await
        .unwrap();
    assert!(gone);
    a.wait_frame("SMSG_FORCE_MOVE_UNROOT", 0x0EA, |_| true)
        .await;
}

/// Thistle Tea's `feather_fall`: Slow Fall tells its client to fall
/// slowly as it comes and to fall as ever as it runs out.
#[tokio::test]
async fn slow_fall_is_told_as_it_comes_and_goes() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    a.clear();
    map.query(move |m| put(m, ga, ga, SLOW_FALL, &[0]).unwrap())
        .await
        .unwrap();
    a.wait_for("SMSG_MOVE_FEATHER_FALL", |m| match m {
        ServerOpcodeMessage::SMSG_MOVE_FEATHER_FALL(f) => (f.guid == ga).then_some(()),
        _ => None,
    })
    .await;
    map.query(move |m| pass(m, Duration::from_secs(31)))
        .await
        .unwrap();
    a.wait_for("SMSG_MOVE_NORMAL_FALL", |m| match m {
        ServerOpcodeMessage::SMSG_MOVE_NORMAL_FALL(f) => (f.guid == ga).then_some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's `unattackable auras`: an unattackable aura sets the unit's
/// not-attackable flag, and its running out clears it.
#[tokio::test]
async fn an_unattackable_aura_flags_the_unit_while_it_lasts() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let flagged = |m: &Map| field(m, ga, UNIT_FIELD_FLAGS) & 0x0001_0000 != 0;
            put(m, ga, ga, UNTOUCHABLE, &[0]).unwrap();
            let on = flagged(m);
            pass(m, Duration::from_secs(15));
            (on, flagged(m))
        })
        .await
        .unwrap();
    assert_eq!(got, (true, false));
}

fn run_speed(m: &Map, g: Guid) -> f32 {
    m.players[&g].speeds.run
}

/// Thistle Tea's `movement speed modifiers`: of two slows only the
/// stronger counts, and a slow running out gives the player their speed
/// back, told to their client.
#[tokio::test]
async fn only_the_strongest_slow_counts_and_the_speed_comes_back() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            put(m, ga, ga, CHILL, &[-50]).unwrap();
            put(m, ga, ga, COLD_SNAP, &[-70]).unwrap();
            let both = run_speed(m, ga);
            for s in [CHILL, COLD_SNAP] {
                auras::remove_spell(m, ga, s, None);
            }
            put(m, ga, ga, CHILLED, &[-30]).unwrap();
            let chilled = run_speed(m, ga);
            (both, chilled)
        })
        .await
        .unwrap();
    assert!((got.0 - 7.0 * 0.3).abs() < 1e-4, "{}", got.0);
    assert!((got.1 - 7.0 * 0.7).abs() < 1e-4, "{}", got.1);
    a.clear();
    let after = map
        .query(move |m| {
            pass(m, Duration::from_secs(6));
            run_speed(m, ga)
        })
        .await
        .unwrap();
    assert_eq!(after, 7.0);
    let told = a
        .wait_for("SMSG_FORCE_RUN_SPEED_CHANGE", |m| match m {
            ServerOpcodeMessage::SMSG_FORCE_RUN_SPEED_CHANGE(s) => Some(s.speed),
            _ => None,
        })
        .await;
    assert_eq!(told, 7.0);
}

/// Thistle Tea's `mod_stat`: Arcane Intellect's two intellect raise the
/// maximum mana by fifteen a point, the mana there is kept; as it runs out
/// both go back.
#[tokio::test]
async fn arcane_intellect_raises_intellect_and_the_most_mana_until_it_ends() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let stats = |m: &Map| {
                (
                    field(m, ga, UNIT_FIELD_STAT0 + 3),
                    field(m, ga, UNIT_FIELD_MAXPOWER1),
                    field(m, ga, UNIT_FIELD_POWER1),
                )
            };
            m.unit_fields_mut(ga)
                .unwrap()
                .set_u32(UNIT_FIELD_POWER1, 100);
            let before = stats(m);
            put(m, ga, ga, ARCANE_INTELLECT, &[2]).unwrap();
            let on = stats(m);
            pass(m, Duration::from_secs(1801));
            (before, on, stats(m))
        })
        .await
        .unwrap();
    assert_eq!(got.0, (20, 100, 100));
    assert_eq!(got.1, (22, 130, 100));
    assert_eq!(got.2, (20, 100, 100));
}

/// Thistle Tea's `talent effect plumbing`: Vigor's ten energy and
/// Survivalist's tenth more health raise a rogue's pools.
#[tokio::test]
async fn pool_auras_raise_the_most_energy_and_health() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Rogue).await;
    let got = map
        .query(move |m| {
            let pools = |m: &Map| {
                (
                    field(m, ga, UNIT_FIELD_MAXPOWER1 + 3),
                    field(m, ga, UNIT_FIELD_MAXHEALTH),
                )
            };
            let before = pools(m);
            put(m, ga, ga, VIGOR, &[10]).unwrap();
            put(m, ga, ga, SURVIVALIST, &[10]).unwrap();
            (before, pools(m))
        })
        .await
        .unwrap();
    assert_eq!(got, ((100, 80), (110, 88)));
}

/// Thistle Tea's `absorb_damage/4`: Fire Ward soaks fire, what it has left
/// kept; frost passes it by; spent, it is gone and the rest gets through.
#[tokio::test]
async fn fire_ward_soaks_fire_until_spent() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let left = |m: &Map| {
                auras::auras(m, ga)
                    .and_then(|a| a.holders.iter().find(|h| h.spell == FIRE_WARD))
                    .map(|h| h.effects[0].left)
            };
            put(m, ga, ga, FIRE_WARD, &[164]).unwrap();
            let fire = auras::absorb(m, ga, 100, SCHOOL_FIRE);
            let after_fire = left(m);
            let frost = auras::absorb(m, ga, 100, SCHOOL_FROST);
            auras::remove_spell(m, ga, FIRE_WARD, None);
            put(m, ga, ga, FIRE_WARD, &[164]).unwrap();
            let spent = auras::absorb(m, ga, 200, SCHOOL_FIRE);
            (fire, after_fire, frost, spent, left(m))
        })
        .await
        .unwrap();
    assert_eq!(got.0, (0, 100));
    assert_eq!(got.1, Some(64));
    assert_eq!(got.2, (100, 0));
    assert_eq!(got.3, (36, 164));
    assert_eq!(got.4, None);
}

/// Thistle Tea's `mod_confuse`: Polymorph is harmful; its sheep's look
/// goes on, and as damage breaks it the unit's own comes back.
#[tokio::test]
async fn polymorph_is_harmful_and_its_look_goes_as_damage_breaks_it() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            let own = field(m, brute, UNIT_FIELD_NATIVEDISPLAYID);
            if let Some(s) = m.realm.data.spells.get(POLYMORPH).cloned() {
                let mut s = s;
                s.aura_interrupt_flags = INTERRUPT_DAMAGE;
                let _ = auras::apply_spell(m, ga, brute, &s, &[(0, 0), (1, 0)], Context::default());
            }
            let positive = auras::auras(m, brute).unwrap().holders[0].positive;
            let sheep = field(m, brute, UNIT_FIELD_DISPLAYID);
            crate::game::map::combat::deal_damage(m, ga, brute, 10);
            (
                own,
                positive,
                sheep,
                field(m, brute, UNIT_FIELD_DISPLAYID),
                holders(m, brute).len(),
            )
        })
        .await
        .unwrap();
    assert!(!got.1, "harmful");
    assert_eq!(got.2, SHEEP_DISPLAY);
    assert_eq!((got.3, got.4), (got.0, 0));
}

/// Thistle Tea's `cancel_spell/3`: a player cancels a helpful aura; not a
/// harmful one, one that can't be cancelled, a passive, nor a spell they
/// don't have on.
#[tokio::test]
async fn only_a_cancelable_helpful_aura_is_cancelled() {
    let (_realm, a, ga, map) = setup_with(data(), Class::Mage).await;
    map.query(move |m| {
        for id in [NOVA, STUCK_ARMOR, PASSIVE_ARMOR, FROST_ARMOR] {
            put(m, ga, ga, id, &[1]).unwrap();
        }
    })
    .await
    .unwrap();
    for id in [NOVA, STUCK_ARMOR, PASSIVE_ARMOR, 99_999, FROST_ARMOR] {
        a.send(CMSG_CANCEL_AURA { id }).await;
    }
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        let left: Vec<u32> = map
            .query(move |m| holders(m, ga).into_iter().map(|h| h.0).collect())
            .await
            .unwrap();
        if !left.contains(&FROST_ARMOR) {
            assert_eq!(left, [NOVA, STUCK_ARMOR, PASSIVE_ARMOR]);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "not cancelled");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
