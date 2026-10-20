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
use crate::game::store::{ItemRow, Records};

pub const KIND_MAIL: &str = "mail";
pub const KIND_TEXT: &str = "text";

pub const POSTAGE: u32 = 30;
/// The mails a mailbox lists.
const MAX_LISTED: usize = 254;
/// The most a mail list's body holds: mangos's `MAX_NETCLIENT_PACKET_SIZE`,
/// which it calls the 1.12 client's hard cap, and where its
/// `HandleGetMailList` stops listing.
pub const LIST_BYTES: usize = 32_766;
/// A listed mail's bytes less its subject's, at most: a player's mail, whose
/// sender is a whole GUID (wow_messages' `Mail`).
const LISTED_BYTES: usize = 75;
/// The longest subject a mail keeps, in letters: the client's
/// `SendMailSubjectEditBox` takes 64 (1.12.1's FrameXML).
pub const SUBJECT_LETTERS: usize = 64;
/// The longest body a mail keeps, in bytes. The client's box takes 500
/// letters; an addon's `SendMail` may send more, and the body comes back
/// whole in an item text's answer, which this keeps well under the
/// client's cap.
pub const BODY_BYTES: usize = 8_000;
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
    /// Every mail, by id. Add, remove and send back through [`Mails::insert`],
    /// [`Mails::remove`] and `send_back`, which keep `by_receiver`; nothing
    /// else changes a mail's receiver.
    pub mails: AHashMap<u32, Mail>,
    /// Texts made into letters, by id.
    pub texts: AHashMap<u32, String>,
    next_id: u32,
    /// Mails due and not yet announced to their receiver.
    pending: Vec<u32>,
    /// Each receiver's mail ids (R4-44): a mailbox, its count and the next
    /// mail time look through their receiver's mail, not the realm's.
    by_receiver: AHashMap<u32, Vec<u32>>,
}

