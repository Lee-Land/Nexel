use std::fmt::{Display, Formatter};
use crate::error::Error;
use crate::Result;
use bytes::{Buf, BytesMut};
use std::io::{BufRead, Cursor, ErrorKind};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter};
use url::Url;
use tokio::time::{timeout, Duration};

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Ver {
    V4 = 4,
    V5 = 5,
    Http,
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

#[derive(Debug, PartialEq)]
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
    raw: Vec<u8>,
}

impl Display for Request {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "version: {:?}, cmd: {:?}, dst_domain: {:?}, dst_addr: {:?}, dst_port: {}",
               self.ver, self.cmd, self.dst_domain, self.dst_addr, self.dst_port)
    }
}

impl Request {
    pub fn raw(&self) -> &[u8] {
        &self.raw[..]
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
            _ => 0
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
            }
            Ver::V5 => {
                self.buffer.write_u8(5).await?;
                self.buffer.write_u8(Reply::get_cmd_by_err(err).as_u8(self.ver)).await?;
                self.buffer.write_u8(0).await?;
                self.buffer.write_u8(1).await?;
                self.buffer.write_u32(0).await?;
                self.buffer.write_u16(0).await?;
            }
            Ver::Http => {
                let response = "HTTP/1.1 400 Connection Failed\r\n\r\n";
                let mut buf = BytesMut::from(response);
                self.buffer.write_buf(&mut buf).await?;
            }
        };
        Ok(self.buffer.buffer())
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
            }
            Ver::V5 => {
                self.buffer.write_u8(5).await?;
                self.buffer.write_u8(ReplyCmd::Successful.as_u8(self.ver)).await?;
                self.buffer.write_u8(0).await?;
                self.buffer.write_u8(addr.0 as u8).await?;
                write_addr(&mut self.buffer, addr).await?;
                self.buffer.write_u16(port).await?;
            }
            Ver::Http => {
                let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
                let mut buf = BytesMut::from(response);
                self.buffer.write_buf(&mut buf).await?;
            }
        };
        Ok(self.buffer.buffer())
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

pub async fn recv_and_parse_req<RW>(io: &mut RW, authorized: bool)
                                    -> Result<Option<ReqFrame>>
where
    RW: AsyncRead + AsyncWrite + Unpin,
{
    let mut buffer = BytesMut::with_capacity(128);
    loop {
        let mut cursor = Cursor::new(&buffer[..]);
        if let Some(req) = pre_check_parsing(&mut cursor, authorized).await? {
            return Ok(Some(req));
        }

        match timeout(Duration::from_secs(120), io.read_buf(&mut buffer)).await {
            Ok(n) => {
                if 0 == n? {
                    return if buffer.is_empty() {
                        Ok(None)
                    } else {
                        Err(Error::IoErr(tokio::io::Error::from(ErrorKind::ConnectionReset)))
                    };
                }
            }
            Err(_) => {
                return Err(Error::Other("read from connection timeout".to_string()));
            }
        }

    }
}
async fn pre_check_parsing(src: &mut Cursor<&[u8]>, authorized: bool) -> Result<Option<ReqFrame>> {
    match parse_req(src, authorized).await {
        Ok(req) => { Ok(Some(req)) }
        Err(err) => {
            match err {
                Error::Incomplete => Ok(None),
                _ => Err(err)
            }
        }
    }
}
async fn parse_req(src: &mut Cursor<&[u8]>, authorized: bool) -> Result<ReqFrame> {
    let mut buf_reader = BufReader::with_capacity(64);
    let n_ver = buf_reader.get_u8(src).await?;
    match n_ver {
        // socks v4
        4 => {
            Ok(ReqFrame::Req(parse_req_v4(src, buf_reader).await?))
        }
        // socks v5
        5 => {
            if !authorized {
                Ok(ReqFrame::Auth(parse_auth(src, &mut buf_reader).await?))
            } else {
                Ok(ReqFrame::Req(parse_req_v5(src, buf_reader).await?))
            }
        }
        // HTTP CONNECT
        b'C' => {
            Ok(ReqFrame::Req(parse_req_http_connect(src, buf_reader).await?))
        }
        _ => Err(Error::VnUnsupported(n_ver)),
    }
}

