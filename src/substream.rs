use futures::{
    io::{Error as IoError, ErrorKind},
    AsyncRead, AsyncWrite,
};
use nym_sphinx::addressing::clients::Recipient;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::message::{
    ConnectionId, Message, OutboundMessage, SubstreamId, SubstreamMessage, TransportMessage,
};

#[derive(Debug)]
pub struct Substream {
    remote_recipient: Recipient,
    connection_id: ConnectionId,
    substream_id: SubstreamId,

    pub(crate) inbound_rx: UnboundedReceiver<Vec<u8>>,
    outbound_tx: UnboundedSender<OutboundMessage>,
}

impl Substream {
    pub(crate) fn new(
        remote_recipient: Recipient,
        connection_id: ConnectionId,
        substream_id: SubstreamId,
        inbound_rx: UnboundedReceiver<Vec<u8>>,
        outbound_tx: UnboundedSender<OutboundMessage>,
    ) -> Self {
        Substream {
            remote_recipient,
            connection_id,
            substream_id,
            inbound_rx,
            outbound_tx,
        }
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

        Poll::Pending
    }
}

impl AsyncWrite for Substream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, IoError>> {
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

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }
}

#[cfg(test)]
mod test {
    use futures::{AsyncReadExt, AsyncWriteExt};

    use super::Substream;
    use crate::message::{ConnectionId, Message, SubstreamId, SubstreamMessage, TransportMessage};
    use crate::mixnet::initialize_mixnet;

    #[tokio::test]
    async fn test_substream_read_write() {
        let uri = "ws://localhost:1977".to_string();
        let (self_address, mut mixnet_inbound_rx, outbound_tx) =
            initialize_mixnet(&uri).await.unwrap();

        let msg_inner = "hello".as_bytes();
        let connection_id = ConnectionId::generate();
        let substream_id = SubstreamId::generate();

        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut substream = Substream::new(
            self_address,
            connection_id,
            substream_id,
            inbound_rx,
            outbound_tx,
        );

        // send message to ourselves over the mixnet
        substream.write(msg_inner).await.unwrap();
        //Pin::new(&mut substream).poll_write(&mut Context::from_waker(&futures::task::noop_waker_ref()), msg_inner);

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
                        assert_eq!(data, msg_inner);
                        // send message to substream inbound channel
                        inbound_tx.send(data).unwrap();
                    }
                    _ => panic!("unexpected message type"),
                }
            }
            _ => panic!("unexpected message"),
        }

        // read message from substream
        let mut buf = [0u8; 5];
        substream.read(&mut buf).await.unwrap();
        assert_eq!(buf, msg_inner);
    }
}
