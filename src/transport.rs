use futures::{future::BoxFuture, prelude::*};
use libp2p_core::{
    multiaddr::{Multiaddr, Protocol},
    transport::{ListenerId, TransportError, TransportEvent},
    Transport,
};
use nym_sphinx::addressing::clients::Recipient;
use std::{
    collections::HashMap,
    pin::Pin,
    str::FromStr,
    task::{Context, Poll, Waker},
};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::debug;

use crate::connection::{Connection, InnerConnection, PendingConnection};
use crate::error::Error;
use crate::message::{
    ConnectionId, ConnectionMessage, InboundMessage, Message, OutboundMessage, TransportMessage,
};
use crate::mixnet::initialize_mixnet;

/// InboundTransportEvent represents an inbound event from the mixnet.
pub enum InboundTransportEvent {
    ConnectionRequest(Upgrade),
    ConnectionResponse,
    TransportMessage,
}

/// NymTransport implements the Transport trait using the Nym mixnet.
pub struct NymTransport {
    /// our Nym address
    self_address: Recipient,
    pub(crate) listen_addr: Multiaddr,
    pub(crate) listener_id: ListenerId,

    /// established connections
    connections: HashMap<ConnectionId, InnerConnection>,

    /// outbound pending dials
    pending_dials: HashMap<ConnectionId, PendingConnection>,

    /// inbound mixnet messages
    inbound_stream: UnboundedReceiverStream<InboundMessage>,

    /// outbound mixnet messages
    outbound_tx: UnboundedSender<OutboundMessage>,

    /// inbound messages for Transport.poll()
    poll_rx: UnboundedReceiver<TransportEvent<Upgrade, Error>>,

    /// outbound messages to Transport.poll()
    poll_tx: UnboundedSender<TransportEvent<Upgrade, Error>>,

    waker: Option<Waker>,
}

impl NymTransport {
    pub async fn new(uri: &String) -> Result<Self, Error> {
        let (self_address, inbound_rx, outbound_tx) = initialize_mixnet(uri).await?;
        let listen_addr = nym_address_to_multiaddress(self_address)?;
        let listener_id = ListenerId::new();

        let (poll_tx, poll_rx) = unbounded_channel::<TransportEvent<Upgrade, Error>>();

        poll_tx
            .send(TransportEvent::NewAddress {
                listener_id,
                listen_addr: listen_addr.clone(),
            })
            .map_err(|_| Error::SendErrorTransportEvent)?;

        let inbound_stream = UnboundedReceiverStream::new(inbound_rx);

        Ok(Self {
            self_address,
            listen_addr,
            listener_id,
            connections: HashMap::<ConnectionId, InnerConnection>::new(),
            pending_dials: HashMap::<ConnectionId, PendingConnection>::new(),
            inbound_stream,
            outbound_tx,
            poll_rx,
            poll_tx,
            waker: None,
        })
    }

    // handle_connection_response resolves the pending connection corresponding to the response
    // (if there is one) into a Connection.
    fn handle_connection_response(&mut self, msg: ConnectionMessage) -> Result<(), Error> {
        if self.connections.contains_key(&msg.id) {
            return Err(Error::ConnectionAlreadyEstablished);
        }

        if let Some(pending_conn) = self.pending_dials.remove(&msg.id) {
            // resolve connection and put into pending_conn channel
            let (conn, inner_conn) =
                self.create_connection_types(pending_conn.remote_recipient, msg.id.clone());

            pending_conn
                .connection_tx
                .send(conn)
                .map_err(|_| Error::ConnectionSendError)?;
            self.connections.insert(msg.id, inner_conn);
            Ok(())
        } else {
            Err(Error::NoConnectionForResponse)
        }
    }

