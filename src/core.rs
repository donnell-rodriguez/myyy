use crate::history::{ConversationHistory, ConversationItem, TurnAborted};
use crate::model::{FakeModel, ModelOutput};
use crate::protocol::UserInput;
use crate::tools::{ToolCallRuntime, ToolRouter};
// use tokio::time::{Duration, sleep};
use crate::protocol::CoreEvent;
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
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
        event_tx: &mpsc::UnboundedSender<CoreEvent>,
    ) {
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

            //调用模型，拿到结果
            let output = session.model.respond(session.history.items()).await;
            //对于不同的结果做不同的事情
            match output {
                ModelOutput::AssistantMessage { text } => {
                    // println!("[FinalAssistantMessage] {text}");
                    // 增量
                    for character in text.chars() {
                        let _ = event_tx.send(CoreEvent::AgentMessageDelta {
                            turn_id: context.turn_id,
                            delta: character.to_string(),
                        });
                        tokio::time::sleep(std::time::Duration::from_millis(35)).await;
                    }
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
    model: FakeModel,
    history: ConversationHistory,
    active_turn: Arc<Mutex<Option<ActiveTurn>>>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            session_id: 1,
            next_turn_id: 0,
            model: FakeModel::new(),
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
        tokio::select! {
        // 当多个分支同时已经准备好时，按照代码从上到下的顺序选择。
        biased;
        _ = cancellation_token.cancelled() =>{
            println!(
            "[Core/Session] \
            Turn 在任务完成前收到取消信号"
            );
        }
        // 在此交给我们的reglartask
        _ = regular_task.run(
                    self,
                    &turn_context,
                    input,
                    task_cancellation_token,
                    &event_tx,
                )=>{}
            }
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
        let terminal_event = if was_cancelled {
            CoreEvent::TurnAborted {
                turn_id: turn_context.turn_id,
                reason: "interrupted".to_string(),
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
    use tokio::sync::mpsc;

    use crate::protocol::{CoreEvent, UserInput};

    use super::Session;
    use crate::history::{ConversationItem, TurnAborted};

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
}
