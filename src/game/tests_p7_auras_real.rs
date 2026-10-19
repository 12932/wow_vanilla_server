//! Round 7's aura tests on the real data, ported from Thistle Tea's
//! `breakable_control_dbc_test.exs`, `engineering_dbc_test.exs`,
//! `form_links_dbc_test.exs`, `form_links_test.exs` and
//! `periodic_trigger_test.exs` (docs/parity/matrix.csv): the crowd
//! control a pet leaves be, the Net-o-Matic's nets, a druid's form boosts
//! and Leader of the Pack, and Azuregos's pulse.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::{Map, auras, combat, spells};

/// Thistle Tea's `breakable_crowd_control?` "protects Polymorph, Gouge,
/// Freezing Trap, Sap, and Repentance" and "permits attacks through Frost
/// Nova, Hammer of Justice, and Fear": the crowd control damage breaks
/// (a confuse, stun or transform that damage ends) a pet leaves be of its
/// own accord; a root, a stun damage doesn't end, and a fear it attacks
/// through.
#[tokio::test]
#[ignore = "extended"]
async fn a_pet_leaves_be_only_the_crowd_control_damage_breaks() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P7KR1", "Holdone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let held = |m: &mut Map, id: u32| {
                let kobold = dummy(m, mage, 60, 8.0);
                apply(m, mage, kobold, id);
                (
                    id,
                    has(m, kobold, id),
                    crate::game::map::pets::breakable_cc(m, kobold),
                )
            };
            let protected: Vec<_> = [118, 1776, 3355, 6770, 20066]
                .into_iter()
                .map(|id| held(m, id))
                .collect();
            let open: Vec<_> = [122, 853, 5782].into_iter().map(|id| held(m, id)).collect();
            (protected, open)
        })
        .await
        .unwrap();
    let (protected, open) = out;
    for (id, on, breakable) in protected {
        assert!(on && breakable, "{id} on and left be");
    }
    for (id, on, breakable) in open {
        assert!(on && !breakable, "{id} on and attacked through");
    }
}

const NET: u32 = 13099;
const LONG_BACKFIRE: u32 = 16566;
const BACKFIRE: u32 = 13119;
const MARKER: u32 = 13139;
const ON_THROWER: u32 = 13138;

/// The holder of `spell` on `g`: its full duration.
fn lasts(m: &Map, g: Guid, spell: u32) -> Option<Option<u32>> {
    holder(m, g, spell).map(|h| h.duration)
}

/// Thistle Tea's `Engineering` "normal nets root the target for ten
/// seconds and expire through ordinary aura cleanup", "the long backfire
/// roots only the caster for thirty seconds" and "the charging net
/// triggers one transient marker and roots its caster despite an enemy
/// selection": the Net-o-Matic's net holds its target ten seconds; its
/// long backfire holds the thrower alone thirty; its backfire holds the
/// target and the thrower twenty seconds each, no marker left; each runs
/// out at its end, and the thrower's goes with its death.
#[tokio::test]
#[ignore = "extended"]
async fn the_net_o_matics_nets_hold_whom_they_net_for_their_time() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, gnome, map) = player(&realm, "P7KR2", "Netone", Race::Gnome, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, gnome, 60, &[]);
            let rooted =
                |m: &Map, g: Guid| auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOD_ROOT));
            // The net.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, NET, Some(kobold));
            let net = (lasts(m, kobold, NET), rooted(m, kobold), rooted(m, gnome));
            pass(m, 9_999);
            let held = rooted(m, kobold);
            pass(m, 1);
            let net_after = (has(m, kobold, NET), rooted(m, kobold));
            // The long backfire.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, LONG_BACKFIRE, Some(kobold));
            let long = (
                lasts(m, gnome, LONG_BACKFIRE),
                rooted(m, gnome),
                rooted(m, kobold),
            );
            pass(m, 30_000);
            let long_after = rooted(m, gnome);
            // The backfire.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, BACKFIRE, Some(kobold));
            let backfire = (
                lasts(m, kobold, BACKFIRE),
                lasts(m, gnome, ON_THROWER),
                has(m, gnome, MARKER),
            );
            pass(m, 20_000);
            let backfire_after = (rooted(m, kobold), rooted(m, gnome));
            // Again, and the thrower dies netted.
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, BACKFIRE, Some(kobold));
            combat::kill(m, kobold, gnome);
            let dead = has(m, gnome, ON_THROWER);
            (
                net,
                held,
                net_after,
                long,
                long_after,
                backfire,
                backfire_after,
                dead,
            )
        })
        .await
        .unwrap();
    let (net, held, net_after, long, long_after, backfire, backfire_after, dead) = out;
    assert_eq!(
        net,
        (Some(Some(10_000)), true, false),
        "the net: its target, ten seconds"
    );
    assert!(held, "held to its end");
    assert_eq!(net_after, (false, false), "and free at it");
    assert_eq!(
        long,
        (Some(Some(30_000)), true, false),
        "the thrower alone, thirty"
    );
    assert!(!long_after);
    assert_eq!(
        backfire,
        (Some(Some(20_000)), Some(Some(20_000)), false),
        "both twenty seconds, no marker"
    );
    assert_eq!(backfire_after, (false, false));
    assert!(!dead, "gone with the thrower's death");
}

