use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::net::{TcpListener};
use tokio::io;
use socks_proxy::connection::Connection;

#[tokio::main]
async fn main() -> io::Result<()> {
    let local_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 6789);
    let listener = TcpListener::bind(local_addr).await?;

    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let conn = Connection::new(socket);
            match conn.run_on_server().await {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("error on connection run: {}", e);
                }
            };
        });
    }
}
