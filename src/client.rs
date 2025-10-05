use std::{io, net::SocketAddr, sync::Arc};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::{sleep, timeout},
};

use crate::{Config, Message, forward};

pub struct Client {
    config: Arc<Config>,
}

impl Client {
    const NO_CLIENT: &'static [u8; 34] = b"HTTP/1.1 555 Client Connect Failed";

    pub fn new(config: Config) -> Self {
        Self { config: config.into() }
    }

    pub async fn run(&self) -> io::Result<()> {
        let config = self.config.clone();
        let master = match timeout(config.timeout, TcpStream::connect(config.server_addr)).await {
            Ok(v) => v?,
            Err(_) => return Ok(eprintln!("Master Conntec Timeout")),
        };

        let (reader, mut writer) = master.into_split();
        let msg = Message::Master(config.build_master());
        msg.send(&mut writer).await.ok();

        let heartbeat = config.heartbeat;
        let reader_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut addr: Arc<SocketAddr> = Arc::new("0.0.0.0:65535".parse().unwrap());
            let mut buf = Vec::with_capacity(1 << 10);

            while let Ok(1..) = reader.read_until(b'\n', &mut buf).await {
                match Message::from_buf(&buf) {
                    Err(err) => eprintln!("Serialization Failed:{err}"),
                    Ok(msg) => match msg {
                        Message::New => new_worker(config.clone(), addr.clone()).await,
                        Message::Msg(msg) => println!("{msg}"),
                        Message::Worker(a) => {
                            println!("│{:21?}│ AcceptAddress", config.accept_addr);
                            println!("│{:21?}│ MasterConnect", config.server_addr);
                            println!("│{:21?}│ WorkerConnect", a);
                            println!("│{:21?}│ ClientConnect", config.client_addr);
                            addr = Arc::new(a)
                        }
                        Message::Error(err) => return eprintln!("{err}"),
                        _ => {}
                    },
                };
                buf.truncate(0);
            }
            eprintln!("Master Connect Disconnected")
        });

        let writer_handle = tokio::spawn(async move {
            loop {
                sleep(heartbeat).await;
                if let Err(err) = Message::Pong.send(&mut writer).await {
                    return eprintln!("Master Connect Disconnected: {err}");
                }
            }
        });

        tokio::select! {
            _ = reader_handle => Ok(()),
            _ = writer_handle => Ok(()),
        }
    }
}

pub async fn new_worker(config: Arc<Config>, addr: Arc<SocketAddr>) {
    tokio::spawn(async move {
        let mut remote = match TcpStream::connect(*addr).await {
            Ok(v) => v,
            Err(err) => return eprintln!("Worker Connect Error: {err}"),
        };

        match TcpStream::connect(config.client_addr).await {
            Ok(local) => forward(remote, local),
            Err(err) => {
                remote.write_all(Client::NO_CLIENT).await.ok();
                remote.flush().await.ok();
                eprintln!("Client Connect Error: {err}")
            }
        }
    });
}
