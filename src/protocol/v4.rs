use std::fmt::{Display, Formatter};
use std::io::{Cursor};
use std::net::{IpAddr, Ipv4Addr};
use tokio::io::{AsyncWriteExt};
use tokio::net::TcpStream;
use crate::error::Error;
use crate::protocol::{get_until, get_u8, get_u16, get_u32, Request, Reply, NVer, ReqCmd};
use crate::Result;
pub async fn send_req(req: Request, stream: &mut tokio::io::BufWriter<TcpStream>) -> Result<()> {
    stream.write_u8(4).await?;
    stream.write_u8(req.cmd as u8).await?;
    stream.write_u16(req.dst_port).await?;
    if let IpAddr::V4(ip_v4) = req.dst_ip.unwrap() {
        stream.write_u32(u32::from(ip_v4)).await?;
    } else {
        return Err(Error::Other("the ip_addr type was not a IpAddr::V4".to_string()));
    }
    if let Some(user_id) = req.user_id {
        stream.write_all(user_id.as_bytes()).await?;
        stream.write_u8(0x00).await?;
    }
    stream.flush().await?;
    Ok(())
}

pub async fn send_reply(reply: &Reply, stream: &mut tokio::io::BufWriter<TcpStream>) -> Result<()> {
    stream.write_u8(0).await?;
    stream.write_u8(reply.cmd.unwrap_v4()? as u8).await?;
    stream.write_u16(reply.bind_port).await?;
    if let IpAddr::V4(ip_v4) = reply.bind_addr.unwrap() {
        stream.write_u32(u32::from(ip_v4)).await?;
    } else {
        return Err(Error::Other("the ip_addr type was not a IpAddr::V4".to_string()));
    }
    stream.flush().await?;
    Ok(())
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ReplyCmd {
    Successful = 90,
    ConnectionRefused = 91,
    ConnectionFailed = 92,
    ConnectionFailedWithUserId = 93,
}

impl Display for ReplyCmd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplyCmd::Successful => write!(f, "SUCCESSFUL"),
            ReplyCmd::ConnectionRefused => write!(f, "CONNECTION_REFUSED"),
            ReplyCmd::ConnectionFailed => write!(f, "CONNECTION_FAILED"),
            ReplyCmd::ConnectionFailedWithUserId => write!(f, "CONNECTION_FAILED_WITH_USER_ID"),
        }
    }
}

impl ReplyCmd {
    pub fn from_u8(byte: u8) -> Option<ReplyCmd> {
        match byte {
            90 => Some(ReplyCmd::Successful),
            91 => Some(ReplyCmd::ConnectionRefused),
            92 => Some(ReplyCmd::ConnectionFailed),
            93 => Some(ReplyCmd::ConnectionFailedWithUserId),
            _ => None
        }
    }
}

pub fn parse_req(src: &mut Cursor<&[u8]>) -> Result<Request> {
    let n_cmd = get_u8(src)?;
    let cmd = ReqCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(n_cmd));
    }
    let cmd = cmd.unwrap();
    let (port, ipv4) = parse_addr(src)?;
    let user_id: String = String::from_utf8(get_until(src, 0x00)?)?;
    Ok(Request {
        ver: NVer::V4,
        cmd,
        dst_ip: Some(IpAddr::V4(Ipv4Addr::from(ipv4))),
        dst_port: port,
        user_id: Some(user_id),
        rsv: 0,
        domain: None,
    })
}

pub fn parse_reply(src: &mut Cursor<&[u8]>) -> Result<Reply> {
    let n_cmd = get_u8(src)?;
    let cmd = ReplyCmd::from_u8(n_cmd);
    if cmd.is_none() {
        return Err(Error::UnknownCmd(n_cmd));
    }
    let cmd = cmd.unwrap();
    let (port, ipv4) = parse_addr(src)?;
    Ok(Reply {
        ver: NVer::V4,
        cmd: crate::protocol::ReplyCmd::V4(cmd),
        bind_addr: Some(IpAddr::V4(Ipv4Addr::from(ipv4))),
        domain: None,
        bind_port: port,
        rsv: 0,
    })
}

fn parse_addr(src: &mut Cursor<&[u8]>) -> Result<(u16, u32)> {
    let port = get_u16(src)?;
    let ipv4 = get_u32(src)?;
    Ok((port, ipv4))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor};
    use std::net::{IpAddr, Ipv4Addr};
    use bytes::BytesMut;
    use crate::protocol::{Error, get_u8, NVer};
    use crate::protocol::v4::{parse_req, ReqCmd, Request};

    #[test]
    fn test_parse() {
        let origin: Vec<u8> = vec![0x04, 0x01, 0x01, 0xbb, 0xc0, 0xa8, 0x01, 0x01, 0x41, 0x42, 0x43, 0x00];
        let buffer = BytesMut::from(&origin[..]);
        let mut buf = Cursor::new(&buffer[..]);
        get_u8(&mut buf).unwrap();
        assert_eq!(parse_req(&mut buf).unwrap(), Request {
            ver: NVer::V4,
            cmd: ReqCmd::Connect,
            dst_ip: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
            dst_port: 443,
            user_id: Some("ABC".into()),
            rsv: 0,
            domain: None,
        });
    }

    #[test]
    fn test_cmd() {}

    #[test]
    fn test_parse_with_incomplete() {
        let origin: Vec<u8> = vec![0x04, 0x01, 0x01, 0xbb, 0xc0, 0xa8, 0x01, 0x01, 0x41, 0x42, 0x43];
        let buffer = BytesMut::from(&origin[..]);
        let mut buf = Cursor::new(&buffer[..]);
        get_u8(&mut buf).unwrap();
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
