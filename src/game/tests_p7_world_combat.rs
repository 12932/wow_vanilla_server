//! Round 7's world tests, ported from Thistle Tea's
//! `test/game/world/combat/` (docs/parity/matrix.csv): whose a kill is
//! (the tapper's group as they tapped it, a pet's master, a possessed
//! player's possessor) and how its experience is split.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::PLAYER_XP;
use crate::game::harness::Client;
use crate::game::map::combat::{self, KillXp};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, HELPER, data, level, setup_with, spawn};
use crate::game::tests_p3_entity::on;

/// `leader` brings `member` (called `name`) into their group.
async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
}

fn xp(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(PLAYER_XP)
}

/// A player of level ten beside Alice, logged in on `account`.
async fn beside(
    realm: &std::sync::Arc<crate::game::Realm>,
    map: &MapHandle,
    account: &str,
    name: &str,
) -> (Client, Guid) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    let (x, y, z) = crate::game::tests_p3_combat::START;
    c.heartbeat(x, y, z).await;
    on(map, move |m| level(m, g, 10)).await;
    (c, g)
}

/// Thistle Tea's `KillReward.selection/3` "keeps the original group after
/// its tagger joins another group", `eligible_members/3` "includes the
/// living original tagger after departure" and `group_rewards/3`
/// "includes the departed tagger once without crediting their new
/// party": Alice taps a brute grouped with Bob and Cid, leaves them and
/// joins Dan; the kill is shared by Alice, Bob and Cid, a third each, and
/// Dan has none of it (mangos's `GetGroupLootRecipient`, the tapper
/// rewarded with the group they tapped in).
#[tokio::test]
async fn a_kill_is_the_tapping_groups_after_its_tapper_leaves_for_another() {
    let (realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    on(&map, move |m| level(m, ga, 10)).await;
    let (mut b, gb) = beside(&realm, &map, "B", "Bob").await;
    let (mut c, gc) = beside(&realm, &map, "C", "Cid").await;
    let (mut d, gd) = beside(&realm, &map, "D", "Dan").await;
    group(&mut b, &mut a, "Alice").await;
    group(&mut b, &mut c, "Cid").await;
    let brute = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 10, 3.0);
        crate::game::map::ai::attacked(m, brute, ga);
        combat::deal_damage(m, ga, brute, 1);
        brute
    })
    .await;
    a.clear();
    a.send(CMSG_GROUP_DISBAND {}).await;
    a.wait_for("the empty roster", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    group(&mut a, &mut d, "Dan").await;
    let (before, after) = on(&map, move |m| {
        let before = [ga, gb, gc, gd].map(|g| xp(m, g));
        combat::kill(m, ga, brute);
        (before, [ga, gb, gc, gd].map(|g| xp(m, g)))
    })
    .await;
    let gained: Vec<u32> = before.iter().zip(after).map(|(b, a)| a - b).collect();
    let (shares, _) = combat::group_shares(&[10, 10, 10], 10, &KillXp::default());
    assert!(shares[0] > 0);
    assert_eq!(
        gained,
        [shares[0], shares[1], shares[2], 0],
        "a third each, none for Dan"
    );
}

/// Thistle Tea's `KillReward.selection/3` "resolves pet killing blows
/// when the creature was not tagged" and `controlling_player/2` "credits
/// a possessed player's controller until control ends": a brute no one
/// tapped, killed by Alice's pet, is her kill; one killed by Bob while she
/// possesses him is hers too; let go, his next kill is his.
#[tokio::test]
async fn a_pets_kill_and_a_possessed_players_are_their_masters() {
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    on(&map, move |m| level(m, ga, 10)).await;
    let (_b, gb) = beside(&realm, &map, "B", "Bob").await;
    let out = on(&map, move |m| {
        let mut out = Vec::new();
        let pet = spawn(m, ga, HELPER, 10, 1.0);
        m.creature_mut(pet).unwrap().owner = Some(ga);
        let (a0, b0) = (xp(m, ga), xp(m, gb));
        let brute = spawn(m, ga, BRUTE, 10, 3.0);
        combat::kill(m, pet, brute);
        out.push((xp(m, ga) - a0, xp(m, gb) - b0));
        crate::game::map::pets::possess::take(m, ga, gb);
        let (a0, b0) = (xp(m, ga), xp(m, gb));
        let brute = spawn(m, ga, BRUTE, 10, 3.0);
        combat::kill(m, gb, brute);
        out.push((xp(m, ga) - a0, xp(m, gb) - b0));
        crate::game::map::pets::possess::release(m, gb);
        let (a0, b0) = (xp(m, ga), xp(m, gb));
        let brute = spawn(m, ga, BRUTE, 10, 3.0);
        combat::kill(m, gb, brute);
        out.push((xp(m, ga) - a0, xp(m, gb) - b0));
        out
    })
    .await;
    let whole = combat::kill_xp(10, 10, false, 1.0);
    assert_eq!(
        out,
        [(whole, 0), (whole, 0), (0, whole)],
        "the pet's kill hers, the possessed's hers, the freed's his"
    );
}

