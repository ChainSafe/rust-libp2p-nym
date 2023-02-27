use anyhow::{anyhow, Error};
use futures::future::BoxFuture;
use futures::{prelude::*, stream::SelectAll};
use libp2p_core::{
    multiaddr::Multiaddr,
    transport::{ListenerId, TransportError, TransportEvent},
    Transport,
};
use nymsphinx::addressing::clients::Recipient;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::{
    pin::Pin,
    str::FromStr,
    task::{Context, Poll},
    thread,
};
use tracing::debug;

use crate::error::NymTransportError;
use crate::keys::{FakeKeypair, FakePublicKey, Keypair};
use crate::message::{ConnectionMessage, InboundMessage, Message, OutboundMessage};
use crate::mixnet::Mixnet;

pub struct Listener {
    id: ListenerId,
    listen_addr: Multiaddr,

    // receives inbound ConnectionRequest messages
    inbound_rx: Receiver<ConnectionMessage>,
}

impl Listener {
    fn new(
        id: ListenerId,
        listen_addr: Multiaddr,
        inbound_rx: Receiver<ConnectionMessage>,
    ) -> Self {
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
        // poll self.inbound_rx and emit TransportEvent::Incoming if we get a ConnectionRequest
        if let Ok(msg) = self.inbound_rx.recv() {
            if msg.recipient.is_none() {
                debug!("received None recipient in ConnectionRequest: {:?}", msg);
                return Poll::Pending; // TODO: should this check the channel again?
            }

            return Poll::Ready(Some(TransportEvent::Incoming {
                listener_id: self.id,
                upgrade: Upgrade::new(msg.recipient.unwrap(), msg.public_key),
                local_addr: self.listen_addr.clone(),
                send_back_addr: self.listen_addr.clone(),
            }));
        }

        Poll::Pending
    }
}

/// NymTransport implements the Transport trait using the Nym mixnet.
pub struct NymTransport {
    self_address: Recipient,

    connections: HashMap<FakePublicKey, Connection>,

    pending_dials: HashMap<FakePublicKey, Upgrade>,

    // all inbound messages
    // inbound_rx: Receiver<InboundMessage>,
    // outbound messages
    outbound_tx: Sender<OutboundMessage>,

    listeners: SelectAll<Listener>,

    // inbound messages for Transport.poll()
    poll_rx: Receiver<TransportEvent<Upgrade, NymTransportError>>,
    // outbound messages to Transport.poll()
    poll_tx: Sender<TransportEvent<Upgrade, NymTransportError>>,
}

impl NymTransport {
    pub async fn new(uri: &String) -> Result<Self, Error> {
        // accept websocket uri and call Mixnet::new()
        // then, cache our Nym address and create the listener for it

        let (mut mixnet, inbound_rx, outbound_tx) = Mixnet::new(uri).await?;
        let self_address = mixnet.get_self_address().await?;
        let listen_addr =
            Multiaddr::from_str(&format!("/nym/{:?}", self_address)).map_err(|e| anyhow!(e))?;

        // inbound connection requests only; used from sending between transport_inbound_loop()
        // and Listener::poll().
        let (connection_req_tx, connection_req_rx): (
            Sender<ConnectionMessage>,
            Receiver<ConnectionMessage>,
        ) = mpsc::channel();

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

        poll_tx.send(TransportEvent::NewAddress {
            listener_id,
            listen_addr,
        })?;

        thread::spawn(move || {
            transport_inbound_loop(inbound_rx, connection_req_tx);
        });

        Ok(Self {
            self_address,
            connections: HashMap::<FakePublicKey, Connection>::new(),
            pending_dials: HashMap::<FakePublicKey, Upgrade>::new(),
            outbound_tx,
            listeners,
            poll_rx,
            poll_tx,
        })
    }

    /// handle_incoming handles an incoming connection request, sends back a
    /// connection response, and finally completes the upgrade into a Connection.
    fn handle_incoming(&self, upgrade: &Upgrade) {
        // TODO
    }
}

// this runs in a thread when the transport is constructed.
fn transport_inbound_loop(
    inbound_rx: Receiver<InboundMessage>,
    connection_req_tx: Sender<ConnectionMessage>,
) {
    loop {
        if let Ok(msg) = inbound_rx.recv() {
            if !msg.0.verify_signature() {
                debug!("failed to verify message signature: {:?}", msg.0);
                continue;
            }

            match msg.0 {
                Message::ConnectionRequest(inner) => {
                    // send to listener channel
                    if connection_req_tx.send(inner).is_err() {
                        debug!("failed to send ConnectionRequest to listener channel");
                    }
                }
                Message::ConnectionResponse(data) => {
                    // TODO: resolve connection
                }
                Message::TransportMessage(data) => {
                    // TODO: send to connection channel
                }
            }
        }
    }
}

/// Connection represents the result of a connection setup process.
#[derive(Debug)]
pub struct Connection {
    //remote_id: Recipient,
    //remote_public_key: FakePublicKey,
    keypair: FakeKeypair,
}

impl Connection {
    pub(crate) fn new(keypair: FakeKeypair) -> Self {
        Connection {
            //remote_id: Recipient::default(),
            keypair,
        }
    }
}

/// PendingConnection represents a potential connection;
/// ie. a ConnectionRequest has been sent out, but we haven't
/// gotten the response yet.
pub(crate) struct PendingConnection {
    remote_recipient: Recipient,
    remote_public_key: FakePublicKey,
    keypair: FakeKeypair,
}

/// Upgrade represents a transport upgrade.
pub struct Upgrade(pub(crate) PendingConnection);

impl Upgrade {
    fn new(remote_recipient: Recipient, remote_public_key: FakePublicKey) -> Self {
        Upgrade(PendingConnection {
            remote_recipient: remote_recipient,
            remote_public_key: remote_public_key,
            keypair: FakeKeypair::generate(),
        })
    }
}

impl Future for Upgrade {
    type Output = Result<Connection, NymTransportError>;

    // poll checks if the upgrade has turned into a connection yet
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // TODO: check inbound ConnectionResponse channel
        let conn = Connection::new(self.0.keypair.clone());
        std::task::Poll::Ready(Ok(conn))
    }
}

impl Transport for NymTransport {
    type Output = Connection;
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

    fn dial(&mut self, _addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        // TODO: put connection future into poll_tx
        Err(TransportError::Other(NymTransportError::Unimplemented))
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
        if let Ok(ev) = self.poll_rx.recv() {
            return Poll::Ready(ev);
        }

        if let Poll::Ready(Some(ev)) = self.listeners.poll_next_unpin(cx) {
            match &ev {
                // TODO: this is kinda sus, put this in the listener poll instead?
                TransportEvent::Incoming {
                    listener_id: _,
                    upgrade,
                    local_addr: _,
                    send_back_addr: _,
                } => {
                    self.handle_incoming(&upgrade);
                }
                _ => {}
            }
            return Poll::Ready(ev);
        }

        Poll::Pending
    }

    fn address_translation(&self, _listen: &Multiaddr, _observed: &Multiaddr) -> Option<Multiaddr> {
        None
    }
}
