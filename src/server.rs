use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use async_channel::{Receiver, Sender, unbounded};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    task::JoinHandle,
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

pub struct Server {
    config: Config,
    mt: Sender<Message>,
    mr: Receiver<Message>,
    at: Sender<Accept>,
    ar: Receiver<Accept>,
}

impl Server {
    pub const NO_MASTER: &'static [u8; 30] = b"HTTP/1.1 555 No Master Connect";

    pub fn new(config: Config) -> Arc<Self> {
        let (at, ar) = unbounded();
        let (mt, mr) = unbounded();
        Arc::new(Self { config, mt, mr, ar, at })
    }

    pub async fn run(self: &Arc<Self>) -> io::Result<()> {
        let master = TcpListener::bind(&self.config.server_addr).await?;
        let accept = TcpListener::bind(&self.config.accept_addr).await?;
        println!("│{:21?}│ AcceptListen", self.config.accept_addr);
        println!("│{:21?}│ MasterLister", self.config.server_addr);
        loop {
            tokio::select! {
                Ok(tcp) = master.accept() => self.master(tcp),
                Ok(tcp) = accept.accept() => self.accept(tcp),
                else => return Ok(()),
            }
        }
    }
    /// 处理连接
    pub fn master(self: &Arc<Self>, (mut tcp, _): (TcpStream, SocketAddr)) {
        let server = self.clone();
        tokio::spawn(async move {
            if server.mr.receiver_count() > 2 {
                let msg = Message::Error("Master already exists".to_string());
                msg.send(&mut tcp).await.ok();
                return;
            }

            let mut buf = [0; 512];
            let n = tcp.read(&mut buf).await.unwrap();

            match bitcode::decode::<Message>(&buf[..n].trim_ascii_end()) {
                Ok(Message::Master(secret)) if secret == server.config.secret => {
                    let (addr, handle) = new_worker(server.clone()).await.unwrap();
                    Message::Worker(addr).send(&mut tcp).await.ok();
                    let (reader, wirter) = tcp.into_split();

                    println!("│{:21?}│ WorkerListen", addr);
                    tokio::spawn(async move {
                        let err = tokio::select! {
                            Err(err) = reader_handle(server.mr.clone(),reader) => err,
                            Err(err) = writer_handle(server,wirter) => err
                        };
                        handle.abort();
                        eprintln!("│{:21?}│ ⇨ Master {} ", addr, err)
                    });
                }
                _ => {
                    let msg = Message::Error("Master Scret Format Error".to_string());
                    msg.send(&mut tcp).await.ok();
                }
            }
        });
    }

    /// 处理访问
    pub fn accept(self: &Arc<Self>, (mut tcp, addr): (TcpStream, SocketAddr)) {
        let server = self.clone();
        tokio::spawn(async move {
            match server.mr.receiver_count() {
                1..=2 => {
                    println!("│{:21?}│ ⇦ Accept No Master Connect", addr);
                    tcp.write_all(Self::NO_MASTER).await.ok();
                    tcp.flush().await.ok();
                }
                3 => {
                    server.mt.send(Message::New).await.unwrap();
                    server.at.send(Accept::new(tcp, addr)).await.unwrap();
                }
                _ => panic!("ACCEPT FRP INNER ERROR"),
            };
        });
    }
}

async fn writer_handle(server: Arc<Server>, mut writer: OwnedWriteHalf) -> io::Result<()> {
    let mr = server.mr.clone();
    loop {
        tokio::select! {
            Ok(msg) = mr.recv() =>msg.send(&mut writer).await?,
            _ = sleep(server.config.heartbeat) =>Message::Ping.send(&mut writer).await?
        }
    }
}

async fn reader_handle(_master_rx: Receiver<Message>, mut reader: OwnedReadHalf) -> io::Result<()> {
    let mut buf = [0; 256];
    while let Ok(true) = reader.read(&mut buf).await.map(|n| n > 1) {}
    Err(io::Error::other("Connect Disconnected"))
}

async fn new_worker(server: Arc<Server>) -> io::Result<(SocketAddr, JoinHandle<()>)> {
    for port in 0xAAAA..0xFFFF {
        let addr = SocketAddr::new(server.config.server_addr.ip(), port);
        if let Ok(worker) = TcpListener::bind(addr).await {
            let task = tokio::spawn(async move {
                while let Ok((tcp, addr)) = worker.accept().await {
                    let server = server.clone();
                    tokio::spawn(async move {
                        loop {
                            match server.ar.try_recv() {
                                Ok(from) => match !from.is_timeout(server.config.timeout) {
                                    true => return forward(from.tcp, tcp),
                                    false => eprintln!("│{:21?}│ ⇨ Accept Wati Timeout", from.addr),
                                },
                                Err(_) => return eprintln!("│{:21?}│ ⇨ Worker Ready But No Accept", addr),
                            }
                        }
                    });
                }
            });
            return Ok((addr, task));
        }
    }
    Err(io::Error::other("Listen Worker Faild"))
}
