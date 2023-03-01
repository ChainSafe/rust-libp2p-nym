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
use tracing::debug;

use crate::message::*;

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
    pub(crate) async fn new(
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

    // poll_inbound checks for the next inbound message from the mixnet and puts it
    // into the inbound channel if it finds one.
    // it blocks if there are no inbound messages.
    pub async fn poll_inbound(&mut self) -> Result<(), Error> {
        if let Some(Ok(msg)) = self.ws_stream.next().await {
            let res = parse_nym_message(msg)
                .map_err(|e| anyhow!("received unknown message: error {:?}", e))?;
            let msg_bytes = match res {
                ServerResponse::Received(msg_bytes) => {
                    debug!("received request {:?}", msg_bytes);
                    msg_bytes
                }
                ServerResponse::Error(err) => return Err(anyhow!(err)),
                _ => return Err(anyhow!("received {:?} unexpectedly", res)),
            };
            let data = parse_message_data(&msg_bytes.message).map_err(|e| anyhow!(e))?;
            self.inbound_tx.send(data)?;
            debug!("put inbound msg into channel");
        }

        Ok(())
    }

    // poll_outbound checks for the next outbound message and sends it over the mixnet.
    // it blocks if there are no outbound messages.
    pub async fn poll_outbound(&mut self) -> Result<(), Error> {
        // returns an error if the channel is closed
        let message = self.outbound_rx.recv().map_err(|e| anyhow!(e))?;
        self.write_bytes(message.recipient, &message.message.to_bytes())
            .await
            .map_err(|e| anyhow!(e))?;
        Ok(())
    }

    async fn write_bytes(&mut self, recipient: Recipient, message: &[u8]) -> Result<(), Error> {
        let nym_packet = ClientRequest::Send {
            recipient,
            message: message.to_vec(),
            connection_id: None, // TODO?
        };

        self.ws_stream
            .send(Message::Binary(nym_packet.serialize()))
            .await
            .map_err(|e| anyhow!("failed to send packet: {:?}", e))?;
        Ok(())
    }
}

async fn get_self_address(
    ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> Result<Recipient, Error> {
    let self_address_request = ClientRequest::SelfAddress.serialize();
    let response = send_message_and_get_response(ws_stream, self_address_request)
        .await
        .map_err(|e| anyhow!(e))?;
    match response {
        ServerResponse::SelfAddress(recipient) => Ok(*recipient),
        ServerResponse::Error(e) => Err(anyhow!(e)),
        _ => Err(anyhow!("received an unexpected response: {:?}", response)),
    }
}

async fn send_message_and_get_response(
    ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    req: Vec<u8>,
) -> Result<ServerResponse, Error> {
    ws_stream
        .send(Message::Binary(req))
        .await
        .map_err(|e| anyhow!(e))?;
    let raw_message = ws_stream.next().await.unwrap().map_err(|e| anyhow!(e))?;
    parse_nym_message(raw_message)
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

#[cfg(test)]
mod test {
    //use crate::keys::{FakePublicKey, SIGNATURE_LENGTH};
    use crate::message::{self, ConnectionId, Message, TransportMessage};
    use crate::mixnet::Mixnet;
    use std::thread;

    #[tokio::test]
    async fn test_mixnet_poll_inbound_and_outbound() {
        let uri = "ws://localhost:1977".to_string();
        let (mut mixnet, inbound_rx, outbound_tx) = Mixnet::new(&uri).await.unwrap();
        let self_address = mixnet.get_self_address().await.unwrap();
        let msg_inner = "hello".as_bytes();
        let msg = Message::TransportMessage(TransportMessage {
            id: ConnectionId::generate(),
            message: msg_inner.to_vec(),
        });

        // send a message to ourselves through the mixnet
        let out_msg = message::OutboundMessage {
            message: msg,
            recipient: self_address,
        };
        thread::spawn(move || {
            outbound_tx.send(out_msg).unwrap();
        });
        mixnet.poll_outbound().await.unwrap();

        // receive the message from ourselves over the mixnet
        mixnet.poll_inbound().await.unwrap();
        let received_msg = inbound_rx.recv().unwrap();
        if let Message::TransportMessage(recv_msg) = received_msg.0 {
            assert_eq!(msg_inner, recv_msg.message);
        } else {
            panic!("expected Message::TransportMessage")
        }
        mixnet.close().await.unwrap();
    }
}
