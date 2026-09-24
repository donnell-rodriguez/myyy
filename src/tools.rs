use std::{collections::HashMap, pin::Pin};

use serde::Deserialize;
use tokio::process::Command;
#[derive(Debug)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug)]
pub struct ToolResult {
    pub output: String,
}

// 这里只是有一个工具叫做exec_command
// 如果多了之后怎么办，这就没有办法进行使用了。所以这里我们需要另外一套逻辑。
// pub struct ToolRouter {
//     exec_command: ExecCommandHandler,
// }
// impl ToolRouter {
//     pub fn new() -> Self {
//         Self {
//             exec_command: ExecCommandHandler::new(),
//         }
//     }

//     pub fn build_tool_call(name: String, arguments: String) -> ToolCall {
//         ToolCall { name, arguments }
//     }
//     // 查看模型想调用哪个工具， 当前是写死的状态，我们进行灵活的写
//     pub async fn dispatch(&self, call: ToolCall) -> Result<ToolResult, String> {
//         match call.name.as_str() {
//             "exec_command" => self.exec_command.handle(call).await,
//             unsupported => Err(format!("不支持的工具:{unsupported}")),
//         }
//     }
// }
pub struct ToolRouter {
    registry: ToolRegistry,
}

impl ToolRouter {
    pub fn new() -> Self {
        let mut registry = ToolRegistry::new();
        // 讲其中一个工具给推送进来
        registry.register(ExecCommandHandler::new());
        Self { registry }
    }

    pub fn build_tool_call(name: String, arguments: String) -> ToolCall {
        ToolCall { name, arguments }
    }

    pub async fn dispatch(&self, call: ToolCall) -> Result<ToolResult, String> {
        self.registry.dispatch(call).await
    }
}

// 不同工具产生的 Future 类型不同，
// 所以统一装入 Box，并通过 dyn Future 隐藏具体类型。
type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<ToolResult, String>> + Send + 'a>>;
// 定义一个trait叫做ToolExecutor
trait ToolExecutor: Send + Sync {
    fn tool_name(&self) -> &'static str;
    // 执行一次工具调用。
    fn handle<'a>(&'a self, call: ToolCall) -> ToolFuture<'a>;
}

struct ToolRegistry {
    tools: HashMap<String, Box<dyn ToolExecutor>>,
}

impl ToolRegistry {
    fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    fn register<T>(&mut self, tool: T)
    where
        T: ToolExecutor + 'static,
    {
        let name = tool.tool_name().to_string();
        let previous = self.tools.insert(name.clone(), Box::new(tool));
        assert!(previous.is_none(), "重复注册工具：{name}");
        println!("[ToolRegistry] 已注册工具：{name}");
    }

    async fn dispatch(&self, call: ToolCall) -> Result<ToolResult, String> {
        let tool_name = call.name.clone();
        let tool = self
            .tools
            .get(&tool_name)
            .ok_or_else(|| format!("未注册工具：{tool_name}"))?;
        println!("[ToolRegistry] 路由到工具：{tool_name}");
        tool.handle(call).await
    }
}

#[derive(Debug, Deserialize)]
struct ExecCommandArgs {
    cmd: String,
}

enum ApprovalDecision {
    Approved,
    Denied(String),
}

struct ApprovalPolicy;
impl ApprovalPolicy {
    fn review(&self, args: &ExecCommandArgs) -> ApprovalDecision {
        if args.cmd == "pwd" {
            ApprovalDecision::Approved
        } else {
            ApprovalDecision::Denied(format!("本阶段只允许pwd， 拒绝命令：{}", args.cmd))
        }
    }
}

struct ExecCommandHandler {
    approval_policy: ApprovalPolicy,
}

impl ExecCommandHandler {
    fn new() -> Self {
        Self {
            approval_policy: ApprovalPolicy,
        }
    }
    async fn execute(&self, call: ToolCall) -> Result<ToolResult, String> {
        let args: ExecCommandArgs = serde_json::from_str(&call.arguments)
            .map_err(|error| format!("工具参数不是合法json:{error}"))?;
        // 审批合法性
        match self.approval_policy.review(&args) {
            ApprovalDecision::Approved => {
                println!(
                    "[ApprovalPolicy] \
                     已批准只读命令：pwd"
                );
            }
            ApprovalDecision::Denied(reason) => {
                return Err(reason);
            }
        }
        // 创建的是“进程启动配置”，此时通常还没有真正启动进程。当前异步任务等待 pwd 运行完成，但不会阻塞整个 Tokio 运行时。
        let output = Command::new("/bin/pwd").output().await.map_err(|error| {
            format!(
                "启动 pwd 子进程失败：\
                         {error}"
            )
        })?;
        if !output.status.success() {
            return Err(format!("pwd 退出失败：{}", output.status));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(ToolResult { output: stdout })
    }
}

impl ToolExecutor for ExecCommandHandler {
    fn tool_name(&self) -> &'static str {
        "exec_command"
    }

    fn handle<'a>(&'a self, call: ToolCall) -> ToolFuture<'a> {
        Box::pin(async move { self.execute(call).await })
    }
}
// struct FakeExecCommandHandler;

// impl FakeExecCommandHandler {
//     async fn handle(&self, call:ToolCall) ->ToolResult {
//         println!(
//             "[FakeExecCommandHandler] 收到参数：{}",
//             call.arguments
//         );
//         ToolResult {
//             output: concat!("模拟结果：/fake/project",
//                 "（没有执行真实 pwd）").to_string(),
//         }
//     }
// }
