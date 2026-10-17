//! Mail, as mangos-zero runs it: letters with money, one item, or cash on
//! delivery, from players, the auction house and creatures. Mail with
//! money or an item takes an hour between accounts; mail lasts 30 days
//! (cash on delivery 3), then goes back to its sender or away. Each mail
//! is kept as a record.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::registry::Team;
use crate::game::social::send_to;
use crate::game::store::ItemRow;

pub const KIND_MAIL: &str = "mail";
pub const KIND_TEXT: &str = "text";

pub const POSTAGE: u32 = 30;
/// The mails a mailbox lists.
const MAX_LISTED: usize = 254;
pub const MAX_MAILS: usize = 100;
pub const DAY: i64 = 86_400;
/// The letter a mail's text is copied onto.
pub const LETTER_ITEM: u32 = 8383;

// Mail kinds.
pub const NORMAL: u8 = 0;
pub const AUCTION: u8 = 2;
pub const CREATURE: u8 = 3;

// Flags.
pub const READ: u32 = 0x01;
pub const RETURNED: u32 = 0x02;
pub const COPIED: u32 = 0x04;
pub const COD_PAYMENT: u32 = 0x08;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Mail {
    pub id: u32,
    pub kind: u8,
    /// A character's GUID counter, an auction house, or a creature entry.
    pub sender: u32,
    pub receiver: u32,
    pub subject: String,
    pub body: String,
    pub item: Option<ItemRow>,
    pub money: u32,
    pub cod: u32,
    /// Unix seconds.
    pub deliver_at: i64,
    pub expires_at: i64,
    pub flags: u32,
    pub stationery: u32,
    /// The auction it settles.
    pub auction: u32,
}

#[derive(Debug, Default)]
pub struct Mails {
    pub mails: AHashMap<u32, Mail>,
    /// Texts made into letters, by id.
    pub texts: AHashMap<u32, String>,
    next_id: u32,
    /// Mails due and not yet announced to their receiver.
    pending: Vec<u32>,
}

impl Mails {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut m = Self::default();
        for (kind, id, data) in records {
            match kind.as_str() {
                KIND_MAIL => match serde_json::from_str::<Mail>(data) {
                    Ok(mail) => {
                        m.next_id = m.next_id.max(mail.id);
                        m.mails.insert(mail.id, mail);
                    }
                    Err(e) => tracing::warn!("a mail record doesn't read: {e}"),
                },
                KIND_TEXT => {
                    if let Ok(text) = serde_json::from_str::<String>(data) {
                        m.next_id = m.next_id.max(*id as u32);
                        m.texts.insert(*id as u32, text);
                    }
                }
                _ => {}
            }
        }
        m
    }

    /// The highest item GUID counter in the mail.
    pub fn max_item(&self) -> u32 {
        self.mails
            .values()
            .filter_map(|m| m.item.as_ref().map(|i| i.guid))
            .max()
            .unwrap_or(0)
    }

    pub fn next_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn of(&self, receiver: u32, now: i64) -> Vec<&Mail> {
        let mut v: Vec<&Mail> = self
            .mails
            .values()
            .filter(|m| m.receiver == receiver && m.deliver_at <= now)
            .collect();
        v.sort_by_key(|m| std::cmp::Reverse(m.deliver_at));
        v
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn save(realm: &Realm, m: &Mail) {
    realm.store.put_record(KIND_MAIL, u64::from(m.id), m);
}

/// Post a mail; the receiver hears of it if it's due and they're online.
pub fn post(realm: &Realm, mut m: Mail) -> u32 {
    let mut mails = realm.social.mail.lock();
    if m.id == 0 {
        m.id = mails.next_id();
    }
    let id = m.id;
    let due = m.deliver_at <= now();
    let receiver = guid::player(m.receiver);
    save(realm, &m);
    if !due {
        mails.pending.push(id);
    }
    mails.mails.insert(id, m);
    drop(mails);
    if due {
        send_to(realm, receiver, &SMSG_RECEIVED_MAIL { unknown1: 0 });
    }
    id
}

/// A quest's mail when it is rewarded: its template's text and items,
/// from the creature that gave it, after its delay.
pub fn quest_mail(realm: &Realm, receiver: Guid, giver: Guid, quest: u32) {
    let data = &realm.data;
    let Some(q) = data.misc.quest_mail.get(&quest) else {
        return;
    };
    let (subject, body) = data
        .misc
        .mail_templates
        .get(&q.template)
        .cloned()
        .unwrap_or_default();
    let sender = if guid::is_unit(giver) {
        guid::entry(giver)
    } else {
        0
    };
    let now = now();
    let items = data
        .misc
        .mail_items
        .get(&q.template)
        .cloned()
        .unwrap_or_default();
    // One mail an item, as mangos sends a template's.
    let mut attachments: Vec<Option<(u32, u32)>> = items.into_iter().map(Some).collect();
    if attachments.is_empty() {
        attachments.push(None);
    }
    for (i, attachment) in attachments.into_iter().enumerate() {
        let item = attachment.map(|(entry, count)| ItemRow {
            guid: realm
                .next_item
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            entry,
            count: count.max(1),
            durability: crate::game::map::player::max_durability(data, entry),
            ..Default::default()
        });
        post(
            realm,
            Mail {
                kind: CREATURE,
                sender,
                receiver: guid::counter(receiver),
                subject: subject.clone(),
                body: body.clone(),
                item,
                money: if i == 0 { q.money } else { 0 },
                deliver_at: now + i64::from(q.delay_secs),
                expires_at: now + i64::from(q.delay_secs) + 30 * DAY,
                ..Default::default()
            },
        );
    }
}

fn result(map: &Map, g: Guid, mail_id: u32, action: SMSG_SEND_MAIL_RESULT_MailAction) {
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_SEND_MAIL_RESULT { mail_id, action });
    }
}

