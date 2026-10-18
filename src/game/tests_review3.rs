//! The third code review's findings (combat, spells, auras, threat and
//! creature AI), each shown on made-up data in the fast suite.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{at, dummy, health, pass, ready};
use crate::game::data::ItemTemplate;
use crate::game::map::inventory::BANK_START;
use crate::game::map::spells;
use crate::game::store::BAG_PLAYER;

const REAGENT: u32 = 990_390;

fn reagent() -> ItemTemplate {
    ItemTemplate {
        entry: REAGENT,
        name: "Made-up Reagent".into(),
        allowable_class: -1,
        allowable_race: -1,
        stackable: 20,
        buy_count: 1,
        ..Default::default()
    }
}

/// A cast's reagents are checked again as it goes off where it takes them
/// from, the bags: put in the bank meanwhile, they fail the cast, rather
/// than it going off for nothing.
#[tokio::test]
async fn reagents_banked_during_a_cast_fail_it() {
    const SPELL: u32 = 990_301;
    let realm = fast_realm(|d| {
        d.items.insert(REAGENT, reagent());
        let mut s = bare(SPELL, 2);
        s.casting_time_index = CAST_2S;
        s.reagents[0] = (REAGENT, 1);
        effect(&mut s, 0, damage(100));
        insert(d, [s]);
    });
    let (_c, mage, map) = human(&realm, "R3RG", "Reagents", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[SPELL]);
            let k = dummy(m, mage, 60, 3.0);
            crate::game::map::inventory::add_item(m, mage, REAGENT, 1, false).unwrap();
            // A GM pays no reagents.
            m.players.get_mut(&mage).unwrap().gm = false;
            let before = health(m, k);
            let began = spells::cast(m, mage, SPELL, at(k), None);
            // Into the bank while the cast goes on.
            let p = m.players.get_mut(&mage).unwrap();
            let i = p.items.iter().position(|i| i.entry == REAGENT).unwrap();
            p.items[i].bag = BAG_PLAYER;
            p.items[i].slot = BANK_START;
            pass(m, 2_100);
            let p = &m.players[&mage];
            (began, before - health(m, k), p.count_of(REAGENT))
        })
        .await
        .unwrap();
    assert!(out.0, "the cast began with the reagent in the bags");
    assert_eq!(out.1, 0, "it doesn't go off with the reagent in the bank");
    assert_eq!(out.2, 1, "and the reagent is still there");
}

fn threat_on(
    m: &crate::game::map::Map,
    creature: wow_world_messages::Guid,
    of: wow_world_messages::Guid,
) -> f32 {
    m.creature(creature)
        .and_then(|c| c.threat.iter().find(|(g, _)| *g == of).map(|(_, t)| *t))
        .unwrap_or(0.0)
}

