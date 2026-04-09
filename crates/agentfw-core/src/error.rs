use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ModelAdapterError {
    #[error("request error: {0}")]
    Request(String),
    #[error("streaming error: {0}")]
    Streaming(String),
}

#[derive(Debug, Error, Clone)]
pub enum FrameworkError {
    #[error("model error: {0}")]
    Model(#[from] ModelAdapterError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("routing error: {0}")]
    Routing(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("driver not found: {0}")]
    DriverNotFound(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
}
