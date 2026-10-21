//! Phase 6's loader round, the spells (Thistle Tea's `world/loader/*_dbc_test.exs`
//! of spells): what wvs reads of the real spells, and what they do.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::spells::{self, fail};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::forced_swing;

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn channel(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_CHANNEL_SPELL))
}

/// `g` casting `spell`, its cast time to go, as though begun.
fn begun(m: &mut Map, g: Guid, spell: u32) {
    let now = m.now;
    if let Some(c) = m.caster_mut(g) {
        c.casting = Some(spells::Casting {
            spell,
            targets: at(g),
            ends: now + Duration::from_millis(1500),
            item: None,
            pushbacks: 0,
            channel_until: None,
            duration: 1500,
            at: None,
        });
    }
}

const BLACKFATHOM_CHANNELING: u32 = 8734;
const DESTROY_SPEAR: u32 = 16_557;

/// Thistle Tea's `CombatState.enter` (`combat_entry_dbc_test.exs`) and
/// mangos-classic's `SetInCombatState`: Blackfathom Channeling (a duration
/// of -1) runs on past a minute, until its creature's fight starts (it is
/// not for one); a fight stops a mount's (458) and Resurrection's (2006)
/// cast, not Fireball's, Conjure Food's, Summon Imp's or the
/// Hearthstone's; Destroy Spear's channel stops at the first blow, which
/// is what starts the fight (mangos's channel that damage breaks, any
/// unit's).
#[tokio::test]
#[ignore = "extended"]
async fn real_channels_and_casts_stop_or_go_on_as_a_fight_starts() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P6LFIGHT", "Fighta", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let acolyte = dummy(m, g, 30, 5.0);
        spells::cast_triggered(m, acolyte, BLACKFATHOM_CHANNELING, Some(acolyte));
        let started = (casting(m, acolyte), channel(m, acolyte));
        pass(m, 61_000);
        let later = (casting(m, acolyte), channel(m, acolyte));
        combat::enter(m, acolyte, false);
        let fought = (casting(m, acolyte), channel(m, acolyte));
        let guard = dummy(m, g, 30, 3.0);
        spells::cast_triggered(m, guard, DESTROY_SPEAR, Some(guard));
        let spear = casting(m, guard);
        forced_swing(m, g, guard, Outcome::Hit);
        let struck = (casting(m, guard), channel(m, guard));
        let mut stopped = Vec::new();
        for id in [458, 2006, 133, 587, 688, 8690] {
            combat::leave_combat(m, g);
            begun(m, g, id);
            combat::enter(m, g, false);
            stopped.push((id, casting(m, g).is_none()));
            fresh(m, g);
        }
        (started, later, fought, spear, struck, stopped)
    })
    .await;
    let (started, later, fought, spear, struck, stopped) = out;
    let on = Some(BLACKFATHOM_CHANNELING);
    assert_eq!(started, (on, BLACKFATHOM_CHANNELING));
    assert_eq!(later, (on, BLACKFATHOM_CHANNELING), "a minute on");
    assert_eq!(fought, (None, 0), "the fight ends it");
    assert_eq!(spear, Some(DESTROY_SPEAR));
    assert_eq!(struck, (None, 0), "a blow ends Destroy Spear");
    assert_eq!(
        stopped,
        vec![
            (458, true),
            (2006, true),
            (133, false),
            (587, false),
            (688, false),
            (8690, false)
        ]
    );
}

const HAMMER_OF_JUSTICE: u32 = 853;
const KIDNEY_SHOT: u32 = 408;
const FEAR: u32 = 5782;
const SILENCE: u32 = 15_487;
const DIVINE_SHIELD: u32 = 642;
const DIVINE_SHIELD_2: u32 = 1020;
const BLESSING_OF_PROTECTION: u32 = 1022;
const ICE_BLOCK: u32 = 11_958;
const BLINK: u32 = 1953;
const WILL_OF_THE_FORSAKEN: u32 = 7744;
const FIREBALL: u32 = 133;

/// What stops `g` casting `spell` at itself under `controls`: the cast
/// check's code.
fn under(m: &mut Map, g: Guid, from: Guid, controls: &[u32], spell: u32) -> Option<u8> {
    let data = m.realm.data.clone();
    for c in [HAMMER_OF_JUSTICE, KIDNEY_SHOT, FEAR, SILENCE] {
        crate::game::map::auras::remove_spell(m, g, c, None);
    }
    for c in controls {
        apply(m, from, g, *c);
    }
    let s = data.spells.get(spell).unwrap().clone();
    let mods = spells::mods::for_spell(m, g, &s);
    fresh(m, g);
    spells::checks::check(m, g, &s, &at(g), None, false, &mods)
}