/// A spell's damage makes threat by the caster's threat auras once, their
/// product, as a swing's does (Thistle Tea's `SpellThreat`, mangos's
/// `ThreatCalcHelper`): an all-school +30% makes 100 damage 130 threat,
/// and a holy-only +60% makes a holy spell's 160 and a fire spell's 100.
#[tokio::test]
async fn a_spells_threat_takes_the_threat_auras_once() {
    const ALL: u32 = 990_310;
    const HOLY_ONLY: u32 = 990_311;
    const FIRE: u32 = 990_312;
    const HOLY: u32 = 990_313;
    let realm = fast_realm(|d| {
        let mut all = bare(ALL, 0);
        all.duration_index = LASTS_LONG;
        effect(
            &mut all,
            0,
            aura(crate::game::map::auras::AURA_MOD_THREAT, 127, 30, 1),
        );
        let mut holy_only = bare(HOLY_ONLY, 0);
        holy_only.duration_index = LASTS_LONG;
        effect(
            &mut holy_only,
            0,
            aura(crate::game::map::auras::AURA_MOD_THREAT, 2, 60, 1),
        );
        let mut fire = bare(FIRE, 2);
        effect(&mut fire, 0, damage(100));
        let mut holy = bare(HOLY, 1);
        effect(&mut holy, 0, damage(100));
        insert(d, [all, holy_only, fire, holy]);
    });
    let (_c, paladin, map) = human(&realm, "R3TH", "Threatened", Class::Paladin).await;
    let out = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{apply_template, cast_fresh};
            ready(m, paladin, 60, &[FIRE, HOLY]);
            let template =
                |m: &crate::game::map::Map, id| m.realm.data.spells.get(id).unwrap().clone();
            let mut seen = Vec::new();
            for (aura, spell) in [(ALL, FIRE), (HOLY_ONLY, HOLY), (HOLY_ONLY, FIRE)] {
                let k = dummy(m, paladin, 60, 3.0);
                let t = template(m, aura);
                apply_template(m, paladin, paladin, &t);
                cast_fresh(m, paladin, spell, at(k));
                seen.push((1_000_000 - health(m, k), threat_on(m, k, paladin)));
                crate::game::map::auras::remove_spell(m, paladin, aura, None);
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(out[0].0, 100);
    assert!((out[0].1 - 130.0).abs() < 0.01, "all schools: {:?}", out[0]);
    assert!(
        (out[1].1 - 160.0).abs() < 0.01,
        "holy on holy: {:?}",
        out[1]
    );
    assert!(
        (out[2].1 - 100.0).abs() < 0.01,
        "holy on fire: {:?}",
        out[2]
    );
}

/// A cast interrupted is told its caster once (mangos's `Spell::cancel`
/// sends one `SMSG_CAST_RESULT`).
#[tokio::test]
async fn an_interrupted_cast_is_told_once() {
    const SPELL: u32 = 990_330;
    let realm = fast_realm(|d| {
        let mut s = bare(SPELL, 2);
        s.casting_time_index = CAST_2S;
        effect(&mut s, 0, damage(100));
        insert(d, [s]);
    });
    let (mut c, mage, map) = human(&realm, "R3IN", "Interrupted", Class::Mage).await;
    map.query(move |m| {
        ready(m, mage, 60, &[SPELL]);
        let k = dummy(m, mage, 60, 3.0);
        assert!(spells::cast(m, mage, SPELL, at(k), None));
        spells::interrupt(m, mage);
    })
    .await
    .unwrap();
    let interrupted =
        |b: &[u8]| b.len() >= 6 && b[..4] == SPELL.to_le_bytes() && b[4] == 2 && b[5] == 0x23;
    c.wait_frame("the cast interrupted", 0x130, interrupted)
        .await;
    let again = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        c.wait_frame("told again", 0x130, interrupted),
    )
    .await;
    assert!(again.is_err(), "told twice");
}

/// A spell cast while channelling ends the channel first (mangos's
/// `SetCurrentCastedSpell`): its fields clear and its aura comes off its
/// target, whether the new spell is instant or has a cast time.
#[tokio::test]
async fn a_cast_during_a_channel_ends_the_channel() {
    use crate::game::data::spells::ATTR_EX_CHANNELED;
    use crate::game::fields::UNIT_CHANNEL_SPELL;
    const CHANNEL: u32 = 990_340;
    const INSTANT_BOLT: u32 = 990_341;
    const SLOW_BOLT: u32 = 990_342;
    let realm = fast_realm(|d| {
        let mut channel = bare(CHANNEL, 5);
        channel.duration_index = LASTS_5S;
        channel.attributes[1] |= ATTR_EX_CHANNELED;
        effect(
            &mut channel,
            0,
            crate::game::data::spells::SpellEffect {
                amplitude: 1_000,
                ..aura(crate::game::map::auras::AURA_PERIODIC_DAMAGE, 0, 10, 6)
            },
        );
        let mut instant = bare(INSTANT_BOLT, 2);
        effect(&mut instant, 0, damage(10));
        let mut slow = bare(SLOW_BOLT, 2);
        slow.casting_time_index = CAST_2S;
        effect(&mut slow, 0, damage(10));
        insert(d, [channel, instant, slow]);
    });
    let (_c, warlock, map) = human(&realm, "R3CH", "Channeller", Class::Warlock).await;
    let out = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{cast_fresh, casting, has};
            ready(m, warlock, 60, &[CHANNEL, INSTANT_BOLT, SLOW_BOLT]);
            let k = dummy(m, warlock, 60, 3.0);
            let mut seen = Vec::new();
            for next in [INSTANT_BOLT, SLOW_BOLT] {
                cast_fresh(m, warlock, CHANNEL, at(k));
                let channelling = (
                    casting(m, warlock),
                    m.unit_fields(warlock).unwrap().get_u32(UNIT_CHANNEL_SPELL),
                    has(m, k, CHANNEL),
                );
                if let Some(c) = m.caster_mut(warlock) {
                    c.gcd_until = None;
                }
                spells::cast(m, warlock, next, at(k), None);
                seen.push((
                    channelling,
                    casting(m, warlock),
                    m.unit_fields(warlock).unwrap().get_u32(UNIT_CHANNEL_SPELL),
                    has(m, k, CHANNEL),
                ));
            }
            seen
        })
        .await
        .unwrap();
    let on = (Some(CHANNEL), CHANNEL, true);
    assert_eq!(out[0], (on, None, 0, false), "an instant spell");
    assert_eq!(
        out[1],
        (on, Some(SLOW_BOLT), 0, false),
        "one with a cast time"
    );
}

