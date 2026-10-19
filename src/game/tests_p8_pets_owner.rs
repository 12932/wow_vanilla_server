//! Round 8's pets (Thistle Tea's `controlled_combat_test.exs`): how a pet's,
//! totem's or guardian's fight draws its master in. wvs does as mangos:
//! contact (a blow, landed or not) brings in a master who can join it, on
//! the foe's threat list at none of his own, and that entry holds him there
//! while the foe fights (where Thistle Tea holds him by his pet's references
//! and a five-second window).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::pets::Kind;
use crate::game::map::{Map, MapHandle, MapKey, ai, combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE};
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{BEAST_WATCHER, creature_at, in_combat, off};

/// A totem, a guardian, and a summon that keeps its owner out of its
/// fights (vmangos's `NO_OWNER_THREAT`: a Sentry Totem, an Eye of Kilrogg).
const TOTEM: u32 = 3770;
const GUARD: u32 = 3771;
const SENTRY: u32 = 3772;
/// A brute no blow touches.
const IMMUNE: u32 = 3773;
/// `UNIT_FLAG_NON_ATTACKABLE`.
const NOT_ATTACKABLE: u32 = 0x2;

fn data() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    d.creature_templates.insert(TOTEM, p3::template(TOTEM, 11));
    d.creature_templates.insert(GUARD, p3::template(GUARD, 7));
    let mut sentry = p3::template(SENTRY, 7);
    sentry.static_flags2 = 0x20;
    d.creature_templates.insert(SENTRY, sentry);
    let mut immune = p3::template(IMMUNE, 7);
    immune.school_immune_mask = 1;
    d.creature_templates.insert(IMMUNE, immune);
    d
}

