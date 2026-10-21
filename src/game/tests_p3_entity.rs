//! Phase 3's world-entity tests, ported from Thistle Tea's
//! `test/game/world/entity` (docs/parity/matrix.csv): what an entity's
//! events tell whom. A fight interrupting a peaceful cast, a controlled
//! creature's cooldowns on its master's bar, the combat log reaching the
//! one it is about and those near, a creature's whisper, yell and boss
//! emote, and the hearthstone.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, *};
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};

/// How long to listen for what shouldn't come.
pub(crate) const NONE_WITHIN: Duration = Duration::from_millis(300);

pub(crate) const SMSG_CAST_RESULT: u16 = 0x130;
pub(crate) const SMSG_SPELL_FAILURE: u16 = 0x133;
pub(crate) const SMSG_SPELL_FAILED_OTHER: u16 = 0x2A6;
pub(crate) const SMSG_SPELL_COOLDOWN: u16 = 0x134;
pub(crate) const SMSG_ENVIRONMENTAL_DAMAGE_LOG: u16 = 0x1FC;
pub(crate) const SMSG_SPELLHEALLOG: u16 = 0x150;
pub(crate) const SMSG_PERIODICAURALOG: u16 = 0x24E;
pub(crate) const SMSG_SPELLNONMELEEDAMAGELOG: u16 = 0x250;
pub(crate) const SMSG_SPELLDISPELLOG: u16 = 0x27B;
pub(crate) const SMSG_DISPEL_FAILED: u16 = 0x262;
pub(crate) const SMSG_SPELLORDAMAGE_IMMUNE: u16 = 0x263;

/// A peaceful cast (a mount's: not in a fight), three seconds long.
const PEACEFUL: u32 = 96_000;
/// A ten-second cooldown, instant, on oneself.
const COOLING: u32 = 96_001;
/// Five hundred shadow, instant, at a unit.
const SHADOW_BOLT: u32 = 96_002;
/// A shadow ward of 454, on oneself.
const SHADOW_WARD: u32 = 96_003;
/// Fire, ticking every tenth of a second for ten seconds.
const BURN: u32 = 96_004;
/// Fire school immunity, on oneself.
const FIRE_PROOF: u32 = 96_005;
/// Home, as the hearthstone takes one.
const HEARTHSTONE: u32 = 96_006;

/// A spell of `effects`, instant, at no cost.
pub(crate) fn spell(id: u32, effects: &[SpellEffect]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    for (i, e) in effects.iter().enumerate() {
        s.effects[i] = *e;
    }
    s
}

/// An aura effect of `kind` holding `amount` (of `misc`), at `target_a`.
pub(crate) fn aura_effect(kind: u32, amount: i32, misc: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect: 6,
        aura: kind,
        base_points: amount - 1,
        base_dice: 1,
        misc_value: misc,
        target_a,
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = data();
    d.spells.cast_times.insert(14, 3000);
    d.spells.durations.insert(
        9,
        SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    let mut peaceful = spell(
        PEACEFUL,
        &[SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        }],
    );
    peaceful.casting_time_index = 14;
    peaceful.attributes[0] = crate::game::data::spells::ATTR_NOT_IN_COMBAT;
    let mut cooling = spell(
        COOLING,
        &[SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        }],
    );
    cooling.recovery_time = 10_000;
    let mut bolt = spell(
        SHADOW_BOLT,
        &[SpellEffect {
            effect: 2,
            base_points: 499,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        }],
    );
    bolt.school = 5;
    let ward = spell(
        SHADOW_WARD,
        &[aura_effect(AURA_SCHOOL_ABSORB, 454, 1 << 5, 1)],
    );
    let mut burn = spell(
        BURN,
        &[SpellEffect {
            amplitude: 100,
            ..aura_effect(AURA_PERIODIC_DAMAGE, 10, 0, 6)
        }],
    );
    burn.school = 2;
    burn.duration_index = 9;
    let proof = spell(
        FIRE_PROOF,
        &[aura_effect(AURA_SCHOOL_IMMUNITY, 1, 1 << 2, 1)],
    );
    let hearthstone = spell(
        HEARTHSTONE,
        &[SpellEffect {
            effect: 5,
            target_a: 1,
            ..Default::default()
        }],
    );
    for s in [peaceful, cooling, bolt, ward, burn, proof, hearthstone] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

