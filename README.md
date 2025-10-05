# frp

## 简易内网穿透

## server

```bash
frp server
frp server ./config.toml
```

## client

```bash
frp client
frp client ./config.toml
```

## config

```toml
secret = "secret" # 密钥
timeout = 20000 # 超时
heartbeat = 5000 # 心跳
client_addr = "127.0.0.1:7777" # 客户端本地地址
server_addr = "10.0.0.1:12345" # 服务器认证地址
accept_addr = "10.0.0.:8000" # 服务器对外访问地址
```
