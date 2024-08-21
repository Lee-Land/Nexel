use std::fmt::{Display, Formatter};
use std::io::{Cursor};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tokio::io::{AsyncWriteExt};
use tokio::net::TcpStream;
use crate::protocol::{get_n_bytes, get_u128, get_u16, get_u32, get_u8, NVer, Reply, ReplyAuth, ReqAuth, ReqCmd, Request};
use crate::Result;
use crate::error::Error;

pub async fn send_reply(
    reply: &Reply,
    stream: &mut tokio::io::BufWriter<TcpStream>,
) -> Result<()> {
    stream.write_u8(5).await?; // ver
    if let crate::protocol::ReplyCmd::V5(cmd) = reply.cmd {
        stream.write_u8(cmd as u8).await?;
    } else {
        return Err(Error::Other("the cmd type was not a v5::ReplyCmd".to_string()));
    }
    stream.write_u8(0).await?; // rsv
    send_addr(stream, (&reply.bind_addr, &reply.domain), reply.bind_port).await?; // a_type, dst.addr, dst.port
    stream.flush().await?;
    Ok(())
}

async fn send_addr(
    stream: &mut tokio::io::BufWriter<TcpStream>,
    addr: (&Option<IpAddr>, &Option<String>),
    port: u16) -> Result<()> {
    let (ip_addr, domain) = addr;
    if let Some(domain) = domain {
        stream.write_u8(3).await?;
        let len = if domain.len() <= 255 { domain.len() } else { 255 };
        stream.write_u8(len as u8).await?;
        stream.write(&domain[..len].as_bytes()).await?;
    } else if ip_addr.is_none() {
        return Err(Error::AddrTypeUnsupported);
    } else {
        match ip_addr.unwrap() {
            IpAddr::V4(ip) => {
                stream.write_u8(1).await?;
                stream.write_u32(u32::from(ip)).await?;
            }
            IpAddr::V6(ip) => {
                stream.write_u8(4).await?;
                stream.write_u128(u128::from(ip)).await?;
            }
        }
    }
    stream.write_u16(port).await?;
    Ok(())
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ReplyCmd {
    Successful = 0,
    ServerError = 1,
    RulesNotAllowed = 2,
    NetworkUnreachable = 3,
    HostUnreachable = 4,
    ConnectionRefused = 5,
    TtlExpired = 6,
    CmdUnsupported = 7,
    AddrTypeUnsupported = 8,
}

impl ReplyCmd {
    pub fn from_u8(byte: u8) -> Option<ReplyCmd> {
        match byte {
            0 => Some(ReplyCmd::Successful),
            1 => Some(ReplyCmd::ServerError),
            2 => Some(ReplyCmd::RulesNotAllowed),
            3 => Some(ReplyCmd::NetworkUnreachable),
            4 => Some(ReplyCmd::HostUnreachable),
            5 => Some(ReplyCmd::ConnectionRefused),
            6 => Some(ReplyCmd::TtlExpired),
            7 => Some(ReplyCmd::CmdUnsupported),
            8 => Some(ReplyCmd::AddrTypeUnsupported),
            _ => None,
        }
    }
}

impl Display for ReplyCmd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplyCmd::Successful => write!(f, "SUCCESSFUL"),
            ReplyCmd::ServerError => write!(f, "SERVER_ERROR"),
            ReplyCmd::RulesNotAllowed => write!(f, "RULES_NOT_ALLOWED"),
            ReplyCmd::NetworkUnreachable => write!(f, "NETWORK_UNREACHABLE"),
            ReplyCmd::HostUnreachable => write!(f, "HOST_UNREACHABLE"),
            ReplyCmd::ConnectionRefused => write!(f, "CONNECTION_REFUSED"),
            ReplyCmd::TtlExpired => write!(f, "TTL_EXPIRED"),
            ReplyCmd::CmdUnsupported => write!(f, "CMD_UNSUPPORTED"),
            ReplyCmd::AddrTypeUnsupported => write!(f, "ADDR_TYPE_UNSUPPORTED"),
        }
    }
}

