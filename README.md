# rust-libp2p-nym [wip]

This repo contains an implementation of a libp2p transport using the Nym mixnet.

## Tests

Install `protoc`. On Ubuntu/Debian, run: `sudo apt-get install protobuf-compiler`.

Ensure that docker is installed on the machine where the tests need to be run.

Then, run the following as usual.
```
DOCKER_BUILD=1 cargo test
```

Note that if you've already built the docker image and want to avoid this step,
you can ignore the `DOCKER_BUILD` environment variable.

## Ping example

To run the libp2p ping example, run the following in one terminal:
```bash
cargo run --example ping
# Local peer id: PeerId("12D3KooWLukBu6q2FerWPFhFFhiYaJkhn2sBmceh9UCaXe6hJf5D")
# Listening on "/nym/FhtkzizQg2JbZ19kGkRKXdjV2QnFbT5ww88ZAKaD4nkF.7Remi4UVYzn1yL3qYtEcQBGh6tzTYxMdYB4uqyHVc5Z4@62F81C9GrHDRja9WCqozemRFSzFPMecY85MbGwn6efve"
```

In another terminal, run ping again, passing the Nym multiaddress printed previously:
```bash
cargo run --example ping -- /nym/FhtkzizQg2JbZ19kGkRKXdjV2QnFbT5ww88ZAKaD4nkF.7Remi4UVYzn1yL3qYtEcQBGh6tzTYxMdYB4uqyHVc5Z4@62F81C9GrHDRja9WCqozemRFSzFPMecY85MbGwn6efve
# Local peer id: PeerId("12D3KooWNsuRwG6DHnFJCDR8B3zdvja6xLcfnbtKCsQWJ8eppyWC")
# Dialed /nym/FhtkzizQg2JbZ19kGkRKXdjV2QnFbT5ww88ZAKaD4nkF.7Remi4UVYzn1yL3qYtEcQBGh6tzTYxMdYB4uqyHVc5Z4@62F81C9GrHDRja9WCqozemRFSzFPMecY85MbGwn6efve
# Listening on "/nym/2oiRW5C9ivyF3Bo3Gpm4H9EqSKH7A6GpcrRRwVSDVUQ9.EajgCnhzimsP6KskUwKcEj8VFCmHR78s2J6FHWcZ4etR@Fo4f4SQLdoyoGkFae5TpVhRVoXCF8UiypLVGtGjujVPf"
```

You should see that the nodes connected and pinged each other:
```bash
# Mar 30 22:56:36.400  INFO ping: BehaviourEvent: Event { peer: PeerId("12D3KooWGf2oYd6U2nrLzfDrN9zxsjSQjPsMh2oDJPUQ9hiHMNtf"), result: Ok(Ping { rtt: 1.06836675s }) }
```
```bash
# Mar 30 22:56:35.595  INFO ping: BehaviourEvent: Event { peer: PeerId("12D3KooWMd5ak31DXuZq7x1JuFSR6toA5RDQrPaHrfXEhy7vqqpC"), result: Ok(Pong) }
```

In order to run the ping example with vanilla libp2p, which uses tcp, pass the
`--features vanilla` flag to the example and follow the instructions on the
rust-libp2p project as usual.

```bash
RUST_LOG=ping=debug cargo run --examples ping --feature vanilla
```

```bash
RUST_LOG=ping=debug cargo run --examples ping --feature vanilla -- "/ip4/127.0.0.1/tcp/$PORT"
```

### Writing New Tests

In order to abstract away the `nym-client` instantiation, we rely on the
[`testcontainers`
crate](https://docs.rs/testcontainers/latest/testcontainers/index.html) that
launches the service for us. Since there are no publicly maintained versions of
this, we use our own Dockerfile.

In order to create a single service, developers can use the following code
snippet.

```rust
let dialer_uri: String = Default::default();
rust_libp2p_nym::new_nym_client!(nym_id, dialer_uri);
```

One can create as many of these as needed, limited only by the server resources.

For more usage patterns, look at `src/transport.rs`. Note that if the code terminates
in a non-clean way, you might have to kill the running docker containers
manually using `docker rm -f $ID".
