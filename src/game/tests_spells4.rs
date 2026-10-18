//! Spells' fourth round, against the real data when it is here: aura
//! capacity, stance rage, periodic damage ramps, override class script
//! procs, Feign Death's resistance, crowd control breaking early, and the
//! smaller spell scripts.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, class_scripts};
use crate::game::map::{Map, MapKey, auras};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// A Kobold Vermin at `level`, `yards` in front of `g`, hardy and without
/// armor.
fn dummy(m: &mut Map, g: Guid, level: u8, yards: f32) -> Guid {
    spawn(m, g, 6, level, yards)
}

/// A creature of `entry` at `level`, `yards` in front of `g`, hardy and
/// without armor.
fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the creature");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &template,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    let c = c.with_base();
    m.add_creature(c)
}

fn ready(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
    }
    let p = m.players.get_mut(&g).unwrap();
    for i in 0..5 {
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
        p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
    }
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// A spell that only gives `amount` of aura `kind`.
fn plain_aura(id: u32, kind: u32, misc: i32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        misc_value: misc,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: "Test Aura".into(),
        equipped_item_class: -1,
        effects,
        ..Default::default()
    }
}

fn apply(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default());
}

fn timed() -> auras::Context {
    auras::Context {
        duration: Some(Some(60_000)),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn capacity_stance_rage_and_ramps() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "WARRIOR5");
    let w = c.enter("Warrc", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            let kobold = dummy(m, w, 60, 5.0);
            // Thirty-two of the warrior's own buffs; one more from someone
            // else takes the oldest's place.
            for i in 0..32 {
                let b = plain_aura(990_400 + i, auras::AURA_MOD_STAT, 0);
                let _ = auras::apply_spell(m, w, w, &b, &[(0, 1)], timed());
            }
            let full = auras::auras(m, w).map_or(0, |a| {
                a.holders
                    .iter()
                    .filter(|h| h.positive && h.slot.is_some())
                    .count()
            });
            let gift = plain_aura(990_500, auras::AURA_MOD_STAT, 0);
            let _ = auras::apply_spell(m, kobold, w, &gift, &[(0, 1)], timed());
            let swapped = (
                has(m, w, 990_400),
                auras::auras(m, w)
                    .and_then(|a| a.holders.iter().find(|h| h.spell == 990_500))
                    .is_some_and(|h| h.slot.is_some()),
            );
            // One more of the warrior's own, of least worth itself: the
            // next oldest goes.
            let own = plain_aura(990_501, auras::AURA_MOD_STAT, 0);
            let _ = auras::apply_spell(m, w, w, &own, &[(0, 1)], timed());
            let next = (has(m, w, 990_401), has(m, w, 990_501), has(m, w, 990_500));
            // A stance keeps only Tactical Mastery's rage.
            m.players
                .get_mut(&w)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_POWER1 + 1, 500);
            let mastery = plain_aura(990_502, auras::AURA_OVERRIDE_CLASS_SCRIPTS, 833);
            let _ = auras::apply_spell(m, w, w, &mastery, &[(0, 0)], auras::Context::default());
            spells::cast_triggered(m, w, 71, None);
            let rage = m.players[&w].fields.get_u32(UNIT_FIELD_POWER1 + 1);
            // Curse of Agony: half, then whole, then one and a half.
            let data = m.realm.data.clone();
            let agony = [1, 5, 9].map(|t| class_scripts::ramp(&data, 980, t));
            (full, swapped, next, rage, agony)
        })
        .await
        .unwrap();
    let (full, swapped, next, rage, agony) = out;
    assert_eq!(full, 32, "thirty-two buffs show");
    assert_eq!(
        swapped,
        (false, true),
        "the oldest of least worth gives way"
    );
    assert_eq!(next, (false, true, true), "then the next oldest");
    assert_eq!(rage, 150, "Tactical Mastery's 15 rage kept");
    assert_eq!(agony, [-3.5, 0.0, 3.5], "Curse of Agony's ramp");
}

#[tokio::test]
#[ignore = "extended"]
async fn override_class_scripts_and_reactive_armor() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "MAGE5");
    let mage = c.enter("Maged", Race::Human, Class::Mage).await;
    let mut d = Client::connect(&realm, "WARRIOR6");
    let w = d.enter("Warrd", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            ready(m, w, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            let holder = |m: &Map, g: Guid, spell: u32| {
                auras::auras(m, g)
                    .and_then(|a| a.holders.iter().find(|h| h.spell == spell).cloned())
                    .unwrap()
            };
            // Improved Blizzard chills.
            apply(m, mage, mage, 11185);
            let h = holder(m, mage, 11185);
            class_scripts::proc(m, mage, kobold, &h, Some(10), true);
            let chilled = has(m, kobold, 12484);
            // Holy Power gives a warrior armor.
            apply(m, mage, mage, 28789);
            let h = holder(m, mage, 28789);
            class_scripts::proc(m, mage, w, &h, Some(635), true);
            let armored = has(m, w, 28790);
            // The druid set's Rejuvenation gives a mana user mana.
            apply(m, mage, mage, 28716);
            let h = holder(m, mage, 28716);
            class_scripts::proc(m, mage, mage, &h, Some(774), true);
            let mana =
                has(m, mage, 28722) || m.players[&mage].fields.get_u32(UNIT_FIELD_POWER1) > 0;
            // Obsidian Armor answers a frost spell with frost resistance.
            apply(m, w, w, 27539);
            let h = holder(m, w, 27539);
            class_scripts::proc(m, w, kobold, &h, Some(116), false);
            let resists = has(m, w, 27534);
            (chilled, armored, mana, resists)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true, true));
}