/// Thistle Tea's `CasterState` (`immunity_escape_dbc_test.exs`) and
/// mangos-classic's `CheckCasterAuras`: a spell whose immunities purge
/// (`SPELL_ATTR_EX_IMMUNITY_PURGES_EFFECT`) is cast under what they cancel,
/// and only that. Divine Shield (both ranks) and Ice Block (every school)
/// under a holy stun, a fear and a silence at once; Blessing of Protection
/// (physical) under Kidney Shot, not Hammer of Justice (holy) nor with a
/// fear; Will of the Forsaken under a fear; Blink (stun and root) under a
/// stun, not a silence, nor a stun and a silence (mangos's silence check
/// comes after the stun's; Thistle Tea lets it through). Fireball, under
/// any of them, isn't.
#[tokio::test]
#[ignore = "extended"]
async fn real_escapes_break_only_what_their_immunities_cancel() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P6LESCAPE", "Escapa", Race::Human, Class::Paladin).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let foe = dummy(m, g, 60, 5.0);
        let all = [HAMMER_OF_JUSTICE, FEAR, SILENCE];
        vec![
            under(m, g, foe, &all, DIVINE_SHIELD),
            under(m, g, foe, &all, DIVINE_SHIELD_2),
            under(m, g, foe, &all, ICE_BLOCK),
            under(m, g, foe, &[KIDNEY_SHOT], BLESSING_OF_PROTECTION),
            under(m, g, foe, &[KIDNEY_SHOT, FEAR], BLESSING_OF_PROTECTION),
            under(m, g, foe, &[HAMMER_OF_JUSTICE], BLESSING_OF_PROTECTION),
            under(m, g, foe, &[FEAR], WILL_OF_THE_FORSAKEN),
            under(m, g, foe, &[HAMMER_OF_JUSTICE], BLINK),
            under(m, g, foe, &[SILENCE], BLINK),
            under(m, g, foe, &[HAMMER_OF_JUSTICE, SILENCE], BLINK),
            under(m, g, foe, &[HAMMER_OF_JUSTICE], FIREBALL),
            under(m, g, foe, &[FEAR], FIREBALL),
            under(m, g, foe, &[SILENCE], FIREBALL),
        ]
    })
    .await;
    let control =
        |c: Option<u8>| c.filter(|c| [fail::STUNNED, fail::FLEEING, fail::SILENCED].contains(c));
    let out: Vec<Option<u8>> = out.into_iter().map(control).collect();
    assert_eq!(
        out,
        vec![
            None,
            None,
            None,
            None,
            Some(fail::FLEEING),
            Some(fail::STUNNED),
            None,
            None,
            Some(fail::SILENCED),
            Some(fail::SILENCED),
            Some(fail::STUNNED),
            Some(fail::FLEEING),
            Some(fail::SILENCED),
        ]
    );
}

const CAT_FORM: u32 = 768;
const BEAR_FORM: u32 = 5487;
const DIRE_BEAR_FORM: u32 = 9634;
const REFLECTION: u32 = 17_108;
const INNERVATE: u32 = 29_166;

/// `g`'s regeneration tick, now: the mana it gave.
fn tick(m: &mut Map, g: Guid) -> i64 {
    let before = power(m, g, 0);
    let now = m.now;
    m.players.get_mut(&g).unwrap().regen_at = now;
    crate::game::map::stats::regen(m);
    i64::from(power(m, g, 0)) - i64::from(before)
}

fn power_type(m: &Map, g: Guid) -> u8 {
    m.players[&g].fields.get_byte(UNIT_FIELD_BYTES_0, 3)
}

fn later(m: &mut Map, ms: u64) {
    m.now += Duration::from_millis(ms);
}

/// A level-60 druid with room for mana: half of it.
fn druid_ready(m: &mut Map, g: Guid) {
    ready(m, g, 60, &[CAT_FORM, BEAR_FORM, DIRE_BEAR_FORM]);
    let p = m.players.get_mut(&g).unwrap();
    p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 4000);
    p.fields.set_u32(UNIT_FIELD_POWER1, 2000);
    p.mana_spent_at = None;
}

