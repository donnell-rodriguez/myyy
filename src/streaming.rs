#[derive(Debug)]
pub struct StreamCompletion {
    pub text: String,
    pub deltas_matched: bool,
}

#[derive(Debug, Default)]
pub struct StreamState {
    active_turn_id: Option<u64>,
    accumulated_text: String,
}

impl StreamState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_delta(&mut self, turn_id: u64, delta: String) -> Result<&str, String> {
        match self.active_turn_id {
            None => {
                self.active_turn_id = Some(turn_id);
            }
            Some(active_turn_id) if active_turn_id == turn_id => {}
            Some(active_turn_id) => {
                return Err(format!(
                    "收到 turn {turn_id} 的增量，但当前活动流属于 turn {active_turn_id}"
                ));
            }
        }
        self.accumulated_text.push_str(&delta);
        Ok(&self.accumulated_text)
    }
    pub fn finish(
        &mut self,
        turn_id: u64,
        completed_text: String,
    ) -> Result<StreamCompletion, String> {
        if self.active_turn_id != Some(turn_id) {
            return Err(format!("turn {turn_id} 没有对应的活动消息流"));
        }
        // 1. std::mem::take()：取出旧字符串，同时把原字段恢复为空字符串。
        let streamed_text = std::mem::take(&mut self.accumulated_text);
        self.active_turn_id = None;
        Ok(StreamCompletion {
            deltas_matched: streamed_text == completed_text,
            text: completed_text,
        })
    }
}
