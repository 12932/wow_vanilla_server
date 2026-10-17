//! Spells' third round, against the real data when it is here: the class
//! scripts Thistle Tea has (Bloodthirst to Bestial Wrath), spells' travel
//! time, a higher rank taking a lower's place, and the combat log's spell
//! packets.

use std::path::Path;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, Targets, class_scripts};
use crate::game::map::{Map, MapKey, auras, pets};

const DB: &str = "<runs>/tt-data/db";

fn have_data() -> bool {
    Path::new(DB).join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(Path::new(DB)))
}

/// A Kobold Vermin at `level`, `yards` in front of `g`, hardy and without
/// armor.
fn dummy(m: &mut Map, g: Guid, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data.creature_template(6).cloned().expect("Kobold Vermin");
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
        guid::unit(6, counter | 0x80_0000),
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

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn hurt(m: &mut Map, g: Guid, by: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        let h = f.get_u32(UNIT_FIELD_HEALTH);
        f.set_u32(UNIT_FIELD_HEALTH, h.saturating_sub(by).max(1));
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

#[tokio::test]
async fn warrior_and_druid_damage_scripts() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "WARRIOR3");
    let w = c.enter("Warra", Race::Orc, Class::Warrior).await;
    let mut d = Client::connect(&realm, "DRUID3");
    let dr = d.enter("Druida", Race::Tauren, Class::Druid).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[]);
            ready(m, dr, 60, &[]);
            // Bloodthirst: 45% of attack power.
            let kobold = dummy(m, w, 60, 3.0);
            let ap = crate::game::map::combat::derived(m, w).attack_power;
            let before = health(m, kobold);
            spells::cast_triggered(m, w, 23881, Some(kobold));
            let bloodthirst = before - health(m, kobold);
            // Ferocious Bite: attack power for the combo points, and the
            // energy left, which it spends.
            let bite = m.realm.data.spells.get(22568).unwrap().clone();
            {
                let p = m.players.get_mut(&dr).unwrap();
                p.fields.set_u32(UNIT_FIELD_POWER1 + 3, 60);
                p.caster.combo_points = 5;
                p.caster.combo_target = kobold;
            }
            let before = health(m, kobold);
            spells::cast_triggered(m, dr, 22568, Some(kobold));
            let bitten = before - health(m, kobold);
            let energy = m.players[&dr].fields.get_u32(UNIT_FIELD_POWER1 + 3);
            let dap = crate::game::map::combat::derived(m, dr).attack_power;
            // Shield Slam's damage effect adds the block value.
            let slam = m.realm.data.spells.get(23922).unwrap().clone();
            let block = crate::game::map::combat::derived(m, w).block_value as f32;
            let l = spells::outcome::Landing::default();
            let slam_base = class_scripts::damage_base(m, w, &slam, 1, 10.0, &l);
            // Enrage: less armor, by the form.
            spells::cast_triggered(m, dr, 5229, None);
            let enrage = auras::auras(m, dr).and_then(|a| {
                a.holders
                    .iter()
                    .find(|h| h.spell == 25503)
                    .and_then(|h| h.effects.iter().find(|x| x.effect == 1).map(|x| x.amount))
            });
            // Intimidating Shout: its target cowers, the one beside it runs.
            let other = dummy(m, w, 60, 4.0);
            spells::cast_triggered(m, w, 5246, Some(kobold));
            let shout = (has(m, kobold, 5246), has(m, other, 5246));
            (
                bloodthirst,
                ap,
                bitten,
                energy,
                dap,
                bite.dmg_multiplier[0],
                slam_base,
                block,
                enrage,
                shout,
            )
        })
        .await
        .unwrap();
    let (bloodthirst, ap, bitten, energy, dap, mult, slam_base, block, enrage, shout) = out;
    let expected = ap as f32 * 0.45;
    assert!(
        (bloodthirst as f32 - expected).abs() <= 2.0,
        "Bloodthirst {bloodthirst} for {ap} attack power"
    );
    assert_eq!(energy, 0, "the bite spends the energy");
    assert!(
        bitten as f32 >= dap as f32 * 5.0 * 0.03 + 60.0 * mult,
        "Ferocious Bite {bitten}"
    );
    assert!(
        (slam_base - (10.0 + block)).abs() < 0.01,
        "Shield Slam's block value"
    );
    assert_eq!(enrage, Some(-27), "Enrage's armor out of dire bear form");
    assert_eq!(shout, (false, true), "the target cowers, the other flees");
}

