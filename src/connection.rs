use crate::error::Error;
use crate::protocol::{Reply, ReqCmd, ReqFrame, Request};
use crate::{protocol, rule, tls, Result};
use bytes::BytesMut;
use std::net::{SocketAddr};
use std::time::Duration;
use log::{error, info};
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::net::{ToSocketAddrs, TcpStream};
use tokio::time::timeout;
use crate::rule::Routing;

pub struct Connection<RW> {
    stream: BufWriter<RW>,
    id: String,
    proxy_cfg: Option<ProxyCfg>,
}

#[derive(Clone)]
pub struct ProxyCfg {
    proxy_srv_host: String,
    proxy_srv_port: u16,
    cert_path: String,
}

impl ProxyCfg {
    pub fn new(h: &str, p: u16, cert: &str) -> ProxyCfg {
        ProxyCfg{
            proxy_srv_host: h.to_string(),
            proxy_srv_port: p,
            cert_path: cert.to_string(),
        }
    }
    pub fn host(&self) -> &str {
        &self.proxy_srv_host
    }
    pub fn addr(&self) -> String {
        format!("{}:{}", self.proxy_srv_host, self.proxy_srv_port)
    }

    pub fn cert(&self) -> &str {
        &self.cert_path
    }
}

impl<RW: AsyncRead + AsyncWrite + Unpin> Connection<RW> {
    pub fn new(socket: RW, proxy_cfg: Option<ProxyCfg>) -> Connection<RW> {
        Connection {
            stream: BufWriter::new(socket),
            id: uuid::Uuid::new_v4().to_string(),
            proxy_cfg,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut authorized = false;
        loop {
            let mut reply = Reply::new();
            match protocol::recv_and_parse_req(self.stream.get_mut(), authorized).await {
                Ok(Some(req_frame)) => {
                    match req_frame {
                        ReqFrame::Auth(_) => {
                            self.reply(reply.auth(0).await?).await?;
                            authorized = true;
                            continue;
                        }
                        ReqFrame::Req(req) => {
                            self.process(&mut reply, &req).await?;
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    self.reply(reply.error(&err).await?).await?;
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    pub async fn run_on_server(&mut self) -> Result<()> {
        let mut reply = Reply::new();
        match protocol::recv_and_parse_req(self.stream.get_mut(), true).await {
            Ok(req) => {
                if let Some(ReqFrame::Req(req)) = req {
                    self.process(&mut reply, &req).await?;
                    reply.set_ver(req.ver);
                }
                Ok(())
            }
            Err(err) => {
                self.reply(reply.error(&err).await?).await?;
                Err(err)
            }
        }
    }

    async fn process(&mut self, reply: &mut Reply, req: &Request) -> Result<()> {
        reply.set_ver(req.ver);
        match self.process_request(&req).await {
            Ok((mut remote, direct)) => {
                if direct {
                    self.reply(reply.successful((req.a_type, req.dst_addr, req.dst_domain.clone()), req.dst_port).await?).await?;
                    info!("[CONNECT-Reply] conn_id = {}, kind = Direct", self.id);
                    connect_two_way(self.stream.get_mut(), &mut remote).await?;
                } else if self.proxy_cfg.is_none() {
                    return Err(Error::Other("Proxy configuration not ".to_string()));
                } else {
                    self.proxy(req, remote).await?;
                }
                Ok(())
            }
            Err(e) => {
                error!("[CONNECT-Reply] conn_id = {}, kind = failed, error = {}", self.id, e);
                self.reply(reply.error(&e).await?).await?;
                Ok(())
            }
        }
    }

    async fn proxy(&mut self, req: &Request, mut remote: TcpStream) -> Result<()> {
        let mut buffer = BytesMut::from(req.raw());
        let proxy_cfg = self.proxy_cfg.clone().unwrap();
        if !proxy_cfg.cert().is_empty() {
            let mut tls_remote = tls::connect(remote, proxy_cfg.cert(), proxy_cfg.host()).await?;
            tls_remote.write_buf(&mut buffer).await?;
            tls_remote.flush().await?;
            info!("[CONNECT-Proxy] conn_id = {}, kind = Proxy", self.id);
            connect_two_way(self.stream.get_mut(), &mut tls_remote).await
        } else {
            remote.write_buf(&mut buffer).await?;
            remote.flush().await?;
            info!("[CONNECT-Proxy] conn_id = {}, kind = Proxy", self.id);
            connect_two_way(self.stream.get_mut(), &mut remote).await
        }
    }

    async fn process_request(&self, req: &Request) -> Result<(TcpStream, bool)> {
        match req.cmd {
            ReqCmd::Connect => {
                info!("[CONNECT-Request] conn_id = {}, Request = {}", self.id, req);
                if let Some(ip) = req.dst_addr {
                    if let Some(proxy) = &self.proxy_cfg {
                        if rule::ip(ip) == Routing::Proxy {
                            return Ok((self.timeout_connect(proxy.addr()).await?, false));
                        }
                    }
                    Ok((self.timeout_connect(SocketAddr::new(ip, req.dst_port)).await?, true))
                } else if let Some(domain) = &req.dst_domain {
                    if let Some(proxy) = &self.proxy_cfg {
                        if rule::domain(domain.as_str()).await? == Routing::Proxy {
                            return Ok((self.timeout_connect(proxy.addr()).await?, false));
                        }
                    }
                    let addr = format!("{}:{}", domain, req.dst_port);
                    Ok((self.timeout_connect(addr).await?, true))
                } else {
                    Err(Error::AddrTypeUnsupported(req.ver as u8))
                }
            }
            ReqCmd::Bind => {
                Err(Error::NotImplemented)
            }
            ReqCmd::Udp => {
                Err(Error::NotImplemented)
            }
        }
    }
    async fn reply(&mut self, buf: &[u8]) -> Result<()> {
        self.stream.write(buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn timeout_connect<A: ToSocketAddrs>(&self, addr: A) -> Result<TcpStream> {
        match timeout(Duration::from_secs(120), TcpStream::connect(addr)).await {
            Ok(Ok(ret)) => Ok(ret),
            Ok(Err(e)) => Err(Error::IoErr(e)),
            Err(_) => Err(Error::Other(format!("connection timout, id: {}", self.id))),
        }
    }
}

async fn connect_two_way<RW1, RW2>(a: &mut RW1, b: &mut RW2) -> Result<()>
where
    RW1: AsyncRead + AsyncWrite + Unpin,
    RW2: AsyncRead + AsyncWrite + Unpin,
{
    let (mut a_reader, mut a_writer) = io::split(a);
    let (mut b_reader, mut b_writer) = io::split(b);

    let copy_a_to_b = async {
        let _ = io::copy(&mut a_reader, &mut b_writer).await;
        let _ = b_writer.shutdown().await;
    };
    let copy_b_to_a = async {
        let _ = io::copy(&mut b_reader, &mut a_writer).await;
        let _ = a_writer.shutdown().await;
    };
    tokio::join!(copy_a_to_b, copy_b_to_a);
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::error::Error;
    use bytes::{Buf, BytesMut};
    use serde::de::Unexpected::Bytes;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::select;

    #[test]
    fn test_advance_buffer() {
        let mut buffer = BytesMut::from("abcde");
        buffer.advance(1);
        assert_eq!(buffer[0], b'b');
    }

    fn call_printer<F>(f: F, n: i32) -> i32
    where
        F: Fn(i32) -> i32,
    {
        f(n)
    }

    fn show_i32(n: i32) -> i32 {
        n
    }

    #[test]
    fn test_closure() {
        let res = call_printer(show_i32, 10);
        assert_eq!(res, 10);
    }

    async fn async_process() -> Result<(), Error> {
        // Err(Error::Other("error".to_string()))
        Ok(())
    }

    #[tokio::test]
    async fn test_select() {
        loop {
            println!("select!");
            let _ = tokio::io::stdout().flush().await;
            select! {
                Ok(_) = async_process() => {
                    println!("process1");
                },
                Ok(_) = async_process() => {
                    println!("process2");
                },
                else => {
                    println!("else");
                        break
                }
            }
        }

        println!("all processes has been worked");
    }

    #[tokio::test]
    async fn test_copy() {
        use tokio::io;

        let mut reader: &[u8] = b"hello";
        let mut writer: Vec<u8> = vec![];

        let l = io::copy(&mut reader, &mut writer).await;
        let l2 = io::copy(&mut reader, &mut writer).await;

        assert_eq!(&b"hello"[..], &writer[..]);
        println!("{:?}", l);
        println!("{:?}", l2);
    }

    #[tokio::test]
    async fn test_client_v5() {
        let mut socket = TcpStream::connect("127.0.0.1:3456").await.unwrap();
        let buf: Vec<u8> = vec![0x05, 0x01, 0x00];
        socket.write(buf.as_slice()).await.unwrap();
        let mut read_buf: Vec<u8> = vec![];
        socket.read_buf(&mut read_buf).await.unwrap();
        println!("{:?}", read_buf);
        let buf: Vec<u8> = vec![5, 1, 0, 3, 8, 0x6e, 0x65, 0x78, 0x65, 0x6c, 0x2e, 0x63, 0x63, 0x00, 0x50];
        socket.write(buf.as_slice()).await.unwrap();
        let mut read_buf: Vec<u8> = vec![];
        socket.read_buf(&mut read_buf).await.unwrap();
        println!("{:?}", read_buf);
    }

    #[tokio::test]
    async fn test_client_proxy_http() {
        let mut socket = TcpStream::connect("127.0.0.1:3456").await.unwrap();
        let mut buf = BytesMut::from("CONNECT nexel.cc HTTP/1.1\r\n\r\n");
        socket.write_buf(&mut buf).await.unwrap();
        let mut read_buf: Vec<u8> = vec![];
        socket.read_buf(&mut read_buf).await.unwrap();
        println!("{:?}", String::from_utf8_lossy(&read_buf[..]));
    }

    #[tokio::test]
    async fn test_connect_to_remote() {
        let mut socket = TcpStream::connect("nexel.cc:6789").await.unwrap();
        let mut buf = BytesMut::from("test");
        socket.write_buf(&mut buf).await.unwrap();
        socket.flush().await.unwrap();
    }
}