pub(crate) async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// A second player, Bob, beside the first and seeing them.
pub(crate) async fn bob(realm: &std::sync::Arc<crate::game::Realm>, alice: Guid) -> (Client, Guid) {
    let mut b = Client::connect(realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 1.0, START.1, START.2).await;
    b.sees(alice).await;
    (b, gb)
}

/// Whether `c` gets a frame of `opcode` that `pick` takes within
/// [`NONE_WITHIN`].
pub(crate) async fn gets_frame(c: &mut Client, opcode: u16, pick: impl Fn(&[u8]) -> bool) -> bool {
    tokio::time::timeout(NONE_WITHIN, c.wait_frame("", opcode, pick))
        .await
        .is_ok()
}

fn spell_of(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(body[at..at + 4].try_into().unwrap())
}

/// Whether `c` hears a chat line `line` within [`NONE_WITHIN`].
pub(crate) async fn heard(c: &mut Client, line: &str) -> bool {
    c.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(x) if x.message == line => Some(()),
        _ => None,
    })
    .await
    .is_some()
}

/// The length of a packed GUID at the front of `body`.
pub(crate) fn packed_len(body: &[u8]) -> usize {
    1 + body[0].count_ones() as usize
}

/// Thistle Tea's "combat interruption sends owner failure and same-world
/// observer cancellation": a fight beginning stops a cast that isn't for
/// one (mangos's `SetInCombatState`), and the caster's client hears it
/// interrupted, its cast result too, and those near hear the cast fail.
#[tokio::test]
async fn a_fight_interrupts_a_peaceful_cast_and_tells_the_caster_and_those_near() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (mut b, _gb) = bob(&realm, ga).await;
    let brute = on(&map, move |m| {
        spells::learn(m, ga, PEACEFUL);
        let brute = spawn(m, ga, BRUTE, 20, 4.0);
        spells::cast(m, ga, PEACEFUL, Targets::default(), None);
        brute
    })
    .await;
    a.clear();
    b.clear();
    let casting = on(&map, move |m| {
        crate::game::map::combat::engage(m, brute, ga);
        m.caster(ga)
            .and_then(|c| c.casting.as_ref())
            .map(|c| c.spell)
    })
    .await;
    assert_eq!(casting, None, "the fight stops the cast");
    let result = a
        .wait_frame("SMSG_CAST_RESULT", SMSG_CAST_RESULT, |f| {
            spell_of(f, 0) == PEACEFUL
        })
        .await;
    assert_eq!(&result[4..], &[2, 0x23], "interrupted");
    let failure = a
        .wait_frame("SMSG_SPELL_FAILURE", SMSG_SPELL_FAILURE, |_| true)
        .await;
    assert_eq!(
        &failure[failure.len() - 5..failure.len() - 1],
        &PEACEFUL.to_le_bytes()
    );
    let other = b
        .wait_frame("SMSG_SPELL_FAILED_OTHER", SMSG_SPELL_FAILED_OTHER, |_| true)
        .await;
    assert_eq!(other[..8], ga.guid().to_le_bytes(), "Alice's");
    assert_eq!(spell_of(&other, 8), PEACEFUL);
    assert!(
        !gets_frame(&mut b, SMSG_CAST_RESULT, |_| true).await,
        "the cast result is the caster's alone"
    );
}

