//! Phase 5's diminishing returns, Thistle Tea's `DiminishingReturnsTest`
//! ported (the rest): groups apart, what never diminishes (a helpful
//! aura, a permanent one, one's own), roots only between players and what
//! they control, and a replaced holder keeping its group on.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::spells;
use crate::game::tests_p3_aura2::{add_spells, aura_spell};
use crate::game::tests_p3_combat::{BRUTE, START, setup_with, spawn};

const STUN: u32 = 97_100;
const ROOT: u32 = 97_101;
const FOREVER: u32 = 97_102;
const HELPFUL: u32 = 97_103;

fn control(id: u32, kind: u32, mechanic: u32, ms: Option<u32>) -> SpellTemplate {
    let mut s = aura_spell(id, ms, &[(kind, 0, 0)]);
    s.mechanic = mechanic;
    s.effects[0].target_a = 6;
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    let mut helpful = control(HELPFUL, AURA_MOD_STAT, 12, Some(8_000));
    helpful.effects[0].target_a = 21;
    add_spells(
        &mut d,
        [
            control(STUN, AURA_MOD_STUN, 12, Some(8_000)),
            control(ROOT, AURA_MOD_ROOT, 7, Some(8_000)),
            control(FOREVER, AURA_MOD_STUN, 12, None),
            helpful,
        ],
    );
    d
}

fn lasts(m: &Map, g: Guid, spell: u32) -> Option<Option<u32>> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| h.duration)
}

/// `by` casts `spell` at `on` twice, the first taken off between: how long
/// each lasted.
fn twice(
    m: &mut Map,
    by: Guid,
    on: Guid,
    spell: u32,
) -> (Option<Option<u32>>, Option<Option<u32>>) {
    spells::cast_triggered(m, by, spell, Some(on));
    let first = lasts(m, on, spell);
    auras::remove_spell(m, on, spell, None);
    spells::cast_triggered(m, by, spell, Some(on));
    let second = lasts(m, on, spell);
    auras::remove_spell(m, on, spell, None);
    (first, second)
}

fn wipe(m: &mut Map, g: Guid) {
    if let Some(c) = m.caster_mut(g) {
        c.diminishing.clear();
    }
}

/// Thistle Tea's groups and exemptions: a stun leaves a root whole (its
/// own group); a permanent stun, a helpful aura of the stun's mechanic and
/// one's own stun never diminish. A creature's stuns diminish; its roots
/// don't on a player, nor a player's on it. Between a player's pet and a
/// player roots diminish. A stun refreshed on its holder counts again and
/// keeps its group from recovering.
#[tokio::test]
async fn returns_go_by_group_and_only_between_players_for_roots() {
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            // A stun, then a root: the root whole.
            spells::cast_triggered(m, brute, STUN, Some(ga));
            spells::cast_triggered(m, brute, ROOT, Some(ga));
            let groups = (lasts(m, ga, STUN), lasts(m, ga, ROOT));
            auras::remove_where(m, ga, |_| true);
            wipe(m, ga);
            let forever = twice(m, brute, ga, FOREVER);
            let helpful = twice(m, gb, ga, HELPFUL);
            let own = twice(m, ga, ga, STUN);
            wipe(m, ga);
            // Creatures: stuns diminish, roots not.
            let creature_stun = twice(m, ga, brute, STUN);
            let creature_root = twice(m, ga, brute, ROOT);
            let player_root = twice(m, brute, ga, ROOT);
            // A player's pet roots Bob's pet.
            let mine = spawn(m, ga, BRUTE, 1, 3.0);
            let his = spawn(m, ga, BRUTE, 1, 4.0);
            m.creature_mut(mine).unwrap().owner = Some(ga);
            m.creature_mut(his).unwrap().owner = Some(gb);
            let pets = twice(m, mine, his, ROOT);
            // Refreshed while on.
            wipe(m, ga);
            spells::cast_triggered(m, brute, STUN, Some(ga));
            spells::cast_triggered(m, brute, STUN, Some(ga));
            let refreshed = (
                lasts(m, ga, STUN),
                m.caster(ga).map(|c| c.diminishing.clone()),
            );
            (
                groups,
                forever,
                helpful,
                own,
                creature_stun,
                creature_root,
                player_root,
                pets,
                refreshed,
            )
        })
        .await
        .unwrap();
    let (groups, forever, helpful, own, creature_stun, creature_root, player_root, pets, refreshed) =
        got;
    let full = Some(Some(8_000));
    let half = Some(Some(4_000));
    assert_eq!(groups, (full, full), "a group of its own");
    assert_eq!(forever, (Some(None), Some(None)), "permanent: no returns");
    assert_eq!(helpful, (full, full), "helpful: no returns");
    assert_eq!(own, (full, full), "one's own: no returns");
    assert_eq!(creature_stun, (full, half), "a creature's stuns diminish");
    assert_eq!(
        creature_root,
        (full, full),
        "a player's root on a creature doesn't"
    );
    assert_eq!(
        player_root,
        (full, full),
        "a creature's root on a player doesn't"
    );
    assert_eq!(pets, (full, half), "pets of players do");
    let (lasting, returns) = refreshed;
    assert_eq!(lasting, half, "refreshed, the second application's");
    assert_eq!(
        returns.map(|r| r
            .iter()
            .map(|(_, n, at)| (*n, at.is_none()))
            .collect::<Vec<_>>()),
        Some(vec![(2, true)]),
        "two counted, the group still on"
    );
}