const HEART_OF_THE_WILD: [(u32, u32); 5] = [
    (17003, 4),
    (17004, 8),
    (17005, 12),
    (17006, 16),
    (24894, 20),
];
const BEAR: u32 = 5487;
const DIRE_BEAR: u32 = 9634;
const CAT: u32 = 768;
const BEAR_BOOST: u32 = 24899;
const CAT_BOOST: u32 = 24900;

fn stat(m: &Map, g: Guid, i: u16) -> u32 {
    m.players[&g].fields.get_u32(UNIT_FIELD_STAT0 + i)
}

fn near(got: u32, want: f32) -> bool {
    (got as f32 - want).abs() <= 1.0
}

/// Thistle Tea's `FormLinks` "all Heart of the Wild ranks apply their
/// exact form-dependent percentage": each rank gives its share more
/// intellect (4, 8, 12, 16, 20), and as much more stamina in bear and dire
/// bear form, strength in cat, each with its form's boost, the bear's
/// health share kept; out of form, neither boost nor its stat.
#[tokio::test]
#[ignore = "extended"]
async fn every_rank_of_heart_of_the_wild_gives_its_share_in_each_form() {
    if !have_data() {
        return;
    }
    let realm = realm();
    for (n, (rank, share)) in HEART_OF_THE_WILD.into_iter().enumerate() {
        let (_c, dr, map) = player(
            &realm,
            &format!("P7KH{n}"),
            &format!("Heartone{}", ["a", "b", "c", "d", "e"][n]),
            Race::NightElf,
            Class::Druid,
        )
        .await;
        let out = map
            .query(move |m| {
                ready(m, dr, 60, &[]);
                let own = [0, 2, 3].map(|i| stat(m, dr, i));
                spells::learn(m, dr, rank);
                let intellect = stat(m, dr, 3);
                let mut forms = Vec::new();
                for (form, stat_index, boost) in [
                    (CAT, 0, CAT_BOOST),
                    (BEAR, 2, BEAR_BOOST),
                    (DIRE_BEAR, 2, BEAR_BOOST),
                ] {
                    let max = m.players[&dr].fields.get_u32(UNIT_FIELD_MAXHEALTH);
                    set_health(m, dr, max / 2);
                    spells::cast_triggered(m, dr, form, None);
                    let f = &m.players[&dr].fields;
                    let share = f.get_u32(UNIT_FIELD_HEALTH) as f32
                        / f.get_u32(UNIT_FIELD_MAXHEALTH).max(1) as f32;
                    let shifted = (stat(m, dr, stat_index), has(m, dr, boost), share);
                    auras::remove_spell(m, dr, form, None);
                    let normal = (
                        [0, 2, 3].map(|i| stat(m, dr, i)),
                        has(m, dr, BEAR_BOOST) || has(m, dr, CAT_BOOST),
                    );
                    forms.push((form, stat_index, shifted, normal));
                }
                (own, intellect, forms)
            })
            .await
            .unwrap();
        let (own, intellect, forms) = out;
        let grown = |base: u32| base as f32 * (100 + share) as f32 / 100.0;
        assert!(
            near(intellect, grown(own[2])),
            "{rank}: intellect {intellect} of {own:?}"
        );
        for (form, stat_index, (value, boosted, health), (normal, any)) in forms {
            let base = own[if stat_index == 0 { 0 } else { 1 }];
            assert!(
                near(value, grown(base)),
                "{rank} in {form}: {value} of {base}"
            );
            assert!(boosted, "{rank} in {form}: its boost");
            assert!(
                (health - 0.5).abs() < 0.01,
                "{rank} in {form}: health share {health}"
            );
            assert_eq!(
                [normal[0], normal[1]],
                [own[0], own[1]],
                "{rank} out of {form}"
            );
            assert!(near(normal[2], grown(own[2])));
            assert!(!any, "{rank} out of {form}: no boost");
        }
    }
}

