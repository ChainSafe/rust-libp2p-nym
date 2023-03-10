use nym_sphinx::addressing::clients::Recipient;
use rand_core::{OsRng, RngCore};

use crate::error::Error;

const RECIPIENT_LENGTH: usize = Recipient::LEN;
const CONNECTION_ID_LENGTH: usize = 32;

/// ConnectionId is a unique, randomly-generated per-connection ID that's used to
/// identity which connection a message belongs to.
#[derive(Clone, Default, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConnectionId([u8; 32]);

impl ConnectionId {
    pub(crate) fn generate() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        ConnectionId(bytes)
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let mut id = [0u8; 32];
        id[..].copy_from_slice(&bytes[0..CONNECTION_ID_LENGTH]);
        ConnectionId(id)
    }
}

#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum Message {
    ConnectionRequest(ConnectionMessage),
    ConnectionResponse(ConnectionMessage),
    TransportMessage(TransportMessage),
}

/// ConnectionMessage is exchanged to open a new connection.
#[derive(Default, Debug)]
pub(crate) struct ConnectionMessage {
    pub(crate) id: ConnectionId,
    /// recipient is the sender's Nym address.
    /// only required if this is a ConnectionRequest.
    pub(crate) recipient: Option<Recipient>,
}

/// TransportMessage is sent over a connection after establishment.
#[derive(Default, Debug)]
pub(crate) struct TransportMessage {
    pub(crate) message: Vec<u8>,
    pub(crate) id: ConnectionId,
}

impl Message {
    fn try_from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() < 2 {
            return Err(Error::InvalidMessageBytes);
        }

        Ok(match bytes[0] {
            0 => Message::ConnectionRequest(ConnectionMessage::try_from_bytes(&bytes[1..])?),
            1 => Message::ConnectionResponse(ConnectionMessage::try_from_bytes(&bytes[1..])?),
            2 => Message::TransportMessage(TransportMessage::try_from_bytes(&bytes[1..])?),
            _ => return Err(Error::InvalidMessageBytes),
        })
    }
}

impl ConnectionMessage {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.id.0.to_vec();
        match self.recipient {
            Some(recipient) => {
                bytes.push(1u8);
                bytes.append(&mut recipient.to_bytes().to_vec());
            }
            None => bytes.push(0u8),
        }
        bytes
    }

    fn try_from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < CONNECTION_ID_LENGTH + 1 {
            return Err(Error::ConnectionMessageBytesTooShort);
        }

        let id = ConnectionId::from_bytes(&bytes[0..CONNECTION_ID_LENGTH]);
        let recipient = match bytes[CONNECTION_ID_LENGTH] {
            0u8 => None,
            1u8 => {
                if bytes.len() < CONNECTION_ID_LENGTH + 1 + RECIPIENT_LENGTH {
                    return Err(Error::ConnectionMessageBytesNoRecipient);
                }

                let mut recipient_bytes = [0u8; RECIPIENT_LENGTH];
                recipient_bytes[..].copy_from_slice(
                    &bytes[CONNECTION_ID_LENGTH + 1..CONNECTION_ID_LENGTH + 1 + RECIPIENT_LENGTH],
                );
                Some(
                    Recipient::try_from_bytes(recipient_bytes)
                        .map_err(Error::InvalidRecipientBytes)?,
                )
            }
            _ => {
                return Err(Error::InvalidRecipientPrefixByte);
            }
        };
        Ok(ConnectionMessage { recipient, id })
    }
}

impl TransportMessage {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.id.0.to_vec();
        bytes.append(&mut self.message.clone());
        bytes
    }

    fn try_from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < CONNECTION_ID_LENGTH {
            return Err(Error::TransportMessageBytesTooShort);
        }

        let id = ConnectionId::from_bytes(&bytes[0..CONNECTION_ID_LENGTH]);
        let message = bytes[CONNECTION_ID_LENGTH..].to_vec();
        Ok(TransportMessage { message, id })
    }
}

impl Message {
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        match self {
            Message::ConnectionRequest(msg) => {
                let mut bytes = 0_u8.to_be_bytes().to_vec();
                bytes.append(&mut msg.to_bytes());
                bytes
            }
            Message::ConnectionResponse(msg) => {
                let mut bytes = 1_u8.to_be_bytes().to_vec();
                bytes.append(&mut msg.to_bytes());
                bytes
            }
            Message::TransportMessage(msg) => {
                let mut bytes = 2_u8.to_be_bytes().to_vec();
                bytes.append(&mut msg.to_bytes());
                bytes
            }
        }
    }
}

/// InboundMessage represents an inbound mixnet message.
pub(crate) struct InboundMessage(pub(crate) Message);

/// OutboundMessage represents an outbound mixnet message.
#[derive(Debug)]
pub(crate) struct OutboundMessage {
    pub(crate) message: Message,
    pub(crate) recipient: Recipient,
}

pub(crate) fn parse_message_data(data: &[u8]) -> Result<InboundMessage, Error> {
    if data.len() < 2 {
        return Err(Error::InvalidMessageBytes);
    }
    let msg = Message::try_from_bytes(data.to_vec())?;
    Ok(InboundMessage(msg))
}