/// A warrior's talents: all effects doubled, 10 more attack power, half
/// again attack power, 10 more haste; attack power auras of 100 (flat,
/// ranged, percent, ranged percent) and speed auras of 40 and −40 (attack
/// speed, melee haste, ranged haste, casting speed).
const ALL_DOUBLED: u32 = 97_110;
const AP_MORE: u32 = 97_111;
const AP_HALF_MORE: u32 = 97_112;
const HASTE_MORE: u32 = 97_113;
const AP_AURAS: [(u32, u32); 4] = [
    (97_120, AURA_MOD_ATTACK_POWER),
    (97_121, AURA_MOD_RANGED_ATTACK_POWER),
    (97_122, AURA_MOD_ATTACK_POWER_PCT),
    (97_123, AURA_MOD_RANGED_ATTACK_POWER_PCT),
];
const SPEED_AURAS: [(u32, u32); 4] = [
    (97_130, AURA_MOD_ATTACKSPEED),
    (97_131, AURA_MOD_HASTE),
    (97_132, AURA_MOD_RANGED_HASTE),
    (97_133, AURA_MOD_CASTING_SPEED),
];

fn modifier_data() -> GameData {
    use crate::game::map::spells::mods::*;
    let mut d = data();
    let talent = |id: u32, kind: u32, op: usize, amount: i32| {
        let mut s = aura_spell(id, None, &[(kind, op as i32, 0)]);
        s.family_name = 4;
        s.effects[0].item_type = 1;
        s.effects[0].base_points = amount - 1;
        s
    };
    let buff = |id: u32, kind: u32, ms: u32| {
        let mut s = aura_spell(id, Some(ms), &[(kind, 0, 0)]);
        s.family_name = 4;
        s.family_flags = 1;
        s.effects[0].base_points = 99;
        s
    };
    let mut all = vec![
        talent(ALL_DOUBLED, AURA_ADD_PCT_MODIFIER, OP_ALL_EFFECTS, 100),
        talent(AP_MORE, AURA_ADD_FLAT_MODIFIER, OP_ATTACK_POWER, 10),
        talent(AP_HALF_MORE, AURA_ADD_PCT_MODIFIER, OP_ATTACK_POWER, 50),
        talent(HASTE_MORE, AURA_ADD_FLAT_MODIFIER, OP_HASTE, 10),
    ];
    for (id, kind) in AP_AURAS {
        all.push(buff(id, kind, 10_000));
    }
    for (id, kind) in SPEED_AURAS {
        let mut fast = buff(id, kind, 10_000);
        fast.effects[0].base_points = 39;
        all.push(fast);
        let mut slow = buff(id + 10, kind, 10_000);
        slow.effects[0].base_points = -41;
        all.push(slow);
    }
    add_spells(&mut d, all);
    d.items.insert(
        BOW,
        crate::game::data::items::ItemTemplate {
            entry: BOW,
            class: 2,
            subclass: 2,
            name: "Bow".into(),
            display_id: 1000,
            inventory_type: 15,
            delay: 2000,
            stackable: 1,
            ..Default::default()
        },
    );
    d
}

/// A bow of two seconds.
const BOW: u32 = 97_140;

fn amount(m: &Map, g: Guid, spell: u32) -> Option<i32> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| h.effects[0].amount)
}

/// The attack power, melee and ranged, the client is shown: base and mods,
/// by the multiplier.
fn attack_power(m: &Map, g: Guid) -> (f32, f32) {
    use crate::game::fields::*;
    let f = m.unit_fields(g).unwrap();
    let ap = |base, mods, mult| {
        (f.get_u32(base) as f32 + f.get_u32(mods) as f32) * (1.0 + f.get_f32(mult))
    };
    (
        ap(
            UNIT_FIELD_ATTACK_POWER,
            UNIT_FIELD_ATTACK_POWER_MODS,
            UNIT_FIELD_ATTACK_POWER_MULTIPLIER,
        ),
        ap(
            UNIT_FIELD_RANGED_ATTACK_POWER,
            UNIT_FIELD_RANGED_ATTACK_POWER_MODS,
            UNIT_FIELD_RANGED_ATTACK_POWER_MULTIPLIER,
        ),
    )
}

