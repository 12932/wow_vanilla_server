//! Round 6's kill reward tests, ported from Thistle Tea's
//! `test/game/world/combat/kill_reward_test.exs` (docs/parity/matrix.csv):
//! whose kill a creature is, by its tap, the tapper's group then, a pet's
//! master and a possessed player's controller.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, combat};
use crate::game::tests_p3_combat::{BRUTE, START, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

fn xp_for(victim: Guid) -> impl FnMut(&ServerOpcodeMessage) -> Option<u32> {
    move |m| match m {
        ServerOpcodeMessage::SMSG_LOG_XPGAIN(x) if x.target == victim => Some(x.total_exp),
        _ => None,
    }
}

async fn player(realm: &std::sync::Arc<crate::game::Realm>, name: &str) -> Client {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    // Hardy enough for a brute's blows meanwhile.
    on(&map, move |m| {
        let f = &mut m.players.get_mut(&g).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
        f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
    })
    .await;
    c.clear();
    c
}

/// `by` invites `c`, who accepts.
async fn joins(by: &mut Client, c: &mut Client, name: &str) {
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

/// Thistle Tea's `KillReward.selection` "keeps the original group after
/// its tagger joins another group", `eligible_members` "includes the
/// living original tagger after departure" and `group_rewards` "includes
/// the departed tagger once without crediting their new party": a
/// creature Alice tapped in her party is that party's kill, though she has
/// left it for Dee's: Bob, Cid and she share it as three, Dee has none
/// (mangos's `SetLootRecipient` keeps the group, `GetGroupLootRecipient`);
/// one tapped with her group gone since is her party's now.
#[tokio::test]
async fn a_kill_is_the_group_it_was_tapped_for() {
    let realm = harness::realm_with(crate::game::tests_p3_combat::data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Cid").await;
    let mut d = player(&realm, "Dee").await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    joins(&mut a, &mut b, "Bob").await;
    joins(&mut a, &mut c, "Cid").await;
    let (brute, other, tapped) = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let other = spawn(m, ga, BRUTE, 20, 3.0);
        crate::game::map::ai::attacked(m, brute, ga);
        combat::deal_damage(m, ga, brute, 1);
        let tapped = m.creature(brute).map(|c| (c.tapped_by, c.tap_group));
        (brute, other, tapped)
    })
    .await;
    let party = realm.social.groups.lock().group_of(ga).map(|g| g.id);
    assert_eq!(tapped, Some((Some(ga), party)), "tapped for her party");
    a.send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut a).await;
    joins(&mut d, &mut a, "Alice").await;
    for x in [&mut a, &mut b, &mut c, &mut d] {
        x.clear();
    }
    let three = on(&map, move |m| {
        let o = combat::kill_options(m, brute);
        let three = combat::group_shares(&[1, 1, 1], 20, &o).0[0];
        combat::deal_damage(m, ga, brute, 1_000_000);
        three
    })
    .await;
    for x in [&mut a, &mut b, &mut c] {
        assert_eq!(x.wait_for("the kill's XP", xp_for(brute)).await, three);
    }
    assert_eq!(
        d.gets(Duration::from_millis(100), xp_for(brute)).await,
        None,
        "not her new party's"
    );
    // Her old party gone, a creature tapped for it is her party's now.
    on(&map, move |m| {
        crate::game::map::tap::take(&m.realm.clone(), m.creature_mut(other).unwrap(), ga);
    })
    .await;
    b.send(CMSG_GROUP_DISBAND {}).await;
    handled(&mut b).await;
    assert!(realm.social.groups.lock().group_of(gc).is_none());
    a.clear();
    d.clear();
    let two = on(&map, move |m| {
        let o = combat::kill_options(m, other);
        let two = combat::group_shares(&[1, 1], 20, &o).0[0];
        combat::deal_damage(m, ga, other, 1_000_000);
        two
    })
    .await;
    assert_eq!(a.wait_for("the kill's XP", xp_for(other)).await, two);
    assert_eq!(d.wait_for("the kill's XP", xp_for(other)).await, two);
    let _ = gb;
}

/// Thistle Tea's `KillReward.selection` "resolves pet killing blows when
/// the creature was not tagged": a hunter's pet's kill of a creature no one
/// tapped is its master's, his XP as one alone's.
#[tokio::test]
async fn a_pets_kill_of_one_untapped_is_its_masters() {
    let (_realm, mut h, _gh, map, pet, wolf) = crate::game::tests_pets::tamed().await;
    h.clear();
    let (untapped, alone) = on(&map, move |m| {
        let untapped = m.creature(wolf).is_some_and(|c| c.tapped_by.is_none());
        let o = combat::kill_options(m, wolf);
        let alone = combat::group_shares(&[1], 1, &o).0[0];
        combat::deal_damage(m, pet, wolf, 1_000_000);
        (untapped, alone)
    })
    .await;
    assert!(untapped && alone > 0);
    assert_eq!(h.wait_for("the kill's XP", xp_for(wolf)).await, alone);
}

