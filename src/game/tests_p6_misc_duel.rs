//! Round 6's duel tests, ported from Thistle Tea's
//! `test/game/core/duel/dueling_test.exs` (docs/parity/matrix.csv): a
//! duel's arbiter and sides, the fights it leaves, what its end undoes, and
//! the blows that end it.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::map::duel::{self, Phase};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_player2_ammo::handled;

/// A slow (harmful), and a strength buff (helpful), of `id`.
fn spell(id: u32, harmful: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: if harmful {
            auras::AURA_MOD_DECREASE_SPEED
        } else {
            auras::AURA_MOD_STAT
        },
        base_points: if harmful { -51 } else { 9 },
        base_dice: 1,
        target_a: if harmful { 6 } else { 1 },
        ..Default::default()
    };
    s
}

fn put(m: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) {
    let amount = s.effects[0].base_points + 1;
    auras::apply_spell(
        m,
        caster,
        target,
        s,
        &[(0, amount)],
        auras::Context::default(),
    )
    .expect("put on");
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
}

/// Each side's arbiter and team.
fn sides(m: &Map, a: Guid, b: Guid) -> [(Guid, u32); 2] {
    [a, b].map(|g| {
        let f = &m.players[&g].fields;
        (f.get_guid(PLAYER_DUEL_ARBITER), f.get_u32(PLAYER_DUEL_TEAM))
    })
}

/// `a` challenges `b`; the flag.
async fn challenged(map: &MapHandle, a: Guid, b: &mut Client) -> Guid {
    let gb = b.guid;
    on(map, move |m| duel::challenge(m, a, gb)).await;
    b.wait_for("SMSG_DUEL_REQUESTED", |m| match m {
        ServerOpcodeMessage::SMSG_DUEL_REQUESTED(r) => Some(r.initiator),
        _ => None,
    })
    .await
}

/// `b` accepts `a`'s challenge, and the fight begins at once.
async fn begun(map: &MapHandle, a: Guid, b: &mut Client, flag: Guid) {
    let gb = b.guid;
    b.send(CMSG_DUEL_ACCEPTED { guid: flag }).await;
    handled(b).await;
    on(map, move |m| {
        for g in [a, gb] {
            if let Some(d) = m.players.get_mut(&g).and_then(|p| p.duel.as_mut()) {
                d.phase = Phase::Countdown {
                    until: Instant::now(),
                };
            }
        }
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while !on(map, move |m| duel::dueling(m, a, gb)).await {
        assert!(tokio::time::Instant::now() < deadline, "never began");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Thistle Tea's `Dueling.requested` and `started` "project the arbiter,
/// opponent, and team without entering combat" and "preserve existing
/// combat when the countdown ends": the flag is each side's arbiter from
/// the challenge (mangos's `EffectDuel`), their teams none until the fight,
/// then the challenger's 1 and the other's 2 (mangos's `UpdateDuelFlag`);
/// the fight's start puts no one in combat, and leaves one already fighting
/// a creature in that fight as it was.
#[tokio::test]
async fn a_duel_has_its_arbiter_at_the_challenge_and_starts_no_fight() {
    use crate::game::tests_p3_combat::{BRUTE, setup, spawn};
    let (realm, _a, ga, map) = setup(Class::Warrior).await;
    let (mut b, gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    let brute = on(&map, move |m| {
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
        f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        crate::game::map::combat::engage(m, ga, brute);
        brute
    })
    .await;
    let before = on(&map, move |m| {
        (in_combat(m, ga), m.combat(ga).and_then(|c| c.combat_until))
    })
    .await;
    assert!(before.0, "Alice fights the brute");
    let flag = challenged(&map, ga, &mut b).await;
    assert_eq!(
        on(&map, move |m| sides(m, ga, gb)).await,
        [(flag, 0), (flag, 0)],
        "the arbiter, no teams yet"
    );
    begun(&map, ga, &mut b, flag).await;
    let after = on(&map, move |m| {
        (
            sides(m, ga, gb),
            in_combat(m, gb),
            in_combat(m, ga),
            m.combat(ga).and_then(|c| c.combat_until),
            m.creature(brute)
                .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == ga)),
        )
    })
    .await;
    assert_eq!(after.0, [(flag, 1), (flag, 2)], "the sides");
    assert!(!after.1, "Bob in no fight");
    assert!(after.2 && after.4, "Alice's fight as it was");
    assert!(after.3 >= before.1, "its timer not cut short");
}

const OLD: u32 = 96_900;
const SLOW: u32 = 96_901;
const PET_SLOW: u32 = 96_902;
const BACK: u32 = 96_903;
const BUFF: u32 = 96_904;

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_pets::data();
    for (id, harmful) in [
        (OLD, true),
        (SLOW, true),
        (PET_SLOW, true),
        (BACK, true),
        (BUFF, false),
    ] {
        d.spells.spells.insert(id, spell(id, harmful));
    }
    d
}

/// The hunter of `tests_pets` with a tamed boar, and Bob, a dwarf warrior
/// beside him.
async fn hunter_and_bob() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
    Guid,
    Client,
) {
    use crate::game::tests_pets::{
        BOAR, CALL_PET, DISMISS_PET, TAME_BEAST, WOLF, cast, enter, learn, pet_bar, place,
    };
    let realm = harness::realm_with(data());
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, CALL_PET, DISMISS_PET]);
    let boar = place(&map, gh, BOAR, 0x70_0001, 4.0).await;
    let wolf = place(&map, gh, WOLF, 0x70_0002, 6.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    assert!(!pet.is_zero(), "tamed");
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Dwarf, Class::Warrior).await;
    b.sees(gh).await;
    h.sees(gb).await;
    (realm, h, gh, map, pet, wolf, b)
}

