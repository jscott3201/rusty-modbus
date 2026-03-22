//! RTU-over-TCP loopback setup for benchmarks.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use modbus_frame::frame::{Frame, FrameHeader};
use modbus_frame::rtu_tcp::RtuOverTcpCodec;
use modbus_rtu::{RtuOverTcpTransport, RtuTcpRecvStream, RtuTcpSink};
use modbus_server::handler;
use modbus_server::store::DataStore;
use modbus_tcp::TcpConfig;
use modbus_types::UnitId;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::codec::Framed;

/// Start an RTU-over-TCP server with handler dispatch.
/// Returns join handle + bound address.
pub async fn make_rtu_tcp_server<S: DataStore + 'static>(
    store: Arc<S>,
) -> (JoinHandle<()>, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        while let Ok((tcp_stream, _)) = listener.accept().await {
            let conn_store = Arc::clone(&store);
            tokio::spawn(async move {
                let framed = Framed::new(tcp_stream, RtuOverTcpCodec);
                let (mut sink, mut stream) = framed.split();

                while let Some(Ok(frame)) = stream.next().await {
                    let unit_id = UnitId(frame.unit_id());
                    if let Some(resp_pdu) =
                        handler::process_request(&frame.pdu, unit_id, conn_store.as_ref(), &modbus_server::DeviceIdentification::default())
                            .await
                    {
                        let resp = Frame {
                            header: FrameHeader::Rtu {
                                unit_id: unit_id.0,
                            },
                            pdu: Bytes::from(resp_pdu),
                        };
                        if sink.send(resp).await.is_err() {
                            break;
                        }
                    }
                }
            });
        }
    });

    (handle, addr)
}

/// Connect an RTU-over-TCP client, returning raw split halves.
pub async fn make_rtu_tcp_client(addr: SocketAddr) -> (RtuTcpSink, RtuTcpRecvStream) {
    RtuOverTcpTransport::connect(addr, TcpConfig::default())
        .await
        .unwrap()
}
