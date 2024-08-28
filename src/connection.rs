use std::net::{IpAddr, SocketAddr};
use bytes::BytesMut;
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::net::{TcpStream};
use crate::error::Error;
use crate::{protocol, tls, Result};
use crate::configuration::{Plat, REMOTE_SERVER_ADDR, REMOTE_SERVER_DOMAIN};
use crate::protocol::{Reply, ReqCmd, ReqFrame, Request};

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
            match protocol::Parser::new(self.stream.get_mut()).recv_and_parse_req(authorized).await {
                Ok(Some(req_frame)) => {
                    match req_frame {
                        ReqFrame::Auth(req) => {
                            println!("[AUTH-Request] accepted a client {} auth request that info is {:?}", self.id, req);
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
        match protocol::Parser::new(self.stream.get_mut()).recv_and_parse_req(true).await {
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
            Ok((mut remote, connect_remote)) => {
                if connect_remote {
                    let mut tls_remote = tls::connect(remote, REMOTE_SERVER_DOMAIN).await?;
                    let raw = req.raw().await?;
                    let mut buffer = BytesMut::from(&raw[..]);
                    tls_remote.write_buf(&mut buffer).await?;
                    tls_remote.flush().await?;
                    connect_two_way(self.stream.get_mut(), &mut tls_remote).await?;
                } else {
                    self.reply(reply.successful((req.a_type, req.dst_addr, req.dst_domain.clone()), req.dst_port).await?).await?;
                    println!("[CONNECT-Reply] {} successful", self.id);
                    connect_two_way(self.stream.get_mut(), &mut remote).await?;
                }
                Ok(())
            }
            Err(e) => {
                self.reply(reply.error(&e).await?).await?;
                Ok(())
            }
        }
    }

    async fn process_request(&self, req: &Request, plat: Plat) -> Result<(TcpStream, bool)> {
        match req.cmd {
            ReqCmd::Connect => {
                println!("[CONNECT-Request] accepted a client {} request that info is {:?}", self.id, req);
                if let Some(ip) = req.dst_addr {
                    if plat == Plat::Client {
                        // todo no local or no in country
                        let is_local = match ip {
                            IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
                            IpAddr::V6(ip) => ip.is_loopback(),
                        };
                        if !is_local {
                            return Ok((TcpStream::connect(REMOTE_SERVER_ADDR).await?, true));
                        }
                    }
                    Ok((TcpStream::connect(SocketAddr::new(ip, req.dst_port)).await?, false))
                } else if let Some(domain) = &req.dst_domain {
                    if plat == Plat::Client {
                        return Ok((TcpStream::connect(REMOTE_SERVER_ADDR).await?, true));
                    }
                    let addr = format!("{}:{}", domain, req.dst_port);
                    Ok((TcpStream::connect(addr).await?, false))
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

#[allow(unused)]
async fn establish_pipe(a: TcpStream, b: TcpStream) {
    let (mut a_reader, mut a_writer) = a.into_split();
    let (mut b_reader, mut b_writer) = b.into_split();
    tokio::spawn(async move {
        let _ = io::copy(&mut a_reader, &mut b_writer).await;
        let _ = b_writer.shutdown().await;
    });
    let _ = io::copy(&mut b_reader, &mut a_writer).await;
    let _ = a_writer.shutdown().await;
}

#[cfg(test)]
mod tests {
    use bytes::{Buf, BytesMut};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::select;
    use crate::error::Error;

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
        let buf: Vec<u8> = vec![5, 1, 0, 1, 0xc0, 0xa8, 1, 1, 0x22, 0xc3];
        socket.write(buf.as_slice()).await.unwrap();
        let mut read_buf: Vec<u8> = vec![];
        socket.read_buf(&mut read_buf).await.unwrap();
        println!("{:?}", read_buf);
    }
}
