use crate::world::database::WorldDatabase;
use crate::world::world::client::character_screen_client::{
    CharacterScreenClient, CharacterScreenProgress,
};
use crate::world::world::get_client_login_messages;
use crate::world::world_opcode_handler::write_client_test;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::{
    Character, WorldResult, SMSG_CHAR_CREATE, SMSG_CHAR_ENUM, SMSG_PONG,
};

mod char_create;

pub async fn handle_character_screen_opcodes(
    client: &mut CharacterScreenClient,
    db: &mut WorldDatabase,
) {
    while let Ok(opcode) = client.received_messages().try_recv() {
        match opcode {
            ClientOpcodeMessage::CMSG_PING(c) => {
                client
                    .send_message(SMSG_PONG {
                        sequence_id: c.sequence_id,
                    })
                    .await;
            }
            ClientOpcodeMessage::CMSG_CHAR_ENUM => {
                let characters: Vec<Character> = db
                    .get_characters_for_account(client.account_name())
                    .into_iter()
                    .map(|a| a.into())
                    .collect();

                client.send_message(SMSG_CHAR_ENUM { characters }).await;
            }
            ClientOpcodeMessage::CMSG_CHAR_CREATE(c) => {
                let result = match char_create::create_character(c, db) {
                    Some(character) => {
                        db.create_character_in_account(client.account_name(), character);
                        WorldResult::CharCreateSuccess
                    }
                    None => {
                        tracing::warn!(
                            "CHAR_CREATE for account {} returned None; replying CharCreateError",
                            client.account_name(),
                        );
                        WorldResult::CharCreateError
                    }
                };
                client.send_message(SMSG_CHAR_CREATE { result }).await;
            }
            ClientOpcodeMessage::CMSG_CHAR_DELETE(c) => {
                db.delete_character_by_guid(client.account_name(), c.guid);
            }
            ClientOpcodeMessage::CMSG_PLAYER_LOGIN(c) => {
                let Some(character) =
                    db.get_character_for_account(client.account_name(), c.guid)
                else {
                    tracing::warn!(
                        "CMSG_PLAYER_LOGIN: account={} requested guid {:?} but no such character belongs to them",
                        client.account_name(),
                        c.guid,
                    );
                    continue;
                };
                tracing::info!(
                    "CMSG_PLAYER_LOGIN: account={} name={} race_class={:?} level={} -> sending login messages",
                    client.account_name(),
                    character.name,
                    character.race_class,
                    character.level.as_int(),
                );
                client.status = CharacterScreenProgress::WaitingToLogIn(c.guid);

                for m in get_client_login_messages(&character) {
                    client.send_opcode(&m).await;
                }
            }
            e => {
                tracing::debug!("unhandled character-screen opcode: {e:?}");
                write_client_test(&e);
            }
        }
    }
}
