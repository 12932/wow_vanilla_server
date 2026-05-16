//! Orchestrator ↔ worker wire protocol.
//!
//! Length-prefixed postcard frames (`u32 LE size || postcard payload`).

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToWorker {
    Spawn { count: u32 },
    Stop { count: u32 },
    Drain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToOrchestrator {
    Hello {
        worker_id: String,
        started_at_unix: u64,
    },
    Metrics(WorkerMetrics),
    Drained,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkerMetrics {
    pub bots_alive: u32,
    pub auth_ok: u64,
    pub auth_fail: u64,
    pub world_ok: u64,
    pub world_fail: u64,
    pub bytes_in_total: u64,
    pub bytes_out_total: u64,
    pub send_errors: u64,
    pub messages_in: u64,
    pub messages_out: u64,
}

/// Sentinel rejecting absurdly large frames so a desynced socket cannot OOM us.
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

pub async fn write_frame<W, T>(w: &mut W, value: &T) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = postcard::to_allocvec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(bytes.len()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large")
    })?;
    w.write_all(&len.to_le_bytes()).await?;
    w.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_frame<R, T>(r: &mut R) -> std::io::Result<T>
where
    R: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {len}"),
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
