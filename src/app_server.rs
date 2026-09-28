use crate::core::Session;
use crate::protocol::AppCommand;
use crate::protocol::CoreEvent;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
pub struct AppServerSession {
    // App 持有发送端；接收端由 App Server 后台任务独占。
    command_tx: mpsc::Sender<AppCommand>,
    event_rx: mpsc::UnboundedReceiver<CoreEvent>,
}

impl AppServerSession {
    pub async fn submit(
        &self,
        command: AppCommand,
    ) -> Result<(), mpsc::error::SendError<AppCommand>> {
        self.command_tx.send(command).await
    }

    pub async fn recv_event(&mut self) -> Option<CoreEvent> {
        self.event_rx.recv().await
    }
}

pub fn start() -> AppServerSession {
    // 使用有界队列，避免生产者无限积压尚未处理的命令。
    let (command_tx, mut command_rx) = mpsc::channel::<AppCommand>(32);
    let (event_tx, event_rx) = mpsc::unbounded_channel::<CoreEvent>();
    // App Server 在独立 Tokio 任务中持续接收并处理 AppCommand。
    // 当前 MVP 直接打印输入；下一阶段会把 `items` 转交给 Core Session。
    tokio::spawn(async move {
        let session = Session::new();
        let session_control = session.control();
        let session = Arc::new(Mutex::new(session));
        while let Some(command) = command_rx.recv().await {
            match command {
                AppCommand::UserTurn { items } => {
                    println!(
                        "[App Server] 收到 UserTurn，\
                         转交给 Core"
                    );
                    let cancellation_token = match session_control.reserve_turn().await {
                        Ok(cancellation_token) => cancellation_token,
                        Err(error) => {
                            println!("[App Server] 无法启动 UserTurn：{error}");
                            continue;
                        }
                    };

                    let session = Arc::clone(&session);
                    let event_tx = event_tx.clone();

                    //把所有权交给了start_turn
                    tokio::spawn(async move {
                        session
                            .lock()
                            .await
                            .start_turn(items, event_tx, cancellation_token)
                            .await;
                    });
                }
                AppCommand::Interrupt => {
                    println!(
                        "[App Server] 收到 Interrupt，\
                         转交给 Core"
                    );
                    session_control.interrupt().await;
                }
            }
        }
    });

    AppServerSession {
        command_tx,
        event_rx,
    }
}
