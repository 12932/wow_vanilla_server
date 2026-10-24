//! Spells' second round, against the real data when it is here: indoors
//! and outdoors, mounts, Grounding Totem's magnet, far sight and Mind
//! Vision, ground areas' dynamic objects, Spirit of Redemption, Power
//! Word: Shield's bonus, Eyes of the Beast, food on the move, level-scaled
//! costs and `spell_area`.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, Targets, dynamic, environment, redemption};
use crate::game::map::{Map, MapKey, auras, pets};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

/// A Kobold Vermin at `level`, `yards` in front of `g`, hardy.
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

fn rested(m: &mut Map, g: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    p.caster.casting = None;
    p.caster.gcd_until = None;
    p.caster.cooldowns.clear();
    p.caster.category_cooldowns.clear();
}

/// The cast under way finishes now.
fn finish(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m
        .players
        .get_mut(&g)
        .and_then(|p| p.caster.casting.as_mut())
    {
        c.ends = now;
    }
    spells::update(m);
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
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

#[tokio::test]
#[ignore = "extended"]
async fn outdoors_mounts_magnets_sight_areas_and_shields() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "PRIEST");
    let g = c.enter("Priesta", Race::Human, Class::Priest).await;
    let mut d = Client::connect(&realm, "MAGE");
    let mage = d.enter("Maga", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const HORSE: u32 = 458;
            const SHIELD: u32 = 17;
            const MIND_VISION: u32 = 2096;
            const FAR_SIGHT: u32 = 6196;
            const BLIZZARD: u32 = 10;
            const FROSTBOLT: u32 = 25304;
            ready(m, g, 60, &[HORSE, SHIELD, MIND_VISION]);
            ready(m, mage, 60, &[FAR_SIGHT, BLIZZARD, FROSTBOLT]);
            let data = m.realm.data.clone();
            // Indoors: no horse, and one already up goes.
            m.spells.outdoors_override.insert(g, false);
            let horse = data.spells.get(HORSE).unwrap().clone();
            let indoors = environment::check(m, g, &horse);
            m.spells.outdoors_override.insert(g, true);
            spells::cast(m, g, HORSE, Targets::default(), None);
            finish(m, g);
            let mounted = auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOUNTED));
            m.spells.outdoors_override.insert(g, false);
            environment::reconcile(m, g, m.now);
            let still = auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOUNTED));
            // Mounted, a spell dismounts first.
            m.spells.outdoors_override.insert(g, true);
            rested(m, g);
            spells::cast(m, g, HORSE, Targets::default(), None);
            finish(m, g);
            rested(m, g);
            spells::cast(m, g, SHIELD, Targets::default(), None);
            let dismounted = !auras::auras(m, g).is_some_and(|a| a.has(auras::AURA_MOUNTED));
            // Power Word: Shield: a tenth of healing power more, less for a
            // spell learned at 6.
            let plain = auras::auras(m, g)
                .and_then(|a| a.holders.iter().find(|h| h.spell == SHIELD))
                .and_then(|h| h.effects.first().map(|x| x.amount))
                .unwrap_or(0);
            auras::remove_spell(m, g, SHIELD, None);
            auras::remove_spell(m, g, 6788, None);
            let healing = plain_aura(990_101, auras::AURA_MOD_HEALING_DONE, 0x7f);
            let _ = auras::apply_spell(m, g, g, &healing, &[(0, 1000)], auras::Context::default());
            rested(m, g);
            spells::cast(m, g, SHIELD, Targets::default(), None);
            let shielded = auras::auras(m, g)
                .and_then(|a| a.holders.iter().find(|h| h.spell == SHIELD))
                .and_then(|h| h.effects.first().map(|x| x.amount))
                .unwrap_or(0);
            // Grounding Totem's magnet on the priest: a creature's Frostbolt
            // at her goes to the totem, and the magnet is spent.
            let kobold = dummy(m, g, 60, 10.0);
            let totem = dummy(m, g, 60, 3.0);
            let grounding = data.spells.get(8178).unwrap().clone();
            let _ = auras::apply_spell(
                m,
                totem,
                g,
                &grounding,
                &[(0, 1)],
                auras::Context {
                    charges: Some(1),
                    ..Default::default()
                },
            );
            let before = (
                health(m, g),
                auras::auras(m, totem).map_or(0, |a| a.holders.len()),
            );
            spells::cast_triggered(m, kobold, FROSTBOLT, Some(g));
            let grounded = (
                health(m, g) == before.0,
                auras::auras(m, totem).is_some_and(|a| a.has_spell(FROSTBOLT)),
                auras::auras(m, g).is_some_and(|a| a.has_spell(8178)),
            );
            // Mind Vision: the priest looks through the kobold.
            rested(m, g);
            spells::cast(
                m,
                g,
                MIND_VISION,
                Targets {
                    unit: Some(kobold),
                    ..Default::default()
                },
                None,
            );
            let vision = m.players[&g].fields.get_guid(PLAYER_FARSIGHT);
            auras::remove_spell(m, kobold, MIND_VISION, None);
            let after_vision = m.players[&g].fields.get_guid(PLAYER_FARSIGHT);
            // Far Sight: the mage looks from a focus far off, until it ends.
            let at = {
                let p = m.players[&mage].position();
                Vector3d {
                    x: p.x + 40.0,
                    y: p.y,
                    z: p.z,
                }
            };
            m.spells.outdoors_override.insert(mage, true);
            spells::cast(
                m,
                mage,
                FAR_SIGHT,
                Targets {
                    dest: Some(at),
                    ..Default::default()
                },
                None,
            );
            let now = m.now;
            if let Some(c) = m
                .players
                .get_mut(&mage)
                .and_then(|p| p.caster.casting.as_mut())
            {
                c.ends = now;
            }
            spells::update(m);
            let focus = m.players[&mage].fields.get_guid(PLAYER_FARSIGHT);
            let focus_object = m.spells.dynamic.get(&focus).map(|d| (d.kind, d.position));
            let view = m.players[&mage].misc.view;
            spells::interrupt(m, mage);
            let after_sight = (
                m.players[&mage].fields.get_guid(PLAYER_FARSIGHT),
                m.spells.dynamic.contains_key(&focus),
            );
            // Blizzard: a dynamic object where it falls, which clients near
            // learn of, gone with the channel.
            rested(m, mage);
            let ground = m.creature(kobold).unwrap().position;
            spells::cast(
                m,
                mage,
                BLIZZARD,
                Targets {
                    dest: Some(ground),
                    ..Default::default()
                },
                None,
            );
            let area = m
                .spells
                .dynamic
                .values()
                .find(|d| d.spell == BLIZZARD)
                .map(|d| (d.guid, d.kind, d.radius));
            crate::game::map::visibility::refresh_dirty(m);
            let known = area.is_some_and(|(o, _, _)| m.players[&mage].known.contains(&o));
            spells::interrupt(m, mage);
            let gone = area.is_some_and(|(o, _, _)| !m.spells.dynamic.contains_key(&o));
            (
                indoors,
                mounted,
                still,
                dismounted,
                plain,
                shielded,
                grounded,
                vision,
                after_vision,
                kobold,
                focus,
                focus_object,
                view,
                at,
                after_sight,
                area,
                known,
                gone,
            )
        })
        .await
        .unwrap();
    let (
        indoors,
        mounted,
        still,
        dismounted,
        plain,
        shielded,
        grounded,
        vision,
        after_vision,
        kobold,
        focus,
        focus_object,
        view,
        at,
        after_sight,
        area,
        known,
        gone,
    ) = out;
    assert_eq!(
        indoors,
        Some(environment::ONLY_OUTDOORS),
        "no horse indoors"
    );
    assert!(mounted, "mounted outdoors");
    assert!(!still, "the horse goes indoors");
    assert!(dismounted, "a spell dismounts");
    // 1,000 healing power, a tenth of it, times 0.475 for level 6.
    assert_eq!(shielded - plain, 47, "{plain} to {shielded}");
    assert_eq!(
        grounded,
        (true, true, false),
        "the totem took the Frostbolt"
    );
    assert_eq!(vision, kobold, "Mind Vision looks through the kobold");
    assert!(after_vision.is_zero(), "and stops with its aura");
    assert!(!focus.is_zero(), "Far Sight looks from a focus");
    let (kind, pos) = focus_object.expect("the focus object");
    assert_eq!(kind, dynamic::KIND_FARSIGHT);
    assert!((pos.x - at.x).abs() < 0.01);
    assert!(
        view.is_some_and(|v| (v.x - at.x).abs() < 0.01),
        "the view is the focus's"
    );
    assert_eq!(
        after_sight,
        (Guid::zero(), false),
        "the channel's end ends it"
    );
    let (_, kind, radius) = area.expect("Blizzard's object");
    assert_eq!(kind, dynamic::KIND_AREA_SPELL);
    assert!(radius > 0.0);
    assert!(known, "the mage's client knows it");
    assert!(gone, "gone with the channel");
}

