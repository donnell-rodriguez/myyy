use crate::history::{ConversationHistory, ConversationItem};
use crate::model::{FakeModel, ModelOutput};
use crate::protocol::UserInput;
use crate::tools::ToolRouter;

// 本轮状态
// 每一轮 Agent 工作都有本轮允许使用的一组工具。
struct TurnContext {
    turn_id: u64,
    tool_router: ToolRouter,
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
    async fn run(&self, session: &mut Session, context: &TurnContext, input: Vec<UserInput>) {
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
                    println!(
                        "[FinalAssistantMessage] \
                         {text}"
                    );
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
                    let (success, tool_output) = match context.tool_router.dispatch(call).await {
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
}

impl Session {
    pub fn new() -> Self {
        Self {
            session_id: 1,
            next_turn_id: 0,
            model: FakeModel::new(),
            history: ConversationHistory::new(),
        }
    }

    pub async fn start_turn(&mut self, input: Vec<UserInput>) {
        self.next_turn_id += 1;
        let turn_context = TurnContext {
            turn_id: self.next_turn_id,
            tool_router: ToolRouter::new(),
        };
        println!(
            "[Core/Session {}] 创建 TurnContext(turn_id={})",
            self.session_id, turn_context.turn_id
        );
        // 在此交给我们的reglartask
        RegularTask::new().run(self, &turn_context, input).await;
    }
}
