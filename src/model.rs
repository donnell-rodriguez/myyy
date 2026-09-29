use crate::history::ConversationItem;
use crate::tools::ToolSpec;
use std::future::Future;
use std::pin::Pin;
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
#[derive(Debug)]
pub enum ModelOutput {
    AssistantMessage { text: String },
    ToolCall { name: String, arguments: String },
}
// 不同模型实现产生的 Future 类型可能不同。
// Box 把它们统一成一种可以放进 trait object 的类型。
pub type ModelFuture<'a> = Pin<Box<dyn Future<Output = ModelOutput> + Send + 'a>>;
// 只要某个类型能够接收 ModelRequest 并产生 ModelOutput，它就可以作为模型客户端。
pub trait ModelClient: Send + Sync {
    fn respond<'a>(&'a self, request: ModelRequest) -> ModelFuture<'a>;
}
pub struct FakeModel;

impl FakeModel {
    pub fn new() -> Self {
        Self
    }
}
impl ModelClient for FakeModel {
    fn respond<'a>(&'a self, request: ModelRequest) -> ModelFuture<'a> {
        // 取出最后一个item
        Box::pin(async move {
            let Some(last_item) = request.input.last() else {
                return ModelOutput::AssistantMessage {
                    text: "当前没有可处理的消息".to_string(),
                };
            };

            // 判断最后一个item的情况，分别进行处理， 这里是模拟modeloutput的情况
            match last_item {
                // 慢工具
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
                //工具调用 并没有完成
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
            }
        })
    }
}
