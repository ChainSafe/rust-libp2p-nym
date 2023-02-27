#[derive(Debug, thiserror::Error)]
pub enum NymTransportError {
    #[error("unimplemented")]
    Unimplemented,
    #[error("failed to decode message")]
    InvalidMessageBytes,
}
