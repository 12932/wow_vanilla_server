//! Where each in-world packet goes.

use tracing::trace;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::player::{PLAYER_FLAGS_HIDE_CLOAK, PLAYER_FLAGS_HIDE_HELM};
use crate::game::map::{Map, chat, login, movement, query};

/// An action button's kinds, its packed data's high byte.
const ACTION_BUTTON_SPELL: u8 = 0x00;
const ACTION_BUTTON_MACRO: u8 = 0x40;
const ACTION_BUTTON_CLICK_MACRO: u8 = 0x41;
const ACTION_BUTTON_ITEM: u8 = 0x80;

impl Map {
    pub fn handle_packet(&mut self, guid: Guid, msg: ClientOpcodeMessage) {
        let Some(p) = self.players.get(&guid) else {
            return;
        };
        let out = p.outbound.clone();
        let msg = match movement::movement_of(msg) {
            Ok((info, relay)) => {
                movement::on_move(self, guid, info, relay);
                return;
            }
            Err(msg) => msg,
        };
        let Some(msg) = crate::game::map::misc::handle_all(self, guid, msg) else {
            return;
        };
        let Some(msg) = movement::handle(self, guid, msg) else {
            return;
        };
        let Some(msg) = crate::game::map::pvp::handle(self, guid, msg) else {
            return;
        };
        let Some(msg) = crate::game::map::social::handle(self, guid, msg) else {
            return;
        };
        let Some(msg) = crate::game::map::loot::handle(self, guid, msg) else {
            return;
        };
        let Some(msg) = crate::game::map::ranged::handle(self, guid, msg) else {
            return;
        };
        let Some(msg) = crate::game::map::pets::handle(self, guid, msg) else {
            return;
        };
        match msg {
            ClientOpcodeMessage::CMSG_NAME_QUERY(m) => query::name(self, &out, m),
            ClientOpcodeMessage::CMSG_CREATURE_QUERY(m) => query::creature(self, &out, m),
            ClientOpcodeMessage::CMSG_ITEM_QUERY_SINGLE(m) => query::item(self, &out, m),
            ClientOpcodeMessage::CMSG_ITEM_NAME_QUERY(m) => query::item_name(self, &out, m),
            ClientOpcodeMessage::CMSG_GAMEOBJECT_QUERY(m) => {
                let t = self.realm.data.gameobject_templates.get(&m.entry_id);
                out.send_frame(crate::game::map::gameobject::query_response(m.entry_id, t));
            }
            ClientOpcodeMessage::CMSG_LOOT(m) => crate::game::map::loot::open(self, guid, m.guid),
            ClientOpcodeMessage::CMSG_AUTOSTORE_LOOT_ITEM(m) => {
                crate::game::map::loot::take_item(self, guid, m.item_slot)
            }
            ClientOpcodeMessage::CMSG_LOOT_MONEY => crate::game::map::loot::take_money(self, guid),
            ClientOpcodeMessage::CMSG_LOOT_RELEASE(m) => {
                crate::game::map::loot::release(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_SWAP_INV_ITEM(m) => crate::game::map::inventory::move_item(
                self,
                guid,
                (crate::game::store::BAG_PLAYER, m.source_slot.as_int()),
                (crate::game::store::BAG_PLAYER, m.destination_slot.as_int()),
            ),
            ClientOpcodeMessage::CMSG_SWAP_ITEM(m) => crate::game::map::inventory::move_item(
                self,
                guid,
                (m.source_bag, m.source_slot),
                (m.destination_bag, m.destionation_slot),
            ),
            ClientOpcodeMessage::CMSG_AUTOEQUIP_ITEM(m) => {
                crate::game::map::inventory::auto_equip(self, guid, (m.source_bag, m.source_slot))
            }
            ClientOpcodeMessage::CMSG_AUTOSTORE_BAG_ITEM(m) => {
                crate::game::map::inventory::auto_store(
                    self,
                    guid,
                    (m.source_bag, m.source_slot),
                    m.destination_bag,
                )
            }
            ClientOpcodeMessage::CMSG_SPLIT_ITEM(m) => crate::game::map::inventory::split(
                self,
                guid,
                (m.source_bag, m.source_slot),
                (m.destination_bag, m.destination_slot),
                u32::from(m.amount),
            ),
            ClientOpcodeMessage::CMSG_DESTROYITEM(m) => crate::game::map::inventory::destroy(
                self,
                guid,
                (m.bag, m.slot),
                u32::from(m.amount),
            ),
            ClientOpcodeMessage::CMSG_LIST_INVENTORY(m) => {
                crate::game::map::vendor::list(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_BUY_ITEM(m) => {
                crate::game::map::vendor::buy(self, guid, m.vendor, m.item, m.amount)
            }
            ClientOpcodeMessage::CMSG_BUY_ITEM_IN_SLOT(m) => crate::game::map::vendor::buy_in_slot(
                self, guid, m.vendor, m.item, m.amount, m.bag, m.bag_slot,
            ),
            ClientOpcodeMessage::CMSG_SELL_ITEM(m) => {
                crate::game::map::vendor::sell(self, guid, m.vendor, m.item, m.amount)
            }
            ClientOpcodeMessage::CMSG_BUYBACK_ITEM(m) => {
                crate::game::map::vendor::buyback(self, guid, m.guid, m.slot)
            }
            ClientOpcodeMessage::CMSG_REPAIR_ITEM(m) => {
                crate::game::map::vendor::repair(self, guid, m.npc, m.item)
            }
            ClientOpcodeMessage::CMSG_GAMEOBJ_USE(m) => {
                crate::game::map::world::use_object(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_ATTACKSWING(m) => {
                crate::game::map::combat::start_attack(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_ATTACKSTOP => {
                crate::game::map::combat::stop_attack(self, guid)
            }
            ClientOpcodeMessage::CMSG_REPOP_REQUEST => crate::game::map::death::release(self, guid),
            ClientOpcodeMessage::CMSG_RECLAIM_CORPSE(_) => {
                crate::game::map::death::reclaim(self, guid)
            }
            ClientOpcodeMessage::MSG_CORPSE_QUERY => {
                crate::game::map::death::corpse_query(self, guid)
            }
            ClientOpcodeMessage::CMSG_SPIRIT_HEALER_ACTIVATE(m) => {
                crate::game::map::death::spirit_healer_activate(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_GOSSIP_HELLO(m) => {
                crate::game::map::npc::hello(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_GOSSIP_SELECT_OPTION(m) => {
                crate::game::map::npc::select(self, guid, m.guid, m.gossip_list_id)
            }
            ClientOpcodeMessage::CMSG_NPC_TEXT_QUERY(m) => {
                crate::game::map::npc::npc_text(self, guid, m.text_id)
            }
            ClientOpcodeMessage::CMSG_TRAINER_LIST(m) => {
                crate::game::map::npc::trainer_list(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_TRAINER_BUY_SPELL(m) => {
                crate::game::map::npc::trainer_buy(self, guid, m.guid, m.id)
            }
            ClientOpcodeMessage::CMSG_BINDER_ACTIVATE(m) => {
                crate::game::map::npc::bind(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_BANKER_ACTIVATE(m) => {
                crate::game::map::npc::banker(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_BUY_BANK_SLOT(m) => {
                crate::game::map::npc::buy_bank_slot(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_TAXINODE_STATUS_QUERY(m) => {
                crate::game::map::npc::taxi_status(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_TAXIQUERYAVAILABLENODES(m) => {
                crate::game::map::npc::taxi_nodes(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_ACTIVATETAXI(m) => crate::game::map::npc::activate_taxi(
                self,
                guid,
                m.guid,
                m.source_node,
                m.destination_node,
            ),
            ClientOpcodeMessage::CMSG_BATTLEMASTER_HELLO(m) => {
                crate::game::map::npc::battlemaster_hello(self, guid, m.guid)
            }
            ClientOpcodeMessage::MSG_AUCTION_HELLO(m) => {
                crate::game::map::npc::auction_hello(self, guid, m.auctioneer)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_STATUS_QUERY(m) => {
                crate::game::map::quests::status_query(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_HELLO(m) => {
                crate::game::map::quests::hello(self, guid, m.guid)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_QUERY_QUEST(m) => {
                crate::game::map::quests::query_quest(self, guid, m.guid, m.quest_id)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_ACCEPT_QUEST(m) => {
                crate::game::map::quests::accept(self, guid, m.guid, m.quest_id)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_COMPLETE_QUEST(m) => {
                crate::game::map::quests::complete_quest(self, guid, m.guid, m.quest_id)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_REQUEST_REWARD(m) => {
                crate::game::map::quests::offer_reward(self, guid, m.guid, m.quest_id)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_CHOOSE_REWARD(m) => {
                crate::game::map::quests::choose_reward(self, guid, m.guid, m.quest_id, m.reward)
            }
            ClientOpcodeMessage::CMSG_QUESTLOG_REMOVE_QUEST(m) => {
                crate::game::map::quests::abandon(self, guid, m.slot)
            }
            ClientOpcodeMessage::CMSG_QUEST_QUERY(m) => {
                crate::game::map::quests::query(self, guid, m.quest_id)
            }
            ClientOpcodeMessage::CMSG_QUESTGIVER_CANCEL => {
                crate::game::map::quests::cancel(self, guid)
            }
            ClientOpcodeMessage::CMSG_RESET_INSTANCES => crate::game::instances::reset(self, guid),
            ClientOpcodeMessage::CMSG_REQUEST_RAID_INFO => {
                crate::game::instances::raid_info(self, guid)
            }
            ClientOpcodeMessage::CMSG_CAST_SPELL(m) => {
                crate::game::map::spells::cast_request(self, guid, m.spell, &m.targets)
            }
            ClientOpcodeMessage::CMSG_CANCEL_CAST(_)
            | ClientOpcodeMessage::CMSG_CANCEL_CHANNELLING(_) => {
                crate::game::map::spells::cancel(self, guid)
            }
            ClientOpcodeMessage::CMSG_CANCEL_AURA(m) => {
                crate::game::map::auras::cancel(self, guid, m.id)
            }
            ClientOpcodeMessage::CMSG_RESURRECT_RESPONSE(m) => {
                crate::game::map::spells::resurrect_response(self, guid, m.guid, m.status != 0)
            }
            ClientOpcodeMessage::CMSG_LEARN_TALENT(m) => crate::game::map::spells::learn_talent(
                self,
                guid,
                m.talent.as_int(),
                m.requested_rank,
            ),
            ClientOpcodeMessage::CMSG_USE_ITEM(m) => {
                crate::game::map::spells::use_item(self, guid, m.bag_index, m.bag_slot, &m.targets)
            }
            ClientOpcodeMessage::CMSG_AREATRIGGER(m) => {
                crate::game::map::world::area_trigger(self, guid, m.trigger_id)
            }
            ClientOpcodeMessage::CMSG_QUERY_TIME => query::time(&out),
            ClientOpcodeMessage::CMSG_PLAYED_TIME => query::played(self, guid),
            ClientOpcodeMessage::CMSG_MESSAGECHAT(m) => {
                if chat::flooded(self, guid) {
                    return;
                }
                if let Some(m) = chat::message(self, guid, m) {
                    crate::game::map::social::message(self, guid, m);
                }
            }
            ClientOpcodeMessage::CMSG_TEXT_EMOTE(m) => chat::text_emote(self, guid, m),
            ClientOpcodeMessage::CMSG_EMOTE(m) => chat::emote(self, guid, m),
            ClientOpcodeMessage::CMSG_SET_SELECTION(m) => {
                if let Some(p) = self.players.get_mut(&guid) {
                    p.fields.set_guid(UNIT_FIELD_TARGET, m.target);
                }
            }
            ClientOpcodeMessage::CMSG_STANDSTATECHANGE(m) => {
                chat::stand_state(self, guid, m.animation_state);
            }
            ClientOpcodeMessage::CMSG_SETSHEATHED(m) => {
                crate::game::map::combat::sheathe(self, guid, m.sheathed.as_int());
            }
            ClientOpcodeMessage::CMSG_TOGGLE_HELM => self.toggle_flag(guid, PLAYER_FLAGS_HIDE_HELM),
            ClientOpcodeMessage::CMSG_TOGGLE_CLOAK => {
                self.toggle_flag(guid, PLAYER_FLAGS_HIDE_CLOAK)
            }
            ClientOpcodeMessage::CMSG_LOGOUT_REQUEST => login::request_logout(self, guid),
            ClientOpcodeMessage::CMSG_LOGOUT_CANCEL => login::cancel_logout(self, guid),
            ClientOpcodeMessage::CMSG_PLAYER_LOGOUT => login::complete_logout(self, guid),
            ClientOpcodeMessage::CMSG_ZONEUPDATE(m) => {
                let mut changed = false;
                if let Some(p) = self.players.get_mut(&guid) {
                    changed = p.zone != m.area.as_int();
                    p.zone = m.area.as_int();
                    p.online
                        .zone
                        .store(p.zone, std::sync::atomic::Ordering::Relaxed);
                    crate::game::weather::tell(&self.realm, &p.online, p.zone);
                }
                if changed {
                    crate::game::map::world_states::send_init(self, guid);
                }
            }
            ClientOpcodeMessage::CMSG_SET_ACTIONBAR_TOGGLES(m) => {
                if let Some(p) = self.players.get_mut(&guid) {
                    p.fields.set_byte(PLAYER_FIELD_BYTES, 2, m.action_bar);
                }
            }
            ClientOpcodeMessage::CMSG_SET_ACTION_BUTTON(m) => self.set_action_button(guid, m),
            ClientOpcodeMessage::CMSG_TUTORIAL_FLAG(m) => {
                if let Some(p) = self.players.get_mut(&guid) {
                    let word = (m.tutorial_flag / 32) as usize;
                    if let Some(w) = p.row.state.tutorials.get_mut(word) {
                        *w |= 1 << (m.tutorial_flag % 32);
                    }
                }
            }
            ClientOpcodeMessage::CMSG_TUTORIAL_CLEAR => {
                if let Some(p) = self.players.get_mut(&guid) {
                    p.row.state.tutorials = [u32::MAX; 8];
                }
            }
            ClientOpcodeMessage::CMSG_TUTORIAL_RESET => {
                if let Some(p) = self.players.get_mut(&guid) {
                    p.row.state.tutorials = [0; 8];
                }
            }
            ClientOpcodeMessage::CMSG_UPDATE_ACCOUNT_DATA(m) => {
                let account = self.players.get(&guid).map(|p| p.account.clone());
                if let Some(account) = account {
                    let kind = m.data_type.as_int();
                    let now = u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
                    self.realm
                        .store
                        .set_account_data(&account, kind, now, m.compressed_data);
                }
            }
            ClientOpcodeMessage::CMSG_REQUEST_ACCOUNT_DATA(_)
            | ClientOpcodeMessage::CMSG_COMPLETE_CINEMATIC
            | ClientOpcodeMessage::CMSG_NEXT_CINEMATIC_CAMERA
            | ClientOpcodeMessage::CMSG_BUG(_)
            | ClientOpcodeMessage::CMSG_MEETINGSTONE_INFO
            | ClientOpcodeMessage::CMSG_GMTICKET_GETTICKET
            | ClientOpcodeMessage::CMSG_GMTICKET_SYSTEMSTATUS
            | ClientOpcodeMessage::CMSG_BATTLEFIELD_STATUS
            | ClientOpcodeMessage::CMSG_CHAR_ENUM => {}
            ClientOpcodeMessage::CMSG_WORLD_TELEPORT(m) => {
                if self.players.get(&guid).is_some_and(|p| p.gm) {
                    let orientation = m.orientation;
                    self.teleport(
                        guid,
                        super::MapKey::world(m.map.as_int()),
                        m.position,
                        orientation,
                    );
                }
            }
            other => trace!("{} not handled in the world", other),
        }
    }

    fn toggle_flag(&mut self, guid: Guid, flag: u32) {
        if let Some(p) = self.players.get_mut(&guid) {
            let on = !p.fields.has_flag(PLAYER_FLAGS, flag);
            p.fields.set_flag(PLAYER_FLAGS, flag, on);
            // Kept with the character: the helm and cloak stay hidden.
            if on {
                p.row.state.flags |= flag;
            } else {
                p.row.state.flags &= !flag;
            }
        }
    }

    /// A button set or cleared. As mangos's `IsActionButtonDataValid` and
    /// Thistle Tea's `ActionButtons.set` have it, a spell must be known and
    /// not passive, an item must exist, a macro is taken as it is, and any
    /// other kind is refused; a refused button leaves the bar as it was.
    fn set_action_button(&mut self, guid: Guid, m: CMSG_SET_ACTION_BUTTON) {
        let data = self.realm.data.clone();
        let Some(p) = self.players.get_mut(&guid) else {
            return;
        };
        if m.button >= 120 {
            return;
        }
        let action = u32::from(m.action);
        let clear = action == 0 && m.misc == 0 && m.action_type == 0;
        let valid = clear
            || match m.action_type {
                ACTION_BUTTON_SPELL => {
                    p.spells.contains(&action)
                        && !data.spells.get(action).is_some_and(|s| s.passive())
                }
                ACTION_BUTTON_ITEM => {
                    data.item(action).is_some() || wow_items::vanilla::lookup_item(action).is_some()
                }
                ACTION_BUTTON_MACRO | ACTION_BUTTON_CLICK_MACRO => true,
                _ => false,
            };
        if !valid {
            return;
        }
        p.row.actions.retain(|a| a.button != m.button);
        if !clear {
            p.row.actions.push(crate::game::store::ActionRow {
                button: m.button,
                action: m.action,
                kind: m.action_type,
            });
        }
    }
}