const LEADER_OF_THE_PACK: u32 = 17007;
const PACK_AURA: u32 = 24932;
const MOONKIN: u32 = 24858;

fn count(m: &Map, g: Guid, spell: u32) -> usize {
    auras::auras(m, g).map_or(0, |a| a.holders.iter().filter(|h| h.spell == spell).count())
}

fn crit(m: &Map, g: Guid) -> f32 {
    m.players[&g].fields.get_f32(PLAYER_CRIT_PERCENTAGE)
}

/// Thistle Tea's `FormLinks` "Bear to Dire Bear refreshes shared boosts
/// without losing or duplicating them" and "death removes form boosts
/// while retaining the parent passive": a druid with Heart of the Wild and
/// Leader of the Pack going from bear to dire bear form has one of each
/// boost, a fifth more stamina and the pack's 3% more crit; dying in bear
/// form takes the boost and leaves the talent; raised at half health, in
/// bear form again, the boost is back and the share of health kept.
#[tokio::test]
#[ignore = "extended"]
async fn a_druids_form_boosts_carry_from_bear_to_dire_bear_and_go_with_death() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, dr, map) = player(&realm, "P7KR4", "Packone", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            ready(m, dr, 60, &[]);
            let stamina = stat(m, dr, 2);
            let own_crit = crit(m, dr);
            spells::learn(m, dr, 24894);
            spells::learn(m, dr, LEADER_OF_THE_PACK);
            spells::cast_triggered(m, dr, BEAR, None);
            spells::cast_triggered(m, dr, DIRE_BEAR, None);
            let dire = (
                auras::form(m, dr),
                count(m, dr, BEAR_BOOST),
                count(m, dr, PACK_AURA),
                stat(m, dr, 2),
                crit(m, dr) - own_crit,
            );
            auras::remove_spell(m, dr, DIRE_BEAR, None);
            spells::cast_triggered(m, dr, BEAR, None);
            let brute = dummy(m, dr, 60, 5.0);
            combat::kill(m, brute, dr);
            let dead = (
                has(m, dr, 24894),
                has(m, dr, BEAR_BOOST),
                has(m, dr, PACK_AURA),
            );
            crate::game::map::death::resurrect(m, dr, 0.5);
            spells::cast_triggered(m, dr, BEAR, None);
            let f = &m.players[&dr].fields;
            let share =
                f.get_u32(UNIT_FIELD_HEALTH) as f32 / f.get_u32(UNIT_FIELD_MAXHEALTH).max(1) as f32;
            let raised = (has(m, dr, BEAR_BOOST), stat(m, dr, 2), share);
            (stamina, dire, dead, raised)
        })
        .await
        .unwrap();
    let (stamina, dire, dead, raised) = out;
    let grown = stamina as f32 * 1.2;
    assert_eq!(
        (dire.0, dire.1, dire.2),
        (auras::FORM_DIREBEAR, 1, 1),
        "one of each"
    );
    assert!(near(dire.3, grown), "stamina {} of {stamina}", dire.3);
    assert!((dire.4 - 3.0).abs() < 0.01, "the pack's crit: {}", dire.4);
    assert_eq!(
        dead,
        (true, false, false),
        "(the talent, the boost, the pack's)"
    );
    assert!(raised.0 && near(raised.1, grown), "{raised:?}");
    assert!(
        (raised.2 - 0.5).abs() < 0.01,
        "half its health: {}",
        raised.2
    );
}