#[tokio::test]
async fn shields_bubbles_and_bandages_leave_their_marks() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "PRIEST3");
    let pr = c.enter("Priestb", Race::Human, Class::Priest).await;
    let mut d = Client::connect(&realm, "PALADIN3");
    let pa = d.enter("Paladinb", Race::Human, Class::Paladin).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, pr, 60, &[]);
            ready(m, pa, 60, &[]);
            // Power Word: Shield, then Weakened Soul, and no shield again.
            spells::cast_triggered(m, pr, 17, Some(pa));
            let shielded = (has(m, pa, 17), has(m, pa, class_scripts::WEAKENED_SOUL));
            auras::remove_spell(m, pa, 17, None);
            spells::cast_triggered(m, pr, 17, Some(pa));
            let again = has(m, pa, 17);
            // Divine Shield, then Forbearance, and no Divine Protection.
            spells::cast_triggered(m, pa, 642, None);
            let bubbled = (has(m, pa, 642), has(m, pa, class_scripts::FORBEARANCE));
            spells::cast_triggered(m, pa, 498, None);
            let protected = has(m, pa, 498);
            // First Aid: Recently Bandaged, and no bandage again.
            spells::cast_triggered(m, pr, 746, Some(pr));
            let bandaged = (
                has(m, pr, 746),
                has(m, pr, class_scripts::RECENTLY_BANDAGED),
            );
            auras::remove_spell(m, pr, 746, None);
            spells::cast_triggered(m, pr, 746, Some(pr));
            let rebandaged = has(m, pr, 746);
            (shielded, again, bubbled, protected, bandaged, rebandaged)
        })
        .await
        .unwrap();
    let (shielded, again, bubbled, protected, bandaged, rebandaged) = out;
    assert_eq!(shielded, (true, true), "a shield, and Weakened Soul");
    assert!(!again, "no shield on a weakened soul");
    assert_eq!(bubbled, (true, true), "a bubble, and Forbearance");
    assert!(!protected, "no other bubble with Forbearance");
    assert_eq!(bandaged, (true, true), "a bandage, and Recently Bandaged");
    assert!(!rebandaged, "no bandage when recently bandaged");
}

