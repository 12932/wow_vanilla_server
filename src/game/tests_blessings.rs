//! The paladin's blessings, on the real data, as mangos-classic has them:
//! Blessing of Protection is a friend's (its pacify an aura at a raid
//! member, so the spell helps: `IsPositiveEffect` goes by the target), and
//! one blessing of each paladin's on a unit is its group 7, which leaves
//! Blessing of Freedom out. Ice Block, a stun at its caster, is a buff its
//! mage cancels.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, UNIT_FLAG_PACIFIED};
use crate::game::map::combat::Outcome;
use crate::game::map::spells::checks::fail;
use crate::game::map::spells::{Targets, class_scripts, wire_targets};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::forced_swing;

const PROTECTION: u32 = 10_278;
const FREEDOM: u32 = 1044;
const MIGHT: u32 = 25_291;
const MIGHT_1: u32 = 19_740;
const KINGS: u32 = 20_217;
const SACRIFICE: u32 = 20_729;
const ICE_BLOCK: u32 = 11_958;
const FROST_NOVA: u32 = 122;
const ARCANE_INTELLECT: u32 = 10_157;

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// `friend` beside `g`, three yards off.
fn beside(m: &mut Map, g: Guid, friend: Guid) {
    let p = m.players[&g].position();
    if let Some(f) = m.players.get_mut(&friend) {
        f.movement.position = Vector3d { x: p.x + 3.0, ..p };
    }
}

/// `c` casts `spell` at `target` through the protocol.
async fn cast_at(c: &mut Client, spell: u32, target: Guid) {
    c.send(CMSG_CAST_SPELL {
        spell,
        targets: wire_targets(&Targets {
            unit: Some(target),
            ..Default::default()
        }),
    })
    .await;
}

