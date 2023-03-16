use futures::{future::BoxFuture, prelude::*};
use libp2p_core::{
    multiaddr::{Multiaddr, Protocol},
    transport::{ListenerId, TransportError, TransportEvent},
    Transport,
};
use nym_sphinx::addressing::clients::Recipient;
use std::{
    collections::HashMap,
    pin::Pin,
    str::FromStr,
    task::{Context, Poll, Waker},
};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::debug;

use crate::connection::{Connection, InnerConnection, PendingConnection};
use crate::error::Error;
use crate::message::{
    ConnectionId, ConnectionMessage, InboundMessage, Message, OutboundMessage, SubstreamMessage,
    TransportMessage,
};
use crate::mixnet::initialize_mixnet;

/// InboundTransportEvent represents an inbound event from the mixnet.
pub enum InboundTransportEvent {
    ConnectionRequest(Upgrade),
    ConnectionResponse,
    TransportMessage,
}

/// NymTransport implements the Transport trait using the Nym mixnet.
pub struct NymTransport {
    /// our Nym address
    self_address: Recipient,
    pub(crate) listen_addr: Multiaddr,
    pub(crate) listener_id: ListenerId,

    /// established connections
    connections: HashMap<ConnectionId, InnerConnection>,

    /// outbound pending dials
    pending_dials: HashMap<ConnectionId, PendingConnection>,

    /// inbound mixnet messages
    inbound_stream: UnboundedReceiverStream<InboundMessage>,

    /// outbound mixnet messages
    outbound_tx: UnboundedSender<OutboundMessage>,

    /// inbound messages for Transport.poll()
    poll_rx: UnboundedReceiver<TransportEvent<Upgrade, Error>>,

    /// outbound messages to Transport.poll()
    poll_tx: UnboundedSender<TransportEvent<Upgrade, Error>>,

    waker: Option<Waker>,
}

impl NymTransport {
    pub async fn new(uri: &String) -> Result<Self, Error> {
        let (self_address, inbound_rx, outbound_tx) = initialize_mixnet(uri).await?;
        let listen_addr = nym_address_to_multiaddress(self_address)?;
        let listener_id = ListenerId::new();

        let (poll_tx, poll_rx) = unbounded_channel::<TransportEvent<Upgrade, Error>>();

        poll_tx
            .send(TransportEvent::NewAddress {
                listener_id,
                listen_addr: listen_addr.clone(),
            })
            .map_err(|_| Error::SendErrorTransportEvent)?;

        let inbound_stream = UnboundedReceiverStream::new(inbound_rx);

        Ok(Self {
            self_address,
            listen_addr,
            listener_id,
            connections: HashMap::new(),
            pending_dials: HashMap::new(),
            inbound_stream,
            outbound_tx,
            poll_rx,
            poll_tx,
            waker: None,
        })
    }

    // handle_connection_response resolves the pending connection corresponding to the response
    // (if there is one) into a Connection.
    fn handle_connection_response(&mut self, msg: ConnectionMessage) -> Result<(), Error> {
        if self.connections.contains_key(&msg.id) {
            return Err(Error::ConnectionAlreadyEstablished);
        }

        if let Some(pending_conn) = self.pending_dials.remove(&msg.id) {
            // resolve connection and put into pending_conn channel
            let (conn, inner_conn) =
                self.create_connection_types(pending_conn.remote_recipient, msg.id.clone());

            pending_conn
                .connection_tx
                .send(conn)
                .map_err(|_| Error::ConnectionSendError)?;
            self.connections.insert(msg.id, inner_conn);

            if let Some(waker) = self.waker.take() {
                waker.wake();
            }

            Ok(())
        } else {
            Err(Error::NoConnectionForResponse)
        }
    }

    /// handle_connection_request handles an incoming connection request, sends back a
    /// connection response, and finally completes the upgrade into a Connection.
    fn handle_connection_request(&mut self, msg: ConnectionMessage) -> Result<Connection, Error> {
        if msg.recipient.is_none() {
            return Err(Error::NoneRecipientInConnectionRequest);
        }

        // ensure we don't already have a conn with the same id
        if self.connections.get(&msg.id).is_some() {
            return Err(Error::ConnectionIDExists);
        }

        let (conn, inner_conn) =
            self.create_connection_types(msg.recipient.unwrap(), msg.id.clone());
        let resp = ConnectionMessage {
            recipient: None,
            id: msg.id.clone(),
        };

        self.outbound_tx
            .send(OutboundMessage {
                message: Message::ConnectionResponse(resp),
                recipient: msg.recipient.unwrap(),
            })
            .map_err(|e| Error::OutboundSendError(e.to_string()))?;

        self.connections.insert(msg.id, inner_conn);

        if let Some(waker) = self.waker.take() {
            waker.wake();
        }

        Ok(conn)
    }

