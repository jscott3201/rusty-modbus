//! Retiring adapters for handing a raw pooled TCP lease to the client engine.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_frame::Frame;
use rusty_modbus_tcp::{
    TcpRecvStream, TcpSink, TransportError,
    transport::{TransportSink, TransportStream},
};
use tokio::sync::Notify;

use crate::pool::{PoolEntry, PoolInner};

/// Releases one active pool charge when both transport adapters are gone.
struct RetirementGuard {
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    addr: SocketAddr,
    is_priority: bool,
}

impl Drop for RetirementGuard {
    fn drop(&mut self) {
        {
            let mut inner = self.pool.lock();
            inner.release_active(self.is_priority, self.addr);
        }
        self.capacity_changed.notify_waiters();
    }
}

/// Client-owned write half that keeps the pool charge until it is dropped.
pub(crate) struct RetiringSink {
    // Field order is significant: retire the transport before releasing the
    // guard's final shared reference.
    inner: TcpSink,
    _retirement: Arc<RetirementGuard>,
}

impl TransportSink for RetiringSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.inner.send(frame).await
    }
}

/// Client-owned read half that keeps the pool charge until it is dropped.
struct RetiringStream {
    // See `RetiringSink`: the TCP half must be dropped before the shared guard.
    inner: TcpRecvStream,
    _retirement: Arc<RetirementGuard>,
}

impl TransportStream for RetiringStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.inner.recv().await
    }
}

pub(crate) fn into_client(
    entry: PoolEntry,
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    config: ClientConfig,
) -> ModbusClient<RetiringSink> {
    let retirement = Arc::new(RetirementGuard {
        pool,
        capacity_changed,
        addr: entry.addr,
        is_priority: entry.is_priority,
    });
    let PoolEntry { sink, stream, .. } = entry;

    let sink = RetiringSink {
        inner: sink,
        _retirement: Arc::clone(&retirement),
    };
    let stream = RetiringStream {
        inner: stream,
        _retirement: retirement,
    };

    ModbusClient::from_transport(sink, stream, config)
}
