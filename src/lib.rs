pub mod error;
pub mod connection;
pub mod protocol;
pub mod configuration;
pub mod tls;
pub type Result<T> = std::result::Result<T, error::Error>;