/// Thistle Tea's `Dueling.finish` "removes duel debuffs and clears
/// duel-only combat state" and "notifies the attacked opponent after
/// selection changes": a duel yielded takes off the harmful auras each
/// side, and the other's pet, put on the other since the fight began, a
/// harm from before it and one's own buff kept (mangos's `DuelComplete`),
/// and the combo points each has on the other; it clears the arbiter and
/// the teams, ends both fights, and the one swinging at the other is told
/// the swings stop at the one they were at, though another is selected
/// (the selection is the client's, as mangos's `DuelComplete` leaves it).
#[tokio::test]
async fn a_duels_end_undoes_what_each_did_the_other() {
    let (_realm, mut h, gh, map, pet, wolf, mut b) = hunter_and_bob().await;
    let gb = b.guid;
    on(&map, move |m| {
        let d = m.realm.data.clone();
        put(m, gh, gb, &d.spells.spells[&OLD]);
    })
    .await;
    let flag = challenged(&map, gh, &mut b).await;
    begun(&map, gh, &mut b, flag).await;
    on(&map, move |m| {
        let d = m.realm.data.clone();
        let s = |id| d.spells.spells[&id].clone();
        put(m, gh, gb, &s(SLOW));
        put(m, pet, gb, &s(PET_SLOW));
        put(m, gb, gh, &s(BACK));
        put(m, gh, gh, &s(BUFF));
        crate::game::map::spells::add_combo_points(m, gb, gh, 3);
        crate::game::map::spells::add_combo_points(m, gh, gb, 2);
        crate::game::map::combat::start_attack(m, gh, gb);
    })
    .await;
    h.send(CMSG_SET_SELECTION { target: wolf }).await;
    handled(&mut h).await;
    let held = on(&map, move |m| {
        (
            has(m, gb, SLOW) && has(m, gb, PET_SLOW) && has(m, gh, BACK),
            m.caster(gb).map(|c| c.combo_points),
            m.caster(gh).map(|c| c.combo_points),
            m.combat(gh).is_some_and(|c| c.attacking && c.victim == gb),
        )
    })
    .await;
    assert_eq!(held, (true, Some(3), Some(2), true), "the fight's state");
    h.clear();
    b.send(CMSG_DUEL_CANCELLED { guid: Guid::zero() }).await;
    let stopped = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTOP(s) if s.player == gh => Some(s.enemy),
        _ => None,
    };
    assert_eq!(h.wait_for("SMSG_ATTACKSTOP", stopped).await, gb);
    assert_eq!(h.gets(NONE_WITHIN, stopped).await, None, "once");
    let after = on(&map, move |m| {
        (
            [
                has(m, gb, OLD),
                has(m, gb, SLOW),
                has(m, gb, PET_SLOW),
                has(m, gh, BACK),
                has(m, gh, BUFF),
            ],
            m.caster(gb).map(|c| c.combo_points),
            m.caster(gh).map(|c| c.combo_points),
            sides(m, gh, gb),
            [m.players[&gh].duel.is_none(), m.players[&gb].duel.is_none()],
            [in_combat(m, gh), in_combat(m, gb)],
            m.combat(gh).is_some_and(|c| c.attacking),
        )
    })
    .await;
    assert_eq!(
        after.0,
        [true, false, false, false, true],
        "the fight's harm gone, what came before and the buff kept"
    );
    assert_eq!((after.1, after.2), (Some(0), Some(0)), "no combo points");
    assert_eq!(after.3, [(Guid::zero(), 0); 2]);
    assert_eq!(after.4, [true, true]);
    assert_eq!(after.5, [false, false], "both fights over");
    assert!(!after.6);
}

