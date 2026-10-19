//! Phase 7's loader round, the spells (Thistle Tea's `world/loader/*_dbc_test.exs`
//! of spells): what wvs reads of the real spells, and what they do.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::auras;
use crate::game::map::combat::Outcome;
use crate::game::map::spells::{self, outcome};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::forced_swing;

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn spell(d: &GameData, id: u32) -> &SpellTemplate {
    d.spells.get(id).unwrap_or_else(|| panic!("spell {id}"))
}

fn cast_ms(d: &GameData, id: u32) -> i32 {
    d.spells.cast_times[&spell(d, id).casting_time_index]
}

fn duration_ms(d: &GameData, id: u32) -> i32 {
    d.spells.durations[&spell(d, id).duration_index].base
}

/// Thistle Tea's spell classes (`spell_binary_dbc_test.exs`,
/// `spell_avoidance_dbc_test.exs`, `spell_cone_vmangos_test.exs`,
/// `spell_target_limit_dbc_test.exs`, `spell_extra_attacks_dbc_test.exs`),
/// on the real spells: Frostbolt, Frost Nova, Earth Shock, Counterspell,
/// Mind Flay and the two of no slow (26143, 26478) are all or nothing;
/// Fireball, Shadow Bolt, Corruption and Arcane Missiles resist in part.
/// Blizzard, Cone of Cold, Frost Nova, Arcane Explosion, Flamestrike,
/// Blast Wave and the other bursts, cones and ground spells are areas;
/// Frostbolt, Fireball, Chain Lightning, Chain Heal and Arcane Missiles
/// aren't. vmangos's cones: 7 degrees, 90, and 120 behind; the rest 60.
/// Each rank's own most targets, none for Frost Nova or Blizzard. Effect
/// 19's extra attacks: Thrash 2, 15601 1, 15642 3, 16459 1, Windfury 2,
/// Reckoning 1.
#[test]
#[ignore = "extended"]
fn real_spells_are_binary_areas_cones_and_limits_as_loaded() {
    let Some(d) = real() else {
        return;
    };
    for id in [116, 122, 8042, 2139, 15_407, 26_143, 26_478] {
        assert!(outcome::binary(spell(d, id)), "{id} is binary");
    }
    for id in [133, 686, 172, 5143] {
        assert!(!outcome::binary(spell(d, id)), "{id} isn't");
    }
    for id in [
        10, 120, 122, 1449, 2120, 19_272, 21_333, 24_820, 26_102, 26_573,
    ] {
        assert!(outcome::area(spell(d, id)), "{id} is an area");
    }
    for id in [116, 133, 421, 1064, 5143] {
        assert!(!outcome::area(spell(d, id)), "{id} isn't");
    }
    assert_eq!(d.spells.cones.get(&24_933), Some(&7));
    assert_eq!(d.spells.cones.get(&25_030), Some(&90));
    assert_eq!(d.spells.cones.get(&15_847), Some(&-120));
    assert_eq!(d.spells.cones.get(&120), None, "Cone of Cold's is 60");
    for (id, most) in [
        (1680, 4),
        (5246, 5),
        (5484, 5),
        (6343, 4),
        (8122, 2),
        (8124, 3),
        (10_888, 4),
        (10_890, 5),
        (11_581, 4),
        (17_928, 5),
        (20_549, 5),
        (24_083, 4),
        (122, 0),
        (10, 0),
    ] {
        assert_eq!(spell(d, id).max_affected_targets, most, "{id}");
    }
    for (id, count) in [
        (3391, 2),
        (15_601, 1),
        (15_642, 3),
        (16_459, 1),
        (8233, 2),
        (20_178, 1),
    ] {
        let s = spell(d, id);
        let i = s.effects.iter().position(|e| e.effect == 19).unwrap();
        assert_eq!(spells::points(s, i, 60), count, "{id}");
    }
}

