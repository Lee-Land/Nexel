pub const REMOTE_SERVER_ADDR: &str = "nexel.cc:6789";
pub const REMOTE_SERVER_DOMAIN: &str = "nexel.cc";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Plat {
    Client,
    Server,
}
