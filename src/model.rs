use crate::history::ConversationItem;

#[derive(Debug)]
pub enum ModelOutput {
    AssistantMessage { text: String },
    ToolCall { name: String, arguments: String },
}

pub struct FakeModel;

impl FakeModel {
    pub fn new() -> Self {
        Self
    }
    // pub async fn respond(&self, input:&str)->ModelOutput {
    //     if input.contains("pwd"){
    //         ModelOutput::ToolCall { name: "exec_command".to_string(), arguments: r#"{"cmd":"pwd"}"#.to_string() }
    //     } else if input.contains("ls") {
    //         ModelOutput::ToolCall { name: "exec_command".to_string(), arguments: r#"{"cmd":"ls"}"#.to_string() }
    //     }else {
    //         ModelOutput::AssistantMessage { text: format!("我收到了你的消息：{input}") }
    //     }
    // }
    pub async fn respond(&self, history: &[ConversationItem]) -> ModelOutput {
        // 取出最后一个item
        let Some(last_item) = history.last() else {
            return ModelOutput::AssistantMessage {
                text: "当前没有可处理的消息".to_string(),
            };
        };
        // 判断最后一个item的情况，分别进行处理， 这里是模拟modeloutput的情况
        match last_item {
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
        }
    }
}
