//! TLS client example — mutual authentication over Modbus/TCP Security.
//!
//! Run: `cargo run -p rusty-modbus --features full --example tls_client -- <host:port> <ca.pem> <client.pem> <client-key.pem>`

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus::frame::Frame;
use rusty_modbus::frame::frame::FrameHeader;
use rusty_modbus::tcp::TransportSink;
use rusty_modbus::tcp::TransportStream;
use rusty_modbus::tls::{TlsClientConfig, TlsTransport};
use rusty_modbus::types::MbapHeader;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 5 {
        eprintln!("Usage: {} <host:port> <ca.pem> <client.pem> <client-key.pem>", args[0]);
        std::process::exit(1);
    }

    let addr = args[1].parse()?;
    let config = TlsClientConfig {
        ca_cert: PathBuf::from(&args[2]),
        client_cert: PathBuf::from(&args[3]),
        client_key: PathBuf::from(&args[4]),
        connect_timeout: Duration::from_secs(5),
        read_timeout: Some(Duration::from_secs(5)),
        write_timeout: Some(Duration::from_secs(5)),
    };

    let (mut sink, mut stream) = TlsTransport::connect(addr, &config).await?;
    println!("TLS 1.3 connection established to {addr}");

    // ReadHoldingRegisters: FC 0x03, addr=0, qty=5.
    let pdu = Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x05]);
    let header = MbapHeader::new(1, 0xFF, 5);
    let frame = Frame {
        header: FrameHeader::Mbap(header),
        pdu,
    };
    sink.send(frame).await?;

    let response = stream.recv().await?;
    println!("Response: {} bytes", response.pdu.len());

    Ok(())
}