    /// handle_connection_request handles an incoming connection request, sends back a
    /// connection response, and finally completes the upgrade into a Connection.
    fn handle_connection_request(&mut self, msg: ConnectionMessage) -> Result<Connection, Error> {
        if msg.recipient.is_none() {
            return Err(Error::NoneRecipientInConnectionRequest);
        }

        // ensure we don't already have a conn with the same id
        if self.connections.get(&msg.id).is_some() {
            return Err(Error::ConnectionIDExists);
        }

        let (conn, inner_conn) =
            self.create_connection_types(msg.recipient.unwrap(), msg.id.clone());
        let resp = ConnectionMessage {
            recipient: None,
            id: msg.id.clone(),
        };

        self.outbound_tx
            .send(OutboundMessage {
                message: Message::ConnectionResponse(resp),
                recipient: msg.recipient.unwrap(),
            })
            .map_err(|e| Error::OutboundSendError(e.to_string()))?;

        self.connections.insert(msg.id, inner_conn);
        Ok(conn)
    }

    fn handle_transport_message(&self, msg: TransportMessage) -> Result<(), Error> {
        if let Some(conn) = self.connections.get(&msg.id) {
            conn.inbound_tx
                .send(msg.message)
                .map_err(|e| Error::InboundSendError(e.to_string()))?;

            if let Some(waker) = self.waker.clone().take() {
                waker.wake();
            }

            Ok(())
        } else {
            Err(Error::NoConnectionForTransportMessage)
        }
    }

    fn create_connection_types(
        &self,
        recipient: Recipient,
        id: ConnectionId,
    ) -> (Connection, InnerConnection) {
        let (inbound_tx, inbound_rx) = unbounded_channel::<Vec<u8>>();

        // "outer" representation of a connection; this contains channels for applications to read/write to.
        let conn = Connection::new(recipient, id.clone(), inbound_rx, self.outbound_tx.clone());

        // "inner" representation of a connection; this is what we
        // read/write to when receiving messages on the mixnet,
        // or we get outbound messages from an application.
        let inner_conn = InnerConnection::new(recipient, id, inbound_tx);
        (conn, inner_conn)
    }

    /// handle_inbound handles an inbound message from the mixnet, received via self.inbound_stream.
    fn handle_inbound(&mut self, msg: Message) -> Result<InboundTransportEvent, Error> {
        match msg {
            Message::ConnectionRequest(inner) => {
                debug!("got connection request {:?}", inner);
                match self.handle_connection_request(inner) {
                    Ok(conn) => {
                        if let Some(waker) = self.waker.take() {
                            waker.wake();
                        };

                        let (connection_tx, connection_rx) = oneshot::channel::<Connection>();
                        let upgrade = Upgrade::new(connection_rx);
                        connection_tx
                            .send(conn)
                            .map_err(|_| Error::ConnectionSendError)?;
                        Ok(InboundTransportEvent::ConnectionRequest(upgrade))
                    }
                    Err(e) => Err(e),
                }
            }
            Message::ConnectionResponse(msg) => {
                debug!("got connection response {:?}", msg);
                self.handle_connection_response(msg)
                    .map(|_| InboundTransportEvent::ConnectionResponse)
            }
            Message::TransportMessage(msg) => {
                debug!("got TransportMessage: {:?}", msg);
                self.handle_transport_message(msg)
                    .map(|_| InboundTransportEvent::TransportMessage)
            }
        }
    }
}

/// Upgrade represents a transport listener upgrade.
/// Note: we immediately upgrade a connection request to a connection,
/// so this only contains a channel for receiving that connection.
pub struct Upgrade {
    connection_tx: oneshot::Receiver<Connection>,
}

impl Upgrade {
    fn new(connection_tx: oneshot::Receiver<Connection>) -> Upgrade {
        Upgrade { connection_tx }
    }
}

impl Future for Upgrade {
    type Output = Result<Connection, Error>;

    // poll checks if the upgrade has turned into a connection yet
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(Ok(conn)) = self.connection_tx.poll_unpin(cx) {
            return Poll::Ready(Ok(conn));
        }

