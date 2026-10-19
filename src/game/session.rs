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
    /// Another login has the player now, where they are: disconnect the
    /// client, the player staying in the world.
    TakenOver,
    /// Panic in the session's task, as a bug there would.
    #[cfg(test)]
    Panic,
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
    control_rx: mpsc::UnboundedReceiver<Control>,
    state: State,
    /// The character last logged in: the client uploads its caches after
    /// its logout, on the character screen.
    last_character: Option<u32>,
}

/// However a session ends (the client goes, the task panics or is
/// aborted), its player leaves their map, which saves them: never a ghost
/// in the world, holding the account's place.
impl Drop for Session {
    fn drop(&mut self) {
        // At shutdown the maps have saved everyone; and with the runtime
        // gone (a test's end) there is no map to go to.
        if self.realm.maps.is_closed() || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // A player handed over and not yet passed on goes on to their
        // map, to be saved there.
        while let Ok(ctl) = self.control_rx.try_recv() {
            if let Control::Transfer { .. } = ctl {
                self.control(ctl);
            }
        }
        self.disconnect();
    }
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
    let c = read_auth_session(stream).await?;
    let account = c.username;
    let session_key = realm.users.as_ref().and_then(|users| {
        let mut users = crate::auth::users_lock(users);
        users.get_mut(&account).map(|srp| *srp.session_key())
    });
    // Banned since its login: the session key doesn't outlast the ban
    // (mangos's HandleAuthSession answers AUTH_BANNED).
    if crate::auth::banned(&account) {
        let _ = SMSG_AUTH_RESPONSE {
            result: SMSG_AUTH_RESPONSE_WorldResult::AuthBanned,
        }
        .tokio_write_unencrypted_server(&mut *stream)
        .await;
        return Err(format!("{account} is banned").into());
    }
    let crypto = match (session_key, NormalizedString::new(&account)) {
        (Some(key), Ok(name)) => seed
            .into_server_header_crypto(&name, key, c.client_proof, c.client_seed)
            .map_err(|e| format!("bad proof from {account}: {e:?}")),
        _ => Err(format!("unknown account {account}")),
    };
    // A refusal is told, unencrypted as there is no key to encrypt it
    // with, as mangos's HandleAuthSession and Thistle Tea's
    // CmsgAuthSession send it, before the connection closes.
    let mut crypto = match crypto {
        Ok(c) => c,
        Err(e) => {
            let _ = SMSG_AUTH_RESPONSE {
                result: SMSG_AUTH_RESPONSE_WorldResult::AuthFailed,
            }
            .tokio_write_unencrypted_server(&mut *stream)
            .await;
            return Err(e.into());
        }
    };
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

/// Read the client's `CMSG_AUTH_SESSION`, refusing one too big or that
/// wow_messages can't read (its addon data not zlib), and catching a panic
/// in wow_messages as a backstop. However much addon data one claims,
/// wow_messages reserves no more than its zlib could hold.
async fn read_auth_session(
    stream: &mut TcpStream,
) -> Result<CMSG_AUTH_SESSION, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::AsyncReadExt;
    let mut header = [0u8; 6];
    stream.read_exact(&mut header).await?;
    let size = usize::from(u16::from_be_bytes([header[0], header[1]]));
    let opcode = u32::from_le_bytes([header[2], header[3], header[4], header[5]]);
    if opcode != 0x1ED || !(4..=conn::MAX_AUTH_SESSION + 4).contains(&size) {
        return Err(format!("expected an auth session, got 0x{opcode:X} of {size} bytes").into());
    }
    let mut plain = vec![0u8; 2 + size];
    plain[..6].copy_from_slice(&header);
    stream.read_exact(&mut plain[6..]).await?;
    match crate::game::supervisor::catch(|| ClientOpcodeMessage::read_unencrypted(&plain[..])) {
        Ok(Ok(ClientOpcodeMessage::CMSG_AUTH_SESSION(c))) => Ok(c),
        Ok(Ok(other)) => Err(format!("expected an auth session, got {other}").into()),
        Ok(Err(e)) => Err(e.into()),
        Err(e) => Err(format!("an auth session wow_messages panicked on: {e}").into()),
    }
}

