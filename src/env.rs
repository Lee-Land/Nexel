#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Plat {
    Client,
    Server,
}
// pub const ENV: Env = Env::Local(EnvConfig { remote_server_addr: "127.0.0.1", remote_server_port: 6789 });
pub const ENV: Env = Env::Online(EnvConfig{ remote_server_addr: "nexel.cc", remote_server_port: 6789 });
#[derive(Debug, Eq, PartialEq)]
pub enum Env {
    Local(EnvConfig),
    Online(EnvConfig),
}
#[derive(Debug, Eq, PartialEq)]
pub struct EnvConfig {
    remote_server_addr: &'static str,
    remote_server_port: u16,
}

impl EnvConfig {
    pub fn domain(&self) -> &str {
        self.remote_server_addr
    }

    pub fn uri(&self) -> String {
        format!("{}:{}", self.remote_server_addr, self.remote_server_port)
    }
}

pub fn config() -> EnvConfig {
    match ENV {
        Env::Local(cfg) => cfg,
        Env::Online(cfg) => cfg,
    }
}
