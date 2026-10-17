//! GM tickets, as Thistle Tea's `Core.GmTicket` keeps them: one open ticket
//! a character, with its category, text and where its writer stood. A GM
//! lists, reads and answers them with `.ticket`; an answered ticket shows
//! its answer and can only be replaced. Each is kept as a record; surveys
//! are logged.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::guid;
use crate::game::map::Map;

pub const KIND_TICKET: &str = "ticket";
const DAY: f32 = 86_400.0;
const READ_ONLY: &str = "This ticket has already been completed. Please open a new one.";
const ANSWERED: &str = "Your ticket has been edited with a response.";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ticket {
    pub id: u32,
    /// The writer's GUID counter.
    pub player: u32,
    pub name: String,
    pub category: u8,
    pub text: String,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Unix seconds.
    pub created: i64,
    pub modified: i64,
    pub response: String,
    pub completed: bool,
    pub viewed: bool,
}

impl Ticket {
    /// What the writer sees: their text, and an answer below it once done.
    pub fn display(&self) -> String {
        if !self.completed {
            return self.text.clone();
        }
        let answer = if self.response.is_empty() {
            String::new()
        } else {
            format!("\n    GM answer:\n{}", self.response)
        };
        format!(
            "{}\n\n-----------------------------------------------------------------------\nCustomer ticket #{} completed.\n{answer}",
            self.text, self.id
        )
    }
}

#[derive(Debug, Default)]
pub struct Tickets {
    pub tickets: AHashMap<u32, Ticket>,
    next_id: u32,
    /// When the queue last changed.
    pub changed: i64,
}

impl Tickets {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut t = Self::default();
        for (kind, _, data) in records.iter().filter(|(k, _, _)| k == KIND_TICKET) {
            let _ = kind;
            if let Ok(ticket) = serde_json::from_str::<Ticket>(data) {
                t.next_id = t.next_id.max(ticket.id);
                t.tickets.insert(ticket.id, ticket);
            }
        }
        t
    }

    pub fn of(&self, player: u32) -> Option<&Ticket> {
        self.tickets.values().find(|t| t.player == player)
    }

    fn oldest_open(&self) -> Option<i64> {
        self.tickets
            .values()
            .filter(|t| !t.completed)
            .map(|t| t.created)
            .min()
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn save(realm: &Realm, t: &Ticket) {
    realm.store.put_record(KIND_TICKET, u64::from(t.id), t);
}

fn days(since: i64, now: i64) -> f32 {
    (now - since).max(0) as f32 / DAY
}

/// `SMSG_GMTICKET_GETTICKET`: the player's ticket, or none.
fn ticket_message(realm: &Realm, player: u32) -> SMSG_GMTICKET_GETTICKET {
    let tickets = realm.social.tickets.lock();
    let now = now();
    match tickets.of(player) {
        Some(t) => SMSG_GMTICKET_GETTICKET {
            status: SMSG_GMTICKET_GETTICKET_GmTicketStatus::HasText {
                text: t.display(),
                ticket_type: GmTicketType::try_from(t.category).unwrap_or(GmTicketType::Stuck),
                days_since_ticket_creation: days(t.modified, now),
                days_since_oldest_ticket_creation: days(tickets.oldest_open().unwrap_or(now), now),
                days_since_last_updated: days(tickets.changed.max(t.created), now),
                escalation_status: GmTicketEscalationStatus::GmticketAssignedtogmStatusNotAssigned,
                read_by_gm: t.viewed,
            },
        },
        None => SMSG_GMTICKET_GETTICKET {
            status: SMSG_GMTICKET_GETTICKET_GmTicketStatus::Default,
        },
    }
}

pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        C::CMSG_GMTICKET_GETTICKET => show(map, g),
        C::CMSG_GMTICKET_SYSTEMSTATUS => {
            if let Some(p) = map.players.get(&g) {
                p.outbound.send(&SMSG_GMTICKET_SYSTEMSTATUS {
                    will_accept_tickets: GmTicketQueueStatus::Enabled,
                });
            }
        }
        C::CMSG_GMTICKET_CREATE(m) => create(
            map,
            g,
            category_of(&m.category),
            m.map.as_int(),
            m.position,
            m.message,
        ),
        C::CMSG_GMTICKET_UPDATETEXT(m) => update(map, g, m.ticket_type.as_int(), m.message),
        C::CMSG_GMTICKET_DELETETICKET => abandon(map, g),
        C::CMSG_GMSURVEY_SUBMIT(m) => {
            let name = map
                .players
                .get(&g)
                .map(|p| p.name.to_string())
                .unwrap_or_default();
            tracing::info!(
                "GM survey {} from {name}: {:?} {}",
                m.survey_id,
                m.questions,
                m.answer_comment
            );
        }
        other => return Some(other),
    }
    None
}

fn category_of(c: &CMSG_GMTICKET_CREATE_GmTicketType) -> u8 {
    use CMSG_GMTICKET_CREATE_GmTicketType as T;
    match c {
        T::Stuck => 1,
        T::BehaviorHarassment { .. } => 2,
        T::Guild => 3,
        T::Item => 4,
        T::Environmental => 5,
        T::NonquestCreep => 6,
        T::QuestQuestnpc => 7,
        T::Technical => 8,
        T::AccountBilling => 9,
        T::Character => 10,
    }
}

fn show(map: &Map, g: Guid) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let msg = ticket_message(&map.realm, guid::counter(g));
    let answered = matches!(
        &msg.status,
        SMSG_GMTICKET_GETTICKET_GmTicketStatus::HasText { text, .. } if text.contains("completed.")
    );
    p.outbound.send(&msg);
    if answered {
        map.system_message(g, ANSWERED);
    }
}