    fn handle_transport_message(&self, msg: TransportMessage) -> Result<(), Error> {
        if let Some(conn) = self.connections.get(&msg.id) {
            conn.inbound_tx
                .send(msg.message)
                .map_err(|e| Error::InboundSendError(e.to_string()))?;

            if let Some(waker) = self.waker.clone().take() {
                waker.wake();
            }

            Ok(())
        } else {
            Err(Error::NoConnectionForTransportMessage)
        }
    }

    fn create_connection_types(
        &self,
        recipient: Recipient,
        id: ConnectionId,
    ) -> (Connection, InnerConnection) {
        let (inbound_tx, inbound_rx) = unbounded_channel::<SubstreamMessage>();

        // "outer" representation of a connection; this contains channels for applications to read/write to.
        let conn = Connection::new(recipient, id, inbound_rx, self.outbound_tx.clone());

        // "inner" representation of a connection; this is what we
        // read/write to when receiving messages on the mixnet,
        // or we get outbound messages from an application.
        let inner_conn = InnerConnection::new(recipient, inbound_tx);
        (conn, inner_conn)
    }

    /// handle_inbound handles an inbound message from the mixnet, received via self.inbound_stream.
    fn handle_inbound(&mut self, msg: Message) -> Result<InboundTransportEvent, Error> {
        match msg {
            Message::ConnectionRequest(inner) => {
                debug!("got connection request {:?}", inner);
                match self.handle_connection_request(inner) {
                    Ok(conn) => {
                        let (connection_tx, connection_rx) = oneshot::channel::<Connection>();
                        let upgrade = Upgrade::new(connection_rx);
                        connection_tx
                            .send(conn)
                            .map_err(|_| Error::ConnectionSendError)?;
                        Ok(InboundTransportEvent::ConnectionRequest(upgrade))
                    }
                    Err(e) => Err(e),
                }
            }
            Message::ConnectionResponse(msg) => {
                debug!("got connection response {:?}", msg);
                self.handle_connection_response(msg)
                    .map(|_| InboundTransportEvent::ConnectionResponse)
            }
            Message::TransportMessage(msg) => {
                debug!("got TransportMessage: {:?}", msg);
                self.handle_transport_message(msg)
                    .map(|_| InboundTransportEvent::TransportMessage)
            }
        }
    }
}

/// Upgrade represents a transport listener upgrade.
/// Note: we immediately upgrade a connection request to a connection,
/// so this only contains a channel for receiving that connection.
pub struct Upgrade {
    connection_tx: oneshot::Receiver<Connection>,
}

impl Upgrade {
    fn new(connection_tx: oneshot::Receiver<Connection>) -> Upgrade {
        Upgrade { connection_tx }
    }
}

impl Future for Upgrade {
    type Output = Result<Connection, Error>;

    // poll checks if the upgrade has turned into a connection yet
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(Ok(conn)) = self.connection_tx.poll_unpin(cx) {
            return Poll::Ready(Ok(conn));
        }

        Poll::Pending
    }
}

impl Transport for NymTransport {
    type Output = Connection; // TODO: this probably needs to be (PeerId, Connection)
    type Error = Error;
    type ListenerUpgrade = Upgrade;
    type Dial = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn listen_on(&mut self, _: Multiaddr) -> Result<ListenerId, TransportError<Self::Error>> {
        // we should only allow listening on the multiaddress containing our Nym address
        Err(TransportError::Other(Error::Unimplemented))
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        if self.listener_id != id {
            return false;
        }

        // TODO: close channels?
        self.poll_tx
            .send(TransportEvent::ListenerClosed {
                listener_id: id,
                reason: Ok(()),
            })
            .expect("failed to send listener closed event");
        true
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        debug!("dialing {}", addr);

        let id = ConnectionId::generate();

        // create remote recipient address
        let recipient = multiaddress_to_nym_address(addr).map_err(TransportError::Other)?;

        // create pending conn structs and store
        let (connection_tx, connection_rx) = oneshot::channel::<Connection>(); // TODO: make this bounded?

        let inner_pending_conn = PendingConnection::new(recipient, connection_tx);
        self.pending_dials.insert(id.clone(), inner_pending_conn);

        // put ConnectionRequest message into outbound message channel
        let msg = ConnectionMessage {
            recipient: Some(self.self_address),
            id,
        };

        let outbound_tx = self.outbound_tx.clone();

        let mut waker = self.waker.clone();
        Ok(async move {
            outbound_tx
                .send(OutboundMessage {
                    message: Message::ConnectionRequest(msg),
                    recipient,
                })
                .map_err(|e| Error::OutboundSendError(e.to_string()))?;

            debug!("sent outbound ConnectionRequest");
            if let Some(waker) = waker.take() {
                waker.wake();
            };

            // TODO: response timeout
            let conn = connection_rx.await.map_err(Error::OneshotRecvError)?;
            Ok(conn)
        }
        .boxed())
    }

