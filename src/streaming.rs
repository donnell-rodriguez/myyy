/// 一条流式 Agent 消息完成后的最终结果。
#[derive(Debug, PartialEq, Eq)]
pub struct StreamCompletion {
    pub text: String,
    pub deltas_matched: bool,
}
/// App 侧正在累积的一条 Agent 消息流。
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

#[cfg(test)]
mod tests {
    use super::StreamCompletion;
    use super::StreamState;

    #[test]
    fn accumulates_deltas_and_finishes() {
        let mut state = StreamState::new();
        assert_eq!(state.push_delta(1, "你".to_string()), Ok("你"));
        assert_eq!(state.push_delta(1, "好".to_string()), Ok("你好"));
        let completion = state.finish(1, "你好".to_string()).unwrap();
        assert_eq!(
            completion,
            StreamCompletion {
                text: "你好".to_string(),
                deltas_matched: true
            }
        );
    }
    #[test]
    fn rejects_delta_from_another_turn() {
        let mut state = StreamState::new();
        state.push_delta(1, "A".to_string()).unwrap();
        let error = state.push_delta(2, "B".to_string()).unwrap_err();
        assert_eq!(error, "收到 turn 2 的增量，但当前活动流属于 turn 1");
        assert_eq!(
            state.finish(1, "A".to_string()).unwrap(),
            StreamCompletion {
                text: "A".to_string(),
                deltas_matched: true,
            }
        )
    }
    #[test]
    fn can_be_reused_after_finish() {
        let mut state = StreamState::new();
        state.push_delta(1, "first".to_string()).unwrap();
        state.finish(1, "first".to_string()).unwrap();

        state.push_delta(2, "second".to_string()).unwrap();

        let completion = state.finish(2, "second".to_string()).unwrap();

        assert_eq!(
            completion,
            StreamCompletion {
                text: "second".to_string(),
                deltas_matched: true,
            }
        );
    }
}