/// Thistle Tea's shapeshifted regeneration (`shapeshift_regeneration_dbc_test.exs`)
/// and mangos-classic's `Player::RegenerateAll` (mana in any form): each
/// form costs mana, and then none comes back for five seconds; after, it
/// comes back in cat, bear and dire bear form (their energy and rage), and
/// still once the form is cancelled (mana again), every two seconds, cat
/// energy full or not. Reflection lets 15% through within the five
/// seconds while shifted, Innervate five times as much and all of it, and
/// with Innervate gone (cancelled or run out) it is as it was. Death ends
/// the form and Innervate, gives nothing while dead, and the mana comes
/// back after a resurrection.
#[tokio::test]
#[ignore = "extended"]
async fn real_forms_cost_mana_and_it_comes_back_in_them() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P6LREGEN", "Regena", Race::NightElf, Class::Druid).await;
    let out = on(&map, move |m| {
        druid_ready(m, g);
        let mut forms = Vec::new();
        for spell in [CAT_FORM, BEAR_FORM, DIRE_BEAR_FORM] {
            let before = power(m, g, 0);
            assert!(cast_fresh(m, g, spell, at(g)), "{spell}");
            let spent = before > power(m, g, 0);
            let shifted = power_type(m, g);
            let within = tick(m, g);
            later(m, 5000);
            let after = tick(m, g);
            crate::game::map::auras::remove_spell(m, g, spell, None);
            let back = (power_type(m, g), crate::game::map::auras::form(m, g));
            later(m, 2000);
            let caster = tick(m, g);
            forms.push((spell, spent, shifted, within, after, back, caster));
            later(m, 10_000);
        }
        // Cat energy full: a tick every two seconds still.
        set_power(m, g, 0, 2000);
        assert!(cast_fresh(m, g, CAT_FORM, at(g)));
        later(m, 6000);
        set_power(m, g, 3, 100);
        let _ = tick(m, g);
        let due = m.players[&g].regen_at;
        let early = {
            later(m, 1999);
            let before = power(m, g, 0);
            crate::game::map::stats::regen(m);
            i64::from(power(m, g, 0)) - i64::from(before)
        };
        later(m, 1);
        let on_time = tick(m, g);
        let cadence = (
            due.duration_since(m.now - Duration::from_millis(2000)),
            early,
            on_time,
        );
        // Reflection and Innervate, shifted, within the five seconds.
        let foe = dummy(m, g, 60, 5.0);
        apply(m, g, g, REFLECTION);
        m.players.get_mut(&g).unwrap().mana_spent_at = Some(m.now);
        let reflected = tick(m, g);
        apply(m, g, g, INNERVATE);
        let innervated = tick(m, g);
        crate::game::map::auras::remove_spell(m, g, INNERVATE, None);
        let cancelled = tick(m, g);
        apply(m, g, g, INNERVATE);
        pass(m, 21_000);
        let gone = !has(m, g, INNERVATE);
        later(m, 5000);
        let expired = tick(m, g);
        // Death, then a resurrection.
        apply(m, g, g, INNERVATE);
        combat::deal_damage(m, foe, g, 10_000_000);
        let dead = (
            crate::game::map::auras::form(m, g),
            power_type(m, g),
            has(m, g, INNERVATE),
            has(m, g, CAT_FORM),
        );
        let while_dead = tick(m, g);
        crate::game::map::death::resurrect(m, g, 0.5);
        later(m, 6000);
        let risen = tick(m, g);
        (
            forms,
            cadence,
            (reflected, innervated, cancelled, gone, expired),
            dead,
            while_dead,
            risen,
        )
    })
    .await;
    let (forms, cadence, auras, dead, while_dead, risen) = out;
    let x = forms[0].4;
    assert!(x > 0, "{forms:?}");
    for (spell, kind) in [(CAT_FORM, 3), (BEAR_FORM, 1), (DIRE_BEAR_FORM, 1)] {
        let f = forms.iter().find(|f| f.0 == spell).unwrap();
        assert_eq!(*f, (spell, true, kind, 0, x, (0, 0), x), "{spell}");
    }
    assert_eq!(cadence, (Duration::from_secs(2), 0, x), "every two seconds");
    let (reflected, innervated, cancelled, gone, expired) = auras;
    assert_eq!(reflected, (x as f32 * 0.15) as i64, "Reflection's 15%");
    assert_eq!(innervated, x * 5, "Innervate's 400% more, all of it");
    assert_eq!(cancelled, reflected, "Innervate cancelled");
    assert!(gone, "Innervate runs out");
    assert_eq!(expired, x);
    assert_eq!(
        dead,
        (0, 0, false, false),
        "death ends the form and Innervate"
    );
    assert_eq!(while_dead, 0);
    assert_eq!(risen, x, "back after a resurrection");
}

