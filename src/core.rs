use crate::history::{ConversationHistory, ConversationItem, TurnAborted};
use crate::model::{
    FakeModel, ModelClient, ModelError, ModelEvent, ModelOutput, ModelRequest, ModelSession,
};
use crate::protocol::CoreEvent;
use crate::protocol::UserInput;
use crate::tools::{ToolCallRuntime, ToolRouter};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
// 本轮状态
// 每一轮 Agent 工作都有本轮允许使用的一组工具。
//本轮运行需要的配置和工具
struct TurnContext {
    turn_id: u64,
    tool_runtime: ToolCallRuntime,
}
// 当前正在执行的任务及其生命周期控制
#[derive(Debug)]
struct ActiveTurn {
    cancellation_token: CancellationToken,
}

#[derive(Debug)]
pub struct SessionControl {
    active_turn: Arc<Mutex<Option<ActiveTurn>>>,
}

impl SessionControl {
    //在真正启动后台任务之前，先为这一轮 Turn 占一个位置，并提前准备好取消开关。
    // reserve_turn() 负责的是“提前建立生命周期控制权”，不是执行 Agent。它让 Turn 在真正运行以前，就已经可以被发现、拒绝重复启动和取消。
    pub async fn reserve_turn(&self) -> Result<CancellationToken, String> {
        // 我要尝试预留一个 Turn，成功就返回它的取消令牌，失败就返回错误。
        let mut active_turn = self.active_turn.lock().await;
        if active_turn.is_some() {
            return Err("当前已经存在 ActiveTurn".to_string());
        }
        let cancellation_token = CancellationToken::new();
        // 为新 Turn 创建一个共享取消开关。
        *active_turn = Some(ActiveTurn {
            cancellation_token: cancellation_token.clone(),
        });
        Ok(cancellation_token)
    }

    pub async fn interrupt(&self) {
        let active_turn = self.active_turn.lock().await;
        match active_turn.as_ref() {
            Some(active_turn) => {
                println!(
                    "[Core/SessionControl] \
                     取消当前 ActiveTurn"
                );
                active_turn.cancellation_token.cancel();
            }
            None => {
                println!(
                    "[Core/SessionControl] \
                     当前没有 ActiveTurn"
                );
            }
        }
    }
}
// 读取输入
// → 请求模型
// → 处理模型输出
// → 调用工具
// → 返回最终消息
struct RegularTask;

