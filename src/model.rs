use crate::history::ConversationItem;
use crate::tools::ToolSpec;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const MODEL_STREAM_CHANNEL_CAPACITY: usize = 1;
// 某一次模型调用看到的完整输入快照。
// 每次请求模型前，把当前完整 ConversationHistory 复制成一个独立的 ModelRequest，而不是让模型直接借用 Session 内部历史。
// 让 ModelRequest 除了携带对话历史，还携带当前模型可以调用的工具定义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRequest {
    pub input: Vec<ConversationItem>,
    pub tools: Vec<ToolSpec>,
}

impl ModelRequest {
    pub fn new(history: &[ConversationItem], tools: Vec<ToolSpec>) -> Self {
        Self {
            input: history.to_vec(),
            tools,
        }
    }
}
// 模型可能返回普通消息，也可能要求调用工具。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelOutput {
    AssistantMessage { text: String },
    ToolCall { name: String, arguments: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelEvent {
    //模型刚生成的一小段文本。
    OutputTextDelta { delta: String },
    // 一个输出项完成，但整个响应还没有结束。
    OutputItemDone { output: ModelOutput },
    // 整个模型响应完成。
    ResponseCompleted,
}

// 程序：保存模型失败原因。
// Rust：实现 Display 后可以调用 .to_string()。
// Agent：模型失败成为正式状态，而不是一条普通 Assistant 消息。

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    RequestFailed(String),
    StreamFailed(String),
    StreamClosed(String),
    InvalidResponse(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            //请求根本没有建立成功
            Self::RequestFailed(message) => {
                write!(f, "模型请求失败：{message}")
            }
            //请求建立成功，也收到了一部分内容，但中途失败
            Self::StreamFailed(message) => {
                write!(f, "模型流失败：{message}")
            }
            // channel 已关闭 通道关闭，却没有收到完成事件
            // 但是 Core 还没有收到 OutputItemDone
            Self::StreamClosed(message) => {
                write!(f, "模型事件流在完成前关闭：{message}")
            }
            Self::InvalidResponse(message) => {
                write!(f, "模型响应协议错误：{message}")
            }
        }
    }
}
impl std::error::Error for ModelError {}

// 持续接收模型生成事件，并在消费者离开时通知后台生产者。
pub struct ModelStream {
    // 有界通道让模型生产速度受到 Core 消费速度约束。
    event_rx: mpsc::Receiver<Result<ModelEvent, ModelError>>,
    consumer_dropped: CancellationToken,
}

impl ModelStream {
    pub async fn recv(&mut self) -> Option<Result<ModelEvent, ModelError>> {
        self.event_rx.recv().await
    }
}
impl Drop for ModelStream {
    fn drop(&mut self) {
        self.consumer_dropped.cancel();
    }
}

// 等待模型请求建立
pub type ModelStreamFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ModelStream, ModelError>> + Send + 'a>>;

// 长期模型客户端。
// 它负责为每个 Turn 创建独立的模型会话。
pub trait ModelClient: Send + Sync {
    fn new_session(&self) -> Box<dyn ModelSession>;
}
// 单个 Turn 内的模型会话。
// 同一个 Turn 中的多次模型请求复用它。
pub trait ModelSession: Send {
    fn stream<'a>(&'a mut self, request: ModelRequest) -> ModelStreamFuture<'a>;
}

// 在独立 Tokio 任务中异步生产模型事件，同时监听消费者取消信号。
fn spawn_fake_stream(events: Vec<Result<ModelEvent, ModelError>>) -> ModelStream {
    let (event_tx, event_rx) = mpsc::channel(MODEL_STREAM_CHANNEL_CAPACITY);
    let consumer_dropped = CancellationToken::new();
    let producer_cancellation = consumer_dropped.clone();
    tokio::spawn(async move {
        for event in events {
            tokio::select! {
                biased;
                // 取消
                _ = producer_cancellation.cancelled() => {
                    return;
                }
                // channel 满时在这里等待，直到 Core 消费一个事件。
                send_result = event_tx.send(event)=>{
                if send_result.is_err(){
                    return;
                }
            }
            }
        }
    });
    ModelStream {
        event_rx,
        consumer_dropped,
    }
}