async fn parse_auth(src: &mut Cursor<&[u8]>, buf_reader: &mut BufReader) -> Result<AuthReq> {
    let n_methods = buf_reader.get_u8(src).await?;
    Ok(AuthReq {
        methods: buf_reader.get_n_bytes(src, n_methods as usize).await?,
    })
}

async fn parse_req_v4(src: &mut Cursor<&[u8]>, mut buf_reader: BufReader) -> Result<Request> {
    let n_cmd = buf_reader.get_u8(src).await?;
    let cmd = ReqCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(4));
    }
    let cmd = cmd.unwrap();
    let port = buf_reader.get_u16(src).await?;
    let ipv4 = buf_reader.get_u32(src).await?;
    let mut user_id_vec = buf_reader.get_until(src, 0x00).await?; // ignore the user_id
    if user_id_vec.pop().unwrap() != 0 {
        return Err(Error::Incomplete);
    }
    Ok(Request {
        ver: Ver::V4,
        cmd,
        dst_addr: Some(IpAddr::V4(Ipv4Addr::from(ipv4))),
        dst_port: port,
        rsv: 0,
        dst_domain: None,
        a_type: AType::Ipv4,
        raw: buf_reader.into_inner().await?,
    })
}

async fn parse_req_v5(src: &mut Cursor<&[u8]>, mut buf_reader: BufReader) -> Result<Request> {
    let n_cmd = buf_reader.get_u8(src).await?;
    let cmd = ReqCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(5));
    }
    let cmd = cmd.unwrap();
    let rsv = buf_reader.get_u8(src).await?; // rsv
    let (dst_addr, dst_domain, a_type) = get_addr(src, &mut buf_reader).await?;
    let dst_port = buf_reader.get_u16(src).await?;

    Ok(Request {
        ver: Ver::V5,
        cmd,
        dst_addr,
        dst_domain,
        dst_port,
        rsv,
        a_type,
        raw: buf_reader.into_inner().await?,
    })
}

async fn parse_req_http_connect(src: &mut Cursor<&[u8]>, mut buf_reader: BufReader) -> Result<Request> {
    let line = buf_reader.get_line(src).await?;
    while buf_reader.get_line(src).await?.len() > 0 {}
    let parts: Vec<&str> = line.split(' ').collect();
    if parts.len() != 3 {
        return Err(Error::Other("Bad Request".to_string()));
    }
    if parts[0] != "ONNECT" {
        return Err(Error::UnknownCmd(0));
    }
    let parsed_url = Url::parse(format!("{}{}", "http://", parts[1]).as_str());
    if parsed_url.is_err() {
        return Err(Error::Other("Bad Request Url".to_string()));
    }
    let mut ret_req = Request {
        ver: Ver::Http,
        cmd: ReqCmd::Connect,
        dst_addr: None,
        dst_domain: None,
        dst_port: 80,
        rsv: 0,
        a_type: AType::Domain,
        raw: buf_reader.into_inner().await?,
    };
    let parsed_url = parsed_url.unwrap();
    match parsed_url.host() {
        Some(url::Host::Ipv4(ipv4)) => {
            ret_req.a_type = AType::Ipv4;
            ret_req.dst_addr = Some(IpAddr::V4(ipv4));
        }
        Some(url::Host::Ipv6(ipv6)) => {
            ret_req.a_type = AType::Ipv6;
            ret_req.dst_addr = Some(IpAddr::V6(ipv6));
        }
        Some(url::Host::Domain(domain)) => {
            ret_req.dst_domain = Some(domain.to_string());
        }
        None => return Err(Error::Other("Bad Request Host".to_string()))
    }
    ret_req.dst_port = parsed_url.port().unwrap_or(ret_req.dst_port);
    Ok(ret_req)
}

