//! Newline-delimited JSON framing with a size limit.

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// Largest accepted message, excluding the newline.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("message exceeds {MAX_MESSAGE_BYTES} bytes")]
    TooLarge,
    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("message is not valid UTF-8")]
    Encoding,
}

/// Reads one message. Returns `Ok(None)` when the peer closed the connection.
/// After [`FrameError::TooLarge`] the stream is no longer aligned to message
/// boundaries and must be closed.
///
/// Not cancel safe: dropping the future while a message is only partly read
/// loses the part read so far. A connection that waits for messages and for
/// something else at once (`select!`, a timeout it recovers from) reads
/// with a [`FrameReader`].
pub async fn read_message<R, T>(reader: &mut R) -> Result<Option<T>, FrameError>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    FrameReader::new(reader).read_message().await
}

/// Reads messages from a connection, keeping a partly read message between
/// reads: [`FrameReader::read_message`] is cancel safe.
#[derive(Debug)]
pub struct FrameReader<R> {
    reader: R,
    /// The message read so far, without its newline; at most
    /// `MAX_MESSAGE_BYTES + 1` bytes.
    partial: Vec<u8>,
}

impl<R: AsyncBufRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            partial: Vec::new(),
        }
    }

    pub fn get_ref(&self) -> &R {
        &self.reader
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    /// Reads one message, as [`read_message`] does. Cancel safe: when the
    /// future is dropped before it completes, the bytes it read stay here
    /// and the next call continues the same message.
    pub async fn read_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, FrameError> {
        let complete = loop {
            // The only await: what it makes available stays in `reader`
            // until consumed below, without another await in between.
            let available = match self.reader.fill_buf().await {
                Ok(available) => available,
                Err(error) => {
                    self.partial.clear();
                    return Err(error.into());
                }
            };
            if available.is_empty() {
                break false;
            }
            // At most one byte beyond the limit, so an oversized message is
            // recognised without reading further.
            let room = MAX_MESSAGE_BYTES + 1 - self.partial.len();
            let window = &available[..available.len().min(room)];
            if let Some(end) = window.iter().position(|&byte| byte == b'\n') {
                self.partial.extend_from_slice(&window[..end]);
                self.reader.consume(end + 1);
                break true;
            }
            let read = window.len();
            self.partial.extend_from_slice(window);
            self.reader.consume(read);
            if self.partial.len() > MAX_MESSAGE_BYTES {
                break false;
            }
        };
        let message = std::mem::take(&mut self.partial);
        if !complete {
            // The peer closed the connection, possibly after part of a
            // message (decoded as it is), or the message is too large.
            if message.is_empty() {
                return Ok(None);
            }
            if message.len() > MAX_MESSAGE_BYTES {
                return Err(FrameError::TooLarge);
            }
        }
        let text = std::str::from_utf8(&message).map_err(|_| FrameError::Encoding)?;
        Ok(Some(serde_json::from_str(text.trim_end_matches('\r'))?))
    }
}