/// A player taken out of the map (a logout, a disconnect) ends their
/// channel, its aura on its target with it, and comes off the threat lists
/// of the creatures they fought (mangos's `RemoveFromWorld`: the cast
/// interrupted, the hostile references deleted).
#[tokio::test]
async fn a_player_leaving_the_map_ends_their_channel_and_their_threat() {
    use crate::game::data::spells::ATTR_EX_CHANNELED;
    const CHANNEL: u32 = 990_350;
    let realm = fast_realm(|d| {
        let mut channel = bare(CHANNEL, 5);
        channel.duration_index = LASTS_5S;
        channel.attributes[1] |= ATTR_EX_CHANNELED;
        effect(
            &mut channel,
            0,
            crate::game::data::spells::SpellEffect {
                amplitude: 1_000,
                ..aura(crate::game::map::auras::AURA_PERIODIC_DAMAGE, 0, 10, 6)
            },
        );
        insert(d, [channel]);
    });
    let (_c, warlock, map) = human(&realm, "R3LV", "Leaver", Class::Warlock).await;
    let out = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{cast_fresh, has};
            ready(m, warlock, 60, &[CHANNEL]);
            let k = dummy(m, warlock, 60, 3.0);
            cast_fresh(m, warlock, CHANNEL, at(k));
            crate::game::map::ai::attacked(m, k, warlock);
            crate::game::map::ai::add_threat(m, k, warlock, 10.0);
            let before = (has(m, k, CHANNEL), threat_on(m, k, warlock) > 0.0);
            let _gone = m.take_player(warlock);
            let listed = m
                .creature(k)
                .is_some_and(|c| c.threat.iter().any(|(g, _)| *g == warlock));
            (before, has(m, k, CHANNEL), listed)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true), "channelling at it, on its list");
    assert!(!out.1, "the channel's aura goes with its caster");
    assert!(!out.2, "and the creature forgets them");
}

/// A creature that turns to a new victim mid-fight (its last feigned death
/// or vanished: `ai::forget`) keeps its fight's start and the leash's
/// origin, where the fight began, not where it turned.
#[tokio::test]
async fn a_creature_turning_mid_fight_keeps_its_leash_origin() {
    let realm = fast_realm(|_| {});
    let (_c, hunter, map) = human(&realm, "R3OR", "Kiter", Class::Rogue).await;
    let (_c2, mage, _) = human(&realm, "R3OS", "Bystander", Class::Mage).await;
    let out = map
        .query(move |m| {
            use crate::game::map::ai;
            let k = dummy(m, hunter, 60, 3.0);
            ai::attacked(m, k, hunter);
            ai::add_threat(m, k, mage, 1.0);
            let c = m.creature(k).unwrap();
            let (origin, start, attacking) =
                (c.engagement.origin, c.combat_start, c.combat.attacking);
            // Dragged forty yards off, it loses the hunter.
            m.now += std::time::Duration::from_secs(5);
            let c = m.creature_mut(k).unwrap();
            c.position.x += 40.0;
            ai::forget(m, hunter);
            let c = m.creature(k).unwrap();
            (
                attacking,
                c.combat.victim == mage && c.combat.attacking,
                origin.is_some() && c.engagement.origin == origin,
                c.combat_start == start,
            )
        })
        .await
        .unwrap();
    assert!(out.0, "it fights the hunter");
    assert!(out.1, "and turns on the mage");
    assert!(out.2, "its leash still from where the fight began");
    assert!(out.3, "and the fight's start kept");
}

/// `g` attacking `victim` in reach, with the swings `hand_due` says due.
fn swinging_at(
    m: &mut crate::game::map::Map,
    g: wow_world_messages::Guid,
    victim: wow_world_messages::Guid,
) {
    let now = m.now;
    let p = m.players.get_mut(&g).unwrap();
    p.combat.attacking = true;
    p.combat.victim = victim;
    p.combat.next_swing = Some(now + std::time::Duration::from_secs(10));
    p.combat.next_offhand = Some(now + std::time::Duration::from_secs(10));
}

