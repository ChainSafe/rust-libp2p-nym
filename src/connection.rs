use nym_sphinx::addressing::clients::Recipient;
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};

use crate::error::Error;
use crate::message::{ConnectionId, Message, OutboundMessage, TransportMessage};

/// Connection represents the result of a connection setup process.
#[derive(Debug)]
#[allow(dead_code)] // TODO: remove later
pub struct Connection {
    remote_recipient: Recipient,
    id: ConnectionId,

    // TODO: implement AsyncRead/AsyncWrite with the below channels
    /// receive messages from the `InnerConnection`
    pub(crate) inbound_rx: UnboundedReceiver<Vec<u8>>,

    /// send messages to the mixnet
    pub(crate) outbound_tx: UnboundedSender<OutboundMessage>,
}

impl Connection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        id: ConnectionId,
        inbound_rx: UnboundedReceiver<Vec<u8>>,
        outbound_tx: UnboundedSender<OutboundMessage>,
    ) -> Self {
        Connection {
            remote_recipient,
            id,
            inbound_rx,
            outbound_tx,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn write(&self, msg: Vec<u8>) -> Result<(), Error> {
        self.outbound_tx
            .send(OutboundMessage {
                recipient: self.remote_recipient,
                message: Message::TransportMessage(TransportMessage {
                    id: self.id.clone(),
                    message: msg,
                }),
            })
            .map_err(|e| Error::OutboundSendError(e.to_string()))?;
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
    pub(crate) inbound_tx: UnboundedSender<Vec<u8>>,
}

impl InnerConnection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        id: ConnectionId,
        inbound_tx: UnboundedSender<Vec<u8>>,
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
    pub(crate) connection_tx: oneshot::Sender<Connection>,
}

impl PendingConnection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        connection_tx: oneshot::Sender<Connection>,
    ) -> Self {
        PendingConnection {
            remote_recipient,
            connection_tx,
        }
    }
}