// 成功的部分
pub(crate) fn fake_stream_from_output(output: ModelOutput) -> ModelStream {
    let mut events = Vec::new();
    // 现在成功事件顺序是：
    // OutputTextDelta*
    // OutputItemDone
    // ResponseCompleted
    if let ModelOutput::AssistantMessage { text } = &output {
        for character in text.chars() {
            events.push(Ok(ModelEvent::OutputTextDelta {
                delta: character.to_string(),
            }));
        }
    }
    events.push(Ok(ModelEvent::OutputItemDone { output }));
    events.push(Ok(ModelEvent::ResponseCompleted));
    spawn_fake_stream(events)
}

pub(crate) fn fake_stream_that_fails() -> ModelStream {
    spawn_fake_stream(vec![
        // 部分回答
        Ok(ModelEvent::OutputTextDelta {
            delta: "部分回答".to_string(),
        }),
        // 失败的部分
        Err(ModelError::StreamFailed("模拟连接中断".to_string())),
    ])
}
pub(crate) fn fake_stream_that_fails_after_item() -> ModelStream {
    spawn_fake_stream(vec![
        Ok(ModelEvent::OutputItemDone {
            output: ModelOutput::AssistantMessage {
                text: "不能提交到历史".to_string(),
            },
        }),
        Err(ModelError::StreamFailed("响应完成前连接中断".to_string())),
    ])
}

pub struct FakeModel;

// 这是单个 Turn 的模型状态。
struct FakeModelSession {
    request_count: usize,
}

impl FakeModel {
    pub fn new() -> Self {
        Self
    }
}

impl ModelClient for FakeModel {
    fn new_session(&self) -> Box<dyn ModelSession> {
        Box::new(FakeModelSession { request_count: 0 })
    }
}

