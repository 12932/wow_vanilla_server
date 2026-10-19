//! Round 8's combat tests, a seated player struck (Thistle Tea's
//! `combat_posture_test.exs`): a blow on one seated crits and stands her,
//! her defensive procs and her attacker's both going off (the 1.12 "sit
//! trick"); a standing crit spends a charge and starts a cooldown; an
//! absorbed one keeps its absorb procs; the immune and god mode stay
//! seated; an ability crits a seated player, a spell of magic or one that
//! can't crit doesn't.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_EX2_CANT_CRIT, ProcEvent, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::harness::Client;
use crate::game::map::auras::{
    self, AURA_DAMAGE_IMMUNITY, AURA_PROC_TRIGGER_SPELL, AURA_SCHOOL_ABSORB,
};
use crate::game::map::combat::{self, HITINFO_CRITICALHIT, Outcome};
use crate::game::map::procs::{
    EX_ABSORB, EX_CRITICAL_HIT, PROC_DONE_MELEE, PROC_TAKEN_MELEE, PROC_TAKEN_MELEE_SPELL,
    PROC_TAKEN_RANGED, PROC_TAKEN_RANGED_SPELL,
};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{self as p3, BRUTE};

/// A crit taken procs DEFENDED (its mark on the player), three charges and
/// three seconds' cooldown; one absorbed procs SOAKED; a crit dealt procs
/// FIERCE (on the attacker).
const DEFENSE: u32 = 96_500;
const DEFENDED: u32 = 96_501;
const ABSORBING: u32 = 96_502;
const SOAKED: u32 = 96_503;
const FIERCE_PROC: u32 = 96_504;
const FIERCE: u32 = 96_505;
/// Ten damage: a melee ability, a ranged one, one of magic, and melee and
/// ranged ones that can't crit.
const MELEE: u32 = 96_510;
const RANGED: u32 = 96_511;
const MAGIC: u32 = 96_512;
const MELEE_NO_CRIT: u32 = 96_513;
const RANGED_NO_CRIT: u32 = 96_514;
const LONG: u32 = 962;

