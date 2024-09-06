use nexel::connection::Connection;
use nexel::{tls};
use std::net::{Ipv4Addr, SocketAddrV4};
use argh::FromArgs;
use log::{error, LevelFilter};
use tokio::io;
use tokio::net::TcpListener;

/// nexeld manual
#[derive(FromArgs)]
struct Option {
    /// specify the number of listening port
    #[argh(option, short = 'p', default = "6789")]
    port: u16,
    /// whether to encrypt communication with the client
    #[argh(switch, short = 't')]
    tls: bool,
    /// specify the cert file path
    #[argh(option, short = 'c', default = "String::from(\"certificate.crt\")")]
    cert: String,
    /// specify the private key file path
    #[argh(option, short ='k', default = "String::from(\"private.key\")")]
    private_key: String,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let op: Option = argh::from_env();

    let local_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), op.port);
    let listener = TcpListener::bind(local_addr).await?;

    // initial logger
    env_logger::Builder::new().filter(None, LevelFilter::Info).init();

    if op.tls {
        listen_tls(listener, &op.cert, &op.private_key).await
    } else {
        listen(listener).await
    }
}

async fn listen_tls(listener: TcpListener, cert: &String, private_key: &String) -> io::Result<()> {
    let tls_acceptor = tls::acceptor(cert, private_key)?;
    loop {
        let (socket, _) = listener.accept().await?;
        match tokio::time::timeout(tokio::time::Duration::from_secs(10), tls_acceptor.accept(socket)).await {
            Ok(Ok(socket)) => {
                tokio::spawn(async move {
                    if let Err(e) = Connection::new(socket, None).run_on_server().await {
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

async fn listen(listener: TcpListener) -> io::Result<()> {
    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
           if let Err(e) = Connection::new(socket, None).run_on_server().await {
               error!("Connection handler run failed: {}", e);
           }
        });
    }
}
