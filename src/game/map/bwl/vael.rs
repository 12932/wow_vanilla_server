//! Vaelastrasz the Corrupt, as mangos-classic's `boss_vaelastrasz` runs
//! him: he lies dying in his hall, at a third of his health. The first
//! living player (not a GM) through its door (area trigger 3626) brings
//! Lord Victor Nefarius to the throne, who corrupts him (two lines and his
//! Red Lightning), and then Vaelastrasz may be spoken to. Pressed twice
//! ("I cannot, Vaelastrasz!", "Vaelastrasz, no!!!") he stands and speaks
//! his three lines, and at the last turns on the lair: hostile, the whole
//! copy his enemy, Essence of the Red on them as he's pulled. He casts as
//! mangos's `CombatAI` has it, Burning Adrenaline among it on a random
//! mana user of those he fights; he yells below 15 percent, and now and
//! then at a kill. Home from a fight, he is at a third of his health again.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use super::*;
use crate::game::data::npc::{GossipMenu, GossipOption};
use crate::game::fields::*;
use crate::game::map::instance_script::{cancel_for, schedule};

pub const AT_VAEL_INTRO: u32 = 3626;

pub const SAY_LINE_1: u32 = 9886;
pub const SAY_LINE_2: u32 = 9887;
pub const SAY_LINE_3: u32 = 9888;
pub const SAY_HALFLIFE: u32 = 9965;
pub const SAY_KILLTARGET: u32 = 9964;
pub const SAY_NEFARIUS_CORRUPT_1: u32 = 9794;
pub const SAY_NEFARIUS_CORRUPT_2: u32 = 9844;

pub const ESSENCE_OF_THE_RED: u32 = 23513;
pub const FLAME_BREATH: u32 = 23461;
pub const FIRE_NOVA: u32 = 23462;
pub const TAIL_SWEEP: u32 = 15847;
pub const BURNING_ADRENALINE_TANK: u32 = 18173;
pub const BURNING_ADRENALINE: u32 = 23620;
pub const CLEAVE: u32 = 19983;
pub const NEFARIUS_CORRUPTION: u32 = 23642;
pub const RED_LIGHTNING: u32 = 19484;

/// The menus, as mangos's gossip builds them: his own text and the plea,
/// then the second text and the cry that starts it.
const GOSSIP_TEXT_VAEL_1: u32 = 7156;
const GOSSIP_TEXT_VAEL_2: u32 = 7256;
pub const GOSSIP_ITEM_VAEL_1: &str =
    "I cannot, Vaelastrasz! Surely something can be done to heal you!";
pub const GOSSIP_ITEM_VAEL_2: &str = "Vaelastrasz, no!!!";
const MENU_1: u32 = BASE + 900;
const MENU_2: u32 = BASE + 901;
const SPEECH_SCRIPT: u32 = BASE + 902;

/// Where Nefarius stands for the corruption (`aNefariusSpawnLoc`).
const NEFARIUS_AT: [f32; 4] = [-7466.16, -1040.80, 412.053, 2.14675];

/// The field the speech's gossip script sets, which starts it.
pub const HOOK_SPEECH: u32 = 6901;
const FACTION_HOSTILE: u32 = 14;
const NPC_FLAG_GOSSIP: u32 = 0x1;
const NPC_FLAG_QUESTGIVER: u32 = 0x2;
const UNIT_FLAG_NOT_SELECTABLE: u32 = 0x0200_0000;
const STAND_DEAD: u32 = 7;

// Timers.
const T_INTRO: u32 = TIMERS + 6200;
const T_SPEECH: u32 = TIMERS + 6210;
const T_ADRENALINE: u32 = TIMERS + 6220;
// What the copy keeps: the intro's step, and its Nefarius.
const INTRO_STEP: u32 = 6200;
const NEFARIUS: u32 = 6200;

fn vael(map: &Map) -> Option<Guid> {
    map.creatures
        .iter()
        .find(|(_, c)| c.entry == NPC_VAELASTRASZ && c.alive())
        .map(|(_, c)| c.guid)
}

fn step(map: &mut Map, g: Guid, command: u32, datalong: [u32; 4]) {
    let s = ScriptStep {
        command,
        datalong,
        ..Default::default()
    };
    scripts::execute(map, &s, g, g);
}

/// Npc flags set (1) or cleared (2).
fn npc_flags(map: &mut Map, g: Guid, flags: u32, mode: u32) {
    step(map, g, 4, [UNIT_NPC_FLAGS as u32, flags, mode, 0]);
}

/// Area trigger 3626, into his hall: the intro, the first time.
pub fn area_trigger(map: &mut Map, player: Guid, id: u32) {
    if id != AT_VAEL_INTRO {
        return;
    }
    let Some(p) = map.players.get(&player) else {
        return;
    };
    if p.gm || p.death != crate::game::map::death::Death::Alive {
        return;
    }
    if get_data(map, VAELASTRASZ) != NOT_STARTED || vael(map).is_none() {
        return;
    }
    // `BeginIntro`.
    set_data(map, VAELASTRASZ, SPECIAL);
    map.instance.scratch.insert(INTRO_STEP, 0);
    schedule(map, T_INTRO, Duration::from_secs(1));
}