/// Thistle Tea's duel lethal damage: "an opponent pet receives duel credit
/// through its owner" and "third-party lethal damage kills normally and
/// interrupts the duel": the opponent's pet's blow that would fell one
/// leaves them at 1 and loses them the duel to its master (mangos's
/// `DealDamage`, by the controlling player); a creature's kills them, and
/// the duel ends interrupted, with no winner.
#[tokio::test]
async fn a_pets_blow_wins_its_master_the_duel_and_anothers_kills() {
    let (_realm, mut h, gh, map, pet, wolf, mut b) = hunter_and_bob().await;
    let gb = b.guid;
    let flag = challenged(&map, gh, &mut b).await;
    begun(&map, gh, &mut b, flag).await;
    h.clear();
    let health = on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, pet, gb, 100_000);
        m.players[&gb].fields.get_u32(UNIT_FIELD_HEALTH)
    })
    .await;
    assert_eq!(health, 1, "left at 1");
    let winner = h
        .wait_for("SMSG_DUEL_WINNER", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_WINNER(w) => {
                Some((w.reason, w.initiator_name.clone(), w.opponent_name.clone()))
            }
            _ => None,
        })
        .await;
    assert_eq!(
        winner,
        (DuelWinnerReason::Won, "Hunter".into(), "Bob".into())
    );
    on(&map, move |m| {
        let f = &mut m.players.get_mut(&gb).unwrap().fields;
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        f.set_u32(UNIT_FIELD_HEALTH, max);
    })
    .await;
    let flag = challenged(&map, gh, &mut b).await;
    begun(&map, gh, &mut b, flag).await;
    h.clear();
    b.clear();
    let alive = on(&map, move |m| {
        crate::game::map::combat::deal_damage(m, wolf, gb, 100_000);
        m.players[&gb].fields.get_u32(UNIT_FIELD_HEALTH) > 0
    })
    .await;
    assert!(!alive, "killed");
    let complete = b
        .wait_for("SMSG_DUEL_COMPLETE", |m| match m {
            ServerOpcodeMessage::SMSG_DUEL_COMPLETE(c) => Some(c.ended_without_interruption),
            _ => None,
        })
        .await;
    assert!(!complete, "interrupted");
    let won = h
        .gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_DUEL_WINNER(_)).then_some(())
        })
        .await;
    assert!(won.is_none(), "no winner");
    assert!(on(&map, move |m| m.players[&gh].duel.is_none()).await);
}

/// mangos's `DealDamage`: a blow that leaves one at 1 health ends the duel
/// (`damage >= health - 1`), as one that would fell them does.
#[tokio::test]
async fn a_blow_that_leaves_one_at_one_health_ends_the_duel() {
    let (_realm, mut h, gh, map, _pet, _wolf, mut b) = hunter_and_bob().await;
    let gb = b.guid;
    let flag = challenged(&map, gh, &mut b).await;
    begun(&map, gh, &mut b, flag).await;
    h.clear();
    let health = on(&map, move |m| {
        let health = m.players[&gb].fields.get_u32(UNIT_FIELD_HEALTH);
        crate::game::map::combat::deal_damage(m, gh, gb, health - 1);
        m.players[&gb].fields.get_u32(UNIT_FIELD_HEALTH)
    })
    .await;
    assert_eq!(health, 1);
    h.wait_for("SMSG_DUEL_WINNER", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_DUEL_WINNER(_)).then_some(())
    })
    .await;
}
