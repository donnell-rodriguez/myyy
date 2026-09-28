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

## MVP 21：Stream State Tests

目标：把流式消息状态的不变量写成可重复执行的单元测试，不再只依赖手动观察终端输出。

被保护的不变量：

- 同一个 Turn 的多个增量必须按顺序合并，并与最终完整消息一致。
- 另一个 Turn 的增量不能混入当前活动消息流。
- 一条消息完成后，`StreamState` 必须清空并可供下一 Turn 复用。

Rust 概念：

- `#[cfg(test)]`：只在测试构建中编译测试模块。
- `#[test]`：把普通同步函数注册为 Rust 测试。
- `PartialEq` 与 `Eq`：允许对完整的 `StreamCompletion` 值执行相等比较。
- `unwrap_err()`：测试预期失败分支并取得错误值。

生产源码对应：Codex 使用测试固定流式增量、完整消息和 Turn 归属等状态不变量；教学项目先对纯状态机做确定性单元测试。

本阶段仍然省略：真实模型流、`item_id`、多条并发消息，以及 UserTurn 与立即 Interrupt 之间的启动竞态修复。

验收结果：`cargo fmt --check` 与 `cargo test` 通过；3 个测试全部成功。

下一阶段：MVP 22，用确定性测试复现并修复“UserTurn 刚被接受时立即取消可能丢失”的竞态。

## MVP 22：Reserve Active Turn Before Spawn

目标：App Server 接受 `UserTurn` 后，先预留 `ActiveTurn`，再启动后台任务，避免紧随其后的 `Interrupt` 找不到取消目标。

数据流：

```text
AppCommand::UserTurn
  -> SessionControl::reserve_turn
  -> ActiveTurn 持有 CancellationToken
  -> tokio::spawn
  -> Session::start_turn
  -> AppCommand::Interrupt
  -> SessionControl::interrupt
  -> CancellationToken::cancel
```

被保护的不变量：一轮任务一旦被 App Server 接受，在后台任务真正开始前到达的取消信号也不能丢失。

Rust 概念：

- `Mutex<Option<ActiveTurn>>`：让“检查是否空闲”和“写入预留状态”成为一次受锁保护的状态转换。
- `Result<CancellationToken, String>`：显式表达预留成功或已有活动 Turn。
- `CancellationToken::clone`：预留状态与后台任务观察同一个共享取消状态。

生产源码对应：Codex 在进行异步 Turn 准备前先在 `active_turn` 中插入一个尚未附加实际任务的 `ActiveTurn`，随后 Interrupt 就能观察到该生命周期状态。

本阶段仍然省略：任务 panic 后的自动清理，以及取消后立刻停止模型循环并禁止继续输出最终消息。

验收结果：`cargo fmt --check` 与 `cargo test` 通过，4 个测试全部成功；连续输入 `请执行 pwd` 和 `/cancel` 时，日志显示 `取消当前 ActiveTurn`、工具任务取消及 `TurnAborted`。

下一阶段：MVP 23，让取消信号终止 `RegularTask` 循环，取消后不再继续请求模型或发送最终 Agent 消息。

## MVP 23：Cancel the Whole Turn Task

目标：让 Session 同时等待取消信号与 `RegularTask` 完成；取消发生时立即丢弃任务 Future，不再继续请求模型或发送最终 Agent 消息。

数据流：

```text
Session::start_turn
  -> tokio::select!
     -> CancellationToken::cancelled
     -> RegularTask::run
  -> 清理 ActiveTurn
  -> TurnAborted / TurnCompleted
```

被保护的不变量：被取消的 Turn 只能发送 `TurnStarted` 和一个 `TurnAborted` 终止结果，不能再发送 `AgentMessageDelta`、`AgentMessage` 或 `TurnCompleted`。

Rust 概念：

- `tokio::select!`：同时轮询取消 Future 与任务 Future，先完成的分支获胜。
- `biased;`：多个分支同时就绪时按书写顺序选择，让已发生的取消优先于任务启动。
- Future 丢弃：取消分支获胜后，未完成的 `RegularTask::run` Future 被丢弃，停止继续推进。

生产源码对应：Codex 的 Session Task 接收取消令牌；中止流程取消该令牌，并阻止已取消任务继续走正常完成生命周期。

本阶段仍然省略：Turn 已经发送部分流式增量后，App 侧主动清空未完成的 `StreamState`。

验收结果：`cargo fmt --check` 与 `cargo test` 通过，5 个测试全部成功；确定性测试验证取消后的完整事件序列只有 `TurnStarted` 和 `TurnAborted`。

下一阶段：MVP 24，在 `TurnAborted` 到达时丢弃属于该 Turn 的部分流式状态，避免污染下一轮消息。
