// Copyright 2018 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

//! Ping example
//!
//! See ../src/tutorial.rs for a step-by-step guide building the example below.
//!
//! In the first terminal window, run:
//!
//! ```sh
//! cargo run --example ping --features=full
//! ```
//!
//! It will print the PeerId and the listening addresses, e.g. `Listening on
//! "/ip4/0.0.0.0/tcp/24915"`
//!
//! In the second terminal window, start a new instance of the example with:
//!
//! ```sh
//! cargo run --example ping --features=full -- /ip4/127.0.0.1/tcp/24915
//! ```
//!
//! The two nodes establish a connection, negotiate the ping protocol
//! and begin pinging each other.

use futures::prelude::*;
use futures::select;
use libp2p::core::{muxing::StreamMuxerBox, transport::Transport};
use libp2p::futures::StreamExt;
use libp2p::identity::Keypair;
use libp2p::swarm::{keep_alive, NetworkBehaviour, SwarmBuilder, SwarmEvent};
use libp2p::{gossipsub, identity, ping, Multiaddr, PeerId};
use rust_libp2p_nym::{new_nym_client, transport::NymTransport};
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use testcontainers::{clients, core::WaitFor, images::generic::GenericImage};
use tokio::io;
use tokio::io::AsyncBufReadExt;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .init();

    let nym_id = rand::random::<u64>().to_string();
    #[allow(unused)]
    let dialer_uri: String = Default::default();
    new_nym_client!(nym_id, dialer_uri);

    let local_key = identity::Keypair::generate_ed25519();
    let id_keys = local_key.clone();
    let local_peer_id = PeerId::from(local_key.public());
    info!("Local peer id: {local_peer_id:?}");

    let transport = NymTransport::new(&dialer_uri, local_key).await?;

    let mut swarm = SwarmBuilder::with_tokio_executor(
        transport
            .map(|a, _| (a.0, StreamMuxerBox::new(a.1)))
            .boxed(),
        Behaviour::new(id_keys),
        local_peer_id,
    )
    .build();

    // Dial the peer identified by the multi-address given as the second
    // command-line argument, if any.
    if let Some(addr) = std::env::args().nth(1) {
        let remote: Multiaddr = addr.parse()?;
        swarm.dial(remote)?;
        info!("Dialed {addr}")
    }

    let topic = gossipsub::IdentTopic::new("test-net");

    // Read full lines from stdin
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => info!("Listening on {address:?}"),
            SwarmEvent::Behaviour(event) => {
                info!("{event:?}");
                println!("sending some gossipsub..");
                swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), b"Hello from lee!".to_vec());
            }
            other => {
                dbg!(other);
            }
        }
    }
}

/// Our network behaviour.
///
/// For illustrative purposes, this includes the [`KeepAlive`](behaviour::KeepAlive) behaviour so a continuous sequence of
/// pings can be observed.
#[derive(NetworkBehaviour)]
struct Behaviour {
    keep_alive: keep_alive::Behaviour,
    ping: ping::Behaviour,
    gossipsub: gossipsub::Behaviour,
}

impl Behaviour {
    fn new(id_keys: Keypair) -> Self {
        // To content-address message, we can take the hash of message and use it as an ID.
        let message_id_fn = |message: &gossipsub::Message| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            gossipsub::MessageId::from(s.finish().to_string())
        };
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
            .validation_mode(gossipsub::ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
            .message_id_fn(message_id_fn) // content-address messages. No two messages of the same content will be propagated.
            .build()
            .expect("Valid config");
        Self {
            keep_alive: keep_alive::Behaviour::default(),
            ping: ping::Behaviour::default(),
            gossipsub: gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(id_keys),
                gossipsub_config,
            )
            .expect("GossipSub"),
        }
    }
}