impl ModelSession for FakeModelSession {
    fn stream<'a>(&'a mut self, request: ModelRequest) -> ModelStreamFuture<'a> {
        self.request_count += 1;
        let request_count = self.request_count;

        // 取出最后一个item
        Box::pin(async move {
            println!("[FakeModelSession] 本 Turn 第 {request_count} 次模型请求");

            let Some(last_item) = request.input.last() else {
                return Ok(fake_stream_from_output(ModelOutput::AssistantMessage {
                    text: "当前没有可处理的消息".to_string(),
                }));
            };

            if matches!(last_item, ConversationItem::UserMessage { text } if text.contains("模型失败"))
            {
                return Err(ModelError::RequestFailed("模拟服务不可用".to_string()));
            }
            if matches!(last_item, ConversationItem::UserMessage { text } if text.contains("流中失败"))
            {
                return Ok(fake_stream_that_fails());
            }
            if matches!(last_item, ConversationItem::UserMessage { text } if text.contains("输出项后失败"))
            {
                return Ok(fake_stream_that_fails_after_item());
            }

            // 判断最后一个item的情况，分别进行处理， 这里是模拟modeloutput的情况
            let output = match last_item {
                // wait工具
                ConversationItem::UserMessage { text } if text.contains("等待") => {
                    ModelOutput::ToolCall {
                        name: "exec_command".to_string(),
                        arguments: r#"{"cmd":"wait"}"#.to_string(),
                    }
                }
                // pwd 的命令
                ConversationItem::UserMessage { text } if text.contains("pwd") => {
                    ModelOutput::ToolCall {
                        name: "exec_command".to_string(),
                        arguments: r#"{"cmd":"pwd"}"#.to_string(),
                    }
                }
                // ls的命令
                ConversationItem::UserMessage { text } if text.contains("ls") => {
                    ModelOutput::ToolCall {
                        name: "exec_command".to_string(),
                        arguments: r#"{"cmd":"ls"}"#.to_string(),
                    }
                }
                // 用户的信息
                ConversationItem::UserMessage { text } => ModelOutput::AssistantMessage {
                    text: format!("我收到了你的消息：：{text}"),
                },
                // 工具执行结果
                ConversationItem::ToolResult {
                    name,
                    output,
                    success,
                } => {
                    let text = if *success {
                        format!("工具{name} 执行成功 结果是{output}")
                    } else {
                        format!("工具{name}执行失败，原因是{output}")
                    };
                    ModelOutput::AssistantMessage { text }
                }
                //工具调用没有完成
                ConversationItem::ToolCall { name, arguments } => ModelOutput::AssistantMessage {
                    text: format!("工具{name}尚未返回结果， 参数是{arguments}"),
                },
                // 模型描述
                ConversationItem::AssistantMessage { text } => {
                    ModelOutput::AssistantMessage { text: text.clone() }
                }
                // 中止
                ConversationItem::TurnAborted(marker) => ModelOutput::AssistantMessage {
                    text: format!("检测到历史中止记录：{}", marker.guidance),
                },
            };
            Ok(fake_stream_from_output(output))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ModelError, ModelEvent, ModelOutput, fake_stream_from_output, fake_stream_that_fails,
    };

    #[tokio::test]
    async fn stream_can_fail_after_partial_delta() {
        let mut stream = fake_stream_that_fails();

        assert_eq!(
            stream.recv().await,
            Some(Ok(ModelEvent::OutputTextDelta {
                delta: "部分回答".to_string(),
            }))
        );

        assert_eq!(
            stream.recv().await,
            Some(Err(ModelError::StreamFailed("模拟连接中断".to_string(),)))
        );

        assert_eq!(stream.recv().await, None);
    }

    #[tokio::test]
    async fn text_deltas_arrive_before_output_done() {
        let output = ModelOutput::AssistantMessage {
            text: "好呀".to_string(),
        };

        let mut stream = fake_stream_from_output(output.clone());

        assert_eq!(
            stream.recv().await,
            Some(Ok(ModelEvent::OutputTextDelta {
                delta: "好".to_string(),
            },))
        );

        assert_eq!(
            stream.recv().await,
            Some(Ok(ModelEvent::OutputTextDelta {
                delta: "呀".to_string(),
            },))
        );

        assert_eq!(
            stream.recv().await,
            Some(Ok(ModelEvent::OutputItemDone { output },))
        );

        assert_eq!(stream.recv().await, Some(Ok(ModelEvent::ResponseCompleted)));

        assert_eq!(stream.recv().await, None);
    }

    #[tokio::test]
    async fn dropping_stream_notifies_background_producer() {
        let stream = fake_stream_from_output(ModelOutput::AssistantMessage {
            text: "不会被继续消费".to_string(),
        });

        let consumer_dropped = stream.consumer_dropped.clone();

        assert!(!consumer_dropped.is_cancelled());

        drop(stream);

        assert!(consumer_dropped.is_cancelled());
    }

    #[tokio::test]
    async fn bounded_stream_keeps_only_one_unread_event() {
        let output = ModelOutput::AssistantMessage {
            text: "好呀".to_string(),
        };

        let mut stream = fake_stream_from_output(output);

        // 让后台生产任务得到一次运行机会。
        tokio::task::yield_now().await;

        // 总容量是 1。
        assert_eq!(stream.event_rx.max_capacity(), 1);

        // 此时只能积压一个未读取事件。
        assert_eq!(stream.event_rx.len(), 1);

        // 剩余容量为 0，生产者不能继续发送。
        assert_eq!(stream.event_rx.capacity(), 0);

        assert!(matches!(
            stream.recv().await,
            Some(Ok(ModelEvent::OutputTextDelta {
                delta
            })) if delta == "好"
        ));

        // Core 消费第一个事件后，生产者才能发送下一个。
        tokio::task::yield_now().await;

        assert_eq!(stream.event_rx.len(), 1);
    }
}
