pub(crate) mod connection;
pub mod error;
pub(crate) mod message;
pub(crate) mod mixnet;
pub mod substream;
pub(crate) mod test_utils;
pub mod transport;

/// The deafult timeout secs for [`transport::Upgrade`] future.
const DEFAULT_HANDSHAKE_TIMEOUT_SECS: u64 = 5;

/// The deafult timeout secs for [`connection::Connection`] polling.
const DEFAULT_OPEN_TIMEOUT_SECS: u64 = 5;