/// Thistle Tea's modifiers on auras: all effects doubled, then 10 more
/// attack power: 100 is 210, on the attack power (flat or a percent,
/// melee or ranged) and back off when the aura goes. Ten more haste makes
/// 40 50 and −40 −30, on attack speed, melee and ranged haste and casting
/// speed. An aura keeps what its caster's talents made it when it went on,
/// until it is cast again or runs out.
#[tokio::test]
async fn talents_change_auras_amounts_as_they_go_on_and_keep_them() {
    use crate::game::fields::*;
    let (_realm, _a, ga, map) = setup_with(modifier_data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let cast = |m: &mut Map, spell: u32| {
                crate::game::tests_p3_spells_kit::fresh(m, ga);
                spells::cast_triggered(m, ga, spell, Some(ga));
            };
            crate::game::tests_p3_combat::equip(m, ga, BOW, 17);
            let base = attack_power(m, ga);
            for t in [ALL_DOUBLED, AP_MORE] {
                cast(m, t);
            }
            let mut ap = Vec::new();
            for (id, _) in AP_AURAS {
                cast(m, id);
                ap.push((amount(m, ga, id), attack_power(m, ga)));
                auras::remove_spell(m, ga, id, None);
                ap.push((None, attack_power(m, ga)));
            }
            for t in [ALL_DOUBLED, AP_MORE] {
                auras::remove_spell(m, ga, t, None);
            }
            cast(m, HASTE_MORE);
            let mut speeds = Vec::new();
            for (id, _) in SPEED_AURAS {
                for spell in [id, id + 10] {
                    cast(m, spell);
                    let f = m.unit_fields(ga).unwrap();
                    speeds.push((
                        amount(m, ga, spell),
                        f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                        f.get_u32(UNIT_FIELD_RANGEDATTACKTIME),
                        f.get_f32(UNIT_MOD_CAST_SPEED),
                    ));
                    auras::remove_spell(m, ga, spell, None);
                }
            }
            auras::remove_spell(m, ga, HASTE_MORE, None);
            let plain = {
                let f = m.unit_fields(ga).unwrap();
                (
                    f.get_u32(UNIT_FIELD_BASEATTACKTIME),
                    f.get_u32(UNIT_FIELD_RANGEDATTACKTIME),
                )
            };
            // A snapshot.
            let (flat, _) = AP_AURAS[0];
            cast(m, AP_HALF_MORE);
            cast(m, flat);
            let talented = amount(m, ga, flat);
            auras::remove_spell(m, ga, AP_HALF_MORE, None);
            let kept = (amount(m, ga, flat), attack_power(m, ga).0);
            cast(m, flat);
            let again = (amount(m, ga, flat), attack_power(m, ga).0);
            (base, ap, speeds, plain, talented, kept, again)
        })
        .await
        .unwrap();
    let (base, ap, speeds, plain, talented, kept, again) = got;
    let (melee, ranged) = base;
    let want = [
        (melee + 210.0, ranged),
        (melee, ranged + 210.0),
        (melee * 3.1, ranged),
        (melee, ranged * 3.1),
    ];
    for (i, w) in want.iter().enumerate() {
        let (got_amount, got_ap) = ap[2 * i];
        assert_eq!(got_amount, Some(210), "aura {i}");
        assert!(
            (got_ap.0 - w.0).abs() < 0.5 && (got_ap.1 - w.1).abs() < 0.5,
            "aura {i}: {got_ap:?} for {w:?}"
        );
        assert_eq!(ap[2 * i + 1].1, base, "aura {i} gone");
    }
    let (attack, shot) = plain;
    for (i, (amount, attack_time, ranged_time, cast_speed)) in speeds.into_iter().enumerate() {
        let (kind, fast) = (i / 2, i % 2 == 0);
        assert_eq!(amount, Some(if fast { 50 } else { -30 }), "speed {kind}");
        let factor = if fast { 1.0 / 1.5 } else { 1.3 };
        match kind {
            0 | 1 => assert_eq!(attack_time, (attack as f32 * factor) as u32, "speed {kind}"),
            2 => assert_eq!(ranged_time, (shot as f32 * factor) as u32, "ranged"),
            _ => assert!((cast_speed - factor).abs() < 1e-4, "casting: {cast_speed}"),
        }
    }
    assert_eq!(talented, Some(150));
    assert_eq!(kept, (Some(150), melee + 150.0), "the snapshot kept");
    assert_eq!(again, (Some(100), melee + 100.0), "cast again, plain");
}
