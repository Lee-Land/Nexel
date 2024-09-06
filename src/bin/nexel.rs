use nexel::connection::{Connection, ProxyCfg};
use std::net::{Ipv4Addr, SocketAddrV4};
use argh::FromArgs;
use log::LevelFilter;
use tokio::io;
use tokio::net::TcpListener;
use nexel::rule;

/// nexel manual
#[derive(FromArgs, Clone)]
struct Option {
    /// specify the number of listening port
    #[argh(option, short = 'p', default = "3456")]
    port: u16,
    /// whether to encrypt communication with the proxy server
    #[argh(switch, short = 't')]
    tls: bool,
    /// specify the cert file path
    #[argh(option, short = 'c', default = "String::from(\"certificate.crt\")")]
    cert: String,
    /// specify server host addr, can be domain or ip
    #[argh(option, short = 'h')]
    server_host: String,
    /// specify server port
    #[argh(option, short = 'o')]
    server_port: u16,
    /// specify rule.yaml file path
    #[argh(option, short = 'r', default = "String::from(\"rule.yaml\")")]
    rule_path: String,
}


#[tokio::main]
async fn main() -> io::Result<()> {
    let op: Option = argh::from_env();

    // rule file loading
    match rule::initial(&op.rule_path) {
        Ok(_) => {}
        Err(e) => {
            log::error!("rule initial failed: {}", e.to_string());
        }
    };

    // initial logger
    env_logger::Builder::new().filter(None, LevelFilter::Info).init();

    // listen port
    let port = op.port;
    let local_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port);
    let listener = TcpListener::bind(local_addr).await?;
    log::info!("listening port: {port}");
    loop {
        let op = op.clone();
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut conn = Connection::new(socket, Some(ProxyCfg::new(&op.server_host, op.server_port, if op.tls {&op.cert}else{""})));
            match conn.run().await {
                Ok(_) => {}
                Err(e) => {
                    log::error!("connection id {} handler run failed: {}", conn.id(), e);
                }
            };
        });
    }
}