#[tokio::test]
async fn heals_sacrifices_and_procs() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "DRUID4");
    let dr = c.enter("Druidb", Race::NightElf, Class::Druid).await;
    let mut d = Client::connect(&realm, "WARLOCK4");
    let wl = d.enter("Warlockb", Race::Human, Class::Warlock).await;
    let mut e = Client::connect(&realm, "PRIEST4");
    let pr = e.enter("Priestc", Race::Human, Class::Priest).await;
    let mut f = Client::connect(&realm, "SHAMAN4");
    let sh = f.enter("Shamanb", Race::Orc, Class::Shaman).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    let map0 = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, dr, 60, &[18562]);
            let data = m.realm.data.clone();
            let mods = spells::mods::Mods::default();
            let swiftmend = data.spells.get(18562).unwrap().clone();
            let at_self = Targets {
                unit: Some(dr),
                ..Default::default()
            };
            // Swiftmend needs a Rejuvenation or Regrowth, and eats it.
            let without = spells::checks::check(m, dr, &swiftmend, &at_self, None, false, &mods);
            spells::cast_triggered(m, dr, 774, Some(dr));
            let tick = auras::auras(m, dr)
                .and_then(|a| a.holders.iter().find(|h| h.spell == 774))
                .and_then(|h| {
                    h.effects
                        .iter()
                        .find(|x| x.kind == auras::AURA_PERIODIC_HEAL)
                })
                .map_or(0, |x| x.amount);
            hurt(m, dr, 50_000);
            let before = health(m, dr);
            spells::cast_triggered(m, dr, 18562, Some(dr));
            let mended = health(m, dr) - before;
            let eaten = !has(m, dr, 774);
            (without, tick, mended, eaten)
        })
        .await
        .unwrap();
    let (without, tick, mended, eaten) = out;
    let out = map0
        .query(move |m| {
            ready(m, wl, 60, &[]);
            ready(m, pr, 60, &[15237]);
            // Holy Nova's heal comes with its damage.
            hurt(m, pr, 50_000);
            let before = health(m, pr);
            spells::cast(m, pr, 15237, Targets::default(), None);
            let nova = health(m, pr) > before;
            // Demonic Sacrifice: the voidwalker dies for its shield.
            spells::cast_triggered(m, wl, 697, None);
            let demon = pets::pet_of(m, wl);
            if let Some(pet) = demon {
                spells::cast_triggered(m, wl, 18788, Some(pet));
            }
            let sacrificed = (
                demon.is_some_and(|p| !m.unit_view(p).is_some_and(|u| u.alive)),
                has(m, wl, 18790),
            );
            (nova, sacrificed)
        })
        .await
        .unwrap();
    let (nova, sacrificed) = out;
    assert_eq!(
        without,
        Some(spells::fail::TARGET_AURASTATE),
        "no Swiftmend without a heal over time"
    );
    assert!(tick > 0);
    assert!(mended >= tick as u32 * 4, "four ticks of it: {mended}");
    assert!(eaten, "and the Rejuvenation is eaten");
    assert!(nova, "Holy Nova heals its caster");
    assert_eq!(
        sacrificed,
        (true, true),
        "the voidwalker given for its buff"
    );
    let out = map
        .query(move |m| {
            ready(m, sh, 60, &[]);
            let kobold = dummy(m, sh, 60, 3.0);
            let before = health(m, kobold);
            // Flametongue Weapon's proc burns.
            spells::cast_triggered(m, sh, 8026, Some(kobold));
            before - health(m, kobold)
        })
        .await
        .unwrap();
    assert!(out > 0, "Flametongue's attack");
}