impl RegularTask {
    fn new() -> Self {
        Self
    }
    async fn run(
        &self,
        session: &mut Session,
        context: &TurnContext,
        model_session: &mut dyn ModelSession,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
        event_tx: &mpsc::UnboundedSender<CoreEvent>,
    ) -> Result<(), ModelError> {
        println!(
            "[Core/RegularTask] session={} turn={} 开始",
            session.session_id, context.turn_id
        );

        let user_text = collect_text(input);
        println!("[Core/RegularTask] 本轮输入：{user_text:?}");
        session
            .history
            .push(ConversationItem::UserMessage { text: user_text });

        // 这里是不断的循环，根据循环的情况来确定我们的结果，是输出的是assistantmessage 还是我们的toolcall
        loop {
            println!(
                "[Core/RegularTask] \
                 请求模型，history_items={}",
                session.history.len()
            );
            let tools = context.tool_runtime.model_visible_specs();
            let request = ModelRequest::new(session.history.items(), tools);
            println!("[Core/ModelRequest] input_items={}", request.input.len());
            println!("[Core/ModelRequest] input={:#?}", request.input);
            println!("[Core/ModelRequest] tools={:#?}", request.tools);

            let mut model_stream = model_session.stream(request).await?;
            // 暂存，但不写入历史
            let mut completed_output = None;
            let mut active_item_id = None;
            let output = loop {
                match model_stream.recv().await {
                    // 开始
                    Some(Ok(ModelEvent::OutputItemStarted { item_id })) => {
                        if active_item_id.replace(item_id).is_some() {
                            return Err(ModelError::InvalidResponse(
                                "上一个输出项尚未完成，又开始了新输出项".to_string(),
                            ));
                        }
                    }
                    // 从模型当中收到增量
                    Some(Ok(ModelEvent::OutputTextDelta { delta })) => {
                        if active_item_id.is_none() {
                            return Err(ModelError::InvalidResponse(
                                "OutputTextDelta 前没有 OutputItemStarted".to_string(),
                            ));
                        }
                        let _ = event_tx.send(CoreEvent::AgentMessageDelta {
                            turn_id: context.turn_id,
                            delta,
                        });
                    }
                    // 收到完成
                    Some(Ok(ModelEvent::OutputItemDone { item_id, output })) => {
                        let Some(active_item_id) = active_item_id.take() else {
                            return Err(ModelError::InvalidResponse(
                                "OutputItemDone 前没有 OutputItemStarted".to_string(),
                            ));
                        };
                        if active_item_id != item_id {
                            return Err(ModelError::InvalidResponse(format!(
                                "完成的是 {item_id}，但当前输出项是 {active_item_id}"
                            )));
                        }
                        // 只暂存输出项；在整个响应完成前，还不能写入历史。
                        completed_output = Some(output);
                    }
                    // 只有这个事件才能结束整个响应。
                    Some(Ok(ModelEvent::ResponseCompleted)) => {
                        let Some(output) = completed_output.take() else {
                            return Err(ModelError::InvalidResponse(
                                "ResponseCompleted 前没有 OutputItemDone".to_string(),
                            ));
                        };
                        break output;
                    }
                    // 收到错误
                    Some(Err(error)) => {
                        return Err(error);
                    }
                    // 收到无
                    None => {
                        return Err(ModelError::StreamClosed("中断".to_string()));
                    }
                }
            };

            //对于不同的结果做不同的事情
            match output {
                // 处理助理信息
                ModelOutput::AssistantMessage { text } => {
                    // 我们这里不进行打印，而是通过event_tx发送出去, 上面已经增量了，这里表示的就是说是agent消息已经结束了。
                    let _ = event_tx.send(CoreEvent::AgentMessage {
                        turn_id: context.turn_id,
                        text: text.clone(),
                    });

                    // 要将输出的存储到history当中去
                    session
                        .history
                        .push(ConversationItem::AssistantMessage { text });
                    break;
                }
                //处理工具输出
                ModelOutput::ToolCall { name, arguments } => {
                    println!(
                        "[ModelOutput::ToolCall] \
                         name={name} \
                         arguments={arguments}"
                    );
                    // 要将输出的存储到history当中去
                    session.history.push(ConversationItem::ToolCall {
                        name: name.clone(),
                        arguments: arguments.clone(),
                    });
                    // 如果出现的是工具，就进行分发
                    let call = ToolRouter::build_tool_call(name.clone(), arguments);
                    // 去调用工具，然后将工具调用跟成功不成功一起写出来。
                    let (success, tool_output) =
                    // 由于我们当前是工具的取消，所以要放在这里, 使用regulartask子令牌
                    // 父令牌取消时，所有子令牌都会收到取消信号。
                    // 反过来，某个子令牌取消，不会取消整个父任务
                    // context当中装有我们运行时所需要的工具内容
                        match context.tool_runtime.handle_tool_call(call, cancellation_token.child_token()).await {
                            Ok(result) => {
                                println!("[ToolResult] {}", result.output);
                                (true, result.output)
                            }
                            Err(error) => {
                                println!("[ToolError] {error}");
                                (false, error)
                            }
                        };
                    session.history.push(ConversationItem::ToolResult {
                        name,
                        output: tool_output,
                        success,
                    });
                    println!(
                        "[Core/RegularTask] \
                         工具结果已写入历史，\
                         继续请求模型"
                    );
                }
            }
        }
        println!(
            "[Core/RegularTask] \
             turn={} 结束，\
             history_items={}",
            context.turn_id,
            session.history.len()
        );
        Ok(())
    }
}

