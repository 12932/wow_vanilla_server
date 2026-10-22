//! The spells and auras bug hunt's findings, each shown on made-up data in
//! the fast suite.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{apply_template, has, ready};
use crate::game::harness::{self, Client};
use crate::game::map::auras;

/// A lasting aura that ends as its unit leaves the world (a polymorph, a
/// flag, Silithyst), a single-target one and a world buff aren't saved at a
/// logout, as mangos's `IsSaveToDbHolder`; a plain one is.
#[tokio::test]
async fn auras_mangos_doesnt_save_dont_come_back_at_login() {
    const PLAIN: u32 = 990_900;
    const LEAVES: u32 = 990_901;
    const SINGLE: u32 = 990_902;
    const ECHOES: u32 = 1386;
    let realm = fast_realm(|d| {
        let made = |id: u32| {
            let mut s = bare(id, 0);
            s.duration_index = LASTS_LONG;
            effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
            s
        };
        let mut leaves = made(LEAVES);
        leaves.aura_interrupt_flags = auras::INTERRUPT_CHANGE_MAP;
        let mut single = made(SINGLE);
        single.custom_flags = crate::game::data::spells::CUSTOM_SINGLE_TARGET_AURA;
        insert(d, [made(PLAIN), leaves, single, made(ECHOES)]);
    });
    let mut c = Client::connect(&realm, "BHSA");
    let g = c.enter("Saver", Race::Human, Class::Mage).await;
    let map = world(&realm);
    let on = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            for id in [PLAIN, LEAVES, SINGLE, ECHOES] {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                apply_template(m, g, g, &s);
            }
            [PLAIN, LEAVES, SINGLE, ECHOES].map(|id| has(m, g, id))
        })
        .await
        .unwrap();
    assert_eq!(on, [true; 4], "all on");
    c.disconnect();
    harness::offline(&realm, g).await;
    let mut back = Client::connect(&realm, "BHSA");
    back.login(g).await;
    harness::an_update(&map).await;
    let back_on = map
        .query(move |m| [PLAIN, LEAVES, SINGLE, ECHOES].map(|id| has(m, g, id)))
        .await
        .unwrap();
    assert_eq!(
        back_on,
        [true, false, false, false],
        "only the plain one back"
    );
}