/// A level-60 hunter at the start, with her pet two yards east.
async fn hunter() -> (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    Guid,
    MapHandle,
) {
    let realm = crate::game::harness::realm_with(data());
    let mut h = crate::game::harness::Client::connect(&realm, "H");
    let gh = h.enter("Hana", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let pet = map
        .query(move |m| {
            p3::level(m, gh, 60);
            put(m, gh, off(0.0, 0.0, 0.0));
            crate::game::map::pets::give_pet(m, gh, BEAST_WATCHER, 60).expect("a pet");
            let pet = crate::game::map::pets::pet_of(m, gh).unwrap();
            put(m, pet, off(2.0, 0.0, 0.0));
            let f = &mut m.creature_mut(pet).unwrap().fields;
            f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
            f.set_u32(UNIT_FIELD_HEALTH, 100_000);
            pet
        })
        .await
        .unwrap();
    (realm, h, gh, pet, map)
}

/// `of`'s threat on `on`'s list, if it is on it.
fn threat(m: &Map, on: Guid, of: Guid) -> Option<f32> {
    m.creature(on)?
        .threat
        .iter()
        .find(|(g, _)| *g == of)
        .map(|(_, t)| *t)
}

/// Out of every fight, and the creatures gone.
fn apart(m: &mut Map, units: &[Guid], creatures: &[Guid]) {
    for c in creatures {
        m.remove_creature(*c);
    }
    for u in units {
        combat::leave_combat(m, *u);
    }
}

/// The next look at `g`'s fight, a check's time on.
fn look_at(m: &mut Map, g: Guid) {
    m.now += combat::COMBAT_CHECK;
    let now = m.now;
    m.combat_mut(g).unwrap().next_check = Some(now);
    combat::update_players(m);
}

/// A summon of `kind` and `entry` for `owner` at `at`, in his slots.
fn summon(m: &mut Map, owner: Guid, entry: u32, kind: Kind, at: Vector3d) -> Guid {
    let g = crate::game::map::pets::testing::spawn(m, owner, entry, kind, 60, at, 0.0);
    let p = m.players.get_mut(&owner).unwrap();
    match kind {
        Kind::Totem => p.owned.totems[0] = Some(g),
        _ => p.owned.guardians.push(g),
    }
    g
}

/// Thistle Tea's "commands and threat membership wait for actual contact"
/// and "outgoing contact adds the owner to the opponent with zero threat":
/// the pet sent at a foe out of reach brings no one into the fight; each
/// blow of its, landed, missed, dodged, parried or on the immune, brings its
/// master in, on the foe's list at no threat of his own; a creature on its
/// way home isn't struck at all, and draws no one in.
#[tokio::test]
async fn a_pets_blow_landed_or_not_brings_its_master_in_at_no_threat_but_its_order_doesnt() {
    let (_realm, _h, gh, pet, map) = hunter().await;
    let r = map
        .query(move |m| {
            let far = creature_at(m, gh, BRUTE, off(30.0, 0.0, 0.0));
            crate::game::map::pets::attack_target(m, pet, far);
            let ordered = (in_combat(m, gh), in_combat(m, pet), threat(m, far, gh));
            apart(m, &[gh, pet], &[far]);
            let mut blows = Vec::new();
            for outcome in [
                combat::Outcome::Hit,
                combat::Outcome::Miss,
                combat::Outcome::Dodge,
                combat::Outcome::Parry,
            ] {
                let b = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
                p3::forced_swing(m, pet, b, outcome);
                blows.push((in_combat(m, gh), threat(m, b, gh)));
                apart(m, &[gh, pet], &[b]);
            }
            // Immune to the blow: contact all the same.
            let b = creature_at(m, gh, IMMUNE, off(3.0, 0.0, 0.0));
            p3::forced_swing(m, pet, b, combat::Outcome::Hit);
            blows.push((in_combat(m, gh), threat(m, b, gh)));
            apart(m, &[gh, pet], &[b]);
            // On its way home: no blow, no fight.
            let e = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
            m.creature_mut(e).unwrap().home = off(25.0, 0.0, 0.0);
            ai::enter_evade(m, e);
            let evading = m.creature(e).unwrap().evading;
            p3::forced_swing(m, pet, e, combat::Outcome::Hit);
            let evaded = (
                evading,
                in_combat(m, gh),
                threat(m, e, gh),
                m.combat(pet).unwrap().attacking,
            );
            (ordered, blows, evaded)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (false, false, None), "the order alone");
    assert_eq!(
        r.1,
        vec![(true, Some(0.0)); 5],
        "hit, miss, dodge, parry, immune"
    );
    assert_eq!(r.2, (true, false, None, false), "on its way home");
}

/// Thistle Tea's "incoming contact starts the owner window without copying
/// pet threat", "totems propagate incoming damage contact", "guardian and
/// totem contact uses their canonical owner slots" and "a lethal pet hit
/// still starts the current owner's combat window": a creature taking on the
/// pet brings its master in at no threat, which stays none however the pet
/// hurts it; a blow on a totem or a guardian, or a guardian's, brings in
/// their master; and so does the blow that kills the pet, the foe turning on
/// him after (mangos's master on the list, where Thistle Tea lets him go).
#[tokio::test]
async fn a_blow_on_a_pet_totem_or_guardian_brings_its_master_in_a_killing_one_too() {
    let (_realm, _h, gh, pet, map) = hunter().await;
    let r = map
        .query(move |m| {
            let b = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
            ai::attacked(m, b, pet);
            p3::forced_swing(m, b, pet, combat::Outcome::Hit);
            let taken = (in_combat(m, gh), threat(m, b, gh));
            p3::forced_swing(m, pet, b, combat::Outcome::Hit);
            let hurt = (threat(m, b, gh), threat(m, b, pet).is_some_and(|t| t > 0.0));
            apart(m, &[gh, pet], &[b]);
            // A totem struck, a guardian struck, a guardian's blow.
            let mut summons = Vec::new();
            for (entry, kind, strikes) in [
                (TOTEM, Kind::Totem, false),
                (GUARD, Kind::Guardian, false),
                (GUARD, Kind::Guardian, true),
            ] {
                let s = summon(m, gh, entry, kind, off(-2.0, 0.0, 0.0));
                let b = creature_at(m, gh, BRUTE, off(-3.0, 0.0, 0.0));
                if strikes {
                    p3::forced_swing(m, s, b, combat::Outcome::Hit);
                } else {
                    p3::forced_swing(m, b, s, combat::Outcome::Hit);
                }
                summons.push(in_combat(m, gh));
                apart(m, &[gh, pet, s], &[b, s]);
            }
            // The blow that kills the pet.
            let b = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
            ai::attacked(m, b, pet);
            m.creature_mut(pet)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 1);
            p3::forced_swing(m, b, pet, combat::Outcome::Hit);
            let killed = (
                m.unit_view(pet).is_some_and(|u| u.alive),
                in_combat(m, gh),
                threat(m, b, gh),
            );
            crate::game::tests_p7_combat::look(m);
            look_at(m, gh);
            let after = (in_combat(m, gh), m.creature(b).unwrap().combat.victim == gh);
            (taken, hurt, summons, killed, after)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (true, Some(0.0)), "taken on: in, at no threat");
    assert_eq!(r.1, (Some(0.0), true), "the pet's threat its own");
    assert_eq!(r.2, vec![true; 3], "a totem's, a guardian's both ways");
    assert_eq!(r.3, (false, true, Some(0.0)), "the pet's killing blow");
    assert_eq!(r.4, (true, true), "the foe turns on him");
}

/// Thistle Tea's "Feign Death suppresses incoming contact", "untargetable
/// owners ignore incoming contact and outgoing threat", "dead creatures,
/// NPC owners and no-owner-threat templates do not propagate" and
/// "discarded ownership and distant or exempt companions cannot send late
/// contact": a master lying in a believed Feign Death isn't drawn in by a
/// blow on his pet; one out of every fight (not attackable) by none, either
/// way, nor put on the foe's list; nor a dead one; and a summon whose
/// template keeps its fights its own (`NO_OWNER_THREAT`) draws no one in.
#[tokio::test]
async fn a_master_who_cant_join_or_one_his_summon_keeps_out_isnt_drawn_into_its_fight() {
    let (_realm, _h, gh, pet, map) = hunter().await;
    let r = map
        .query(move |m| {
            let state = |m: &Map, b: Guid| (in_combat(m, gh), threat(m, b, gh));
            // Feigning: a blow on the pet.
            combat::feigned(m, gh);
            let b = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
            ai::attacked(m, b, pet);
            p3::forced_swing(m, b, pet, combat::Outcome::Hit);
            let feigning = state(m, b);
            m.combat_mut(gh).unwrap().feigning = false;
            apart(m, &[gh, pet], &[b]);
            // Not attackable: either way.
            let flags = |m: &mut Map, on: bool| {
                m.unit_fields_mut(gh)
                    .unwrap()
                    .set_flag(UNIT_FIELD_FLAGS, NOT_ATTACKABLE, on)
            };
            flags(m, true);
            let mut unattackable = Vec::new();
            for incoming in [false, true] {
                let b = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
                if incoming {
                    ai::attacked(m, b, pet);
                    p3::forced_swing(m, b, pet, combat::Outcome::Hit);
                } else {
                    p3::forced_swing(m, pet, b, combat::Outcome::Hit);
                }
                unattackable.push(state(m, b));
                apart(m, &[gh, pet], &[b]);
            }
            flags(m, false);
            // A summon that keeps him out, either way.
            let mut kept_out = Vec::new();
            for incoming in [false, true] {
                let s = summon(m, gh, SENTRY, Kind::Guardian, off(-2.0, 0.0, 0.0));
                let b = creature_at(m, gh, BRUTE, off(-3.0, 0.0, 0.0));
                if incoming {
                    ai::attacked(m, b, s);
                    p3::forced_swing(m, b, s, combat::Outcome::Hit);
                } else {
                    p3::forced_swing(m, s, b, combat::Outcome::Hit);
                }
                kept_out.push(state(m, b));
                apart(m, &[gh, s], &[b, s]);
            }
            // Dead: the guardian's blow brings no one back.
            let s = summon(m, gh, GUARD, Kind::Guardian, off(-2.0, 0.0, 0.0));
            let b = creature_at(m, gh, BRUTE, off(-3.0, 0.0, 0.0));
            crate::game::map::death::kill(m, gh);
            p3::forced_swing(m, s, b, combat::Outcome::Hit);
            let dead = state(m, b);
            (feigning, unattackable, kept_out, dead)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (false, None), "feigning");
    assert_eq!(r.1, vec![(false, None); 2], "not attackable");
    assert_eq!(r.2, vec![(false, None); 2], "kept out by its summon");
    assert_eq!(r.3, (false, None), "dead");
}

/// Thistle Tea's "summoned pet references retain combat beyond contact",
/// "pet removal preserves an overlapping owner threat reference", "death,
/// removal, replacement and stale enemy references release inherited
/// combat" and "threat membership alone cannot initiate owner combat": the
/// master stays in his pet's fight as long as the foe holds him on its list
/// (mangos's entry of his own, where Thistle Tea has the pet's), the pet sent
/// away and the foe turning on him; the foe going home lets him go; and being
/// on a list alone puts no one in a fight.
#[tokio::test]
async fn a_master_stays_in_his_pets_fight_while_its_foe_holds_him_and_leaves_with_it() {
    let (_realm, _h, gh, pet, map) = hunter().await;
    let r = map
        .query(move |m| {
            let b = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
            p3::forced_swing(m, pet, b, combat::Outcome::Hit);
            // Twenty seconds on, no blow since.
            for _ in 0..17 {
                look_at(m, gh);
            }
            let held = in_combat(m, gh);
            // The pet sent away: the foe turns on him.
            crate::game::map::pets::dismiss(m, pet);
            look_at(m, gh);
            let dismissed = (in_combat(m, gh), m.creature(b).unwrap().combat.victim == gh);
            // Home it goes: he is out at the next look.
            m.creature_mut(b).unwrap().home = off(25.0, 0.0, 0.0);
            ai::enter_evade(m, b);
            look_at(m, gh);
            let evaded = in_combat(m, gh);
            // On a list, and no fight: none for him.
            let idle = creature_at(m, gh, BRUTE, off(-30.0, 0.0, 0.0));
            m.creature_mut(idle).unwrap().threat.push((gh, 0.0));
            look_at(m, gh);
            m.now += Duration::from_secs(1);
            combat::update_players(m);
            (held, dismissed, evaded, in_combat(m, gh))
        })
        .await
        .unwrap();
    assert!(r.0, "held while the foe holds him");
    assert_eq!(r.1, (true, true), "the pet gone, the foe on him");
    assert!(!r.2, "out once it goes home");
    assert!(!r.3, "a list alone");
}
