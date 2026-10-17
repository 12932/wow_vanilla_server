//! A client's session: the world handshake, the character screen, and the
//! routing of its packets to its map.
//!
//! The session switches its route before the client can act on a change:
//! it sends `SMSG_LOGOUT_COMPLETE` itself after the map hands it back, and
//! passes a player to their new map before the map tells the client of
//! the new world. So no packet goes to a map that no longer has the player.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{HeaderCrypto, ProofSeed};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::conn::{self, Connection, Outbound};
use crate::game::data;
use crate::game::guid;
use crate::game::map::player::Player;
use crate::game::map::{Arrival, ArrivalKind, MapEvent, MapHandle, MapKey};
use crate::game::registry::{NameEntry, Online};
use crate::game::store::{AT_LOGIN_FIRST, BAG_PLAYER, CharacterRow, CreateError, ItemRow};

/// What a map tells a session.
pub enum Control {
    /// The player is bound for another map: pass them on, and route there.
    Transfer { player: Box<Player>, to: MapKey },
    /// The player has logged out: back to the character screen.
    LoggedOut,
    /// Disconnect the client.
    Kick,
}

enum State {
    Screen,
    InWorld { guid: Guid, map: MapHandle },
}

pub struct Session {
    realm: Arc<Realm>,
    account: Arc<str>,
    outbound: Outbound,
    control: mpsc::UnboundedSender<Control>,
    state: State,
}

/// Run a connection from its first byte: the world handshake, then the
/// session.
pub async fn serve(
    mut stream: TcpStream,
    realm: Arc<Realm>,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let deadline = crate::config::config().network.handshake_timeout();
    let handshake = tokio::time::timeout(deadline, handshake(&mut stream, &realm)).await;
    drop(permit);
    let (account, crypto) = match handshake {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            debug!("world handshake failed: {e}");
            return;
        }
        Err(_) => {
            debug!("world handshake timed out");
            return;
        }
    };
    let connection = conn::start(stream, crypto);
    run(realm, account, connection).await;
}

async fn handshake(
    stream: &mut TcpStream,
    realm: &Realm,
) -> Result<(String, HeaderCrypto), Box<dyn std::error::Error + Send + Sync>> {
    let seed = ProofSeed::new();
    SMSG_AUTH_CHALLENGE {
        server_seed: seed.seed(),
    }
    .tokio_write_unencrypted_server(&mut *stream)
    .await?;
    let c = tokio_expect_client_message::<CMSG_AUTH_SESSION, _>(&mut *stream).await?;
    let account = c.username;
    let session_key = {
        let users = realm.users.as_ref().ok_or("no auth server")?;
        let mut users = users.lock().map_err(|_| "users mutex poisoned")?;
        let srp = users
            .get_mut(&account)
            .ok_or_else(|| format!("unknown account {account}"))?;
        *srp.session_key()
    };
    let mut crypto = seed
        .into_server_header_crypto(
            &NormalizedString::new(&account)?,
            session_key,
            c.client_proof,
            c.client_seed,
        )
        .map_err(|e| format!("bad proof from {account}: {e:?}"))?;
    SMSG_AUTH_RESPONSE {
        result: SMSG_AUTH_RESPONSE_WorldResult::AuthOk {
            billing_flags: 0,
            billing_rested: 0,
            billing_time: 0,
        },
    }
    .tokio_write_encrypted_server(&mut *stream, crypto.encrypter())
    .await?;
    Ok((account, crypto))
}

/// Run an authenticated session until the client goes.
pub async fn run(realm: Arc<Realm>, account: String, connection: Connection) {
    let Connection {
        outbound,
        mut inbound,
    } = connection;
    let (control, mut control_rx) = mpsc::unbounded_channel();
    let mut rate = PacketRate::new(realm.config.max_packets_per_sec);
    let mut s = Session {
        realm,
        account: Arc::from(account),
        outbound,
        control,
        state: State::Screen,
    };
    loop {
        tokio::select! {
            msg = inbound.recv() => match msg {
                Some(msg) => {
                    if !rate.allow(std::time::Instant::now()) {
                        warn!("{} sends packets too fast; disconnecting", s.account);
                        break;
                    }
                    s.packet(msg).await
                }
                None => break,
            },
            ctl = control_rx.recv() => match ctl {
                Some(ctl) => if !s.control(ctl) { break },
                None => break,
            },
        }
    }
    s.disconnect();
}