pub async fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut bytes = serde_json::to_vec(message)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(FrameError::TooLarge);
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn round_trips_and_enforces_limits() {
        let mut out = Vec::new();
        write_message(&mut out, &serde_json::json!({ "a": 1 }))
            .await
            .unwrap();
        write_message(&mut out, &serde_json::json!([1, 2]))
            .await
            .unwrap();
        let mut reader = BufReader::new(out.as_slice());
        let first: serde_json::Value = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(first["a"], 1);
        let second: serde_json::Value = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(second[1], 2);
        assert!(
            read_message::<_, serde_json::Value>(&mut reader)
                .await
                .unwrap()
                .is_none()
        );

        let mut reader = BufReader::new(&b"{not json}\n"[..]);
        assert!(matches!(
            read_message::<_, serde_json::Value>(&mut reader).await,
            Err(FrameError::Malformed(_))
        ));

        let huge = vec![b'x'; MAX_MESSAGE_BYTES + 10];
        let mut reader = BufReader::new(huge.as_slice());
        assert!(matches!(
            read_message::<_, serde_json::Value>(&mut reader).await,
            Err(FrameError::TooLarge)
        ));

        let mut reader = BufReader::new(&b"\xff\xfe\n"[..]);
        assert!(matches!(
            read_message::<_, serde_json::Value>(&mut reader).await,
            Err(FrameError::Encoding)
        ));
    }

    /// Gives up a read that waits for more of a message, as `select!` does
    /// when something else is ready first.
    async fn interrupted(reader: &mut FrameReader<impl AsyncBufRead + Unpin>) {
        let read = reader.read_message::<serde_json::Value>();
        let waited = tokio::time::timeout(std::time::Duration::from_millis(20), read).await;
        assert!(waited.is_err(), "expected the read to wait, got {waited:?}");
    }

    #[tokio::test]
    async fn an_interrupted_read_keeps_the_partial_message() {
        let (mut peer, ours) = tokio::io::duplex(4 * MAX_MESSAGE_BYTES);
        let mut reader = FrameReader::new(BufReader::new(ours));
        for part in [&br#"{"id": 1, "#[..], br#""text": "#, br#""a\nb"}"#] {
            peer.write_all(part).await.unwrap();
            interrupted(&mut reader).await;
        }
        peer.write_all(b"\r\n{\"id\": 2}\n").await.unwrap();
        let first: serde_json::Value = reader.read_message().await.unwrap().unwrap();
        assert_eq!(first, serde_json::json!({ "id": 1, "text": "a\nb" }));
        let second: serde_json::Value = reader.read_message().await.unwrap().unwrap();
        assert_eq!(second["id"], 2);
        drop(peer);
        assert!(
            reader
                .read_message::<serde_json::Value>()
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_limit_holds_across_interrupted_reads() {
        let (mut peer, ours) = tokio::io::duplex(4 * MAX_MESSAGE_BYTES);
        let mut reader = FrameReader::new(BufReader::new(ours));

        // Exactly the limit, then the newline: accepted.
        let mut exact = br#"{"id": 3}"#.to_vec();
        exact.resize(MAX_MESSAGE_BYTES, b' ');
        for part in exact.chunks(MAX_MESSAGE_BYTES / 3 + 1) {
            peer.write_all(part).await.unwrap();
            interrupted(&mut reader).await;
        }
        peer.write_all(b"\n").await.unwrap();
        let message: serde_json::Value = reader.read_message().await.unwrap().unwrap();
        assert_eq!(message["id"], 3);

        // One byte more: refused once that byte arrives, without waiting
        // for a newline, and nothing after it is read.
        let oversized = vec![b' '; MAX_MESSAGE_BYTES + 1];
        let (last, first) = oversized.split_last().unwrap();
        for part in first.chunks(MAX_MESSAGE_BYTES / 3 + 1) {
            peer.write_all(part).await.unwrap();
            interrupted(&mut reader).await;
        }
        peer.write_all(&[*last]).await.unwrap();
        peer.write_all(b"{}\n").await.unwrap();
        assert!(matches!(
            reader.read_message::<serde_json::Value>().await,
            Err(FrameError::TooLarge)
        ));
        let next: serde_json::Value = reader.read_message().await.unwrap().unwrap();
        assert_eq!(next, serde_json::json!({}));
    }

    #[tokio::test]
    async fn a_partial_message_at_the_end_is_decoded_as_it_is() {
        let (mut peer, ours) = tokio::io::duplex(1024);
        let mut reader = FrameReader::new(BufReader::new(ours));
        peer.write_all(br#"{"id":"#).await.unwrap();
        interrupted(&mut reader).await;
        drop(peer);
        assert!(matches!(
            reader.read_message::<serde_json::Value>().await,
            Err(FrameError::Malformed(_))
        ));
        assert!(
            reader
                .read_message::<serde_json::Value>()
                .await
                .unwrap()
                .is_none()
        );

        // A complete message without its newline is still read.
        let mut reader = FrameReader::new(BufReader::new(&br#"{"id": 4}"#[..]));
        let message: serde_json::Value = reader.read_message().await.unwrap().unwrap();
        assert_eq!(message["id"], 4);
    }

    #[tokio::test]
    async fn a_malformed_message_leaves_the_reader_aligned() {
        let mut reader = FrameReader::new(BufReader::new(&b"{not json}\n\xff\n{\"id\": 5}\n"[..]));
        assert!(matches!(
            reader.read_message::<serde_json::Value>().await,
            Err(FrameError::Malformed(_))
        ));
        assert!(matches!(
            reader.read_message::<serde_json::Value>().await,
            Err(FrameError::Encoding)
        ));
        let message: serde_json::Value = reader.read_message().await.unwrap().unwrap();
        assert_eq!(message["id"], 5);
    }
}