const FLASH_HEAL: u32 = 10_917;
const PROPHECY_BONUS: u32 = 21_973;
const IMMOLATE: u32 = 11_668;
const IMMOLATE_BONUS: u32 = 23_047;
const MIND_QUICKENING: u32 = 23_723;
const SINISTER_STRIKE: u32 = 1752;
const BESTIAL_WRATH: u32 = 19_574;
const DEVOUR_MAGIC: u32 = 19_734;

/// `spell` cast by `g` at `t`, fresh: its cast time and global cooldown,
/// in milliseconds.
fn timed(m: &mut Map, g: Guid, spell: u32, t: spells::Targets) -> (u64, u64) {
    let now = m.now;
    assert!(cast_fresh(m, g, spell, t), "{spell}");
    let c = m.caster(g).unwrap();
    let cast = c
        .casting
        .as_ref()
        .map_or(0, |c| c.ends.duration_since(now).as_millis() as u64);
    let gcd = c
        .gcd_until
        .map_or(0, |t| t.duration_since(now).as_millis() as u64);
    fresh(m, g);
    (cast, gcd)
}

/// The global cooldown `g`'s modifiers give `spell`.
fn gcd_of(m: &Map, g: Guid, spell: u32) -> u32 {
    let s = m.realm.data.spells.get(spell).unwrap();
    spells::mods::for_spell(m, g, s)
        .apply_ms(spells::mods::OP_GLOBAL_COOLDOWN, s.start_recovery_time)
}

/// Thistle Tea's global cooldowns (`global_cooldown_dbc_test.exs`) and
/// mangos-classic's `Player::AddGCD`: the Prophecy set's Flash Heal bonus
/// takes Flash Heal's cast and global cooldown to 1.4 s, the Nemesis
/// set's Immolate's cast to 1.8 s and its global cooldown to 1.3 s, and
/// neither touches Frostbolt's 1.5 s; off, they are as they were. Mind
/// Quickening leaves the magic (1.5 s) and rogue (1 s) global cooldowns
/// alone and has none of its own; Bestial Wrath, of the shared category
/// with no time, waits on another's (Devour Magic's) and starts none.
#[tokio::test]
#[ignore = "extended"]
async fn real_global_cooldowns_follow_their_modifiers() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P6LGCD", "Gcdia", Race::Human, Class::Priest).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[FLASH_HEAL, IMMOLATE]);
        let foe = dummy(m, g, 60, 10.0);
        let plain = (
            timed(m, g, FLASH_HEAL, at(g)),
            timed(m, g, IMMOLATE, at(foe)),
        );
        apply(m, g, g, PROPHECY_BONUS);
        apply(m, g, g, IMMOLATE_BONUS);
        let bonus = (
            timed(m, g, FLASH_HEAL, at(g)),
            timed(m, g, IMMOLATE, at(foe)),
            gcd_of(m, g, 116),
        );
        crate::game::map::auras::remove_spell(m, g, PROPHECY_BONUS, None);
        crate::game::map::auras::remove_spell(m, g, IMMOLATE_BONUS, None);
        let off = (
            timed(m, g, FLASH_HEAL, at(g)),
            timed(m, g, IMMOLATE, at(foe)),
        );
        apply(m, g, g, MIND_QUICKENING);
        let quick = (
            gcd_of(m, g, FLASH_HEAL),
            gcd_of(m, g, SINISTER_STRIKE),
            gcd_of(m, g, MIND_QUICKENING),
        );
        // Bestial Wrath after Devour Magic's 1.5 s.
        let wrath = m.realm.data.spells.get(BESTIAL_WRATH).unwrap().clone();
        let devour = m.realm.data.spells.get(DEVOUR_MAGIC).unwrap().clone();
        let now = m.now;
        if let Some(c) = m.caster_mut(g) {
            c.gcd_until = Some(now + Duration::from_millis(u64::from(devour.start_recovery_time)));
        }
        let c = m.caster(g).unwrap();
        let waits = (
            spells::on_gcd(
                c,
                wrath.start_recovery_category,
                now + Duration::from_millis(1499),
            ),
            spells::on_gcd(
                c,
                wrath.start_recovery_category,
                now + Duration::from_millis(1500),
            ),
        );
        (plain, bonus, off, quick, waits, gcd_of(m, g, BESTIAL_WRATH))
    })
    .await;
    let (plain, bonus, off, quick, waits, wrath) = out;
    assert_eq!(plain, ((1500, 1500), (2000, 1500)));
    assert_eq!(bonus, ((1400, 1400), (1800, 1300), 1500));
    assert_eq!(off, plain);
    assert_eq!(quick, (1500, 1000, 0));
    assert_eq!(waits, (true, false));
    assert_eq!(wrath, 0, "Bestial Wrath starts none");
}

