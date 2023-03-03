use anyhow::{anyhow, Error};
use async_channel::{self, Receiver, Sender};
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
use tracing::debug;

use crate::connection::{Connection, InnerConnection, PendingConnection};
use crate::error::NymTransportError;
use crate::message::{ConnectionId, ConnectionMessage, InboundMessage, Message, OutboundMessage};
use crate::mixnet::Mixnet;

/// NymTransport implements the Transport trait using the Nym mixnet.
pub struct NymTransport {
    // our Nym address
    self_address: Recipient,
    pub(crate) listen_addr: Multiaddr,
    pub(crate) listener_id: ListenerId,

    // established connections
    connections: HashMap<ConnectionId, InnerConnection>,

    // outbound pending dials
    pending_dials: HashMap<ConnectionId, PendingConnection>,

    // inbound mixnet messages
    inbound_rx: Receiver<InboundMessage>,

    // outbound mixnet messages
    outbound_tx: Sender<OutboundMessage>,

    // inbound messages for Transport.poll()
    poll_rx: Receiver<TransportEvent<Upgrade, NymTransportError>>,

    // outbound messages to Transport.poll()
    poll_tx: Sender<TransportEvent<Upgrade, NymTransportError>>,

    mixnet: Mixnet,
    waker: Option<Waker>,
}

impl NymTransport {
    pub async fn new(uri: &String) -> Result<Self, Error> {
        // accept websocket uri and call Mixnet::new()
        // then, cache our Nym address and create the listener for it
        let (mut mixnet, inbound_rx, outbound_tx) = Mixnet::new(uri).await?;
        let self_address = mixnet.get_self_address().await?;
        let listen_addr = nym_address_to_multiaddress(self_address)?;
        let listener_id = ListenerId::new();

        #[allow(clippy::type_complexity)]
        let (poll_tx, poll_rx): (
            Sender<TransportEvent<Upgrade, NymTransportError>>,
            Receiver<TransportEvent<Upgrade, NymTransportError>>,
        ) = async_channel::unbounded();

        poll_tx
            .send(TransportEvent::NewAddress {
                listener_id,
                listen_addr: listen_addr.clone(),
            })
            .await?;

        Ok(Self {
            self_address,
            listen_addr,
            listener_id,
            connections: HashMap::<ConnectionId, InnerConnection>::new(),
            pending_dials: HashMap::<ConnectionId, PendingConnection>::new(),
            inbound_rx,
            outbound_tx,
            poll_rx,
            poll_tx,
            mixnet,
            waker: None,
        })
    }

    // handle_connection_response resolves the pending connection corresponding to the response
    // (if there is one) into a Connection.
    fn handle_connection_response(&mut self, msg: ConnectionMessage) -> Result<(), Error> {
        let pending_conn = self.pending_dials.get(&msg.id);
        if pending_conn.is_none() {
            return Err(anyhow!("no connection found for ConnectionRespone"));
        }

        if self.connections.contains_key(&msg.id) {
            return Err(anyhow!(
                "received ConnectionResponse but connection was already established"
            ));
        }

        let pending_conn = pending_conn.unwrap();

        // resolve connection and put into pending_conn channel
        let conn = Connection::new(pending_conn.remote_recipient, msg.id.clone());
        let inner_conn = InnerConnection::new(pending_conn.remote_recipient, msg.id.clone());
        let connection_tx = pending_conn.connection_tx.clone();

        tokio::task::spawn(async move {
            connection_tx.send(conn).await.unwrap();
        });

        self.connections.insert(msg.id, inner_conn);
        Ok(())
    }

    /// handle_connection_request handles an incoming connection request, sends back a
    /// connection response, and finally completes the upgrade into a Connection.
    fn handle_connection_request(&mut self, msg: ConnectionMessage) -> Result<Connection, Error> {
        if msg.recipient.is_none() {
            return Err(anyhow!(
                "received None recipient in ConnectionRequest: {:?}",
                msg
            ));
        }

        // ensure we don't already have a conn with the same id
        if self.connections.get(&msg.id).is_some() {
            return Err(anyhow!(
                "cannot handle connection request; already have connection with given ID"
            ));
        }

        // "outer" representation of a connection; this is returned in
        // Upgrade::poll().
        // contains channels for applications to read/write to.
        let conn = Connection::new(msg.recipient.unwrap(), msg.id.clone());

        // "inner" representation of a connection; this is what we
        // read/write to when receiving messages on the mixnet,
        // or we get outbound messages from an application.
        let inner_conn = InnerConnection::new(msg.recipient.unwrap(), msg.id.clone());

        let resp = ConnectionMessage {
            recipient: None,
            id: msg.id.clone(),
        };

        let outbound_tx = self.outbound_tx.clone();
        tokio::task::spawn(async move {
            outbound_tx
                .send(OutboundMessage {
                    message: Message::ConnectionResponse(resp),
                    recipient: msg.recipient.unwrap(),
                })
                .await
                .unwrap();
        });

        self.connections.insert(msg.id, inner_conn);
        Ok(conn)
    }
}

impl Stream for NymTransport {
    type Item = Result<Upgrade, Error>;