    // dial_as_listener is unsupported.
    fn dial_as_listener(
        &mut self,
        _addr: Multiaddr,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        Err(TransportError::Other(Error::Unimplemented))
    }

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        // new addresses + listener close events
        if let Poll::Ready(Some(res)) = self.poll_rx.recv().boxed().poll_unpin(cx) {
            return Poll::Ready(res);
        }

        // check for and handle inbound messages
        while let Poll::Ready(Some(msg)) = self.inbound_stream.poll_next_unpin(cx) {
            match self.handle_inbound(msg.0) {
                Ok(event) => match event {
                    InboundTransportEvent::ConnectionRequest(upgrade) => {
                        debug!("InboundTransportEvent::ConnectionRequest");
                        return Poll::Ready(TransportEvent::Incoming {
                            listener_id: self.listener_id,
                            upgrade,
                            local_addr: self.listen_addr.clone(),
                            send_back_addr: self.listen_addr.clone(),
                        });
                    }
                    InboundTransportEvent::ConnectionResponse => {
                        debug!("InboundTransportEvent::ConnectionResponse");
                    }
                    InboundTransportEvent::TransportMessage => {
                        debug!("InboundTransportEvent::TransportMessage");
                    }
                },
                Err(e) => {
                    return Poll::Ready(TransportEvent::ListenerError {
                        listener_id: self.listener_id,
                        error: e,
                    });
                }
            };
        }

        self.waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn address_translation(&self, _listen: &Multiaddr, _observed: &Multiaddr) -> Option<Multiaddr> {
        None
    }
}

fn nym_address_to_multiaddress(addr: Recipient) -> Result<Multiaddr, Error> {
    Multiaddr::from_str(&format!("/nym/{}", addr)).map_err(Error::FailedToFormatMultiaddr)
}

fn multiaddress_to_nym_address(multiaddr: Multiaddr) -> Result<Recipient, Error> {
    let mut multiaddr = multiaddr;
    match multiaddr.pop().unwrap() {
        Protocol::Nym(addr) => Recipient::from_str(&addr).map_err(Error::InvalidRecipientBytes),
        _ => Err(Error::InvalidProtocolForMultiaddr),
    }
}

#[cfg(test)]
mod test {
    use crate::connection::Connection;
    use crate::message::{SubstreamId, SubstreamMessage, SubstreamMessageType};
    use crate::new_nym_client;
    use crate::substream::Substream;

    use super::{nym_address_to_multiaddress, NymTransport};
    use futures::{future::poll_fn, AsyncReadExt, AsyncWriteExt, FutureExt};
    use libp2p_core::{
        transport::{Transport, TransportEvent},
        StreamMuxer,
    };
    use std::pin::Pin;
    use std::time::Duration;
    use testcontainers::{clients, core::WaitFor, images::generic::GenericImage};
    use tracing::info;
    use tracing_subscriber::EnvFilter;

    #[tokio::test]
    async fn test_transport_connection() {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
            )
            .init();

        let sleep_duration = std::time::Duration::from_millis(1200);

        let nym_id = "test_transport_connection_dialer";
        #[allow(unused)]
        let dialer_uri: String;
        new_nym_client!(nym_id, dialer_uri);
        let mut dialer_transport = NymTransport::new(&dialer_uri).await.unwrap();

        let nym_id = "test_transport_connection_listener";
        #[allow(unused)]
        let listener_uri: String;
        new_nym_client!(nym_id, listener_uri);
        let mut listener_transport = NymTransport::new(&listener_uri).await.unwrap();
        let listener_multiaddr =
            nym_address_to_multiaddress(listener_transport.self_address).unwrap();
        assert_new_address_event(Pin::new(&mut dialer_transport)).await;
        assert_new_address_event(Pin::new(&mut listener_transport)).await;

        // dial the remote peer
        let mut dial = dialer_transport.dial(listener_multiaddr).unwrap();

        // poll the dial to send the connection request message
        assert!(poll_fn(|cx| Pin::new(&mut dial).as_mut().poll_unpin(cx))
            .now_or_never()
            .is_none());
        tokio::time::sleep(sleep_duration).await;