/// A blow that kills its own striker (a damage shield's answer) ends its
/// swings: the extra attacks owed don't swing from the corpse.
#[tokio::test]
async fn a_striker_killed_by_a_damage_shield_swings_no_more() {
    const THORNS: u32 = 990_370;
    let realm = fast_realm(|d| {
        let mut thorns = bare(THORNS, 3);
        thorns.duration_index = LASTS_LONG;
        effect(
            &mut thorns,
            0,
            aura(crate::game::map::auras::AURA_DAMAGE_SHIELD, 0, 5_000, 1),
        );
        insert(d, [thorns]);
    });
    let (_c, warrior, map) = human(&realm, "R3DS", "Thorned", Class::Warrior).await;
    let (_c2, friend, _) = human(&realm, "R3DT", "Friend", Class::Mage).await;
    let out = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{apply_template, set_health};
            use crate::game::map::combat;
            ready(m, warrior, 60, &[]);
            let k = dummy(m, warrior, 60, 2.0);
            let t = m.realm.data.spells.get(THORNS).unwrap().clone();
            apply_template(m, k, k, &t);
            set_health(m, warrior, 100);
            // Someone else on its list, so the striker's death doesn't send
            // it home whole.
            crate::game::map::ai::attacked(m, k, friend);
            crate::game::map::ai::add_threat(m, k, friend, 1.0);
            swinging_at(m, warrior, k);
            m.players.get_mut(&warrior).unwrap().combat.extra_attacks = 3;
            combat::FORCED_OUTCOME.with(|f| f.set(Some(combat::Outcome::Hit)));
            let before = health(m, k);
            combat::try_swing(m, warrior);
            combat::FORCED_OUTCOME.with(|f| f.set(None));
            let after = health(m, k);
            // One blow's worth, against three.
            let one = m
                .unit_fields(warrior)
                .map(|f| f.get_f32(crate::game::fields::UNIT_FIELD_MAXDAMAGE))
                .unwrap_or(0.0);
            (health(m, warrior), before - after, one)
        })
        .await
        .unwrap();
    assert_eq!(out.0, 0, "the shield killed the striker");
    assert!(out.1 > 0, "its first blow landed");
    assert!(
        (out.1 as f32) <= out.2 + 1.0,
        "and only the first: {} dealt, a blow at most {}",
        out.1,
        out.2
    );
}

/// A dual-wielder's two hands don't swing in one step: the main hand's
/// swing puts the off-hand's a moment later (mangos's
/// `ATTACK_DISPLAY_DELAY`), and it waits for it.
#[tokio::test]
async fn the_hands_dont_swing_together() {
    let realm = fast_realm(|_| {});
    let (_c, rogue, map) = human(&realm, "R3DW", "Twohands", Class::Rogue).await;
    let out = map
        .query(move |m| {
            use crate::game::map::combat;
            ready(m, rogue, 60, &[]);
            let k = dummy(m, rogue, 60, 2.0);
            swinging_at(m, rogue, k);
            let now = m.now;
            let p = m.players.get_mut(&rogue).unwrap();
            p.derived.dual_wield = true;
            p.combat.next_swing = Some(now);
            p.combat.next_offhand = Some(now);
            combat::FORCED_OUTCOME.with(|f| f.set(Some(combat::Outcome::Hit)));
            combat::try_swing(m, rogue);
            combat::FORCED_OUTCOME.with(|f| f.set(None));
            let c = &m.players[&rogue].combat;
            (
                c.next_swing.map(|t| t - now),
                c.next_offhand.map(|t| t - now),
            )
        })
        .await
        .unwrap();
    assert!(
        out.0.is_some_and(|d| d.as_millis() >= 1_000),
        "the main hand swung: {out:?}"
    );
    assert_eq!(
        out.1,
        Some(std::time::Duration::from_millis(200)),
        "the off-hand waits its moment"
    );
}