async fn get_addr(src: &mut Cursor<&[u8]>, buf_reader: &mut BufReader) -> Result<(Option<IpAddr>, Option<String>, AType)> {
    match buf_reader.get_u8(src).await? {
        1 => Ok((Some(IpAddr::V4(Ipv4Addr::from(buf_reader.get_u32(src).await?))), None, AType::Ipv4)),
        3 => {
            let a_len = buf_reader.get_u8(src).await? as usize;
            Ok((None, Some(String::from_utf8(buf_reader.get_n_bytes(src, a_len).await?)?), AType::Domain))
        }
        4 => Ok((Some(IpAddr::V6(Ipv6Addr::from(buf_reader.get_u128(src).await?))), None, AType::Ipv6)),
        _ => Err(Error::AddrTypeUnsupported(5))
    }
}

struct BufReader {
    buffer: BufWriter<Vec<u8>>,
}

impl BufReader {
    fn with_capacity(size: usize) -> BufReader {
        BufReader {
            buffer: BufWriter::with_capacity(size, vec![])
        }
    }
    async fn into_inner(mut self) -> Result<Vec<u8>> {
        self.buffer.flush().await?;
        Ok(self.buffer.into_inner())
    }
    async fn get_u8(&mut self, src: &mut Cursor<&[u8]>) -> Result<u8> {
        if !src.has_remaining() {
            return Err(Error::Incomplete);
        }
        let ret = src.get_u8();
        self.buffer.write_u8(ret).await?;
        Ok(ret)
    }

    async fn get_u16(&mut self, src: &mut Cursor<&[u8]>) -> Result<u16> {
        if src.remaining() < 2 {
            return Err(Error::Incomplete);
        }
        let ret = src.get_u16();
        self.buffer.write_u16(ret).await?;
        Ok(ret)
    }

    async fn get_u32(&mut self, src: &mut Cursor<&[u8]>) -> Result<u32> {
        if src.remaining() < 4 {
            return Err(Error::Incomplete);
        }
        let ret = src.get_u32();
        self.buffer.write_u32(ret).await?;
        Ok(ret)
    }

    async fn get_u128(&mut self, src: &mut Cursor<&[u8]>) -> Result<u128> {
        if src.remaining() < 128 {
            return Err(Error::Incomplete);
        }
        let ret = src.get_u128();
        self.buffer.write_u128(ret).await?;
        Ok(ret)
    }

    async fn get_n_bytes(&mut self, src: &mut Cursor<&[u8]>, n: usize) -> Result<Vec<u8>> {
        if src.remaining() < n {
            return Err(Error::Incomplete);
        }
        let mut result = Vec::new();
        let mut n = n;
        while n > 0 {
            result.push(self.get_u8(src).await?);
            n -= 1;
        }
        Ok(result)
    }

    async fn get_until(&mut self, src: &mut Cursor<&[u8]>, c: u8) -> Result<Vec<u8>> {
        let mut result = Vec::new();
        let read_len = src.read_until(c, &mut result)?;
        if read_len == 0 {
            return Err(Error::Incomplete);
        }
        self.buffer.write_buf(&mut BytesMut::from(&result[..])).await?;
        Ok(result)
    }

    async fn get_line(&mut self, src: &mut Cursor<&[u8]>) -> Result<String> {
        let line = self.get_until(src, b'\r').await?;
        let next = self.get_u8(src).await?;
        if next != b'\n' {
            return Err(Error::Other("broken line".to_string()));
        }
        Ok(String::from_utf8_lossy(&line[..line.len() - 1]).to_string())
    }
}

#[cfg(test)]
mod test {
    use crate::protocol::{parse_req_http_connect, parse_req_v4, recv_and_parse_req, AType, BufReader, ReqCmd, ReqFrame, Request, Ver};
    use std::io::{BufWriter, Cursor};
    use std::net::{IpAddr, Ipv4Addr};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use bytes::{Buf, BufMut};
    use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
    use crate::error::Error;

