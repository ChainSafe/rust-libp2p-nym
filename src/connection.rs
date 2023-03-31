use libp2p::core::{muxing::StreamMuxerEvent, PeerId, StreamMuxer};
use nym_sphinx::addressing::clients::Recipient;
use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll, Waker},
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
pub struct Connection {
    pub(crate) peer_id: PeerId,
    pub(crate) remote_recipient: Recipient,
    pub(crate) id: ConnectionId,

    /// receive inbound messages from the `InnerConnection`
    pub(crate) inbound_rx: UnboundedReceiver<SubstreamMessage>,

    /// substream ID -> outbound pending substream exists
    /// the key is deleted when the response is received, or the request times out (TODO)
    pending_substreams: HashMap<SubstreamId, ()>,

    /// substream ID -> any pending substream data that's to be written
    /// to the stream once the substream request/response is received
    pending_substream_data: HashMap<SubstreamId, Vec<u8>>,

    /// substream ID -> substream's inbound_tx channel
    substream_inbound_txs: HashMap<SubstreamId, UnboundedSender<Vec<u8>>>,

    /// substream ID -> substream's close_tx channel
    substream_close_txs: HashMap<SubstreamId, oneshot::Sender<()>>,

    /// send messages to the mixnet
    /// used for sending `SubstreamMessageType::OpenRequest` messages
    /// also passed to each substream so they can write to the mixnet
    pub(crate) mixnet_outbound_tx: UnboundedSender<OutboundMessage>,

    /// inbound substream open requests; used in poll_inbound
    inbound_open_tx: UnboundedSender<Substream>,
    inbound_open_rx: UnboundedReceiver<Substream>,

    /// closed substream IDs; used in poll_close
    close_tx: UnboundedSender<SubstreamId>,
    close_rx: UnboundedReceiver<SubstreamId>,

    waker: Option<Waker>,
}

impl Connection {
    pub(crate) fn new(
        peer_id: PeerId,
        remote_recipient: Recipient,
        id: ConnectionId,
        inbound_rx: UnboundedReceiver<SubstreamMessage>,
        mixnet_outbound_tx: UnboundedSender<OutboundMessage>,
    ) -> Self {
        let (inbound_open_tx, inbound_open_rx) = unbounded_channel();
        let (close_tx, close_rx) = unbounded_channel();

        Connection {
            peer_id,
            remote_recipient,
            id,
            inbound_rx,
            pending_substreams: HashMap::new(),
            pending_substream_data: HashMap::new(),
            substream_inbound_txs: HashMap::new(),
            substream_close_txs: HashMap::new(),
            mixnet_outbound_tx,
            inbound_open_tx,
            inbound_open_rx,
            close_tx,
            close_rx,
            waker: None,
        }
    }

    fn new_outbound_substream(&mut self) -> Result<Substream, Error> {
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

        // track pending outbound substreams
        // TODO we should probably lock this? storing map values should be atomic
        let res = self.new_substream(substream_id.clone());
        if res.is_ok() {
            self.pending_substreams.insert(substream_id, ());
        }
        res
    }

    // creates a new substream instance with the given ID.
    fn new_substream(&mut self, id: SubstreamId) -> Result<Substream, Error> {
        // check we don't already have a substream with this ID
        if self.substream_inbound_txs.get(&id).is_some() {
            return Err(Error::SubstreamIdExists(id));
        }

        let (inbound_tx, inbound_rx) = unbounded_channel::<Vec<u8>>();
        let (close_tx, close_rx) = oneshot::channel::<()>();
        self.substream_inbound_txs.insert(id.clone(), inbound_tx);
        self.substream_close_txs.insert(id.clone(), close_tx);

        if let Some(waker) = self.waker.take() {
            waker.wake();
        }

        Ok(Substream::new(
            self.remote_recipient,
            self.id.clone(),
            id,
            inbound_rx,
            self.mixnet_outbound_tx.clone(),
            close_rx,
        ))
    }

    fn handle_close(&mut self, substream_id: SubstreamId) -> Result<(), Error> {
        if self.substream_inbound_txs.remove(&substream_id).is_none() {
            return Err(Error::SubstreamIdDoesNotExist(substream_id));
        }

        // notify substream that it's closed
        let close_tx = self.substream_close_txs.remove(&substream_id);
        close_tx.unwrap().send(()).unwrap();

        // notify poll_close that the substream is closed
        self.close_tx
            .send(substream_id)
            .map_err(|e| Error::InboundSendError(e.to_string()))
    }

