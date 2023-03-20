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
    use testcontainers::clients;
    use testcontainers::core::WaitFor;
    use testcontainers::images::generic::GenericImage;
    #[tokio::test]
    async fn test_with_nym() {
        let docker_client = clients::Cli::default();
        let nym_ready_message = WaitFor::message_on_stderr("Client startup finished!");
        let nym_dialer_image = GenericImage::new("nym", "latest")
            .with_env_var("NYM_ID", "test_connection_dialer")
            .with_wait_for(nym_ready_message.clone())
            .with_exposed_port(1977);
        let dialer_container = docker_client.run(nym_dialer_image);
        let dialer_port = dialer_container.get_host_port_ipv4(1977);
        let dialer_uri = format!("ws://0.0.0.0:{dialer_port}");
    }
}
```

One can create as many of these as needed, limited only by the server resources.
