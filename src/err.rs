pub type I2GResult<T> = Result<T, I2GError>;

#[derive(Debug, thiserror::Error)]
pub enum I2GError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),
    #[error("Ingress missing required annotation: {0}")]
    MissingAnnotation(String),
    #[error("Failed to parse annotation value: {0}")]
    ParseError(String),
    #[error("General error: {0}")]
    General(String),
    #[error(transparent)]
    SerdeJsonError(#[from] serde_json::Error),
    #[error(transparent)]
    AnyhowError(#[from] anyhow::Error),
}