    #[tokio::test] // todo: it tests parsing protocol v4 that is completed
    async fn parse_v4_that_completed() {
        let buf: Vec<u8> = vec![4, 1, 0x22, 0xc3, 0xc0, 0xa8, 1, 1, 0];
        let mut cursor = Cursor::new(&buf[..]);
        let mut buf_reader = BufReader::with_capacity(64);
        buf_reader.get_u8(&mut cursor).await.unwrap();
        let ret = parse_req_v4(&mut cursor, buf_reader).await.unwrap();
        assert_eq!(ret, Request {
            ver: Ver::V4,
            cmd: ReqCmd::Connect,
            rsv: 0,
            dst_domain: None,
            dst_addr: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
            dst_port: 8899,
            a_type: AType::Ipv4,
            raw: buf,
        });
    }
    // #[test] // todo: it tests parsing protocol v4 that is incomplete
    // fn parse_v4_that_incomplete() {
    //     let buf: Vec<u8> = vec![4, 1, 0x22, 0xc3, 0xc0, 0xa8];
    //     let mut cursor = Cursor::new(&buf[..]);
    //     cursor.advance(1);
    //     let ret = parse_req_v4(&mut cursor);
    //     match ret {
    //         Ok(_) => { assert!(false); }
    //         Err(e) => {
    //             match e {
    //                 Error::Incomplete => assert!(true),
    //                 _ => assert!(false)
    //             }
    //         }
    //     }
    // }
    //
    // #[test] // todo: it tests parsing protocol v5 that completed
    // fn parse_v5_that_completed() {
    //     let buf: Vec<u8> = vec![5, 1, 0, 1, 0xc0, 0xa8, 1, 1, 0x22, 0xc3];
    //     let mut cursor = Cursor::new(&buf[..]);
    //     cursor.advance(1);
    //     let ret = parse_req_v5(&mut cursor, &mut BufReader::with_capacity(64)).unwrap();
    //     assert_eq!(ret, Request {
    //         ver: Ver::V5,
    //         cmd: ReqCmd::Connect,
    //         rsv: 0,
    //         dst_domain: None,
    //         dst_addr: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
    //         dst_port: 8899,
    //         a_type: AType::Ipv4,
    //         raw: buf,
    //     });
    // }
    // #[tokio::test] // todo: it tests parsing protocol v5 that completed
    // async fn parse_v5_that_completed_with_domain() {
    //     let buf: Vec<u8> = vec![5, 1, 0, 3, 0x08, 0x6e, 0x65, 0x78, 0x65, 0x6c, 0x2e, 0x63, 0x63, 0x22, 0xc3];
    //     let mut cursor = Cursor::new(&buf[..]);
    //     cursor.advance(1);
    //     let ret = parse_req_v5(&mut cursor).await.unwrap();
    //     assert_eq!(ret, Request {
    //         ver: Ver::V5,
    //         cmd: ReqCmd::Connect,
    //         rsv: 0,
    //         dst_domain: Some(String::from("Nexel.cc")),
    //         dst_addr: None,
    //         dst_port: 8899,
    //         a_type: AType::Domain,
    //         raw: buf,
    //     });
    // }
    // #[test] // todo: it tests parsing protocol v5 that incomplete
    // fn parse_v5_that_incomplete() {
    //     let buf: Vec<u8> = vec![5, 1, 0, 3, 0x08, 0x6e, 0x65, 0x78, 0x65, 0x6c, 0x2e, 0x63];
    //     let mut cursor = Cursor::new(&buf[..]);
    //     cursor.advance(1);
    //     let ret = parse_req_v5(&mut cursor);
    //     match ret {
    //         Ok(_) => { assert!(false); }
    //         Err(e) => {
    //             match e {
    //                 Error::Incomplete => assert!(true),
    //                 _ => assert!(false)
    //             }
    //         }
    //     }
    // }

    #[test]
    fn split() {
        let mut str = "nexel.cc".split(':');
        assert_eq!(str.next(), Some("nexel.cc"));
        assert_eq!(str.next(), None);

        let mut str = "nexel.cc:".split(':');
        assert_eq!(str.next(), Some("nexel.cc"));
        assert_eq!(str.next(), Some(""));
    }