type Two = SMSG_SEND_MAIL_RESULT_MailResultTwo;

/// `CMSG_GET_MAIL_LIST`.
pub fn list(map: &Map, g: Guid) {
    let realm = &map.realm;
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let now = now();
    let mails = realm.social.mail.lock();
    let list = mails
        .of(guid::counter(g), now)
        .into_iter()
        // As many as the list's count byte holds, as Thistle Tea shows.
        .take(MAX_LISTED)
        .map(|m| {
            let item = m.item.as_ref();
            let max_durability = item
                .map(|i| crate::game::map::player::max_durability(&realm.data, i.entry))
                .unwrap_or(0);
            let shown = item
                .map(|i| crate::game::map::player::Item::shown(&realm.data, i))
                .unwrap_or_default();
            wow_world_messages::vanilla::Mail {
                message_id: m.id,
                message_type: match m.kind {
                    AUCTION => Mail_MailType::Auction {
                        auction_id: m.sender,
                    },
                    CREATURE => Mail_MailType::Creature {
                        sender_id: m.sender,
                    },
                    _ => Mail_MailType::Normal {
                        sender: guid::player(m.sender),
                    },
                },
                subject: m.subject.clone(),
                item_text_id: if m.body.is_empty() { 0 } else { m.id },
                unknown1: 0,
                stationery: m.stationery,
                item: item.map_or(0, |i| i.entry),
                item_enchant_id: shown.enchantment,
                item_random_property_id: shown.random_property,
                item_suffix_factor: shown.suffix_factor,
                item_stack_size: item.map_or(0, |i| i.count.min(255) as u8),
                item_spell_charges: shown.charges,
                max_durability,
                durability: item.map_or(0, |i| i.durability),
                money: Gold::new(m.money),
                cash_on_delivery_amount: m.cod,
                checked_timestamp: m.flags,
                expiration_time: (m.expires_at - now) as f32 / DAY as f32,
                mail_template_id: 0,
            }
        })
        .collect();
    p.outbound.send(&SMSG_MAIL_LIST_RESULT { mails: list });
}