/// `CMSG_CANCEL_CAST` stops only the spell it names, and takes back only a
/// queued strike of that spell; `CMSG_CANCEL_CHANNELLING` stops only a
/// channel (mangos's `HandleCancelCastOpcode` and `HandleCancelChanneling`).
/// A late cancel of a cast already done doesn't kill the next.
#[tokio::test]
async fn a_cancel_stops_only_what_it_names() {
    use super::tests_p3_spells_kit::{at, cast_fresh, casting};
    use crate::game::map::spells::{self, checks::fail};
    const SLOW: u32 = 990_910;
    const QUICK: u32 = 990_911;
    const STRIKE: u32 = 990_912;
    let realm = fast_realm(|d| {
        let made = |id: u32, cast: u32| {
            let mut s = bare(id, 0);
            s.casting_time_index = cast;
            s.duration_index = LASTS_5S;
            effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
            s
        };
        insert(d, [made(SLOW, CAST_10S), made(QUICK, INSTANT)]);
    });
    let mut c = Client::connect(&realm, "BHCC");
    let g = c.enter("Canceller", Race::Human, Class::Mage).await;
    let map = world(&realm);
    let started = map
        .query(move |m| {
            ready(m, g, 60, &[SLOW, QUICK]);
            let started = cast_fresh(m, g, SLOW, at(g));
            crate::game::map::combat::queue_next_swing(m, g, STRIKE);
            started
        })
        .await
        .unwrap();
    assert!(started, "the slow cast began");
    c.clear();
    c.send(CMSG_CANCEL_CAST { id: QUICK }).await;
    c.send(CMSG_CANCEL_CHANNELLING { id: SLOW }).await;
    c.send(CMSG_CAST_SPELL {
        spell: QUICK,
        targets: spells::wire_targets(&at(g)),
    })
    .await;
    c.wait_frame("the quick cast refused", spells::SMSG_CAST_RESULT, |b| {
        b[..4] == QUICK.to_le_bytes() && b[4] == 2 && b[5] == fail::SPELL_IN_PROGRESS
    })
    .await;
    let kept = map
        .query(move |m| (casting(m, g), m.players[&g].combat.queued))
        .await
        .unwrap();
    assert_eq!(kept, (Some(SLOW), Some(STRIKE)), "nothing named stopped");
    c.send(CMSG_CANCEL_CAST { id: SLOW }).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let now = map
            .query(move |m| (casting(m, g), m.players[&g].combat.queued))
            .await
            .unwrap();
        if now == (None, Some(STRIKE)) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{now:?}");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    c.send(CMSG_CANCEL_CAST { id: STRIKE }).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while map
        .query(move |m| m.players[&g].combat.queued.is_some())
        .await
        .unwrap()
    {
        assert!(tokio::time::Instant::now() < deadline, "the strike stays");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// A cast whose target steps out of sight before it ends fails with
/// LINE_OF_SIGHT, and one its caster is pacified for meanwhile fails
/// PACIFIED, as mangos's `CheckCast(false)` at `Spell::cast` tests sight
/// and the caster's auras again; neither does its damage.
#[tokio::test]
async fn a_cast_ends_only_in_sight_and_unrestrained() {
    use super::tests_p3_spells_kit::{at, dummy, health, pass};
    use crate::game::map::spells::{self, checks::fail};
    use wow_world_messages::vanilla::Vector3d;
    const BOLT: u32 = 990_920;
    const STRIKE: u32 = 990_921;
    const PACIFY: u32 = 990_922;
    let realm = fast_realm(|d| {
        let mut bolt = bare(BOLT, 2);
        bolt.casting_time_index = CAST_2S;
        effect(&mut bolt, 0, damage(100));
        let mut strike = bare(STRIKE, 0);
        strike.casting_time_index = CAST_2S;
        strike.prevention_type = spells::checks::PREVENTION_PACIFY;
        effect(&mut strike, 0, damage(100));
        let mut pacify = bare(PACIFY, 0);
        pacify.duration_index = LASTS_15S;
        effect(&mut pacify, 0, aura(auras::AURA_MOD_PACIFY, 0, 0, 1));
        insert(d, [bolt, strike, pacify]);
    });
    let (mut c, g, map) = human(&realm, "BHLS", "Sighted", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[BOLT, STRIKE]);
            m.players.get_mut(&g).unwrap().gm = false;
            let k = dummy(m, g, 60, 4.0);
            let (a, b) = (m.players[&g].position(), m.unit_view(k).unwrap().position);
            let mid = Vector3d {
                x: (a.x + b.x) / 2.0,
                y: (a.y + b.y) / 2.0,
                z: a.z,
            };
            // Across the way between them, ten yards wide.
            let (dx, dy) = ((b.x - a.x) / 4.0, (b.y - a.y) / 4.0);
            let across = |f: f32| Vector3d {
                x: mid.x - dy * f,
                y: mid.y + dx * f,
                z: mid.z,
            };
            let before = health(m, k);
            let began = spells::cast(m, g, BOLT, at(k), None);
            m.spells.walls.push((across(-5.0), across(5.0)));
            pass(m, 2_100);
            let walled = before - health(m, k);
            m.spells.walls.clear();
            let began2 = spells::cast(m, g, STRIKE, at(k), None);
            let s = m.realm.data.spells.get(PACIFY).unwrap().clone();
            super::tests_p3_spells_kit::apply_template(m, g, g, &s);
            pass(m, 2_100);
            (began, walled, began2, before - health(m, k))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, 0, true, 0), "neither lands");
    for (spell, code) in [(BOLT, fail::LINE_OF_SIGHT), (STRIKE, fail::PACIFIED)] {
        c.wait_frame("the cast failed", spells::SMSG_CAST_RESULT, |b| {
            b[..4] == spell.to_le_bytes() && b[4] == 2 && b[5] == code
        })
        .await;
    }
}

