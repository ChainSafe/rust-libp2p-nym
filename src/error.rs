use anyhow::Error as AnyhowError;
use async_channel::SendError;

use crate::message::OutboundMessage;

#[derive(Debug, thiserror::Error)]
pub enum NymTransportError {
    #[error("unimplemented")]
    Unimplemented,
    #[error("failed to decode message")]
    InvalidMessageBytes,
    #[error("other")]
    Other(#[from] AnyhowError),
    #[error("failed to send ConnectionRequest on outbound_tx channel")]
    DialError(#[from] Box<SendError<OutboundMessage>>),
}
