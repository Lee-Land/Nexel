use std::io::{BufRead, Cursor, ErrorKind};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use bytes::{Buf, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use crate::error::Error;
use crate::Result;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Ver {
    V4 = 4,
    V5 = 5,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ReqCmd {
    Connect = 1,
    Bind = 2,
    Udp = 3,
}

impl ReqCmd {
    pub fn from_u8(n: u8) -> Option<ReqCmd> {
        match n {
            1 => Some(ReqCmd::Connect),
            2 => Some(ReqCmd::Bind),
            3 => Some(ReqCmd::Udp),
            _ => None
        }
    }
}

pub enum ReqFrame {
    Auth(AuthReq),
    Req(Request),
}

#[derive(Debug, PartialEq)]
pub struct AuthReq {
    pub methods: Vec<u8>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Request {
    pub ver: Ver,
    pub cmd: ReqCmd,
    pub rsv: u8,
    pub dst_domain: Option<String>,
    pub dst_addr: Option<IpAddr>,
    pub dst_port: u16,
    pub a_type: AType,
}

impl Request {
    pub async fn raw(&self) -> Result<Vec<u8>> {
        let mut buf = BufWriter::with_capacity(64, vec![]);
        match self.ver {
            Ver::V4 => {
                buf.write_u8(0x04).await?;
                buf.write_u8(self.cmd as u8).await?;
                buf.write_u16(self.dst_port).await?;
                if let IpAddr::V4(ip) = self.dst_addr.unwrap() {
                    buf.write_u32(u32::from(ip)).await?
                }
                buf.write_u8(0).await?;
                buf.flush().await?;
                Ok(buf.into_inner())
            }
            Ver::V5 => {
                buf.write_u8(0x05).await?;
                buf.write_u8(self.cmd as u8).await?;
                buf.write_u8(0).await?;
                buf.write_u8(self.a_type as u8).await?;
                write_addr(&mut buf, (self.a_type, self.dst_addr, self.dst_domain.clone())).await?;
                buf.write_u16(self.dst_port).await?;
                buf.flush().await?;
                Ok(buf.into_inner())
            }
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum AType {
    Ipv4 = 1,
    Domain = 3,
    Ipv6 = 4,
}

#[derive(Copy, Clone)]
pub enum ReplyCmd {
    Successful = 0,
    ServerError = 1,
    RulesNotAllowed = 2,
    NetworkUnreachable = 3,
    HostUnreachable = 4,
    ConnectionRefused = 5,
    TtlExpired = 6,
    CmdTypeUnsupported = 7,
    AddrTypeUnsupported = 8,
}

impl ReplyCmd {
    pub fn as_u8(&self, ver: Ver) -> u8 {
        match ver {
            Ver::V4 => {
                match self {
                    ReplyCmd::Successful => 90,
                    ReplyCmd::ConnectionRefused => 91,
                    _ => 92,
                }
            }
            Ver::V5 => {
                *self as u8
            }
        }
    }
}

pub struct Reply {
    pub buffer: BufWriter<Vec<u8>>,
    pub ver: Ver,
}

impl Reply {
    pub fn new() -> Reply {
        Reply { buffer: BufWriter::with_capacity(64, vec![]), ver: Ver::V5 }
    }

    pub fn set_ver(&mut self, ver: Ver) {
        self.ver = ver;
    }
    pub async fn error(&mut self, err: &Error) -> Result<&[u8]> {
        match self.ver {
            Ver::V4 => {
                self.buffer.write_u8(0).await?;
                self.buffer.write_u8(Reply::get_cmd_by_err(err).as_u8(self.ver)).await?;
                self.buffer.write_u16(0).await?;
                self.buffer.write_u32(0).await?;
                Ok(self.buffer.buffer())
            }
            Ver::V5 => {
                self.buffer.write_u8(5).await?;
                self.buffer.write_u8(Reply::get_cmd_by_err(err).as_u8(self.ver)).await?;
                self.buffer.write_u8(0).await?;
                self.buffer.write_u8(1).await?;
                self.buffer.write_u32(0).await?;
                self.buffer.write_u16(0).await?;
                Ok(self.buffer.buffer())
            }
        }
    }

    pub async fn successful(&mut self, addr: (AType, Option<IpAddr>, Option<String>), port: u16) -> Result<&[u8]> {
        match self.ver {
            Ver::V4 => {
                self.buffer.write_u8(0).await?;
                self.buffer.write_u8(ReplyCmd::Successful.as_u8(self.ver)).await?;
                self.buffer.write_u16(port).await?;
                if let IpAddr::V4(ip) = addr.1.unwrap() {
                    self.buffer.write_u32(u32::from(ip)).await?;
                }
                Ok(self.buffer.buffer())
            }
            Ver::V5 => {
                self.buffer.write_u8(5).await?;
                self.buffer.write_u8(ReplyCmd::Successful.as_u8(self.ver)).await?;
                self.buffer.write_u8(0).await?;
                self.buffer.write_u8(addr.0 as u8).await?;
                write_addr(&mut self.buffer, addr).await?;
                self.buffer.write_u16(port).await?;
                Ok(self.buffer.buffer())
            }
        }
    }

    pub async fn auth(&mut self, n_method: u8) -> Result<&[u8]> {
        self.buffer.write_u8(0x05).await?;
        self.buffer.write_u8(n_method).await?;
        Ok(self.buffer.buffer())
    }

    fn get_cmd_by_err(err: &Error) -> ReplyCmd {
        match err {
            Error::AddrTypeUnsupported(_) => ReplyCmd::CmdTypeUnsupported,
            Error::UnknownCmd(_) => ReplyCmd::CmdTypeUnsupported,
            Error::IoErr(e) => {
                match e.kind() {
                    ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset => ReplyCmd::ConnectionRefused,
                    _ => ReplyCmd::ServerError,
                }
            }
            _ => ReplyCmd::ServerError,
        }
    }
}

async fn write_addr(writer: &mut BufWriter<Vec<u8>>, addr: (AType, Option<IpAddr>, Option<String>)) -> Result<()> {
    match addr.0 {
        AType::Ipv4 => {
            if let IpAddr::V4(ip) = addr.1.unwrap() {
                writer.write_u32(u32::from(ip)).await?;
                Ok(())
            } else {
                Err(Error::AddrTypeUnsupported(addr.0 as u8))
            }
        }
        AType::Domain => {
            let domain = addr.2.unwrap();
            let mut domain_bs = domain.as_bytes();
            writer.write_u8(domain_bs.len() as u8).await?;
            writer.write_buf(&mut domain_bs).await?;
            Ok(())
        }
        AType::Ipv6 => {
            if let IpAddr::V6(ip) = addr.1.unwrap() {
                writer.write_u128(u128::from(ip)).await?;
                Ok(())
            } else {
                Err(Error::AddrTypeUnsupported(addr.0 as u8))
            }
        }
    }
}

pub struct Parser<'a> {
    socket: &'a mut TcpStream,
}

impl Parser<'_> {
    pub fn new(socket: &mut TcpStream) -> Parser {
        Parser { socket }
    }
    pub async fn recv_and_parse_req(&mut self, authorized: bool) -> Result<Option<ReqFrame>> {
        let mut buffer = BytesMut::with_capacity(128);
        loop {
            let mut cursor = Cursor::new(&buffer[..]);
            if let Some(req) = self.pre_check_parsing(&mut cursor, authorized).await? {
                return Ok(Some(req));
            }

            if 0 == self.socket.read_buf(&mut buffer).await? {
                return if buffer.is_empty() {
                    Ok(None)
                } else {
                    Err(Error::IoErr(tokio::io::Error::from(ErrorKind::ConnectionReset)))
                };
            }
        }
    }
    async fn pre_check_parsing(&mut self, src: &mut Cursor<&[u8]>, authorized: bool) -> Result<Option<ReqFrame>> {
        match self.parse_req(src, authorized).await {
            Ok(req) => { Ok(Some(req)) }
            Err(err) => {
                match err {
                    Error::Incomplete => Ok(None),
                    _ => Err(err)
                }
            }
        }
    }
    async fn parse_req(&mut self, src: &mut Cursor<&[u8]>, authorized: bool) -> Result<ReqFrame> {
        let n_ver = get_u8(src)?;
        match n_ver {
            4 => {
                Ok(ReqFrame::Req(parse_req_v4(src)?))
            }
            5 => {
                if !authorized {
                    Ok(ReqFrame::Auth(parse_auth(src)?))
                } else {
                    Ok(ReqFrame::Req(parse_req_v5(src)?))
                }
            }
            _ => Err(Error::VnUnsupported(n_ver)),
        }
    }
}

fn parse_auth(src: &mut Cursor<&[u8]>) -> Result<AuthReq> {
    let n_methods = get_u8(src)?;
    Ok(AuthReq {
        methods: get_n_bytes(src, n_methods as usize)?,
    })
}

fn parse_req_v4(src: &mut Cursor<&[u8]>) -> Result<Request> {
    let n_cmd = get_u8(src)?;
    let cmd = ReqCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(4));
    }
    let cmd = cmd.unwrap();
    let port = get_u16(src)?;
    let ipv4 = get_u32(src)?;
    let _ = get_until(src, 0x00)?; // do not use the user_id
    Ok(Request {
        ver: Ver::V4,
        cmd,
        dst_addr: Some(IpAddr::V4(Ipv4Addr::from(ipv4))),
        dst_port: port,
        rsv: 0,
        dst_domain: None,
        a_type: AType::Ipv4,
    })
}

fn parse_req_v5(src: &mut Cursor<&[u8]>) -> Result<Request> {
    let n_cmd = get_u8(src)?;
    let cmd = ReqCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(5));
    }
    let rsv = get_u8(src)?; // rsv
    let (dst_addr, dst_domain, a_type) = get_addr(src)?;
    let dst_port = get_u16(src)?;
    Ok(Request {
        ver: Ver::V5,
        cmd: cmd.unwrap(),
        dst_addr,
        dst_domain,
        dst_port,
        rsv,
        a_type,
    })
}

