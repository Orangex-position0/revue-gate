//! 数据面：HTTP 服务生命周期管理（启动/停止 + 优雅停机）。
//!
//! `ServerManager` 与 Tauri 解耦：命令层负责调用并把状态变化广播为事件（见 commands）。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::oneshot;

use super::router::build_router;

/// 服务生命周期错误。
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("server is already running")]
    AlreadyRunning,
    #[error("server is not running")]
    NotRunning,
    #[error("failed to bind http listener: {0}")]
    Bind(#[source] std::io::Error),
}

/// 运行中的服务句柄：地址 + 停机信号 + 服务任务。
struct RunningServer {
    addr: SocketAddr,
    shutdown_tx: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

/// HTTP 服务管理器：跟踪运行状态，支持启动/停止与优雅停机。
#[derive(Default)]
pub struct ServerManager {
    // Arc 共享：服务任务退出时也需要清空登记（见 start 内 take），不能只由方法侧访问。
    inner: Arc<Mutex<Option<RunningServer>>>,
}

impl ServerManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 在 `host:port` 上启动服务，返回实际监听地址（port 0 = 随机端口）。
    ///
    /// 先异步绑定监听器（持锁期间无 await，遵守 tokio 协作式调度约束），
    /// 再进入短临界区登记运行句柄；已运行时返回 `AlreadyRunning`。
    pub async fn start(&self, host: &str, port: u16) -> Result<SocketAddr, ServerError> {
        let listener = tokio::net::TcpListener::bind((host, port))
            .await
            .map_err(ServerError::Bind)?;
        let addr = listener.local_addr().map_err(ServerError::Bind)?;

        let mut guard = self.lock_inner();
        if guard.is_some() {
            return Err(ServerError::AlreadyRunning);
        }
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        // 服务任务退出时（正常停机或 serve 出错）清空登记，保证 is_running()/addr() 与实际一致，
        // 也让 serve 自退出后能再次 start。
        let inner = Arc::clone(&self.inner);
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, build_router())
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

    /// 停止服务：触发优雅停机并等待服务任务退出，返回后端口应已关闭。
    pub async fn stop(&self) -> Result<(), ServerError> {
        let server = self.lock_inner().take().ok_or(ServerError::NotRunning)?;
        let _ = server.shutdown_tx.send(());
        let _ = server.task.await;
        Ok(())
    }

    /// 服务是否在运行。
    pub fn is_running(&self) -> bool {
        self.lock_inner().is_some()
    }

    /// 当前监听地址（未运行时为 None）。
    pub fn addr(&self) -> Option<SocketAddr> {
        self.lock_inner().as_ref().map(|s| s.addr)
    }

    fn lock_inner(&self) -> MutexGuard<'_, Option<RunningServer>> {
        // 中毒时恢复守卫（测试/命令侧异常不应让管理器永久不可用）
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::Duration;

    use super::*;

    /// 生命周期：start 后端口可连接，stop（优雅停机完成）后端口拒绝连接。
    #[tokio::test]
    async fn server_starts_accepts_connections_then_stop_closes_port() {
        let manager = ServerManager::new();
        let addr = manager.start("127.0.0.1", 0).await.expect("start");

        // 启动后应能建立 TCP 连接（listener 已绑定）
        let _stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("connect while running");
        assert!(manager.is_running());

        manager.stop().await.expect("stop");

        // 停止并完成优雅停机后，端口应拒绝新连接
        let result =
            tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(addr))
                .await;
        let closed = match result {
            Err(_) => true,     // 超时（无监听）视为已关闭
            Ok(Err(_)) => true, // ECONNREFUSED
            Ok(Ok(_)) => false, // 仍能连接 —— 失败
        };
        assert!(closed, "port {addr} should refuse connections after stop");
        assert!(!manager.is_running());
    }

    /// 空闲时 stop 应报错（幂等保护），而不是静默吞掉。
    #[tokio::test]
    async fn stop_when_not_running_errors() {
        let manager = ServerManager::new();
        let err = manager.stop().await.expect_err("stop on idle should error");
        assert!(matches!(err, ServerError::NotRunning));
    }

    /// 重复 start 应报错，已运行的实例保持不变。
    #[tokio::test]
    async fn double_start_errors() {
        let manager = ServerManager::new();
        let _first = manager.start("127.0.0.1", 0).await.expect("first start");
        let err = manager
            .start("127.0.0.1", 0)
            .await
            .expect_err("second start should error");
        assert!(matches!(err, ServerError::AlreadyRunning));
        manager.stop().await.expect("cleanup stop");
    }

    /// 停机后登记被清空，可再次 start（服务任务退出后管理器不残留死状态）。
    #[tokio::test]
    async fn restart_after_stop() {
        let manager = ServerManager::new();
        let first = manager.start("127.0.0.1", 0).await.expect("first start");
        manager.stop().await.expect("stop");
        assert!(!manager.is_running());

        let second = manager.start("127.0.0.1", 0).await.expect("restart");
        assert_ne!(first, second, "restart should bind a new addr");
        manager.stop().await.expect("cleanup stop");
    }
}