        Poll::Pending
    }
}

impl Transport for NymTransport {
    type Output = Connection; // TODO: this probably needs to be (PeerId, Connection)
    type Error = Error;
    type ListenerUpgrade = Upgrade;
    type Dial = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn listen_on(&mut self, _: Multiaddr) -> Result<ListenerId, TransportError<Self::Error>> {
        // we should only allow listening on the multiaddress containing our Nym address
        Err(TransportError::Other(Error::Unimplemented))
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        if self.listener_id != id {
            return false;
        }

        // TODO: close channels?
        self.poll_tx
            .send(TransportEvent::ListenerClosed {
                listener_id: id,
                reason: Ok(()),
            })
            .expect("failed to send listener closed event");
        true
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        debug!("dialing {}", addr);

        let id = ConnectionId::generate();

        // create remote recipient address
        let recipient = multiaddress_to_nym_address(addr).map_err(TransportError::Other)?;

        // create pending conn structs and store
        let (connection_tx, connection_rx) = oneshot::channel::<Connection>(); // TODO: make this bounded?

        let inner_pending_conn = PendingConnection::new(recipient, connection_tx);
        self.pending_dials.insert(id.clone(), inner_pending_conn);

        // put ConnectionRequest message into outbound message channel
        let msg = ConnectionMessage {
            recipient: Some(self.self_address),
            id,
        };

        let outbound_tx = self.outbound_tx.clone();

        let mut waker = self.waker.clone();
        Ok(async move {
            outbound_tx
                .send(OutboundMessage {
                    message: Message::ConnectionRequest(msg),
                    recipient,
                })
                .map_err(|e| Error::OutboundSendError(e.to_string()))?;

            debug!("sent outbound ConnectionRequest");
            if let Some(waker) = waker.take() {
                waker.wake();
            };

            let conn = connection_rx.await.map_err(Error::OneshotRecvError)?;
            Ok(conn)
        }
        .boxed())
    }

    // dial_as_listener is unsupported.
    fn dial_as_listener(
        &mut self,
        _addr: Multiaddr,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        Err(TransportError::Other(Error::Unimplemented))
    }

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        // new addresses + listener close events
        if let Poll::Ready(Some(res)) = self.poll_rx.recv().boxed().poll_unpin(cx) {
            return Poll::Ready(res);
        }

        // check for and handle inbound messages
        while let Poll::Ready(Some(msg)) = self.inbound_stream.poll_next_unpin(cx) {
            match self.handle_inbound(msg.0) {
                Ok(event) => match event {
                    InboundTransportEvent::ConnectionRequest(upgrade) => {
                        debug!("InboundTransportEvent::ConnectionRequest");
                        return Poll::Ready(TransportEvent::Incoming {
                            listener_id: self.listener_id,
                            upgrade,
                            local_addr: self.listen_addr.clone(),
                            send_back_addr: self.listen_addr.clone(),
                        });
                    }
                    InboundTransportEvent::ConnectionResponse => {
                        debug!("InboundTransportEvent::ConnectionResponse");
                    }
                    InboundTransportEvent::TransportMessage => {
                        debug!("InboundTransportEvent::TransportMessage");
                    }
                },
                Err(e) => {
                    return Poll::Ready(TransportEvent::ListenerError {
                        listener_id: self.listener_id,
                        error: e,
                    });
                }
            };
        }

        self.waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn address_translation(&self, _listen: &Multiaddr, _observed: &Multiaddr) -> Option<Multiaddr> {
        None
    }
}

fn nym_address_to_multiaddress(addr: Recipient) -> Result<Multiaddr, Error> {
    Multiaddr::from_str(&format!("/nym/{}", addr)).map_err(Error::FailedToFormatMultiaddr)
}