    fn send_pending_inbound_data_to_substream(
        &mut self,
        substream_id: &SubstreamId,
    ) -> Result<(), Error> {
        if let Some(data) = self.pending_substream_data.remove(substream_id) {
            debug!(
                "sending pending inbound data to substream: {:?}",
                substream_id
            );
            // send data to substream
            // this error should NOT happen!!
            let Some(inbound_tx) = self
                    .substream_inbound_txs
                    .get_mut(substream_id) else {
                    return Err(Error::InboundSendError(format!("SubstreamMessageType::OpenResponse no substream channel for ID: {:?}", substream_id)));
            };
            inbound_tx.send(data).map_err(|e| {
                Error::InboundSendError(format!(
                    "failed to send pending inbound data to substream: {}",
                    e
                ))
            })?;
        }

        Ok(())
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
        _cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        Poll::Ready(self.new_outbound_substream())
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
                                    substream_id: msg.substream_id.clone(),
                                    message_type: SubstreamMessageType::OpenResponse,
                                },
                            }),
                        })
                        .map_err(|e| Error::OutboundSendError(e.to_string()))?;
                    debug!("wrote OpenResponse for substream: {:?}", &msg.substream_id);

                    // check if we have any pending inbound data for this substream and send it if so
                    self.send_pending_inbound_data_to_substream(&msg.substream_id)?;

                    // send the substream to our own channel to be returned in poll_inbound
                    self.inbound_open_tx
                        .send(substream)
                        .map_err(|e| Error::InboundSendError(e.to_string()))?;

                    debug!("new inbound substream: {:?}", &msg.substream_id);
                }
                SubstreamMessageType::OpenResponse => {
                    if self.pending_substreams.remove(&msg.substream_id).is_some() {
                        // check if we have any pending inbound data for this substream and send it if so
                        self.send_pending_inbound_data_to_substream(&msg.substream_id)?;
                        debug!("new outbound substream: {:?}", &msg.substream_id);
                    } else {
                        debug!(
                            "SubstreamMessageType::OpenResponse no substream pending for ID: {:?}",
                            &msg.substream_id
                        );
                    }
                }
                SubstreamMessageType::Close => {
                    self.handle_close(msg.substream_id)?;
                }
                SubstreamMessageType::Data(data) => {
                    debug!("SubstreamMessageType::Data: {:?}", &data);

                    // check if this is data that we don't have an inbound stream for
                    // TODO this is kinda sus and we should clear out the data from memory if we don't
                    // get an inbound stream within a certain amount of time
                    let is_pending_inbound =
                        !self.substream_inbound_txs.contains_key(&msg.substream_id);

                    // check if this an outbound stream that's still pending response
                    let is_pending_outbound =
                        self.pending_substreams.contains_key(&msg.substream_id);

                    if is_pending_inbound || is_pending_outbound {
                        debug!(
                            "SubstreamMessageType::Data is pending inbound: {:?} or pending outbound: {:?}",
                            is_pending_inbound, is_pending_outbound
                        );
                        // this substream is still pending, so we need to store the data and send it to the substream later
                        if let Some(mut existing_data) =
                            self.pending_substream_data.remove(&msg.substream_id)
                        {
                            existing_data.extend(data);
                            self.pending_substream_data
                                .insert(msg.substream_id.clone(), existing_data);
                        } else {
                            self.pending_substream_data
                                .insert(msg.substream_id.clone(), data);
                        }
                        continue;
                    }

                    let inbound_tx = self
                        .substream_inbound_txs
                        .get_mut(&msg.substream_id)
                        .expect("must have a substream channel for non-pending substream");
                    inbound_tx.send(data).map_err(|e| {
                        Error::InboundSendError(format!(
                            "failed to send inbound data to substream: {}",
                            e
                        ))
                    })?;
                }
            }
        }

        self.waker = Some(cx.waker().clone());
        Poll::Pending
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

#[cfg(test)]
mod test {
    use futures::future::poll_fn;
    use futures::{AsyncReadExt, AsyncWriteExt, FutureExt};
    use testcontainers::{clients, core::WaitFor, images::generic::GenericImage};

    use super::*;
    use crate::message::InboundMessage;
    use crate::mixnet::initialize_mixnet;
    use crate::new_nym_client;

