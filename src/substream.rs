use futures::{
    io::{Error as IoError, ErrorKind},
    AsyncRead, AsyncWrite,
};
use nym_sphinx::addressing::clients::Recipient;
use std::{
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot::Receiver,
};

use crate::message::{
    ConnectionId, Message, OutboundMessage, SubstreamId, SubstreamMessage, TransportMessage,
};

#[derive(Debug)]
pub struct Substream {
    remote_recipient: Recipient,
    connection_id: ConnectionId,
    substream_id: SubstreamId,

    /// inbound messages; inbound_tx is in the corresponding Connection
    pub(crate) inbound_rx: UnboundedReceiver<Vec<u8>>,

    /// outbound messages; go directly to the mixnet
    outbound_tx: UnboundedSender<OutboundMessage>,

    /// used to signal when the substream is closed
    close_rx: Receiver<()>,
    closed: Mutex<bool>,
}

impl Substream {
    pub(crate) fn new(
        remote_recipient: Recipient,
        connection_id: ConnectionId,
        substream_id: SubstreamId,
        inbound_rx: UnboundedReceiver<Vec<u8>>,
        outbound_tx: UnboundedSender<OutboundMessage>,
        close_rx: Receiver<()>,
    ) -> Self {
        Substream {
            remote_recipient,
            connection_id,
            substream_id,
            inbound_rx,
            outbound_tx,
            close_rx,
            closed: Mutex::new(false),
        }
    }

    fn check_closed(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Result<(), IoError> {
        let closed_err = IoError::new(ErrorKind::Other, "stream closed");

        // close_rx will return an error if the channel is closed (ie. sender was dropped),
        // or if it's empty
        let received_closed = self.close_rx.try_recv();

        let mut closed = self.closed.lock().unwrap();
        if *closed {
            return Err(closed_err);
        }

        if received_closed.is_ok() {
            *closed = true;
            return Err(closed_err);
        }

        Ok(())
    }
}

impl AsyncRead for Substream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, IoError>> {
        if let Poll::Ready(Some(data)) = self.inbound_rx.poll_recv(cx) {
            let len = data.len();
            buf[..len].copy_from_slice(&data);
            return Poll::Ready(Ok(len));
        }

        if let Err(e) = self.check_closed(cx) {
            return Poll::Ready(Err(e));
        }

        Poll::Pending
    }
}

impl AsyncWrite for Substream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, IoError>> {
        if let Err(e) = self.as_mut().check_closed(cx) {
            return Poll::Ready(Err(e));
        }

        self.outbound_tx
            .send(OutboundMessage {
                recipient: self.remote_recipient,
                message: Message::TransportMessage(TransportMessage {
                    id: self.connection_id.clone(),
                    message: SubstreamMessage::new_with_data(
                        self.substream_id.clone(),
                        buf.to_vec(),
                    ),
                }),
            })
            .map_err(|e| IoError::new(ErrorKind::Other, e.to_string()))?;

        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        if let Err(e) = self.check_closed(cx) {
            return Poll::Ready(Err(e));
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        let mut closed = self.closed.lock().unwrap();
        if *closed {
            return Poll::Ready(Err(IoError::new(ErrorKind::Other, "stream closed")));
        }

        *closed = true;

        // send a close message to the mixnet
        self.outbound_tx
            .send(OutboundMessage {
                recipient: self.remote_recipient,
                message: Message::TransportMessage(TransportMessage {
                    id: self.connection_id.clone(),
                    message: SubstreamMessage::new_close(self.substream_id.clone()),
                }),
            })
            .map_err(|e| IoError::new(ErrorKind::Other, e.to_string()))?;

        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod test {
    use futures::{AsyncReadExt, AsyncWriteExt};
    use testcontainers::{clients, core::WaitFor, images::generic::GenericImage};

    use super::Substream;
    use crate::message::{ConnectionId, Message, SubstreamId, SubstreamMessage, TransportMessage};
    use crate::mixnet::initialize_mixnet;
    use crate::new_nym_client;

    #[tokio::test]
    async fn test_substream_read_write() {
        let nym_id = "test_substream_read_write";
        #[allow(unused)]
        let uri: String;
        new_nym_client!(nym_id, uri);
        let (self_address, mut mixnet_inbound_rx, outbound_tx) =
            initialize_mixnet(&uri, None).await.unwrap();

        const MSG_INNER: &[u8] = "hello".as_bytes();
        let connection_id = ConnectionId::generate();
        let substream_id = SubstreamId::generate();

        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_, close_rx) = tokio::sync::oneshot::channel();

        let mut substream = Substream::new(
            self_address,
            connection_id,
            substream_id,
            inbound_rx,
            outbound_tx,
            close_rx,
        );

        // send message to ourselves over the mixnet
        substream.write_all(MSG_INNER).await.unwrap();

        // receive full message over the mixnet
        let recv_msg = mixnet_inbound_rx.recv().await.unwrap();
        match recv_msg.0 {
            Message::TransportMessage(TransportMessage {
                id: _,
                message:
                    SubstreamMessage {
                        substream_id: _,
                        message_type: msg,
                    },
            }) => {
                match msg {
                    crate::message::SubstreamMessageType::Data(data) => {
                        assert_eq!(data, MSG_INNER);
                        // send message to substream inbound channel
                        inbound_tx.send(data).unwrap();
                    }
                    _ => panic!("unexpected message type"),
                }
            }
            _ => panic!("unexpected message"),
        }

        // read message from substream
        let mut buf = [0u8; MSG_INNER.len()];
        substream.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, MSG_INNER);

        // close substream
        substream.close().await.unwrap();

        // try to read/write to closed substream; should error
        substream.write_all(MSG_INNER).await.unwrap_err();
        substream.read_exact(&mut buf).await.unwrap_err();

        // assert a close message was sent over the mixnet
        let recv_msg = mixnet_inbound_rx.recv().await.unwrap();
        match recv_msg.0 {
            Message::TransportMessage(TransportMessage {
                id: _,
                message:
                    SubstreamMessage {
                        substream_id: _,
                        message_type: msg,
                    },
            }) => match msg {
                crate::message::SubstreamMessageType::Close => {}
                _ => panic!("unexpected message type"),
            },
            _ => panic!("unexpected message"),
        }
    }

    #[tokio::test]
    async fn test_substream_recv_close() {
        let nym_id = "test_substream_recv_close";
        #[allow(unused)]
        let uri: String;
        new_nym_client!(nym_id, uri);
        let (self_address, _, outbound_tx) = initialize_mixnet(&uri, None).await.unwrap();

        const MSG_INNER: &[u8] = "hello".as_bytes();
        let connection_id = ConnectionId::generate();
        let substream_id = SubstreamId::generate();

        let (_, inbound_rx) = tokio::sync::mpsc::unbounded_channel();
        let (close_tx, close_rx) = tokio::sync::oneshot::channel();

        let mut substream = Substream::new(
            self_address,
            connection_id,
            substream_id,
            inbound_rx,
            outbound_tx,
            close_rx,
        );

        // close substream
        close_tx.send(()).unwrap();

        // try to read/write to closed substream; should error
        substream.write_all(MSG_INNER).await.unwrap_err();
        let mut buf = [0u8; MSG_INNER.len()];
        substream.read_exact(&mut buf).await.unwrap_err();
    }
}
