use crate::history::{ConversationHistory, ConversationItem};
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
    pub async fn reserve_turn(&self) -> Result<CancellationToken, String> {
        let mut active_turn = self.active_turn.lock().await;
        if active_turn.is_some() {
            return Err("当前已经存在 ActiveTurn".to_string());
        }
        let cancellation_token = CancellationToken::new();
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
        // for item in input {
        //     match item {
        //         UserInput::Text { text } => {
        //             println!("[Core/RegularTask] 本轮输入：{text:?}");
        //         }
        //     }
        // }

        let user_text = collect_text(input);
        println!("[Core/RegularTask] 本轮输入：{user_text:?}");
        session
            .history
            .push(ConversationItem::UserMessage { text: user_text });

        // // println!("[Core/RegularTask] 模型调用尚未实现");
        // let output = session.model.respond(&user_text).await;
        // match output {
        //     ModelOutput::AssistantMessage { text }=>{
        //         println!(
        //             "[ModelOutput::AssistantMessage] {text}"
        //         );
        //     }
        //     ModelOutput::ToolCall { name, arguments }=>{
        //         println!(
        //             "[ModelOutput::ToolCall] \
        //              name={name} arguments={arguments}"
        //         );

        //         // println!(
        //         //     "[SAFETY PLACEHOLDER] \
        //         //      本轮只展示工具调用，不执行命令"
        //         // );
        //         let call = ToolRouter::build_tool_call(name, arguments);
        //         match context.tool_router.dispatch(call).await{
        //             Ok(result)=>{
        //                 println!(
        //                     "[ToolResult] {}",
        //                     result.output
        //                 );
        //             }
        //             Err(error)=>{
        //                 println!(
        //                     "[ToolError] {error}"
        //                 );
        //             }
        //         }
        //     }
        // }
        // 这里是不断的循环，根据循环的情况来确定我们的结果，是输出的是assistantmessage 还是我们的toolcall
        loop {
            println!(
                "[Core/RegularTask] \
                 请求模型，history_items={}",
                session.history.len()
            );

            //调用模型
            let output = session.model.respond(session.history.items()).await;
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
        // let should_auto_cancel = input.iter().any(|item| match item {
        //     UserInput::Text { text } => text.contains("等待"),
        // });
        // self.active_turn = Some(ActiveTurn {
        //     cancellation_token: CancellationToken::new(),
        // });
        // let cancellation_token = self
        //     .active_turn
        //     .as_ref()
        //     .expect("activeturn 刚刚创建，必然存在")
        //     .cancellation_token
        //     .clone();

        // if should_auto_cancel {
        //     let cancellation_token = cancellation_token.clone();
        //     tokio::spawn(async move {
        //         sleep(Duration::from_millis(500)).await;
        //         println!("[Session/ActiveTurn] 发出取消信号");
        //         cancellation_token.cancel();
        //     });
        // }
        let turn_context = TurnContext {
            turn_id: self.next_turn_id,
            tool_runtime: ToolCallRuntime::new(ToolRouter::new()),
        };
        println!(
            "[Core/Session {}] 创建 TurnContext(turn_id={})",
            self.session_id, turn_context.turn_id
        );
        // 开始了，那么就要讲event_tx发送给app
        let _ = event_tx.send(CoreEvent::TurnStarted {
            turn_id: turn_context.turn_id,
        });

        // 在此交给我们的reglartask
        RegularTask::new()
            .run(
                self,
                &turn_context,
                input,
                cancellation_token.child_token(),
                &event_tx,
            )
            .await;
        let was_cancelled = cancellation_token.is_cancelled();

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
    use super::Session;

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
}
