use async_channel::{self, Receiver, Sender};
use nym_sphinx::addressing::clients::Recipient;

use crate::message::{ConnectionId, Message, OutboundMessage, TransportMessage};

/// Connection represents the result of a connection setup process.
#[derive(Debug, Clone)]
#[allow(dead_code)] // TODO: remove later
pub struct Connection {
    remote_recipient: Recipient,
    id: ConnectionId,

    // TODO: implement AsyncRead/AsyncWrite with the below channels
    /// receive messages from the `InnerConnection`
    pub(crate) inbound_rx: Receiver<Vec<u8>>,

    /// send messages to the mixnet
    pub(crate) outbound_tx: Sender<OutboundMessage>,
}

impl Connection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        id: ConnectionId,
        inbound_rx: Receiver<Vec<u8>>,
        outbound_tx: Sender<OutboundMessage>,
    ) -> Self {
        Connection {
            remote_recipient,
            id,
            inbound_rx,
            outbound_tx,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn write(
        &self,
        msg: Vec<u8>,
    ) -> Result<(), async_channel::SendError<OutboundMessage>> {
        self.outbound_tx
            .send(OutboundMessage {
                recipient: self.remote_recipient,
                message: Message::TransportMessage(TransportMessage {
                    id: self.id.clone(),
                    message: msg,
                }),
            })
            .await?;
        Ok(())
    }
}

/// InnerConnection is the transport's internal representation of
/// a Connection; it contains channels that interact with the mixnet.
#[allow(dead_code)] // TODO: remove later
pub(crate) struct InnerConnection {
    pub(crate) remote_recipient: Recipient,
    pub(crate) id: ConnectionId,

    /// receives messages from the mixnet and sends to the `Connection`
    pub(crate) inbound_tx: Sender<Vec<u8>>,
}

impl InnerConnection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        id: ConnectionId,
        inbound_tx: Sender<Vec<u8>>,
    ) -> Self {
        InnerConnection {
            remote_recipient,
            id,
            inbound_tx,
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
