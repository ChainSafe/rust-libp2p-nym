use libp2p::futures::{future, StreamExt};
use libp2p::ping::Success;
use libp2p::swarm::{keep_alive, NetworkBehaviour, Swarm, SwarmEvent};
use libp2p::{identity, ping, Multiaddr, PeerId};

use rust_libp2p_nym::testing::NymClient;
use std::error::Error;
use std::time::Duration;
use tracing::{debug, error, info};

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("connections=info,debug,rust_libp2p_nym=debug,libp2p_swarm=debug")
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

    let _futures = transports.into_iter().map(|(swarm, address)|{
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

        tokio::spawn(async move {
            let mut total_ping_rtt: Duration = Duration::from_micros(0);
            let mut counter: u128 = 0;

            loop {
                match swarm.select_next_some().await {
                    SwarmEvent::NewListenAddr { address, .. } => info!("Listening on {address:?}"),
                    SwarmEvent::Behaviour(event) => {
                        // Get the round-trip duration for the pings.
                        // This value is already captured in the BehaviourEvent::Ping's `Success::Ping`
                        // field.
                        debug!("{event:?}");
                        if let BehaviourEvent::Ping(ping_event) = event {
                            let result: Success = ping_event.result.expect("ping failed");
                            match result {
                                Success::Ping { rtt } => {
                                    counter += 1;
                                    total_ping_rtt += rtt;
                                    let average_ping_rtt = Duration::from_micros(
                                        (total_ping_rtt.as_micros() / counter).try_into().unwrap(),
                                    );
                                    info!("Ping RTT: {rtt:?} AVERAGE RTT: ({counter} pings): {average_ping_rtt:?}");
                                }
                                Success::Pong => info!("Pong Event"),
                            }
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
        })
    });

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
    let swarm = SwarmBuilder::with_tokio_executor(
        transport
            .map(|a, _| (a.0, StreamMuxerBox::new(a.1)))
            .boxed(),
        Behaviour::default(),
        local_peer_id,
    )
    .build();

    (swarm, address)
}

/// Our network behaviour.
///
/// For illustrative purposes, this includes the [`KeepAlive`](behaviour::KeepAlive) behaviour so a continuous sequence of
/// pings can be observed.
#[derive(NetworkBehaviour, Default)]
struct Behaviour {
    keep_alive: keep_alive::Behaviour,
    ping: ping::Behaviour,
}
