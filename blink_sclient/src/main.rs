use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum BlinkMessage {
    Eval { id: String, code: String },
    Result { id: String, value: String },
    Error { id: String, message: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:5555").await?;

    let msg = BlinkMessage::Eval {
        id: "1".to_string(),
        code: "(+ 1 2)".to_string(),
    };

    let mut buf = Vec::new();
    msg.serialize(&mut Serializer::new(&mut buf))?;

    let len = (buf.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&buf).await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    stream.read_exact(&mut resp_buf).await?;

    let mut de = Deserializer::new(&resp_buf[..]);
    let response: BlinkMessage = BlinkMessage::deserialize(&mut de)?;

    println!("🔮 Response: {:?}", response);

    Ok(())
}
