// 一轮任务被用户主动中止后，写入模型上下文的说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAborted {
    pub guidance: String,
}

impl TurnAborted {
    pub fn interrupted() -> Self {
        Self {
            guidance: concat!(
                "用户主动中止了上一轮任务。",
                "如果上一轮执行了工具或命令，它们可能只完成了一部分。"
            )
            .to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationItem {
    UserMessage {
        text: String,
    },
    AssistantMessage {
        text: String,
    },
    ToolCall {
        name: String,
        arguments: String,
    },
    ToolResult {
        name: String,
        output: String,
        success: bool,
    },
    TurnAborted(TurnAborted),
}

pub struct ConversationHistory {
    items: Vec<ConversationItem>,
}

impl ConversationHistory {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, item: ConversationItem) {
        self.items.push(item);
    }
    pub fn items(&self) -> &[ConversationItem] {
        &self.items
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
}
