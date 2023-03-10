# rust-libp2p-nym [wip]

This repo contains an implementation of a libp2p transport using the Nym mixnet.

## Tests

Firstly, install protoc. On Ubuntu/Debian:
```
sudo apt-get install protobuf-compiler
```

Then, clone and build Nym:
```
git clone https://github.com/nymtech/nym.git
cd nym && cargo build --release 
```

Run Nym client 1:
```
./target/release/nym-client init --id test0
./target/release/nym-client run --id test0
```

Run Nym client 2:
```
./target/release/nym-client init --id test1 --port 1978
./target/release/nym-client run --id test1
```

Finally, run the tests:
```
cargo test -- --test-threads=1
```