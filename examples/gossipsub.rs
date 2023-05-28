use libp2p::futures::{future, StreamExt};
use libp2p::gossipsub::{
    self, subscription_filter::AllowAllSubscriptionFilter, Behaviour as BaseGossipsub,
    ConfigBuilder, IdentTopic, IdentityTransform, MessageAuthenticity, ValidationMode,
};
use libp2p::swarm::{keep_alive, NetworkBehaviour, Swarm, SwarmEvent};
use libp2p::{identity, Multiaddr, PeerId};
use rust_libp2p_nym::testing::NymClient;
use std::error::Error;
use tokio::io::{self, AsyncBufReadExt};
use tracing::{error, info};

use tracing_subscriber::EnvFilter;

type Gossipsub = BaseGossipsub<IdentityTransform, AllowAllSubscriptionFilter>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(
                "gossipsub=debug,libp2p_swarm=trace,nym_client=debug,libp2p_gossipsub=debug",
            )
        }))
        .init();

    let node_count = 2;
    let nym_clients = {
        println!("Setting up {} nym clients...", node_count);
        future::join_all((0..node_count).map(|_| NymClient::start())).await
    };

    let ports = nym_clients
        .iter()
        .map(|client| client.port)
        .collect::<Vec<_>>();

    let transports = future::join_all(ports.iter().map(|port| async {
        let transport = build_transport(*port).await;
        info!(
            "built transport for port {}, address {}",
            *port, transport.1
        );

        transport
    }))
    .await;

    let addrs = transports
        .iter()
        .map(|(_, address)| address.clone())
        .collect::<Vec<_>>();

    let _futures = transports.into_iter().map(|(swarm, address)| {
        let mut swarm = swarm;
        swarm
            .listen_on(address.clone())
            .expect("failed to listen on {address}");

        for addr in addrs.clone() {
            if addr == address {
                continue;
            }

            swarm.dial(addr).expect("failed to dial address {&addr}");
        }

        let mut stdin = io::BufReader::new(io::stdin()).lines();
        let topic = IdentTopic::new("test-net");
        swarm.behaviour_mut().gossipsub.subscribe(&topic).expect("failed to subscribe to topic");

        async move {
            loop {
                tokio::select! {
                    line = stdin.next_line() => {
                        let line = line.expect("failed to read line from stdin");
                        let message = line.expect("failed to read line from stdin");
                        swarm.behaviour_mut().gossipsub.publish(topic.clone(), message.as_bytes()).expect("failed to publish message");
                    }
                    event =  swarm.select_next_some() => match event {
                        SwarmEvent::NewListenAddr { address, .. } => info!("Listening on {address:?}"),
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            info!("Connection established");
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        }
                        SwarmEvent::Behaviour(event) => {
                            info!("{event:?}");
                            match event {
                                BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                                    propagation_source: peer_id,
                                    message_id: id,
                                    message,
                                }) => {
                                    println!(
                                        "\n\n\nGot message: '{}' with id: {id} from peer: {peer_id}",
                                        String::from_utf8_lossy(&message.data),
                                    );
                                }
                                _ => {}
                            }
                        }
                        SwarmEvent::IncomingConnection {
                            local_addr,
                            send_back_addr,
                        } => {
                            info!("Incoming connection local_addr {local_addr}, from {send_back_addr}");
                        }
                        SwarmEvent::IncomingConnectionError {
                            local_addr,
                            send_back_addr,
                            error,
                        } => {
                            error!("Failed incoming connection our_addr => {local_addr}, from => {send_back_addr}, error => {error}");
                        }
                        _ => {}
                    }
                }
            }
        }
    }).collect::<Vec<_>>();

    future::join_all(_futures).await;

    Ok(())
}

async fn build_transport(port: u16) -> (Swarm<Behaviour>, Multiaddr) {
    use libp2p::core::{muxing::StreamMuxerBox, transport::Transport};
    use libp2p::swarm::SwarmBuilder;
    use rust_libp2p_nym::transport::NymTransport;

    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());

    let transport = NymTransport::new(&format!("ws://0.0.0.0:{port}"), local_key.clone())
        .await
        .expect("build transport failed");

    let address = transport.listen_addr.clone();

    let config = ConfigBuilder::default()
        .heartbeat_interval(std::time::Duration::from_secs(10))
        .validate_messages()
        .validation_mode(ValidationMode::Anonymous)
        .allow_self_origin(true)
        .build()
        .expect("build gossipsub config failed");

    let behaviour = Behaviour {
        gossipsub: Gossipsub::new(MessageAuthenticity::Anonymous, config)
            .expect("build gossipsub failed"),
        keep_alive: Default::default(),
    };

    let swarm = SwarmBuilder::with_tokio_executor(
        transport
            .map(|a, _| (a.0, StreamMuxerBox::new(a.1)))
            .boxed(),
        behaviour,
        local_peer_id,
    )
    .build();

    (swarm, address)
}

/// Our network behaviour.
///
/// For illustrative purposes, this includes the [`KeepAlive`](behaviour::KeepAlive) behaviour so a continuous sequence of
/// pings can be observed.
#[derive(NetworkBehaviour)]
struct Behaviour {
    keep_alive: keep_alive::Behaviour,
    gossipsub: Gossipsub,
}
