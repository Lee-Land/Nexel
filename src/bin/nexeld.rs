use nexel::connection::Connection;
use nexel::{tls};
use std::net::{Ipv4Addr, SocketAddrV4};
use log::{error, LevelFilter};
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite};
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
        match tls_acceptor.accept(socket).await {
            Ok(socket) => {
                tokio::spawn(run(socket));
            },
            Err(e) => {
                error!("tls handler has an error: {}", e);
            }
        }
    }
}

async fn run<IO: AsyncRead + AsyncWrite + Unpin>(conn: IO) {
    let mut conn = Connection::new(conn);
    match conn.run_on_server().await {
        Ok(_) => {}
        Err(e) => {
            error!("connection handler run failed: {}", e);
        }
    }
}