        // should receive the connection request from the mixnet and send the connection response
        let res = poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx)).await;
        let mut upgrade = match res {
            TransportEvent::Incoming {
                listener_id,
                upgrade,
                local_addr,
                send_back_addr,
            } => {
                assert_eq!(listener_id, listener_transport.listener_id);
                assert_eq!(local_addr, listener_transport.listen_addr);
                assert_eq!(send_back_addr, listener_transport.listen_addr);
                upgrade
            }
            _ => panic!("expected TransportEvent::Incoming, got {:?}", res),
        };
        tokio::time::sleep(sleep_duration).await;

        // should receive the connection response from the mixnet
        assert!(
            poll_fn(|cx| Pin::new(&mut dialer_transport).as_mut().poll(cx))
                .now_or_never()
                .is_none()
        );
        info!("waiting for connections...");

        // should be able to resolve the connections now
        let mut listener_conn = poll_fn(|cx| Pin::new(&mut upgrade).as_mut().poll_unpin(cx))
            .now_or_never()
            .unwrap()
            .unwrap();
        let mut dialer_conn = poll_fn(|cx| Pin::new(&mut dial).as_mut().poll_unpin(cx))
            .now_or_never()
            .unwrap()
            .unwrap();
        info!("connections established");

        // write messages from the dialer to the listener and vice versa
        send_and_receive_over_conns(
            b"hello".to_vec(),
            &mut dialer_conn,
            &mut listener_conn,
            Pin::new(&mut listener_transport),
        )
        .await;
        send_and_receive_over_conns(
            b"hi".to_vec(),
            &mut dialer_conn,
            &mut listener_conn,
            Pin::new(&mut listener_transport),
        )
        .await;
        send_and_receive_over_conns(
            b"world".to_vec(),
            &mut listener_conn,
            &mut dialer_conn,
            Pin::new(&mut dialer_transport),
        )
        .await;
    }

    async fn assert_new_address_event(mut transport: Pin<&mut NymTransport>) {
        match poll_fn(|cx| transport.as_mut().poll(cx)).await {
            TransportEvent::NewAddress {
                listener_id,
                listen_addr,
            } => {
                assert_eq!(listener_id, transport.listener_id);
                assert_eq!(listen_addr, transport.listen_addr);
            }
            _ => panic!("expected TransportEvent::NewAddress"),
        }
    }

    async fn send_and_receive_over_conns(
        msg: Vec<u8>,
        conn1: &mut Connection,
        conn2: &mut Connection,
        mut transport2: Pin<&mut NymTransport>,
    ) {
        // send message over conn1 to conn2
        let substream_id = SubstreamId::generate();
        conn1
            .write(SubstreamMessage::new_with_data(
                substream_id.clone(),
                msg.clone(),
            ))
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;

        // poll transport2 to push message from transport to connection
        assert!(poll_fn(|cx| transport2.as_mut().poll(cx))
            .now_or_never()
            .is_none());
        let substream_msg = conn2.inbound_rx.recv().await.unwrap();
        if let SubstreamMessageType::Data(data) = substream_msg.message_type {
            assert_eq!(data, msg);
        } else {
            panic!("expected data message");
        }
    }

    #[tokio::test]
    async fn test_transport_substream() {
        let sleep_duration = std::time::Duration::from_millis(1200);

        let nym_id = "test_transport_substream_dialer";
        #[allow(unused)]
        let dialer_uri: String;
        new_nym_client!(nym_id, dialer_uri);
        let mut dialer_transport = NymTransport::new(&dialer_uri).await.unwrap();

        let nym_id = "test_transport_substream_listener";
        #[allow(unused)]
        let listener_uri: String;
        new_nym_client!(nym_id, listener_uri);
        let mut listener_transport = NymTransport::new(&listener_uri).await.unwrap();
        let listener_multiaddr =
            nym_address_to_multiaddress(listener_transport.self_address).unwrap();
        assert_new_address_event(Pin::new(&mut dialer_transport)).await;
        assert_new_address_event(Pin::new(&mut listener_transport)).await;

        // dial the remote peer
        let mut dial = dialer_transport.dial(listener_multiaddr).unwrap();

        // poll the dial to send the connection request message
        assert!(poll_fn(|cx| Pin::new(&mut dial).as_mut().poll_unpin(cx))
            .now_or_never()
            .is_none());
        tokio::time::sleep(sleep_duration).await;

        // should receive the connection request from the mixnet and send the connection response
        let res = poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx)).await;
        let mut upgrade = match res {
            TransportEvent::Incoming {
                listener_id,
                upgrade,
                local_addr,
                send_back_addr,
            } => {
                assert_eq!(listener_id, listener_transport.listener_id);
                assert_eq!(local_addr, listener_transport.listen_addr);
                assert_eq!(send_back_addr, listener_transport.listen_addr);
                upgrade
            }
            _ => panic!("expected TransportEvent::Incoming, got {:?}", res),
        };
        tokio::time::sleep(sleep_duration * 2).await;

        // should receive the connection response from the mixnet
        assert!(
            poll_fn(|cx| Pin::new(&mut dialer_transport).as_mut().poll(cx))
                .now_or_never()
                .is_none()
        );
        info!("waiting for connections...");

        // should be able to resolve the connections now
        let mut listener_conn = poll_fn(|cx| Pin::new(&mut upgrade).as_mut().poll_unpin(cx))
            .now_or_never()
            .unwrap()
            .unwrap();
        let mut dialer_conn = poll_fn(|cx| Pin::new(&mut dial).as_mut().poll_unpin(cx))
            .now_or_never()
            .unwrap()
            .unwrap();
        info!("connections established");

        // initiate a new substream from the dialer
        let substream_id = SubstreamId::generate();
        let dialer_stream_fut = dialer_conn
            .new_stream_with_id(substream_id.clone())
            .unwrap();
        tokio::time::sleep(sleep_duration).await;

        // accept the substream on the listener
        assert!(
            poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx))
                .now_or_never()
                .is_none()
        );
        poll_fn(|cx| Pin::new(&mut listener_conn).as_mut().poll(cx)).now_or_never();

        // poll recipient's poll_inbound to receive the substream
        let mut listener_substream =
            poll_fn(|cx| Pin::new(&mut listener_conn).as_mut().poll_inbound(cx))
                .now_or_never()
                .unwrap()
                .unwrap();
        info!("got listener substream");
        tokio::time::sleep(sleep_duration).await;

        // poll sender's poll_outbound to get the substream
        assert!(
            poll_fn(|cx| Pin::new(&mut dialer_transport).as_mut().poll(cx))
                .now_or_never()
                .is_none()
        );
        poll_fn(|cx| Pin::new(&mut dialer_conn).as_mut().poll(cx)).now_or_never();
        let mut dialer_substream =
            poll_fn(|cx| Pin::new(&mut dialer_conn).as_mut().poll_outbound(cx))
                .await
                .unwrap();
        dialer_stream_fut.await.unwrap();
        info!("got dialer substream");

        // write message from dialer to listener
        send_and_receive_substream_message(
            b"hello world".to_vec(),
            Pin::new(&mut dialer_substream),
            Pin::new(&mut listener_substream),
            Pin::new(&mut listener_transport),
            Pin::new(&mut listener_conn),
        )
        .await;

        // write message from listener to dialer
        send_and_receive_substream_message(
            b"hello back".to_vec(),
            Pin::new(&mut listener_substream),
            Pin::new(&mut dialer_substream),
            Pin::new(&mut dialer_transport),
            Pin::new(&mut dialer_conn),
        )
        .await;

        // close the substream from the dialer side
        println!("closing dialer substream");
        dialer_substream.close().await.unwrap();
        tokio::time::sleep(sleep_duration).await;
        println!("dialer substream closed");

        // assert we can't read or write to either substream
        dialer_substream.write_all(b"hello").await.unwrap_err();
        poll_fn(|cx| Pin::new(&mut listener_transport).as_mut().poll(cx)).now_or_never();
        poll_fn(|cx| Pin::new(&mut listener_conn).as_mut().poll(cx)).now_or_never();
        listener_substream.write_all(b"hello").await.unwrap_err();
        let mut buf = vec![0u8; 5];
        dialer_substream.read(&mut buf).await.unwrap_err();
        listener_substream.read(&mut buf).await.unwrap_err();
        dialer_substream.close().await.unwrap_err();
        listener_substream.close().await.unwrap_err();
    }

    async fn send_and_receive_substream_message(
        data: Vec<u8>,
        mut sender_substream: Pin<&mut Substream>,
        mut recipient_substream: Pin<&mut Substream>,
        mut recipient_transport: Pin<&mut NymTransport>,
        mut recipient_conn: Pin<&mut Connection>,
    ) {
        // write message
        sender_substream.write_all(&data).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // poll recipient for message
        poll_fn(|cx| recipient_transport.as_mut().poll(cx)).now_or_never();
        poll_fn(|cx| recipient_conn.as_mut().poll(cx)).now_or_never();
        let mut buf = vec![0u8; data.len()];
        let n = recipient_substream.read(&mut buf).await.unwrap();
        assert_eq!(n, data.len());
        assert_eq!(buf, data[..]);
    }
}
