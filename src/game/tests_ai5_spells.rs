//! Creature spell lists, script casts and what a creature's spells summon,
//! as Thistle Tea runs them (the EventAI audit's E1, E2, E3, E11, E14 and
//! S19), on the real world database and spells.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::scripts::ScriptTable;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::combat::dist;
use crate::game::map::spells::lists;
use crate::game::map::{Map, MapHandle, ai, auras, event_ai, scripts, spells};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const PROTECTOR: u32 = 487;
const EVOKER: u32 = 1729;
const ROGUE_WIZARD: u32 = 474;
const NETTER: u32 = 513;
const CORE_HOUND: u32 = 11673;
const COBRAHN: u32 = 3669;
const FIRELORD: u32 = 11668;
const LAVA_SPAWN: u32 = 12265;
const TAERAR: u32 = 14890;
const SHADE: u32 = 15302;

fn realm() -> Option<Arc<Realm>> {
    let dir = DB.as_path();
    if !dir.join("vmangos.sqlite").exists() {
        return None;
    }
    Some(harness::realm_with(crate::game::data::load::load(dir)))
}

/// A player at the start, out of the intro, whom nothing aggroes.
async fn player(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.fields.set_flag(
                PLAYER_FLAGS,
                crate::game::map::player::PLAYER_FLAGS_GM,
                true,
            );
        }
    })
    .await
    .unwrap();
    (c, g, map)
}

fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

/// A creature of `entry` at `dx`, `dy` from the start, nobody's summons.
fn spawn(m: &mut Map, entry: u32, dx: f32, dy: f32) -> Guid {
    let g = scripts::summon(m, Guid::zero(), entry, at(dx, dy), 0.0, None).expect("the creature");
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.summoner = None;
    }
    g
}

/// A Stormwind protector to fight: it stands, swings at nothing, casts
/// nothing, and lasts.
fn dummy(m: &mut Map, dx: f32, dy: f32) -> Guid {
    let g = spawn(m, PROTECTOR, dx, dy);
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.passive = true;
        s.no_melee = true;
        s.no_combat_movement = true;
        s.spell_list = Some(0);
    }
    if let Some(c) = m.creature_mut(g) {
        c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
        c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
        c.base = crate::game::map::derive::UnitBase::capture(&c.fields, c.speeds.run);
    }
    g
}

fn clear(map: &MapHandle, gone: Vec<Guid>) {
    map.call(move |m| {
        for g in gone {
            scripts::despawn(m, g);
        }
    });
}

/// The lists load with their flags, target parameters and scripts, their
/// delays from seconds to milliseconds.
fn the_lists_load_whole(realm: &Arc<Realm>) {
    let lists = &realm.data.spells.creature_spells;
    let evoker = &lists[&17290];
    assert_eq!(
        (evoker[0].spell, evoker[0].target, evoker[0].flags),
        (11829, 1, 0x20)
    );
    assert_eq!(
        (evoker[0].delay_initial, evoker[0].delay_repeat),
        ((0, 1000), (3000, 17_000))
    );
    assert_eq!(
        (evoker[1].spell, evoker[1].target, evoker[1].flags),
        (4979, 0, 0x1)
    );
    assert_eq!(evoker[1].delay_repeat, (42_000, 63_000));
    assert_eq!(lists[&4740][0].flags, lists::CF_MAIN_RANGED);
    let raise = lists[&104400].iter().find(|e| e.spell == 17473).unwrap();
    assert_eq!((raise.script, raise.flags), (17473, 0x20));
    assert!(
        lists
            .values()
            .flatten()
            .any(|e| e.target == 19 && e.target_param2 == 4979),
        "a buff list's own buff"
    );
}

