use futures::future::BoxFuture;
use libp2p_core::{muxing::StreamMuxerEvent, StreamMuxer};
use nym_sphinx::addressing::clients::Recipient;
use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tracing::debug;

use crate::error::Error;
use crate::message::{
    ConnectionId, Message, OutboundMessage, SubstreamId, SubstreamMessage, SubstreamMessageType,
    TransportMessage,
};
use crate::substream::Substream;

/// Connection represents the result of a connection setup process.
/// It implements `StreamMuxer` and thus has stream multiplexing built in.
#[derive(Debug)]
#[allow(dead_code)] // TODO: remove later
pub struct Connection {
    remote_recipient: Recipient,
    id: ConnectionId,

    /// receive inbound messages from the `InnerConnection`
    pub(crate) inbound_rx: UnboundedReceiver<SubstreamMessage>,

    /// substream ID -> pending substream channels
    /// a result is sent on the channel when the substream is established
    pending_substreams_tx: HashMap<SubstreamId, oneshot::Sender<Result<(), Error>>>,

    /// substream ID -> substream's inbound_tx channel
    substream_inbound_txs: HashMap<SubstreamId, UnboundedSender<Vec<u8>>>,

    /// send messages to the mixnet
    /// used for sending `SubstreamMessageType::OpenRequest` messages
    /// also passed to each substream so they can write to the mixnet
    pub(crate) mixnet_outbound_tx: UnboundedSender<OutboundMessage>,

    /// inbound substream open requests; used in poll_inbound
    inbound_open_tx: UnboundedSender<Substream>,
    inbound_open_rx: UnboundedReceiver<Substream>,

    /// outbound substream open requests; used in poll_outbound
    outbound_open_tx: UnboundedSender<Substream>,
    outbound_open_rx: UnboundedReceiver<Substream>,

    /// closed substream IDs; used in poll_close
    close_tx: UnboundedSender<SubstreamId>,
    close_rx: UnboundedReceiver<SubstreamId>,
}

impl Connection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        id: ConnectionId,
        inbound_rx: UnboundedReceiver<SubstreamMessage>,
        mixnet_outbound_tx: UnboundedSender<OutboundMessage>,
    ) -> Self {
        let (inbound_open_tx, inbound_open_rx) = unbounded_channel();
        let (outbound_open_tx, outbound_open_rx) = unbounded_channel();
        let (close_tx, close_rx) = unbounded_channel();

        Connection {
            remote_recipient,
            id,
            inbound_rx,
            pending_substreams_tx: HashMap::new(),
            substream_inbound_txs: HashMap::new(),
            mixnet_outbound_tx,
            inbound_open_tx,
            inbound_open_rx,
            outbound_open_tx,
            outbound_open_rx,
            close_tx,
            close_rx,
        }
    }

    /// this is only used in tests right now
    #[allow(dead_code)]
    pub(crate) async fn write(&self, msg: SubstreamMessage) -> Result<(), Error> {
        self.mixnet_outbound_tx
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

    /// attempts to open a new stream over the connection; returns a future that resolves when the stream is established.
    pub fn new_stream(&mut self) -> Result<BoxFuture<'static, Result<(), Error>>, Error> {
        let substream_id = SubstreamId::generate();

        // send the substream open request that requests to open a substream with the given ID
        self.mixnet_outbound_tx
            .send(OutboundMessage {
                recipient: self.remote_recipient,
                message: Message::TransportMessage(TransportMessage {
                    id: self.id.clone(),
                    message: SubstreamMessage {
                        substream_id: substream_id.clone(),
                        message_type: SubstreamMessageType::OpenRequest,
                    },
                }),
            })
            .map_err(|e| Error::OutboundSendError(e.to_string()))?;

        let (tx, rx) = oneshot::channel::<Result<(), Error>>();
        self.pending_substreams_tx.insert(substream_id, tx);

        // TODO: response timeout
        Ok(Box::pin(async move {
            rx.await.map_err(Error::OneshotRecvError)?
        }))
    }

    /// creates a new substream instance with the given ID.
    fn new_substream(&mut self, id: SubstreamId) -> Result<Substream, Error> {
        // check we don't already have a substream with this ID
        if self.substream_inbound_txs.get(&id).is_some() {
            return Err(Error::SubstreamIdExists(id));
        }

        let (inbound_tx, inbound_rx) = unbounded_channel::<Vec<u8>>();
        self.substream_inbound_txs.insert(id.clone(), inbound_tx);
        Ok(Substream::new(
            self.remote_recipient,
            self.id.clone(),
            id,
            inbound_rx,
            self.mixnet_outbound_tx.clone(),
        ))
    }

    fn handle_close(&mut self, substream_id: SubstreamId) -> Result<(), Error> {
        self.substream_inbound_txs.remove(&substream_id);
        self.close_tx
            .send(substream_id)
            .map_err(|e| Error::InboundSendError(e.to_string()))
    }
}

