use futures::future::BoxFuture;
use futures::{prelude::*, stream::SelectAll};
use libp2p_core::{
    multiaddr::Multiaddr,
    transport::{ListenerId, TransportError, TransportEvent},
    PeerId, Transport,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

use crate::error::NymTransportError;

pub struct Listener {
    id: ListenerId,
}

impl Stream for Listener {
    type Item = TransportEvent<Upgrade, NymTransportError>;

    /// poll_next should return any inbound messages on the listener.
    fn poll_next(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // TODO: connect this to Mixnet.listen()
        Poll::Pending
    }
}

/// NymTransport implements the Transport trait using the Nym mixnet.
pub struct NymTransport {
    listeners: SelectAll<Listener>,
}

impl NymTransport {
    pub fn new() -> Self {
        // TODO: accept websocket uri and call Mixnet::new()
        // cache our Nym address and create the listener for it
        Self {
            listeners: SelectAll::<Listener>::new(),
        }
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

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Self::Output> {
        // note: we probably don't need to support upgrades; return pending?
        std::task::Poll::Ready(Ok(Connection::default()))
    }
}

impl Transport for NymTransport {
    type Output = Connection;
    type Error = NymTransportError;
    type ListenerUpgrade = Upgrade;
    type Dial = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn listen_on(&mut self, _addr: Multiaddr) -> Result<ListenerId, TransportError<Self::Error>> {
        let listener_id = ListenerId::new();
        let listener = Listener { id: listener_id };
        self.listeners.push(listener);
        // TODO: we should only allow listening on the multiaddress containing our Nym address
        Ok(listener_id)
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        if let Some(_listener) = self.listeners.iter_mut().find(|l| l.id == id) {
            // TODO implement this
            // listener.close(Ok(()));
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
        self: Pin<&mut Self>,
        _ctx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Poll::Pending
    }

    fn address_translation(&self, _listen: &Multiaddr, _observed: &Multiaddr) -> Option<Multiaddr> {
        None
    }
}