/// Defias Evoker: Flamestrike (aura not present) then Quick Flame Ward on
/// itself, each on its own timer. Out of range, Flamestrike is passed over
/// with its timer as it was, and the ward goes; back in range, Flamestrike
/// goes at once and waits three to seventeen seconds.
async fn the_evoker_casts_both_its_spells(a: &mut Client, map: &MapHandle) {
    a.clear();
    let (evoker, foe) = map
        .query(|m| (spawn(m, EVOKER, 20.0, 15.0), dummy(m, 20.0, 55.0)))
        .await
        .unwrap();
    a.sees(evoker).await;
    let (casts, waits) = map
        .query(move |m| {
            ai::attacked(m, evoker, foe);
            let t0 = m.now;
            let mut casts = Vec::new();
            let mut waits = Vec::new();
            for step in 0..4u32 {
                let now = t0 + Duration::from_millis(1300 * u64::from(step));
                m.now = now;
                // Back within reach for the third try.
                if step == 2 {
                    scripts::place(m, foe, at(20.0, 25.0), 0.0);
                }
                spells::creature_cast(m, evoker, foe);
                let c = m.creature_mut(evoker).unwrap();
                if let Some(x) = c.caster.casting.take() {
                    casts.push((x.spell, x.targets.unit));
                    let s = c.caster.spell_list.as_deref().unwrap();
                    let i = usize::from(x.spell == 4979);
                    waits.push(s.ready_at[i].duration_since(now));
                }
            }
            (casts, waits)
        })
        .await
        .unwrap();
    assert_eq!(
        casts,
        vec![(4979, Some(evoker)), (11829, Some(foe))],
        "the ward while Flamestrike is out of range, then Flamestrike"
    );
    assert!(
        (Duration::from_secs(42)..=Duration::from_secs(63)).contains(&waits[0]),
        "{waits:?}"
    );
    assert!(
        (Duration::from_secs(3)..=Duration::from_secs(17)).contains(&waits[1]),
        "{waits:?}"
    );
    // Both went out to those near.
    for spell in [4979, 11829] {
        a.wait_for("the evoker's SMSG_SPELL_START", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_START(s) if s.caster == evoker && s.spell == spell => {
                Some(())
            }
            _ => None,
        })
        .await;
    }
    clear(map, vec![evoker, foe]);
}

/// Defias Rogue Wizard: Frostbolt is its main-ranged spell. When it goes
/// the wizard stops its attack and stands at range; when it can't (out of
/// mana) the wizard takes up the chase again.
async fn the_rogue_wizard_holds_at_range(a: &mut Client, map: &MapHandle) {
    a.clear();
    let (wizard, foe) = map
        .query(|m| (spawn(m, ROGUE_WIZARD, -20.0, 10.0), dummy(m, -20.0, 30.0)))
        .await
        .unwrap();
    a.sees(wizard).await;
    map.call(move |m| ai::attacked(m, wizard, foe));
    a.wait_for("the wizard's SMSG_ATTACKSTOP", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTOP(s) if s.player == wizard && s.enemy == foe => Some(()),
        _ => None,
    })
    .await;
    a.wait_for("the wizard's Frostbolt", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_START(s) if s.caster == wizard && s.spell == 13322 => {
            Some(())
        }
        _ => None,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (moved, holding, swinging) = map
        .query(move |m| {
            let c = m.creature(wizard).unwrap();
            (
                dist(c.position, at(-20.0, 10.0)),
                lists::holding(m, wizard),
                event_ai::melee(m, wizard) || event_ai::combat_movement(m, wizard),
            )
        })
        .await
        .unwrap();
    assert!(
        moved < 0.5 && holding && !swinging,
        "{moved} {holding} {swinging}"
    );
    // Out of mana: the next try fails, and it chases.
    a.clear();
    map.call(move |m| {
        spells::interrupt(m, wizard);
        if let Some(f) = m.unit_fields_mut(wizard) {
            f.set_u32(UNIT_FIELD_POWER1, 0);
        }
    });
    a.wait_for("the wizard's SMSG_ATTACKSTART", |m| match m {
        ServerOpcodeMessage::SMSG_ATTACKSTART(s) if s.attacker == wizard && s.victim == foe => {
            Some(())
        }
        _ => None,
    })
    .await;
    let mut closed = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let d = map
            .query(move |m| {
                let (w, f) = (m.position_of(wizard)?, m.position_of(foe)?);
                Some(dist(w, f))
            })
            .await
            .flatten()
            .unwrap_or(f32::MAX);
        if d < 15.0 {
            closed = true;
            break;
        }
    }
    assert!(closed, "it came on");
    clear(map, vec![wizard, foe]);
}