/// Wait for `check` to hold on the map, a short poll up to a limit.
async fn until(map: &MapHandle, what: &str, check: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
    loop {
        let c = check.clone();
        if on(map, move |m| c(m)).await {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Blessing of Protection, cast at a friend through the protocol: it lands
/// helpful (in a buff's slot), its friend pacified, immune to a blow, and
/// marked with Forbearance, which they can't cancel; cast at a hostile
/// kobold it is refused (mangos-classic casts it on friends, its pacify a
/// friend's aura: `IsPositiveEffect` by its raid target).
#[tokio::test]
#[ignore = "extended"]
async fn blessing_of_protection_shields_a_friend_and_is_refused_on_an_enemy() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for id in [1022, 5599, PROTECTION] {
        assert!(auras::positive(d.spells.get(id).unwrap()), "{id} helps");
    }
    assert!(!auras::positive(
        d.spells.get(class_scripts::FORBEARANCE).unwrap()
    ));
    let (mut c, pa, map) = player(
        &realm,
        "BLESSPROT1",
        "Protecta",
        Race::Human,
        Class::Paladin,
    )
    .await;
    let (f, fr, _) = player(
        &realm,
        "BLESSPROT2",
        "Protectb",
        Race::Human,
        Class::Warrior,
    )
    .await;
    let kobold = on(&map, move |m| {
        ready(m, pa, 60, &[PROTECTION]);
        ready(m, fr, 60, &[]);
        beside(m, pa, fr);
        dummy(m, pa, 60, 5.0)
    })
    .await;
    cast_at(&mut c, PROTECTION, fr).await;
    until(&map, "the blessing and Forbearance", move |m| {
        has(m, fr, PROTECTION) && has(m, fr, class_scripts::FORBEARANCE)
    })
    .await;
    let (positive, pacified, before, after) = on(&map, move |m| {
        let positive = holder(m, fr, PROTECTION).is_some_and(|h| h.positive);
        let pacified = m.players[&fr]
            .fields
            .get_u32(crate::game::fields::UNIT_FIELD_FLAGS)
            & UNIT_FLAG_PACIFIED
            != 0;
        let before = health(m, fr);
        forced_swing(m, kobold, fr, Outcome::Hit);
        (positive, pacified, before, health(m, fr))
    })
    .await;
    assert!(positive, "a buff");
    assert!(pacified, "pacified");
    assert_eq!(after, before, "immune to the blow");
    // Forbearance is a debuff: not cancelled.
    f.send(CMSG_CANCEL_AURA {
        id: class_scripts::FORBEARANCE,
    })
    .await;
    on(&map, |_| ()).await;
    on(&map, |_| ()).await;
    assert!(on(&map, move |m| has(m, fr, class_scripts::FORBEARANCE)).await);
    // At the kobold: refused, an enemy.
    on(&map, move |m| fresh(m, pa)).await;
    cast_at(&mut c, PROTECTION, kobold).await;
    let result = c
        .wait_frame("SMSG_CAST_RESULT", 0x130, |b| {
            b[..4] == PROTECTION.to_le_bytes() && b.get(4) == Some(&2)
        })
        .await;
    assert_eq!(&result[4..6], &[2, fail::TARGET_ENEMY], "an enemy");
    assert!(!on(&map, move |m| has(m, kobold, PROTECTION)).await);
}

/// Ice Block, a stun at its own caster, is a buff (mangos-classic's
/// `IsPositiveEffect`: an aura at the caster helps), and its mage cancels
/// it (`HandleCancelAuraOpcode` refuses only a harmful one); Frost Nova's
/// root from a kobold stays, cancelled before Arcane Intellect is.
#[tokio::test]
#[ignore = "extended"]
async fn a_mage_cancels_ice_block_but_not_a_frost_nova() {
    if !have_data() {
        return;
    }
    let realm = realm();
    assert!(auras::positive(realm.data.spells.get(ICE_BLOCK).unwrap()));
    let (c, g, map) = player(&realm, "BLESSICE1", "Iceblocka", Race::Human, Class::Mage).await;
    let rooted = on(&map, move |m| {
        ready(m, g, 60, &[ICE_BLOCK, ARCANE_INTELLECT]);
        let kobold = dummy(m, g, 60, 5.0);
        apply(m, kobold, g, FROST_NOVA);
        apply(m, g, g, ARCANE_INTELLECT);
        holder(m, g, FROST_NOVA).map(|h| h.positive)
    })
    .await;
    assert_eq!(rooted, Some(false), "a debuff");
    c.send(CMSG_CANCEL_AURA { id: FROST_NOVA }).await;
    c.send(CMSG_CANCEL_AURA {
        id: ARCANE_INTELLECT,
    })
    .await;
    until(&map, "Arcane Intellect cancelled", move |m| {
        !has(m, g, ARCANE_INTELLECT)
    })
    .await;
    let blocked = on(&map, move |m| {
        let rooted = has(m, g, FROST_NOVA);
        auras::remove_where(m, g, |h| h.spell == FROST_NOVA);
        assert!(cast_fresh(m, g, ICE_BLOCK, Targets::default()));
        (rooted, holder(m, g, ICE_BLOCK).map(|h| h.positive))
    })
    .await;
    assert_eq!(
        blocked,
        (true, Some(true)),
        "the root stays; Ice Block a buff"
    );
    c.send(CMSG_CANCEL_AURA { id: ICE_BLOCK }).await;
    until(&map, "Ice Block cancelled", move |m| !has(m, g, ICE_BLOCK)).await;
}

/// The blessings on `friend` and who cast them.
fn blessings_on(m: &Map, friend: Guid) -> Vec<(u32, Guid)> {
    let mut v: Vec<(u32, Guid)> = auras::auras(m, friend)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| [FREEDOM, MIGHT, MIGHT_1, KINGS, SACRIFICE].contains(&h.spell))
                .map(|h| (h.spell, h.caster))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(s, c)| (*s, c.guid()));
    v
}