/// A damage shield (Thorns) meets the striker's immunity to its school, as
/// mangos's `DealMeleeDamage` tests it: one immune takes none.
#[tokio::test]
async fn a_damage_shield_meets_immunity() {
    use super::tests_p3_spells_kit::{dummy, health, set_health};
    use crate::game::map::procs;
    const THORNS: u32 = 990_930;
    const IMMUNE: u32 = 990_931;
    let realm = fast_realm(|d| {
        let mut thorns = bare(THORNS, 3);
        thorns.duration_index = LASTS_LONG;
        effect(&mut thorns, 0, aura(auras::AURA_DAMAGE_SHIELD, 0, 10, 1));
        let mut immune = bare(IMMUNE, 0);
        immune.duration_index = LASTS_LONG;
        effect(
            &mut immune,
            0,
            aura(auras::AURA_SCHOOL_IMMUNITY, 1 << 3, 0, 1),
        );
        insert(d, [thorns, immune]);
    });
    let (_c, g, map) = human(&realm, "BHDS", "Thorny", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let s = m.realm.data.spells.get(THORNS).unwrap().clone();
            apply_template(m, g, g, &s);
            let k = dummy(m, g, 60, 3.0);
            set_health(m, k, 10_000);
            procs::damage_shields(m, k, g);
            let struck = 10_000 - health(m, k);
            let i = m.realm.data.spells.get(IMMUNE).unwrap().clone();
            apply_template(m, k, k, &i);
            set_health(m, k, 10_000);
            procs::damage_shields(m, k, g);
            (struck, 10_000 - health(m, k))
        })
        .await
        .unwrap();
    assert_eq!(out, (10, 0), "ten, then none through the immunity");
}

/// A saved aura whose spell's cooldown waits for its end (Stealth's kind)
/// and which doesn't come back at login (run out while away) leaves no
/// cooldown waiting, as mangos's `_LoadAuras` adds it only for an aura put
/// back: the spell may be cast again, not locked until the next login.
#[tokio::test]
async fn a_waiting_cooldown_goes_with_an_aura_that_didnt_come_back() {
    use crate::game::map::spells::cooldowns::{self, SavedAura};
    const VEIL: u32 = 990_940;
    let realm = fast_realm(|d| {
        let mut veil = bare(VEIL, 0);
        veil.duration_index = LASTS_15S;
        veil.recovery_time = 10_000;
        veil.attributes[0] |= 0x200_0000;
        // Its time runs while its player is away.
        veil.attributes[4] |= 0x4;
        effect(&mut veil, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        insert(d, [veil]);
    });
    let (_c, g, map) = human(&realm, "BHWC", "Veiled", Class::Rogue).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[VEIL]);
            let p = m.players.get_mut(&g).unwrap();
            p.row.state.saved_at_ms = chrono::Utc::now().timestamp_millis() - 60_000;
            p.row.state.auras = vec![SavedAura {
                spell: VEIL,
                caster: g.guid(),
                stacks: 1,
                left: Some(5_000),
                amounts: vec![(0, 1)],
                ..Default::default()
            }];
            cooldowns::restore(m, g);
            (
                has(m, g, VEIL),
                m.caster(g).unwrap().pending_cooldowns.contains(&VEIL),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (false, false), "neither the aura nor its wait");
}

/// A charged cast-time modifier that comes on during a cast (Shadow Trance
/// from a Corruption tick) isn't spent by the cast under way, whose time it
/// didn't change, as mangos spends only the modifiers a cast was prepared
/// with: the next one has it. One the cast began with is spent.
#[tokio::test]
async fn a_trance_gained_mid_cast_is_kept_for_the_next() {
    use super::tests_p3_spells_kit::{at, dummy, pass};
    use crate::game::map::spells;
    const BOLT: u32 = 990_950;
    const TRANCE: u32 = 990_951;
    let realm = fast_realm(|d| {
        let mut bolt = bare(BOLT, 5);
        bolt.family_name = 5;
        bolt.family_flags = 1;
        bolt.casting_time_index = CAST_2S;
        effect(&mut bolt, 0, damage(10));
        let mut trance = bare(TRANCE, 5);
        trance.family_name = 5;
        trance.duration_index = LASTS_15S;
        trance.proc_charges = 1;
        let mut e = aura(
            auras::AURA_ADD_PCT_MODIFIER,
            spells::mods::OP_CASTING_TIME as i32,
            -100,
            1,
        );
        e.item_type = 1;
        effect(&mut trance, 0, e);
        insert(d, [bolt, trance]);
    });
    let (_c, g, map) = human(&realm, "BHST", "Tranced", Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[BOLT]);
            let k = dummy(m, g, 60, 4.0);
            let t = m.realm.data.spells.get(TRANCE).unwrap().clone();
            let began = spells::cast(m, g, BOLT, at(k), None);
            apply_template(m, g, g, &t);
            pass(m, 2_100);
            let kept = has(m, g, TRANCE);
            // The next is instant, and spends it.
            let instant =
                spells::cast(m, g, BOLT, at(k), None) && m.caster(g).unwrap().casting.is_none();
            (began, kept, instant, has(m, g, TRANCE))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true, false));
}

