use crate::app_server::AppServerSession;
use crate::protocol::AppCommand;
use crate::protocol::TuiEvent;
use crate::protocol::UserMessage;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct App {
    // 接收输入任务产生的 TuiEvent。
    event_rx: mpsc::UnboundedReceiver<TuiEvent>,
    // 向进程内 App Server 提交 AppCommand 的客户端句柄。
    app_server: AppServerSession,
}

impl App {
    /// 组装 App 需要的两个边界：TUI 事件接收端和 App Server 客户端。
    pub fn new(event_rx: mpsc::UnboundedReceiver<TuiEvent>, app_server: AppServerSession) -> Self {
        Self {
            event_rx,
            app_server,
        }
    }

    pub async fn run(&mut self) {
        // App 除了接收用户输入，也需要处理系统内部事件。
        // `tick()` 返回 Future，因此可以与 channel 接收一起放进 `select!`。
        let mut system_tick = tokio::time::interval(Duration::from_secs(3));
        system_tick.tick().await;

        loop {
            // 同时等待两类事件；哪个 Future 先就绪，就执行哪个分支。
            tokio::select! {
                // 从 TUI 事件通道接收下一条输入事件。
                maybe_event = self.event_rx.recv()=> {
                match maybe_event {
                    Some(TuiEvent::Submitted(text)) => {
                        println!("1. 收到 TUI 事件：{text:?}");

                        // 第一次变形：终端字符串 -> TUI 层整理后的 UserMessage。
                        let user_message = UserMessage::from(text);
                        println!(
                                "2. 构造 UserMessage：{user_message:?}"
                            );

                        // 第二次变形：UserMessage -> Agent 协议可以接收的输入项。
                        let items = user_message.into_user_inputs();
                        println!(
                                "3. 构造 UserInput 列表，共 {} 项",
                                items.len()
                            );

                        // 第三次变形：输入项 -> 发给 App Server 的一轮用户命令。
                        let command = AppCommand::user_turn(items);
                        println!(
                                "4. App 将 UserTurn 发送给 App Server"
                            );

                        // `submit().await` 只保证命令成功进入队列，
                        // 不代表 App Server 已经处理完这一轮。
                        if self.app_server.submit(command).await.is_err(){
                            println!(
                                    "[App] App Server 已关闭"
                                );
                            break;

                        }

                    }
                    None=>break,
                }
            }
            // 处理与用户输入无关的周期性系统事件。
            _ = system_tick.tick() =>{
                println!("【系统事件】定时刷新")
            }
                }
        }
    }
}
