//! What spans maps: chat channels, groups, and the friend and ignore lists
//! of those online. Each sits behind its own short lock, never held across
//! an `.await`; players elsewhere are reached through the registry.

pub mod auction;
pub mod channels;
pub mod friends;
pub mod groups;
pub mod guilds;
pub mod mail;

use std::sync::Arc;

use parking_lot::Mutex;
use wow_world_messages::Guid;

use crate::game::Realm;
use crate::game::registry::Online;

#[derive(Default)]
pub struct Social {
    pub channels: Mutex<channels::Channels>,
    pub groups: Mutex<groups::Groups>,
    pub lists: Mutex<friends::Lists>,
    pub guilds: Mutex<guilds::Guilds>,
    pub mail: Mutex<mail::Mails>,
    pub auctions: Mutex<auction::Auctions>,
    pub instances: Mutex<crate::game::instances::Instances>,
    pub battlegrounds: Mutex<crate::game::battlegrounds::Battlegrounds>,
    /// The seasonal events on.
    pub events: Mutex<ahash::AHashSet<u32>>,
    pub weather: Mutex<ahash::AHashMap<u32, crate::game::weather::Weather>>,
    /// Server-wide world states events publish: state → value.
    pub world_states: Mutex<ahash::AHashMap<u32, u32>>,
    /// Events the world starts and stops, not the calendar.
    pub driven_events: Mutex<ahash::AHashMap<u32, bool>>,
    pub invasion: Mutex<crate::game::invasion::State>,
    pub elemental: Mutex<crate::game::elemental::State>,
}

impl Social {
    /// The services, with what the store kept of them.
    pub fn load(records: &[(String, u64, String)]) -> Self {
        Self {
            guilds: Mutex::new(guilds::Guilds::load(records)),
            mail: Mutex::new(mail::Mails::load(records)),
            auctions: Mutex::new(auction::Auctions::load(records)),
            instances: Mutex::new(crate::game::instances::Instances::load(records)),
            ..Default::default()
        }
    }
}

/// A message for whoever is online as `guid`.
pub fn send_to<M: wow_world_messages::vanilla::ServerMessage>(realm: &Realm, guid: Guid, msg: &M) {
    if let Some(o) = realm.registry.online(guid) {
        o.outbound.send(msg);
    }
}

/// A frame for whoever is online as `guid`.
pub fn send_frame(realm: &Realm, guid: Guid, frame: &Arc<[u8]>) {
    if let Some(o) = realm.registry.online(guid) {
        o.outbound.send_frame(frame.clone());
    }
}

/// Whether `a` and `b` may talk: the same team, or either a GM.
pub fn same_side(a: &Online, b: &Online) -> bool {
    a.gm || b.gm
        || crate::game::registry::Team::of_race(a.race)
            == crate::game::registry::Team::of_race(b.race)
}

/// How often mail and auctions are swept for what has come due.
pub const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The sweeps of mail and auctions, while the realm lasts.
pub fn start_timers(realm: &Arc<Realm>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let weak = Arc::downgrade(realm);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            let Some(realm) = weak.upgrade() else {
                return;
            };
            sweep(&realm);
        }
    });
}

/// Everything time drives outside the maps.
pub fn sweep(realm: &Realm) {
    auction::sweep(realm);
    mail::sweep(realm);
    crate::game::instances::sweep(realm);
    crate::game::battlegrounds::sweep(realm);
    crate::game::events::sweep(realm);
    crate::game::invasion::sweep(realm);
    crate::game::elemental::sweep(realm);
    crate::game::weather::sweep(realm);
}
