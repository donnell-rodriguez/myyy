mod app;
mod app_server;
mod core;
mod history;
mod model;
mod protocol;
mod tools;
mod tui;
use crate::app::App;

#[tokio::main]
async fn main() {
    println!("请输入一条信息：");

    // 创建 TUI -> App 的事件通道：输入任务持有发送端，App 持有接收端。
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

    // 启动进程内 App Server，并取得用于提交 AppCommand 的客户端句柄。
    let app_server = app_server::start();

    // 启动后台输入任务：它读取终端，并把 TuiEvent 发送给 App。
    tui::spawn_input_task(event_tx);

    // App 负责事件循环，以及 TuiEvent -> AppCommand 的数据转换。
    let mut app = App::new(event_rx, app_server);
    app.run().await;
}