/// `CMSG_SEND_MAIL`.
pub fn send(map: &mut Map, g: Guid, m: CMSG_SEND_MAIL) {
    let realm = map.realm.clone();
    let fail = |map: &Map, e: Two| {
        result(
            map,
            g,
            0,
            SMSG_SEND_MAIL_RESULT_MailAction::Send { result2: e },
        );
    };
    let Some(me) = realm.registry.online(g) else {
        return;
    };
    let Some(to) = realm
        .registry
        .guid_by_name(&m.receiver)
        .and_then(|r| realm.registry.name(r))
    else {
        fail(map, Two::ErrRecipientNotFound);
        return;
    };
    if to.guid == g {
        fail(map, Two::ErrCannotSendToSelf);
        return;
    }
    if Team::of_race(to.race) != Team::of_race(me.race) && !me.gm {
        fail(map, Two::ErrNotYourTeam);
        return;
    }
    let money = m.money.as_int();
    let item_at = (!m.item.is_zero())
        .then(|| map.players.get(&g).and_then(|p| p.item_by_guid(m.item)))
        .flatten();
    if !m.item.is_zero() && item_at.is_none() {
        fail(map, Two::ErrMailAttachmentInvalid);
        return;
    }
    let postage = POSTAGE * if item_at.is_some() { 2 } else { 1 };
    let Some(p) = map.players.get(&g) else {
        return;
    };
    if p.money() < money.saturating_add(postage) {
        fail(map, Two::ErrNotEnoughMoney);
        return;
    }
    if let Some(i) = item_at {
        let item = &p.items[i];
        let bound = item.fields.get_u32(ITEM_FIELD_FLAGS) & 0x1 != 0;
        let bag_with_items = p
            .items
            .iter()
            .any(|x| x.bag == item.slot && item.bag == crate::game::store::BAG_PLAYER)
            && realm
                .data
                .item(item.entry)
                .is_some_and(|t| t.container_slots > 0);
        if (bound && !me.gm)
            || bag_with_items
            || item.bag == crate::game::store::BAG_PLAYER && item.slot < 19
        {
            fail(map, Two::ErrMailAttachmentInvalid);
            return;
        }
        if m.cash_on_delivery_amount > 0 && item.fields.get_u32(ITEM_FIELD_FLAGS) & 0x8 != 0 {
            fail(map, Two::ErrCantSendWrappedCod);
            return;
        }
    }
    let full = realm
        .social
        .mail
        .lock()
        .mails
        .values()
        .filter(|x| x.receiver == guid::counter(to.guid))
        .count()
        >= MAX_MAILS;
    if full {
        fail(map, Two::ErrRecipientCapReached);
        return;
    }
    let p = map.players.get_mut(&g).expect("checked above");
    let left = p.money() - money - postage;
    p.set_money(left);
    let item = item_at.and_then(|i| crate::game::map::inventory::detach_item(map, g, i));
    let other_account = to.account != me.account;
    let delay = if other_account && (item.is_some() || money > 0) {
        realm.config.mail_delay.as_secs() as i64
    } else {
        0
    };
    let now = now();
    let cod = if item.is_some() {
        m.cash_on_delivery_amount
    } else {
        0
    };
    let id = post(
        &realm,
        Mail {
            kind: NORMAL,
            sender: guid::counter(g),
            receiver: guid::counter(to.guid),
            subject: m.subject,
            body: m.body,
            item,
            money,
            cod,
            deliver_at: now + delay,
            expires_at: now + delay + if cod > 0 { 3 * DAY } else { 30 * DAY },
            ..Default::default()
        },
    );
    result(
        map,
        g,
        id,
        SMSG_SEND_MAIL_RESULT_MailAction::Send { result2: Two::Ok },
    );
}

/// The player's own mail, delivered, by id.
fn with_mail<T>(map: &Map, g: Guid, id: u32, f: impl FnOnce(&mut Mail) -> T) -> Option<T> {
    let mut mails = map.realm.social.mail.lock();
    let m = mails
        .mails
        .get_mut(&id)
        .filter(|m| m.receiver == guid::counter(g) && m.deliver_at <= now())?;
    Some(f(m))
}

/// `CMSG_MAIL_TAKE_MONEY`.
pub fn take_money(map: &mut Map, g: Guid, id: u32) {
    let realm = map.realm.clone();
    let Some(money) = with_mail(map, g, id, |m| {
        let money = std::mem::take(&mut m.money);
        save(&realm, m);
        money
    }) else {
        return;
    };
    if let Some(p) = map.players.get_mut(&g) {
        let have = p.money();
        p.set_money(have.saturating_add(money));
    }
    result(
        map,
        g,
        id,
        SMSG_SEND_MAIL_RESULT_MailAction::MoneyTaken { result2: Two::Ok },
    );
}