/// Thistle Tea's avoidance and binary resists (`spell_avoidance_dbc_test.exs`,
/// `spell_binary_dbc_test.exs`) and vmangos's `MagicSpellHitResult`: the
/// Avoidance passive (23198) is 25 points off an area spell's hit
/// (Blizzard's), none off Frostbolt's; 300 frost resistance makes
/// Frostbolt (all or nothing) miss more, and 300 of the caster's
/// penetration takes it all away again; at itself it never misses.
#[tokio::test]
#[ignore = "extended"]
async fn avoidance_takes_areas_and_resistance_takes_binary_spells() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P7LAVOID", "Avoida", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let d = m.realm.data.clone();
        let foe = dummy(m, g, 60, 5.0);
        let hit = |m: &mut Map, id: u32| {
            let s = d.spells.get(id).unwrap().clone();
            let mods = spells::mods::for_spell(m, g, &s);
            outcome::spell_hit_chance(m, g, foe, &s, &mods)
        };
        let avoidance = d.spells.get(23_198).unwrap();
        let passive = (
            avoidance.passive(),
            avoidance.effects[0].aura,
            spells::points(avoidance, 0, 60),
        );
        let before = (hit(m, 10), hit(m, 116));
        apply(m, foe, foe, 23_198);
        let avoided = (hit(m, 10), hit(m, 116));
        auras::remove_spell(m, foe, 23_198, None);
        if let Some(f) = m.unit_fields_mut(foe) {
            f.set_u32(UNIT_FIELD_RESISTANCES + 4, 300);
        }
        let resisted = hit(m, 116);
        let pen = made_up(990_001, 6, auras::AURA_MOD_TARGET_RESISTANCE, 1 << 4, -300);
        apply_template(m, g, g, &pen);
        let pierced = hit(m, 116);
        let s = d.spells.get(116).unwrap().clone();
        let mods = spells::mods::for_spell(m, g, &s);
        let own = outcome::roll_hit(m, g, g, &s, &mods);
        (passive, before, avoided, resisted, pierced, own)
    })
    .await;
    let (passive, before, avoided, resisted, pierced, own) = out;
    assert_eq!(passive, (true, auras::AURA_MOD_AOE_AVOIDANCE, 25));
    assert_eq!(avoided.0, before.0 - 2500, "an area, 25 points less");
    assert_eq!(avoided.1, before.1, "Frostbolt isn't an area");
    assert!(resisted < before.1, "{resisted} < {}", before.1);
    assert_eq!(pierced, before.1, "penetration takes the resistance away");
    assert_eq!(own, outcome::Hit::Hit);
}

/// Thistle Tea's extra attacks (`spell_extra_attacks_dbc_test.exs`) and
/// mangos's `m_extraAttacks`: Thrash (3391) owes two attacks; a second
/// while they are owed, or one during an extra attack, owes no more.
#[tokio::test]
#[ignore = "extended"]
async fn extra_attacks_bring_no_more_extra_attacks() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P7LEXTRA", "Extra", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let owed = |m: &Map| m.combat(g).map_or(0, |c| c.extra_attacks);
        let mut seen = Vec::new();
        for id in [3391, 15_601, 15_642, 16_459, 8233] {
            if let Some(c) = m.combat_mut(g) {
                c.extra_attacks = 0;
                c.in_extra_attack = false;
            }
            spells::cast_triggered(m, g, id, Some(g));
            let first = owed(m);
            spells::cast_triggered(m, g, id, Some(g));
            let again = owed(m);
            if let Some(c) = m.combat_mut(g) {
                c.extra_attacks = 0;
                c.in_extra_attack = true;
            }
            spells::cast_triggered(m, g, id, Some(g));
            seen.push((id, first, again, owed(m)));
        }
        seen
    })
    .await;
    assert_eq!(
        out,
        vec![
            (3391, 2, 2, 0),
            (15_601, 1, 1, 0),
            (15_642, 3, 3, 0),
            (16_459, 1, 1, 0),
            (8233, 2, 2, 0),
        ]
    );
}

const DESERTER: u32 = 26_013;
const HONORLESS_TARGET: u32 = 2479;

/// Thistle Tea's honor spells (`spell_honor_dbc_test.exs`): Deserter
/// lasts 15 minutes, a negative dummy aura that death doesn't take and
/// no dispel does; Honorless Target 30 seconds on its caster, ended by
/// attacking (`AURA_INTERRUPT_FLAG_ATTACKING`: a swing takes it, as
/// mangos's `AttackerStateUpdate`); Warsong's mark spells (24950, 24951)
/// give no honor of their own.
#[tokio::test]
#[ignore = "extended"]
async fn deserter_and_honorless_target_last_as_loaded() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let deserter = spell(&d, DESERTER);
    assert_eq!(duration_ms(&d, DESERTER), 900_000);
    assert!(!auras::positive(deserter));
    assert!(deserter.attr(3, crate::game::data::spells::ATTR_EX3_DEATH_PERSISTENT));
    assert_eq!(deserter.dispel, 0);
    let live: Vec<(u32, u32)> = deserter
        .effects
        .iter()
        .filter(|e| e.effect != 0)
        .map(|e| (e.effect, e.aura))
        .collect();
    assert_eq!(live, vec![(6, 4)]);
    let honorless = spell(&d, HONORLESS_TARGET);
    assert_eq!(duration_ms(&d, HONORLESS_TARGET), 30_000);
    assert_eq!(honorless.aura_interrupt_flags, auras::INTERRUPT_ATTACK);
    assert_eq!(honorless.effects[0].target_a, 1);
    for id in [24_950, 24_951] {
        assert!(!spell(&d, id).has_effect(45), "{id} gives no honor");
    }

    let (_c, g, map) = player(&realm, "P7LHONOR", "Honora", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let foe = dummy(m, g, 60, 2.0);
        apply(m, g, g, DESERTER);
        apply(m, g, g, HONORLESS_TARGET);
        let on_both = (has(m, g, DESERTER), has(m, g, HONORLESS_TARGET));
        forced_swing(m, g, foe, Outcome::Hit);
        (on_both, has(m, g, DESERTER), has(m, g, HONORLESS_TARGET))
    })
    .await;
    assert_eq!(out, ((true, true), true, false), "a swing ends it");
}

