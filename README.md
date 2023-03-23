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

**NOTE**: When using a URI for connections, ensure that you're prefixing the
URI with the `ws://` protocol type, otherwise the nym client assumes http and
crashes unceremoniously.
