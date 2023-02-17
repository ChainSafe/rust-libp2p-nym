use anyhow::{anyhow, Error};
use futures::future::BoxFuture;
use futures::{prelude::*, stream::SelectAll};
use libp2p_core::{
    multiaddr::Multiaddr,
    transport::{ListenerId, TransportError, TransportEvent},
    PeerId, Transport,
};
use nymsphinx::addressing::clients::Recipient;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::{
    pin::Pin,
    str::FromStr,
    task::{Context, Poll},
};
use tracing::debug;

use crate::error::NymTransportError;
use crate::message::{InboundMessage, OutboundMessage};
use crate::mixnet::Mixnet;

pub struct Listener {
    id: ListenerId,

    // receives inbound mixnet messages
    inbound_rx: Receiver<InboundMessage>,
}

impl Listener {
    fn new(id: ListenerId, inbound_rx: Receiver<InboundMessage>) -> Self {
        Self { id, inbound_rx }
    }

    fn close(&self) {
        // TODO
    }
}

impl Stream for Listener {
    type Item = TransportEvent<Upgrade, NymTransportError>;

    /// poll_next should return any inbound messages on the listener.
    fn poll_next(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // TODO: poll self.inbound_rx
        // and maybe emit TransportEvent::Incoming
        Poll::Pending
    }
}

/// NymTransport implements the Transport trait using the Nym mixnet.
pub struct NymTransport {
    self_address: Recipient,
    outbound_tx: Sender<OutboundMessage>,
    listeners: SelectAll<Listener>,
    poll_rx: Receiver<TransportEvent<Upgrade, NymTransportError>>,
    poll_tx: Sender<TransportEvent<Upgrade, NymTransportError>>,
}

impl NymTransport {
    pub async fn new(uri: &String) -> Result<Self, Error> {
        // accept websocket uri and call Mixnet::new()
        // then, cache our Nym address and create the listener for it

        let (mut mixnet, inbound_rx, outbound_tx) = Mixnet::new(uri).await?;
        let self_address = mixnet.get_self_address().await?;

        let mut listeners = SelectAll::<Listener>::new();
        let listener_id = ListenerId::new();
        listeners.push(Listener::new(listener_id, inbound_rx));

        let (poll_tx, poll_rx): (
            Sender<TransportEvent<Upgrade, NymTransportError>>,
            Receiver<TransportEvent<Upgrade, NymTransportError>>,
        ) = mpsc::channel();

        let listen_addr =
            Multiaddr::from_str(&format!("/nym/{:?}", self_address)).map_err(|e| anyhow!(e))?;
        poll_tx
            .send(TransportEvent::NewAddress {
                listener_id,
                listen_addr,
            })
            .map_err(|e| anyhow!(e))?;

        Ok(Self {
            self_address,
            outbound_tx,
            listeners,
            poll_rx,
            poll_tx,
        })
    }
}

/// Connection represents the result of a connection setup process.
#[derive(Default, Debug)]
pub struct Connection {
    // TODO
    // peer_id: PeerId,
}

/// Upgrade represents a transport upgrade.
/// Currently unsupported.
pub struct Upgrade {}

impl Future for Upgrade {
    type Output = Result<Connection, NymTransportError>;

    fn poll(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Self::Output> {
        // note: we probably don't need to support upgrades; return pending?
        std::task::Poll::Ready(Ok(Connection::default()))
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
        ctx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        if let Ok(ev) = self.poll_rx.recv() {
            return Poll::Ready(ev);
        }

        if let Poll::Ready(Some(ev)) = self.listeners.poll_next_unpin(ctx) {
            return Poll::Ready(ev);
        }

        Poll::Pending
    }

    fn address_translation(&self, _listen: &Multiaddr, _observed: &Multiaddr) -> Option<Multiaddr> {
        None
    }
}