/// Thistle Tea's home and disenchanting (`spell_home_bind_dbc_test.exs`,
/// `spell_disenchant_dbc_test.exs`): the Hearthstone (8690) and Astral
/// Recall (556) take ten seconds and teleport their caster to their home
/// bind (target 9); Disenchant (13262) takes three seconds, is enchanting's (333),
/// yellow at 20 and grey at 60.
#[tokio::test]
#[ignore = "extended"]
async fn the_hearthstone_and_astral_recall_go_home() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for id in [8690, 556] {
        assert_eq!(cast_ms(&d, id), 10_000, "{id}");
        let e = &spell(&d, id).effects[0];
        assert_eq!((e.effect, e.target_a, e.target_b), (5, 1, 9), "{id}");
    }
    let de = spell(&d, 13_262);
    assert_eq!(
        (de.name.as_str(), cast_ms(&d, 13_262)),
        ("Disenchant", 3000)
    );
    assert_eq!(de.effects[0].effect, 99);
    let line = d.spells.skill_abilities[&13_262]
        .iter()
        .find(|a| a.skill == 333)
        .map(|a| (a.trivial_low, a.trivial_high));
    assert_eq!(line, Some((20, 60)));

    let (_c, g, map) = player(&realm, "P7LHOME", "Homer", Race::Orc, Class::Shaman).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let mut back = Vec::new();
        for id in [8690, 556] {
            let p = m.players.get_mut(&g).unwrap();
            let at = p.position();
            let home = (at.x + 40.0, at.y + 10.0, at.z);
            if let Some(b) = p.row.state.bind.as_mut() {
                (b.x, b.y, b.z) = home;
                b.map = 1;
            }
            spells::cast_triggered(m, g, id, Some(g));
            let now = m.players[&g].position();
            back.push((
                (now.x - home.0).abs() < 0.5 && (now.y - home.1).abs() < 0.5,
                id,
            ));
            let p = m.players.get_mut(&g).unwrap();
            p.movement.position = at;
        }
        back
    })
    .await;
    assert_eq!(out, vec![(true, 8690), (true, 556)]);
}

/// Thistle Tea's resurrections (`spell_resurrection_dbc_test.exs`): the
/// class spells (Resurrection 2006, Redemption 7328, Ancestral Spirit
/// 2008, Rebirth 20484) bring back flat health and mana; Jumper Cables
/// (8342) a share; only Rebirth's ranks skip the corpse's wait
/// (`SPELL_ATTR_EX3_NO_RES_TIMER`).
#[test]
#[ignore = "extended"]
fn real_resurrections_give_flat_health_and_only_rebirth_skips_the_wait() {
    let Some(d) = real() else {
        return;
    };
    for (id, health, mana) in [
        (2006, 70, 135),
        (2008, 65, 120),
        (7328, 65, 120),
        (20_484, 400, 700),
    ] {
        let s = spell(d, id);
        let e = &s.effects[0];
        assert_eq!(e.effect, 113, "{id}");
        assert_eq!(
            (spells::points(s, 0, 60), e.misc_value),
            (health, mana),
            "{id}"
        );
    }
    assert_eq!(spell(d, 8342).effects[0].effect, 18);
    let skips = |id| spell(d, id).attr(3, crate::game::data::spells::ATTR_EX3_NO_RES_TIMER);
    for id in [20_484, 20_739, 20_742, 20_747, 20_748] {
        assert!(skips(id), "{id}");
    }
    for id in [2006, 2008, 7328, 8342] {
        assert!(!skips(id), "{id}");
    }
}