/// `CMSG_CANCEL_AURA` refuses what mangos's `HandleCancelAuraOpcode` does:
/// an aura the client shows no icon for, and another's party aura; one of
/// the player's own goes.
#[tokio::test]
async fn a_cancel_aura_spares_hidden_and_others_party_auras() {
    use super::tests_p3_spells_kit::dummy;
    const HIDDEN: u32 = 990_960;
    const PARTY: u32 = 990_961;
    const PLAIN: u32 = 990_962;
    let realm = fast_realm(|d| {
        let made = |id: u32| {
            let mut s = bare(id, 0);
            s.duration_index = LASTS_LONG;
            effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
            s
        };
        let mut hidden = made(HIDDEN);
        hidden.attributes[1] |= 0x1000_0000;
        let mut party = made(PARTY);
        party.effects[0].effect = crate::game::map::spells::EFFECT_APPLY_AREA_AURA_PARTY;
        insert(d, [hidden, party, made(PLAIN)]);
    });
    let (c, g, map) = human(&realm, "BHCA", "Aurae", Class::Mage).await;
    map.query(move |m| {
        ready(m, g, 60, &[]);
        let other = dummy(m, g, 60, 3.0);
        for (id, by) in [(HIDDEN, g), (PARTY, other), (PLAIN, g)] {
            let s = m.realm.data.spells.get(id).unwrap().clone();
            apply_template(m, by, g, &s);
        }
    })
    .await
    .unwrap();
    for id in [HIDDEN, PARTY, PLAIN] {
        c.send(CMSG_CANCEL_AURA { id }).await;
    }
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while map.query(move |m| has(m, g, PLAIN)).await.unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the plain one stays"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let left = map
        .query(move |m| (has(m, g, HIDDEN), has(m, g, PARTY)))
        .await
        .unwrap();
    assert_eq!(
        left,
        (true, true),
        "the hidden and the other's party aura stay"
    );
}

/// A bolt whose caster leaves the map while it flies (a logout) is lost,
/// as mangos ends a unit's spell events as it leaves the world, and as an
/// arrow is: it doesn't land for someone not there.
#[tokio::test]
async fn a_bolt_of_a_caster_gone_doesnt_land() {
    use super::tests_p3_spells_kit::{at, dummy, health, pass};
    use crate::game::map::spells;
    const BOLT: u32 = 990_970;
    let realm = fast_realm(|d| {
        let mut bolt = bare(BOLT, 2);
        bolt.speed = 1.0;
        effect(&mut bolt, 0, damage(100));
        insert(d, [bolt]);
    });
    let (c, g, map) = human(&realm, "BHIF", "Bolter", Class::Mage).await;
    let (k, before, flying) = map
        .query(move |m| {
            ready(m, g, 60, &[BOLT]);
            m.spells.travel = Some(true);
            let k = dummy(m, g, 60, 4.0);
            let before = health(m, k);
            spells::cast(m, g, BOLT, at(k), None);
            (k, before, m.spells.in_flight.len())
        })
        .await
        .unwrap();
    assert_eq!(flying, 1, "on its way");
    c.disconnect();
    harness::offline(&realm, g).await;
    let after = map
        .query(move |m| {
            pass(m, 10_000);
            health(m, k)
        })
        .await
        .unwrap();
    assert_eq!(after, before, "it never lands");
}