    async fn inbound_receive_and_send(
        connection_id: ConnectionId,
        mixnet_inbound_rx: &mut UnboundedReceiver<InboundMessage>,
        inbound_tx: &UnboundedSender<SubstreamMessage>,
    ) {
        let recv_msg = mixnet_inbound_rx.recv().await.unwrap();
        match recv_msg.0 {
            Message::TransportMessage(TransportMessage { id, message: msg }) => {
                assert_eq!(id, connection_id);
                inbound_tx.send(msg).unwrap();
            }
            _ => panic!("unexpected message"),
        }
    }

    #[tokio::test]
    async fn test_connection_stream_muxer() {
        let nym_id = "test_connection_stream_muxer_sender";
        #[allow(unused)]
        let sender_uri: String;
        new_nym_client!(nym_id, sender_uri);
        let (sender_address, mut sender_mixnet_inbound_rx, sender_outbound_tx) =
            initialize_mixnet(&sender_uri, None).await.unwrap();

        let nym_id = "test_connection_stream_muxer_recipient";
        #[allow(unused)]
        let recipient_uri: String;
        new_nym_client!(nym_id, recipient_uri);
        let (recipient_address, mut recipient_mixnet_inbound_rx, recipient_outbound_tx) =
            initialize_mixnet(&recipient_uri, None).await.unwrap();

        let connection_id = ConnectionId::generate();

        let recipient_peer_id = PeerId::random();
        let sender_peer_id = PeerId::random();

        // create the connections
        let (sender_inbound_tx, sender_inbound_rx) = unbounded_channel::<SubstreamMessage>();
        let mut sender_connection = Connection::new(
            recipient_peer_id,
            recipient_address,
            connection_id.clone(),
            sender_inbound_rx,
            sender_outbound_tx,
        );
        let (recipient_inbound_tx, recipient_inbound_rx) = unbounded_channel::<SubstreamMessage>();
        let mut recipient_connection = Connection::new(
            sender_peer_id,
            sender_address,
            connection_id.clone(),
            recipient_inbound_rx,
            recipient_outbound_tx,
        );

        // send the substream OpenRequest to the mixnet
        let mut sender_substream = sender_connection.new_outbound_substream().unwrap();
        assert!(sender_connection
            .pending_substreams
            .contains_key(&sender_substream.substream_id));

        // poll the recipient inbound stream; should receive the OpenRequest and create the substream
        inbound_receive_and_send(
            connection_id.clone(),
            &mut recipient_mixnet_inbound_rx,
            &recipient_inbound_tx,
        )
        .await;
        poll_fn(|cx| Pin::new(&mut recipient_connection).as_mut().poll(cx)).now_or_never();

        // poll recipient's poll_inbound to receive the substream
        let maybe_recipient_substream = poll_fn(|cx| {
            Pin::new(&mut recipient_connection)
                .as_mut()
                .poll_inbound(cx)
        })
        .now_or_never();
        let mut recipient_substream = maybe_recipient_substream.unwrap().unwrap();

        // poll sender's connection to receive the OpenResponse and send it to the Connection inbound channel
        inbound_receive_and_send(
            connection_id.clone(),
            &mut sender_mixnet_inbound_rx,
            &sender_inbound_tx,
        )
        .await;

        // poll sender's poll_outbound to get the substream
        poll_fn(|cx| Pin::new(&mut sender_connection).as_mut().poll(cx)).now_or_never();
        assert!(sender_connection.pending_substreams.is_empty());

        // finally, write message to the substream
        let data = b"hello world";
        sender_substream.write_all(data).await.unwrap();

        // receive message from the mixnet, push to the recipient Connection inbound channel
        inbound_receive_and_send(
            connection_id.clone(),
            &mut recipient_mixnet_inbound_rx,
            &recipient_inbound_tx,
        )
        .await;

        // poll the sender's connection to send the msg from the connection inbound channel to the substream's
        poll_fn(|cx| Pin::new(&mut sender_connection).as_mut().poll(cx)).now_or_never();

        // poll the recipient's connection to read the msg from the mixnet and mux it into the substream
        poll_fn(|cx| Pin::new(&mut recipient_connection).as_mut().poll(cx)).now_or_never();

        let mut buf = [0u8; 11];
        let n = recipient_substream.read(&mut buf).await.unwrap();
        assert_eq!(n, 11);
        assert_eq!(buf, data[..]);

        // test closing the stream; assert the stream is closed on both sides
        sender_substream.close().await.unwrap();
        inbound_receive_and_send(
            connection_id.clone(),
            &mut recipient_mixnet_inbound_rx,
            &recipient_inbound_tx,
        )
        .await;
    }
}