fn collect_text(input: Vec<UserInput>) -> String {
    input
        .into_iter()
        .map(|item| match item {
            UserInput::Text { text } => text,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
// 长期状态
pub struct Session {
    session_id: u64,
    next_turn_id: u64,
    model: Arc<dyn ModelClient>,
    history: ConversationHistory,
    active_turn: Arc<Mutex<Option<ActiveTurn>>>,
}

impl Session {
    pub fn new() -> Self {
        // 具体的model
        Self::with_model(Arc::new(FakeModel::new()))
    }
    pub fn with_model(model: Arc<dyn ModelClient>) -> Self {
        Self {
            session_id: 1,
            next_turn_id: 0,
            model,
            history: ConversationHistory::new(),
            active_turn: Arc::new(Mutex::new(None)),
        }
    }

    pub fn control(&self) -> SessionControl {
        SessionControl {
            active_turn: Arc::clone(&self.active_turn),
        }
    }

    pub async fn start_turn(
        &mut self,
        input: Vec<UserInput>,
        event_tx: mpsc::UnboundedSender<CoreEvent>,
        cancellation_token: CancellationToken,
    ) {
        self.next_turn_id += 1;

        let turn_context = TurnContext {
            turn_id: self.next_turn_id,
            tool_runtime: ToolCallRuntime::new(ToolRouter::new()),
        };
        let mut model_session = self.model.new_session();
        println!("[Core/Session] 为 Turn 创建新的 ModelSession");

        println!(
            "[Core/Session {}] 创建 TurnContext(turn_id={})",
            self.session_id, turn_context.turn_id
        );
        // 开始了，那么就要通过event_tx将开始的信号发送给app
        let _ = event_tx.send(CoreEvent::TurnStarted {
            turn_id: turn_context.turn_id,
        });
        // 取消部分
        let task_cancellation_token = cancellation_token.child_token();
        // 任务创建部分
        let regular_task = RegularTask::new();
        let task_result = tokio::select! {
        // 当多个分支同时已经准备好时，按照代码从上到下的顺序选择。
        biased;
        _ = cancellation_token.cancelled() =>{
            println!(
            "[Core/Session] \
            Turn 在任务完成前收到取消信号"
            );
            None
        }
        // 在此交给我们的reglartask
        result = regular_task.run(
                    self,
                    &turn_context,
                    //对盒子内部真实模型会话的可变借用。
                    model_session.as_mut(),
                    input,
                    task_cancellation_token,
                    &event_tx,
                )=>{
                    Some(result)
                }
        };
        let model_err = task_result.and_then(Result::err);
        // 取消
        let was_cancelled = cancellation_token.is_cancelled();
        // 将取消的信息写入历史
        if was_cancelled {
            self.history
                .push(ConversationItem::TurnAborted(TurnAborted::interrupted()));
            println!("[Core/Session] 已将 TurnAborted 标记写入历史");
            println!("[Core/History] {:#?}", self.history.items());
        }
        // 将对应的active_turn给清理掉
        *self.active_turn.lock().await = None;

        println!("[Core/Session] ActiveTurn 已清理");
        // 如果出现了取消的情况，那么也要把这个信号发出去。发到app上
        // 先判断取消
        // 再判断模型错误
        // 最后才是正常完成
        let terminal_event = if was_cancelled {
            CoreEvent::TurnAborted {
                turn_id: turn_context.turn_id,
                reason: "interrupted".to_string(),
            }
        } else if let Some(error) = model_err {
            CoreEvent::TurnFailed {
                turn_id: turn_context.turn_id,
                error: error.to_string(),
            }
        } else {
            CoreEvent::TurnCompleted {
                turn_id: turn_context.turn_id,
            }
        };

        let _ = event_tx.send(terminal_event);
    }
}

#[cfg(test)]
mod tests {
    use super::ModelRequest;
    use super::Session;
    use crate::history::{ConversationItem, TurnAborted};
    use crate::model::{
        ModelClient, ModelOutput, ModelSession, ModelStreamFuture, fake_stream_from_output,
    };
    use crate::protocol::{CoreEvent, UserInput};
    use crate::tools::ToolSpec;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    struct FixedModel;
    struct FixedModelSession;

    impl ModelClient for FixedModel {
        fn new_session(&self) -> Box<dyn ModelSession> {
            Box::new(FixedModelSession)
        }
    }
    // 同一个 Turn 的连续模型请求共享连接状态、请求计数以及未来的路由信息。

    impl ModelSession for FixedModelSession {
        fn stream<'a>(&'a mut self, _request: ModelRequest) -> ModelStreamFuture<'a> {
            Box::pin(async {
                Ok(fake_stream_from_output(ModelOutput::AssistantMessage {
                    text: "来自替代模型".to_string(),
                }))
            })
        }
    }

    #[tokio::test]
    async fn session_accepts_alternative_model_client() {
        let model: Arc<dyn ModelClient> = Arc::new(FixedModel);

        let mut session = Session::with_model(model);

        let control = session.control();
        let cancellation_token = control.reserve_turn().await.expect("应该成功预留 Turn");

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        session
            .start_turn(
                vec![UserInput::Text {
                    text: "你好".to_string(),
                }],
                event_tx,
                cancellation_token,
            )
            .await;

        let mut final_text = None;

        while let Some(event) = event_rx.recv().await {
            match event {
                CoreEvent::AgentMessage { text, .. } => {
                    final_text = Some(text);
                }

                CoreEvent::TurnCompleted { .. } => {
                    break;
                }

                CoreEvent::TurnAborted { reason, .. } => {
                    panic!("Turn 不应该终止：{reason}");
                }

                _ => {}
            }
        }

        assert_eq!(final_text.as_deref(), Some("来自替代模型"));

        assert!(matches!(
            session.history.items().last(),
            Some(ConversationItem::AssistantMessage { text })
                if text == "来自替代模型"
        ));
    }

    #[test]
    fn model_request_contains_complete_history() {
        let history = vec![
            ConversationItem::UserMessage {
                text: "请执行 pwd".to_string(),
            },
            ConversationItem::TurnAborted(TurnAborted::interrupted()),
            ConversationItem::UserMessage {
                text: "请继续".to_string(),
            },
        ];
        let tools = vec![ToolSpec {
            name: "exec_command".to_string(),
            description: "执行终端命令".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string"
                    }
                },
                "required": ["cmd"],
                "additionalProperties": false
            }),
        }];
        let actual = ModelRequest::new(history.as_slice(), tools.clone());
        let expected = ModelRequest {
            input: history,
            tools,
        };
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn interrupt_after_reservation_is_not_lost() {
        let session = Session::new();
        let control = session.control();
        let cancellation_token = control
            .reserve_turn()
            .await
            .expect("应该成功预留第一轮任务");
        control.interrupt().await;
        assert!(cancellation_token.is_cancelled())
    }

    #[tokio::test]
    async fn cancelled_turn_emits_no_agent_message() {
        let mut session = Session::new();
        let control = session.control();
        let cancellation_token = control
            .reserve_turn()
            .await
            .expect("应该成功预留第一轮任务");
        control.interrupt().await;
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        session
            .start_turn(
                vec![UserInput::Text {
                    text: "请执行 pwd".to_string(),
                }],
                event_tx,
                cancellation_token,
            )
            .await;
        let first_event = event_rx.recv().await.expect("应该收到 TurnStarted");
        assert!(matches!(first_event, CoreEvent::TurnStarted { turn_id: 1 }));

        let second_event = event_rx.recv().await.expect("应该收到 TurnAborted");
        assert!(
            matches!(second_event, CoreEvent::TurnAborted { turn_id:1, reason } if reason == "interrupted")
        );
        println!("[Test/History] {:#?}", session.history.items());
        assert_eq!(
            session.history.items(),
            &[ConversationItem::TurnAborted(TurnAborted::interrupted())]
        );

        assert!(
            event_rx.recv().await.is_none(),
            "TurnAborted 后不应该再出现 AgentMessage 或其他事件"
        )
    }

    #[tokio::test]
    async fn model_error_emits_turn_failed_not_completed() {
        let mut session = Session::new();
        let control = session.control();

        let cancellation_token = control.reserve_turn().await.expect("应该成功预留 Turn");

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        session
            .start_turn(
                vec![UserInput::Text {
                    text: "请模拟模型失败".to_string(),
                }],
                event_tx,
                cancellation_token,
            )
            .await;

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnStarted { turn_id: 1 })
        ));

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnFailed {
                turn_id: 1,
                error,
            }) if error
                == "模型请求失败：模拟服务不可用"
        ));

        assert!(
            event_rx.recv().await.is_none(),
            "TurnFailed 后不能再出现 \
         TurnCompleted 或 AgentMessage"
        );

        control
            .reserve_turn()
            .await
            .expect("失败后 ActiveTurn 应该已经清理");
    }
    #[tokio::test]
    async fn mid_stream_error_emits_partial_delta_then_turn_failed() {
        let mut session = Session::new();
        let control = session.control();

        let cancellation_token = control.reserve_turn().await.expect("应该成功预留 Turn");

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        session
            .start_turn(
                vec![UserInput::Text {
                    text: "请模拟流中失败".to_string(),
                }],
                event_tx,
                cancellation_token,
            )
            .await;

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnStarted { turn_id: 1 })
        ));

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::AgentMessageDelta {
                turn_id: 1,
                delta,
            }) if delta == "部分回答"
        ));

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnFailed {
                turn_id: 1,
                error,
            }) if error == "模型流失败：模拟连接中断"
        ));

        assert!(
            event_rx.recv().await.is_none(),
            "流失败后不能出现 AgentMessage 或 TurnCompleted"
        );

        assert_eq!(
            session.history.items(),
            &[ConversationItem::UserMessage {
                text: "请模拟流中失败".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn output_item_done_is_not_response_completion() {
        let mut session = Session::new();
        let control = session.control();

        let cancellation_token = control.reserve_turn().await.expect("应该成功预留 Turn");

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        session
            .start_turn(
                vec![UserInput::Text {
                    text: "请模拟输出项后失败".to_string(),
                }],
                event_tx,
                cancellation_token,
            )
            .await;

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnStarted { turn_id: 1 })
        ));

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnFailed {
                turn_id: 1,
                error
            }) if error
                == "模型流失败：响应完成前连接中断"
        ));

        assert!(event_rx.recv().await.is_none());

        assert_eq!(
            session.history.items(),
            &[ConversationItem::UserMessage {
                text: "请模拟输出项后失败".to_string(),
            }]
        );
    }
    #[tokio::test]
    async fn mismatched_item_ids_fail_the_turn() {
        let mut session = Session::new();
        let control = session.control();

        let cancellation_token = control.reserve_turn().await.expect("应该成功预留 Turn");

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        session
            .start_turn(
                vec![UserInput::Text {
                    text: "请模拟输出项关联失败".to_string(),
                }],
                event_tx,
                cancellation_token,
            )
            .await;

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnStarted { turn_id: 1 })
        ));

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::AgentMessageDelta {
                turn_id: 1,
                delta
            }) if delta == "不能提交到历史"
        ));

        assert!(matches!(
            event_rx.recv().await,
            Some(CoreEvent::TurnFailed {
                turn_id: 1,
                error
            }) if error
                == "模型响应协议错误：完成的是 item-2，但当前输出项是 item-1"
        ));

        assert!(event_rx.recv().await.is_none());

        assert_eq!(
            session.history.items(),
            &[ConversationItem::UserMessage {
                text: "请模拟输出项关联失败".to_string(),
            }]
        );
    }
}
