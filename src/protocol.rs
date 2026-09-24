/// 从终端输入层发送给 App 的事件。
#[derive(Debug)]
pub enum TuiEvent {
    Submitted(String),
}

/// TUI 层整理完成的一条用户消息。
///
/// 它把“终端读取到的字符串”提升为具有明确业务含义的用户消息。
#[derive(Debug)]
pub struct UserMessage {
    pub text: String,
}

impl From<String> for UserMessage {
    fn from(text: String) -> Self {
        Self { text }
    }
}
/// Agent 协议能够接受的统一输入项。
///
/// 当前 MVP 只实现文本；以后可以在这里扩展图片等输入类型。
#[derive(Debug)]
pub enum UserInput {
    Text { text: String },
}

impl UserMessage {
    /// 将一条 TUI 消息转换为 Agent 协议输入列表。
    pub fn into_user_inputs(self) -> Vec<UserInput> {
        vec![UserInput::Text { text: self.text }]
    }
}

/// App 发给 App Server 的命令。
///
/// `UserTurn` 表示用户提交了一组输入项，希望启动一轮 Agent 工作。
#[derive(Debug)]
pub enum AppCommand {
    UserTurn { items: Vec<UserInput> },
}

impl AppCommand {
    pub fn user_turn(items: Vec<UserInput>) -> Self {
        Self::UserTurn { items }
    }
}
