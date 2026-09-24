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
