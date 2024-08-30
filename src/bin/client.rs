use nexel::connection::Connection;
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::io;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> io::Result<()> {
    let local_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 3456);
    let listener = TcpListener::bind(local_addr).await?;

    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut conn = Connection::new(socket);
            match conn.run().await {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("error on connection run: {}", e);
                }
            };
        });
    }
}
