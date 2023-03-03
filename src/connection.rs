use async_channel::{self, Sender};
use nym_sphinx::addressing::clients::Recipient;

use crate::message::ConnectionId;

/// Connection represents the result of a connection setup process.
#[derive(Debug, Clone)]
#[allow(dead_code)] // TODO: remove later
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
#[allow(dead_code)] // TODO: remove later
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

/// PendingConnection represents a connection that's been initiated, but not completed.
pub(crate) struct PendingConnection {
    pub(crate) remote_recipient: Recipient,
    pub(crate) connection_tx: Sender<Connection>,
}

impl PendingConnection {
    pub(crate) fn new(remote_recipient: Recipient, connection_tx: Sender<Connection>) -> Self {
        PendingConnection {
            remote_recipient,
            connection_tx,
        }
    }
}