/// Run an authenticated session until the client goes.
pub async fn run(realm: Arc<Realm>, account: String, connection: Connection) {
    let Connection {
        outbound,
        mut inbound,
    } = connection;
    let (control, control_rx) = mpsc::unbounded_channel();
    // Two buckets: the read-only queries a client asks of what it sees
    // (a crowd's names, a city's objects) in a generous one of their own,
    // so that walking into a crowd isn't a flood; everything else in the
    // flood guard's.
    let mut rate = PacketRate::new(realm.config().max_packets_per_sec);
    let mut queries = PacketRate::new(realm.config().max_queries_per_sec);
    let mut recent = Recent::default();
    let mut s = Session {
        realm,
        account: Arc::from(account),
        outbound,
        control,
        control_rx,
        state: State::Screen,
        last_character: None,
    };
    #[allow(clippy::large_enum_variant)] // Each packet goes through once.
    enum Next {
        Packet(Option<crate::game::conn::Inbound>),
        Control(Option<Control>),
    }
    loop {
        let next = tokio::select! {
            msg = inbound.recv() => Next::Packet(msg),
            ctl = s.control_rx.recv() => Next::Control(ctl),
        };
        match next {
            Next::Packet(Some(msg)) => {
                let now = std::time::Instant::now();
                recent.note(now, &msg);
                let query = is_query(&msg);
                let bucket = if query { &mut queries } else { &mut rate };
                if !bucket.allow(now) {
                    warn!(
                        "{} sends {} too fast; disconnecting. Lately: {}",
                        s.account,
                        if query { "queries" } else { "packets" },
                        recent.summary(now)
                    );
                    break;
                }
                s.packet(msg).await
            }
            Next::Control(Some(ctl)) => {
                if !s.control(ctl) {
                    break;
                }
            }
            Next::Packet(None) | Next::Control(None) => break,
        }
    }
    // Dropping the session disconnects it.
}