/// Murloc Netter's Net checks its result: cast out of range it fails, and
/// the event is due again at once rather than in twenty seconds; in range
/// it goes, and waits its repeat.
async fn a_failed_net_is_tried_again(map: &MapHandle) {
    let (first, second, netted) = map
        .query(|m| {
            let foe = dummy(m, -50.0, 30.0);
            let netter = spawn(m, NETTER, -50.0, 0.0);
            ai::attacked(m, netter, foe);
            if let Some(s) = event_ai::scripted_mut(m, netter) {
                s.no_combat_movement = true;
            }
            let i = m.realm.data.scripts.ai_events[&NETTER]
                .iter()
                .position(|e| e.id == 51305)
                .unwrap();
            let due = |m: &mut Map| {
                let now = m.now;
                let s = event_ai::scripted_mut(m, netter).unwrap();
                s.timers[i] = Some(now);
                s.next_tick = None;
                event_ai::tick(m, netter);
                let s = event_ai::scripted(m, netter).unwrap();
                (
                    s.timers[i].map(|t| t.saturating_duration_since(now)),
                    s.disabled[i],
                )
            };
            let first = due(m);
            scripts::place(m, foe, at(-50.0, 10.0), 0.0);
            let second = due(m);
            let netted = auras::auras(m, foe).is_some_and(|a| a.has_spell(12024));
            scripts::despawn(m, netter);
            scripts::despawn(m, foe);
            (first, second, netted)
        })
        .await
        .unwrap();
    assert_eq!(first, (Some(Duration::ZERO), false), "due again at once");
    assert!(
        second.0.is_some_and(|d| d >= Duration::from_millis(20_500)) && netted,
        "{second:?} {netted}"
    );
}

/// Ancient Core Hound's Serrated Bite is only in melee (0x40, vmangos's
/// meaning); 0x80 only out of it, 0x10 and 0x100 never; a cast its checks
/// stop (out of range) fails.
async fn script_cast_flags_mean_what_vmangos_means(map: &MapHandle) {
    let r = map
        .query(|m| {
            let hound = spawn(m, CORE_HOUND, 60.0, -20.0);
            let foe = dummy(m, 60.0, -10.0);
            let (h, f) = (m.unit_view(hound).unwrap(), m.unit_view(foe).unwrap());
            let melee = crate::game::map::combat::melee_range(&h, &f) + 1.0;
            scripts::place(m, foe, at(60.0, -20.0 + melee + 1.5), 0.0);
            let near_only = scripts::cast(m, hound, foe, 19771, lists::CF_ONLY_IN_MELEE);
            let off = !auras::auras(m, foe).is_some_and(|a| a.has_spell(19771));
            let far_only = scripts::cast(m, hound, foe, 19771, lists::CF_NOT_IN_MELEE);
            auras::remove_spell(m, foe, 19771, None);
            scripts::place(m, foe, at(60.0, -18.0), 0.0);
            let in_melee = scripts::cast(m, hound, foe, 19771, lists::CF_ONLY_IN_MELEE);
            let bitten = auras::auras(m, foe).is_some_and(|a| a.has_spell(19771));
            let not_in_melee = scripts::cast(m, hound, foe, 19771, lists::CF_NOT_IN_MELEE);
            let unreachable = scripts::cast(m, hound, foe, 19771, lists::CF_TARGET_UNREACHABLE);
            let casting = scripts::cast(m, hound, foe, 19771, lists::CF_TARGET_CASTING);
            scripts::place(m, foe, at(60.0, 20.0), 0.0);
            let out_of_range = scripts::cast(m, hound, foe, 19771, 0);
            scripts::despawn(m, hound);
            scripts::despawn(m, foe);
            [
                near_only,
                !off,
                !far_only,
                !in_melee,
                !bitten,
                not_in_melee,
                unreachable,
                casting,
                out_of_range,
            ]
        })
        .await
        .unwrap();
    assert_eq!(r, [false; 9], "{r:?}");
}