    // poll_next polls for inbound messages from the mixnet
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Ok(msg)) = self.inbound_rx.recv().poll_unpin(cx) {
            match msg.0 {
                Message::ConnectionRequest(inner) => {
                    debug!("got connection request {:?}", inner);
                    match self.handle_connection_request(inner) {
                        Ok(conn) => {
                            // let (connection_tx, connection_rx): (
                            //     Sender<Connection>,
                            //     Receiver<Connection>,
                            // ) = async_channel::unbounded(); // TODO: only size 1, close after?

                            let upgrade = Upgrade::new(conn);

                            // // send connection into Upgrade channel
                            // tokio::task::spawn(async move {
                            //     connection_tx.send(conn).await.unwrap();
                            // });

                            if let Some(waker) = self.waker.take() {
                                waker.wake();
                            };

                            return Poll::Ready(Some(Ok(upgrade)));
                        }
                        Err(e) => {
                            return Poll::Ready(Some(Err(anyhow!(
                                "failed to handle incoming ConnectionRequest: {:?}",
                                e
                            ))))
                        }
                    }
                }
                Message::ConnectionResponse(msg) => {
                    debug!("got connection response {:?}", msg);
                    match self.handle_connection_response(msg) {
                        Ok(()) => {
                            return Poll::Pending;
                        }
                        Err(e) => {
                            return Poll::Ready(Some(Err(anyhow!(
                                "failed to handle incoming ConnectionResponse: {:?}",
                                e
                            ))))
                        }
                    }
                }
                Message::TransportMessage(_msg) => {
                    // TODO: send to connection channel
                    return Poll::Pending;
                }
            };
        };

        Poll::Pending
    }
}

/// Upgrade represents a transport listener upgrade.
/// Note: we immediately upgrade a connection request to a connection,
/// so this only contains a channel for receiving that connection.
pub struct Upgrade {
    conn: Connection,
    //connection_rx: Receiver<Connection>,
}

impl Upgrade {
    fn new(conn: Connection) -> Upgrade {
        Upgrade { conn }
    }
}

impl Future for Upgrade {
    type Output = Result<Connection, NymTransportError>;

    // poll checks if the upgrade has turned into a connection yet
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // if let Poll::Ready(res) = self.connection_rx.recv().poll_unpin(cx) {
        //     return Poll::Ready(res.map_err(|e| NymTransportError::Other(anyhow!(e))));
        // }

        // TODO: idk if this will still be possible once Connection is fully implemented
        let conn = self.conn.clone();
        Poll::Ready(Ok(conn))
    }
}

impl Transport for NymTransport {
    type Output = Connection; // TODO: this needs to be (PeerId, Connection)
    type Error = NymTransportError;
    type ListenerUpgrade = Upgrade;
    type Dial = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn listen_on(&mut self, _: Multiaddr) -> Result<ListenerId, TransportError<Self::Error>> {
        // we should only allow listening on the multiaddress containing our Nym address
        Err(TransportError::Other(NymTransportError::Unimplemented))
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        if self.listener_id != id {
            return false;
        }

        // TODO: close channels?
        let poll_tx = self.poll_tx.clone();
        tokio::task::spawn(async move {
            poll_tx
                .send(TransportEvent::ListenerClosed {
                    listener_id: id,
                    reason: Ok(()),
                })
                .await
                .unwrap();
        });

        true
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        debug!("dialing {}", addr);

        let id = ConnectionId::generate();

        // create remote recipient address
        let recipient = multiaddress_to_nym_address(addr)
            .map_err(|e| TransportError::Other(NymTransportError::Other(e)))?;

        // create pending conn structs and store
        let (connection_tx, connection_rx): (Sender<Connection>, Receiver<Connection>) =
            async_channel::unbounded(); // TODO: make this bounded?

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
                .await
                .map_err(|e| NymTransportError::Other(anyhow!(e)))?;

            if let Some(waker) = waker.take() {
                waker.wake();
            };

            let conn = connection_rx
                .recv()
                .await
                .map_err(|e| NymTransportError::Other(anyhow!(e)))?;
            Ok(conn)
        }
        .boxed())
    }

    // dial_as_listener is unsupported.
    fn dial_as_listener(
        &mut self,
        _addr: Multiaddr,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        Err(TransportError::Other(NymTransportError::Unimplemented))
    }

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        // new addresses + listener close events
        if let Poll::Ready(Ok(res)) = self.poll_rx.recv().poll_unpin(cx) {
            return Poll::Ready(res);
        }

        // loop for mixnet events
        while let Poll::Ready(res) = self.mixnet.poll_unpin(cx) {
            // TODO: more informative events?
            debug!("got mixnet event: {:?}", res);
        }

        // check for and handle inbound messages
        if let Poll::Ready(Some(res)) = self.as_mut().poll_next(cx) {
            return match res {
                Ok(upgrade) => Poll::Ready(TransportEvent::Incoming {
                    listener_id: self.listener_id,
                    upgrade,
                    local_addr: self.listen_addr.clone(),
                    send_back_addr: self.listen_addr.clone(),
                }),
                Err(e) => Poll::Ready(TransportEvent::ListenerError {
                    listener_id: self.listener_id,
                    error: NymTransportError::Other(e),
                }),
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
    Multiaddr::from_str(&format!("/nym/{}", addr)).map_err(|e| anyhow!(e))
}

fn multiaddress_to_nym_address(multiaddr: Multiaddr) -> Result<Recipient, Error> {
    let mut multiaddr = multiaddr;
    match multiaddr.pop().unwrap() {
        Protocol::Nym(addr) => Recipient::from_str(&addr).map_err(|e| anyhow!(e)),
        _ => Err(anyhow!("unexpected protocol in multiaddress")),
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
        let maybe_conn = tokio::task::spawn(async move {
            dial.await.unwrap();
        });

        tokio::task::spawn(async move {
            // should send the connection request message and receive the response from the mixnet
            let _res = poll_fn(|cx| Pin::new(&mut dialer_transport).as_mut().poll(cx)).await;
        });

        tokio::task::spawn(async move {
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

            let _listener_conn = upgrade.await.unwrap();

            // should send the response into the mixnet
            let _res = poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx)).await;
        });

        // dialer's connection
        maybe_conn.await.unwrap();
    }
}