    #[tokio::test]
    async fn parse_http_req_with_incomplete() {
        let mut cursor = Cursor::new("CONNECT http://nexel.cc HTTP/1.1\r\n".as_bytes());
        cursor.advance(1);
        let buf_reader = BufReader::with_capacity(64);
        let ret = parse_req_http_connect(&mut cursor, buf_reader).await;
        match ret {
            Ok(_) => assert!(false),
            Err(e) => {
                match e {
                    Error::Incomplete => assert!(true),
                    _ => assert!(false),
                }
            }
        }
    }
    #[tokio::test]
    async fn parse_http_req_with_successful() {
        let req = "CONNECT exp.notion.so:443 HTTP/1.1\r\nHost: exp.notion.so:443\r\nProxy-Connection: keep-alive\r\nUser-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Notion/3.13.0 Chrome/126.0.6478.127 Electron/31.2.0 Safari/537.36 WantsServiceWorker\r\n\r\n";
        let mut cursor = Cursor::new(req.as_bytes());
        let mut buf_reader = BufReader::with_capacity(64);
        buf_reader.get_u8(&mut cursor).await.unwrap();
        let ret = parse_req_http_connect(&mut cursor, buf_reader).await.unwrap();
        assert_eq!(ret, Request {
            ver: Ver::Http,
            cmd: ReqCmd::Connect,
            rsv: 0,
            dst_domain: Some(String::from("exp.notion.so")),
            dst_port: 443,
            dst_addr: None,
            a_type: AType::Domain,
            raw: req.as_bytes().to_vec(),
        });
        assert_eq!(ret.raw(), req.as_bytes());
    }
    #[test]
    fn parse_host() {
        println!("{}", reqwest::Url::parse("http://nexel.cc").unwrap().host().unwrap())
    }
    #[tokio::test]
    async fn test_buf_writer() {
        let mut buf = tokio::io::BufWriter::with_capacity(10, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        buf.write_u8(11).await.unwrap();
        buf.flush().await.unwrap();
        println!("{:?}", buf.into_inner());
    }
    struct TestIO {
        times: usize,
        half1: Vec<u8>,
        half2: Vec<u8>,
        half3: Vec<u8>,
    }

    impl AsyncWrite for TestIO {
        fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncRead for TestIO {
        fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            if self.times == 0 {
                buf.put_slice(&self.half1[..]);
                self.get_mut().times += 1;
                Poll::Ready(Ok(()))
            } else if self.times == 1 {
                buf.put_slice(&self.half2[..]);
                self.get_mut().times += 1;
                Poll::Ready(Ok(()))
            } else {
                buf.put_slice(&self.half3[..]);
                Poll::Ready(Ok(()))
            }
        }
    }

    impl Unpin for TestIO {}

    #[tokio::test]
    async fn poll_read_with_socks_v4() {
        let mut io = TestIO {
            times: 0,
            half1: vec![4, 1, 0x22, 0xc3, 0xc0],
            half2: vec![0xa8, 1, 1, 0],
            half3: vec![]
        };
        let ret = recv_and_parse_req(&mut io, true).await.unwrap();
        assert_eq!(ret, Some(ReqFrame::Req(Request {
            ver: Ver::V4,
            cmd: ReqCmd::Connect,
            rsv: 0,
            dst_domain: None,
            dst_addr: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
            dst_port: 8899,
            a_type: AType::Ipv4,
            raw: vec![4, 1, 0x22, 0xc3, 0xc0, 0xa8, 1, 1, 0],
        })))
    }

    #[tokio::test]
    async fn poll_read_with_http() {
        let req1 = "CONNECT nexel.cc:443 HTTP/1.1\r";
        let req2 = "\nHost: nexel.cc:443\r\n";
        let req3 = "\r\n";
        let mut io = TestIO {
            times: 0,
            half1: req1.as_bytes().to_vec(),
            half2: req2.as_bytes().to_vec(),
            half3: req3.as_bytes().to_vec(),
        };
        let ret = recv_and_parse_req(&mut io, true).await.unwrap();
        let mut raw: Vec<u8> = vec![];
        raw.put_slice(req1.as_bytes());
        raw.put_slice(req2.as_bytes());
        raw.put_slice(req3.as_bytes());
        assert_eq!(ret, Some(ReqFrame::Req(Request {
            ver: Ver::Http,
            cmd: ReqCmd::Connect,
            rsv: 0,
            dst_domain: Some(String::from("nexel.cc")),
            dst_addr: None,
            dst_port: 443,
            a_type: AType::Domain,
            raw,
        })))
    }
}