const SHIELD_BASH: u32 = 72;
const HAMSTRING: u32 = 1715;
const HEROIC_STRIKE: u32 = 78;
const MORTAL_STRIKE: u32 = 12_294;

/// Thistle Tea's `completely_blocked` (`spell_vmangos_test.exs`) and
/// mangos-classic's `CanBlockAbility`: Shield Bash and Hamstring, marked
/// blocked whole, are a miss when blocked (told as a block, nothing of
/// them lands: no damage, no interrupt, no snare), where Heroic Strike,
/// Mortal Strike and the judgements, not marked, are blocked in part and
/// land.
#[test]
#[ignore = "extended"]
fn real_shield_bash_and_hamstring_are_blocked_whole_the_rest_in_part() {
    use crate::game::map::spells::outcome::{self, Hit};
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let of = |id: u32| outcome::block_of(d.spells.get(id).unwrap());
    assert_eq!(of(SHIELD_BASH), Hit::Blocked);
    assert_eq!(of(HAMSTRING), Hit::Blocked);
    for id in [HEROIC_STRIKE, MORTAL_STRIKE, 20_467, 20_424] {
        assert_eq!(of(id), Hit::Block, "{id}");
    }
    assert!(!Hit::Blocked.lands() && Hit::Block.lands());
    assert_eq!((Hit::Blocked.wire(), Hit::Block.wire()), (5, 5));
    assert_eq!(
        Hit::Blocked.proc_ex(),
        crate::game::map::procs::EX_BLOCK,
        "a block, no hit"
    );
}

const BATTLE_SHOUT: u32 = 6673;
const BATTLE_SHOUT_TOP: u32 = 11_551;
const SUNDER_ARMOR: u32 = 7386;
const SUNDER_ARMOR_TOP: u32 = 11_597;
const FROSTBOLT: u32 = 116;
const FROSTBOLT_2: u32 = 205;

/// Thistle Tea's `SpellBook.learn` (`spell_vmangos_test.exs`) and
/// mangos-classic's inactive ranks (`addSpell`, the skill line's
/// `superseded_by`): a warrior who has learned every rank to sixty has
/// only the highest of each in the spellbook at login (Battle Shout's
/// 11551, Sunder Armor's 11597), whatever order the ranks came in; a
/// lower rank is still known (a trainer counts it) but not cast at the
/// client's request. A mage's Frostbolt ranks stack, and both are shown.
#[tokio::test]
#[ignore = "extended"]
async fn real_superseded_ranks_leave_the_spellbook_but_stay_known() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, g, map) = player(&realm, "P6LRANKS", "Ranka", Race::Human, Class::Warrior).await;
    let known = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        // The lower ranks first, then every rank the trainers teach.
        spells::learn(m, g, BATTLE_SHOUT);
        spells::learn(m, g, SUNDER_ARMOR);
        crate::game::map::npc::learn_class_spells(m, g);
        spells::learn(m, g, FROSTBOLT);
        spells::learn(m, g, FROSTBOLT_2);
        let p = &m.players[&g];
        [
            BATTLE_SHOUT,
            BATTLE_SHOUT_TOP,
            SUNDER_ARMOR,
            SUNDER_ARMOR_TOP,
        ]
        .map(|s| p.spells.contains(&s))
    })
    .await;
    assert_eq!(known, [true; 4], "every rank known");
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    c.login(g).await;
    let book: Vec<u32> = c
        .wait_for("SMSG_INITIAL_SPELLS", |m| match m {
            ServerOpcodeMessage::SMSG_INITIAL_SPELLS(s) => Some(
                s.initial_spells
                    .iter()
                    .map(|i| u32::from(i.spell_id))
                    .collect(),
            ),
            _ => None,
        })
        .await;
    assert!(book.contains(&BATTLE_SHOUT_TOP) && book.contains(&SUNDER_ARMOR_TOP));
    assert!(!book.contains(&BATTLE_SHOUT) && !book.contains(&SUNDER_ARMOR));
    assert!(
        book.contains(&FROSTBOLT) && book.contains(&FROSTBOLT_2),
        "Frostbolt's stack"
    );
    c.send(CMSG_CAST_SPELL {
        spell: BATTLE_SHOUT,
        targets: SpellCastTargets::default(),
    })
    .await;
    let refused = c
        .wait_frame("SMSG_CAST_RESULT", 0x130, |b| {
            b[..4] == BATTLE_SHOUT.to_le_bytes()
        })
        .await;
    assert_eq!(refused[4..6], [2, fail::NOT_KNOWN]);
}
