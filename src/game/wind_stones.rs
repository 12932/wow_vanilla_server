//! The wind stones of Silithus, as vmangos's `go_wind_stone` runs them
//! (Thistle Tea's `GameObjectScript.WindStone`): a player in Twilight's
//! Hammer regalia speaks at a stone, and the summon spell its gossip casts
//! (24734 and its kin, an activate-object effect at the stone) calls one
//! of the Abyssal Council in the stone's place, and the stone is gone
//! until it respawns. A lesser stone calls a templar, a wind stone a duke,
//! a greater stone a royal: the plain challenge one of its rank at random,
//! a crest, signet or scepter the lord of its element. The lord stands
//! (immune to players, as its template has it) for eight seconds: it turns
//! to its summoner after a second and a half, denounces them, then may be
//! fought and attacks them. It goes a minute later unless it is fighting.
//!
//! mangos-classic has no script for the stones.

use rand::RngExt;
use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptStep;
use crate::game::map::Map;

pub const LESSER: [u32; 6] = [180_456, 180_518, 180_529, 180_544, 180_549, 180_564];
pub const STONES: [u32; 3] = [180_461, 180_534, 180_554];
pub const GREATER: [u32; 3] = [180_466, 180_539, 180_559];

pub const CRIMSON_TEMPLAR: u32 = 15_209;
pub const AZURE_TEMPLAR: u32 = 15_211;
pub const HOARY_TEMPLAR: u32 = 15_212;
pub const EARTHEN_TEMPLAR: u32 = 15_307;
pub const DUKE_OF_CYNDERS: u32 = 15_206;
pub const DUKE_OF_FATHOMS: u32 = 15_207;
pub const DUKE_OF_SHARDS: u32 = 15_208;
pub const DUKE_OF_ZEPHYRS: u32 = 15_220;
pub const PRINCE_SKALDRENOX: u32 = 15_203;
pub const HIGH_MARSHAL_WHIRLAXIS: u32 = 15_204;
pub const BARON_KAZUM: u32 = 15_205;
pub const LORD_SKWOL: u32 = 15_305;

pub const TEMPLARS: [u32; 4] = [
    CRIMSON_TEMPLAR,
    AZURE_TEMPLAR,
    HOARY_TEMPLAR,
    EARTHEN_TEMPLAR,
];
pub const DUKES: [u32; 4] = [
    DUKE_OF_CYNDERS,
    DUKE_OF_FATHOMS,
    DUKE_OF_SHARDS,
    DUKE_OF_ZEPHYRS,
];
pub const ROYALS: [u32; 4] = [
    PRINCE_SKALDRENOX,
    HIGH_MARSHAL_WHIRLAXIS,
    BARON_KAZUM,
    LORD_SKWOL,
];

/// Each summon spell and whom it calls: one of a rank, or a lord.
const SUMMONS: [(u32, &[u32]); 15] = [
    (24_734, &TEMPLARS),
    (24_744, &[CRIMSON_TEMPLAR]),
    (24_756, &[HOARY_TEMPLAR]),
    (24_758, &[EARTHEN_TEMPLAR]),
    (24_760, &[AZURE_TEMPLAR]),
    (24_763, &DUKES),
    (24_765, &[DUKE_OF_CYNDERS]),
    (24_768, &[DUKE_OF_ZEPHYRS]),
    (24_770, &[DUKE_OF_SHARDS]),
    (24_772, &[DUKE_OF_FATHOMS]),
    (24_784, &ROYALS),
    (24_786, &[PRINCE_SKALDRENOX]),
    (24_788, &[HIGH_MARSHAL_WHIRLAXIS]),
    (24_789, &[BARON_KAZUM]),
    (24_790, &[LORD_SKWOL]),
];

/// The lines each rank denounces its summoner with, one of four.
const TEMPLAR_TEXTS: [i32; 4] = [10_686, 10_694, 10_695, 10_696];
const DUKE_TEXTS: [i32; 4] = [10_801, 10_802, 10_803, 10_804];
const ROYAL_TEXTS: [i32; 4] = [10_805, 10_806, 10_807, 10_810];

/// Where a wind stone's duke comes: a place of its own, off the stone.
const DUKE_PLACES: [(u32, [f32; 4]); 3] = [
    (180_461, [-7_927.48, 1_935.30, 5.61, 4.76475]),
    (180_534, [-6_998.52, 1_223.02, 9.16, 4.76475]),
    (180_554, [-6_716.82, 1_674.36, 8.51, 4.76475]),
];