/// One blessing of each paladin's on a unit, as mangos-classic's group 7
/// (`UNIQUE_PER_CASTER`) has them: a paladin's Kings takes the place of
/// its Might, and its Might of its Sacrifice; its Freedom, outside the
/// group, stands beside its Might, either first; two paladins' Might and
/// Kings stand together; the same Might from another paladin takes the
/// first's place, and a weaker rank of it is refused (`AddSpellAuraHolder`
/// keeping the stronger: `IsSimilarExistingAuraStronger`, a higher rank).
#[tokio::test]
#[ignore = "extended"]
async fn a_paladins_blessings_replace_one_another_but_freedom_and_another_paladins() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_a, pa, map) = player(&realm, "BLESSGRP1", "Blessera", Race::Human, Class::Paladin).await;
    let (_b, pb, _) = player(&realm, "BLESSGRP2", "Blesserb", Race::Human, Class::Paladin).await;
    let (_f, fr, _) = player(&realm, "BLESSGRP3", "Blessedc", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        let learn = [FREEDOM, MIGHT, MIGHT_1, KINGS, SACRIFICE];
        ready(m, pa, 60, &learn);
        ready(m, pb, 60, &learn);
        ready(m, fr, 60, &[]);
        beside(m, pa, fr);
        beside(m, pa, pb);
        let clear = |m: &mut Map| auras::remove_where(m, fr, |h| !h.passive);
        let cast = |m: &mut Map, by: Guid, spell: u32| {
            assert!(cast_fresh(m, by, spell, at(fr)), "{spell} began");
            spells_done(m);
        };
        let mut seen = Vec::new();
        // Might, then Freedom; Freedom, then Might.
        cast(m, pa, MIGHT);
        cast(m, pa, FREEDOM);
        seen.push(blessings_on(m, fr));
        clear(m);
        cast(m, pa, FREEDOM);
        cast(m, pa, MIGHT);
        seen.push(blessings_on(m, fr));
        clear(m);
        // Might, then Kings; Sacrifice, then Might.
        cast(m, pa, MIGHT);
        cast(m, pa, KINGS);
        seen.push(blessings_on(m, fr));
        clear(m);
        cast(m, pa, SACRIFICE);
        cast(m, pa, MIGHT);
        seen.push(blessings_on(m, fr));
        clear(m);
        // Two paladins' Might and Kings.
        cast(m, pa, MIGHT);
        cast(m, pb, KINGS);
        seen.push(blessings_on(m, fr));
        clear(m);
        // The same Might from two: the later; a weaker rank, refused.
        cast(m, pa, MIGHT);
        cast(m, pb, MIGHT);
        seen.push(blessings_on(m, fr));
        fresh(m, pa);
        let weaker = crate::game::map::spells::cast(m, pa, MIGHT_1, at(fr), None);
        spells_done(m);
        seen.push(blessings_on(m, fr));
        (seen, weaker)
    })
    .await;
    let (seen, weaker) = out;
    assert_eq!(
        seen[0],
        vec![(FREEDOM, pa), (MIGHT, pa)],
        "Might and Freedom"
    );
    assert_eq!(
        seen[1],
        vec![(FREEDOM, pa), (MIGHT, pa)],
        "Freedom and Might"
    );
    assert_eq!(seen[2], vec![(KINGS, pa)], "Kings for Might");
    assert_eq!(seen[3], vec![(MIGHT, pa)], "Might for Sacrifice");
    let mut two = vec![(MIGHT, pa), (KINGS, pb)];
    two.sort_by_key(|(s, c)| (*s, c.guid()));
    assert_eq!(seen[4], two, "two paladins' Might and Kings");
    assert_eq!(seen[5], vec![(MIGHT, pb)], "the later Might");
    assert!(!weaker, "a weaker Might refused");
    assert_eq!(seen[6], vec![(MIGHT, pb)], "the stronger Might stays");
}

/// Let the spells cast go off.
fn spells_done(m: &mut Map) {
    crate::game::map::spells::update(m);
}

const WISDOM: u32 = 25_290;
const GREATER_WISDOM: u32 = 25_918;
const GREATER_WISDOM_1: u32 = 25_894;
const SALVATION: u32 = 1038;
const GREATER_SALVATION: u32 = 25_895;
const SYMBOL_OF_KINGS: u32 = 21_177;

/// The blessings of Wisdom, Salvation and Kings on `g`, and who cast them.
fn healers_blessings(m: &Map, g: Guid) -> Vec<(u32, Guid)> {
    const KINDS: [u32; 6] = [
        WISDOM,
        GREATER_WISDOM,
        GREATER_WISDOM_1,
        SALVATION,
        GREATER_SALVATION,
        KINGS,
    ];
    let mut v: Vec<(u32, Guid)> = auras::auras(m, g)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| KINDS.contains(&h.spell))
                .map(|h| (h.spell, h.caster))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(s, c)| (*s, c.guid()));
    v
}

/// `leader` invites `other`, who accepts: in a party.
async fn party(leader: &mut Client, other: &mut Client, name: &str, members: usize) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    other
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    other.send(CMSG_GROUP_ACCEPT {}).await;
    leader
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if l.members.len() >= members => Some(()),
            _ => None,
        })
        .await;
}

