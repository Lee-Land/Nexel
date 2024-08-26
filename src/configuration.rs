pub const REMOTE_SERVER_ADDR: &str = "127.0.0.1:6789";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Plat {
    Client,
    Server,
}
