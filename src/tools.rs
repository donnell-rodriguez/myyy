use std::{collections::HashMap, pin::Pin, sync::Arc, time::Duration};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
#[derive(Debug)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug)]
pub struct ToolResult {
    pub output: String,
}

// 提供给模型看的工具说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

// ToolRouter：接收并转交工具调用
pub struct ToolRouter {
    registry: ToolRegistry,
}

impl ToolRouter {
    pub fn new() -> Self {
        let mut registry = ToolRegistry::new();
        // 把其中一个工具给推送进来
        registry.register(ExecCommandHandler::new());
        Self { registry }
    }

    pub fn build_tool_call(name: String, arguments: String) -> ToolCall {
        ToolCall { name, arguments }
    }

    pub async fn dispatch(&self, call: ToolCall) -> Result<ToolResult, String> {
        self.registry.dispatch(call).await
    }
    pub fn model_visible_specs(&self) -> Vec<ToolSpec> {
        self.registry.model_visible_specs()
    }
}

// 不同工具产生的 Future 类型不同，
// 所以统一装入 Box，并通过 dyn Future 隐藏具体类型。
type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<ToolResult, String>> + Send + 'a>>;
// 定义一个trait叫做ToolExecutor
// ToolExecutor：执行具体工具行为
trait ToolExecutor: Send + Sync {
    fn tool_name(&self) -> &'static str;
    // 返回给模型看的工具说明。
    fn spec(&self) -> ToolSpec;
    // 执行模型产生的工具调用。
    fn handle<'a>(&'a self, call: ToolCall) -> ToolFuture<'a>;
}

// ToolRegistry：根据名称找到工具
struct ToolRegistry {
    tools: HashMap<String, Arc<dyn ToolExecutor>>,
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
        let previous = self.tools.insert(name.clone(), Arc::new(tool));
        assert!(previous.is_none(), "重复注册工具：{name}");
        println!("[ToolRegistry] 已注册工具：{name}");
    }

    // 查找并克隆句柄
    fn tool(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        self.tools.get(name).map(Arc::clone)
    }

    async fn dispatch(&self, call: ToolCall) -> Result<ToolResult, String> {
        let tool_name = call.name.clone();
        let tool = self
            .tool(&tool_name)
            .ok_or_else(|| format!("未注册工具：{tool_name}"))?;
        println!("[ToolRegistry] 路由到工具：{tool_name}");
        tool.handle(call).await
    }

    fn model_visible_specs(&self) -> Vec<ToolSpec> {
        let mut specs = self
            .tools
            .values()
            .map(|tool| tool.spec())
            .collect::<Vec<_>>();
        // HashMap 本身不保证迭代顺序。如果以后有多个工具，不排序可能导致每次请求的工具顺序不同。
        // 稳定顺序有利于：
        // - 测试稳定
        // - Prompt 稳定
        // - 请求缓存
        // - 调试比较
        specs.sort_by(|left, right| left.name.cmp(&right.name));
        specs
    }
}

// ToolCallRuntime：管理一次工具任务的生命周期
#[derive(Clone)]
pub struct ToolCallRuntime {
    router: Arc<ToolRouter>,
}
impl ToolCallRuntime {
    pub fn new(router: ToolRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
    pub fn model_visible_specs(&self) -> Vec<ToolSpec> {
        self.router.model_visible_specs()
    }

    pub async fn handle_tool_call(
        &self,
        call: ToolCall,
        cancellation_token: CancellationToken,
    ) -> Result<ToolResult, String> {
        let router = Arc::clone(&self.router);

        println!("[ToolCallRuntime] 创建独立工具任务");
        let mut task_handle = tokio::spawn(async move {
            println!("[ToolCallRuntime] 工具任务开始");
            router.dispatch(call).await
        });
        // 对于创建的异步来说，谁先完成谁拿到结果
        tokio::select! {
                join_result = &mut task_handle=>{
                    let result = join_result.map_err(|error| {
                        format!("工具任务连接失败：{error}")
                    })?;
                    println!("[ToolCallRuntime] 工具任务结束");
                    result
                },

                // 取消的这个信号收到
                _ = cancellation_token.cancelled()=> {
                    println!("[ToolCallRuntime]收到取消信号，终止工具任务");
                // 处理取消后的结果
                task_handle.abort();
                match task_handle.await {
                     Ok(result)=> result,
                     Err(error) if error.is_cancelled()=>{Err("工具任务已取消".to_string())}
                     Err(error)=>{Err(format!("取消工具任务失败：{error}"))}
                }

            }
        }
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
        if args.cmd == "pwd" || args.cmd == "wait" {
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
                println!("[ApprovalPolicy] 已批准命令：{}", args.cmd);
            }
            ApprovalDecision::Denied(reason) => {
                return Err(reason);
            }
        }
        // 具体执行
        if args.cmd == "wait" {
            println!("[ExecCommandHandler] 开始等待 5 秒");
            sleep(Duration::from_secs(5)).await;
            return Ok(ToolResult {
                output: "等待完成 ".to_string(),
            });
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

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.tool_name().to_string(),
            description: "执行一个经过审批的终端命令".to_string(),
            // JSON 只是描述参数，并没有执行命令。
            parameters: json!(
                {
                    "type":"object",
                    "properties":{
                        "cmd":{
                            "type":"string",
                            "description":"需要执行的终端命令",
                        }
                    },
                    "required":["cmd"],
                    "additionalProperties":false
                }
            ),
        }
    }
    // 这里来到了具体执行的工具这个阶段了
    fn handle<'a>(&'a self, call: ToolCall) -> ToolFuture<'a> {
        Box::pin(async move { self.execute(call).await })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ExecCommandHandler, ToolExecutor, ToolRouter};

    #[test]
    fn registered_tools_are_model_visible() {
        let router = ToolRouter::new();

        let expected = vec![ExecCommandHandler::new().spec()];

        assert_eq!(router.model_visible_specs(), expected);

        let specs = router.model_visible_specs();
        assert_eq!(specs[0].name, "exec_command");
        assert_eq!(specs[0].parameters["required"], json!(["cmd"]));
        assert_eq!(specs[0].parameters["additionalProperties"], json!(false));
    }
}
