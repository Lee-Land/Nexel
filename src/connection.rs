use std::io::{Cursor, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use bytes::{BytesMut};
use tokio::io;
use tokio::io::{AsyncReadExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use crate::error::Error;
use crate::protocol;
use crate::protocol::{NVer, Reply, ReplyAuth, ReplyCmd, ReqCmd, Request};
use crate::Result;

pub struct Connection {
    stream: BufWriter<TcpStream>,
    buffer: BytesMut,
    id: String,
}

impl Connection {
    pub fn new(socket: TcpStream) -> Connection {
        Connection {
            stream: BufWriter::new(socket),
            buffer: BytesMut::with_capacity(128),
            id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let n_ver = self.stream.read_u8().await?;
        match n_ver {
            4 => {
                self.process_request_trip(NVer::V4).await?;
                Ok(())
            }
            5 => {
                self.process_auth_request_trip().await?;
                let _ = self.stream.read_u8().await?;
                self.process_request_trip(NVer::V5).await?;
                Ok(())
            }
            _ => return Err(Error::VnUnsupported(n_ver))
        }
    }

    async fn process_auth_request_trip(&mut self) -> Result<()> {
        if let Some(_) = self.parse_while_read(protocol::v5::parse_auth_req).await? {
            let reply = ReplyAuth { method: protocol::Method::None };
            reply.send(&mut self.stream).await?;
        }
        Ok(())
    }

    async fn process_request_trip(mut self, ver: NVer) -> Result<()> {
        let request = match ver {
            NVer::V4 => self.parse_while_read(protocol::v4::parse_req).await?,
            NVer::V5 => self.parse_while_read(protocol::v5::parse_req).await?,
        };
        if let Some(req) = request {
            match req.cmd {
                ReqCmd::Connect => {
                    println!("[CONNECT-Request] accepted a client {} request that info is {:?}", self.id, req);
                    let (reply, socket) = self.process_connect(req).await?;
                    reply.send(&mut self.stream).await?;
                    println!("[CONNECT-Reply] replied the client {} with info {:?}", self.id, reply);
                    if let Some(socket) = socket {
                        establish_pipe(self.stream.into_inner(), socket).await;
                    }
                }
                ReqCmd::Bind => {
                    println!("[BIND-Request] accepted a client {} request that info is {:?}", self.id, req);
                    let (reply, listener) = self.process_bind(req).await?;
                    reply.send(&mut self.stream).await?;
                    println!("[BIND-Reply replied the client {} with info {:?}", self.id, reply);
                    if let Some(listener) = listener {
                        let (sock, addr) = listener.accept().await?;
                        _ = sock;
                        _ = addr;
                    }
                }
                ReqCmd::Udp => {
                    if ver == NVer::V4 {
                        return Err(Error::Other(String::from("v4 doesn't support cmd udp")));
                    }
                    return Err(Error::NotImplemented);
                }
            }
        }
        Ok(())
    }

    async fn parse_while_read<F, T>(&mut self, mut f: F) -> Result<Option<T>>
        where
            F: FnMut(&mut Cursor<&[u8]>) -> Result<T> {
        loop {
            let mut cursor = Cursor::new(&self.buffer[..]);
            let parsed = match f(&mut cursor) {
                Ok(frame) => { Ok(Some(frame)) }
                Err(e) => {
                    match e {
                        Error::Incomplete => Ok(None),
                        _ => Err(e)
                    }
                }
            };
            if let Some(frame) = parsed? {
                return Ok(Some(frame));
            }

            if 0 == self.stream.read_buf(&mut self.buffer).await? {
                return if self.buffer.is_empty() {
                    Ok(None)
                } else {
                    Err(Error::IoErr(io::Error::from(ErrorKind::ConnectionReset)))
                };
            }
        }
    }

    async fn process_connect(&mut self, req: Request) -> Result<(Reply, Option<TcpStream>)> {
        let mut reply = initial_reply_by_req(&req);
        match TcpStream::connect(SocketAddr::new(req.dst_ip.unwrap(), req.dst_port)).await {
            Ok(sock) => {
                Ok((reply, Some(sock)))
            }
            Err(e) => {
                reply.cmd = get_cmd_by_error(e.kind(), req.ver);
                Ok((reply, None))
            }
        }
    }

    async fn process_bind(&mut self, req: Request) -> Result<(Reply, Option<TcpListener>)> {
        let mut reply = initial_reply_by_req(&req);
        match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0)).await {
            Ok(ls) => {
                match ls.local_addr() {
                    Ok(addr) => reply.bind_port = addr.port(),
                    Err(e) => {
                        reply.cmd = get_cmd_by_error(e.kind(), req.ver);
                    }
                }
                Ok((reply, Some(ls)))
            }
            Err(e) => {
                reply.cmd = get_cmd_by_error(e.kind(), req.ver);
                Ok((reply, None))
            }
        }
    }
}

fn initial_reply_by_req(req: &Request) -> Reply {
    Reply {
        ver: req.ver,
        cmd: match req.ver {
            NVer::V4 => ReplyCmd::V4(protocol::v4::ReplyCmd::Successful),
            NVer::V5 => ReplyCmd::V5(protocol::v5::ReplyCmd::Successful),
        },
        rsv: 0,
        bind_addr: req.dst_ip,
        domain: None,
        bind_port: req.dst_port,
    }
}

fn get_cmd_by_error(e: ErrorKind, ver: NVer) -> ReplyCmd {
    match ver {
        NVer::V4 => ReplyCmd::V4(get_cmd_by_v4(e)),
        NVer::V5 => ReplyCmd::V5(get_cmd_by_v5(e)),
    }
}

fn get_cmd_by_v4(e: ErrorKind) -> protocol::v4::ReplyCmd {
    match e {
        ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset => protocol::v4::ReplyCmd::ConnectionRefused,
        _ => protocol::v4::ReplyCmd::ConnectionFailed,
    }
}

fn get_cmd_by_v5(e: ErrorKind) -> protocol::v5::ReplyCmd {
    match e {
        ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset => protocol::v5::ReplyCmd::ConnectionRefused,
        _ => protocol::v5::ReplyCmd::ServerError,
    }
}

async fn establish_pipe(a: TcpStream, b: TcpStream) {
    let (mut a_reader, mut a_writer) = a.into_split();
    let (mut b_reader, mut b_writer) = b.into_split();
    tokio::spawn(async move {
        match io::copy(&mut a_reader, &mut b_writer).await {
            Ok(n) => {
                println!("copied bytes {n} data into b from a");
            },
            Err(e) => {
                eprintln!("failed to copy, error: {}", e);
            }
        }
    });
    match io::copy(&mut b_reader, &mut a_writer).await {
        Ok(n) => {
            println!("copied bytes {n} data into a from b");
        },
        Err(e) => {
            eprintln!("failed to copy, error: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::{Buf, BytesMut};
    use tokio::io::AsyncWriteExt;
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
            F: Fn(i32) -> i32 {
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
}