/// An aura refreshed keeps its proc's cooldown (mangos keeps it on the
/// unit, by spell): a seal cast again doesn't proc at once.
#[tokio::test]
async fn a_refreshed_aura_keeps_its_procs_cooldown() {
    use super::tests_p3_spells_kit::holder;
    const SEAL: u32 = 990_980;
    let realm = fast_realm(|d| {
        let mut seal = bare(SEAL, 0);
        seal.duration_index = LASTS_LONG;
        effect(&mut seal, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        insert(d, [seal]);
    });
    let (_c, g, map) = human(&realm, "BHPR", "Sealed", Class::Paladin).await;
    let (set, kept) = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let s = m.realm.data.spells.get(SEAL).unwrap().clone();
            apply_template(m, g, g, &s);
            let at = m.now + std::time::Duration::from_secs(10);
            auras::auras_mut(m, g)
                .unwrap()
                .holders
                .iter_mut()
                .find(|h| h.spell == SEAL)
                .unwrap()
                .proc_ready_at = Some(at);
            apply_template(m, g, g, &s);
            (Some(at), holder(m, g, SEAL).and_then(|h| h.proc_ready_at))
        })
        .await
        .unwrap();
    assert_eq!(kept, set);
}

/// A rider's cast that fails its checks (on cooldown) leaves them mounted;
/// one that may be cast takes them off their mount and goes (review 3's
/// R3-24).
#[tokio::test]
async fn a_failed_cast_doesnt_dismount() {
    use crate::game::map::spells;
    const MOUNT: u32 = 990_990;
    const FLASH: u32 = 990_991;
    let realm = fast_realm(|d| {
        let mut mount = bare(MOUNT, 0);
        mount.duration_index = LASTS_LONG;
        effect(&mut mount, 0, aura(auras::AURA_MOUNTED, 6_000, 0, 1));
        let mut flash = bare(FLASH, 0);
        flash.recovery_time = 30_000;
        flash.duration_index = LASTS_5S;
        effect(&mut flash, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        insert(d, [mount, flash]);
    });
    let (_c, g, map) = human(&realm, "BHMT", "Rider", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[FLASH]);
            let s = m.realm.data.spells.get(MOUNT).unwrap().clone();
            apply_template(m, g, g, &s);
            let mounted = has(m, g, MOUNT);
            let later = m.now + std::time::Duration::from_secs(30);
            m.caster_mut(g).unwrap().cooldowns.insert(FLASH, later);
            let refused = !spells::cast(m, g, FLASH, Default::default(), None);
            let still = has(m, g, MOUNT);
            m.caster_mut(g).unwrap().cooldowns.clear();
            let went = spells::cast(m, g, FLASH, Default::default(), None);
            (
                mounted,
                refused,
                still,
                went,
                has(m, g, MOUNT),
                has(m, g, FLASH),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true, true, false, true));
}

/// An instant ranged ability that fails as it goes for want of a ranged
/// weapon or ammunition gives its global cooldown back, as mangos's checks
/// refuse it before one starts (review 3's R3-23).
#[tokio::test]
async fn an_instant_failed_at_launch_gives_its_global_cooldown_back() {
    use super::tests_p3_spells_kit::{at, dummy};
    use crate::game::map::spells;
    const SHOT: u32 = 991_000;
    let realm = fast_realm(|d| {
        let mut shot = bare(SHOT, 0);
        shot.attributes[0] |= 0x2;
        shot.dmg_class = 3;
        shot.start_recovery_category = spells::GCD_SPELLS;
        shot.start_recovery_time = 1_500;
        effect(&mut shot, 0, damage(10));
        insert(d, [shot]);
    });
    let (_c, g, map) = human(&realm, "BHGC", "Shooter", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[SHOT]);
            let k = dummy(m, g, 60, 4.0);
            spells::cast(m, g, SHOT, at(k), None);
            m.caster(g).unwrap().gcd_until
        })
        .await
        .unwrap();
    assert_eq!(out, None, "no global cooldown for a shot not loosed");
}

