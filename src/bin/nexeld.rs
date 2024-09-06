use nexel::connection::Connection;
use nexel::{tls};
use std::net::{Ipv4Addr, SocketAddrV4};
use log::{error, LevelFilter};
use tokio::io;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> io::Result<()> {
    let local_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 6789);
    let listener = TcpListener::bind(local_addr).await?;

    // initial logger
    env_logger::Builder::new().filter(None, LevelFilter::Info).init();

    let tls_acceptor = tls::acceptor()?;
    loop {
        let (socket, _) = listener.accept().await?;
        match tokio::time::timeout(tokio::time::Duration::from_secs(10), tls_acceptor.accept(socket)).await {
            Ok(Ok(socket)) => {
                tokio::spawn(async move {
                    if let Err(e) = Connection::new(socket).run_on_server().await {
                        error!("Connection handler run failed: {}", e);
                    }
                });
            },
            Ok(Err(e)) => {
                error!("TLS handshake has an error: {}", e);
            },
            Err(e) => {
                error!("TLS handshake time out, error: {}", e);
            }
        }
    }
}