/// A creature down in a linked feign death runs none of its timed events
/// while it lies there: an in-combat timer that raises its phase doesn't.
#[tokio::test]
async fn a_creature_down_in_a_linked_feign_death_runs_no_timed_events() {
    use crate::game::data::scripts::{AiEvent, EVENT_TIMER_IN_COMBAT, ScriptStep};
    use crate::game::fields::{UNIT_FIELD_HEALTH, UNIT_FIELD_MAXHEALTH};
    use crate::game::map::{ai, combat, event_ai, scripts};
    use crate::game::tests_p3_combat::{data, setup_with, spawn, template};
    use crate::game::tests_p3_entity::{on, step};
    const HOUND: u32 = 3_790;
    const RAISE: u32 = 1;
    let mut d = data();
    let mut hound = template(HOUND, 1);
    hound.static_flags1 |= combat::STATIC_UNKILLABLE;
    d.creature_templates.insert(HOUND, hound);
    // Every second in a fight, its phase one higher.
    d.scripts.ai_scripts.insert(
        RAISE,
        vec![ScriptStep {
            id: RAISE,
            command: 44,
            datalong: [1, 1, 0, 0],
            ..Default::default()
        }],
    );
    d.scripts.ai_events.insert(
        HOUND,
        vec![AiEvent {
            kind: EVENT_TIMER_IN_COMBAT,
            chance: 100,
            flags: 1,
            params: [60_000, 60_000, 1_000, 1_000],
            actions: [RAISE, 0, 0],
            ..Default::default()
        }],
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000_000);
        f.set_u32(UNIT_FIELD_HEALTH, 100_000_000);
        let h = spawn(m, ga, HOUND, 20, 3.0);
        let feign = ScriptStep {
            dataint: [0, 0, 0, 0],
            ..step(scripts::C_LINKED_FEIGN_DEATH, [60_000, HOUND, 30, 0])
        };
        scripts::execute(m, &feign, h, Guid::zero());
        ai::attacked(m, h, ga);
        // Its timer due now; then down.
        let now = m.now;
        if let Some(s) = m.creature_mut(h).and_then(|c| c.scripted.as_deref_mut()) {
            for t in s.timers.iter_mut() {
                *t = Some(now);
            }
        }
        combat::deal_damage(m, ga, h, 10_000_000);
        let down = event_ai::feigning(m, h);
        let before = event_ai::phase(m, h);
        for _ in 0..5 {
            m.now += std::time::Duration::from_secs(1);
            ai::update_creatures(m);
        }
        (
            down,
            before,
            event_ai::phase(m, h),
            event_ai::feigning(m, h),
        )
    })
    .await;
    assert!(out.0, "down");
    assert_eq!(out.2, out.1, "its timer waits while it lies there");
    assert!(out.3, "still down");
}

/// A stacked aura saved at logout comes back at login with its stacks on
/// its bar and in the stats it gives, not one stack's.
#[tokio::test]
async fn a_restored_aura_shows_and_gives_its_stacks() {
    use crate::game::fields::{UNIT_FIELD_AURAAPPLICATIONS, UNIT_FIELD_STAT0};
    use crate::game::map::spells::cooldowns;
    const MIGHT: u32 = 990_360;
    let realm = fast_realm(|d| {
        let mut might = bare(MIGHT, 0);
        might.duration_index = LASTS_LONG;
        might.stack_amount = 5;
        effect(
            &mut might,
            0,
            aura(crate::game::map::auras::AURA_MOD_STAT, 0, 10, 1),
        );
        insert(d, [might]);
    });
    let (_c, warrior, map) = human(&realm, "R3ST", "Stacked", Class::Warrior).await;
    let out = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{apply_template, holder};
            ready(m, warrior, 60, &[]);
            let shown = |m: &crate::game::map::Map| {
                let f = m.unit_fields(warrior).unwrap();
                (
                    f.get_u32(UNIT_FIELD_STAT0),
                    (0..12)
                        .map(|i| f.get_u32(UNIT_FIELD_AURAAPPLICATIONS + i))
                        .collect::<Vec<_>>(),
                )
            };
            let bare = shown(m).0;
            let t = m.realm.data.spells.get(MIGHT).unwrap().clone();
            for _ in 0..5 {
                apply_template(m, warrior, warrior, &t);
            }
            let stacked = shown(m);
            let saved = cooldowns::saved_auras(&m.players[&warrior], m.now);
            crate::game::map::auras::remove_spell(m, warrior, MIGHT, None);
            m.players.get_mut(&warrior).unwrap().row.state.auras = saved;
            cooldowns::restore(m, warrior);
            (
                bare,
                stacked,
                shown(m),
                holder(m, warrior, MIGHT).map(|h| h.stacks),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.1.0, out.0 + 50, "five stacks of ten strength");
    assert_eq!(out.3, Some(5), "five stacks back");
    assert_eq!(out.2, out.1, "shown and given as before the logout");
}

