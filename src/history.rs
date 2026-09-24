#[derive(Debug)]
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
