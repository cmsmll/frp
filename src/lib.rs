use std::{fs, net::SocketAddr, time::Duration};

use bitcode::{Decode, Encode};
use serde::{Deserialize, Deserializer, Serialize};
use tokio::{
    io::{self, AsyncWriteExt},
    net::TcpStream,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub secret: String,
    #[serde(deserialize_with = "deserialize_duration")]
    pub timeout: Duration,
    #[serde(deserialize_with = "deserialize_duration")]
    pub heartbeat: Duration,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub accept_addr: SocketAddr,
}

fn deserialize_duration<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    let n: u64 = Deserialize::deserialize(deserializer)?;
    Ok(Duration::from_millis(n))
}

impl Config {
    pub fn load(path: &str) -> io::Result<Self> {
        let data = fs::read_to_string(path)?;
        toml::from_str(&data).map_err(io::Error::other)
    }
}

#[derive(Debug, Encode, Decode)]
pub enum Message {
    New,
    Ping,
    Pong,
    Msg(String),
    Error(String),
    Master(String),
    Worker(SocketAddr),
}

impl Message {
    pub fn from_buf(buf: &[u8]) -> io::Result<Self> {
        bitcode::decode(buf).map_err(|err| io::Error::other(format!("反序列化失败: {err}")))
    }

    pub fn to_vec(&self) -> Vec<u8> {
        let mut vec = bitcode::encode(self);
        vec.push(b'\n');
        vec
    }

    pub async fn send<T>(&self, writer: &mut T) -> io::Result<()>
    where
        T: AsyncWriteExt + Unpin,
    {
        let src = self.to_vec();
        writer.write_all(&src).await?;
        writer.flush().await
    }
}
