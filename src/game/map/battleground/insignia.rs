//! Insignia, as Thistle Tea's `Battleground.Insignia` has them: a player
//! fallen in a battleground leaves a body the enemy may take the insignia
//! of (Remove Insignia, vmangos's `SPELL_EFFECT_SKIN_PLAYER_CORPSE`). The
//! fallen is made to release if they haven't; their corpse becomes bones,
//! which they can't take back; the looter finds a little gold on them.

use std::time::Duration;

use rand::RngExt;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::combat::dist;
use crate::game::map::death::{self, Death};

/// A body that can be skinned: a player's, of their insignia.
pub const UNIT_FLAG_SKINNABLE: u32 = 0x0400_0000;
pub const EFFECT_SKIN_PLAYER_CORPSE: u32 = 116;
/// `CORPSE_FIELD_FLAGS`: bones, and insignia to take.
const CORPSE_BONES: u32 = 0x01;
pub const CORPSE_INSIGNIA: u32 = 0x20;
const CORPSE_LOOTABLE: u32 = 0x01;
const RANGE: f32 = 10.0;
/// How long bones lie.
pub const BONES_LIE: Duration = Duration::from_secs(3600);

/// A player falls in the match: theirs is a body the enemy can take the
/// insignia of, until they rise or release.
pub fn fallen(map: &mut Map, g: Guid) {
    if map.battleground.is_none() {
        return;
    }
    if let Some(p) = map.players.get_mut(&g) {
        p.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE, true);
    }
}

/// The gold on a fallen player of `level`, by a roll of 50 to 150.
pub fn gold(level: u32, roll: u32) -> u32 {
    (roll as f64 * 0.016 * (f64::from(level.clamp(1, 60)) / 5.76).powf(2.5)) as u32
}

/// Remove Insignia on `target` (a fallen player, or their corpse) by
/// `looter`.
pub fn remove(map: &mut Map, looter: Guid, target: Option<Guid>) {
    let Some(target) = target else {
        return;
    };
    let Some(m) = map.battleground.as_ref() else {
        return;
    };
    let Some(side) = m.team_of(looter) else {
        return;
    };
    let Some(at) = map
        .players
        .get(&looter)
        .filter(|p| p.death == Death::Alive)
        .map(|p| p.position())
    else {
        return;
    };
    let victim = if guid::is_player(target) {
        target
    } else {
        match map.corpses.get(&target) {
            Some(c) => c.owner,
            None => return,
        }
    };
    if victim == looter || m.team_of(victim).is_none_or(|t| t == side) {
        return;
    }
    // Not yet released: they are made to.
    let body = map.players.get(&victim).and_then(|p| match p.death {
        Death::Dead { .. } if p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_SKINNABLE) => {
            Some(p.position())
        }
        _ => None,
    });
    if let Some(b) = body {
        if dist(b, at) > RANGE {
            return;
        }
        death::release(map, victim);
    }
    let corpse = guid::corpse(guid::counter(victim));
    let Some((pos, o, flags)) = map.corpses.get(&corpse).map(|c| {
        (
            c.position,
            c.orientation,
            c.fields.get_u32(CORPSE_FIELD_FLAGS),
        )
    }) else {
        return;
    };
    if flags & CORPSE_INSIGNIA == 0 || flags & CORPSE_BONES != 0 || dist(pos, at) > RANGE {
        return;
    }
    let data = map.realm.data.clone();
    let Some(p) = map.players.get_mut(&victim) else {
        return;
    };
    let level = u32::from(p.level());
    // Bones, where the corpse was: theirs no more.
    let mut bones = death::Corpse::new(&data, p, pos, o);
    p.row.state.corpse = None;
    p.outbound.send(&SMSG_PLAYER_SKINNED {
        spirit_released: true,
    });
    p.outbound.send(&MSG_CORPSE_QUERY_Server {
        result: MSG_CORPSE_QUERY_Server_CorpseQueryResult::NotFound,
    });
    map.remove_corpse(corpse);
    let counter = map.next_creature_counter;
    map.next_creature_counter += 1;
    let g = guid::corpse(0x8000_0000 | counter);
    bones.guid = g;
    bones.fields.set_guid(OBJECT_FIELD_GUID, g);
    bones
        .fields
        .set_u32(CORPSE_FIELD_FLAGS, CORPSE_BONES | 0x04);
    bones
        .fields
        .set_u32(CORPSE_FIELD_DYNAMIC_FLAGS, CORPSE_LOOTABLE);
    for slot in 0..19u16 {
        bones.fields.set_u32(CORPSE_FIELD_ITEM + slot, 0);
    }
    map.add_corpse(bones);
    let lies = map.now + BONES_LIE;
    if let Some(m) = map.battleground.as_mut() {
        m.bones.push((lies, g));
    }
    let gold = gold(level, rand::rng().random_range(50..=150));
    crate::game::map::loot::show_item_loot(
        map,
        looter,
        g,
        crate::game::map::loot::Loot {
            gold,
            owner: Some(looter),
            kind: crate::game::map::loot::LootKind::Pickpocket,
            ..Default::default()
        },
    );
}

/// Bones that have lain long enough go.
pub fn update(map: &mut Map) {
    let now = map.now;
    let Some(m) = map.battleground.as_mut() else {
        return;
    };
    if m.bones.iter().all(|(t, _)| *t > now) {
        return;
    }
    let (gone, rest): (Vec<_>, Vec<_>) = m.bones.drain(..).partition(|(t, _)| *t <= now);
    m.bones = rest;
    for (_, g) in gone {
        map.remove_corpse(g);
    }
}
