use nexel::connection::Connection;
use std::net::{Ipv4Addr, SocketAddrV4};
use log::LevelFilter;
use tokio::io;
use tokio::net::TcpListener;
use nexel::rule;

#[tokio::main]
async fn main() -> io::Result<()> {
    // rule file loading
    match rule::initial() {
        Ok(_) => {}
        Err(e) => {
            log::error!("rule initial failed: {}", e.to_string());
        }
    };

    // initial logger
    env_logger::Builder::new().filter(None, LevelFilter::Info).init();

    // listen port
    let port = 3456;
    let local_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port);
    let listener = TcpListener::bind(local_addr).await?;
    log::info!("listening port: {port}");
    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut conn = Connection::new(socket);
            match conn.run().await {
                Ok(_) => {}
                Err(e) => {
                    log::error!("connection id {} handler run failed: {}", conn.id(), e);
                }
            };
        });
    }
}
