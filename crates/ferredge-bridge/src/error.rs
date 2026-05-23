/// Planner error raised when core semantics cannot be expressed as a bridge message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BridgePlannerError {
    /// The requested bridge capability does not support the source intent.
    #[error("unsupported command intent for requested bridge capability")]
    UnsupportedIntent,
    /// A planner or codec expected a different bridge message envelope.
    #[error("expected {expected} bridge message")]
    UnexpectedMessageKind { expected: &'static str },
}