#[tokio::test]
async fn auras_targets_and_timers_the_scripts_keep() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "HUNTER3");
    let h = c.enter("Huntera", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, h, 60, &[23989, 3044]);
            let data = m.realm.data.clone();
            let kobold = dummy(m, h, 60, 10.0);
            // Wyvern Sting's sleep, ended, leaves its poison.
            apply(m, h, kobold, 19386);
            let asleep = has(m, kobold, 19386);
            auras::remove_spell(m, kobold, 19386, None);
            let poisoned = has(m, kobold, 24131);
            // Readiness: the hunter's cooldowns are done.
            let later = m.now + Duration::from_secs(30);
            m.players
                .get_mut(&h)
                .unwrap()
                .caster
                .cooldowns
                .insert(3044, later);
            spells::cast_triggered(m, h, 23989, None);
            let ready_again = !m.players[&h].caster.cooldowns.contains_key(&3044);
            // Mongoose Bite: only at whoever opened the window.
            let bite = data.spells.get(1495).unwrap().clone();
            let none = class_scripts::check_target(m, h, &bite, kobold);
            let now = m.now;
            if let Some(c) = m.combat_mut(h) {
                c.defense_window = Some((kobold, now + Duration::from_secs(4)));
            }
            let opened = class_scripts::check_target(m, h, &bite, kobold);
            // Earthbind's first pulse is at once.
            apply(m, h, kobold, 6474);
            let pulse = auras::auras(m, kobold)
                .and_then(|a| a.holders.iter().find(|x| x.spell == 6474))
                .and_then(|x| x.effects.first().and_then(|e| e.next_tick))
                .is_some_and(|t| t <= m.now);
            // Stoneform and Shadowmeld carry their scripts' extra effects.
            apply(m, h, h, 20594);
            let stoneform = auras::auras(m, h).is_some_and(|a| a.dispel_immune(3));
            let shadowmeld = data
                .spells
                .get(20580)
                .is_some_and(|s| s.effects.iter().any(|e| e.aura == 154));
            // Judgement of Command is half on the standing, Hammer of Wrath
            // crits as a blow, Touch of Weakness's damage is its rank's.
            let joc = data.spells.get(20467).unwrap().clone();
            let half = class_scripts::damage_multiplier(m, kobold, &joc);
            let how = data
                .spells
                .get(24275)
                .is_some_and(class_scripts::melee_crit);
            let touch = class_scripts::proc_spell(19261, 28598);
            (
                asleep,
                poisoned,
                ready_again,
                none,
                opened,
                pulse,
                stoneform,
                shadowmeld,
                half,
                how,
                touch,
            )
        })
        .await
        .unwrap();
    let (
        asleep,
        poisoned,
        ready_again,
        none,
        opened,
        pulse,
        stoneform,
        shadowmeld,
        half,
        how,
        touch,
    ) = out;
    assert!(asleep && poisoned, "Wyvern Sting's poison after its sleep");
    assert!(ready_again, "Readiness");
    assert_eq!(none, Some(spells::fail::BAD_TARGETS), "no window, no bite");
    assert_eq!(opened, None, "the one who opened it");
    assert!(pulse, "Earthbind pulses at once");
    assert!(stoneform, "Stoneform's disease immunity");
    assert!(shadowmeld, "Shadowmeld's stealth");
    assert!(
        (half - 0.5).abs() < 0.001,
        "Judgement of Command on the standing"
    );
    assert!(how, "Hammer of Wrath crits as a blow");
    assert_eq!(touch, 19249, "Touch of Weakness by rank");
}

#[tokio::test]
async fn spells_travel_and_ranks_supersede() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "MAGE3");
    let mage = c.enter("Magec", Race::Human, Class::Mage).await;
    let mut d = Client::connect(&realm, "WARRIOR4");
    let w = d.enter("Warrb", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            ready(m, w, 60, &[78, 18182]);
            // Frostbolt flies: nothing at the cast, its damage on arrival.
            m.spells.travel = Some(true);
            let kobold = dummy(m, mage, 60, 20.0);
            let before = health(m, kobold);
            spells::cast_triggered(m, mage, 116, Some(kobold));
            let at_cast = (health(m, kobold), m.spells.in_flight.len());
            m.now += Duration::from_secs(2);
            spells::update(m);
            let arrived = (health(m, kobold) < before, m.spells.in_flight.len());
            m.spells.travel = None;
            // A higher rank takes the lower's place; a talent's in the
            // spellbook too.
            spells::learn(m, w, 284);
            spells::learn(m, w, 18183);
            let book = (
                m.players[&w].spells.contains(&78),
                m.players[&w].spells.contains(&18182),
                m.players[&w].spells.contains(&18183),
            );
            (before, at_cast, arrived, book)
        })
        .await
        .unwrap();
    let (before, at_cast, arrived, book) = out;
    assert_eq!(at_cast, (before, 1), "Frostbolt is on its way");
    assert_eq!(arrived, (true, 0), "and lands");
    assert_eq!(
        book,
        (true, false, true),
        "Heroic Strike stays, the talent's rank goes"
    );
    // mangos-zero writes the old rank first; wow_messages calls that field
    // `new_spell_id`.
    let pair = d
        .wait_for("SMSG_SUPERCEDED_SPELL", |m| match m {
            ServerOpcodeMessage::SMSG_SUPERCEDED_SPELL(s) => Some((s.new_spell_id, s.old_spell_id)),
            _ => None,
        })
        .await;
    assert_eq!(pair, (78, 284));
}