/// `CMSG_MAIL_TAKE_ITEM`: cash on delivery is paid, and goes to the sender.
pub fn take_item(map: &mut Map, g: Guid, id: u32) {
    let realm = map.realm.clone();
    type R = SMSG_SEND_MAIL_RESULT_MailResult;
    let Some((item, cod, sender, subject)) = with_mail(map, g, id, |m| {
        (m.item.clone(), m.cod, m.sender, m.subject.clone())
    }) else {
        return;
    };
    let Some(item) = item else {
        return;
    };
    let (entry, count) = (item.entry, item.count);
    let money = map.players.get(&g).map_or(0, |p| p.money());
    if money < cod {
        result(
            map,
            g,
            id,
            SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
                result: R::ErrNotEnoughMoney {
                    item: entry,
                    item_count: count,
                },
            },
        );
        return;
    }
    if let Err(e) = crate::game::map::inventory::attach_item(map, g, &item) {
        result(
            map,
            g,
            id,
            SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
                result: R::ErrEquipError {
                    equip_error: match e {
                        crate::game::map::inventory::Fail::CantCarryMore => {
                            InventoryResult::CantCarryMoreOfThis.as_int().into()
                        }
                        _ => InventoryResult::InventoryFull.as_int().into(),
                    },
                },
            },
        );
        return;
    }
    if cod > 0 {
        if let Some(p) = map.players.get_mut(&g) {
            p.set_money(money - cod);
        }
        let now = now();
        post(
            &realm,
            Mail {
                kind: NORMAL,
                sender: guid::counter(g),
                receiver: sender,
                subject,
                money: cod,
                deliver_at: now,
                expires_at: now + 30 * DAY,
                flags: COD_PAYMENT,
                ..Default::default()
            },
        );
    }
    with_mail(map, g, id, |m| {
        m.item = None;
        m.cod = 0;
        save(&realm, m);
    });
    result(
        map,
        g,
        id,
        SMSG_SEND_MAIL_RESULT_MailAction::ItemTaken {
            result: R::Ok {
                item: entry,
                item_count: count,
            },
        },
    );
}

/// `CMSG_MAIL_MARK_AS_READ`.
pub fn mark_read(map: &Map, g: Guid, id: u32) {
    let realm = map.realm.clone();
    with_mail(map, g, id, |m| {
        // Read, it goes in three days at most (Thistle Tea's `mark_read`).
        if m.flags & READ == 0 {
            m.expires_at = m.expires_at.min(now() + 3 * DAY);
        }
        m.flags |= READ;
        save(&realm, m);
    });
}

/// Send a mail back: to its sender, now, for 30 days.
fn send_back(realm: &Realm, m: &mut Mail) {
    std::mem::swap(&mut m.sender, &mut m.receiver);
    m.flags = RETURNED;
    m.cod = 0;
    m.deliver_at = now();
    m.expires_at = m.deliver_at + 30 * DAY;
    save(realm, m);
    send_to(
        realm,
        guid::player(m.receiver),
        &SMSG_RECEIVED_MAIL { unknown1: 0 },
    );
}

/// `CMSG_MAIL_RETURN_TO_SENDER`: a player's mail goes back.
pub fn return_to_sender(map: &Map, g: Guid, id: u32) {
    let realm = map.realm.clone();
    let returned = with_mail(map, g, id, |m| {
        if m.kind != NORMAL || m.flags & RETURNED != 0 {
            return false;
        }
        send_back(&realm, m);
        true
    });
    if returned == Some(true) {
        result(
            map,
            g,
            id,
            SMSG_SEND_MAIL_RESULT_MailAction::ReturnedToSender { result2: Two::Ok },
        );
    }
}

/// `CMSG_MAIL_DELETE`: the mail, and what it still holds, go.
pub fn delete(map: &Map, g: Guid, id: u32) {
    let realm = &map.realm;
    let mut mails = realm.social.mail.lock();
    if mails
        .mails
        .get(&id)
        .is_some_and(|m| m.receiver == guid::counter(g))
    {
        mails.mails.remove(&id);
        realm.store.delete_record(KIND_MAIL, u64::from(id));
        drop(mails);
        result(
            map,
            g,
            id,
            SMSG_SEND_MAIL_RESULT_MailAction::Deleted { result2: Two::Ok },
        );
    }
}

