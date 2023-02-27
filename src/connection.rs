use nymsphinx::addressing::clients::Recipient;

use crate::keys::{FakeKeypair, FakePublicKey};

/// Connection represents the result of a connection setup process.
#[derive(Debug)]
pub struct Connection {
    remote_recipient: Recipient,
    remote_public_key: FakePublicKey,
    keypair: FakeKeypair,
}

impl Connection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        remote_public_key: FakePublicKey,
        keypair: FakeKeypair,
    ) -> Self {
        Connection {
            remote_recipient,
            remote_public_key,
            keypair,
        }
    }
}

/// InnerConnection is the transport's internal representation of
/// a Connection; it contains channels that interact with the mixnet.
pub(crate) struct InnerConnection {
    remote_recipient: Recipient,
    remote_public_key: FakePublicKey,
    keypair: FakeKeypair,
}

impl InnerConnection {
    pub(crate) fn new(
        remote_recipient: Recipient,
        remote_public_key: FakePublicKey,
        keypair: FakeKeypair,
    ) -> Self {
        InnerConnection {
            remote_recipient,
            remote_public_key,
            keypair,
        }
    }
}

/// PendingConnection represents a potential connection;
/// ie. a ConnectionRequest has been sent out, but we haven't
/// gotten the response yet.
pub(crate) struct PendingConnection {
    remote_recipient: Recipient,
    remote_public_key: FakePublicKey,
    keypair: FakeKeypair,
}
