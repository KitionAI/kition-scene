//! Typed errors: every failure names its reason (`invalid-params`, `not-found`, ...).

/// Why an operation on the scene failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SceneError {
    /// A parameter is missing, of the wrong type, or out of range.
    #[error("invalid-params: {0}")]
    InvalidParams(String),
    /// An object, material, or modifier the call names does not exist.
    #[error("not-found: {0}")]
    NotFound(String),
    /// The operation does not apply to what it was given (a mesh operation on a light).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The operation cannot run in the state things are in (tweak mode, say).
    #[error("not-allowed: {0}")]
    NotAllowed(String),
    /// The bytes are not a scene, or are damaged.
    #[error("invalid-file: {0}")]
    InvalidFile(String),
}

impl SceneError {
    /// A shorthand for [`SceneError::InvalidParams`].
    pub fn params(message: impl Into<String>) -> Self {
        Self::InvalidParams(message.into())
    }

    /// A shorthand for [`SceneError::NotFound`].
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
}

/// The model's result type.
pub type Result<T> = std::result::Result<T, SceneError>;
