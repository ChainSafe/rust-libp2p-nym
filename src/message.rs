use anyhow::{anyhow, Error};
use nymsphinx::addressing::clients::Recipient;

pub(crate) enum Message {
    ConnectionRequest(Vec<u8>),
    Message(Vec<u8>),
    Unknown,
}

impl From<Vec<u8>> for Message {
    fn from(value: Vec<u8>) -> Self {
        if value.len() < 2 {
            return Message::Unknown;
        }

        match value[0] {
            0 => Message::ConnectionRequest(value[1..].to_vec()),
            1 => Message::Message(value[1..].to_vec()),
            _ => Message::Unknown,
        }
    }
}

impl Message {
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        match self {
            Message::ConnectionRequest(msg) => {
                let mut bytes = 0_u8.to_be_bytes().to_vec();
                bytes.append(&mut msg.clone());
                bytes
            }
            Message::Message(msg) => {
                let mut bytes = 1_u8.to_be_bytes().to_vec();
                bytes.append(&mut msg.clone());
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