/// The blessings of two paladins on two priests of their party, as
/// mangos-classic's group 7 (`UNIQUE_PER_CASTER`) and its stacker have
/// them: one blessing of each paladin's on a unit, so a paladin's new one
/// takes the place of its own other there (Kings for its Wisdom, Greater
/// Salvation for its Kings), and another paladin's of another kind stands
/// beside it (Wisdom and Salvation together); a Greater Blessing goes on
/// all of the target's class in the group; and a blessing and its Greater
/// kind are one spell (one family and family flags: the stacker's
/// "multirank", unstackable for helpful spells), whoever cast them, so the
/// other paladin's as strong or stronger takes its place (a Wisdom for a
/// Greater Wisdom of the same 33 mana, a Salvation for a Greater
/// Salvation) and a weaker is refused (`IsSimilarExistingAuraStronger`: a
/// Greater Wisdom of 30 under a Wisdom of 33).
#[tokio::test]
#[ignore = "extended"]
async fn two_paladins_blessings_on_healers_stand_together_one_of_a_kind() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut a, pa, map) = player(
        &realm,
        "BLESSHEAL1",
        "Wisblessa",
        Race::Human,
        Class::Paladin,
    )
    .await;
    let (mut b, pb, _) = player(
        &realm,
        "BLESSHEAL2",
        "Wisblessb",
        Race::Human,
        Class::Paladin,
    )
    .await;
    let (mut c, h1, _) = player(&realm, "BLESSHEAL3", "Wishealc", Race::Human, Class::Priest).await;
    let (mut d, h2, _) = player(&realm, "BLESSHEAL4", "Wisheald", Race::Human, Class::Priest).await;
    party(&mut a, &mut b, "Wisblessb", 1).await;
    party(&mut a, &mut c, "Wishealc", 2).await;
    party(&mut a, &mut d, "Wisheald", 3).await;
    let seen = on(&map, move |m| {
        let learn = [
            WISDOM,
            GREATER_WISDOM,
            GREATER_WISDOM_1,
            SALVATION,
            GREATER_SALVATION,
            KINGS,
        ];
        for p in [pa, pb] {
            ready(m, p, 60, &learn);
            crate::game::map::inventory::add_item(m, p, SYMBOL_OF_KINGS, 20, false)
                .expect("the symbols in its bags");
        }
        ready(m, h1, 60, &[]);
        ready(m, h2, 60, &[]);
        beside(m, pa, pb);
        beside(m, pa, h1);
        beside(m, pa, h2);
        let cast = |m: &mut Map, by: Guid, spell: u32, to: Guid| {
            assert!(cast_fresh(m, by, spell, at(to)), "{spell} began");
            spells_done(m);
        };
        let both = |m: &Map| (healers_blessings(m, h1), healers_blessings(m, h2));
        let mut seen = Vec::new();
        // A's Wisdom, B's Salvation: together.
        cast(m, pa, WISDOM, h1);
        cast(m, pb, SALVATION, h1);
        seen.push(both(m));
        // A's Kings takes its own Wisdom's place; B's Salvation stays.
        cast(m, pa, KINGS, h1);
        seen.push(both(m));
        // B's Greater Wisdom on the priests takes its own Salvation's.
        cast(m, pb, GREATER_WISDOM, h1);
        seen.push(both(m));
        // A's Wisdom on the second priest, as strong as B's Greater: A's.
        cast(m, pa, WISDOM, h2);
        seen.push(both(m));
        // B's weaker Greater Wisdom: refused on both (its own stronger on
        // the first, A's on the second).
        fresh(m, pb);
        crate::game::map::spells::cast(m, pb, GREATER_WISDOM_1, at(h1), None);
        spells_done(m);
        seen.push(both(m));
        // A's Greater Salvation takes its own Kings' and Wisdom's places.
        cast(m, pa, GREATER_SALVATION, h1);
        seen.push(both(m));
        // B's Salvation, as strong, takes A's Greater Salvation's place
        // on the second priest.
        cast(m, pb, SALVATION, h2);
        seen.push(both(m));
        seen
    })
    .await;
    let sorted = |mut v: Vec<(u32, Guid)>| {
        v.sort_by_key(|(s, c)| (*s, c.guid()));
        v
    };
    assert_eq!(
        seen[0],
        (sorted(vec![(WISDOM, pa), (SALVATION, pb)]), vec![]),
        "two paladins' Wisdom and Salvation"
    );
    assert_eq!(
        seen[1],
        (sorted(vec![(KINGS, pa), (SALVATION, pb)]), vec![]),
        "A's Kings for its Wisdom"
    );
    assert_eq!(
        seen[2],
        (
            sorted(vec![(KINGS, pa), (GREATER_WISDOM, pb)]),
            vec![(GREATER_WISDOM, pb)]
        ),
        "B's Greater Wisdom for its Salvation, on both priests"
    );
    assert_eq!(
        seen[3],
        (
            sorted(vec![(KINGS, pa), (GREATER_WISDOM, pb)]),
            vec![(WISDOM, pa)]
        ),
        "A's Wisdom for B's Greater Wisdom of as much"
    );
    assert_eq!(seen[4], seen[3], "a weaker Greater Wisdom refused");
    assert_eq!(
        seen[5],
        (
            sorted(vec![(GREATER_SALVATION, pa), (GREATER_WISDOM, pb)]),
            vec![(GREATER_SALVATION, pa)]
        ),
        "A's Greater Salvation for its Kings and its Wisdom"
    );
    assert_eq!(
        seen[6],
        (
            sorted(vec![(GREATER_SALVATION, pa), (GREATER_WISDOM, pb)]),
            vec![(SALVATION, pb)]
        ),
        "B's Salvation for A's Greater Salvation"
    );
}
