use anyhow::Error as AnyhowError;

#[derive(Debug, thiserror::Error)]
pub enum NymTransportError {
    #[error("unimplemented")]
    Unimplemented,
    #[error("failed to decode message")]
    InvalidMessageBytes,
    #[error("other")]
    Other(#[from] AnyhowError),
}
