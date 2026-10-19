//! Queries: names, creatures, items, time. All answered at once, from
//! memory. A name, creature or item answer names no object a create could
//! be waiting for, so it goes past a hold; the time's doesn't, so that a
//! client can take it for the map having sent all it had.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::{
    CMSG_CREATURE_QUERY, CMSG_ITEM_NAME_QUERY, CMSG_ITEM_QUERY_SINGLE, CMSG_NAME_QUERY, Class,
    CreatureFamily, Race, SMSG_CREATURE_QUERY_RESPONSE, SMSG_CREATURE_QUERY_RESPONSE_found,
    SMSG_ITEM_QUERY_SINGLE_RESPONSE, SMSG_NAME_QUERY_RESPONSE, SMSG_PLAYED_TIME,
    SMSG_QUERY_TIME_RESPONSE, item_to_name_query_response, item_to_query_response,
};

use crate::game::conn::Outbound;
use crate::game::data;
use crate::game::map::Map;

pub fn name(map: &Map, out: &Outbound, m: CMSG_NAME_QUERY) {
    let Some(n) = map.realm.registry.name(m.guid) else {
        return;
    };
    out.send_unheld(&SMSG_NAME_QUERY_RESPONSE {
        guid: m.guid,
        character_name: n.name.to_string(),
        realm_name: String::new(),
        race: Race::try_from(n.race).unwrap_or(Race::Human),
        gender: data::gender(n.gender),
        class: Class::try_from(n.class).unwrap_or(Class::Warrior),
    });
}

pub fn creature(map: &Map, out: &Outbound, m: CMSG_CREATURE_QUERY) {
    let found =
        map.realm
            .data
            .creature_template(m.creature)
            .map(|t| SMSG_CREATURE_QUERY_RESPONSE_found {
                name1: t.name.clone(),
                name2: String::new(),
                name3: String::new(),
                name4: String::new(),
                sub_name: t.sub_name.clone(),
                type_flags: t.type_flags,
                creature_type: t.creature_type,
                creature_family: CreatureFamily::try_from(t.family).unwrap_or(CreatureFamily::None),
                creature_rank: t.rank,
                unknown0: 0,
                spell_data_id: 0,
                display_id: t.display_ids.iter().copied().find(|d| *d != 0).unwrap_or(0),
                civilian: u8::from(t.civilian),
                racial_leader: 0,
            });
    out.send_unheld(&SMSG_CREATURE_QUERY_RESPONSE {
        creature_entry: m.creature,
        found,
    });
}

pub fn item(map: &Map, out: &Outbound, m: CMSG_ITEM_QUERY_SINGLE) {
    if let Some(t) = map.realm.data.item(m.item) {
        out.send_frame_unheld(t.query_response());
        return;
    }
    match wow_items::vanilla::lookup_item(m.item) {
        Some(item) => {
            out.send_unheld(&item_to_query_response(item));
        }
        None => {
            out.send_unheld(&SMSG_ITEM_QUERY_SINGLE_RESPONSE {
                item: m.item | 0x8000_0000,
                found: None,
            });
        }
    }
}

pub fn item_name(map: &Map, out: &Outbound, m: CMSG_ITEM_NAME_QUERY) {
    if let Some(t) = map.realm.data.item(m.item) {
        out.send_unheld(
            &wow_world_messages::vanilla::SMSG_ITEM_NAME_QUERY_RESPONSE {
                item: m.item,
                item_name: t.name.clone(),
            },
        );
        return;
    }
    if let Some(item) = wow_items::vanilla::lookup_item(m.item) {
        out.send_unheld(&item_to_name_query_response(item));
    }
}

pub fn time(out: &Outbound) {
    out.send(&SMSG_QUERY_TIME_RESPONSE {
        time: u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0),
    });
}

pub fn played(map: &mut Map, guid: Guid) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&guid) else {
        return;
    };
    p.count_played(now);
    p.outbound.send(&SMSG_PLAYED_TIME {
        total_played_time: p.row.played_total,
        level_played_time: p.row.played_level,
    });
}