fn create(map: &mut Map, g: Guid, category: u8, at_map: u32, at: Vector3d, text: String) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let name = p.name.to_string();
    let response = {
        let mut tickets = map.realm.social.tickets.lock();
        let player = guid::counter(g);
        let open = tickets.of(player).is_some_and(|t| !t.completed);
        if open || !(1..=10).contains(&category) {
            GmTicketResponse::CreateError
        } else {
            // A completed ticket is replaced.
            let done: Vec<u32> = tickets
                .tickets
                .values()
                .filter(|t| t.player == player)
                .map(|t| t.id)
                .collect();
            for id in done {
                tickets.tickets.remove(&id);
                map.realm.store.delete_record(KIND_TICKET, u64::from(id));
            }
            tickets.next_id += 1;
            let now = now();
            let t = Ticket {
                id: tickets.next_id,
                player,
                name,
                category,
                text,
                map: at_map,
                x: at.x,
                y: at.y,
                z: at.z,
                created: now,
                modified: now,
                ..Default::default()
            };
            save(&map.realm, &t);
            tickets.tickets.insert(t.id, t);
            tickets.changed = now;
            GmTicketResponse::CreateSuccess
        }
    };
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_GMTICKET_CREATE { response });
    }
}

fn update(map: &mut Map, g: Guid, category: u8, text: String) {
    let player = guid::counter(g);
    let outcome = {
        let mut tickets = map.realm.social.tickets.lock();
        let id = tickets.of(player).map(|t| t.id);
        match id.and_then(|id| tickets.tickets.get_mut(&id)) {
            Some(t) if t.completed => Some(false),
            Some(t) => {
                t.text = text;
                t.category = category;
                t.modified = now();
                save(&map.realm, t);
                Some(true)
            }
            None => None,
        }
    };
    let response = if outcome == Some(true) {
        GmTicketResponse::UpdateSuccess
    } else {
        GmTicketResponse::UpdateError
    };
    if outcome == Some(false) {
        map.system_message(g, READ_ONLY);
        show(map, g);
    }
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_GMTICKET_UPDATETEXT { response });
    }
}

fn abandon(map: &mut Map, g: Guid) {
    let player = guid::counter(g);
    let gone = {
        let mut tickets = map.realm.social.tickets.lock();
        let id = tickets.of(player).map(|t| t.id);
        let gone = id.and_then(|id| tickets.tickets.remove(&id));
        if gone.is_some() {
            tickets.changed = now();
        }
        gone
    };
    let Some(t) = gone else {
        return;
    };
    map.realm.store.delete_record(KIND_TICKET, u64::from(t.id));
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_GMTICKET_DELETETICKET {
            response: GmTicketResponse::TicketDeleted,
        });
        p.outbound.send(&SMSG_GMTICKET_GETTICKET {
            status: SMSG_GMTICKET_GETTICKET_GmTicketStatus::Default,
        });
    }
}

/// `.ticket list`, `.ticket <id>`, `.ticket respond <id> <answer>`,
/// `.ticket close <id>`.
pub fn command(map: &mut Map, _gm: Guid, args: &[&str]) -> String {
    let realm = map.realm.clone();
    match args {
        [] | ["list"] => {
            let tickets = realm.social.tickets.lock();
            let mut open: Vec<&Ticket> =
                tickets.tickets.values().filter(|t| !t.completed).collect();
            open.sort_by_key(|t| t.id);
            if open.is_empty() {
                return "No open tickets.".into();
            }
            open.iter()
                .map(|t| format!("#{} {} ({}): {}", t.id, t.name, t.category, t.text))
                .collect::<Vec<_>>()
                .join("\n")
        }
        ["respond", id, answer @ ..] => {
            let Ok(id) = id.parse::<u32>() else {
                return "Use: .ticket respond <id> <answer>".into();
            };
            let player = {
                let mut tickets = realm.social.tickets.lock();
                let Some(t) = tickets.tickets.get_mut(&id) else {
                    return format!("No ticket #{id}.");
                };
                t.response = answer.join(" ");
                t.completed = true;
                t.viewed = true;
                t.modified = now();
                save(&realm, t);
                let player = t.player;
                tickets.changed = now();
                player
            };
            // The writer, if online, sees the answer.
            if let Some(o) = realm.registry.online(guid::player(player)) {
                o.outbound.send(&ticket_message(&realm, player));
                crate::game::map::chat::send_system(&o.outbound, ANSWERED);
            }
            format!("Ticket #{id} answered.")
        }
        ["close", id] => {
            let Ok(id) = id.parse::<u32>() else {
                return "Use: .ticket close <id>".into();
            };
            let gone = realm.social.tickets.lock().tickets.remove(&id);
            match gone {
                Some(t) => {
                    realm.store.delete_record(KIND_TICKET, u64::from(id));
                    if let Some(o) = realm.registry.online(guid::player(t.player)) {
                        o.outbound.send(&SMSG_GMTICKET_DELETETICKET {
                            response: GmTicketResponse::TicketDeleted,
                        });
                    }
                    format!("Ticket #{id} closed.")
                }
                None => format!("No ticket #{id}."),
            }
        }
        [id] => {
            let Ok(id) = id.parse::<u32>() else {
                return "Use: .ticket list|<id>|respond <id> <answer>|close <id>".into();
            };
            let mut tickets = realm.social.tickets.lock();
            match tickets.tickets.get_mut(&id) {
                Some(t) => {
                    t.viewed = true;
                    save(&realm, t);
                    format!(
                        "#{} by {} at map {} ({:.1}, {:.1}, {:.1}): {}",
                        t.id, t.name, t.map, t.x, t.y, t.z, t.text
                    )
                }
                None => format!("No ticket #{id}."),
            }
        }
        _ => "Use: .ticket list|<id>|respond <id> <answer>|close <id>".into(),
    }
}