/// Lord Cobrahn's serpent form sets his other list; out of the fight it
/// gives way to his template's, and his timers go.
async fn a_scripts_list_goes_with_the_fight(map: &MapHandle) {
    let (during, after) = map
        .query(|m| {
            let foe = dummy(m, 30.0, -40.0);
            let cobrahn = spawn(m, COBRAHN, 30.0, -30.0);
            ai::attacked(m, cobrahn, foe);
            spells::creature_cast(m, cobrahn, foe);
            scripts::start(m, ScriptTable::CreatureAi, 366903, cobrahn, foe);
            let during = lists::list_of(m, cobrahn);
            ai::enter_evade(m, cobrahn);
            let c = m.creature(cobrahn).unwrap();
            let after = (
                lists::list_of(m, cobrahn),
                c.scripted.as_deref().and_then(|s| s.spell_list),
                c.caster.spell_list.is_none(),
            );
            scripts::despawn(m, cobrahn);
            scripts::despawn(m, foe);
            (during, after)
        })
        .await
        .unwrap();
    assert_eq!(during, 36691);
    assert_eq!(after, (36690, None, true));
}

/// Firelord's Summon Lava Spawn (a wild summon): a creature of its own,
/// its summoner's, which his summoned-unit event sets on one he fights; its
/// own EventAI splits it in two, the two summoned by it.
async fn a_lava_spawn_is_its_own(map: &MapHandle) {
    let r = map
        .query(|m| {
            let foe = dummy(m, 0.0, -50.0);
            let firelord = spawn(m, FIRELORD, 0.0, -40.0);
            ai::attacked(m, firelord, foe);
            spells::cast_triggered(m, firelord, 19392, Some(firelord));
            // Its spawned events, and then its summoner hears of it.
            event_ai::update_despawns(m);
            let of = |m: &Map, summoner: Guid| -> Vec<Guid> {
                m.creatures
                    .iter()
                    .filter(|(_, c)| {
                        c.entry == LAVA_SPAWN
                            && c.scripted.as_deref().and_then(|s| s.summoner) == Some(summoner)
                    })
                    .map(|(_, c)| c.guid)
                    .collect()
            };
            let spawned = of(m, firelord);
            let lava = spawned[0];
            let c = m.creature(lava).unwrap();
            let own = (
                spawned.len(),
                c.owner.is_none() && c.pet.is_none(),
                c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
                c.fields.get_u32(UNIT_CREATED_BY_SPELL),
                c.hostile_to_players,
                c.scripted.as_deref().and_then(|s| s.despawn_at).is_some(),
                event_ai::scripted(m, firelord).is_some_and(|s| s.summoned.contains(&lava)),
                (c.combat.attacking, c.combat.victim == foe),
            );
            // Its Split, due: two more of it, and it goes.
            let i = m.realm.data.scripts.ai_events[&LAVA_SPAWN]
                .iter()
                .position(|e| e.id == 1226502)
                .unwrap();
            let now = m.now;
            if let Some(s) = event_ai::scripted_mut(m, lava) {
                s.timers[i] = Some(now);
                s.next_tick = None;
            }
            event_ai::tick(m, lava);
            event_ai::update_despawns(m);
            let split = (m.creature(lava).is_none(), of(m, lava).len());
            let mut gone = of(m, lava);
            gone.extend([firelord, foe]);
            for g in gone {
                scripts::despawn(m, g);
            }
            (own, split)
        })
        .await
        .unwrap();
    assert_eq!(
        r.0,
        (1, true, 54, 19392, true, true, true, (true, true)),
        "its own, and set on his foe"
    );
    assert_eq!(r.1, (true, 2), "split in two");
}

