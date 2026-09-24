use crate::protocol::TuiEvent;
use std::io::Write;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;

/// 启动终端输入任务：读取整行文本，将其包装成 `TuiEvent` 后发送给 App。
pub fn spawn_input_task(event_tx: mpsc::UnboundedSender<TuiEvent>) {
    // 输入等待不会阻塞 App 的主事件循环，因为它运行在独立 Tokio 任务中。
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut lines = tokio::io::BufReader::new(stdin).lines();
        loop {
            print!(">");
            let _ = std::io::stdout().flush();
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) | Err(_) => break,
            };
            let event = TuiEvent::Submitted(line);

            // 如果 App 已退出，接收端会被释放；此时输入任务也应该结束。
            if event_tx.send(event).is_err() {
                break;
            }
        }
    });
}