impl Session {
    async fn packet(&mut self, msg: crate::game::conn::Inbound) {
        let msg = match msg {
            crate::game::conn::Inbound::Msg(m) => m,
            crate::game::conn::Inbound::Raw(opcode, body) => {
                if let State::InWorld { guid, map } = &self.state {
                    let g = *guid;
                    map.call(move |m| m.handle_raw(g, opcode, &body));
                }
                return;
            }
        };
        if let State::InWorld { guid, map } = &self.state {
            if !map.send(MapEvent::Packet(*guid, msg)) {
                warn!("{}'s map is gone; disconnecting", self.account);
                self.outbound.close();
            }
            return;
        }
        match msg {
            ClientOpcodeMessage::CMSG_CHAR_ENUM => self.enumerate().await,
            ClientOpcodeMessage::CMSG_CHAR_CREATE(c) => self.create(c).await,
            ClientOpcodeMessage::CMSG_CHAR_DELETE(c) => self.delete(c.guid).await,
            ClientOpcodeMessage::CMSG_PLAYER_LOGIN(c) => self.login(c.guid).await,
            other => debug!("{} on the character screen: {other}", self.account),
        }
    }

    /// False to disconnect.
    fn control(&mut self, ctl: Control) -> bool {
        match ctl {
            Control::Transfer { player, to } => {
                let guid = player.guid;
                let map = self.realm.maps.get(&self.realm, to);
                let online = player.online.clone();
                map.send(MapEvent::Arrive(Box::new(Arrival {
                    kind: ArrivalKind::Transfer(player),
                    online,
                })));
                self.state = State::InWorld { guid, map };
                true
            }
            Control::LoggedOut => {
                self.state = State::Screen;
                self.outbound.send(&SMSG_LOGOUT_COMPLETE {});
                true
            }
            Control::Kick => false,
        }
    }

    fn disconnect(&mut self) {
        if let State::InWorld { guid, map } = &self.state {
            map.send(MapEvent::Disconnect(*guid));
        }
        self.state = State::Screen;
        self.outbound.close();
    }

    async fn enumerate(&mut self) {
        let rows = self.realm.store.characters(&self.account).await;
        let characters = rows
            .iter()
            .map(|r| character_entry(&self.realm.data, r))
            .collect();
        self.outbound.send(&SMSG_CHAR_ENUM { characters });
    }

    async fn create(&mut self, c: CMSG_CHAR_CREATE) {
        let result = self.try_create(c).await;
        self.outbound.send(&SMSG_CHAR_CREATE { result });
    }

    async fn try_create(&mut self, c: CMSG_CHAR_CREATE) -> WorldResult {
        let name = match valid_name(&c.name) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let (race, class, gender) = (c.race.as_int(), c.class.as_int(), c.gender.as_int());
        let Some(start) = self.realm.data.start_info(race, class) else {
            return WorldResult::CharCreateFailed;
        };
        if self.realm.registry.name_in_use(&name) {
            return WorldResult::CharCreateNameInUse;
        }
        let existing = self.realm.store.characters(&self.account).await;
        if existing.len() >= self.realm.config.max_characters_per_account {
            return WorldResult::CharCreateAccountLimit;
        }
        // On a PvP realm, an account's characters are all of one side.
        let team = crate::game::registry::Team::of_race(race);
        if self.realm.config.pvp_realm
            && existing
                .iter()
                .any(|r| crate::game::registry::Team::of_race(r.race) != team)
        {
            return WorldResult::CharCreatePvpTeamsViolation;
        }
        let counter = self.realm.next_character.fetch_add(1, Ordering::Relaxed);
        let mut row = CharacterRow {
            guid: counter,
            account: self.account.to_string(),
            name: name.clone(),
            race,
            class,
            gender,
            skin: c.skin_color,
            face: c.face,
            hair_style: c.hair_style,
            hair_color: c.hair_color,
            facial_hair: c.facial_hair,
            level: 1,
            map: start.map,
            x: start.x,
            y: start.y,
            z: start.z,
            orientation: start.orientation,
            zone: start.zone,
            at_login: AT_LOGIN_FIRST,
            spells: start.spells.clone(),
            ..Default::default()
        };
        let spawn = &crate::config::config().spawn;
        if spawn.enabled {
            row.map = spawn.map;
            row.x = spawn.x;
            row.y = spawn.y;
            row.z = spawn.z;
            row.orientation = spawn.orientation;
        }
        for (entry, slot, count) in &start.items {
            let item = self.realm.next_item.fetch_add(1, Ordering::Relaxed);
            row.items.push(ItemRow {
                guid: item,
                entry: *entry,
                bag: BAG_PLAYER,
                slot: *slot,
                count: *count,
                durability: crate::game::map::player::max_durability(&self.realm.data, *entry),
                ..Default::default()
            });
        }
        place_items(&self.realm.data, &mut row.items);
        for (button, action, kind) in &start.actions {
            row.actions.push(crate::game::store::ActionRow {
                button: *button,
                action: *action,
                kind: *kind,
            });
        }
        match self.realm.store.create(row).await {
            Ok(()) => {
                self.realm.registry.add_name(NameEntry {
                    guid: guid::player(counter),
                    account: self.account.clone(),
                    name: Arc::from(name.as_str()),
                    race,
                    class,
                    gender,
                    level: 1.into(),
                });
                info!("{} created {name}", self.account);
                WorldResult::CharCreateSuccess
            }
            Err(CreateError::NameInUse) => WorldResult::CharCreateNameInUse,
            Err(CreateError::Failed) => WorldResult::CharCreateFailed,
        }
    }

