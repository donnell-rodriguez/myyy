use crate::core::Session;
use crate::protocol::AppCommand;
use tokio::sync::mpsc;
pub struct AppServerSession {
    // App 持有发送端；接收端由 App Server 后台任务独占。
    command_tx: mpsc::Sender<AppCommand>,
}

impl AppServerSession {
    pub async fn submit(
        &self,
        command: AppCommand,
    ) -> Result<(), mpsc::error::SendError<AppCommand>> {
        self.command_tx.send(command).await
    }
}

pub fn start() -> AppServerSession {
    // 使用有界队列，避免生产者无限积压尚未处理的命令。
    let (command_tx, mut command_rx) = mpsc::channel::<AppCommand>(32);

    // App Server 在独立 Tokio 任务中持续接收并处理 AppCommand。
    // 当前 MVP 直接打印输入；下一阶段会把 `items` 转交给 Core Session。
    tokio::spawn(async move {
        let mut session = Session::new();
        while let Some(command) = command_rx.recv().await {
            match command {
                AppCommand::UserTurn { items } => {
                    // println!("[App Server] 收到 UserTurn，共 {} 个输入项", items.len());
                    // for item in items {
                    //     match item {
                    //         UserInput::Text { text } => {
                    //             println!("[App Server] Text：{text:?}");
                    //         }
                    //     }
                    // }
                    println!(
                        "[App Server] 收到 UserTurn，\
                         转交给 Core"
                    );

                    //把所有权交给了start_turn
                    session.start_turn(items).await;
                }
            }
        }
    });
    AppServerSession { command_tx }
}