fn get_addr(src: &mut Cursor<&[u8]>) -> Result<(Option<IpAddr>, Option<String>, AType)> {
    match get_u8(src)? {
        1 => Ok((Some(IpAddr::V4(Ipv4Addr::from(get_u32(src)?))), None, AType::Ipv4)),
        3 => {
            let a_len = get_u8(src)? as usize;
            Ok((None, Some(String::from_utf8(get_n_bytes(src, a_len)?)?), AType::Domain))
        }
        4 => Ok((Some(IpAddr::V6(Ipv6Addr::from(get_u128(src)?))), None, AType::Ipv6)),
        _ => Err(Error::AddrTypeUnsupported(5))
    }
}

fn get_u8(src: &mut Cursor<&[u8]>) -> Result<u8> {
    if !src.has_remaining() {
        return Err(Error::Incomplete);
    }
    Ok(src.get_u8())
}

fn get_u16(src: &mut Cursor<&[u8]>) -> Result<u16> {
    if src.remaining() < 2 {
        return Err(Error::Incomplete);
    }
    Ok(src.get_u16())
}

fn get_u32(src: &mut Cursor<&[u8]>) -> Result<u32> {
    if src.remaining() < 4 {
        return Err(Error::Incomplete);
    }
    Ok(src.get_u32())
}