/// A cast whose target is gone as it ends fails for those who watched it
/// too (`SMSG_SPELL_FAILED_OTHER`), and gives its global cooldown back, as
/// mangos's `cancel` at a lost target (review 3's R3-22).
#[tokio::test]
async fn a_cast_at_a_target_gone_fails_for_its_watchers() {
    use super::tests_p3_spells_kit::{at, dummy, pass};
    use crate::game::map::spells;
    const BOLT: u32 = 991_010;
    let realm = fast_realm(|d| {
        let mut bolt = bare(BOLT, 2);
        bolt.casting_time_index = CAST_2S;
        bolt.start_recovery_category = spells::GCD_SPELLS;
        bolt.start_recovery_time = 2_500;
        effect(&mut bolt, 0, damage(10));
        insert(d, [bolt]);
    });
    let (_c, g, map) = human(&realm, "BHBT", "Caster", Class::Mage).await;
    let mut w = Client::connect(&realm, "BHBW");
    let _watcher = w.enter("Watcher", Race::Human, Class::Mage).await;
    w.sees(g).await;
    let gcd = map
        .query(move |m| {
            ready(m, g, 60, &[BOLT]);
            let k = dummy(m, g, 60, 4.0);
            spells::cast(m, g, BOLT, at(k), None);
            m.remove_creature(k);
            pass(m, 2_100);
            m.caster(g).unwrap().gcd_until
        })
        .await
        .unwrap();
    assert_eq!(gcd, None, "the global cooldown given back");
    w.wait_for("the cast failed", |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_SPELL_FAILED_OTHER(f)
            if f.caster == g && f.id == BOLT =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
}

/// A ground area's ticks roll their hit with their caster's modifiers (a
/// talent's spell hit, as Arcane Focus's), as its spell's other hits do
/// (review 3's R3-20): a low caster with +78% hit lands nearly every tick
/// on one far above them, rather than the 22% at worst.
#[tokio::test]
async fn ground_ticks_roll_with_their_casters_hit() {
    use super::tests_p3_spells_kit::{dummy, health, pass, set_health};
    use crate::game::map::spells;
    const GROUND: u32 = 991_020;
    const FOCUS: u32 = 991_021;
    let realm = fast_realm(|d| {
        d.spells.radii.insert(96, 5.0);
        let mut ground = bare(GROUND, 4);
        ground.family_name = 3;
        ground.family_flags = 1;
        ground.duration_index = LASTS_LONG;
        ground.effects[0] = crate::game::data::spells::SpellEffect {
            effect: spells::EFFECT_PERSISTENT_AREA_AURA,
            aura: auras::AURA_PERIODIC_DAMAGE,
            amplitude: 1_000,
            base_points: 9,
            base_dice: 1,
            target_a: 18,
            radius_index: 96,
            bonus_coefficient: -1.0,
            ..Default::default()
        };
        let mut focus = bare(FOCUS, 0);
        focus.family_name = 3;
        focus.duration_index = LASTS_LONG;
        let mut e = aura(
            auras::AURA_ADD_FLAT_MODIFIER,
            spells::mods::OP_RESIST_MISS_CHANCE as i32,
            78,
            1,
        );
        e.item_type = 1;
        effect(&mut focus, 0, e);
        insert(d, [ground, focus]);
    });
    let (_c, g, map) = human(&realm, "BHGT", "Focused", Class::Mage).await;
    let hits = map
        .query(move |m| {
            ready(m, g, 10, &[]);
            m.spells.rolls = Some(true);
            let f = m.realm.data.spells.get(FOCUS).unwrap().clone();
            apply_template(m, g, g, &f);
            let k = dummy(m, g, 60, 4.0);
            set_health(m, k, 100_000);
            let at = m.position_of(k);
            spells::cast_triggered_with(
                m,
                g,
                GROUND,
                spells::Targets {
                    dest: at,
                    ..Default::default()
                },
            );
            let mut hits = 0;
            for _ in 0..20 {
                let before = health(m, k);
                pass(m, 1_000);
                if health(m, k) < before {
                    hits += 1;
                }
            }
            hits
        })
        .await
        .unwrap();
    assert!(hits >= 15, "{hits} of 20 landed");
}

/// A proc that takes another proc aura off (a dispel) keeps that one from
/// firing in the same event, as mangos skips a holder removed meanwhile
/// (review 3's R3-26).
#[tokio::test]
async fn an_aura_taken_off_by_a_proc_doesnt_proc_after() {
    use super::tests_p3_spells_kit::dummy;
    use crate::game::map::{procs, spells};
    const CLEANSE: u32 = 991_030;
    const CLEANSER: u32 = 991_031;
    const MARKER: u32 = 991_032;
    const MARK: u32 = 991_033;
    const MECHANIC: u32 = 7;
    let realm = fast_realm(|d| {
        let proc_trigger = |id: u32, trigger: u32| {
            let mut s = bare(id, 0);
            s.duration_index = LASTS_LONG;
            s.proc_flags = procs::PROC_DONE_MELEE;
            s.proc_chance = 100;
            effect(
                &mut s,
                0,
                crate::game::data::spells::SpellEffect {
                    trigger_spell: trigger,
                    ..aura(auras::AURA_PROC_TRIGGER_SPELL, 0, 0, 1)
                },
            );
            s
        };
        let mut cleanse = bare(CLEANSE, 0);
        effect(
            &mut cleanse,
            0,
            crate::game::data::spells::SpellEffect {
                effect: spells::EFFECT_DISPEL_MECHANIC,
                misc_value: MECHANIC as i32,
                target_a: 1,
                ..Default::default()
            },
        );
        let mut marker = proc_trigger(MARKER, MARK);
        marker.mechanic = MECHANIC;
        let mut mark = bare(MARK, 0);
        mark.duration_index = LASTS_LONG;
        effect(&mut mark, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        insert(d, [proc_trigger(CLEANSER, CLEANSE), cleanse, marker, mark]);
    });
    let (_c, g, map) = human(&realm, "BHPS", "Procced", Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            for id in [CLEANSER, MARKER] {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                apply_template(m, g, g, &s);
            }
            let k = dummy(m, g, 60, 3.0);
            procs::trigger(
                m,
                procs::Event {
                    actor: g,
                    target: k,
                    actor_flags: procs::PROC_DONE_MELEE,
                    target_flags: procs::PROC_TAKEN_MELEE,
                    ex: procs::EX_NORMAL_HIT,
                    damage: 10,
                    school: 1,
                    ..Default::default()
                },
            );
            (has(m, g, MARKER), has(m, g, MARK))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (false, false),
        "the marker cleansed, and no mark from it"
    );
}

/// A new aura whose replacing of another takes it off too (its source, the
/// aura whose pulse put it on, replaced and its lasting auras going with
/// it) leaves the unit's other auras as they were: the new holder is found
/// again, not taken to be the last, which was given the replaced one's
/// slot (review 3's R3-16).
#[tokio::test]
async fn an_aura_gone_with_what_it_replaced_leaves_the_rest_alone() {
    const PULSE: u32 = 991_040;
    const FED: u32 = 991_041;
    const HIDDEN: u32 = 991_042;
    let realm = fast_realm(|d| {
        let mut pulse = bare(PULSE, 0);
        pulse.duration_index = LASTS_15S;
        pulse.attributes[2] |= 0x8000_0000;
        effect(
            &mut pulse,
            0,
            crate::game::data::spells::SpellEffect {
                trigger_spell: FED,
                amplitude: 5_000,
                ..aura(auras::AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 1)
            },
        );
        let mut fed = bare(FED, 0);
        fed.attributes[2] |= 0x8000_0000;
        effect(&mut fed, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        // On no bar.
        let mut hidden = bare(HIDDEN, 0);
        hidden.duration_index = LASTS_LONG;
        hidden.attributes[0] = crate::game::data::spells::ATTR_HIDDEN;
        effect(&mut hidden, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        insert(d, [pulse, fed, hidden]);
    });
    let (_c, g, map) = human(&realm, "BHRI", "Replaced", Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            for id in [HIDDEN, PULSE, FED] {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                apply_template(m, g, g, &s);
            }
            let slotted: Vec<u32> = auras::auras(m, g)
                .unwrap()
                .holders
                .iter()
                .filter(|h| h.slot.is_some())
                .map(|h| h.spell)
                .collect();
            (slotted, has(m, g, HIDDEN), has(m, g, PULSE), has(m, g, FED))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (vec![], true, false, false),
        "the hidden one given no slot"
    );
}
