use std::fs::File;
use std::io::{self, BufReader, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, private_key};
use tokio::net::TcpStream;
use tokio_rustls::{rustls, TlsAcceptor, TlsConnector, TlsStream};

fn load_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    certs(&mut BufReader::new(File::open(path)?)).collect()
}

fn load_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    Ok(private_key(&mut BufReader::new(File::open(path)?))
        .unwrap()
        .ok_or(io::Error::new(
            ErrorKind::Other,
            "no private key found".to_string(),
        ))?)
}

pub fn acceptor(cert: &String, private_key: &String) -> io::Result<TlsAcceptor> {
    let certs = load_certs(&PathBuf::from(cert))?;
    let key = load_key(&PathBuf::from(private_key))?;
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn connect(stream: TcpStream,cert: &str, server_domain: &str) -> io::Result<TlsStream<TcpStream>> {
    let mut root_cert_store = rustls::RootCertStore::empty();
    let mut pem = BufReader::new(File::open(PathBuf::from(cert))?);
    for cert in certs(&mut pem) {
        root_cert_store.add(cert?).unwrap();
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth(); // i guess this was previously the default?
    let connector = TlsConnector::from(Arc::new(config));

    let domain = pki_types::ServerName::try_from(server_domain)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid dnsname"))?
        .to_owned();
    Ok(TlsStream::from(connector.connect(domain, stream).await?))
}