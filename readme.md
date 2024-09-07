# 概览
Nexel 是一个使用 rust 开发的基于规则的代理服务器软件，支持 HTTP，SOCKS4, SOCKS5协议。并且可以支持代理客户端与代理服务器之间加密通信。  
注：**仅供学习和参考。**

使用如下命令编译源代码：
```shell
cargo build --release
```
会在 ``./target/release`` 目录下生成两个二进制文件
```shell
target/
|__ release/
    |__ nexel
    |__ nexeld
```
- nexel 为客户端程序，一般部署在本地。
- nexeld 为服务器程序，部署在远程，用于代理出口网络流量。
## 运行
```shell
# client
./nexel -p 3456 -h remote_domain -o remote_port -t -c cert.crt -r rule.ymal
```
- -p 监听端口
- -h 服务器地址
- -o 服务器端口
- -t 与服务器通信使用 TLS 加密
- -c 指定 TLS 证书路径
- -r 指定规则定义文件，可以使用参考给出的自定义 rule.yaml 文件，也可参考 [rules 规则](https://clash.wiki/configuration/rules.html)
- -g 指定 mmdb 文件，用于查询 IP 所属地区数据库，可以使用仓库给出的 GeoLite2-Country.mmdb文件，也可以参考 [MAXMIND](https://www.maxmind.com/en/accounts/1057003/geoip/downloads)
```shell
# server
./nexeld -p 6789 -t -c cert_path -k private_key_path
```
- -p 监听端口
- -t 与客户端建立 tls 加密连接
- -c TLS 证书路径
- -k TLS 私钥路径
## 拓扑图
