use futures::{future::Future, FutureExt, SinkExt, StreamExt};
use nym_sphinx::addressing::clients::Recipient;
use nym_websocket::{requests::ClientRequest, responses::ServerResponse};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::{
    net::TcpStream,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};
use tracing::debug;

use crate::error::Error;
use crate::message::*;

/// Mixnet implements a read/write connection to a Nym websockets endpoint.
pub(crate) struct Mixnet {
    /// the websocket connection between us and the Nym endpoint we're using.
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,

    /// a channel of inbound messages from the endpoint.
    /// the transport reads from (listens) to the receiver of this channel.
    inbound_tx: UnboundedSender<InboundMessage>,

    /// a channel of outbound messages to be written to the endpoint.
    /// the transport writes to the sender of this channel.
    outbound_rx: UnboundedReceiver<OutboundMessage>,
}

impl Mixnet {
    pub(crate) async fn new(
        uri: &String,
    ) -> Result<
        (
            Mixnet,
            UnboundedReceiver<InboundMessage>,
            UnboundedSender<OutboundMessage>,
        ),
        Error,
    > {
        let (ws_stream, _) = connect_async(uri)
            .await
            .map_err(Error::WebsocketStreamError)?;
        let (inbound_tx, inbound_rx) = unbounded_channel::<InboundMessage>();
        let (outbound_tx, outbound_rx) = unbounded_channel::<OutboundMessage>();
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

    pub(crate) async fn get_self_address(&mut self) -> Result<Recipient, Error> {
        let recipient = get_self_address(&mut self.ws_stream).await?;
        Ok(recipient)
    }

    #[allow(dead_code)]
    pub(crate) async fn close(&mut self) -> Result<(), Error> {
        self.ws_stream
            .close(None)
            .await
            .map_err(Error::WebsocketStreamError)
    }

    async fn check_inbound(&mut self) -> Result<(), Error> {
        if let Some(res) = self.ws_stream.next().await {
            debug!("got inbound message from mixnet: {:?}", res);
            match res {
                Ok(msg) => return self.handle_inbound(msg).await,
                Err(e) => return Err(Error::WebsocketStreamError(e)),
            }
        }

        Err(Error::WebsocketStreamReadNone)
    }

    async fn handle_inbound(&self, msg: Message) -> Result<(), Error> {
        let res = parse_nym_message(msg)?;
        let msg_bytes = match res {
            ServerResponse::Received(msg_bytes) => {
                debug!("received request {:?}", msg_bytes);
                msg_bytes
            }
            ServerResponse::Error(e) => return Err(Error::NymMessageError(e.to_string())),
            _ => return Err(Error::UnexpectedNymMessage),
        };
        let data = parse_message_data(&msg_bytes.message)?;
        self.inbound_tx
            .send(data)
            .map_err(|e| Error::InboundSendError(e.to_string()))
    }

    async fn check_outbound(&mut self) -> Result<(), Error> {
        match self.outbound_rx.recv().await {
            Some(message) => {
                self.write_bytes(message.recipient, &message.message.to_bytes())
                    .await
            }
            None => Err(Error::RecvError),
        }
    }

    async fn write_bytes(&mut self, recipient: Recipient, message: &[u8]) -> Result<(), Error> {
        let nym_packet = ClientRequest::Send {
            recipient,
            message: message.to_vec(),
            connection_id: None,
        };

        self.ws_stream
            .send(Message::Binary(nym_packet.serialize()))
            .await
            .map_err(Error::WebsocketStreamError)?;

        debug!(
            "wrote message to mixnet: recipient: {:?}",
            recipient.to_string()
        );
        Ok(())
    }
}

impl Future for Mixnet {
    type Output = Result<MixnetEvent, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(res) = Box::pin(self.check_outbound()).poll_unpin(cx) {
            return Poll::Ready(res.map(|_| MixnetEvent::Outbound));
        }

        if let Poll::Ready(res) = Box::pin(self.check_inbound()).poll_unpin(cx) {
            return Poll::Ready(res.map(|_| MixnetEvent::Inbound));
        }

        Poll::Pending
    }
}

#[derive(Debug)]
pub(crate) enum MixnetEvent {
    Inbound,
    Outbound,
}

async fn get_self_address(
    ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> Result<Recipient, Error> {
    let self_address_request = ClientRequest::SelfAddress.serialize();
    ws_stream
        .send(Message::Binary(self_address_request))
        .await
        .map_err(Error::WebsocketStreamError)?;
    
    // loop until we receive the SelfAddress respone, since the next message might not
    // necessarily be the SelfAddress response.
    while let Some(raw_message) = ws_stream.next().await {
        let raw_message = raw_message.map_err(Error::WebsocketStreamError)?;
        let response = parse_nym_message(raw_message)?;
        return match response {
            ServerResponse::SelfAddress(recipient) => Ok(*recipient),
            ServerResponse::Error(e) => Err(Error::NymMessageError(e.to_string())),
            _ => continue,
        };
    }
    Err(Error::RecvError)
}

fn parse_nym_message(msg: Message) -> Result<ServerResponse, Error> {
    match msg {
        Message::Text(str) => ServerResponse::deserialize(&str.into_bytes())
            .map_err(|e| Error::NymMessageError(e.to_string())),
        Message::Binary(bytes) => {
            ServerResponse::deserialize(&bytes).map_err(|e| Error::NymMessageError(e.to_string()))
        }
        _ => Err(Error::UnknownNymMessage),
    }
}

#[cfg(test)]
mod test {
    use crate::message::{self, ConnectionId, Message, TransportMessage};
    use crate::mixnet::Mixnet;

    #[tokio::test]
    async fn test_mixnet_poll_inbound_and_outbound() {
        let uri = "ws://localhost:1977".to_string();
        let (mut mixnet, mut inbound_rx, outbound_tx) = Mixnet::new(&uri).await.unwrap();
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

        outbound_tx.send(out_msg).unwrap();

        tokio::task::spawn(async move {
            mixnet.check_outbound().await.unwrap();
            mixnet.check_inbound().await.unwrap();
            mixnet.close().await.unwrap();
        });

        // receive the message from ourselves over the mixnet
        let received_msg = inbound_rx.recv().await.unwrap();
        if let Message::TransportMessage(recv_msg) = received_msg.0 {
            assert_eq!(msg_inner, recv_msg.message);
        } else {
            panic!("expected Message::TransportMessage")
        }
    }
}
