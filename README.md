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
# BehaviourEvent: Event { peer: PeerId("12D3KooWG4QnLbms8v9xHqKJGycjtzGU8RocKXMiREpgFJ7mxSzD"), result: Ok(Ping { rtt: 880.249128ms }) }
```
```bash
# BehaviourEvent: Event { peer: PeerId("12D3KooWPymxhnqbH2dALhC2Yd2rSCkVZYYPdMxv55cn7U7x9a8c"), result: Ok(Pong) }
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
#[cfg(test)]
mod test {
    use crate::new_nym_client;
    #[tokio::test]
    async fn test_with_nym() {

        let nym_id = "test_transport_connection_dialer";
        #[allow(unused)]
        let dialer_uri: String;
        new_nym_client!(nym_id, dialer_uri);
        todo!("Now you can use the dialer_uri to make requests.")
    }
}
```

One can create as many of these as needed, limited only by the server resources.
