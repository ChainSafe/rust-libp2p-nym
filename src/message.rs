use anyhow::{anyhow, Error};
use nymsphinx::addressing::clients::Recipient;

use crate::error::NymTransportError;
use crate::keys::{FakePublicKey, Keypair, PublicKey, SIGNATURE_LENGTH};

const RECIPIENT_LENGTH: usize = Recipient::LEN;

/// CONNECTION_MESSAGE_MESSAGE is the message signed in a ConnectionMessage.
pub(crate) const CONNECTION_MESSAGE_MESSAGE: &[u8] = b"NYM_CONNECTION_0";

#[derive(Debug)]
pub(crate) enum Message {
    ConnectionRequest(ConnectionMessage),
    ConnectionResponse(ConnectionMessage),
    TransportMessage(TransportMessage),
}

/// ConnectionMessage is exchanged to open a new connection.
#[derive(Default, Debug)]
pub(crate) struct ConnectionMessage {
    /// recipient is the sender's Nym address.
    /// only required if this is a ConnectionRequest.
    pub(crate) recipient: Option<Recipient>,
    pub(crate) public_key: FakePublicKey,
    pub(crate) signature: Vec<u8>,
}

/// TransportMessage is sent over a connection after establishment.
#[derive(Default, Debug)]
pub(crate) struct TransportMessage {
    pub(crate) public_key: FakePublicKey,
    pub(crate) signature: Vec<u8>,
    pub(crate) message: Vec<u8>,
}

impl Message {
    fn try_from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() < 2 {
            return Err(anyhow!(NymTransportError::InvalidMessageBytes));
        }

        Ok(match bytes[0] {
            0 => Message::ConnectionRequest(ConnectionMessage::try_from_bytes(&bytes[1..])?),
            1 => Message::ConnectionResponse(ConnectionMessage::try_from_bytes(&bytes[1..])?),
            2 => Message::TransportMessage(TransportMessage::from_bytes(&bytes[1..])),
            _ => return Err(anyhow!(NymTransportError::InvalidMessageBytes)),
        })
    }
}

impl ConnectionMessage {
    pub(crate) fn new_from_key<K: Keypair>(keypair: K) -> Self {
        ConnectionMessage {
            recipient: None,
            public_key: keypair.public_key(),
            signature: keypair.sign(CONNECTION_MESSAGE_MESSAGE),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.public_key.to_bytes();
        bytes.append(&mut self.signature.clone());
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
        let public_key = FakePublicKey::from_bytes(&bytes[0..32]);
        let signature = bytes[32..32 + SIGNATURE_LENGTH].to_vec();
        let recipient = match bytes[32 + SIGNATURE_LENGTH] {
            0u8 => None,
            1u8 => {
                let mut recipient_bytes = [0u8; RECIPIENT_LENGTH];
                recipient_bytes[..].copy_from_slice(
                    &bytes[33 + SIGNATURE_LENGTH..33 + SIGNATURE_LENGTH + RECIPIENT_LENGTH],
                );
                Some(Recipient::try_from_bytes(recipient_bytes)?)
            }
            _ => {
                return Err(anyhow!("invalid recipient prefix byte"));
            }
        };
        Ok(ConnectionMessage {
            recipient,
            public_key,
            signature,
        })
    }
}

impl TransportMessage {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.public_key.to_bytes();
        bytes.append(&mut self.signature.clone());
        bytes.append(&mut self.message.clone());
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let public_key = FakePublicKey::from_bytes(&bytes[0..32]);
        let signature = bytes[32..32 + SIGNATURE_LENGTH].to_vec();
        let message = bytes[32 + SIGNATURE_LENGTH..].to_vec();
        TransportMessage {
            public_key,
            signature,
            message,
        }
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

    pub(crate) fn verify_signature(&self) -> bool {
        true
    }
}

pub(crate) struct InboundMessage(pub Message);

pub(crate) struct OutboundMessage {
    pub message: Message,
    pub recipient: Recipient,
}

pub(crate) fn parse_message_data(data: &[u8]) -> Result<InboundMessage, Error> {
    if data.len() < 2 {
        return Err(anyhow!(NymTransportError::InvalidMessageBytes));
    }
    let msg = Message::try_from_bytes(data.to_vec())?;
    Ok(InboundMessage(msg))
}
