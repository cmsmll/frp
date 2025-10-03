use std::{io, net::SocketAddr, sync::Arc};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::{sleep, timeout},
};

use crate::{Config, Message, forward};

pub struct Client {
    config: Config,
}

impl Client {
    const NO_CLIENT: &'static [u8; 34] = b"HTTP/1.1 555 Client Connect Failed";

    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Client { config })
    }

    pub async fn run(self: &Arc<Self>) -> io::Result<()> {
        let master = match timeout(self.config.timeout, TcpStream::connect(&self.config.server_addr)).await {
            Ok(v) => v?,
            Err(_) => return Ok(eprintln!("Master Conntec Timeout")),
        };

        println!("│{:21?}│ ClientConnect", self.config.client_addr);
        println!("│{:21?}│ MasterConnect", self.config.server_addr);
        let (reader, mut writer) = master.into_split();
        let secret = Message::Master(self.config.secret.clone());
        secret.send(&mut writer).await.ok();

        let client = self.clone();
        let heartbeat = client.config.heartbeat;
        let reader_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut addr: Arc<SocketAddr> = Arc::new("0.0.0.0:65535".parse().unwrap());
            let mut buf = Vec::with_capacity(1 << 10);
            while let Ok(true) = reader.read_until(b'\n', &mut buf).await.map(|n| n > 1) {
                let buf2 = &buf[0..buf.len() - 1];
                match Message::from_buf(buf2) {
                    Err(err) => eprintln!("Serialization Failed:{err} Content:{}", String::from_utf8_lossy(&buf)),
                    Ok(msg) => match msg {
                        Message::New => new_worker(client.clone(), addr.clone()).await,
                        Message::Msg(msg) => println!("{msg}"),
                        Message::Worker(a) => {
                            println!("│{:21?}│ WorkerConnect", a);
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
            _ = reader_handle => {},
            _ = writer_handle => {},
        }

        Ok(())
    }
}

pub async fn new_worker(client: Arc<Client>, addr: Arc<SocketAddr>) {
    tokio::spawn(async move {
        let mut remote = match TcpStream::connect(*addr).await {
            Ok(v) => v,
            Err(err) => return eprintln!("Worker Connect Error: {err}"),
        };

        match TcpStream::connect(client.config.client_addr).await {
            Ok(local) => forward(remote, local),
            Err(err) => {
                remote.write_all(Client::NO_CLIENT).await.ok();
                remote.flush().await.ok();
                eprintln!("Client Connect Error: {err}")
            }
        }
    });
}
