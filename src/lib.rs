pub mod protocol;
pub mod error;
pub mod connection;

pub type Result<T> = std::result::Result<T, error::Error>;