/// Thistle Tea's `KillReward.group_rewards/3` "splits NPC pet experience
/// after applying its reduction", as mangos-classic has it since round 11
/// (`XP::Gain` gives nothing for `IsPet()`): a creature's pet is worth
/// nothing, alone or to two of level ten.
#[test]
fn an_npcs_pet_is_worth_nothing_alone_or_split() {
    let pet = KillXp {
        pet_victim: true,
        ..Default::default()
    };
    assert_eq!(combat::kill_xp_with(10, 10, 10, &pet), 0, "a pet's nothing");
    let (shares, top) = combat::group_shares(&[10, 10], 10, &pet);
    assert_eq!((shares, top), (vec![0, 0], Some(10)));
}

const FEIGN: u32 = 997_200;

/// Thistle Tea's `FeignDeath.prepare/5` "ignores distant, stale, dead and
/// player-controlled references" and "only checks creatures already on
/// the hunter's hostile reference list": Feign Death is seen through by
/// a brute fighting her beside her that resists it (rolled as a spell of
/// hers); not by one fighting her dead, one a player owns, one sixty yards
/// off, nor one beside her that isn't fighting her, as sure to resist.
#[tokio::test]
async fn feign_death_is_seen_through_only_by_a_living_wild_creature_near_that_fights_the_hunter() {
    use crate::game::map::spells::class_scripts::feign_death_resisted;
    let mut d = data();
    crate::game::tests_p3_cast_kit::insert(
        &mut d,
        [crate::game::tests_p3_cast_kit::bare(FEIGN, 0)],
    );
    let (realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    on(&map, move |m| level(m, ga, 10)).await;
    let (_b, gb) = beside(&realm, &map, "B", "Bob").await;
    let out = on(&map, move |m| {
        // Hard to land a spell on, a hit is all but sure to miss (a
        // miss certain, without the rolls).
        let hard = |m: &mut Map, brute: Guid| {
            crate::game::tests_p3_combat::aura_of(
                m,
                brute,
                997_201,
                crate::game::map::auras::AURA_MOD_ATTACKER_SPELL_HIT_CHANCE,
                127,
                -100,
            );
        };
        let fighting = |m: &mut Map, yards: f32| {
            let brute = spawn(m, ga, BRUTE, 30, yards);
            hard(m, brute);
            crate::game::map::ai::attacked(m, brute, ga);
            brute
        };
        let mut out = Vec::new();
        let near = fighting(m, 5.0);
        out.push(feign_death_resisted(m, ga, FEIGN));
        combat::kill(m, ga, near);
        out.push(feign_death_resisted(m, ga, FEIGN));
        let owned = fighting(m, 5.0);
        m.creature_mut(owned).unwrap().owner = Some(gb);
        out.push(feign_death_resisted(m, ga, FEIGN));
        m.remove_creature(owned);
        let far = fighting(m, 60.0);
        out.push(feign_death_resisted(m, ga, FEIGN));
        m.remove_creature(far);
        let idle = spawn(m, ga, BRUTE, 30, 5.0);
        hard(m, idle);
        out.push(feign_death_resisted(m, ga, FEIGN));
        out
    })
    .await;
    assert_eq!(
        out,
        [true, false, false, false, false],
        "near and fighting; dead; a player's; far; not fighting her"
    );
}