#[tokio::test]
#[ignore = "extended"]
async fn feign_death_and_breaking_crowd_control() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "HUNTER5");
    let h = c.enter("Hunterb", Race::Dwarf, Class::Hunter).await;
    let mut d = Client::connect(&realm, "MAGE6");
    let mage = d.enter("Magee", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, h, 60, &[]);
            ready(m, mage, 1, &[]);
            // Feign Death: low creatures believe it, and forget the hunter.
            let low = dummy(m, h, 1, 5.0);
            m.creature_mut(low).unwrap().threat.push((h, 10.0));
            spells::cast_triggered(m, h, 5384, None);
            let believed = !m.creature(low).unwrap().threat.iter().any(|(t, _)| *t == h);
            auras::remove_spell(m, h, 5384, None);
            // Far higher ones see through it: the hunter stays on their minds.
            m.spells.rolls = Some(true);
            m.players
                .get_mut(&h)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_LEVEL, 1);
            let high: Vec<Guid> = (0..6).map(|i| dummy(m, h, 63, 4.0 + i as f32)).collect();
            for k in &high {
                m.creature_mut(*k).unwrap().threat.push((h, 10.0));
            }
            spells::cast_triggered(m, h, 5384, None);
            let seen = high
                .iter()
                .any(|k| m.creature(*k).unwrap().threat.iter().any(|(t, _)| *t == h));
            m.spells.rolls = None;
            // A player's Polymorph breaks about twelve seconds in.
            apply(m, low, mage, 12826);
            let at = auras::auras(m, mage)
                .and_then(|a| a.holders.iter().find(|x| x.spell == 12826))
                .and_then(|x| x.heartbeat)
                .and_then(|b| b.break_at);
            let applied = m.now;
            m.now += Duration::from_secs(13);
            auras::update(m);
            let broke = !has(m, mage, 12826);
            // A creature's rolls the spell's hit every five seconds: a low
            // caster's Hibernate on a high kobold soon misses (Polymorph, a
            // confusion, isn't rolled on creatures).
            m.spells.rolls = Some(true);
            let kobold = high[0];
            apply(m, mage, kobold, 2637);
            let creature_beat = auras::auras(m, kobold)
                .and_then(|a| a.holders.iter().find(|x| x.spell == 2637))
                .and_then(|x| x.heartbeat)
                .is_some_and(|b| b.next_check.is_some() && b.hit_chance < 10_000);
            for _ in 0..12 {
                m.now += Duration::from_secs(5);
                auras::update(m);
            }
            let freed = !has(m, kobold, 2637);
            m.spells.rolls = None;
            (
                believed,
                seen,
                at.map(|t| t - applied),
                broke,
                creature_beat,
                freed,
            )
        })
        .await
        .unwrap();
    let (believed, seen, after, broke, creature_beat, freed) = out;
    assert!(believed, "the low kobold forgets the hunter");
    assert!(seen, "a high one sees through it");
    assert_eq!(
        after,
        Some(Duration::from_secs(12)),
        "twelve seconds at the median"
    );
    assert!(broke, "and breaks then");
    assert!(creature_beat, "a creature's rolls");
    assert!(freed, "and one misses");
    let _ = c
        .wait_for("SMSG_FEIGN_DEATH_RESISTED", |m| match m {
            ServerOpcodeMessage::SMSG_FEIGN_DEATH_RESISTED => Some(()),
            _ => None,
        })
        .await;
}

