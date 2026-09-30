use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum XrsError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Proxy link parse error: {0}")]
    ParseError(String),

    #[error("Xray process error: {0}")]
    Process(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Permission/Capability error: {0}")]
    Permission(String),
}