/// Thistle Tea's "a charmed creature's cooldowns reach its charming
/// player": a creature a player controls tells its master's client of its
/// cooldowns, under its own GUID, for its bar (mangos's
/// `Creature::AddCooldown`); let go, it tells no one.
#[tokio::test]
async fn a_charmed_creatures_cooldowns_reach_its_master() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warlock).await;
    a.clear();
    // Charmed and casting in one go: a charm with no aura holding it
    // doesn't outlast the map's next look at it.
    let brute = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 4.0);
        crate::game::map::pets::charmed(m, ga, brute, false);
        spells::cast(m, brute, COOLING, Targets::default(), None);
        brute
    })
    .await;
    let cooldown = a
        .wait_for("the creature's cooldown", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(c) => Some(c.clone()),
            _ => None,
        })
        .await;
    assert_eq!(cooldown.guid, brute, "under the creature's GUID");
    assert_eq!(cooldown.cooldowns.len(), 1);
    assert_eq!(cooldown.cooldowns[0].id, COOLING);
    assert_eq!(cooldown.cooldowns[0].cooldown_time, Duration::from_secs(10));
    // Free again, its cooldowns are its own.
    a.clear();
    on(&map, move |m| {
        crate::game::map::pets::uncharmed(m, ga, brute);
        if let Some(c) = m.caster_mut(brute) {
            c.cooldowns.clear();
        }
        spells::cast(m, brute, COOLING, Targets::default(), None);
    })
    .await;
    assert!(
        !gets_frame(&mut a, SMSG_SPELL_COOLDOWN, |_| true).await,
        "no one's bar once it is free"
    );
}