#[tokio::test]
#[ignore = "extended"]
async fn spirit_of_redemption_heals_on_then_dies() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "SPIRIT");
    let g = c.enter("Priesta", Race::Human, Class::Priest).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[redemption::TALENT, 2050, 585]);
            let max = m.players[&g].fields.get_u32(UNIT_FIELD_MAXHEALTH);
            let h = health(m, g);
            crate::game::map::combat::deal_damage(m, g, g, h + 100);
            let spirit = redemption::spirit(m, g);
            let alive = m.players[&g].death == crate::game::map::death::Death::Alive;
            let full = health(m, g) == max;
            // Only heals: Lesser Heal yes, Smite no.
            let data = m.realm.data.clone();
            let mods = spells::mods::Mods::default();
            let heal = data.spells.get(2050).unwrap().clone();
            let smite = data.spells.get(585).unwrap().clone();
            let at_self = Targets {
                unit: Some(g),
                ..Default::default()
            };
            let heals = spells::checks::check(m, g, &heal, &at_self, None, false, &mods);
            let smites = spells::checks::check(m, g, &smite, &at_self, None, false, &mods);
            // The spirit lasts the client's 10 s (Spell.dbc's duration 1 for 27827).
            let now = m.now;
            let lasts = m.players[&g]
                .auras
                .holders
                .iter()
                .find(|h| h.spell == redemption::FORM)
                .and_then(|h| h.expires)
                .map(|e| e.duration_since(now).as_millis());
            // The spirit's time ends.
            for h in m.players.get_mut(&g).unwrap().auras.holders.iter_mut() {
                if redemption::STATE.contains(&h.spell) || h.spell == redemption::FORM {
                    h.expires = Some(now);
                }
            }
            auras::update(m);
            let dead = m.players[&g].death != crate::game::map::death::Death::Alive;
            (spirit, alive, full, heals, smites, dead, lasts)
        })
        .await
        .unwrap();
    let (spirit, alive, full, heals, smites, dead, lasts) = out;
    assert_eq!(lasts, Some(10_000), "the spirit's 10 s, as the client's");
    assert!(
        spirit && alive && full,
        "a spirit at full health: {spirit} {alive} {full}"
    );
    assert_eq!(heals, None, "a spirit heals");
    assert_eq!(
        smites,
        Some(spells::fail::NOT_SHAPESHIFT),
        "but doesn't smite"
    );
    assert!(dead, "and dies when its time is up");
}

