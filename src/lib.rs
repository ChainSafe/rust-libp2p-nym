pub(crate) mod connection;
pub mod error;
pub(crate) mod message;
pub(crate) mod mixnet;
pub mod substream;
pub(crate) mod test_utils;
pub mod transport;

/// The timeout secs of waiting for a response from the mixnet.
const RESPONSE_TIMEOUT_SECS: u64 = 10;