/// Thistle Tea's environmental damage, direct healing, spell damage,
/// dispel and damage immunity feedback: each reaches the one it is about
/// and those near alike. The world's harm is logged at what is left, a
/// heal's crit with it and a tick's in the periodic log instead, spell
/// damage net of what a ward soaked with the soak beside it, a dispel's
/// work and its failures, and a tick shrugged off.
#[tokio::test]
async fn the_combat_log_reaches_the_one_it_is_about_and_those_near() {
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (mut b, gb) = bob(&realm, ga).await;
    let brute = on(&map, move |m| {
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
        f.set_u32(UNIT_FIELD_HEALTH, 5_000);
        spawn(m, ga, BRUTE, 20, 4.0)
    })
    .await;
    b.sees(brute).await;
    a.clear();
    b.clear();
    // Exhaustion's twenty, and none at all: both are told.
    on(&map, move |m| {
        for amount in [20, 0] {
            crate::game::map::environment::damage(
                m,
                ga,
                EnvironmentalDamageType::Exhausted,
                amount,
            );
        }
    })
    .await;
    for c in [&mut a, &mut b] {
        for amount in [20u32, 0] {
            let log = c
                .wait_frame(
                    "SMSG_ENVIRONMENTAL_DAMAGE_LOG",
                    SMSG_ENVIRONMENTAL_DAMAGE_LOG,
                    |_| true,
                )
                .await;
            assert_eq!(log[..8], ga.guid().to_le_bytes());
            assert_eq!(log[8], EnvironmentalDamageType::Exhausted.as_int());
            assert_eq!(spell_of(&log, 9), amount, "{}", c.account);
        }
    }
    // Bob's heal on Alice, a crit; then its tick, logged as periodic only.
    on(&map, move |m| {
        spells::heal_crit(m, gb, ga, 2050, 25, false, true);
        spells::heal_crit(m, gb, ga, 139, 25, true, false);
    })
    .await;
    for c in [&mut a, &mut b] {
        let heal = c
            .wait_frame("SMSG_SPELLHEALLOG", SMSG_SPELLHEALLOG, |_| true)
            .await;
        let at = packed_len(&heal);
        let at = at + packed_len(&heal[at..]);
        assert_eq!(spell_of(&heal, at), 2050);
        assert_eq!(spell_of(&heal, at + 4), 25);
        assert_eq!(heal[at + 8], 1, "a crit");
        let tick = c
            .wait_frame("SMSG_PERIODICAURALOG", SMSG_PERIODICAURALOG, |_| true)
            .await;
        assert!(tick.windows(4).any(|w| w == 139u32.to_le_bytes()));
        assert!(
            !gets_frame(c, SMSG_SPELLHEALLOG, |_| true).await,
            "{}: no heal log for the tick",
            c.account
        );
    }
    // The brute's shadow bolt on Alice behind her ward: 500 less 454.
    on(&map, move |m| {
        let ward = m.realm.data.spells.get(SHADOW_WARD).cloned().unwrap();
        auras::apply(m, ga, ga, &ward, 0, 454);
        spells::cast(
            m,
            brute,
            SHADOW_BOLT,
            Targets {
                unit: Some(ga),
                ..Default::default()
            },
            None,
        );
    })
    .await;
    let mut seen = Vec::new();
    for c in [&mut a, &mut b] {
        let log = c
            .wait_frame(
                "SMSG_SPELLNONMELEEDAMAGELOG",
                SMSG_SPELLNONMELEEDAMAGELOG,
                |_| true,
            )
            .await;
        let at = packed_len(&log);
        let at = at + packed_len(&log[at..]);
        assert_eq!(spell_of(&log, at), SHADOW_BOLT);
        let (damage, absorbed) = (spell_of(&log, at + 4), spell_of(&log, at + 9));
        assert_eq!((damage, absorbed), (46, 454), "{}", c.account);
        assert_eq!(log[at + 17], 0, "not a periodic log");
        seen.push(log);
    }
    assert_eq!(seen[0], seen[1], "the same log to both");
    // A dispel's work and its failure.
    on(&map, move |m| {
        spells::log::dispelled(m, gb, ga, vec![123, 456]);
        spells::log::dispel_failed(m, gb, ga, vec![789, 789]);
    })
    .await;
    for c in [&mut a, &mut b] {
        let dispel = c
            .wait_frame("SMSG_SPELLDISPELLOG", SMSG_SPELLDISPELLOG, |_| true)
            .await;
        assert!(dispel.windows(4).any(|w| w == 456u32.to_le_bytes()));
        let failed = c
            .wait_frame("SMSG_DISPEL_FAILED", SMSG_DISPEL_FAILED, |_| true)
            .await;
        assert_eq!(failed[..8], gb.guid().to_le_bytes(), "Bob's");
        assert_eq!(failed[8..16], ga.guid().to_le_bytes(), "on Alice");
    }
    // The brute's burn ticking on Alice, proof against fire since.
    on(&map, move |m| {
        let burn = m.realm.data.spells.get(BURN).cloned().unwrap();
        auras::apply(m, brute, ga, &burn, 0, 10);
        let proof = m.realm.data.spells.get(FIRE_PROOF).cloned().unwrap();
        auras::apply(m, ga, ga, &proof, 0, 1);
    })
    .await;
    for c in [&mut a, &mut b] {
        let immune = c
            .wait_frame(
                "SMSG_SPELLORDAMAGE_IMMUNE",
                SMSG_SPELLORDAMAGE_IMMUNE,
                |_| true,
            )
            .await;
        assert_eq!(immune[..8], brute.guid().to_le_bytes(), "the brute's");
        assert_eq!(immune[8..16], ga.guid().to_le_bytes(), "on Alice");
        assert_eq!(spell_of(&immune, 16), BURN);
    }
}