#[tokio::test]
#[ignore = "extended"]
async fn the_smaller_spell_scripts() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "ROGUE5");
    let r = c.enter("Rogueb", Race::Human, Class::Rogue).await;
    let mut d = Client::connect(&realm, "HUNTER6");
    let h = d.enter("Hunterc", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, r, 60, &[1784, 6770]);
            ready(m, h, 60, &[19388]);
            let data = m.realm.data.clone();
            let kobold = dummy(m, r, 60, 3.0);
            // Curse of Idiocy never pulses on its caster.
            apply(m, h, kobold, 1010);
            let idiocy = auras::auras(m, kobold)
                .and_then(|a| a.holders.iter().find(|x| x.spell == 1010).cloned());
            let pulses = idiocy.as_ref().map(|x| {
                (
                    class_scripts::periodic_trigger_allowed(m, kobold, x),
                    class_scripts::periodic_trigger_allowed(m, h, x),
                )
            });
            // General's Warcry: one of them at a time.
            apply(m, r, r, 28418);
            apply(m, r, r, 28419);
            let warcry = (has(m, r, 28418), has(m, r, 28419));
            // Sap doesn't keep stealth without Improved Sap; a lucky one
            // with it does.
            let sap = data.spells.get(6770).unwrap().clone();
            let plain = class_scripts::keeps_stealth(m, r, &sap);
            spells::learn(m, r, 14095);
            m.spells.rolls = Some(true);
            let kept = (0..40)
                .filter(|_| class_scripts::keeps_stealth(m, r, &sap))
                .count();
            // Frost Trap's aura goes off as a trap: Entrapment roots.
            apply(m, h, kobold, 13810);
            let trap = auras::auras(m, kobold)
                .and_then(|a| a.holders.iter().find(|x| x.spell == 13810).cloned())
                .unwrap();
            for _ in 0..60 {
                if has(m, kobold, 19185) {
                    break;
                }
                class_scripts::periodic_triggered(m, kobold, &trap);
            }
            let entrapped = has(m, kobold, 19185);
            m.spells.rolls = None;
            // Collecting Fallout keeps one of its two effects.
            let fallout = data.spells.get(12709).unwrap().clone();
            let one = class_scripts::one_of(m, &fallout);
            // Cannibalize's emote.
            let cannibalize = data.spells.get(20578).unwrap().clone();
            let _ = auras::apply_spell(m, r, r, &cannibalize, &[(0, 1)], auras::Context::default());
            let emote = m.players[&r].fields.get_u32(UNIT_NPC_EMOTESTATE);
            auras::remove_spell(m, r, 20578, None);
            let after = m.players[&r].fields.get_u32(UNIT_NPC_EMOTESTATE);
            // The druid's cleanse shakes off a root.
            let mut root = plain_aura(990_600, auras::AURA_MOD_ROOT, 0);
            root.mechanic = 7;
            let _ = auras::apply_spell(m, kobold, r, &root, &[(0, 1)], auras::Context::default());
            spells::cast_triggered(m, r, 9033, Some(r));
            let cleansed = !has(m, r, 990_600);
            // Capturing a creature takes it; two oozes make one.
            let pup = spawn(m, r, 10221, 10, 6.0);
            spells::cast_triggered(m, r, 15998, Some(pup));
            let captured = m.creature(pup).is_none();
            let (a, b) = (spawn(m, r, 6557, 30, 7.0), spawn(m, r, 6557, 30, 8.0));
            spells::cast_triggered(m, a, 16032, Some(b));
            let merged = (
                m.creature(b).is_none(),
                m.creatures.iter().any(|(_, c)| c.entry == 9621),
            );
            // Symbol of Life brings a dead creature back. Henze Faulk is
            // unkillable (static_flags1 0x8): a script's floor of 0 lets
            // him die here.
            let corpse = spawn(m, r, 6172, 10, 9.0);
            if let Some(s) = crate::game::map::event_ai::scripted_mut(m, corpse) {
                s.invincible_hp = Some(0);
            }
            crate::game::map::combat::deal_damage(m, r, corpse, 2_000_000);
            let died = !m.creature(corpse).unwrap().alive();
            spells::cast_triggered(m, r, 8593, Some(corpse));
            let restored = died && m.creature(corpse).unwrap().alive();
            (
                pulses, warcry, plain, kept, entrapped, one, emote, after, cleansed, captured,
                merged, restored,
            )
        })
        .await
        .unwrap();
    let (
        pulses,
        warcry,
        plain,
        kept,
        entrapped,
        one,
        emote,
        after,
        cleansed,
        captured,
        merged,
        restored,
    ) = out;
    assert_eq!(
        pulses,
        Some((true, false)),
        "Curse of Idiocy, never on its caster"
    );
    assert_eq!(warcry, (false, true), "one General's Warcry");
    assert!(!plain, "Sap breaks stealth");
    assert!(kept > 20, "Improved Sap keeps it nine times in ten: {kept}");
    assert!(entrapped, "Entrapment on Frost Trap's pulse");
    assert_eq!(one, Some(0), "one of Collecting Fallout's effects");
    assert_eq!((emote, after), (398, 0), "Cannibalize's emote");
    assert!(cleansed, "the cleanse");
    assert!(captured, "a captured creature");
    assert_eq!(merged, (true, true), "merged oozes");
    assert!(restored, "Symbol of Life");
}
