use anyhow::{anyhow, Error};
use futures::future::BoxFuture;
use futures::{prelude::*, stream::SelectAll};
use libp2p_core::{
    multiaddr::Multiaddr,
    transport::{ListenerId, TransportError, TransportEvent},
    Transport,
};
use nym_sphinx::addressing::clients::{Recipient, RecipientFormattingError};
use std::{
    collections::HashMap,
    pin::Pin,
    str::FromStr,
    sync::mpsc::{self, Receiver, Sender},
    task::{Context, Poll},
};
use tracing::debug;

use crate::connection::{Connection, InnerConnection, InnerPendingConnection, PendingConnection};
use crate::error::NymTransportError;
use crate::message::{ConnectionId, ConnectionMessage, InboundMessage, Message, OutboundMessage};
use crate::mixnet::Mixnet;

/// Listener listens for new inbound connection requests.
pub struct Listener {
    id: ListenerId,
    listen_addr: Multiaddr,

    // receives Upgrades; handling has already been done in handle_connection_request
    inbound_rx: Receiver<Upgrade>,
}

impl Listener {
    fn new(id: ListenerId, listen_addr: Multiaddr, inbound_rx: Receiver<Upgrade>) -> Self {
        Self {
            id,
            listen_addr,
            inbound_rx,
        }
    }

    fn close(&self) {
        // TODO
    }
}

impl Stream for Listener {
    type Item = TransportEvent<Upgrade, NymTransportError>;

    /// poll_next should return any inbound messages on the listener.
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // poll self.inbound_rx and emit TransportEvent::Incoming if we get an Upgrade
        if let Ok(upgrade) = self.inbound_rx.recv() {
            return Poll::Ready(Some(TransportEvent::Incoming {
                listener_id: self.id,
                upgrade,
                local_addr: self.listen_addr.clone(),
                send_back_addr: self.listen_addr.clone(),
            }));
        }

        Poll::Pending
    }
}

/// NymTransport implements the Transport trait using the Nym mixnet.
pub struct NymTransport {
    // our Nym address
    self_address: Recipient,

    // established connections
    connections: HashMap<ConnectionId, InnerConnection>,

    // outbound pending dials
    pending_dials: HashMap<ConnectionId, InnerPendingConnection>,

    // inbound connection requests - sent to Listener::poll()
    connection_req_tx: Sender<Upgrade>,

    // inbound mixnet messages
    inbound_rx: Receiver<InboundMessage>,

    // outbound mixnet messages
    outbound_tx: Sender<OutboundMessage>,

    // listeners for inbound connection requests
    listeners: SelectAll<Listener>,

    // inbound messages for Transport.poll()
    poll_rx: Receiver<TransportEvent<Upgrade, NymTransportError>>,

    // outbound messages to Transport.poll()
    poll_tx: Sender<TransportEvent<Upgrade, NymTransportError>>,
}

fn nym_address_to_multiaddress(addr: Recipient) -> Result<Multiaddr, Error> {
    Multiaddr::from_str(&format!("/nym/{:?}", addr.to_string())).map_err(|e| anyhow!(e))
}

fn multiaddress_to_nym_address(
    multiaddr: Multiaddr,
) -> Result<Recipient, RecipientFormattingError> {
    let mut addr = multiaddr;
    Recipient::from_str(&addr.pop().unwrap().to_string())
    //.map_err(|e| TransportError::Other(NymTransportError::InvalidNymMultiaddress(e)))?;
}

impl NymTransport {
    pub async fn new(uri: &String) -> Result<Self, Error> {
        // accept websocket uri and call Mixnet::new()
        // then, cache our Nym address and create the listener for it

        let (mut mixnet, inbound_rx, outbound_tx) = Mixnet::new(uri).await?;
        let self_address = mixnet.get_self_address().await?;
        let listen_addr = nym_address_to_multiaddress(self_address)?;

        // inbound connection requests only; used from sending between transport_inbound_loop()
        // and Listener::poll().
        let (connection_req_tx, connection_req_rx): (Sender<Upgrade>, Receiver<Upgrade>) =
            mpsc::channel();

        let mut listeners = SelectAll::<Listener>::new();
        let listener_id = ListenerId::new();
        listeners.push(Listener::new(
            listener_id,
            listen_addr.clone(),
            connection_req_rx,
        ));

        let (poll_tx, poll_rx): (
            Sender<TransportEvent<Upgrade, NymTransportError>>,
            Receiver<TransportEvent<Upgrade, NymTransportError>>,
        ) = mpsc::channel();

        poll_tx
            .send(TransportEvent::NewAddress {
                listener_id,
                listen_addr,
            })
            .unwrap();

        Ok(Self {
            self_address,
            connections: HashMap::<ConnectionId, InnerConnection>::new(),
            pending_dials: HashMap::<ConnectionId, InnerPendingConnection>::new(),
            connection_req_tx,
            inbound_rx,
            outbound_tx,
            listeners,
            poll_rx,
            poll_tx,
        })
    }

    fn transport_inbound_loop(&mut self) {
        loop {
            // TODO: return if channel closes
            if let Ok(msg) = self.inbound_rx.recv() {
                if !msg.0.verify_signature() {
                    debug!("failed to verify message signature: {:?}", msg.0);
                    continue;
                }

                match msg.0 {
                    Message::ConnectionRequest(inner) => {
                        match self.handle_connection_request(inner) {
                            Ok(conn) => {
                                let (connection_tx, connection_rx): (
                                    Sender<Connection>,
                                    Receiver<Connection>,
                                ) = mpsc::channel();
                                let upgrade = Upgrade::new(connection_rx);
                                // send connection into Upgrade channel
                                connection_tx.send(conn).unwrap();

                                // send upgrade to listener channel
                                if self.connection_req_tx.send(upgrade).is_err() {
                                    debug!("failed to send ConnectionRequest to listener channel");
                                }
                            }
                            Err(e) => {
                                debug!("failed to handle incoming ConnectionRequest: {:?}", e);
                                continue;
                            }
                        }

                        break;
                    }
                    Message::ConnectionResponse(msg) => {
                        match self.handle_connection_response(msg) {
                            Ok(()) => {}
                            Err(e) => {
                                debug!("failed to handle incoming ConnectionResponse: {:?}", e);
                                continue;
                            }
                        }
                    }
                    Message::TransportMessage(msg) => {
                        // TODO: send to connection channel
                    }
                }
            }
        }
    }