/// Thistle Tea's "monster whispers reach only their target while boss
/// emotes carry to nearby players" and "monster yells reach players in
/// range who cannot see the speaker yet": a whisper (and a boss whisper)
/// is for its target alone, and for no one without one; a boss emote and
/// a yell carry 300 yards, to those too far to see the speaker.
#[tokio::test]
async fn a_creatures_whisper_is_its_targets_and_its_yell_carries_past_sight() {
    use crate::game::map::scripts::{
        self, CHAT_BOSS_EMOTE, CHAT_BOSS_WHISPER, CHAT_WHISPER, CHAT_YELL,
    };
    let (realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let (mut b, gb) = bob(&realm, ga).await;
    let brute = on(&map, move |m| spawn(m, ga, BRUTE, 20, 1.0)).await;
    let place = move |alice: f32, bob: f32| {
        move |m: &mut Map| {
            let at = m.position_of(brute).unwrap();
            for (g, off) in [(ga, alice), (gb, bob)] {
                let p = m.players.get_mut(&g).unwrap();
                p.movement.position = Vector3d {
                    x: at.x + off,
                    ..at
                };
            }
        }
    };
    // Alice beside it, Bob a hundred yards off.
    on(&map, place(1.0, 100.0)).await;
    a.clear();
    b.clear();
    on(&map, move |m| {
        scripts::say(m, brute, "Come, $n.", CHAT_WHISPER, 0, ga);
        scripts::say(m, brute, "No one.", CHAT_WHISPER, 0, Guid::zero());
        scripts::say(
            m,
            brute,
            "The Nightmare stirs.",
            CHAT_BOSS_EMOTE,
            0,
            Guid::zero(),
        );
    })
    .await;
    assert!(heard(&mut a, "Come, $n.").await, "the whisper is Alice's");
    assert!(!heard(&mut b, "Come, $n.").await, "and not Bob's");
    assert!(!heard(&mut a, "No one.").await, "a whisper to no one");
    assert!(
        heard(&mut a, "The Nightmare stirs.").await,
        "a boss emote near"
    );
    assert!(
        heard(&mut b, "The Nightmare stirs.").await,
        "and a hundred yards off"
    );
    on(&map, move |m| {
        scripts::say(m, brute, "Come closer.", CHAT_BOSS_WHISPER, 0, ga);
    })
    .await;
    assert!(
        a.wait_frame("the boss whisper", 0x096, |f| f.first() == Some(&0x59))
            .await
            .windows(12)
            .any(|w| w == b"Come closer."),
        "the boss whisper is Alice's"
    );
    assert!(
        !gets_frame(&mut b, 0x096, |f| f.first() == Some(&0x59)).await,
        "and not Bob's"
    );
    // A yell from one 200 yards off, out of Alice's sight: she hears it;
    // Bob, 320 yards from it, doesn't.
    let far = on(&map, move |m| spawn(m, ga, BRUTE, 20, 200.0)).await;
    on(&map, move |m| {
        let at = m.position_of(far).unwrap();
        m.players.get_mut(&gb).unwrap().movement.position = Vector3d {
            x: at.x + 320.0,
            ..at
        };
    })
    .await;
    a.clear();
    b.clear();
    let seen = on(&map, move |m| {
        let seen = m.players[&ga].known.contains(&far);
        scripts::say(
            m,
            far,
            "Elune, hear my prayers.",
            CHAT_YELL,
            0,
            Guid::zero(),
        );
        seen
    })
    .await;
    assert!(!seen, "Alice doesn't see the speaker");
    assert!(
        heard(&mut a, "Elune, hear my prayers.").await,
        "Alice hears the yell"
    );
    assert!(
        !heard(&mut b, "Elune, hear my prayers.").await,
        "Bob is too far"
    );
}

/// Thistle Tea's "hearthstone teleports a character to their home bind":
/// a teleport with no place of its own takes the caster to where they are
/// bound, within the map and to another.
#[tokio::test]
async fn the_hearthstone_takes_a_player_to_their_bind() {
    use crate::game::store::Bind;
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| {
        spells::learn(m, ga, HEARTHSTONE);
        m.players.get_mut(&ga).unwrap().row.state.bind = Some(Bind {
            map: 0,
            zone: 12,
            x: -9_464.0,
            y: 62.0,
            z: 56.0,
        });
        spells::cast(m, ga, HEARTHSTONE, Targets::default(), None);
    })
    .await;
    let to = a
        .wait_for("MSG_MOVE_TELEPORT_ACK", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info.position),
            _ => None,
        })
        .await;
    assert_eq!(
        (to.x, to.y, to.z),
        (-9_464.0, 62.0, 56.0),
        "home in Goldshire"
    );
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().row.state.bind = Some(Bind {
            map: 1,
            zone: 1637,
            x: 1_629.0,
            y: -4_373.0,
            z: 31.0,
        });
        if let Some(c) = m.caster_mut(ga) {
            c.gcd_until = None;
        }
        spells::cast(m, ga, HEARTHSTONE, Targets::default(), None);
    })
    .await;
    let world = a
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some((w.map, w.position)),
            _ => None,
        })
        .await;
    assert_eq!(world.0, wow_world_messages::vanilla::Map::Kalimdor);
    assert_eq!(
        (world.1.x, world.1.y),
        (1_629.0, -4_373.0),
        "home in Orgrimmar"
    );
}

