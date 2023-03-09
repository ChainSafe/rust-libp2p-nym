use libp2p_core::{muxing::StreamMuxerEvent, StreamMuxer};
use nym_sphinx::addressing::clients::Recipient;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};

use crate::error::Error;
use crate::message::{ConnectionId, Message, OutboundMessage, SubstreamMessage, TransportMessage};
use crate::substream::Substream;

/// Connection represents the result of a connection setup process.
#[derive(Debug)]
#[allow(dead_code)] // TODO: remove later
pub struct Connection {
    remote_recipient: Recipient,
    id: ConnectionId,

    /// receive messages from the `InnerConnection`
    pub(crate) inbound_rx: UnboundedReceiver<SubstreamMessage>,

    /// send messages to the mixnet
    pub(crate) outbound_tx: UnboundedSender<OutboundMessage>,
}

impl Connection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        id: ConnectionId,
        inbound_rx: UnboundedReceiver<SubstreamMessage>,
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
    pub(crate) async fn write(&self, msg: SubstreamMessage) -> Result<(), Error> {
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

impl StreamMuxer for Connection {
    type Substream = Substream;
    type Error = Error;

    fn poll_inbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        Poll::Ready(Err(Error::Unimplemented))
    }

    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        Poll::Ready(Err(Error::Unimplemented))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Err(Error::Unimplemented))
    }

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<StreamMuxerEvent, Self::Error>> {
        Poll::Ready(Err(Error::Unimplemented))
    }
}

/// InnerConnection is the transport's internal representation of
/// a Connection; it contains channels that interact with the mixnet.
#[allow(dead_code)] // TODO: remove later
pub(crate) struct InnerConnection {
    pub(crate) remote_recipient: Recipient,

    /// receives messages from the mixnet and sends to the `Connection`
    pub(crate) inbound_tx: UnboundedSender<SubstreamMessage>,
}

impl InnerConnection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        inbound_tx: UnboundedSender<SubstreamMessage>,
    ) -> Self {
        InnerConnection {
            remote_recipient,
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