/// A lord's time out of a fight, timed or dead (despawn type 1).
pub const LORD_LASTS_MS: u32 = 60_000;
const TIMED_OR_DEAD: i32 = 1;
/// The challenge's steps: the turn, the line, and the fight.
pub const TURN_MS: u32 = 1_500;
pub const TALK_MS: u32 = 1_600;
pub const ATTACK_MS: u32 = 8_000;
const UNIT_FIELD_FLAGS: u32 = 46;
const IMMUNE_TO_PLAYER: u32 = 0x100;
/// The stone's own action, in place of the spell's: gone.
const STONE_GONE: u32 = 15;

/// The challenge scripts, one a rank, among the generic scripts (clear of
/// the world database's and of Thistle Tea's imports).
pub const CHALLENGE: [u32; 3] = [89_100_001, 89_100_002, 89_100_003];

/// The challenges, into the game data.
pub fn install(data: &mut GameData) {
    for (id, texts) in CHALLENGE
        .into_iter()
        .zip([TEMPLAR_TEXTS, DUKE_TEXTS, ROYAL_TEXTS])
    {
        data.scripts.generic.insert(id, challenge(texts));
    }
}

fn challenge(texts: [i32; 4]) -> Vec<ScriptStep> {
    vec![
        ScriptStep {
            command: 35,
            delay_ms: TURN_MS,
            ..Default::default()
        },
        ScriptStep {
            command: 0,
            delay_ms: TALK_MS,
            dataint: texts,
            ..Default::default()
        },
        ScriptStep {
            command: 4,
            delay_ms: ATTACK_MS,
            datalong: [UNIT_FIELD_FLAGS, IMMUNE_TO_PLAYER, 2, 0],
            ..Default::default()
        },
        ScriptStep {
            command: 26,
            delay_ms: ATTACK_MS,
            ..Default::default()
        },
    ]
}

/// Whether `entry` is a wind stone.
pub fn is_stone(entry: u32) -> bool {
    LESSER.contains(&entry) || STONES.contains(&entry) || GREATER.contains(&entry)
}

/// Whom `spell` calls at a stone of `entry`, if it is a stone's and the
/// stone's rank's.
pub fn calls(entry: u32, spell: u32) -> Option<&'static [u32]> {
    let (_, whom) = SUMMONS.iter().find(|(s, _)| *s == spell)?;
    let rank = if LESSER.contains(&entry) {
        &TEMPLARS
    } else if STONES.contains(&entry) {
        &DUKES
    } else if GREATER.contains(&entry) {
        &ROYALS
    } else {
        return None;
    };
    whom.iter().all(|w| rank.contains(w)).then_some(*whom)
}

/// `spell`'s activate-object effect reached `object`, cast by `caster`:
/// a stone's summon calls its lord and the stone goes, in place of the
/// spell's own action (vmangos's `OnActivateBySpell`). Whether it was one.
pub fn activated(map: &mut Map, caster: Guid, object: Guid, spell: u32) -> bool {
    let Some((entry, at, o)) = map
        .gameobject(object)
        .map(|g| (g.entry, g.position, g.orientation))
    else {
        return false;
    };
    let Some(whom) = calls(entry, spell) else {
        return false;
    };
    let lord = whom[rand::rng().random_range(0..whom.len())];
    let [x, y, z, o] = DUKE_PLACES
        .iter()
        .find(|(e, _)| *e == entry)
        .map_or([at.x, at.y, at.z, o], |(_, p)| *p);
    let script = if TEMPLARS.contains(&lord) {
        CHALLENGE[0]
    } else if DUKES.contains(&lord) {
        CHALLENGE[1]
    } else {
        CHALLENGE[2]
    };
    let summon = ScriptStep {
        command: 10,
        datalong: [lord, LORD_LASTS_MS, 0, 0],
        dataint: [0, script as i32, -1, TIMED_OR_DEAD],
        x,
        y,
        z,
        o,
        ..Default::default()
    };
    crate::game::map::scripts::execute(map, &summon, caster, caster);
    crate::game::map::objects::activate(map, caster, object, STONE_GONE);
    true
}