/// A forced reaction of friendly to the brutes' faction.
const BEFRIEND: u32 = 96_007;

/// Thistle Tea's "forced reaction changes update the client and request
/// friendly attack cancellation": a faction forced friendly is told the
/// client, and the player stops fighting its units, and they the player
/// (mangos's `HandleForceReaction` and `StopAttackFaction`).
#[tokio::test]
async fn a_faction_forced_friendly_stops_the_fight_with_it() {
    use crate::game::tests_p3_combat::PACK;
    let mut d = world();
    let befriend = spell(
        BEFRIEND,
        &[aura_effect(AURA_FORCE_REACTION, 4, PACK as i32, 1)],
    );
    d.spells.spells.insert(BEFRIEND, befriend);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let brute = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 2.0);
        crate::game::map::combat::start_attack(m, ga, brute);
        crate::game::map::ai::attacked(m, brute, ga);
        // The brute's first blow kills her, if an update comes before the
        // next query (on a busy machine): she takes none.
        m.players.get_mut(&ga).unwrap().misc.god = true;
        brute
    })
    .await;
    a.clear();
    let (attacking, threat, victim) = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, BEFRIEND, Some(ga));
        let c = m.creature(brute).unwrap();
        (
            m.players[&ga].combat.attacking,
            c.threat.iter().any(|(t, _)| *t == ga),
            c.combat.victim == ga && c.combat.attacking,
        )
    })
    .await;
    let forced = a
        .wait_frame("SMSG_SET_FORCED_REACTIONS", 0x2A5, |f| {
            f[..4] == 1u32.to_le_bytes()
        })
        .await;
    assert_eq!(
        forced[4..12],
        [&PACK.to_le_bytes()[..], &4u32.to_le_bytes()].concat()
    );
    assert!(!attacking, "Alice stops attacking the brute");
    assert!(!threat, "the brute forgets her");
    assert!(!victim, "and stops fighting her");
    a.wait_for("SMSG_ATTACKSTOP", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTOP(s) if s.player == ga => Some(()),
        _ => None,
    })
    .await;
}

const CAST_QUEST: u32 = 96_100;

/// Put `quest` in `g`'s log.
pub(crate) fn take_quest(m: &mut Map, g: Guid, quest: u32) {
    let d = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.journal.log.push(crate::game::store::Logged {
        quest,
        ..Default::default()
    });
    crate::game::map::quests::refresh_log(p, &d);
}

/// A script step of `command` and `datalong`.
pub(crate) fn step(command: u32, datalong: [u32; 4]) -> crate::game::data::scripts::ScriptStep {
    crate::game::data::scripts::ScriptStep {
        command,
        datalong,
        ..Default::default()
    }
}