fn nefarius(map: &Map) -> Option<Guid> {
    map.instance
        .lists
        .get(&NEFARIUS)
        .and_then(|l| l.first().copied())
        .filter(|g| map.creature(*g).is_some_and(|c| c.alive()))
}

/// The intro's steps, as `HandleIntro`.
fn intro(map: &mut Map) {
    let phase = map.instance.scratch.get(&INTRO_STEP).copied().unwrap_or(0);
    map.instance.scratch.insert(INTRO_STEP, phase + 1);
    let Some(v) = vael(map) else {
        return;
    };
    let next = match phase {
        0 => {
            let [x, y, z, o] = NEFARIUS_AT;
            if let Some(n) = scripts::summon(
                map,
                v,
                NPC_VICTOR_NEFARIUS,
                Vector3d { x, y, z },
                o,
                Some((Duration::from_secs(25), 3)),
            ) {
                if let Some(s) = crate::game::map::event_ai::scripted_mut(map, n) {
                    s.passive = true;
                }
                step(
                    map,
                    n,
                    4,
                    [UNIT_FIELD_FLAGS as u32, UNIT_FLAG_NOT_SELECTABLE, 1, 0],
                );
                map.instance.lists.insert(NEFARIUS, vec![n]);
            }
            Some(1000)
        }
        1 => {
            if let Some(n) = nefarius(map) {
                crate::game::map::spells::cast_triggered(map, n, NEFARIUS_CORRUPTION, Some(v));
                scripts::talk(map, n, SAY_NEFARIUS_CORRUPT_1, Guid::zero(), None);
            }
            Some(14_000)
        }
        2 => {
            if let Some(n) = nefarius(map) {
                scripts::talk(map, n, SAY_NEFARIUS_CORRUPT_2, Guid::zero(), None);
            }
            Some(2000)
        }
        3 => {
            if let Some(n) = nefarius(map) {
                scripts::cast(map, n, v, RED_LIGHTNING, 0);
            }
            // He may be spoken to now.
            npc_flags(map, v, NPC_FLAG_GOSSIP | NPC_FLAG_QUESTGIVER, 1);
            crate::game::map::auras::remove_spell(map, v, NEFARIUS_CORRUPTION, None);
            None
        }
        _ => None,
    };
    if let Some(ms) = next {
        schedule(map, T_INTRO, Duration::from_millis(ms));
    }
}

/// `BeginSpeech`: the second option chosen.
fn begin_speech(map: &mut Map) {
    let Some(v) = vael(map) else {
        return;
    };
    if matches!(get_data(map, VAELASTRASZ), IN_PROGRESS | DONE) {
        return;
    }
    scripts::talk(map, v, SAY_LINE_1, Guid::zero(), None);
    step(map, v, 28, [0, 0, 0, 0]);
    npc_flags(map, v, NPC_FLAG_GOSSIP | NPC_FLAG_QUESTGIVER, 2);
    schedule(map, T_SPEECH, Duration::from_secs(10));
}

/// `HandleSpeech`: his two lines more, and then the fight.
fn speech(map: &mut Map, n: u32) {
    let Some(v) = vael(map) else {
        return;
    };
    match n {
        0 => {
            scripts::talk(map, v, SAY_LINE_2, Guid::zero(), None);
            schedule(map, T_SPEECH + 1, Duration::from_secs(16));
        }
        1 => {
            scripts::talk(map, v, SAY_LINE_3, Guid::zero(), None);
            schedule(map, T_SPEECH + 2, Duration::from_secs(10));
        }
        _ => {
            // Hostile until he respawns, and the whole copy his enemy.
            step(map, v, 22, [FACTION_HOSTILE, 0x1, 0, 0]);
            step(map, v, 49, [1, 0, 0, 0]);
        }
    }
}

/// A field one of the lair's scripts sets to tell the copy to act. Whether
/// it was one.
pub fn hook(map: &mut Map, field: u32) -> bool {
    if field == HOOK_SPEECH {
        begin_speech(map);
        return true;
    }
    false
}

/// A creature's event: his.
pub fn creature_event(map: &mut Map, g: Guid, entry: u32, event: Event) {
    if entry != NPC_VAELASTRASZ {
        return;
    }
    match event {
        // Dying on the floor, until the intro.
        Event::Spawned if get_data(map, VAELASTRASZ) != DONE => {
            step(map, g, 28, [STAND_DEAD, 0, 0, 0]);
            if get_data(map, VAELASTRASZ) != SPECIAL {
                npc_flags(map, g, NPC_FLAG_GOSSIP | NPC_FLAG_QUESTGIVER, 2);
            }
        }
        Event::Aggro => {
            set_data(map, VAELASTRASZ, IN_PROGRESS);
            scripts::cast(map, g, g, ESSENCE_OF_THE_RED, 0);
            cancel_for(map, g);
            schedule_for(map, T_ADRENALINE, g, Duration::from_secs(15));
        }
        Event::Death => set_data(map, VAELASTRASZ, DONE),
        _ => {}
    }
}

