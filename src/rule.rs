use std::collections::HashMap;
use std::net::{IpAddr};
use std::path::PathBuf;
use lazy_static::lazy_static;
use std::sync::Mutex;
use serde::Deserialize;
use serde_yml;
use crate::error::Error;
use std::str::FromStr;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Routing {
    Direct,
    Proxy,
    Reject, // actually, haven't used it yet.
}
impl TryFrom<&str> for Routing {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "DIRECT" => Ok(Routing::Direct),
            "PROXY" => Ok(Routing::Proxy),
            "REJECT" => Ok(Routing::Reject),
            _ => Err(Error::Other("unknown routing".to_string()))
        }
    }
}
#[derive(Deserialize, Debug)]
struct Rule {
    rules: Vec<String>,
}

lazy_static! {
    static ref domain_set: Mutex<HashMap<String, Routing>> = Mutex::new(HashMap::new());
    static ref domain_suffix_set: Mutex<HashMap<String, Routing>> = Mutex::new(HashMap::new());
    static ref domain_keyword_set: Mutex<HashMap<String, Routing>> = Mutex::new(HashMap::new());
    static ref ip_cidr: Mutex<HashMap<ipnetwork::IpNetwork, Routing>> = Mutex::new(HashMap::new());
    static ref ip_cidr6: Mutex<HashMap<ipnetwork::IpNetwork, Routing>> = Mutex::new(HashMap::new());
    static ref geo_ip: Mutex<HashMap<String, Routing>> = Mutex::new(HashMap::new());

    static ref maxmindb_reader: Mutex<maxminddb::Reader<Vec<u8>>> = {
        let reader = maxminddb::Reader::open_readfile(PathBuf::from("GeoLite2-Country.mmdb")).unwrap();
        Mutex::new(reader)
    };
}

pub fn initial() -> Result<(), Box<dyn std::error::Error>> {
    let rule_yaml = std::fs::File::open("rule.yaml")?;
    let rule: Rule = serde_yml::from_reader(rule_yaml)?;
    for item in rule.rules {
        let mut split_iter = item.split(',');
        let kind = split_iter.next().unwrap_or("_");
        let content = split_iter.next().unwrap_or("_");
        let routing = split_iter.next().unwrap_or("_");
        match kind {
            "DOMAIN" => {
                domain_set.lock().unwrap().insert(content.to_string().clone(), Routing::try_from(routing)?);
            }
            "DOMAIN-SUFFIX" => {
                domain_suffix_set.lock().unwrap().insert(content.to_string().clone(), Routing::try_from(routing)?);
            }
            "DOMAIN-KEYWORD" => {
                domain_keyword_set.lock().unwrap().insert(content.to_string().clone(), Routing::try_from(routing)?);
            }
            "IP-CIDR" => {
                let cidr = ipnetwork::IpNetwork::from_str(content)?;
                ip_cidr.lock().unwrap().insert(cidr, Routing::try_from(routing)?);
            }
            "IP-CIDR6" => {
                let cidr = ipnetwork::IpNetwork::from_str(content)?;
                ip_cidr6.lock().unwrap().insert(cidr, Routing::try_from(routing)?);
            }
            _ => continue,
        };
    }
    Ok(())
}

pub async fn domain(domain: &str) -> crate::Result<Routing> {
    if let Some(routing) = domain_set.lock().unwrap().get(domain) {
        return Ok(routing.clone());
    }
    for (suffix, routing) in domain_suffix_set.lock().unwrap().iter() {
        if domain.ends_with(suffix) {
            return Ok(routing.clone());
        }
    }
    for (keyword, routing) in domain_keyword_set.lock().unwrap().iter() {
        if domain.contains(keyword) {
            return Ok(routing.clone());
        }
    }

    let mut addrs_iter = tokio::net::lookup_host(domain).await?;

    if let Some(routing) = addrs_iter.next().map(|ret| ip(ret.ip())) {
        return Ok(routing.clone());
    }

    Ok(Routing::Proxy)
}

pub fn ip(ip: IpAddr) -> Routing {
    let cidr_ip_list = match ip {
        IpAddr::V4(_) => ip_cidr.lock().unwrap(),
        IpAddr::V6(_) => ip_cidr6.lock().unwrap()
    };
    for (cidr, routing) in cidr_ip_list.iter() {
        if cidr.contains(ip) {
            return routing.clone();
        }
    }

    if let Ok(country) =
        maxmindb_reader.lock().unwrap().lookup::<maxminddb::geoip2::Country>(ip) {
        if let Some(c) = country.country {
            if c.iso_code.unwrap_or("_") == "CN" {
                return Routing::Direct;
            }
        }
    }

    Routing::Proxy
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::str::FromStr;
    use std::path::PathBuf;
    use crate::rule;
    use crate::rule::Routing;

    #[tokio::test]
    async fn check_domain() {
        rule::initial().unwrap();
        assert_eq!(rule::domain("itunes.apple.com").await.unwrap(), Routing::Proxy);
        assert_eq!(rule::domain("www.163.com").await.unwrap(), Routing::Direct);
        assert_eq!(rule::domain("pan.baidu.com").await.unwrap(), Routing::Direct);
    }

    #[test]
    fn check_ip() {
        rule::initial().unwrap();
        assert_eq!(rule::ip("8.220.210.182".parse::<IpAddr>().unwrap()), Routing::Proxy);
    }

    #[test]
    fn geo_lite_ip() {
        let reader = maxminddb::Reader::open_readfile(PathBuf::from("GeoLite2-Country.mmdb")).unwrap();
        let ip = "8.220.210.182".parse::<IpAddr>().unwrap();
        let county: maxminddb::geoip2::Country = reader.lookup(ip).unwrap();
        println!("{:?}", county);
    }

    #[test]
    fn ipnetwork_test() {
        let cidr = ipnetwork::IpNetwork::from_str("127.0.0.1/8").unwrap();
        println!("{}", cidr.network());
        println!("{}", cidr.ip());

        let ip = IpAddr::from_str("127.0.0.255").unwrap();
        println!("{}", cidr.contains(ip));
    }
}