/// Thistle Tea's `KillReward.controlling_player` "credits a possessed
/// player's controller until control ends": a possessed player's blow taps
/// a creature for the one controlling them (mangos's
/// `GetBeneficiaryPlayer`), whose kill it is; let go, their kills are
/// their own again.
#[tokio::test]
async fn a_possessed_players_kill_is_their_controllers() {
    let realm = harness::realm_with(crate::game::tests_p3_combat::data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (brute, tapper, alone) = on(&map, move |m: &mut Map| {
        crate::game::map::pets::charmed(m, ga, gb, true);
        let brute = spawn(m, gb, BRUTE, 20, 3.0);
        combat::engage(m, gb, brute);
        combat::deal_damage(m, gb, brute, 1);
        let tapper = m.creature(brute).and_then(|c| c.tapped_by);
        let o = combat::kill_options(m, brute);
        let alone = combat::group_shares(&[1], 20, &o).0[0];
        combat::deal_damage(m, gb, brute, 1_000_000);
        (brute, tapper, alone)
    })
    .await;
    assert_eq!(tapper, Some(ga), "tapped for the controller");
    assert_eq!(a.wait_for("the kill's XP", xp_for(brute)).await, alone);
    assert_eq!(
        b.gets(Duration::from_millis(100), xp_for(brute)).await,
        None
    );
    let next = on(&map, move |m| {
        crate::game::map::pets::uncharmed(m, ga, gb);
        let next = spawn(m, gb, BRUTE, 20, 3.0);
        combat::engage(m, gb, next);
        combat::deal_damage(m, gb, next, 1_000_000);
        next
    })
    .await;
    assert_eq!(b.wait_for("his own kill's XP", xp_for(next)).await, alone);
}

/// Bug hunt (combat): a creature's tap went as its tapper left its list
/// (died, feigned death, vanished), so the next to strike it took a kill
/// they had no part of. Its tap stays until it evades or respawns
/// (mangos-classic's `TriggerEvadeEvents`, Thistle Tea's
/// `Engagement.leave`): the kill is still the tapper's.
#[tokio::test]
async fn a_tap_outlasts_its_tapper_leaving_the_fight() {
    let realm = harness::realm_with(crate::game::tests_p3_combat::data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (brute, kept) = on(&map, move |m: &mut Map| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        combat::engage(m, ga, brute);
        combat::deal_damage(m, ga, brute, 1);
        combat::engage(m, gb, brute);
        combat::deal_damage(m, gb, brute, 1);
        // Alice feigns death: off its list, it fights Bob on.
        crate::game::map::ai::forget(m, ga);
        let kept = m.creature(brute).and_then(|c| c.tapped_by);
        combat::deal_damage(m, gb, brute, 1_000_000);
        (brute, kept)
    })
    .await;
    assert_eq!(kept, Some(ga), "still Alice's");
    a.wait_for("the kill's XP", xp_for(brute)).await;
    assert_eq!(
        b.gets(Duration::from_millis(100), xp_for(brute)).await,
        None,
        "not Bob's"
    );
}

/// The SITREP's call on the tap (mangos-classic's): only damage taps a
/// creature (`Unit::DealDamage` → `SetLootRecipient`), not its aggro
/// (Thistle Tea's `combat_entered` → `maybe_tap`). A watcher that notices
/// Alice walking by and runs at her is no one's; Bob, who hits it first,
/// has it: his kill, his XP and his loot, Alice none.
#[tokio::test]
async fn a_creature_that_aggroes_a_passer_by_is_whoever_hurts_it_first() {
    use crate::game::tests_p7_combat::{WATCHER, creature_at, look, off};
    let realm = harness::realm_with(crate::game::tests_p7_combat::data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let (ga, gb) = (a.guid, b.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (w, aggro, tapped, loot) = on(&map, move |m: &mut Map| {
        for g in [ga, gb] {
            // Short of the cap, to have XP.
            crate::game::tests_p3_combat::level(m, g, 59);
        }
        crate::game::tests_p6_wspell::put(m, ga, off(0.0, 0.0, 0.0));
        crate::game::tests_p6_wspell::put(m, gb, off(0.0, 25.0, 0.0));
        let w = creature_at(m, ga, WATCHER, off(4.0, 0.0, 0.0));
        look(m);
        let c = m.creature(w).unwrap();
        let aggro = (c.combat.attacking && c.combat.victim == ga, c.tapped_by);
        combat::deal_damage(m, gb, w, 1);
        let tapped = m.creature(w).unwrap().tapped_by;
        combat::deal_damage(m, gb, w, 10_000_000);
        let l = crate::game::map::loot::loot_ref(m, w);
        let loot = l.map(|l| (l.may_loot(ga), l.may_loot(gb)));
        (w, aggro, tapped, loot)
    })
    .await;
    assert_eq!(aggro, (true, None), "it runs at Alice, no one's tap");
    assert_eq!(tapped, Some(gb), "Bob's first hit taps it");
    assert_eq!(loot, Some((false, true)), "Bob's loot, not Alice's");
    b.wait_for("Bob's XP", xp_for(w)).await;
    assert_eq!(
        a.gets(Duration::from_millis(100), xp_for(w)).await,
        None,
        "none for Alice"
    );
}

/// Threat without damage taps no one (mangos's tap is in `DealDamage`,
/// past its no-damage return): a heal's threat (Bob healing Alice, whom
/// the brute fights), a taunt (Cid's), a miss. Cid's hit, after, does.
#[tokio::test]
async fn a_taunt_a_heal_or_a_miss_taps_no_one() {
    let realm = harness::realm_with(crate::game::tests_p3_combat::data());
    let a = player(&realm, "Alice").await;
    let b = player(&realm, "Bob").await;
    let c = player(&realm, "Cid").await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m: &mut Map| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        crate::game::map::ai::attacked(m, brute, ga);
        let f = m.unit_fields_mut(ga).unwrap();
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        f.set_u32(UNIT_FIELD_HEALTH, max - 500);
        crate::game::map::spells::heal(m, gb, ga, 2050, 200, false);
        let tap = |m: &Map| m.creature(brute).unwrap().tapped_by;
        let on_list = |m: &Map, g: Guid| m.creature(brute).unwrap().threat.iter().any(|t| t.0 == g);
        let healed = (on_list(m, gb), tap(m));
        crate::game::map::ai::taunted(m, brute, gc);
        let taunted = (m.creature(brute).unwrap().combat.victim == gc, tap(m));
        crate::game::tests_p3_combat::forced_swing(m, gc, brute, combat::Outcome::Miss);
        let missed = tap(m);
        combat::deal_damage(m, gc, brute, 1);
        (healed, taunted, missed, tap(m))
    })
    .await;
    assert_eq!(
        out.0,
        (true, None),
        "the heal's threat: on its list, no tap"
    );
    assert_eq!(out.1, (true, None), "taunted: on Cid, no tap");
    assert_eq!(out.2, None, "a miss: none");
    assert_eq!(out.3, Some(gc), "Cid's hit taps it");
}

/// A pet's first hit taps a creature for its owner (mangos's
/// `GetBeneficiaryPlayer`), whose kill, by the pet, it is.
#[tokio::test]
async fn a_pets_first_hit_taps_for_its_owner() {
    use crate::game::tests_p7_combat::{creature_at, off};
    let (_realm, mut h, gh, pet, map) = crate::game::tests_p8_pets_owner::hunter().await;
    let (brute, untapped, tapped) = on(&map, move |m: &mut Map| {
        // Short of the cap, to have XP.
        crate::game::tests_p3_combat::level(m, gh, 59);
        let brute = creature_at(m, gh, BRUTE, off(3.0, 0.0, 0.0));
        crate::game::map::ai::attacked(m, brute, pet);
        let untapped = m.creature(brute).unwrap().tapped_by;
        combat::deal_damage(m, pet, brute, 1);
        let tapped = m.creature(brute).unwrap().tapped_by;
        combat::deal_damage(m, pet, brute, 10_000_000);
        (brute, untapped, tapped)
    })
    .await;
    assert_eq!(untapped, None, "the pet's pull alone: no one's");
    assert_eq!(tapped, Some(gh), "its hit: its owner's");
    h.wait_for("the owner's XP", xp_for(brute)).await;
}

/// A group member's first hit taps a creature for the group: aggroed on
/// a stranger, Cid, then hit by Bob, it is Alice's and Bob's party's kill,
/// shared by the two of them, and not grey to Alice.
#[tokio::test]
async fn a_members_hit_taps_for_the_group() {
    let realm = harness::realm_with(crate::game::tests_p3_combat::data());
    let mut a = player(&realm, "Alice").await;
    let mut b = player(&realm, "Bob").await;
    let mut c = player(&realm, "Cid").await;
    let (ga, gb, gc) = (a.guid, b.guid, c.guid);
    joins(&mut a, &mut b, "Bob").await;
    let party = realm.social.groups.lock().group_of(ga).map(|g| g.id);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (brute, tapped, holds, share) = on(&map, move |m: &mut Map| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        crate::game::map::ai::attacked(m, brute, gc);
        combat::deal_damage(m, gb, brute, 1);
        let k = m.creature(brute).unwrap();
        let tapped = (k.tapped_by, k.tap_group);
        let holds = [ga, gb, gc].map(|g| crate::game::map::tap::holds(m, k, g));
        let o = combat::kill_options(m, brute);
        let share = combat::group_shares(&[1, 1], 20, &o).0[0];
        combat::deal_damage(m, ga, brute, 10_000_000);
        (brute, tapped, holds, share)
    })
    .await;
    assert!(party.is_some());
    assert_eq!(tapped, (Some(gb), party), "Bob's, for the party");
    assert_eq!(holds, [true, true, false], "the party's, not Cid's");
    assert_eq!(a.wait_for("Alice's share", xp_for(brute)).await, share);
    assert_eq!(b.wait_for("Bob's share", xp_for(brute)).await, share);
    assert_eq!(
        c.gets(Duration::from_millis(100), xp_for(brute)).await,
        None,
        "none for Cid"
    );
}