#[tokio::test]
async fn eyes_of_the_beast_moves_the_pet_then_gives_it_back() {
    let (_realm, _h, gh, map, pet, _wolf) = crate::game::tests_pets::tamed().await;
    let out = map
        .query(move |m| {
            let before = pets::state(m, pet).map(|s| (s.react, s.command));
            let eyes = plain_aura(990_128, auras::AURA_MOD_POSSESS_PET, 0);
            let _ = auras::apply_spell(m, gh, pet, &eyes, &[(0, 1)], auras::Context::default());
            pets::set_mover(m, gh, pet);
            let on = (
                m.creature(pet).unwrap().fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::pets::UNIT_FLAG_POSSESSED,
                ),
                m.players[&gh].fields.get_guid(PLAYER_FARSIGHT) == pet,
                m.players[&gh].owned.mover == Some(pet),
                pets::state(m, pet).map(|s| s.react) == Some(pets::REACT_PASSIVE),
            );
            auras::remove_spell(m, pet, 990_128, None);
            let off = (
                m.creature(pet).unwrap().fields.has_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::pets::UNIT_FLAG_POSSESSED,
                ),
                m.players[&gh].fields.get_guid(PLAYER_FARSIGHT).is_zero(),
                m.players[&gh].owned.mover.is_none(),
                pets::state(m, pet).map(|s| (s.react, s.command)) == before,
                m.creature(pet).and_then(|c| c.owner) == Some(gh),
            );
            (on, off)
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true, true, true), "the hunter moves the pet");
    assert_eq!(
        out.1,
        (false, true, true, true, true),
        "and it is their pet again"
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn eating_still_level_scaled_costs_and_spell_areas() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "AREAS");
    let g = c.enter("Areana", Race::Human, Class::Warlock).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const FOOD: u32 = 433;
            const MOONSTALKER: u32 = 6298;
            const BLESSING: u32 = 31906;
            const BLESSED: u32 = 30238;
            const DRAUGHT: u32 = 21393;
            ready(m, g, 60, &[FOOD, MOONSTALKER, DRAUGHT]);
            let data = m.realm.data.clone();
            let mods = spells::mods::Mods::default();
            let at_self = Targets {
                unit: Some(g),
                ..Default::default()
            };
            // Food isn't had on the move.
            let food = data.spells.get(FOOD).unwrap().clone();
            m.players.get_mut(&g).unwrap().movement.flags =
                MovementInfo_MovementFlags::new_forward();
            let walking = spells::checks::check(m, g, &food, &at_self, None, false, &mods);
            m.players.get_mut(&g).unwrap().movement.flags = MovementInfo_MovementFlags::empty();
            let still = spells::checks::check(m, g, &food, &at_self, None, false, &mods);
            // A level 60 casting a level 20 spell that scales pays some four
            // times as much; Life Tap costs no more past its last level.
            let flamestrike = data.spells.get(11829).unwrap().clone();
            let scaled = spells::power_cost(m, g, &flamestrike);
            let life_tap = data.spells.get(1455).unwrap().clone();
            let tap = spells::power_cost(m, g, &life_tap);
            let tap_capped = life_tap.mana_cost
                + life_tap.mana_cost_per_level * (life_tap.max_level - life_tap.base_level);
            // A Healing Draught is for battlegrounds.
            let draught = data.spells.get(DRAUGHT).unwrap().clone();
            let outside = spells::checks::check(m, g, &draught, &at_self, None, false, &mods);
            // Form of the Moonstalker is for Darkshore only.
            let moon = data.spells.get(MOONSTALKER).unwrap().clone();
            let away = spells::checks::check(m, g, &moon, &at_self, None, false, &mods);
            let (zone, area) = {
                let p = &m.players[&g];
                (p.zone, p.area)
            };
            m.players.get_mut(&g).unwrap().zone = 148;
            let there = spells::checks::check(m, g, &moon, &at_self, None, false, &mods);
            // Lordaeron's Blessing: had in the Plaguelands by the blessed,
            // gone when they aren't.
            m.players.get_mut(&g).unwrap().zone = 139;
            spells::area::reconcile(m, g);
            let unblessed = auras::auras(m, g).is_some_and(|a| a.has_spell(BLESSING));
            let blessed = plain_aura(BLESSED, auras::AURA_DUMMY, 0);
            let _ = auras::apply_spell(m, g, g, &blessed, &[(0, 1)], auras::Context::default());
            spells::area::reconcile(m, g);
            let had = auras::auras(m, g).is_some_and(|a| a.has_spell(BLESSING));
            {
                let p = m.players.get_mut(&g).unwrap();
                p.zone = zone;
                p.area = area;
            }
            spells::area::reconcile(m, g);
            let left = auras::auras(m, g).is_some_and(|a| a.has_spell(BLESSING));
            (
                walking,
                still,
                scaled,
                flamestrike.mana_cost,
                tap,
                tap_capped,
                away,
                there,
                unblessed,
                had,
                left,
                outside,
            )
        })
        .await
        .unwrap();
    let (walking, still, scaled, base, tap, tap_capped, away, there, unblessed, had, left, outside) =
        out;
    assert_eq!(
        outside,
        Some(spells::fail::ONLY_BATTLEGROUNDS),
        "a draught outside"
    );
    assert_eq!(walking, Some(spells::fail::MOVING), "no eating on the move");
    assert_ne!(still, Some(spells::fail::MOVING), "standing still, food");
    // 260 / (1.117 * 20 / 60 - 0.1327) = 1084.99, truncated.
    assert_eq!(base, 260);
    assert_eq!(scaled, 1084, "a level-scaled cost");
    assert_eq!(tap, tap_capped, "Life Tap's cost stops at its last level");
    assert_eq!(away, Some(spells::fail::REQUIRES_AREA), "not here");
    assert_ne!(there, Some(spells::fail::REQUIRES_AREA), "but in Darkshore");
    assert!(!unblessed, "only the blessed have it");
    assert!(had, "the blessed have it in the Plaguelands");
    assert!(!left, "and lose it leaving");
}

