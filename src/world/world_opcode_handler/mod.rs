use crate::world::database::WorldDatabase;
use crate::world::world::client::Client;
use crate::world::world::pathfinding_maps::PathfindingMaps;
use crate::world::world_opcode_handler::entities::Entities;
use crate::world::world_opcode_handler::opcode_handler::handle_opcodes;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::ServerMessage;
#[cfg(feature = "wowm-capture")]
use {
    crate::file_utils::append_string_to_file,
    std::fs::read_to_string,
    std::path::{Path, PathBuf},
    tracing::debug,
    walkdir::WalkDir,
};

pub mod character;
pub mod chat;
pub mod combat;
pub mod creature;
pub(crate) mod entities;
pub(crate) mod gm_command;
pub mod inventory;
pub(crate) mod item;
mod opcode_handler;

#[tracing::instrument(level = "info", skip_all, name = "handle_received_client_opcodes")]
pub(crate) async fn handle_received_client_opcodes(
    client: &mut Client,
    entities: &mut Entities<'_>,
    db: &mut WorldDatabase,
    move_to_character_screen: &mut bool,
    maps: &mut PathfindingMaps,
    commands: &mut crate::world::command::CommandQueue,
) {
    while let Ok(opcode) = client.received_messages().try_recv() {
        handle_opcodes(
            client,
            entities,
            db,
            move_to_character_screen,
            opcode,
            maps,
            commands,
        )
        .await;
    }
}

/// Broadcast `message` to every in-AOI observer **and** to the source
/// `client`. The broadcast leg goes through the serialize-once
/// [`crate::world::aoi::broadcast_within_aoi`] path — only one clone of
/// the message is paid (for the local-self send), instead of one clone per
/// recipient.
async fn send_to_all(
    message: impl ServerMessage + Clone + Sync,
    client: &mut Client,
    clients: &mut slab::Slab<Client>,
) {
    let anchor = client.character().info.position;
    let anchor_map = client.character().map;
    crate::world::aoi::broadcast_within_aoi(message.clone(), anchor, anchor_map, clients).await;
    client.send_message(message).await;
}

/// Captures unhandled client opcodes into the wowm test corpus. No-op unless
/// the `wowm-capture` feature is enabled — see Cargo.toml.
#[allow(unused_variables)]
pub(crate) fn write_client_test(msg: &ClientOpcodeMessage) {
    #[cfg(feature = "wowm-capture")]
    if let Some(contents) = msg.to_test_case_string() {
        write_test_case_inner(contents.as_str(), msg.message_name());
    } else {
        tracing::debug!("unhandled client opcode (no test case): {msg:?}");
    }
}

/// Captures outgoing server opcodes into the wowm test corpus. No-op unless
/// the `wowm-capture` feature is enabled.
#[allow(unused_variables)]
pub(crate) fn write_server_test(msg: &ServerOpcodeMessage) {
    #[cfg(feature = "wowm-capture")]
    if let Some(contents) = msg.to_test_case_string() {
        write_test_case_inner(contents.as_str(), msg.message_name());
    }
}

/// Captures a serialized server message into the wowm test corpus. No-op
/// unless the `wowm-capture` feature is enabled. Used from generic send paths
/// where we don't have a concrete `ServerOpcodeMessage` value.
#[allow(unused_variables)]
pub(crate) fn write_message_test<M: ServerMessage>(msg: &M) {
    #[cfg(feature = "wowm-capture")]
    if let Some(contents) = msg.to_test_case_string() {
        write_test_case_inner(contents.as_str(), msg.message_name());
    }
}

#[cfg(feature = "wowm-capture")]
fn write_test_case_inner(contents: &str, message_name: &str) {
    if let Some(path) = find_wowm_file(message_name) {
        debug!("Added {message_name} to {path}", path = path.display());
        append_string_to_file("\n", &path);
        append_string_to_file(contents, &path);
    } else {
        let path = Path::new("./tests.wowm");
        debug!("Added {message_name} to {path}", path = path.display());
        append_string_to_file("\n", path);
        append_string_to_file(contents, path);
    }
}

#[cfg(feature = "wowm-capture")]
fn find_wowm_file(name: &str) -> Option<PathBuf> {
    let search_name = format!(" {name} ");

    for file in WalkDir::new(Path::new("../wow_messages/wow_message_parser/wowm"))
        .into_iter()
        .filter_map(|a| a.ok())
    {
        let Ok(contents) = read_to_string(file.path()) else {
            continue;
        };

        if contents.contains(&search_name) {
            return Some(file.path().to_path_buf());
        }
    }

    None
}