fn get_u128(src: &mut Cursor<&[u8]>) -> Result<u128> {
    if src.remaining() < 128 {
        return Err(Error::Incomplete);
    }
    Ok(src.get_u128())
}

fn get_n_bytes(src: &mut Cursor<&[u8]>, n: usize) -> Result<Vec<u8>> {
    if src.remaining() < n {
        return Err(Error::Incomplete);
    }
    let mut result = Vec::new();
    let mut n = n;
    while n > 0 {
        result.push(get_u8(src)?);
        n -= 1;
    }
    Ok(result)
}

fn get_until(src: &mut Cursor<&[u8]>, c: u8) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let read_len = src.read_until(c, &mut result)?;
    if read_len == 0 {
        return Err(Error::Incomplete);
    }

    if result.pop().unwrap() == 0 {
        return Ok(result);
    }

    Err(Error::Incomplete)
}

#[cfg(test)]
mod test {
    use std::io::Cursor;
    use std::net::{IpAddr, Ipv4Addr};
    use bytes::Buf;
    use tokio::net::TcpStream;
    use crate::error::Error;
    use crate::protocol::{parse_req_v4, parse_req_v5, AType, ReqCmd, Request, Ver};

    #[test] // todo: it tests parsing protocol v4 that is completed
    fn parse_v4_that_completed() {
        let buf: Vec<u8> = vec![4, 1, 0x22, 0xc3, 0xc0, 0xa8, 1, 1, 0];
        let mut cursor = Cursor::new(&buf[..]);
        cursor.advance(1);
        let ret = parse_req_v4(&mut cursor).unwrap();
        assert_eq!(ret, Request {
            ver: Ver::V4,
            cmd: ReqCmd::Connect,
            rsv: 0,
            dst_domain: None,
            dst_addr: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
            dst_port: 8899,
            a_type: AType::Ipv4,
        });
    }
    #[test] // todo: it tests parsing protocol v4 that is incomplete
    fn parse_v4_that_incomplete() {
        let buf: Vec<u8> = vec![4, 1, 0x22, 0xc3, 0xc0, 0xa8];
        let mut cursor = Cursor::new(&buf[..]);
        cursor.advance(1);
        let ret = parse_req_v4(&mut cursor);
        match ret {
            Ok(_) => { assert!(false); }
            Err(e) => {
                match e {
                    Error::Incomplete => assert!(true),
                    _ => assert!(false)
                }
            }
        }
    }