impl StreamMuxer for Connection {
    type Substream = Substream;
    type Error = Error;

    fn poll_inbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        if let Poll::Ready(Some(substream)) = self.inbound_open_rx.poll_recv(cx) {
            return Poll::Ready(Ok(substream));
        }

        Poll::Pending
    }

    fn poll_outbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        if let Poll::Ready(Some(substream)) = self.outbound_open_rx.poll_recv(cx) {
            return Poll::Ready(Ok(substream));
        }

        Poll::Pending
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let Poll::Ready(Some(_)) = self.close_rx.poll_recv(cx) {
            return Poll::Ready(Ok(()));
        }

        Poll::Pending
    }

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<StreamMuxerEvent, Self::Error>> {
        while let Poll::Ready(Some(msg)) = self.inbound_rx.poll_recv(cx) {
            match msg.message_type {
                SubstreamMessageType::OpenRequest => {
                    // create a new substream with the given ID
                    let substream = self.new_substream(msg.substream_id.clone())?;

                    // send the response to the remote peer
                    self.mixnet_outbound_tx
                        .send(OutboundMessage {
                            recipient: self.remote_recipient,
                            message: Message::TransportMessage(TransportMessage {
                                id: self.id.clone(),
                                message: SubstreamMessage {
                                    substream_id: msg.substream_id,
                                    message_type: SubstreamMessageType::OpenResponse,
                                },
                            }),
                        })
                        .map_err(|e| Error::OutboundSendError(e.to_string()))?;

                    // send the substream to our own channel to be returned in poll_inbound
                    self.inbound_open_tx
                        .send(substream)
                        .map_err(|e| Error::InboundSendError(e.to_string()))?;
                }
                SubstreamMessageType::OpenResponse => {
                    if let Some(pending_substream_tx) =
                        self.pending_substreams_tx.remove(&msg.substream_id)
                    {
                        let substream = self.new_substream(msg.substream_id)?;

                        // send the substream to our own channel to be returned in poll_outbound
                        self.outbound_open_tx
                            .send(substream)
                            .map_err(|e| Error::OutboundSendError(e.to_string()))?;

                        // send result to future returned in new_stream
                        pending_substream_tx
                            .send(Ok(()))
                            .map_err(|_| Error::SubstreamSendError)?;
                    } else {
                        debug!("no substream pending for ID: {:?}", &msg.substream_id);
                    }
                }
                SubstreamMessageType::Close => {
                    self.handle_close(msg.substream_id)?;
                }
                SubstreamMessageType::Data(data) => {
                    if let Some(inbound_tx) = self.substream_inbound_txs.get_mut(&msg.substream_id)
                    {
                        inbound_tx
                            .send(data)
                            .map_err(|e| Error::InboundSendError(e.to_string()))?;
                    } else {
                        debug!("no substream for ID: {:?}", &msg.substream_id);
                    }
                }
            }
        }

        Poll::Pending
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
