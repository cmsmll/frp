use std::{
    io::{self, Error},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use async_channel::{Receiver, Sender, unbounded};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    time::sleep,
};

use crate::{Config, Message, forward};

pub struct Accept {
    tcp: TcpStream,
    addr: SocketAddr,
    instant: Instant,
}

impl Accept {
    pub fn new(tcp: TcpStream, addr: SocketAddr) -> Self {
        let instant = Instant::now();
        Self { tcp, addr, instant }
    }

    pub fn is_timeout(&self, timeout: Duration) -> bool {
        self.instant.elapsed() > timeout
    }
}

pub struct Proxy {
    config: Arc<Config>,
    ct: Sender<Message>,
    cr: Receiver<Message>,
    at: Sender<Accept>,
    ar: Receiver<Accept>,
}

impl Proxy {
    pub async fn run(config: Arc<Config>) -> io::Result<()> {
        let master = TcpListener::bind(&config.server_addr).await?;
        println!("│{:21?}│ MasterLister", config.server_addr);
        while let Ok(tcp) = master.accept().await {
            if let Err(err) = Self::new(tcp, config.clone()).await {
                eprintln!("{err}");
            }
        }
        Ok(())
    }

    pub async fn new(mut tcp: (TcpStream, SocketAddr), config: Arc<Config>) -> Result<Arc<Self>, String> {
        let mut buf = [0; 512];
        let n = tcp.0.read(&mut buf).await.map_err(|e| e.to_string())?;
        let mut msg = "Scret Error".to_string();

        if let Ok(Message::Master(master)) = Message::from_buf(&buf[..n])
            && master.secret == config.secret
        {
            match TcpListener::bind(master.access).await {
                Ok(accept) => {
                    let (worker, worker_addr) = random_listen(config.server_addr.ip()).await;
                    if let Err(err) = Message::Worker(worker_addr).send(&mut tcp.0).await {
                        return Err(format!("发送Worker地址失败: {err}"));
                    };
                    let (ct, cr) = unbounded();
                    let (at, ar) = unbounded();
                    let proxy = Arc::new(Self { ct, cr, at, ar, config });
                    println!("\n===========客户端加入连接===========");
                    println!("│{:21?}│ ProxyConnect", tcp.1);
                    println!("│{:21?}│ AcceptListen", master.access);
                    println!("│{:21?}│ WorkerListen", worker_addr);
                    proxy.run_master(tcp);
                    proxy.run_worker(worker);
                    proxy.run_accept(accept);
                    return Ok(proxy);
                }
                Err(err) => msg = format!("创建 Accept Listener 失败: {err}"),
            };
        }

        Message::Error(msg.clone()).send(&mut tcp.0).await.ok();
        Err(msg)
    }

    /// 建立主连接
    pub fn run_master(self: &Arc<Self>, (mut tcp, addr): (TcpStream, SocketAddr)) {
        let proxy = self.clone();
        let heartbeat = proxy.config.heartbeat;
        tokio::spawn(async move {
            let (mut reader, mut writer) = tcp.split();
            let mut buf = [0; 256];
            let err: Error;
            loop {
                tokio::select! {
                    Ok(msg) = proxy.cr.recv() => if let Err(e) = msg.send(&mut writer).await { break err = e },
                    Ok(n) = reader.read(&mut buf) => if n == 0 { break err = io::Error::other("EOF") },
                    _ = sleep(heartbeat) => if let Err(e) = Message::Ping.send(&mut writer).await { break err = e },
                    else => { break err = io::Error::other("其他原因") }
                }
            }
            proxy.close(); // 关闭所有通道
            println!("│{:21?}│ 连接断开 {err}", addr);
            println!("===========客户端连接断开===========");
        });
    }

    /// 监听用户访问
    pub fn run_accept(self: &Arc<Self>, accept: TcpListener) {
        let proxy = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                   Ok((tcp, addr)) = accept.accept() => if let (Err(_), _) | (_, Err(_)) = (
                       proxy.ct.send(Message::New).await,
                       proxy.at.send(Accept::new(tcp, addr)).await,
                   ) { break },
                   _ = proxy.at.closed() => break,
                   else => break,
                }
            }
        });
    }

    pub fn run_worker(self: &Arc<Self>, worker: TcpListener) {
        let proxy = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                   Ok((tcp, addr)) = worker.accept() => worker_swap_accept(proxy.clone(), tcp, addr),
                   _ = proxy.at.closed() => break,
                   else => break,
                }
            }
        });
    }

    pub fn close(self: &Arc<Self>) {
        self.ct.close();
        self.cr.close();
        self.at.close();
        self.ar.close();
    }
}

fn worker_swap_accept(proxy: Arc<Proxy>, tcp: TcpStream, addr: SocketAddr) {
    tokio::spawn(async move {
        loop {
            match proxy.ar.try_recv() {
                Ok(from) => match !from.is_timeout(proxy.config.timeout) {
                    true => return forward(from.tcp, tcp),
                    false => eprintln!("│{:21?}│ ⇨ Accept Wati Timeout", from.addr),
                },
                Err(_) => return eprintln!("│{:21?}│ ⇨ Worker Ready But No Accept", addr),
            }
        }
    });
}

async fn random_listen(ip: IpAddr) -> (TcpListener, SocketAddr) {
    loop {
        let port = rand::random_range(0xAAAA..0xFFFF);
        let addr = SocketAddr::new(ip, port);
        if let Ok(linten) = TcpListener::bind(addr).await {
            return (linten, addr);
        }
    }
}