/// The copy's timers for him; whether the key was one.
pub fn timer(map: &mut Map, key: u32) -> bool {
    match key {
        T_INTRO => intro(map),
        k if (T_SPEECH..T_SPEECH + 3).contains(&k) => speech(map, k - T_SPEECH),
        _ => return false,
    }
    true
}

/// Burning Adrenaline on one of those he fights at random, a player of
/// mana not burning already; none, he looks again at once.
pub fn creature_timer(map: &mut Map, key: u32, g: Guid) -> bool {
    if key != T_ADRENALINE {
        return false;
    }
    let Some(c) = map.creature(g).filter(|c| c.alive() && c.combat.attacking) else {
        return true;
    };
    let candidates: Vec<Guid> = c
        .threat
        .iter()
        .map(|(u, _)| *u)
        .filter(|u| {
            map.players.get(u).is_some_and(|p| {
                p.death == crate::game::map::death::Death::Alive
                    && p.fields.get_byte(UNIT_FIELD_BYTES_0, 3) == 0
            }) && !crate::game::map::auras::auras(map, *u)
                .is_some_and(|a| a.has_spell(BURNING_ADRENALINE))
        })
        .collect();
    let went = !candidates.is_empty() && {
        let t = candidates[rand::rng().random_range(0..candidates.len())];
        scripts::cast(map, g, t, BURNING_ADRENALINE, 0)
    };
    let again = if went { 15_000 } else { 1000 };
    schedule_for(map, T_ADRENALINE, g, Duration::from_millis(again));
    true
}

/// His timed casts (mangos's `CombatAI`), his yells, his health, and his
/// menus.
pub(super) fn install(b: &mut Build) {
    b.data.scripts.ai_events.remove(&NPC_VAELASTRASZ);
    let v = NPC_VAELASTRASZ;
    b.timer(
        v,
        1,
        BURNING_ADRENALINE_TANK,
        At::Victim,
        45_000,
        (45_000, 45_000),
    );
    b.timer(v, 2, FLAME_BREATH, At::Victim, 11_000, (4000, 8000));
    b.timer(v, 3, FIRE_NOVA, At::Me, 5000, (5000, 5000));
    b.timer(v, 4, CLEAVE, At::Victim, 8000, (15_000, 15_000));
    b.timer(v, 5, TAIL_SWEEP, At::Me, 20_000, (20_000, 20_000));
    // Below 15 percent, once a fight.
    b.event(v, 6, EVENT_HP, 0, [15, 0, 0, 0], vec![say(SAY_HALFLIFE)]);
    // A kill, one time in five.
    b.event_chance(
        v,
        7,
        EVENT_KILL,
        EF_REPEATABLE,
        20,
        [0; 4],
        vec![say(SAY_KILLTARGET)],
    );
    // At a third of his health, spawned and home from a fight.
    for (n, kind) in [(8, EVENT_SPAWNED), (9, EVENT_REACHED_HOME)] {
        b.event(
            v,
            n,
            kind,
            EF_REPEATABLE,
            [0; 4],
            vec![ScriptStep {
                command: crate::game::map::scripts::C_SET_HEALTH_PCT,
                datalong: [30, 0, 0, 0],
                ..Default::default()
            }],
        );
    }
    b.home_fails(v, 90, VAELASTRASZ);
    // His menus.
    let data = &mut *b.data;
    data.npc.gossip_menus.insert(
        MENU_1,
        GossipMenu {
            texts: vec![(GOSSIP_TEXT_VAEL_1, 0)],
            options: vec![GossipOption {
                id: 0,
                text: GOSSIP_ITEM_VAEL_1.to_owned(),
                kind: 1,
                action_menu: MENU_2 as i32,
                ..Default::default()
            }],
        },
    );
    data.scripts.gossip.insert(
        SPEECH_SCRIPT,
        vec![ScriptStep {
            command: 37,
            datalong: [HOOK_SPEECH, 1, 0, 0],
            ..Default::default()
        }],
    );
    data.npc.gossip_menus.insert(
        MENU_2,
        GossipMenu {
            texts: vec![(GOSSIP_TEXT_VAEL_2, 0)],
            options: vec![GossipOption {
                id: 0,
                text: GOSSIP_ITEM_VAEL_2.to_owned(),
                kind: 1,
                action_menu: -1,
                script: SPEECH_SCRIPT,
                ..Default::default()
            }],
        },
    );
    if let Some(t) = data.creature_templates.get_mut(&NPC_VAELASTRASZ) {
        t.gossip_menu_id = MENU_1;
    }
}

fn say(text: u32) -> ScriptStep {
    ScriptStep {
        command: 0,
        dataint: [text as i32, 0, 0, 0],
        ..Default::default()
    }
}
