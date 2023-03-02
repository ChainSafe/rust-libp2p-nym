use anyhow::anyhow;
use async_channel::{self, Receiver, Sender};
use futures::prelude::*;
use nym_sphinx::addressing::clients::Recipient;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

use crate::error::NymTransportError;
use crate::message::ConnectionId;

/// Connection represents the result of a connection setup process.
#[derive(Debug)]
pub struct Connection {
    remote_recipient: Recipient,
    id: ConnectionId,
    // TODO: add channels for AsyncRead/AsyncWrite
}

impl Connection {
    pub(crate) fn new(remote_recipient: Recipient, id: ConnectionId) -> Self {
        Connection {
            remote_recipient,
            id,
        }
    }
}

/// InnerConnection is the transport's internal representation of
/// a Connection; it contains channels that interact with the mixnet.
pub(crate) struct InnerConnection {
    remote_recipient: Recipient,
    id: ConnectionId,
    // TODO: add channels for interfacing with mixnet
}

impl InnerConnection {
    pub(crate) fn new(remote_recipient: Recipient, id: ConnectionId) -> Self {
        InnerConnection {
            remote_recipient,
            id,
        }
    }
}

/// PendingConnection represents a potential connection;
/// ie. a ConnectionRequest has been sent out, but we haven't
/// gotten the response yet.
pub(crate) struct PendingConnection {
    connection_rx: Receiver<Connection>,
}

impl PendingConnection {
    pub(crate) fn new(connection_rx: Receiver<Connection>) -> Self {
        PendingConnection { connection_rx }
    }
}

impl Future for PendingConnection {
    type Output = Result<Connection, NymTransportError>;

    // poll checks if the PendingConnection has turned into a connection yet
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(res) = self.connection_rx.recv().poll_unpin(cx) {
            return Poll::Ready(res.map_err(|e| NymTransportError::Other(anyhow!(e))));
        }

        Poll::Pending
    }
}

/// InnerPendingConnection is the internal representation of a PendingConnection
/// that interfaces with the mixnet.
/// When we receive a ConnectionResponse, the PendingConnection corresponding
/// to this InnerPendingConnection receives the resolved connection.
pub(crate) struct InnerPendingConnection {
    pub(crate) remote_recipient: Recipient,
    pub(crate) connection_tx: Sender<Connection>,
}

impl InnerPendingConnection {
    pub(crate) fn new(remote_recipient: Recipient, connection_tx: Sender<Connection>) -> Self {
        InnerPendingConnection {
            remote_recipient,
            connection_tx,
        }
    }
}