/// `CMSG_MAIL_CREATE_TEXT_ITEM`: the mail's text, copied onto a letter.
pub fn make_letter(map: &mut Map, g: Guid, id: u32) {
    let realm = map.realm.clone();
    let Some(body) = with_mail(map, g, id, |m| {
        (m.flags & COPIED == 0 && !m.body.is_empty()).then(|| m.body.clone())
    })
    .flatten() else {
        return;
    };
    let text_id = {
        let mut mails = realm.social.mail.lock();
        let tid = mails.next_id();
        mails.texts.insert(tid, body.clone());
        tid
    };
    realm.store.put_record(KIND_TEXT, u64::from(text_id), &body);
    let counter = realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let row = ItemRow {
        guid: counter,
        entry: LETTER_ITEM,
        count: 1,
        data: format!("text:{text_id}"),
        ..Default::default()
    };
    if crate::game::map::inventory::attach_item(map, g, &row).is_err() {
        return;
    }
    with_mail(map, g, id, |m| {
        m.flags |= COPIED;
        save(&realm, m);
    });
    result(
        map,
        g,
        id,
        SMSG_SEND_MAIL_RESULT_MailAction::MadePermanent { result2: Two::Ok },
    );
}

/// `CMSG_ITEM_TEXT_QUERY`: a mail's text, or a letter's.
pub fn text_query(map: &Map, g: Guid, text_id: u32) {
    let mails = map.realm.social.mail.lock();
    let text = mails
        .texts
        .get(&text_id)
        .cloned()
        .or_else(|| mails.mails.get(&text_id).map(|m| m.body.clone()))
        .unwrap_or_default();
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&SMSG_ITEM_TEXT_QUERY_RESPONSE {
            item_text_id: text_id,
            text,
        });
    }
}

/// `MSG_QUERY_NEXT_MAIL_TIME`: 0 with unread mail, a day back without.
pub fn next_mail_time(map: &Map, g: Guid) {
    let unread = {
        let mails = map.realm.social.mail.lock();
        mails
            .of(guid::counter(g), now())
            .iter()
            .any(|m| m.flags & READ == 0)
    };
    if let Some(p) = map.players.get(&g) {
        p.outbound.send(&MSG_QUERY_NEXT_MAIL_TIME_Server {
            unread_mails: if unread { 0.0 } else { -86_400.0 },
        });
    }
}

/// Mail that has come due is announced; mail that has expired goes back
/// to a player who sent it, or away.
pub fn sweep(realm: &Realm) {
    let now = now();
    let (due, expired): (Vec<u32>, Vec<u32>) = {
        let mut mails = realm.social.mail.lock();
        let pending = std::mem::take(&mut mails.pending);
        let (due, later): (Vec<u32>, Vec<u32>) = pending
            .into_iter()
            .partition(|id| mails.mails.get(id).is_some_and(|m| m.deliver_at <= now));
        mails.pending = later
            .into_iter()
            .filter(|id| mails.mails.contains_key(id))
            .collect();
        let due = due
            .into_iter()
            .filter_map(|id| mails.mails.get(&id).map(|m| m.receiver))
            .collect();
        let expired = mails
            .mails
            .values()
            .filter(|m| m.expires_at <= now)
            .map(|m| m.id)
            .collect();
        (due, expired)
    };
    for receiver in due {
        send_to(
            realm,
            guid::player(receiver),
            &SMSG_RECEIVED_MAIL { unknown1: 0 },
        );
    }
    for id in expired {
        let mut mails = realm.social.mail.lock();
        let Some(m) = mails.mails.get_mut(&id) else {
            continue;
        };
        let keeps = m.item.is_some() || m.money > 0;
        if m.kind == NORMAL && m.flags & RETURNED == 0 && keeps {
            send_back(realm, m);
        } else {
            mails.mails.remove(&id);
            realm.store.delete_record(KIND_MAIL, u64::from(id));
        }
    }
}

/// A character deleted: the mail waiting for them goes.
pub fn character_deleted(realm: &Realm, counter: u32) {
    let mut mails = realm.social.mail.lock();
    let gone: Vec<u32> = mails
        .mails
        .values()
        .filter(|m| m.receiver == counter)
        .map(|m| m.id)
        .collect();
    for id in gone {
        let Some(mut m) = mails.mails.remove(&id) else {
            continue;
        };
        // An item sent cash on delivery goes back to its sender (Thistle
        // Tea's `Mail.forfeit`); the rest goes with the character.
        if m.cod > 0 && m.item.is_some() && m.kind == NORMAL && m.sender != counter {
            m.money = 0;
            send_back(realm, &mut m);
            mails.mails.insert(id, m);
            continue;
        }
        realm.store.delete_record(KIND_MAIL, u64::from(id));
    }
}