/// What `c` is told of party commands is let pass; it takes the
/// invitation it has, and is in.
async fn joins(by: &mut harness::Client, c: &mut harness::Client, name: &str) {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    by.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    c.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    c.send(CMSG_GROUP_ACCEPT {}).await;
    c.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
}

use crate::game::harness;

/// Thistle Tea's `FormLinks` "party aura sources retain visibility,
/// refresh recipients and never stack with another source" (and the DBC
/// test's "learning Leader of the Pack in form creates the visible party
/// source"): a druid in cat form with Leader of the Pack shows the pack's
/// aura, 3% more crit, and its party member near gets one of it, kept as
/// the pulses come; out of cat form (into moonkin), the druid's goes with
/// the form, the talent stays, and the member's goes at the next pulse.
#[tokio::test]
#[ignore = "extended"]
async fn leader_of_the_pack_shows_on_the_druid_and_reaches_the_party() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, dr, map) = player(&realm, "P7KR5", "Leadone", Race::NightElf, Class::Druid).await;
    let (mut w, warrior, _) =
        player(&realm, "P7KR6", "Followone", Race::NightElf, Class::Warrior).await;
    joins(&mut c, &mut w, "Followone").await;
    let out = map
        .query(move |m| {
            ready(m, dr, 60, &[]);
            ready(m, warrior, 60, &[]);
            spells::learn(m, dr, LEADER_OF_THE_PACK);
            let own_crit = crit(m, warrior);
            spells::cast_triggered(m, dr, CAT, None);
            let shown = holder(m, dr, PACK_AURA).map(|h| (h.slot.is_some(), h.passive));
            let leader_crit = crit(m, dr);
            pass(m, 1000);
            pass(m, 1000);
            let member = (count(m, warrior, PACK_AURA), crit(m, warrior) - own_crit);
            pass(m, 3000);
            let kept = count(m, warrior, PACK_AURA);
            auras::remove_spell(m, dr, CAT, None);
            spells::cast_triggered(m, dr, MOONKIN, None);
            let moonkin = (has(m, dr, PACK_AURA), has(m, dr, LEADER_OF_THE_PACK));
            pass(m, 1000);
            pass(m, 1000);
            (
                shown,
                leader_crit,
                member,
                kept,
                moonkin,
                has(m, warrior, PACK_AURA),
            )
        })
        .await
        .unwrap();
    let (shown, leader_crit, member, kept, moonkin, member_after) = out;
    assert_eq!(shown, Some((true, false)), "seen, not a passive");
    assert!(leader_crit > 0.0);
    assert_eq!(member.0, 1, "one on the member");
    assert!((member.1 - 3.0).abs() < 0.01, "its crit: {}", member.1);
    assert_eq!(kept, 1, "kept, not stacked, as the pulses come");
    assert_eq!(moonkin, (false, true), "(the pack's, the talent)");
    assert!(!member_after, "the member's gone at the next pulse");
}

