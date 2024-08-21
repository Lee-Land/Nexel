use std::fmt::{Debug, Display, Formatter};
use std::string::FromUtf8Error;
use tokio::io;

#[derive(Debug)]
pub enum Error {
    Incomplete,
    VnUnsupported(u8),
    UnknownCmd(u8),
    NotIpV4,
    NotIpV6,
    AddrTypeUnsupported,
    NotImplemented,
    ServerRefusedAuth,
    IoErr(io::Error),
    Other(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Incomplete => write!(f, "the frame that read from socket was incomplete"),
            Error::VnUnsupported(vn) => write!(f, "the number {vn} of version was not supported"),
            Error::UnknownCmd(cmd) => write!(f, "the cmd {cmd} was not supported"),
            Error::NotIpV4 => write!(f, "the ipv4 address format was invalid"),
            Error::NotIpV6 => write!(f, "the ipv6 address format was invalid"),
            Error::AddrTypeUnsupported => write!(f, "the addr type was invalid"),
            Error::NotImplemented => write!(f, "protocol was not implemented"),
            Error::ServerRefusedAuth => write!(f, "server has refused the client auth"),
            Error::IoErr(e) => write!(f, "{}", e),
            Error::Other(desc) => write!(f, "{desc}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IoErr(e) => Some(e),
            _ => { None }
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Error::IoErr(value)
    }
}

impl From<FromUtf8Error> for Error {
    fn from(value: FromUtf8Error) -> Self {
        Error::Other(value.to_string())
    }
}
