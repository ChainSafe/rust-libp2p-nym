use nymsphinx::addressing::clients::RecipientFormattingError;
use std::sync::mpsc::SendError;

use crate::message::OutboundMessage;

#[derive(Debug, thiserror::Error)]
pub enum NymTransportError {
    #[error("unimplemented")]
    Unimplemented,
    #[error("failed to decode message")]
    InvalidMessageBytes,
    #[error("invalid Nym multiaddress")]
    InvalidNymMultiaddress(#[from] RecipientFormattingError),
    #[error("failed to send ConnectionRequest on outbound_tx channel")]
    DialError(#[from] Box<SendError<OutboundMessage>>),
}
