pub mod v4;
pub mod v5;

use std::fmt::{Display, Formatter};
use std::io::{BufRead, Cursor};
use std::net::IpAddr;
use bytes::Buf;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use crate::Result;
use crate::error::Error;

#[derive(Debug, PartialEq)]
pub struct ReqAuth {
    pub methods: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq)]
pub struct ReplyAuth {
    pub method: Method,
}

impl ReplyAuth {
    pub async fn send(&self, stream: &mut BufWriter<TcpStream>) -> Result<()> {
        stream.write_u8(5).await?;
        stream.write_u8(self.method as u8).await?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Method {
    None = 0,
    GssApi = 1,
    UserVerify = 2,
    IanaAssigned = 3,
    Private = 0x80,
}

impl Method {
    pub fn from_u8(byte: u8) -> Option<Method> {
        match byte {
            0 => Some(Method::None),
            1 => Some(Method::GssApi),
            2 => Some(Method::UserVerify),
            0x03..=0x7f => Some(Method::IanaAssigned),
            0x80..=0xfe => Some(Method::Private),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub ver: NVer,
    pub cmd: ReqCmd,
    pub dst_ip: Option<IpAddr>,
    pub dst_port: u16,
    pub user_id: Option<String>,
    pub rsv: u8,
    pub domain: Option<String>,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ReqCmd {
    Connect = 1,
    Bind = 2,
    Udp = 3,
}

impl Display for ReqCmd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ReqCmd::Connect => write!(f, "CONNECT"),
            ReqCmd::Bind => write!(f, "BIND"),
            ReqCmd::Udp => write!(f, "UDP")
        }
    }
}

impl ReqCmd {
    pub fn from_u8(byte: u8) -> Option<ReqCmd> {
        match byte {
            1 => Some(ReqCmd::Connect),
            2 => Some(ReqCmd::Bind),
            3 => Some(ReqCmd::Udp),
            _ => None
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum NVer {
    V4 = 4,
    V5 = 5,
}

impl Display for NVer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            NVer::V4 => write!(f, "V4"),
            NVer::V5 => write!(f, "V5"),
        }
    }
}

impl Display for Request {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "vn: {}, cmd: {:?}, dst_ip: {}, domain: {}, dst_port: {}, user_id: {}",
            self.ver,
            self.cmd,
            if let Some(ip) = self.dst_ip { ip.to_string() } else { "".to_string() },
            if let Some(domain) = &self.domain { domain } else { "" },
            self.dst_port,
            if let Some(user_id) = &self.user_id { user_id } else { "" }
        )
    }
}

#[derive(Debug, PartialEq)]
pub struct Reply {
    pub ver: NVer,
    pub cmd: ReplyCmd,
    pub rsv: u8,
    pub bind_addr: Option<IpAddr>,
    pub domain: Option<String>,
    pub bind_port: u16,
}

impl Reply {
    pub async fn send(&self, stream: &mut tokio::io::BufWriter<TcpStream>) -> Result<()> {
        match self.ver {
            NVer::V4 => v4::send_reply(self, stream).await,
            NVer::V5 => v5::send_reply(self, stream).await,
        }
    }
}

impl Display for Reply {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "vn: {}, cmd: {:?}, dst_ip: {}, dst_port: {}",
            self.ver, self.cmd,
            if let Some(addr) = self.bind_addr { addr.to_string() } else { "".to_string() },
            self.bind_port
        )
    }
}

#[derive(Debug, PartialEq)]
pub enum ReplyCmd {
    V4(v4::ReplyCmd),
    V5(v5::ReplyCmd),
}

impl ReplyCmd {
    pub fn unwrap_v4(&self) -> Result<v4::ReplyCmd> {
        match self {
            ReplyCmd::V4(cmd) => Ok(*cmd),
            _ => Err(Error::Other("invalid cmd".to_string()))
        }
    }

    pub fn unwrap_v5(&self) -> Result<v5::ReplyCmd> {
        match self {
            ReplyCmd::V5(cmd) => Ok(*cmd),
            _ => Err(Error::Other("invalid cmd".to_string()))
        }
    }
}

pub fn parse_auth_req(src: &mut Cursor<&[u8]>) -> Result<ReqAuth> {
    v5::parse_auth_req(src)
}

pub fn parse_req(src: &mut Cursor<&[u8]>, ver: NVer) -> Result<Request> {
    match ver {
        NVer::V4 => v4::parse_req(src),
        NVer::V5 => v5::parse_req(src),
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