/// Taerar's shades are his guardians: of his faction, with his summoned
/// events as they die; his evade takes those left away, and not what a
/// script summoned for him.
async fn taerars_shades_are_his_guardians(map: &MapHandle) {
    let r = map
        .query(|m| {
            let foe = dummy(m, -40.0, -50.0);
            let taerar = spawn(m, TAERAR, -40.0, -40.0);
            ai::attacked(m, taerar, foe);
            // Not his template's faction, so the shades' taking his shows.
            if let Some(f) = m.unit_fields_mut(taerar) {
                f.set_u32(UNIT_FIELD_FACTIONTEMPLATE, 14);
            }
            event_ai::set_phase(m, taerar, 1);
            for spell in [24841, 24842, 24843] {
                spells::cast_triggered(m, taerar, spell, Some(taerar));
            }
            let shades: Vec<Guid> = m
                .creatures
                .iter()
                .filter(|(_, c)| {
                    c.entry == SHADE
                        && c.scripted.as_deref().and_then(|s| s.summoner) == Some(taerar)
                })
                .map(|(_, c)| c.guid)
                .collect();
            let theirs = shades.iter().all(|s| {
                m.creature(*s).is_some_and(|c| {
                    c.owner.is_none()
                        && c.fields.get_u32(UNIT_FIELD_FACTIONTEMPLATE) == 14
                        && c.fields.get_guid(UNIT_FIELD_SUMMONEDBY) == taerar
                })
            });
            crate::game::map::combat::kill(m, foe, shades[0]);
            let phase = event_ai::phase(m, taerar);
            // Another shade, and a creature a script summoned for him.
            event_ai::set_phase(m, taerar, 1);
            spells::cast_triggered(m, taerar, 24841, Some(taerar));
            let fresh = m
                .creatures
                .iter()
                .find(|(_, c)| {
                    c.entry == SHADE
                        && c.alive()
                        && !shades.contains(&c.guid)
                        && c.scripted.as_deref().and_then(|s| s.summoner) == Some(taerar)
                })
                .map(|(_, c)| c.guid)
                .unwrap();
            let scripted = scripts::summon(m, taerar, SHADE, at(-40.0, -30.0), 0.0, None).unwrap();
            ai::enter_evade(m, taerar);
            let after = (m.creature(fresh).is_none(), m.creature(scripted).is_some());
            let mut gone: Vec<Guid> = shades.clone();
            gone.extend([scripted, taerar, foe]);
            for g in gone {
                scripts::despawn(m, g);
            }
            (shades.len(), theirs, phase, after)
        })
        .await
        .unwrap();
    assert_eq!((r.0, r.1), (3, true), "three of his");
    assert_ne!(r.2, 1, "his summoned-died event moved his phase on");
    assert_eq!(r.3, (true, true), "his guardian went, the script's stayed");
}

#[tokio::test]
async fn creature_spell_lists_and_summons_on_the_real_data() {
    let Some(realm) = realm() else {
        return;
    };
    the_lists_load_whole(&realm);
    let (mut a, _me, map) = player(&realm, "Lister").await;
    the_evoker_casts_both_its_spells(&mut a, &map).await;
    the_rogue_wizard_holds_at_range(&mut a, &map).await;
    a_failed_net_is_tried_again(&map).await;
    script_cast_flags_mean_what_vmangos_means(&map).await;
    a_scripts_list_goes_with_the_fight(&map).await;
    a_lava_spawn_is_its_own(&map).await;
    taerars_shades_are_his_guardians(&map).await;
}