#[tokio::test]
#[ignore = "extended"]
async fn a_meteor_is_shared_among_those_it_falls_on() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "METEOR1");
    let a = c.enter("Meteora", Race::Human, Class::Warrior).await;
    let mut d = Client::connect(&realm, "METEOR2");
    let b = d.enter("Meteorb", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const METEOR: u32 = 28884;
            for g in [a, b] {
                ready(m, g, 60, &[]);
                let p = m.players.get_mut(&g).unwrap();
                p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
                p.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            }
            let at = m.players[&a].position();
            let caster = dummy(m, a, 63, 10.0);
            // B is far off: A takes it all.
            let away = Vector3d {
                x: at.x + 200.0,
                ..at
            };
            m.players.get_mut(&b).unwrap().movement.position = away;
            spells::cast_triggered_at(m, caster, METEOR, at);
            let alone = 1_000_000 - health(m, a);
            // Side by side, they take half each.
            m.players
                .get_mut(&a)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            m.players.get_mut(&b).unwrap().movement.position = at;
            spells::cast_triggered_at(m, caster, METEOR, at);
            let shared = (1_000_000 - health(m, a), 1_000_000 - health(m, b));
            (alone, shared)
        })
        .await
        .unwrap();
    let (alone, (sa, sb)) = out;
    assert!(alone > 10_000, "the meteor fell: {alone}");
    for share in [sa, sb] {
        let half = alone as f32 / 2.0;
        assert!(
            (share as f32 - half).abs() < half * 0.15,
            "half of {alone} each: {sa} {sb}"
        );
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn a_buff_too_high_for_its_target_goes_on_at_a_lower_rank() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "RANK1");
    let g = c.enter("Rankpriest", Race::Human, Class::Priest).await;
    let mut d = Client::connect(&realm, "RANK2");
    let low = d.enter("Ranklow", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const FORTITUDE: [u32; 6] = [1243, 1244, 1245, 2791, 10937, 10938];
            const PRAYER: u32 = 21562;
            ready(m, g, 60, &FORTITUDE);
            ready(m, low, 20, &[]);
            let at_low = Targets {
                unit: Some(low),
                ..Default::default()
            };
            spells::cast(m, g, FORTITUDE[5], at_low, None);
            let on = auras::auras(m, low)
                .map(|a| {
                    FORTITUDE
                        .iter()
                        .copied()
                        .filter(|id| a.has_spell(*id))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            // Prayer of Fortitude (level 48) passes over a level 20 who
            // isn't the one it was cast at.
            let data = m.realm.data.clone();
            let prayer = data.spells.get(PRAYER).unwrap().clone();
            let all = vec![(g, [true, false, false]), (low, [true, false, false])];
            let kept = spells::aura_rank::recipients(m, g, &prayer, &Targets::default(), all);
            let kept: Vec<Guid> = kept.into_iter().map(|(x, _)| x).collect();
            (on, kept)
        })
        .await
        .unwrap();
    let (on, kept) = out;
    assert_eq!(on, vec![1245], "rank 3, for a level 20");
    assert_eq!(kept, vec![g], "the group buff passes over the low one");
}

#[tokio::test]
#[ignore = "extended"]
async fn hallows_end_tricks_treats_and_costumes() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, "HALLOW1");
    let a = c.enter("Hallowa", Race::Human, Class::Warrior).await;
    let mut d = Client::connect(&realm, "HALLOW2");
    let b = d.enter("Hallowb", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            const PIRATE_WAND: u32 = 24717;
            const PIRATE: [u32; 2] = [24708, 24709];
            ready(m, a, 60, &[]);
            ready(m, b, 60, &[]);
            // An innkeeper's Trick or Treat.
            let innkeeper = dummy(m, a, 60, 3.0);
            spells::cast_triggered(m, innkeeper, 24751, Some(a));
            let marked = auras::auras(m, a).is_some_and(|x| x.has_spell(24755));
            // A Hallowed Wand dresses its target as a pirate.
            spells::cast_triggered(m, a, PIRATE_WAND, Some(b));
            let pirate = auras::auras(m, b).is_some_and(|x| PIRATE.iter().any(|p| x.has_spell(*p)));
            (marked, pirate)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true), "tricked or treated, and a pirate");
}
