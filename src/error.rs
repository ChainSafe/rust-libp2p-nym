#[derive(Debug, thiserror::Error)]
pub enum NymTransportError {
    #[error("unimplemented")]
    Unimplemented,
}