    #[test] // todo: it tests parsing protocol v5 that completed
    fn parse_v5_that_completed() {
        let buf: Vec<u8> = vec![5, 1, 0, 1, 0xc0, 0xa8, 1, 1, 0x22, 0xc3];
        let mut cursor = Cursor::new(&buf[..]);
        cursor.advance(1);
        let ret = parse_req_v5(&mut cursor).unwrap();
        assert_eq!(ret, Request {
            ver: Ver::V5,
            cmd: ReqCmd::Connect,
            rsv: 0,
            dst_domain: None,
            dst_addr: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
            dst_port: 8899,
            a_type: AType::Ipv4,
        });
    }
    #[test] // todo: it tests parsing protocol v5 that completed
    fn parse_v5_that_completed_with_domain() {
        let buf: Vec<u8> = vec![5, 1, 0, 3, 0x08, 0x6e, 0x65, 0x78, 0x65, 0x6c, 0x2e, 0x63, 0x63, 0x22, 0xc3];
        let mut cursor = Cursor::new(&buf[..]);
        cursor.advance(1);
        let ret = parse_req_v5(&mut cursor).unwrap();
        assert_eq!(ret, Request {
            ver: Ver::V5,
            cmd: ReqCmd::Connect,
            rsv: 0,
            dst_domain: Some(String::from("nexel.cc")),
            dst_addr: None,
            dst_port: 8899,
            a_type: AType::Domain,
        });
    }
    #[test] // todo: it tests parsing protocol v5 that incomplete
    fn parse_v5_that_incomplete() {
        let buf: Vec<u8> = vec![5, 1, 0, 3, 0x08, 0x6e, 0x65, 0x78, 0x65, 0x6c, 0x2e, 0x63];
        let mut cursor = Cursor::new(&buf[..]);
        cursor.advance(1);
        let ret = parse_req_v5(&mut cursor);
        match ret {
            Ok(_) => { assert!(false); }
            Err(e) => {
                match e {
                    Error::Incomplete => assert!(true),
                    _ => assert!(false)
                }
            }
        }
    }

    #[tokio::test]
    async fn connect_to_domain() {
        match TcpStream::connect("nexel.cc:80").await {
            Ok(_) => assert!(true),
            Err(_) => assert!(false)
        }
    }
}
