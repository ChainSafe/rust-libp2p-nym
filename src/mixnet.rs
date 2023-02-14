use anyhow::{anyhow, Error};

use futures::{SinkExt, StreamExt};
use nym_websocket::{requests::ClientRequest, responses::ServerResponse};
use nymsphinx::addressing::clients::Recipient;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, error, info, warn};

/// Mixnet implements a read/write connection to a Nym websockets endpoint.
pub struct Mixnet {
    // the websocket connection between us and the Nym endpoint we're using.
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,

    // a channel of inbound messages from the endpoint.
    // the transport reads from (listens) to the receiver of this channel.
    inbound_tx: Sender<InboundMessage>,

    // a channel of outbound messages to be written to the endpoint.
    // the transport writes to the sender of this channel.
    outbound_rx: Receiver<OutboundMessage>,
}

impl Mixnet {
    pub async fn new(
        uri: &String,
    ) -> Result<(Mixnet, Receiver<InboundMessage>, Sender<OutboundMessage>), Error> {
        let (ws_stream, _) = connect_async(uri).await.map_err(|e| anyhow!(e))?;
        let (inbound_tx, inbound_rx): (Sender<InboundMessage>, Receiver<InboundMessage>) =
            mpsc::channel();
        let (outbound_tx, outbound_rx): (Sender<OutboundMessage>, Receiver<OutboundMessage>) =
            mpsc::channel();
        Ok((
            Mixnet {
                ws_stream,
                inbound_tx,
                outbound_rx,
            },
            inbound_rx,
            outbound_tx,
        ))
    }

    pub async fn get_self_address(&mut self) -> Result<Recipient, Error> {
        let recipient = get_self_address(&mut self.ws_stream)
            .await
            .map_err(|e| anyhow!(e))?;
        Ok(recipient)
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        self.ws_stream
            .close(None)
            .await
            .map_err(|e| anyhow!("failed to close: {:?}", e))
    }

    pub async fn listen(&mut self) {
        while let Some(Ok(msg)) = self.ws_stream.next().await {
            let res = parse_nym_message(msg);
            if res.is_err() {
                warn!("received unknown message: error {:?}", res.err());
                continue;
            }

            let msg_bytes = match res.unwrap() {
                ServerResponse::Received(msg_bytes) => {
                    debug!("received request {:?}", msg_bytes);
                    msg_bytes
                }
                ServerResponse::SelfAddress(addr) => {
                    info!("listening on {}", addr);
                    continue;
                }
                ServerResponse::Error(err) => {
                    error!("received error: {}", err);
                    continue;
                }
            };

            let data_res = parse_message_data(&msg_bytes.message);
            if data_res.is_err() {
                warn!("{:?}", data_res.err());
                continue;
            }
        }
    }

    pub async fn write_bytes(&mut self, recipient: Recipient, message: &[u8]) -> Result<(), Error> {
        let nym_packet = ClientRequest::Send {
            recipient: recipient,
            message: message.to_vec(),
            with_reply_surb: false,
        };

        self.ws_stream
            .send(Message::Binary(nym_packet.serialize()))
            .await
            .map_err(|e| anyhow!("failed to send packet: {:?}", e))?;
        Ok(())
    }

    // TODO
    pub async fn dial() {}
    pub async fn send_message() {}
}

async fn get_self_address(
    ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> Result<Recipient, Error> {
    let self_address_request = ClientRequest::SelfAddress.serialize();
    let response = send_message_and_get_response(ws_stream, self_address_request).await;
    match response {
        ServerResponse::SelfAddress(recipient) => Ok(recipient),
        _ => return Err(anyhow!("received an unexpected response!")),
    }
}

async fn send_message_and_get_response(
    ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    req: Vec<u8>,
) -> ServerResponse {
    ws_stream.send(Message::Binary(req)).await.unwrap();
    let raw_message = ws_stream.next().await.unwrap().unwrap();
    match raw_message {
        Message::Binary(bin_payload) => ServerResponse::deserialize(&bin_payload).unwrap(),
        _ => panic!("received an unexpected response type!"),
    }
}

fn parse_nym_message(msg: Message) -> Result<ServerResponse, Error> {
    match msg {
        Message::Text(str) => ServerResponse::deserialize(&str.into_bytes())
            .map_err(|e| anyhow!("failed to deserialize text message: {:?}", e)),
        Message::Binary(bytes) => ServerResponse::deserialize(&bytes)
            .map_err(|e| anyhow!("failed to deserialize binary message: {:?}", e)),
        _ => Err(anyhow!("unknown message")),
    }
}

pub enum MessageType {
    ConnectionRequest,
    Message,
    Unknown,
}

impl From<u8> for MessageType {
    fn from(value: u8) -> Self {
        match value {
            0 => MessageType::ConnectionRequest,
            1 => MessageType::Message,
            _ => MessageType::Unknown,
        }
    }
}

pub struct InboundMessage {
    message_type: MessageType,
    message: Vec<u8>,
}

pub struct OutboundMessage {
    message_type: MessageType,
    message: Vec<u8>,
    recipient: Recipient,
}

fn parse_message_data(data: &[u8]) -> Result<InboundMessage, Error> {
    if data.len() < 2 {
        return Err(anyhow!("message data too short"));
    }
    let msg_type = MessageType::from(data[0]);
    Ok(InboundMessage {
        message_type: msg_type,
        message: data[1..].to_vec(),
    })
}