/// An aura replaced by another (a higher rank of it) is taken off as any
/// removal takes it: a cooldown waiting for it to go starts, rather than
/// waiting for good with the spell never ready again.
#[tokio::test]
async fn an_aura_replaced_starts_the_cooldown_waiting_for_it() {
    use crate::game::data::spells::SpellRank;
    const RANK_1: u32 = 990_380;
    const RANK_2: u32 = 990_381;
    let realm = fast_realm(|d| {
        for (id, rank) in [(RANK_1, 1), (RANK_2, 2)] {
            let mut s = bare(id, 0);
            s.duration_index = LASTS_LONG;
            // Its cooldown starts when its aura goes (Stealth's way).
            s.attributes[0] |= 0x200_0000;
            s.recovery_time = 10_000;
            effect(
                &mut s,
                0,
                aura(crate::game::map::auras::AURA_MOD_STAT, 0, 10 * rank, 1),
            );
            d.spells.ranks.insert(
                id,
                SpellRank {
                    first: RANK_1,
                    rank: rank as u32,
                    ..Default::default()
                },
            );
            insert(d, [s]);
        }
    });
    let (_c, rogue, map) = human(&realm, "R3RP", "Replaced", Class::Rogue).await;
    let out = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{cast_fresh, has};
            ready(m, rogue, 60, &[RANK_1, RANK_2]);
            cast_fresh(m, rogue, RANK_1, at(rogue));
            let waiting = m.caster(rogue).unwrap().pending_cooldowns.contains(&RANK_1);
            if let Some(c) = m.caster_mut(rogue) {
                c.gcd_until = None;
            }
            spells::cast(m, rogue, RANK_2, at(rogue), None);
            let now = m.now;
            let c = m.caster(rogue).unwrap();
            (
                waiting,
                has(m, rogue, RANK_1),
                has(m, rogue, RANK_2),
                c.pending_cooldowns.contains(&RANK_1),
                c.cooldowns.get(&RANK_1).is_some_and(|t| *t > now),
            )
        })
        .await
        .unwrap();
    assert!(out.0, "rank 1's cooldown waits for its aura");
    assert_eq!((out.1, out.2), (false, true), "rank 2 took its place");
    assert!(!out.3, "rank 1 waits no more");
    assert!(out.4, "its cooldown started as it went");
}

/// A pet area aura of two effects (Soul Link's damage done and split,
/// Stalker's Ally's two powers) puts both on the pet, in one holder.
#[tokio::test]
async fn a_pet_area_aura_of_two_effects_puts_both_on_the_pet() {
    use crate::game::data::spells::SpellEffect;
    use crate::game::map::auras::{AURA_MOD_RESISTANCE, AURA_MOD_STAT};
    use crate::game::map::pets::EFFECT_APPLY_AREA_AURA_PET;
    use crate::game::tests_pets::{
        BOAR, CALL_PET, DISMISS_PET, TAME_BEAST, cast, data, enter, learn, pet_bar, place,
    };
    const LINK: u32 = 990_395;
    let mut d = data();
    let mut link = bare(LINK, 0);
    link.duration_index = LASTS_LONG;
    for (i, (kind, misc, amount)) in [(AURA_MOD_STAT, 0, 10), (AURA_MOD_RESISTANCE, 1, 100)]
        .into_iter()
        .enumerate()
    {
        effect(
            &mut link,
            i,
            SpellEffect {
                effect: EFFECT_APPLY_AREA_AURA_PET,
                ..aura(kind, misc, amount, 1)
            },
        );
    }
    insert(&mut d, [link]);
    let realm = crate::game::harness::realm_with(d);
    let (mut h, gh, map) = enter(&realm, "R3PA", "Linked", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, CALL_PET, DISMISS_PET, LINK]);
    let boar = place(&map, gh, BOAR, 0x70_0001, 4.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    let kinds = map
        .query(move |m| {
            use super::tests_p3_spells_kit::{cast_fresh, holder};
            cast_fresh(m, gh, LINK, at(gh));
            holder(m, pet, LINK).map(|h| {
                let mut k: Vec<u32> = h.effects.iter().map(|x| x.kind).collect();
                k.sort_unstable();
                k
            })
        })
        .await
        .unwrap();
    assert_eq!(
        kinds,
        Some(vec![AURA_MOD_RESISTANCE, AURA_MOD_STAT]),
        "both effects on the pet"
    );
}
