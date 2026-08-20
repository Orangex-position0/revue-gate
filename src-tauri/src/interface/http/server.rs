//! Data plane: HTTP server lifecycle management (start/stop + graceful shutdown).
//!
//! `ServerManager` is decoupled from Tauri and routing: the route tree is built by the caller and
//! passed in (data-plane assembly lives in lib.rs / the command layer), and the server layer only
//! handles listening, serving, and graceful shutdown.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use tokio::sync::oneshot;

/// Server lifecycle error.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("server is already running")]
    AlreadyRunning,
    #[error("server is not running")]
    NotRunning,
    #[error("failed to bind http listener: {0}")]
    Bind(#[source] std::io::Error),
}

/// Handle to a running server: address + shutdown signal + server task.
struct RunningServer {
    addr: SocketAddr,
    /// Shutdown signal
    shutdown_tx: oneshot::Sender<()>,
    // The spawned server task
    task: tokio::task::JoinHandle<()>,
}

/// HTTP server manager: tracks runtime state and supports start/stop with graceful shutdown.
#[derive(Default)]
pub struct ServerManager {
    // Arc-shared: the server task also needs to clear the registration on exit (see the take in start), so it cannot be owned by the method side alone.
    inner: Arc<Mutex<Option<RunningServer>>>,
}

impl ServerManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start `router` on `host:port`, returning the actual listen address (port 0 = random port).
    ///
    /// Binds the listener asynchronously first (no await while holding the lock, honoring tokio
    /// cooperative scheduling), then enters a short critical section to register the running handle;
    /// returns `AlreadyRunning` if already running.
    pub async fn start(
        &self,
        host: &str,
        port: u16,
        router: Router,
    ) -> Result<SocketAddr, ServerError> {
        let listener = tokio::net::TcpListener::bind((host, port))
            .await
            .map_err(ServerError::Bind)?;
        let addr = listener.local_addr().map_err(ServerError::Bind)?;

        let mut guard = self.lock_inner();
        if guard.is_some() {
            return Err(ServerError::AlreadyRunning);
        }
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        // Clear the registration when the server task exits (normal shutdown or a serve error), keeping
        // is_running()/addr() consistent with reality and allowing another start after serve exits on its own.
        let inner = Arc::clone(&self.inner);
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
            if let Err(err) = result {
                tracing::error!(error = %err, "http server stopped with error");
            }
            inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
        });
        *guard = Some(RunningServer {
            addr,
            shutdown_tx,
            task,
        });
        Ok(addr)
    }

    /// Stop the server: trigger graceful shutdown and wait for the server task to exit; the port should be closed on return.
    pub async fn stop(&self) -> Result<(), ServerError> {
        let server = self.lock_inner().take().ok_or(ServerError::NotRunning)?;
        let _ = server.shutdown_tx.send(());
        let _ = server.task.await;
        Ok(())
    }

    /// Whether the server is running.
    pub fn is_running(&self) -> bool {
        self.lock_inner().is_some()
    }

    /// Current listen address (None when not running).
    pub fn addr(&self) -> Option<SocketAddr> {
        self.lock_inner().as_ref().map(|s| s.addr)
    }

    /// Acquire the inner lock, recovering from poisoning instead of panicking.
    fn lock_inner(&self) -> MutexGuard<'_, Option<RunningServer>> {
        // Recover the guard when poisoned (a test/command-side panic should not leave the manager permanently unusable)
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::Duration;

    use super::*;

    /// Lifecycle: after start the port accepts connections; after stop (graceful shutdown complete) it refuses them.
    #[tokio::test]
    async fn server_starts_accepts_connections_then_stop_closes_port() {
        let manager = ServerManager::new();
        let addr = manager
            .start("127.0.0.1", 0, Router::new())
            .await
            .expect("start");

        // After start, a TCP connection should be establishable (listener is bound)
        let _stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("connect while running");
        assert!(manager.is_running());

        manager.stop().await.expect("stop");

        // After stop and graceful shutdown complete, the port should refuse new connections
        let result =
            tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(addr))
                .await;
        let closed = match result {
            Err(_) => true,     // timeout (no listener) treated as closed
            Ok(Err(_)) => true, // ECONNREFUSED
            Ok(Ok(_)) => false, // still connectable -- failure
        };
        assert!(closed, "port {addr} should refuse connections after stop");
        assert!(!manager.is_running());
    }

    /// stop on an idle server should error (idempotency guard), not be silently swallowed.
    #[tokio::test]
    async fn stop_when_not_running_errors() {
        let manager = ServerManager::new();
        let err = manager.stop().await.expect_err("stop on idle should error");
        assert!(matches!(err, ServerError::NotRunning));
    }

    /// A second start should error, leaving the running instance unchanged.
    #[tokio::test]
    async fn double_start_errors() {
        let manager = ServerManager::new();
        let _first = manager
            .start("127.0.0.1", 0, Router::new())
            .await
            .expect("first start");
        let err = manager
            .start("127.0.0.1", 0, Router::new())
            .await
            .expect_err("second start should error");
        assert!(matches!(err, ServerError::AlreadyRunning));
        manager.stop().await.expect("cleanup stop");
    }

    /// After stop the registration is cleared and start works again (no dead state lingers after the server task exits).
    #[tokio::test]
    async fn restart_after_stop() {
        let manager = ServerManager::new();
        let first = manager
            .start("127.0.0.1", 0, Router::new())
            .await
            .expect("first start");
        manager.stop().await.expect("stop");
        assert!(!manager.is_running());

        let second = manager
            .start("127.0.0.1", 0, Router::new())
            .await
            .expect("restart");
        assert_ne!(first, second, "restart should bind a new addr");
        manager.stop().await.expect("cleanup stop");
    }
}