    async fn delete(&mut self, g: Guid) {
        let counter = guid::counter(g);
        let result = if !guid::is_player(g) || self.realm.registry.online(g).is_some() {
            WorldResult::CharDeleteFailed
        } else if crate::game::social::guilds::leads_others(&self.realm, counter) {
            // A guild's leader passes it on first.
            WorldResult::CharDeleteFailed
        } else if self.realm.store.delete(&self.account, counter).await {
            self.realm.registry.remove_name(g);
            crate::game::social::guilds::character_deleted(&self.realm, counter);
            crate::game::social::mail::character_deleted(&self.realm, counter);
            crate::game::social::auction::character_deleted(&self.realm, counter);
            crate::game::social::friends::character_deleted(&self.realm, g);
            WorldResult::CharDeleteSuccess
        } else {
            WorldResult::CharDeleteFailed
        };
        self.outbound.send(&SMSG_CHAR_DELETE { result });
    }

    async fn login(&mut self, g: Guid) {
        let fail = |out: &Outbound, result| {
            out.send(&SMSG_CHARACTER_LOGIN_FAILED { result });
        };
        if !guid::is_player(g) {
            return fail(&self.outbound, WorldResult::CharLoginNoCharacter);
        }
        let Some(row) = self.realm.store.load(guid::counter(g)).await else {
            return fail(&self.outbound, WorldResult::CharLoginNoCharacter);
        };
        if *row.account != *self.account {
            return fail(&self.outbound, WorldResult::CharLoginNoCharacter);
        }
        let Some(name) = self.realm.registry.name(g) else {
            return fail(&self.outbound, WorldResult::CharLoginNoCharacter);
        };
        let key = MapKey::world(row.map);
        let map = self.realm.maps.get(&self.realm, key);
        let gm_level = if self.realm.config.everyone_gm {
            crate::accounts::ADMINISTRATOR
        } else {
            crate::accounts::get().map_or(crate::accounts::PLAYER, |a| a.gm_level(&self.account))
        };
        let online = Arc::new(Online::new(
            &name,
            gm_level,
            self.outbound.clone(),
            self.control.clone(),
            map.clone(),
        ));
        if let Err(existing) = self.realm.registry.go_online(online.clone()) {
            // The account is still in the world from another connection:
            // that one goes, and this client can try again.
            let _ = existing.session.send(Control::Kick);
            return fail(&self.outbound, WorldResult::CharLoginDuplicateCharacter);
        }
        map.send(MapEvent::Arrive(Box::new(Arrival {
            kind: ArrivalKind::Login(Box::new(row)),
            online,
        })));
        self.state = State::InWorld { guid: g, map };
    }
}

/// Give each starting item a slot: its equipment slot if it is equipment
/// and the slot is free, else the next free backpack slot. Items the
/// starting data already placed keep their slot.
fn place_items(data: &crate::game::data::GameData, items: &mut Vec<ItemRow>) {
    let mut used = std::collections::HashSet::new();
    for item in items.iter() {
        if item.slot != u8::MAX {
            used.insert(item.slot);
        }
    }
    let mut next_pack = 23u8;
    let mut placed = Vec::with_capacity(items.len());
    for mut item in items.drain(..) {
        if item.slot == u8::MAX {
            let equip = data
                .item(item.entry)
                .and_then(|t| t.equipment_slot())
                .filter(|s| !used.contains(s));
            item.slot = match equip {
                Some(s) => s,
                None => {
                    while used.contains(&next_pack) && next_pack < 39 {
                        next_pack += 1;
                    }
                    if next_pack >= 39 {
                        continue;
                    }
                    next_pack
                }
            };
            used.insert(item.slot);
        }
        placed.push(item);
    }
    *items = placed;
}

