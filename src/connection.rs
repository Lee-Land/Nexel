use crate::env::Plat;
use crate::error::Error;
use crate::protocol::{Reply, ReqCmd, ReqFrame, Request};
use crate::{env, protocol, rule, tls, Result};
use bytes::BytesMut;
use std::net::{SocketAddr};
use log::{error, info};
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use crate::rule::Routing;

pub struct Connection<RW> {
    stream: BufWriter<RW>,
    id: String,
}

impl<RW: AsyncRead + AsyncWrite + Unpin> Connection<RW> {
    pub fn new(socket: RW) -> Connection<RW> {
        Connection {
            stream: BufWriter::new(socket),
            id: uuid::Uuid::new_v4().to_string(),
        }
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
                            self.process(&mut reply, &req, Plat::Client).await?;
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
                    self.process(&mut reply, &req, Plat::Server).await?;
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

    async fn process(&mut self, reply: &mut Reply, req: &Request, plat: Plat) -> Result<()> {
        reply.set_ver(req.ver);
        match self.process_request(&req, plat).await {
            Ok((mut remote, direct)) => {
                if direct {
                    self.reply(reply.successful((req.a_type, req.dst_addr, req.dst_domain.clone()), req.dst_port).await?).await?;
                    info!("[CONNECT-Reply] conn_id = {}, kind = Direct", self.id);
                    connect_two_way(self.stream.get_mut(), &mut remote).await?;
                } else {
                    let mut tls_remote = tls::connect(remote, &env::config().remote_domain()).await?;
                    let mut buffer = BytesMut::from(req.raw());
                    tls_remote.write_buf(&mut buffer).await?;
                    tls_remote.flush().await?;
                    info!("[CONNECT-Proxy] conn_id = {}, kind = Proxy", self.id);
                    connect_two_way(self.stream.get_mut(), &mut tls_remote).await?;
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

    async fn process_request(&self, req: &Request, plat: Plat) -> Result<(TcpStream, bool)> {
        match req.cmd {
            ReqCmd::Connect => {
                info!("[CONNECT-Request] conn_id = {}, Request = {}", self.id, req);
                if let Some(ip) = req.dst_addr {
                    if plat == Plat::Client && rule::ip(ip) == Routing::Proxy {
                        return Ok((TcpStream::connect(env::config().remote_uri()).await?, false));
                    }
                    Ok((TcpStream::connect(SocketAddr::new(ip, req.dst_port)).await?, true))
                } else if let Some(domain) = &req.dst_domain {
                    if plat == Plat::Client && rule::domain(domain.as_str()).await? == Routing::Proxy {
                        return Ok((TcpStream::connect(env::config().remote_uri()).await?, false));
                    }
                    let addr = format!("{}:{}", domain, req.dst_port);
                    Ok((TcpStream::connect(addr).await?, true))
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
}

async fn connect_two_way<RW1, RW2>(a: &mut RW1, b: &mut RW2) -> Result<()>
where
    RW1: AsyncRead + AsyncWrite + Unpin,
    RW2: AsyncRead + AsyncWrite + Unpin,
{
    let (mut a_reader, mut a_writer) = io::split(a);
    let (mut b_reader, mut b_writer) = io::split(b);
    tokio::select! {
        ret = io::copy( & mut a_reader, & mut b_writer) => {
            ret ?;
            b_writer.shutdown().await?;
        },
        ret = io::copy( & mut b_reader, & mut a_writer) => {
            ret ?;
            a_writer.shutdown().await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::error::Error;
    use bytes::{Buf, BytesMut};
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
}