impl Session {
    async fn packet(&mut self, msg: crate::game::conn::Inbound) {
        let msg = match msg {
            crate::game::conn::Inbound::Msg(m) => m,
            crate::game::conn::Inbound::Raw(opcode, body) => {
                if let State::InWorld { guid, map } = &self.state {
                    let g = *guid;
                    map.call(move |m| m.handle_raw(g, opcode, &body));
                } else if opcode == crate::game::map::account_data::CMSG_UPDATE_ACCOUNT_DATA {
                    self.screen_account_data(&body);
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
            // The player is another session's now: they stay in the world,
            // and this connection goes without taking them out of it.
            Control::TakenOver => {
                self.state = State::Screen;
                false
            }
            #[cfg(test)]
            Control::Panic => panic!("the session's test panic"),
        }
    }

    /// The player, if in the world, leaves their map (which saves them),
    /// and the connection closes. Run once whatever ends the session.
    fn disconnect(&mut self) {
        match std::mem::replace(&mut self.state, State::Screen) {
            State::InWorld { guid, map } => {
                if !map.send(MapEvent::Disconnect(guid, self.control.clone())) {
                    // Their map is gone, and them with it: off the online
                    // list, so the account can come back.
                    warn!(
                        "{}'s map has gone; {guid} is lost since their last save",
                        self.account
                    );
                    self.realm.registry.go_offline(guid);
                }
            }
            State::Screen => {
                // Gone between going online and reaching a map (a panic
                // in the login).
                if let Some(o) = self.realm.registry.online_for_account(&self.account)
                    && o.session.same_channel(&self.control)
                {
                    self.realm.registry.go_offline(o.guid);
                }
            }
        }
        self.outbound.close();
    }

    async fn enumerate(&mut self) {
        let rows = self.realm.store.characters(&self.account).await;
        let characters = rows
            .iter()
            .map(|r| {
                let mut c = character_entry(&self.realm.data, r);
                // The guild it is in (Thistle Tea's character list shows it).
                c.guild_id = self
                    .realm
                    .social
                    .guilds
                    .lock()
                    .of(guid::player(r.guid))
                    .map_or(0, |g| g.id);
                c
            })
            .collect();
        self.outbound.send(&SMSG_CHAR_ENUM { characters });
    }

    /// The account's GM level.
    fn gm_level(&self) -> u8 {
        if self.realm.config().everyone_gm {
            crate::accounts::ADMINISTRATOR
        } else {
            crate::accounts::get().map_or(crate::accounts::PLAYER, |a| a.gm_level(&self.account))
        }
    }

    /// The level a new character starts at: `[players] start_level`, or a
    /// GM's `[gm] start_level` if higher.
    fn start_level(&self) -> u8 {
        let cfg = self.realm.config();
        let level = if self.gm_level() > crate::accounts::PLAYER {
            cfg.players.start_level.max(cfg.gm.start_level)
        } else {
            cfg.players.start_level
        };
        level.clamp(1, crate::game::map::stats::max_level())
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
        // A look the client has no texture for crashes whoever draws it.
        let look = crate::game::data::appearance::Look {
            skin: c.skin_color,
            face: c.face,
            hair_style: c.hair_style,
            hair_color: c.hair_color,
            facial_hair: c.facial_hair,
        };
        if !self.realm.data.appearances.valid(race, gender, look) {
            return WorldResult::CharCreateFailed;
        }
        if self.realm.registry.name_in_use(&name) {
            return WorldResult::CharCreateNameInUse;
        }
        let existing = self.realm.store.characters(&self.account).await;
        if existing.len() >= self.realm.config().max_characters_per_account {
            return WorldResult::CharCreateAccountLimit;
        }
        // On a PvP realm, an account's characters are all of one side.
        let team = crate::game::registry::Team::of_race(race);
        if self.realm.config().pvp_realm
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
            level: self.start_level(),
            money: self.realm.config().players.start_money,
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
        // Its skills start as a new character's: weapons and defense at 1
        // (`skills::init`).
        row.state.skills = Some(Vec::new());
        // The flight points the race knows from the first.
        if let Some(r) = self.realm.data.races.get(&race) {
            row.state.taxi_mask = crate::game::map::npc::mask_from_client(&[r.starting_taxi_mask]);
        }
        let spawn = &crate::config::config().spawn;
        if spawn.enabled {
            row.map = spawn.map;
            row.x = spawn.x;
            row.y = spawn.y;
            row.z = spawn.z;
            row.orientation = spawn.orientation;
        }
        // Home is where they start, until an innkeeper binds them
        // elsewhere (mangos's `_LoadHomeBind`, from the race's start).
        row.state.bind = Some(crate::game::store::Bind {
            map: row.map,
            zone: row.zone,
            x: row.x,
            y: row.y,
            z: row.z,
        });
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
        place_items(&self.realm.data, &mut row.items, || {
            self.realm.next_item.fetch_add(1, Ordering::Relaxed)
        });
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
        } else if crate::game::social::guilds::leads(&self.realm, counter) {
            // A guild's leader passes it on, or disbands it, first.
            WorldResult::CharDeleteFailed
        } else if self.realm.store.delete(&self.account, counter).await {
            self.realm.registry.remove_name(g);
            crate::game::social::guilds::character_deleted(&self.realm, counter);
            crate::game::social::mail::character_deleted(&self.realm, counter);
            crate::game::social::auction::character_deleted(&self.realm, counter);
            crate::game::social::friends::character_deleted(&self.realm, g);
            crate::game::social::groups::character_deleted(&self.realm, g);
            WorldResult::CharDeleteSuccess
        } else {
            WorldResult::CharDeleteFailed
        };
        self.outbound.send(&SMSG_CHAR_DELETE { result });
    }

    /// A cache the client uploads on the character screen, after a
    /// logout: the account's, or the character last logged in's, as
    /// Thistle Tea's `AccountCaches` files it.
    fn screen_account_data(&self, body: &[u8]) {
        use crate::game::map::account_data;
        let Some((kind, data)) = account_data::parse_update(body) else {
            return;
        };
        let character = match self.last_character {
            Some(c) => c,
            None if account_data::account_wide(kind) => 0,
            None => return,
        };
        let now = u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
        let k = account_data::key(&self.account, character, kind);
        self.realm.store.set_account_data(&k, kind, now, data);
    }

    /// Log a character in. One still in the world from another connection
    /// is taken over where they are; one leaving it (their client gone a
    /// moment ago, or between maps) is waited for, so that the row read
    /// is the one their leaving saved. Their row is read only once nobody
    /// of the account is online: a session's save is queued in the store
    /// before it goes offline, and the store reads after what was queued
    /// before.
    async fn login(&mut self, g: Guid) {
        if !guid::is_player(g) {
            return login_failed(&self.outbound, WorldResult::CharLoginNoCharacter);
        }
        let Some(name) = self.realm.registry.name(g) else {
            return login_failed(&self.outbound, WorldResult::CharLoginNoCharacter);
        };
        if *name.account != *self.account {
            return login_failed(&self.outbound, WorldResult::CharLoginNoCharacter);
        }
        let started = tokio::time::Instant::now();
        loop {
            let existing = self
                .realm
                .registry
                .online(g)
                .or_else(|| self.realm.registry.online_for_account(&self.account));
            let Some(existing) = existing else {
                match self.enter(g, &name).await {
                    Entered::Done => return,
                    // Another login of the account went online meanwhile.
                    Entered::Raced => continue,
                }
            };
            if existing.guid == g {
                // Still in the world from another connection: this one
                // takes them over where they are, their state kept, and
                // the other goes (mangos's `HandlePlayerReconnect`, the
                // user's #35).
                let new = Arc::new(Online::new(
                    &name,
                    self.gm_level(),
                    self.outbound.clone(),
                    self.control.clone(),
                    existing.map(),
                ));
                let old = existing.clone();
                let taken = existing
                    .map()
                    .query(move |m| crate::game::map::login::take_over(m, g, &old, new))
                    .await;
                if let Some(Some(map)) = taken {
                    self.last_character = Some(guid::counter(g));
                    self.state = State::InWorld { guid: g, map };
                    return;
                }
                // Leaving the world, to be saved, or between maps: wait
                // for them to go or arrive. One whose session has gone
                // has its leaving queued already: if it hasn't gone soon,
                // it is a ghost.
                let wait = if existing.session.is_closed() {
                    GHOST_WAIT
                } else {
                    LOGIN_WAIT
                };
                if started.elapsed() < wait {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    continue;
                }
            }
            // Another of the account's characters, or one that neither
            // went nor arrived: that connection goes, and this client can
            // try again.
            if existing.session.send(Control::Kick).is_err() {
                reap(&self.realm, existing);
            }
            return login_failed(&self.outbound, WorldResult::CharLoginDuplicateCharacter);
        }
    }

    /// Read the character's row and bring them into the world, once nobody
    /// of the account is online.
    async fn enter(&mut self, g: Guid, name: &Arc<NameEntry>) -> Entered {
        let fail = |out: &Outbound, result| {
            login_failed(out, result);
            Entered::Done
        };
        let Some(mut row) = self.realm.store.load(guid::counter(g)).await else {
            return fail(&self.outbound, WorldResult::CharLoginNoCharacter);
        };
        if *row.account != *self.account {
            return fail(&self.outbound, WorldResult::CharLoginNoCharacter);
        }
        // A look made before it was checked, which would crash whoever saw
        // them: what the client has no texture for, back to what it has.
        let look = crate::game::data::appearance::Look {
            skin: row.skin,
            face: row.face,
            hair_style: row.hair_style,
            hair_color: row.hair_color,
            facial_hair: row.facial_hair,
        };
        let mended = self.realm.data.appearances.mend(row.race, row.gender, look);
        if mended != look {
            tracing::info!("{}'s look mended: {look:?} to {mended:?}", row.name);
            (
                row.skin,
                row.face,
                row.hair_style,
                row.hair_color,
                row.facial_hair,
            ) = (
                mended.skin,
                mended.face,
                mended.hair_style,
                mended.hair_color,
                mended.facial_hair,
            );
        }
        self.last_character = Some(row.guid);
        // The client's caches, and the account's tutorials.
        let (caches, tutorials) =
            crate::game::map::account_data::load(&self.realm, &self.account, row.guid).await;
        row.state.caches = caches;
        if let Some(t) = tutorials {
            row.state.tutorials = t;
        }
        let key = MapKey::world(row.map);
        let map = self.realm.maps.get(&self.realm, key);
        let gm_level = self.gm_level();
        let online = Arc::new(Online::new(
            name,
            gm_level,
            self.outbound.clone(),
            self.control.clone(),
            map.clone(),
        ));
        if self.realm.registry.go_online(online.clone()).is_err() {
            return Entered::Raced;
        }
        // Logged out in a dungeon or raid: back into their copy, or, if it
        // won't have them, home. In a battleground: back into the match
        // that kept their place, or home.
        let mut map = map;
        let battleground = crate::game::battlegrounds::bg_of_map(row.map).is_some();
        if battleground
            || matches!(
                crate::game::instances::instance_type(&self.realm, row.map),
                1 | 2
            )
        {
            let back = if battleground {
                crate::game::battlegrounds::rejoin(&self.realm, g, row.map).ok_or(())
            } else {
                crate::game::instances::resume(&self.realm, g, online.gm, row.map).ok_or(())
            };
            match back {
                Ok(k) => map = self.realm.maps.get(&self.realm, k),
                Err(()) => {
                    if let Some(b) = row.state.bind {
                        row.map = b.map;
                        row.x = b.x;
                        row.y = b.y;
                        row.z = b.z;
                        map = self.realm.maps.get(&self.realm, MapKey::world(b.map));
                    }
                }
            }
        }
        if !map.send(MapEvent::Arrive(Box::new(Arrival {
            kind: ArrivalKind::Login(Box::new(row)),
            online,
        }))) {
            self.realm.registry.go_offline(g);
            return fail(&self.outbound, WorldResult::CharLoginNoWorld);
        }
        self.state = State::InWorld { guid: g, map };
        Entered::Done
    }
}

/// How long a login waits for the character to finish leaving the world
/// (their old client's save) or arriving on a map, before it lets that
/// session go and fails, for the client to try again.
const LOGIN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// [`LOGIN_WAIT`], for a character whose session has gone.
const GHOST_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// What [`Session::enter`] came to.
enum Entered {
    /// In the world, or failed and told.
    Done,
    /// Another login of the account went online between the check and
    /// this one: look again.
    Raced,
}

fn login_failed(out: &Outbound, result: WorldResult) {
    out.send(&SMSG_CHARACTER_LOGIN_FAILED { result });
}

/// Pass a player a map has let go of to their session, to route to their
/// new map. If the session has gone, they go on to that map anyway, and
/// leave it at once, saved there: as though the client went on arrival.
pub fn transfer(
    realm: &Arc<Realm>,
    session: &mpsc::UnboundedSender<Control>,
    player: Player,
    to: MapKey,
) {
    let Err(mpsc::error::SendError(Control::Transfer { player, to })) =
        session.send(Control::Transfer {
            player: Box::new(player),
            to,
        })
    else {
        return;
    };
    let guid = player.guid;
    let map = realm.maps.get(realm, to);
    let online = player.online.clone();
    // Their own session's disconnect, whatever session passed them on.
    let from = online.session.clone();
    map.send(MapEvent::Arrive(Box::new(Arrival {
        kind: ArrivalKind::Transfer(player),
        online,
    })));
    map.send(MapEvent::Disconnect(guid, from));
}

/// An online entry whose session has gone (a ghost): its player, if their
/// map still has them, leaves it saved; else the entry goes.
fn reap(realm: &Arc<Realm>, ghost: Arc<Online>) {
    warn!("{} was online with no session; letting them go", ghost.name);
    let registry_realm = realm.clone();
    let reaped = ghost.clone();
    let delivered = ghost.map().call(move |m| {
        if m.players.contains_key(&reaped.guid) {
            m.handle_disconnect(reaped.guid);
        } else if registry_realm
            .registry
            .online(reaped.guid)
            .is_some_and(|o| Arc::ptr_eq(&o, &reaped))
        {
            registry_realm.registry.go_offline(reaped.guid);
        }
    });
    if !delivered
        && realm
            .registry
            .online(ghost.guid)
            .is_some_and(|o| Arc::ptr_eq(&o, &ghost))
    {
        realm.registry.go_offline(ghost.guid);
    }
}

/// Give each starting item a slot: its equipment slot if it is equipment
/// and the slot is free, else the next free backpack slot. Items the
/// starting data already placed keep their slot.
///
/// From vmangos's tables (not `wow_world_base`'s), a starting item
/// without a template isn't given, and one given in more than a stack
/// holds is merged with the rest of it and dealt out in stacks, each its
/// own item (`next_guid`), as mangos's `StoreNewItemInBestSlots` and
/// Thistle Tea's `Characters` store a grant: whole, or not at all.
fn place_items(
    data: &crate::game::data::GameData,
    items: &mut Vec<ItemRow>,
    mut next_guid: impl FnMut() -> u32,
) {
    let vmangos = !data.start.is_empty();
    let stack_of = |entry: u32| data.item(entry).map_or(1, |t| t.stackable.max(1));
    if vmangos {
        items.retain(|i| data.item(i.entry).is_some());
        let mut merged: Vec<ItemRow> = Vec::with_capacity(items.len());
        for item in items.drain(..) {
            let stack = stack_of(item.entry);
            match merged
                .iter_mut()
                .find(|m| stack > 1 && m.entry == item.entry && m.slot == u8::MAX)
            {
                Some(m) if item.slot == u8::MAX => m.count += item.count,
                _ => merged.push(item),
            }
        }
        *items = merged;
    }
    let mut used = ahash::AHashSet::new();
    for item in items.iter() {
        if item.slot != u8::MAX {
            used.insert(item.slot);
        }
    }
    let free_pack = |used: &ahash::AHashSet<u8>| (23u8..39).find(|s| !used.contains(s));
    let mut placed = Vec::with_capacity(items.len());
    for mut item in items.drain(..) {
        if item.slot != u8::MAX {
            placed.push(item);
            continue;
        }
        let stack = stack_of(item.entry);
        if vmangos && item.count > stack {
            // Into the backpack in stacks, if all of them fit.
            let stacks = item.count.div_ceil(stack) as usize;
            let room = (23u8..39).filter(|s| !used.contains(s)).count();
            if stacks > room {
                continue;
            }
            let mut left = item.count;
            let mut guid = item.guid;
            while left > 0 {
                let Some(slot) = free_pack(&used) else {
                    break;
                };
                let n = left.min(stack);
                left -= n;
                used.insert(slot);
                placed.push(ItemRow {
                    guid,
                    slot,
                    count: n,
                    ..item.clone()
                });
                if left > 0 {
                    guid = next_guid();
                }
            }
            continue;
        }
        let equip = data
            .item(item.entry)
            .and_then(|t| t.equipment_slot())
            .filter(|s| !used.contains(s));
        item.slot = match equip.or_else(|| free_pack(&used)) {
            Some(s) => s,
            None => continue,
        };
        used.insert(item.slot);
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
    // The pet that comes back at login, unless a ghost's (display, level,
    // family).
    let ghost = r.state.dead && r.state.corpse.is_some();
    let pet = r
        .state
        .pets
        .current
        .as_ref()
        .filter(|p| r.state.pets.active && !ghost && !p.dead)
        .map(|p| {
            let display = if p.display_id != 0 {
                p.display_id
            } else {
                data.creature_template(p.entry)
                    .and_then(|t| t.display_ids.iter().copied().find(|d| *d != 0))
                    .unwrap_or(0)
            };
            (display, p.level, p.family)
        });
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
        flags: CharacterFlags::new(enum_flags(r)),
        first_login: r.at_login & AT_LOGIN_FIRST != 0,
        pet_display_id: pet.map_or(0, |p| p.0),
        pet_level: Level::new(pet.map_or(0, |p| p.1)),
        pet_family: pet
            .and_then(|p| CreatureFamily::try_from(p.2 as u8).ok())
            .unwrap_or(CreatureFamily::None),
        equipment,
    }
}

/// The character list's flags (Thistle Tea's `enum_flags`): the helm and
/// cloak hidden, and a ghost.
fn enum_flags(r: &CharacterRow) -> u32 {
    use crate::game::map::player::{PLAYER_FLAGS_HIDE_CLOAK, PLAYER_FLAGS_HIDE_HELM};
    let mut flags = 0;
    if r.state.flags & PLAYER_FLAGS_HIDE_HELM != 0 {
        flags |= CharacterFlags::HIDE_HELM;
    }
    if r.state.flags & PLAYER_FLAGS_HIDE_CLOAK != 0 {
        flags |= CharacterFlags::HIDE_CLOAK;
    }
    if r.state.dead && r.state.corpse.is_some() {
        flags |= CharacterFlags::GHOST;
    }
    flags
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

/// Whether a packet is a cheap, read-only query, counted apart from the
/// flood guard's packets: what a client asks of each player, creature,
/// object, item, quest giver and flight master it sees (so hundreds at
/// once in a crowd), the texts and names it looks up by id, and the time
/// and mail-time queries. Pings never get here: the reader answers them.
pub fn is_query(msg: &crate::game::conn::Inbound) -> bool {
    use ClientOpcodeMessage as C;
    matches!(
        msg,
        crate::game::conn::Inbound::Msg(
            C::CMSG_NAME_QUERY(_)
                | C::CMSG_CREATURE_QUERY(_)
                | C::CMSG_GAMEOBJECT_QUERY(_)
                | C::CMSG_ITEM_QUERY_SINGLE(_)
                | C::CMSG_ITEM_NAME_QUERY(_)
                | C::CMSG_PAGE_TEXT_QUERY(_)
                | C::CMSG_NPC_TEXT_QUERY(_)
                | C::CMSG_QUEST_QUERY(_)
                | C::CMSG_QUESTGIVER_STATUS_QUERY(_)
                | C::CMSG_PET_NAME_QUERY(_)
                | C::CMSG_GUILD_QUERY(_)
                | C::CMSG_PETITION_QUERY(_)
                | C::CMSG_ITEM_TEXT_QUERY(_)
                | C::CMSG_TAXINODE_STATUS_QUERY(_)
                | C::CMSG_REQUEST_PARTY_MEMBER_STATS(_)
                | C::CMSG_AREA_SPIRIT_HEALER_QUERY(_)
                | C::MSG_CORPSE_QUERY
                | C::CMSG_QUERY_TIME
                | C::CMSG_PLAYED_TIME
                | C::MSG_QUERY_NEXT_MAIL_TIME
                | C::CMSG_PING(_)
        )
    )
}

/// A packet's kind: its message, or the opcode of one read raw.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Msg(std::mem::Discriminant<ClientOpcodeMessage>),
    Raw(u16),
}

/// The packets a session sent lately, by kind, for the warning when its
/// rate cuts it off: those of the last [`Recent::FOR`], at most
/// [`Recent::MAX`].
#[derive(Default)]
pub struct Recent {
    packets: std::collections::VecDeque<(std::time::Instant, Kind)>,
    /// Each kind's name, written once the first time it is sent.
    names: Vec<(Kind, Box<str>)>,
}

impl Recent {
    const FOR: std::time::Duration = std::time::Duration::from_secs(5);
    const MAX: usize = 2048;

