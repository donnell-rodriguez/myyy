# myyy 第一阶段学习主干

这是一个可以运行的最小 Agent TUI 主干，不连接真实模型，也不执行真实沙箱命令。

数据流：

```text
终端整行输入
  -> TuiEvent::Submitted
  -> UserMessage
  -> Vec<UserInput>
  -> AppCommand::UserTurn
  -> 进程内 App Server Channel
  -> TurnContext
  -> core::run_turn
  -> AgentEvent
  -> App 显示结果
```

建议按以下顺序手抄：

1. `src/protocol.rs`：先认识所有数据盒子。
2. `src/main.rs`：观察通道和组件怎样组装。
3. `src/tui.rs`：输入怎样变成 `TuiEvent`。
4. `src/app.rs`：主事件循环和三次数据变形。
5. `src/app_server.rs`：Channel 边界和 turn 创建。
6. `src/core.rs`：一轮 Agent 怎样开始、输出和完成。

运行：

```bash
cargo run
```

输入：

```text
请执行 pwd
```

退出：

```text
/quit
```

## MVP 12：Shared Tool Ownership

目标：让工具注册表和工具执行流程能够同时拥有同一个工具，为后续独立异步任务做准备。

数据流：

```text
ToolRegistry
  -> Arc<dyn ToolExecutor>
  -> tool() 调用 Arc::clone
  -> dispatch 获得独立共享句柄
  -> ToolExecutor::handle
```

Rust 概念：

- `Arc<T>`：通过原子引用计数共享同一个值。
- `Arc::clone`：增加引用计数，不复制底层工具。
- `Arc<dyn ToolExecutor>`：共享一个经过 trait object 类型擦除的工具执行器。

生产源码对应：Codex 的 `ToolRegistry` 使用 `Arc<dyn CoreToolRuntime>` 保存并返回工具运行时。

本阶段仍然省略：独立 Tokio 任务、取消令牌、并行工具、sandbox 和生产遥测。

验收结果：`cargo fmt --check`、`cargo check --locked --offline` 和 `请执行 pwd` 完整 Agent 回路均通过。

下一阶段：MVP 13，使用独立异步任务执行工具调用。

## MVP 13：Spawned Tool Tasks

目标：通过 `ToolCallRuntime` 把一次工具调用放入独立 Tokio 任务，并通过 `JoinHandle` 等待任务结果。

数据流：

```text
RegularTask
  -> ToolCallRuntime::handle_tool_call
  -> tokio::spawn
  -> ToolRouter
  -> ToolRegistry
  -> ToolExecutor
  -> JoinHandle 返回工具结果
```

Rust 概念：

- `tokio::spawn`：在当前 Tokio runtime 中创建独立异步任务，不是创建操作系统进程。
- `async move`：把 `Arc<ToolRouter>` 和 `ToolCall` 的所有权移动进任务。
- `JoinHandle<Result<ToolResult, String>>`：同时表达任务层错误和工具层错误。

生产源码对应：Codex 的 `ToolCallRuntime` 创建独立工具调度任务，并在外层管理任务完成、错误与后续取消。

本阶段仍然省略：取消令牌、`tokio::select!`、主动 abort、多工具并行、sandbox 和生产遥测。

验收结果：格式与编译通过；`请执行 pwd` 经独立任务成功执行；`请执行 ls` 经独立任务后仍被审批策略拒绝。

下一阶段：MVP 14，为工具任务增加可观察的取消信号与取消结果。

## MVP 18–20：Core Events and Streaming State

目标：让 Core 不再直接展示最终回答，而是通过事件通道把 Turn 生命周期、Agent 消息增量和完整消息交给 App，并由 App 持有 `StreamState` 累积流式文本。

数据流：

```text
Core
  -> CoreEvent::TurnStarted
  -> CoreEvent::AgentMessageDelta × N
  -> CoreEvent::AgentMessage
  -> CoreEvent::TurnCompleted / TurnAborted
  -> App Server event channel
  -> App::StreamState
  -> FinalAssistantMessage
```

Rust 概念：

- `mpsc::UnboundedSender<CoreEvent>`：Core 向界面异步发送事件。
- `Option<u64>`：记录当前流属于哪个 Turn。
- `String::push_str`：把消息增量依次拼接起来。
- `std::mem::take`：取出累积结果，同时清空流状态。

生产源码对应：Codex Core 发送 Agent 消息增量和完成事件；TUI 的流式状态负责累积、渲染，并在完整消息到达后完成合并。

本阶段仍然省略：真实模型响应流、`item_id`、Markdown 增量渲染、流队列背压，以及 UserTurn 与立即 Interrupt 之间的启动竞态治理。

验收结果：`cargo fmt --check`、`cargo check`、`cargo test`、`请执行 pwd` 流式回路和 ActiveTurn 建立后的 `/cancel` 回路均通过；正常完成与取消完成时 `deltas_matched=true`。

下一阶段：MVP 21，为 `StreamState` 编写单元测试，验证正常完成、Turn 不匹配和完成后状态复用。