/// A name the client may have: 2 to 12 letters, capitalised.
fn valid_name(name: &str) -> Result<String, WorldResult> {
    let len = name.chars().count();
    if len == 0 {
        return Err(WorldResult::CharNameNoName);
    }
    if len < 2 {
        return Err(WorldResult::CharNameTooShort);
    }
    if len > 12 {
        return Err(WorldResult::CharNameTooLong);
    }
    if !name.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(WorldResult::CharNameOnlyLetters);
    }
    if crate::config::config()
        .world
        .reserved_names
        .iter()
        .any(|r| r.eq_ignore_ascii_case(name))
    {
        return Err(WorldResult::CharNameReserved);
    }
    let mut chars = name.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
    Ok(first.unwrap_or_default() + &chars.as_str().to_lowercase())
}

fn character_entry(data: &crate::game::data::GameData, r: &CharacterRow) -> Character {
    let mut equipment: [CharacterGear; 19] = Default::default();
    for item in &r.items {
        if item.bag != BAG_PLAYER || item.slot >= 19 {
            continue;
        }
        if let Some((display, kind)) = crate::game::map::player::item_display(data, item.entry) {
            equipment[usize::from(item.slot)] = CharacterGear {
                equipment_display_id: display,
                inventory_type: InventoryType::try_from(kind as u8).unwrap_or_default(),
            };
        }
    }
    Character {
        guid: guid::player(r.guid),
        name: r.name.clone(),
        race: Race::try_from(r.race).unwrap_or(Race::Human),
        class: Class::try_from(r.class).unwrap_or(Class::Warrior),
        gender: data::gender(r.gender),
        skin: r.skin,
        face: r.face,
        hair_style: r.hair_style,
        hair_color: r.hair_color,
        facial_hair: r.facial_hair,
        level: Level::new(r.level.max(1)),
        area: Area::try_from(r.zone).unwrap_or_default(),
        map: crate::game::map::movement::map_enum(r.map),
        position: Vector3d {
            x: r.x,
            y: r.y,
            z: r.z,
        },
        guild_id: 0,
        flags: CharacterFlags::empty(),
        first_login: r.at_login & AT_LOGIN_FIRST != 0,
        pet_display_id: 0,
        pet_level: Level::new(0),
        pet_family: CreatureFamily::None,
        equipment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_letters_and_capitalised() {
        assert_eq!(valid_name("bOB").unwrap(), "Bob");
        assert_eq!(valid_name("a"), Err(WorldResult::CharNameTooShort));
        assert_eq!(valid_name("abc1"), Err(WorldResult::CharNameOnlyLetters));
        assert_eq!(
            valid_name("Abcdefghijklm"),
            Err(WorldResult::CharNameTooLong)
        );
    }
}

/// A token bucket over a client's packets: `per_sec` sustained, three
/// seconds' worth at once.
pub struct PacketRate {
    per_sec: f32,
    tokens: f32,
    at: std::time::Instant,
}

impl PacketRate {
    pub fn new(per_sec: u32) -> Self {
        let per_sec = per_sec.max(1) as f32;
        Self {
            per_sec,
            tokens: per_sec * 3.0,
            at: std::time::Instant::now(),
        }
    }

    /// Whether one more packet at `now` is within the rate.
    pub fn allow(&mut self, now: std::time::Instant) -> bool {
        let dt = now.saturating_duration_since(self.at).as_secs_f32();
        self.at = now;
        self.tokens = (self.tokens + dt * self.per_sec).min(self.per_sec * 3.0);
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod rate_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_flood_runs_dry_and_a_steady_client_never_does() {
        let start = Instant::now();
        let mut flood = PacketRate::new(10);
        let allowed = (0..100).filter(|_| flood.allow(start)).count();
        assert_eq!(allowed, 30);
        let mut steady = PacketRate::new(10);
        assert!((0..1000).all(|i| steady.allow(start + Duration::from_millis(i * 100))));
    }
}