fn proc_holder(id: u32, flags: u32, trigger: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Proc {id}"),
        equipped_item_class: -1,
        proc_flags: flags,
        proc_chance: 100,
        proc_charges: 3,
        duration_index: LONG,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_PROC_TRIGGER_SPELL,
        trigger_spell: trigger,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn mark(id: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Mark {id}"),
        equipped_item_class: -1,
        duration_index: LONG,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_DUMMY,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn ability(id: u32, class: u32, cant_crit: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Ability {id}"),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: class,
        school: if class == 1 { 2 } else { 0 },
        ..Default::default()
    };
    if cant_crit {
        s.attributes[2] |= ATTR_EX2_CANT_CRIT;
    }
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = p3::data();
    d.spells.durations.insert(
        LONG,
        crate::game::data::spells::SpellDuration {
            base: 600_000,
            per_level: 0,
            max: 600_000,
        },
    );
    let taken =
        PROC_TAKEN_MELEE | PROC_TAKEN_MELEE_SPELL | PROC_TAKEN_RANGED | PROC_TAKEN_RANGED_SPELL;
    for s in [
        proc_holder(DEFENSE, taken, DEFENDED),
        proc_holder(ABSORBING, PROC_TAKEN_MELEE, SOAKED),
        proc_holder(FIERCE_PROC, PROC_DONE_MELEE, FIERCE),
        mark(DEFENDED),
        mark(SOAKED),
        mark(FIERCE),
        ability(MELEE, 2, false),
        ability(RANGED, 3, false),
        ability(MAGIC, 1, false),
        ability(MELEE_NO_CRIT, 2, true),
        ability(RANGED_NO_CRIT, 3, true),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    for (id, ex) in [
        (DEFENSE, EX_CRITICAL_HIT),
        (ABSORBING, EX_ABSORB),
        (FIERCE_PROC, EX_CRITICAL_HIT),
    ] {
        d.spells.proc_events.insert(
            id,
            ProcEvent {
                ex,
                cooldown: 3_000,
                ..Default::default()
            },
        );
    }
    d
}

type Setup = (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
);

/// Alice at 60 before a brute of 60 that knows a crit's proc of its own.
async fn setup() -> Setup {
    let (realm, mut a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let brute = p3::spawn(m, ga, BRUTE, 60, 3.0);
            on(m, brute, FIERCE_PROC);
            crate::game::map::visibility::refresh_dirty(m);
            brute
        })
        .await
        .unwrap();
    a.sees(brute).await;
    a.clear();
    (realm, a, ga, map, brute)
}

/// `spell`'s aura on `g`, as its own.
fn on(m: &mut Map, g: Guid, spell: u32) {
    crate::game::tests_p3_spells_kit::apply(m, g, g, spell);
}

fn holder(m: &Map, g: Guid, spell: u32) -> Option<auras::Holder> {
    crate::game::tests_p3_spells_kit::holder(m, g, spell)
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    crate::game::tests_p3_spells_kit::has(m, g, spell)
}

fn sit(m: &mut Map, g: Guid) {
    m.players.get_mut(&g).unwrap().set_stand_state(1);
}

fn stand_state(m: &Map, g: Guid) -> u8 {
    m.players[&g].stand_state()
}

/// The brute's swing at Alice, as it rolls.
fn swing(m: &mut Map, brute: Guid, ga: Guid) {
    let now = m.now;
    if let Some(c) = m.combat_mut(brute) {
        c.victim = ga;
        c.attacking = true;
        c.next_swing = Some(now);
        c.next_offhand = Some(now + Duration::from_secs(60));
    }
    combat::try_swing(m, brute);
    combat::stop_attack(m, brute);
}

/// Every proc ready again, and their marks gone.
fn rearm(m: &mut Map, ga: Guid, brute: Guid) {
    for (g, mark) in [(ga, DEFENDED), (ga, SOAKED), (brute, FIERCE)] {
        auras::remove_spell(m, g, mark, None);
    }
    for g in [ga, brute] {
        if let Some(a) = auras::auras_mut(m, g) {
            for h in a.holders.iter_mut() {
                h.proc_ready_at = None;
            }
        }
    }
    m.now += Duration::from_secs(5);
}

/// The hit info of the next blow Alice is told of.
async fn hit_info(a: &mut Client) -> u32 {
    let b = a
        .wait_frame("the blow", combat::SMSG_ATTACKERSTATEUPDATE, |_| true)
        .await;
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Thistle Tea's "a seated critical hit preserves damage and attacker
/// procs" and "standing critical hits still spend defensive proc charges
/// and start their cooldown": a blow on a seated player crits and stands
/// her; the attacker's crit proc goes off. Her own crit-taken proc goes off
/// too, a charge spent and its cooldown begun, as it does on a crit taken
/// standing: mangos's `DealDamage` stands her before the blow's procs, and
/// sitting to be crit was 1.12's way to set off Reckoning and Enrage
/// (Thistle Tea spares a seated victim's procs).
#[tokio::test]
async fn a_seated_player_is_crit_and_stood_and_both_sides_crit_procs_go_off() {
    let (_realm, mut a, ga, map, brute) = setup().await;
    let seated = map
        .query(move |m| {
            on(m, ga, DEFENSE);
            sit(m, ga);
            swing(m, brute, ga);
            let h = holder(m, ga, DEFENSE).unwrap();
            let cooling = h.proc_ready_at.map(|t| t.duration_since(m.now).as_millis());
            (
                stand_state(m, ga),
                has(m, brute, FIERCE),
                has(m, ga, DEFENDED),
                h.charges,
                cooling,
            )
        })
        .await
        .unwrap();
    assert_ne!(hit_info(&mut a).await & HITINFO_CRITICALHIT, 0, "a crit");
    assert_eq!(seated.0, 0, "stood up");
    assert!(seated.1, "the attacker's crit proc");
    assert_eq!(
        (seated.2, seated.3, seated.4),
        (true, 2, Some(3_000)),
        "hers: a charge spent, three seconds' cooldown"
    );
    let standing = map
        .query(move |m| {
            rearm(m, ga, brute);
            p3::forced_swing(m, brute, ga, Outcome::Crit);
            combat::stop_attack(m, brute);
            let h = holder(m, ga, DEFENSE).unwrap();
            (
                has(m, ga, DEFENDED),
                h.charges,
                h.proc_ready_at.is_some_and(|t| t > m.now),
            )
        })
        .await
        .unwrap();
    assert_eq!(standing, (true, 1, true), "standing, the same");
}

/// Thistle Tea's "a seated absorbed crit retains absorb procs": a crit on
/// a seated player that her shield soaks whole stands her and sets off her
/// absorb proc (and, as above, her crit proc).
#[tokio::test]
async fn a_seated_players_absorbed_crit_sets_off_her_absorb_procs() {
    let (_realm, _a, ga, map, brute) = setup().await;
    let r = map
        .query(move |m| {
            on(m, ga, DEFENSE);
            on(m, ga, ABSORBING);
            p3::aura_of(m, ga, 96_520, AURA_SCHOOL_ABSORB, 1, 100_000);
            sit(m, ga);
            let before = m.players[&ga]
                .fields
                .get_u32(crate::game::fields::UNIT_FIELD_HEALTH);
            swing(m, brute, ga);
            let after = m.players[&ga]
                .fields
                .get_u32(crate::game::fields::UNIT_FIELD_HEALTH);
            (
                before - after,
                stand_state(m, ga),
                has(m, ga, SOAKED),
                holder(m, ga, ABSORBING).map(|h| h.charges),
                has(m, ga, DEFENDED),
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (0, 0, true, Some(2), true));
}

/// Thistle Tea's "immune and godmode players do not change posture": a
/// seated player immune to the blow's school, or in god mode, stays
/// seated when struck.
#[tokio::test]
async fn the_immune_and_god_mode_stay_seated_when_struck() {
    let (_realm, _a, ga, map, brute) = setup().await;
    let r = map
        .query(move |m| {
            p3::aura_of(m, ga, 96_521, AURA_DAMAGE_IMMUNITY, 1, 0);
            sit(m, ga);
            swing(m, brute, ga);
            let immune = stand_state(m, ga);
            auras::remove_spell(m, ga, 96_521, None);
            m.players.get_mut(&ga).unwrap().misc.god = true;
            sit(m, ga);
            swing(m, brute, ga);
            let god = stand_state(m, ga);
            m.players.get_mut(&ga).unwrap().misc.god = false;
            swing(m, brute, ga);
            (immune, god, stand_state(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(r, (1, 1, 0), "(immune, god mode) seated; out of it, stood");
}

/// Whether Alice was told `spell`'s damage crit
/// (`SMSG_SPELLNONMELEEDAMAGELOG`).
async fn crit(a: &mut Client, spell: u32) -> bool {
    let b = a
        .wait_frame(
            "the spell's damage",
            crate::game::map::spells::outcome::SMSG_SPELLNONMELEEDAMAGELOG,
            |b| {
                let mut i = 0;
                for _ in 0..2 {
                    i += 1 + b[i].count_ones() as usize;
                }
                b[i..i + 4] == spell.to_le_bytes()
            },
        )
        .await;
    let mut i = 0;
    for _ in 0..2 {
        i += 1 + b[i].count_ones() as usize;
    }
    // Spell, damage, school, absorbed, resisted, two bytes, blocked, then
    // the hit flags.
    let at = i + 4 + 4 + 1 + 4 + 4 + 2 + 4;
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) & 0x2 != 0
}

/// Thistle Tea's "seated melee and ranged ability crits still trigger
/// defensive talents" and "sitting does not force magic or cannot-crit
/// spell damage to crit": the brute's melee or ranged ability at a seated
/// Alice crits and stands her, and her crit-taken proc goes off; a spell
/// of magic doesn't crit her for her sitting, nor an ability that can't
/// crit, though both stand her.
#[tokio::test]
async fn an_ability_crits_a_seated_player_and_her_procs_answer_but_magic_and_cant_crit_dont() {
    let (_realm, mut a, ga, map, brute) = setup().await;
    let mut seen = Vec::new();
    for (spell, crits) in [
        (MELEE, true),
        (RANGED, true),
        (MAGIC, false),
        (MELEE_NO_CRIT, false),
        (RANGED_NO_CRIT, false),
    ] {
        let r = map
            .query(move |m| {
                rearm(m, ga, brute);
                auras::remove_spell(m, ga, DEFENSE, None);
                on(m, ga, DEFENSE);
                sit(m, ga);
                crate::game::map::spells::cast_triggered(m, brute, spell, Some(ga));
                (
                    stand_state(m, ga),
                    has(m, ga, DEFENDED),
                    holder(m, ga, DEFENSE).map(|h| h.charges),
                )
            })
            .await
            .unwrap();
        let critted = crit(&mut a, spell).await;
        assert_eq!(critted, crits, "{spell} crits: {crits}");
        let charges = if crits { 2 } else { 3 };
        seen.push((spell, r));
        assert_eq!(
            r,
            (0, crits, Some(charges)),
            "{spell}: stood, and her proc as it crit"
        );
    }
    assert_eq!(seen.len(), 5);
}