fn multiaddress_to_nym_address(multiaddr: Multiaddr) -> Result<Recipient, Error> {
    let mut multiaddr = multiaddr;
    match multiaddr.pop().unwrap() {
        Protocol::Nym(addr) => Recipient::from_str(&addr).map_err(Error::InvalidRecipientBytes),
        _ => Err(Error::InvalidProtocolForMultiaddr),
    }
}

#[cfg(test)]
mod test {
    use super::{nym_address_to_multiaddress, NymTransport};
    use futures::future::poll_fn;
    use libp2p_core::transport::{Transport, TransportEvent};
    use std::pin::Pin;

    #[tokio::test]
    async fn test_connection() {
        let dialer_uri = "ws://localhost:1977".to_string();
        let mut dialer_transport = NymTransport::new(&dialer_uri).await.unwrap();
        let listener_uri = "ws://localhost:1978".to_string();
        let mut listener_transport = NymTransport::new(&listener_uri).await.unwrap();
        let listener_multiaddr =
            nym_address_to_multiaddress(listener_transport.self_address).unwrap();

        let res = poll_fn(|cx| Pin::new(&mut dialer_transport).as_mut().poll(cx)).await;
        match res {
            TransportEvent::NewAddress {
                listener_id,
                listen_addr,
            } => {
                assert_eq!(listener_id, dialer_transport.listener_id);
                assert_eq!(listen_addr, dialer_transport.listen_addr);
            }
            _ => panic!("expected TransportEvent::NewAddress"),
        }

        let res = poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx)).await;
        match res {
            TransportEvent::NewAddress {
                listener_id,
                listen_addr,
            } => {
                assert_eq!(listener_id, listener_transport.listener_id);
                assert_eq!(listen_addr, listener_transport.listen_addr);
            }
            _ => panic!("expected TransportEvent::NewAddress"),
        }

        // dial the remote peer; put awaiting the dial into a thread
        let dial = dialer_transport.dial(listener_multiaddr).unwrap();
        let maybe_dialer_conn = tokio::task::spawn(async move { dial.await.unwrap() });

        tokio::task::spawn(async move {
            // should send the connection request message and receive the response from the mixnet
            poll_fn(|cx| Pin::new(&mut dialer_transport).as_mut().poll(cx)).await;
        });

        let maybe_listener_conn = tokio::task::spawn(async move {
            // should receive the connection request from the mixnet
            let res = poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx)).await;
            let upgrade = match res {
                TransportEvent::Incoming {
                    listener_id,
                    upgrade,
                    local_addr,
                    send_back_addr,
                } => {
                    assert_eq!(listener_id, listener_transport.listener_id);
                    assert_eq!(local_addr, listener_transport.listen_addr);
                    assert_eq!(send_back_addr, listener_transport.listen_addr);
                    upgrade
                }
                _ => panic!("expected TransportEvent::Incoming, got {:?}", res),
            };

            tokio::task::spawn(async move {
                // should send the connection response into the mixnet
                poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx)).await;
            });

            let listener_conn = upgrade.await.unwrap();
            listener_conn
        });

        println!("waiting for connections...");
        let mut dialer_conn = maybe_dialer_conn.await.unwrap();
        let mut listener_conn = maybe_listener_conn.await.unwrap();

        // write a message from the dialer to the listener
        let msg_string = b"hello".to_vec();
        dialer_conn.write(msg_string.clone()).await.unwrap();
        let msg = listener_conn.inbound_rx.recv().await.unwrap();
        assert_eq!(msg, msg_string);

        // write a message from the dialer to the listener again
        let msg_string = b"hi".to_vec();
        dialer_conn.write(msg_string.clone()).await.unwrap();
        let msg = listener_conn.inbound_rx.recv().await.unwrap();
        assert_eq!(msg, msg_string);

        // write a message from the listener to the dialer
        let msg_string = b"world".to_vec();
        listener_conn.write(msg_string.clone()).await.unwrap();
        let msg = dialer_conn.inbound_rx.recv().await.unwrap();
        assert_eq!(msg, msg_string);
    }
}