    // handle_connection_response resolves the pending connection corresponding to the response
    // (if there is one) into a Connection.
    fn handle_connection_response(&mut self, msg: ConnectionMessage) -> Result<(), Error> {
        let inner_pending_conn = self.pending_dials.get(&msg.id);
        if inner_pending_conn.is_none() {
            return Err(anyhow!("no connection found for ConnectionRespone"));
        }

        if self.connections.contains_key(&msg.id) {
            return Err(anyhow!(
                "received ConnectionResponse but connection was already established"
            ));
        }

        let inner_pending_conn = inner_pending_conn.unwrap();

        // resolve connection and put into inner_pending_conn channel
        let conn = Connection::new(inner_pending_conn.remote_recipient, msg.id.clone());
        let inner_conn = InnerConnection::new(inner_pending_conn.remote_recipient, msg.id.clone());
        inner_pending_conn.connection_tx.send(conn)?;
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

        let id = ConnectionId::generate();

        // "outer" representation of a connection; this is returned in
        // Upgrade::poll().
        // contains channels for applications to read/write to.
        let conn = Connection::new(msg.recipient.unwrap(), id.clone());

        // "inner" representation of a connection; this is what we
        // read/write to when receiving messages on the mixnet,
        // or we get outbound messages from an application.
        let inner_conn = InnerConnection::new(msg.recipient.unwrap(), id.clone());

        let resp = ConnectionMessage {
            recipient: None,
            id: id.clone(),
        };

        self.outbound_tx.send(OutboundMessage {
            message: Message::ConnectionResponse(resp),
            recipient: msg.recipient.unwrap(),
        })?;

        self.connections.insert(id, inner_conn);
        Ok(conn)
    }
}

/// Upgrade represents a transport listener upgrade.
/// Note: we immediately upgrade a connection request to a connection,
/// so this only contains a channel for receiving that connection.
pub struct Upgrade {
    connection_rx: Receiver<Connection>,
}

impl Upgrade {
    fn new(connection_rx: Receiver<Connection>) -> Upgrade {
        Upgrade { connection_rx }
    }
}

impl Future for Upgrade {
    type Output = Result<Connection, NymTransportError>;

    // poll checks if the upgrade has turned into a connection yet
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Ok(conn) = self.connection_rx.recv() {
            return Poll::Ready(Ok(conn));
        }

        Poll::Pending
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
        if let Some(listener) = self.listeners.iter_mut().find(|l| l.id == id) {
            listener.close();
            if let Err(e) = self.poll_tx.send(TransportEvent::ListenerClosed {
                listener_id: id,
                reason: Ok(()),
            }) {
                debug!("failed to send TransportEvent::ListenerClosed: {:?}", e);
                return false;
            }
            true
        } else {
            false
        }
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        let id = ConnectionId::generate();

        // create remote recipient address
        // TODO: we need to fork and update the multiaddress crate to support the Nym transport
        //let mut addr = addr;
        //let recipient = Recipient::from_str(&addr.pop().unwrap().to_string())
        let recipient = multiaddress_to_nym_address(addr)
            .map_err(|e| TransportError::Other(NymTransportError::InvalidNymMultiaddress(e)))?;

        // create pending conn structs and store
        let (connection_tx, connection_rx): (Sender<Connection>, Receiver<Connection>) =
            mpsc::channel();
        let pending_conn = PendingConnection::new(connection_rx);
        let inner_pending_conn = InnerPendingConnection::new(recipient, connection_tx);
        self.pending_dials.insert(id.clone(), inner_pending_conn);

        // put ConnectionRequest message into outbound message channel
        let msg = ConnectionMessage {
            recipient: Some(self.self_address),
            id,
        };

        self.outbound_tx
            .send(OutboundMessage {
                message: Message::ConnectionRequest(msg),
                recipient,
            })
            .map_err(|e| TransportError::Other(NymTransportError::DialError(Box::new(e))))?;
        Ok(Box::pin(pending_conn))
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
        // check for and handle inbound messages
        self.transport_inbound_loop();

        // new addresses? etc
        if let Ok(ev) = self.poll_rx.recv() {
            return Poll::Ready(ev);
        }

        // new inbound connections?
        if let Poll::Ready(Some(ev)) = self.listeners.poll_next_unpin(cx) {
            return Poll::Ready(ev);
        }

        Poll::Pending
    }

    fn address_translation(&self, _listen: &Multiaddr, _observed: &Multiaddr) -> Option<Multiaddr> {
        None
    }
}

#[cfg(test)]
mod test {
    use super::{nym_address_to_multiaddress, NymTransport};
    use libp2p_core::transport::Transport;

    #[tokio::test]
    async fn test_connection() {
        let uri = "ws://localhost:1977".to_string();
        let mut dialer_transport = NymTransport::new(&uri).await.unwrap();
        let listener_transport = NymTransport::new(&uri).await.unwrap();
        let listener_multiaddr =
            nym_address_to_multiaddress(listener_transport.self_address).unwrap();
        dialer_transport
            .dial(listener_multiaddr)
            .unwrap()
            .await
            .unwrap();
    }
}