/// Thistle Tea's "scripted cast credit reaches its player with the
/// creature's reported entry", "delivers scripted casts only through the
/// explicit player owner context" and "script attack_start schedules a
/// forced attack": a script's cast credit counts for the player's quest
/// asking for that spell on the creature, by the entry it is now; a
/// script's cast from a player is the player's own; and a script's attack
/// start sets the creature on its target.
#[tokio::test]
async fn a_scripts_credit_cast_and_attack_go_to_whom_they_name() {
    use crate::game::map::scripts::{self, C_CAST_CREDIT};
    let mut d = world();
    d.npc.quests.insert(
        CAST_QUEST,
        crate::game::data::npc::Quest {
            id: CAST_QUEST,
            required_kills: [(BRUTE as i32, 2), (0, 0), (0, 0), (0, 0)],
            required_spell_casts: [COOLING, 0, 0, 0],
            ..Default::default()
        },
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let brute = on(&map, move |m| {
        take_quest(m, ga, CAST_QUEST);
        spawn(m, ga, BRUTE, 20, 4.0)
    })
    .await;
    a.clear();
    on(&map, move |m| {
        // Another spell credits nothing; the quest's credits its creature.
        scripts::execute(m, &step(C_CAST_CREDIT, [SHADOW_BOLT, 0, 0, 0]), brute, ga);
        scripts::execute(m, &step(C_CAST_CREDIT, [COOLING, 0, 0, 0]), brute, ga);
    })
    .await;
    let credit = a
        .wait_for("SMSG_QUESTUPDATE_ADD_KILL", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) => {
                Some((k.quest_id, k.creature_id, k.kill_count, k.guid))
            }
            _ => None,
        })
        .await;
    assert_eq!(credit, (CAST_QUEST, BRUTE, 1, brute), "once, for the brute");
    // Made another entry, it credits as that one: nothing here.
    on(&map, move |m| {
        m.creature_mut(brute).unwrap().entry = BRUTE + 1;
        scripts::execute(m, &step(C_CAST_CREDIT, [COOLING, 0, 0, 0]), brute, ga);
        m.creature_mut(brute).unwrap().entry = BRUTE;
    })
    .await;
    assert!(
        a.gets(NONE_WITHIN, |m| {
            matches!(m, ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(_)).then_some(())
        })
        .await
        .is_none(),
        "as another entry, no credit"
    );
    // A cast step from Alice is Alice's, at the brute.
    a.clear();
    on(&map, move |m| {
        scripts::execute(m, &step(15, [SHADOW_BOLT, 0, 0, 0]), ga, brute);
    })
    .await;
    let go = a
        .wait_for("SMSG_SPELL_GO", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == SHADOW_BOLT => Some(g.caster),
            _ => None,
        })
        .await;
    assert_eq!(go, ga, "Alice's own cast");
    // The script's attack start: the brute goes for Alice.
    let (victim, threat) = on(&map, move |m| {
        let other = spawn(m, ga, BRUTE, 20, 6.0);
        scripts::execute(m, &step(26, [0; 4]), other, ga);
        let c = m.creature(other).unwrap();
        (c.combat.victim, c.threat.iter().any(|(t, _)| *t == ga))
    })
    .await;
    assert_eq!(victim, ga, "the brute attacks its target");
    assert!(threat);
}

/// Thistle Tea's "spell miss outcomes are delivered to the victim owner":
/// a harmful spell the victim shrugs off still reaches it: the miss is
/// logged and it turns on its caster.
#[tokio::test]
async fn a_spell_shrugged_off_still_reaches_its_victim() {
    const SHADOW_PROOF: u32 = 96_008;
    let mut d = world();
    d.spells.spells.insert(
        SHADOW_PROOF,
        spell(
            SHADOW_PROOF,
            &[aura_effect(AURA_SCHOOL_IMMUNITY, 1, 1 << 5, 1)],
        ),
    );
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let brute = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 4.0);
        let proof = m.realm.data.spells.get(SHADOW_PROOF).cloned().unwrap();
        auras::apply(m, brute, brute, &proof, 0, 1);
        brute
    })
    .await;
    a.clear();
    let (health, fights) = on(&map, move |m| {
        spells::cast(
            m,
            ga,
            SHADOW_BOLT,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
            None,
        );
        let c = m.creature(brute).unwrap();
        (
            c.fields.get_u32(UNIT_FIELD_HEALTH),
            c.combat.victim == ga && c.threat.iter().any(|(t, _)| *t == ga),
        )
    })
    .await;
    let (missed, why) = a.go_miss(SHADOW_BOLT).await;
    assert_eq!((missed, why), (brute, 7), "immune, in the go");
    assert_eq!(health, 100_000, "unhurt");
    assert!(fights, "the brute turns on Alice all the same");
}
