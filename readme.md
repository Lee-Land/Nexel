# 概览
Nexel 是一个使用 rust 开发的基于规则的代理服务器软件，支持 HTTP，SOCKS4, SOCKS5协议。并且可以支持代理客户端与代理服务器之间加密通信。

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