    pub fn note(&mut self, now: std::time::Instant, msg: &crate::game::conn::Inbound) {
        let kind = match msg {
            crate::game::conn::Inbound::Msg(m) => Kind::Msg(std::mem::discriminant(m)),
            crate::game::conn::Inbound::Raw(opcode, _) => Kind::Raw(*opcode),
        };
        if !self.names.iter().any(|(k, _)| *k == kind) {
            let name = match msg {
                crate::game::conn::Inbound::Msg(m) => m.to_string(),
                crate::game::conn::Inbound::Raw(opcode, _) => {
                    wow_world_messages::vanilla::opcode_to_name(u32::from(*opcode))
                        .map_or_else(|| format!("0x{opcode:X}"), str::to_owned)
                }
            };
            self.names.push((kind, name.into()));
        }
        while self.packets.len() >= Self::MAX
            || self
                .packets
                .front()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) > Self::FOR)
        {
            self.packets.pop_front();
        }
        self.packets.push_back((now, kind));
    }

    /// The packets of the last few seconds by kind, the most first:
    /// `412 in 1.3 s: CMSG_NAME_QUERY 380, MSG_MOVE_HEARTBEAT 32`.
    pub fn summary(&self, now: std::time::Instant) -> String {
        let mut counts: Vec<(Kind, usize)> = Vec::new();
        for (_, kind) in &self.packets {
            match counts.iter_mut().find(|(k, _)| k == kind) {
                Some((_, n)) => *n += 1,
                None => counts.push((*kind, 1)),
            }
        }
        counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let span = self.packets.front().map_or(0.0, |(at, _)| {
            now.saturating_duration_since(*at).as_secs_f32()
        });
        let by_kind: Vec<String> = counts
            .iter()
            .map(|(kind, n)| {
                let name = self
                    .names
                    .iter()
                    .find(|(k, _)| k == kind)
                    .map_or("?", |(_, name)| name);
                format!("{name} {n}")
            })
            .collect();
        format!(
            "{} in {span:.1} s: {}",
            self.packets.len(),
            by_kind.join(", ")
        )
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

    fn msg(m: impl Into<ClientOpcodeMessage>) -> crate::game::conn::Inbound {
        crate::game::conn::Inbound::Msg(m.into())
    }

    #[test]
    fn what_a_client_asks_of_what_it_sees_is_a_query_and_what_it_does_isnt() {
        let g = Guid::new(1);
        let queries = [
            msg(CMSG_NAME_QUERY { guid: g }),
            msg(CMSG_GAMEOBJECT_QUERY {
                entry_id: 1,
                guid: g,
            }),
            msg(CMSG_QUESTGIVER_STATUS_QUERY { guid: g }),
            msg(CMSG_QUERY_TIME {}),
        ];
        assert!(queries.iter().all(is_query));
        let not = [
            msg(CMSG_SET_SELECTION { target: g }),
            msg(CMSG_ATTACKSWING { guid: g }),
            crate::game::conn::Inbound::Raw(0x318, Vec::new()),
        ];
        assert!(!not.iter().any(is_query));
    }

    /// The warning names what a session sent lately, by kind, the most
    /// first, and forgets what is older than a few seconds.
    #[test]
    fn the_recent_packets_are_counted_by_kind() {
        let start = Instant::now();
        let mut r = Recent::default();
        let g = Guid::new(1);
        r.note(start, &msg(CMSG_SET_SELECTION { target: g }));
        let late = start + Duration::from_secs(10);
        for _ in 0..3 {
            r.note(late, &msg(CMSG_NAME_QUERY { guid: g }));
        }
        r.note(late, &crate::game::conn::Inbound::Raw(0x318, Vec::new()));
        let s = r.summary(late);
        assert!(
            s.starts_with("4 in 0.0 s: CMSG_NAME_QUERY 3, "),
            "the selection is forgotten: {s}"
        );
        assert!(s.contains("CMSG_SET_WATCHED_FACTION 1"), "{s}");
    }
}
