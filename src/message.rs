use anyhow::{anyhow, Error};
use nymsphinx::addressing::clients::Recipient;

use crate::transport::Connection;

/// CONNECTION_MESSAGE_MESSAGE is the message signed in a ConnectionMessage.
pub(crate) const CONNECTION_MESSAGE_MESSAGE: &str = "NYM_CONNECTION_0";

pub(crate) enum Message {
    ConnectionRequest(ConnectionMessage),
    ConnectionResponse(ConnectionMessage),
    TransportMessage(TransportMessage),
    Unknown,
}

pub(crate) struct PublicKey(Vec<u8>);

impl PublicKey {
    fn to_bytes(&self) -> Vec<u8> {
        [0u8; 32].to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        PublicKey(bytes.to_vec())
    }
}

/// ConnectionMessage is exchanged to open a new connection.
pub(crate) struct ConnectionMessage {
    public_key: PublicKey,
    signature: Vec<u8>,
}

/// TransportMessage is sent over a connection after establishment.
pub(crate) struct TransportMessage {
    public_key: PublicKey,
    signature: Vec<u8>,
    message: Vec<u8>,
}

impl From<Vec<u8>> for Message {
    fn from(value: Vec<u8>) -> Self {
        if value.len() < 2 {
            return Message::Unknown;
        }

        match value[0] {
            0 => Message::ConnectionRequest(ConnectionMessage::from_bytes(&value[1..])),
            1 => Message::ConnectionResponse(ConnectionMessage::from_bytes(&value[1..])),
            2 => Message::TransportMessage(TransportMessage::from_bytes(&value[1..])),
            _ => Message::Unknown,
        }
    }
}

impl ConnectionMessage {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.public_key.to_bytes();
        bytes.append(&mut self.signature.clone());
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let public_key = PublicKey::from_bytes(&bytes[0..32]);
        let signature = bytes[32..96].to_vec();
        ConnectionMessage {
            public_key,
            signature,
        }
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
        let public_key = PublicKey::from_bytes(&bytes[0..32]);
        let signature = bytes[32..96].to_vec();
        let message = bytes[96..].to_vec();
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
            Message::Unknown => vec![2], // TODO: should this return None?
        }
    }
}

pub(crate) struct InboundMessage(pub Message);

pub(crate) struct OutboundMessage {
    pub message: Message,
    pub recipient: Recipient,
}

pub(crate) fn parse_message_data(data: &[u8]) -> Result<InboundMessage, Error> {
    if data.len() < 2 {
        return Err(anyhow!("message data too short"));
    }
    let msg = Message::from(data.to_vec());
    Ok(InboundMessage(msg))
}