pub fn parse_auth_req(src: &mut Cursor<&[u8]>) -> Result<ReqAuth> {
    let n_methods = get_u8(src)?;
    Ok(ReqAuth {
        methods: if n_methods > 0 { Some(get_n_bytes(src, n_methods as usize)?) } else { None },
    })
}

pub fn parse_auth_reply(src: &mut Cursor<&[u8]>) -> Result<ReplyAuth> {
    if let Some(method) = crate::protocol::Method::from_u8(get_u8(src)?) {
        Ok(ReplyAuth { method })
    } else {
        Err(Error::ServerRefusedAuth)
    }
}

pub fn parse_req(src: &mut Cursor<&[u8]>) -> Result<Request> {
    let n_cmd = get_u8(src)?;
    let cmd = ReqCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(n_cmd));
    }
    let rsv = get_u8(src)?; // rsv
    let (dst_addr, domain) = get_addr(src)?;
    let port = get_u16(src)?;
    Ok(Request {
        ver: NVer::V5,
        cmd: cmd.unwrap(),
        dst_ip: dst_addr,
        domain,
        dst_port: port,
        user_id: None,
        rsv,
    })
}

fn get_addr(src: &mut Cursor<&[u8]>) -> Result<(Option<IpAddr>, Option<String>)> {
    match get_u8(src)? {
        1 => Ok((Some(IpAddr::V4(Ipv4Addr::from(get_u32(src)?))), None)),
        3 => {
            let a_len = get_u8(src)? as usize;
            Ok((None, Some(String::from_utf8(get_n_bytes(src, a_len)?)?)))
        }
        4 => Ok((Some(IpAddr::V6(Ipv6Addr::from(get_u128(src)?))), None)),
        _ => Err(Error::AddrTypeUnsupported)
    }
}

pub fn parse_reply(src: &mut Cursor<&[u8]>) -> Result<Reply> {
    let n_cmd = get_u8(src)?;
    let cmd = ReplyCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(n_cmd));
    }
    let cmd = cmd.unwrap();
    let rsv = get_u8(src)?;
    let (bind_addr, domain) = get_addr(src)?;
    let port = get_u16(src)?;
    Ok(Reply {
        ver: NVer::V5,
        cmd: crate::protocol::ReplyCmd::V5(cmd),
        bind_addr,
        domain,
        bind_port: port,
        rsv,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use bytes::BytesMut;
    use crate::protocol::{Error, get_u8, NVer};
    use crate::protocol::v5::{parse_req, Request};
    use crate::protocol::v5::ReqCmd::Bind;

    #[test]
    fn test_parse() {
        let origin: Vec<u8> = vec![0x05, 0x02, 0x00, 0x03, 0x0e, 0x77, 0x77, 0x77, 0x2e, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x2e, 0x63, 0x6f, 0x6d, 0x18, 0xeb];
        let buffer = BytesMut::from(&origin[..]);
        let mut buf = Cursor::new(&buffer[..]);
        let ver = get_u8(&mut buf).unwrap();
        _ = ver;
        assert_eq!(parse_req(&mut buf).unwrap(), Request {
            ver: NVer::V5,
            cmd: Bind,
            dst_ip: None,
            domain: Some("www.google.com".to_string()),
            dst_port: 6379,
            user_id: None,
            rsv: 0,
        });
    }

    #[test]
    fn test_parse_with_incomplete() {
        let origin: Vec<u8> = vec![0x05, 0x02, 0x00, 0x03, 0x0e, 0x77, 0x77, 0x77, 0x2e, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x2e, 0x63, 0x6f, 0x6d, 0x18];
        let buffer = BytesMut::from(&origin[..]);
        let mut buf = Cursor::new(&buffer[..]);
        let ver = get_u8(&mut buf).unwrap();
        _ = ver;
        let ret = parse_req(&mut buf);
        assert!(ret.is_err());
        if let Err(e) = ret {
            match e {
                Error::Incomplete => {}
                _ => assert!(false)
            }
        }
    }
}