/// `g` dead with the self-resurrection of `aura` (a soulstone's) or
/// Reincarnation, raised by `CMSG_SELF_RES`: their health and mana.
async fn rises(
    c: &mut crate::game::harness::Client,
    map: &MapHandle,
    g: Guid,
    aura: Option<u32>,
) -> (u32, u32, u32, u32) {
    on(map, move |m| {
        let foe = dummy(m, g, 60, 3.0);
        if let Some(a) = aura {
            apply(m, foe, g, a);
        }
        crate::game::map::combat::deal_damage(m, foe, g, 1_000_000);
    })
    .await;
    c.send(CMSG_SELF_RES {}).await;
    let mut out = (0, 0, 0, 0);
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        out = on(map, move |m| {
            let p = &m.players[&g];
            let alive = p.death == crate::game::map::death::Death::Alive;
            let f = &p.fields;
            (
                u32::from(alive),
                f.get_u32(UNIT_FIELD_HEALTH),
                f.get_u32(UNIT_FIELD_POWER1),
                f.get_u32(UNIT_FIELD_MAXHEALTH),
            )
        })
        .await;
        if out.0 == 1 {
            break;
        }
    }
    out
}

/// Thistle Tea's self-resurrection (`self_resurrection_dbc_test.exs`):
/// each Soulstone rank brings back its flat health and mana (400 and 700
/// to 2200 and 2800), no more than the most there is; Reincarnation (21169) a fifth of each, its Ankh
/// (17030) taken and its category's hour started; Improved Reincarnation
/// (16184, 16209) takes 10 or 20 minutes off it and adds 10 or 20 points.
#[tokio::test]
#[ignore = "extended"]
async fn soulstones_and_reincarnation_bring_back_what_they_should() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let reincarnation = spell(&d, 21_169);
    assert_eq!(reincarnation.reagents[0], (17_030, 1));
    assert_eq!(
        (reincarnation.category, reincarnation.category_recovery_time),
        (1161, 3_600_000)
    );
    for (id, cooldown, more) in [(16_184, -600_000, 10), (16_209, -1_200_000, 20)] {
        let s = spell(&d, id);
        let by = |op: i32| {
            s.effects
                .iter()
                .position(|e| e.misc_value == op)
                .map(|i| spells::points(s, i, 60))
        };
        assert_eq!((by(11), by(8)), (Some(cooldown), Some(more)), "{id}");
    }

    let (mut c, g, map) = player(&realm, "P7LSTONE", "Stona", Race::Human, Class::Mage).await;
    on(&map, move |m| ready(m, g, 60, &[])).await;
    for (aura, health, mana) in [
        (20_707, 400, 700),
        (20_762, 750, 1200),
        (20_763, 1100, 1700),
        (20_764, 1600, 2200),
        (20_765, 2200, 2800),
    ] {
        let (alive, h, p, max) = rises(&mut c, &map, g, Some(aura)).await;
        assert_eq!((alive, h, p), (1, health.min(max), mana), "{aura}");
    }

    let (mut c, g, map) = player(&realm, "P7LREINC", "Reinca", Race::Orc, Class::Shaman).await;
    let plain = reborn(&mut c, &map, g, None).await;
    assert_eq!(plain, (1, 20, 20, 0, Some(3600)), "Reincarnation");
    let first = reborn(&mut c, &map, g, Some(16_184)).await;
    assert_eq!(first, (1, 30, 30, 0, Some(3000)), "Improved, 1");
    let second = reborn(&mut c, &map, g, Some(16_209)).await;
    assert_eq!(second, (1, 40, 40, 0, Some(2400)), "Improved, 2");
}

/// A shaman of 60 with Reincarnation, an Ankh and `talent`, dead and
/// raised by it: alive, the percent of health and mana back, the Ankhs
/// left, and the seconds left of its category's cooldown.
async fn reborn(
    c: &mut crate::game::harness::Client,
    map: &MapHandle,
    g: Guid,
    talent: Option<u32>,
) -> (u32, u32, u32, u32, Option<i64>) {
    on(map, move |m| {
        ready(m, g, 60, &[20_608]);
        fresh(m, g);
        for t in [16_184, 16_209] {
            auras::remove_spell(m, g, t, None);
        }
        if let Some(t) = talent {
            apply(m, g, g, t);
        }
        let _ = crate::game::map::inventory::add_item(m, g, 17_030, 1, false);
    })
    .await;
    let (alive, health, mana, max_health) = rises(c, map, g, None).await;
    let (ankhs, max_mana, cooldown) = on(map, move |m| {
        let p = &m.players[&g];
        let left = p
            .caster
            .category_cooldowns
            .get(&1161)
            .map(|t| (t.saturating_duration_since(m.now).as_millis() as i64 + 999) / 1000);
        (
            p.count_of(17_030),
            p.fields.get_u32(UNIT_FIELD_MAXPOWER1),
            left,
        )
    })
    .await;
    (
        alive,
        (health * 100).div_ceil(max_health.max(1)),
        (mana * 100).div_ceil(max_mana.max(1)),
        ankhs,
        cooldown,
    )
}