#[tokio::test]
async fn the_combat_log_packets() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "LOGGER");
    let g = c.enter("Loggera", Race::Human, Class::Paladin).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let kobold = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let kobold = dummy(m, g, 60, 5.0);
            crate::game::map::visibility::refresh(m, g);
            kobold
        })
        .await
        .unwrap();
    c.sees(kobold).await;
    let out = map
        .query(move |m| {
            // A drain logs what it took.
            spells::cast_triggered(m, g, 3233, Some(kobold));
            // A dispel logs what it took, and Devour Magic heals.
            let buff = plain_aura(990_301, auras::AURA_MOD_STAT, 0);
            let mut buff = buff;
            buff.dispel = 1;
            let _ = auras::apply_spell(
                m,
                kobold,
                kobold,
                &buff,
                &[(0, 5)],
                auras::Context::default(),
            );
            hurt(m, g, 50_000);
            let before = health(m, g);
            spells::cast_triggered(m, g, 19505, Some(kobold));
            let devoured = (!has(m, kobold, 990_301), health(m, g) > before);
            // A tick on one immune to its school.
            spells::cast_triggered(m, kobold, 172, Some(g));
            spells::cast_triggered(m, g, 642, None);
            let now = m.now;
            if let Some(p) = m.players.get_mut(&g) {
                for h in p.auras.holders.iter_mut().filter(|h| h.spell == 172) {
                    for x in h.effects.iter_mut() {
                        x.next_tick = Some(now);
                    }
                }
            }
            let before = health(m, g);
            auras::update(m);
            let untouched = health(m, g) == before;
            (devoured, untouched)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true), "dispelled, and the heal for it");
    assert!(out.1, "Divine Shield's immunity to the tick");
    let drained = c
        .wait_for("SMSG_SPELLLOGEXECUTE", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLLOGEXECUTE(x) if x.spell == 3233 => Some(x.logs.len()),
            _ => None,
        })
        .await;
    assert_eq!(drained, 1);
    let dispelled = c
        .wait_for("SMSG_SPELLDISPELLOG", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLDISPELLOG(x) => Some(x.spells.clone()),
            _ => None,
        })
        .await;
    assert_eq!(dispelled, vec![990_301]);
    let immune = c
        .wait_for("SMSG_SPELLORDAMAGE_IMMUNE", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLORDAMAGE_IMMUNE(x) => Some((x.target, x.id)),
            _ => None,
        })
        .await;
    assert_eq!(immune, (g, 172));
}

#[tokio::test]
async fn vanish_and_blade_flurry() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "ROGUE3");
    let r = c.enter("Roguea", Race::Human, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, r, 60, &[1784, 1856, 13877]);
            // Vanish: the root goes, and the rogue is stealthed.
            let root = plain_aura(990_302, auras::AURA_MOD_ROOT, 0);
            let kobold = dummy(m, r, 60, 3.0);
            let _ = auras::apply_spell(m, kobold, r, &root, &[(0, 1)], auras::Context::default());
            let rooted = has(m, r, 990_302);
            spells::cast_triggered(m, r, 1856, None);
            let vanished = (!has(m, r, 990_302), has(m, r, 1784));
            // Blade Flurry: a blow's damage on another close by.
            auras::remove_spell(m, r, 1784, None);
            apply(m, r, r, 13877);
            let other = dummy(m, r, 60, 4.0);
            let before = health(m, other);
            crate::game::map::procs::trigger(
                m,
                crate::game::map::procs::Event {
                    actor: r,
                    target: kobold,
                    actor_flags: crate::game::map::procs::PROC_DONE_MELEE,
                    target_flags: crate::game::map::procs::PROC_TAKEN_MELEE,
                    ex: crate::game::map::procs::EX_NORMAL_HIT,
                    spell: None,
                    damage: 100,
                    school: 1,
                    offhand: false,
                    ranged: false,
                    extra_attack: false,
                },
            );
            let flurried = before - health(m, other);
            (rooted, vanished, flurried)
        })
        .await
        .unwrap();
    let (rooted, vanished, flurried) = out;
    assert!(rooted);
    assert_eq!(vanished, (true, true), "free, and stealthed");
    assert_eq!(flurried, 100, "Blade Flurry's second blow");
}