impl Mails {
    pub fn load(records: &[(String, u64, String)]) -> Self {
        let mut m = Self::default();
        for (kind, id, data) in records {
            match kind.as_str() {
                KIND_MAIL => match serde_json::from_str::<Mail>(data) {
                    Ok(mail) => {
                        m.next_id = m.next_id.max(mail.id);
                        m.insert(mail);
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

    /// A mail in, or over the one of its id.
    pub fn insert(&mut self, m: Mail) {
        let (id, receiver) = (m.id, m.receiver);
        if let Some(old) = self.mails.insert(id, m) {
            if old.receiver == receiver {
                return;
            }
            self.unindex(old.receiver, id);
        }
        self.by_receiver.entry(receiver).or_default().push(id);
    }

    /// A mail out.
    pub fn remove(&mut self, id: u32) -> Option<Mail> {
        let m = self.mails.remove(&id)?;
        self.unindex(m.receiver, id);
        Some(m)
    }

    fn unindex(&mut self, receiver: u32, id: u32) {
        if let Some(ids) = self.by_receiver.get_mut(&receiver) {
            ids.retain(|x| *x != id);
            if ids.is_empty() {
                self.by_receiver.remove(&receiver);
            }
        }
    }

    /// The ids of the mail for `receiver`, delivered or not.
    pub fn ids_of(&self, receiver: u32) -> &[u32] {
        self.by_receiver.get(&receiver).map_or(&[], Vec::as_slice)
    }

    /// The mail delivered to `receiver`, newest first.
    fn of(&self, receiver: u32, now: i64) -> Vec<&Mail> {
        let mut v: Vec<&Mail> = self
            .ids_of(receiver)
            .iter()
            .filter_map(|id| self.mails.get(id))
            .filter(|m| m.deliver_at <= now)
            .collect();
        v.sort_by_key(|m| std::cmp::Reverse(m.deliver_at));
        v
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `s` cut to at most `letters` characters and `bytes` bytes, at a
/// character's boundary.
fn cut(s: &str, letters: usize, bytes: usize) -> &str {
    let mut end = s.len().min(bytes);
    if let Some((at, _)) = s.char_indices().nth(letters) {
        end = end.min(at);
    }
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A subject as a mail keeps and lists it.
fn subject(s: &str) -> &str {
    cut(s, SUBJECT_LETTERS, usize::MAX)
}

/// A body as a mail keeps and shows it.
fn body(s: &str) -> &str {
    cut(s, usize::MAX, BODY_BYTES)
}

fn save(realm: &Realm, m: &Mail) {
    realm.store.put_record(KIND_MAIL, u64::from(m.id), m);
}

/// Post a mail; the receiver hears of it if it's due and they're online.
pub fn post(realm: &Realm, m: Mail) -> u32 {
    let mut records = Records::new();
    let mut mails = realm.social.mail.lock();
    let id = post_in(realm, &mut mails, m, &mut records);
    realm.store.save_together(Vec::new(), records);
    drop(mails);
    id
}

/// Post a mail whose record is saved with `records`: with what it holds
/// taken from a character, or with the auction it settles.
///
/// The caller holds the mail's lock (`mails`) until `records` is sent to
/// the store. Any map can take from the mail once it is in `mails`, and a
/// take sends its save under the lock. Were the lock let go first, a take
/// in between would be written first and the post's older record over it:
/// a refund taken would be back in the mail after a restart.
pub fn post_in(realm: &Realm, mails: &mut Mails, mut m: Mail, records: &mut Records) -> u32 {
    if m.id == 0 {
        m.id = mails.next_id();
    }
    let id = m.id;
    let due = m.deliver_at <= now();
    let receiver = guid::player(m.receiver);
    records.put(KIND_MAIL, u64::from(id), &m);
    if !due {
        mails.pending.push(id);
    }
    mails.insert(m);
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
    // As many as the list's count byte holds, as Thistle Tea shows, and as
    // fit the client's cap, as mangos's `HandleGetMailList` stops: past
    // 65,535 bytes the frame's size wraps and the client reads the rest as
    // packets.
    let mut bytes = 1;
    let list: Vec<_> = mails
        .of(guid::counter(g), now)
        .into_iter()
        .take(MAX_LISTED)
        .take_while(|m| {
            bytes += LISTED_BYTES + subject(&m.subject).len();
            bytes <= LIST_BYTES
        })
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
                subject: subject(&m.subject).to_owned(),
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
    drop(mails);
    let list = SMSG_MAIL_LIST_RESULT { mails: list };
    debug_assert!(wow_world_messages::Message::size_without_header(&list) as usize <= LIST_BYTES);
    p.outbound.send(&list);
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
        let bound = crate::game::map::items::bound_now(&realm.data, item);
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
            || crate::game::map::vendor::in_buyback(item.bag, item.slot)
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
        .ids_of(guid::counter(to.guid))
        .len()
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
        realm.config().mail_delay.as_secs() as i64
    } else {
        0
    };
    let now = now();
    let cod = if item.is_some() {
        m.cash_on_delivery_amount
    } else {
        0
    };
    let mut records = Records::new();
    let mut mails = realm.social.mail.lock();
    let id = post_in(
        &realm,
        &mut mails,
        Mail {
            kind: NORMAL,
            sender: guid::counter(g),
            receiver: guid::counter(to.guid),
            // Cut to what the client sends: a list of long subjects, or a
            // long body's text, would pass the client's cap.
            subject: subject(&m.subject).to_owned(),
            body: body(&m.body).to_owned(),
            item,
            money,
            cod,
            deliver_at: now + delay,
            expires_at: now + delay + if cod > 0 { 3 * DAY } else { 30 * DAY },
            ..Default::default()
        },
        &mut records,
    );
    // The mail and the sender's bags and money in one transaction
    // (mangos's `HandleSendMail`), under the mail's lock.
    map.save_together(&[g], records);
    drop(mails);
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
    own_mail(&mut mails, g, id).map(f)
}

/// The player's own mail, delivered, by id, in the mail already locked.
fn own_mail(mails: &mut Mails, g: Guid, id: u32) -> Option<&mut Mail> {
    mails
        .mails
        .get_mut(&id)
        .filter(|m| m.receiver == guid::counter(g) && m.deliver_at <= now())
}

/// `CMSG_MAIL_TAKE_MONEY`.
pub fn take_money(map: &mut Map, g: Guid, id: u32) {
    let realm = map.realm.clone();
    let mut mails = realm.social.mail.lock();
    let Some(m) = own_mail(&mut mails, g, id) else {
        return;
    };
    let money = std::mem::take(&mut m.money);
    if money > 0 {
        let mut records = Records::new();
        records.put(KIND_MAIL, u64::from(id), m);
        if let Some(p) = map.players.get_mut(&g) {
            let have = p.money();
            p.set_money(have.saturating_add(money));
        }
        // The mail and its taker in one transaction, sent under the mail's
        // lock so that no later change to the mail is written before it.
        map.save_together(&[g], records);
    }
    drop(mails);
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
    let mut records = Records::new();
    let mut mails = realm.social.mail.lock();
    if cod > 0 {
        if let Some(p) = map.players.get_mut(&g) {
            p.set_money(money - cod);
        }
        let now = now();
        post_in(
            &realm,
            &mut mails,
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
            &mut records,
        );
    }
    // The mail without its item, the payment and the taker in one
    // transaction (mangos's `HandleMailTakeItem`), under the mail's lock.
    if let Some(m) = own_mail(&mut mails, g, id) {
        m.item = None;
        m.cod = 0;
        records.put(KIND_MAIL, u64::from(id), m);
    }
    map.save_together(&[g], records);
    drop(mails);
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
fn send_back(realm: &Realm, mails: &mut Mails, id: u32) {
    let Some(mut m) = mails.remove(id) else {
        return;
    };
    std::mem::swap(&mut m.sender, &mut m.receiver);
    m.flags = RETURNED;
    m.cod = 0;
    m.deliver_at = now();
    m.expires_at = m.deliver_at + 30 * DAY;
    save(realm, &m);
    let receiver = guid::player(m.receiver);
    mails.insert(m);
    send_to(realm, receiver, &SMSG_RECEIVED_MAIL { unknown1: 0 });
}

/// `CMSG_MAIL_RETURN_TO_SENDER`: a player's mail goes back.
pub fn return_to_sender(map: &Map, g: Guid, id: u32) {
    let realm = map.realm.clone();
    let mut mails = realm.social.mail.lock();
    let returned =
        own_mail(&mut mails, g, id).is_some_and(|m| m.kind == NORMAL && m.flags & RETURNED == 0);
    if returned {
        send_back(&realm, &mut mails, id);
    }
    drop(mails);
    if returned {
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
        mails.remove(id);
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
    let mut records = Records::new();
    records.put(KIND_TEXT, u64::from(text_id), &body);
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
    // The letter, its text and the mail marked copied, together.
    let mut mails = realm.social.mail.lock();
    if let Some(m) = own_mail(&mut mails, g, id) {
        m.flags |= COPIED;
        records.put(KIND_MAIL, u64::from(id), m);
    }
    map.save_together(&[g], records);
    drop(mails);
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
        .or_else(|| mails.mails.get(&text_id).map(|m| &m.body))
        .map(|t| body(t).to_owned())
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
        let Some(m) = mails.mails.get(&id) else {
            continue;
        };
        let keeps = m.item.is_some() || m.money > 0;
        if m.kind == NORMAL && m.flags & RETURNED == 0 && keeps {
            send_back(realm, &mut mails, id);
        } else {
            mails.remove(id);
            realm.store.delete_record(KIND_MAIL, u64::from(id));
        }
    }
}

/// A character deleted: the mail waiting for them goes.
pub fn character_deleted(realm: &Realm, counter: u32) {
    let mut mails = realm.social.mail.lock();
    let gone: Vec<u32> = mails.ids_of(counter).to_vec();
    for id in gone {
        let Some(m) = mails.mails.get_mut(&id) else {
            continue;
        };
        // An item sent cash on delivery goes back to its sender (Thistle
        // Tea's `Mail.forfeit`); the rest goes with the character.
        if m.cod > 0 && m.item.is_some() && m.kind == NORMAL && m.sender != counter {
            m.money = 0;
            send_back(realm, &mut mails, id);
            continue;
        }
        mails.remove(id);
        realm.store.delete_record(KIND_MAIL, u64::from(id));
    }
}