/// Thistle Tea's `PeriodicTrigger` "pulses as aura-triggered spells so an
/// idle Azuregos engages nobody": Azuregos's Mark of Frost pulse marks the
/// player near him, and neither is put in a fight by it.
#[tokio::test]
#[ignore = "extended"]
async fn azuregos_pulse_marks_without_a_fight() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P7KR7", "Frostone", Race::NightElf, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let azuregos = spawn(m, g, 6109, 63, 6.0);
            if !has(m, azuregos, 23184) {
                apply(m, azuregos, azuregos, 23184);
            }
            pass(m, 10_000);
            let fighting = |m: &Map, u: Guid| {
                m.unit_fields(u).is_some_and(|f| {
                    f.has_flag(
                        UNIT_FIELD_FLAGS,
                        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    )
                })
            };
            (has(m, g, 23183), fighting(m, azuregos), fighting(m, g))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (true, false, false),
        "(marked, Azuregos fighting, the player fighting)"
    );
}

/// Thistle Tea's `ReactiveArmor` "accepted spells spend charges once and
/// respect the proc cooldown": Obsidian Armor with two charges answers a
/// fire spell with its fire resistance, spends one charge and waits its
/// ten seconds; a frost spell a moment before those are out gets nothing
/// and spends nothing; one at ten seconds gets the frost resistance and
/// spends the last charge, the armor with it.
#[tokio::test]
#[ignore = "extended"]
async fn obsidian_armor_spends_a_charge_an_answer_and_waits_its_cooldown() {
    use crate::game::map::procs::{self, Event};
    const OBSIDIAN_ARMOR: u32 = 27539;
    const FIRE_RESISTANCE: u32 = 27533;
    const FROST_RESISTANCE: u32 = 27534;
    if !have_data() {
        return;
    }
    // Sure to proc once its cooldown allows.
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&OBSIDIAN_ARMOR) {
            s.proc_chance = 100;
        }
        if let Some(p) = d.spells.proc_events.get_mut(&OBSIDIAN_ARMOR) {
            p.chance = 100.0;
        }
    });
    let (_c, mage, map) = player(&realm, "P7KR8", "Obsidone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 3.0);
            apply(m, mage, mage, OBSIDIAN_ARMOR);
            if let Some(a) = auras::auras_mut(m, mage)
                && let Some(h) = a.holders.iter_mut().find(|h| h.spell == OBSIDIAN_ARMOR)
            {
                h.charges = 2;
            }
            let hit = |m: &mut Map, spell: u32| {
                procs::trigger(
                    m,
                    Event {
                        actor: kobold,
                        target: mage,
                        actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
                        target_flags: procs::PROC_TAKEN_NEGATIVE_SPELL | procs::PROC_TAKEN_DAMAGE,
                        ex: procs::EX_NORMAL_HIT,
                        spell: Some(spell),
                        damage: 10,
                        school: 1,
                        ..Default::default()
                    },
                );
                let got = [FIRE_RESISTANCE, FROST_RESISTANCE].map(|a| has(m, mage, a));
                for a in [FIRE_RESISTANCE, FROST_RESISTANCE] {
                    auras::remove_spell(m, mage, a, None);
                }
                let charges = holder(m, mage, OBSIDIAN_ARMOR).map(|h| h.charges);
                (got, charges)
            };
            let fire = hit(m, 133);
            pass(m, 9_999);
            let early = hit(m, 116);
            pass(m, 1);
            let frost = hit(m, 116);
            (fire, early, frost)
        })
        .await
        .unwrap();
    let (fire, early, frost) = out;
    assert_eq!(
        fire,
        ([true, false], Some(1)),
        "fire: its answer, a charge spent"
    );
    assert_eq!(
        early,
        ([false, false], Some(1)),
        "before its cooldown: nothing"
    );
    assert_eq!(
        frost,
        ([false, true], None),
        "frost at ten seconds, the last charge"
    );
}
