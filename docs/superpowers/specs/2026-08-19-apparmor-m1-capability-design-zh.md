# AppArmor M1 Capability 约束设计

## 状态

已于 2026-08-19 在对话中确认。

## 目标

为 Asterinas 交付第一个可运行的 AppArmor 纵向闭环。进程可以在成功执行 `exec` 后获得具名 profile，在 `fork` 和 `clone` 后保留该 profile；即使普通凭据中的 capability 集原本允许某项操作，AppArmor 仍可拒绝对应的 Linux capability。

M1 同时提供最小 ptrace 隔离和可审计的拒绝记录，但不实现策略加载，也不实现 capability 与 ptrace 之外的资源规则。

## 范围

M1 包含：

- 一个高层内核组件 `aster-apparmor`。
- 支持 enforce 和 complain 模式的不可变 profile。
- POSIX 线程级 AppArmor 标签。
- 成功 `exec` 后，按精确可执行文件路径附加 profile。
- 创建进程和线程时继承标签。
- 通过现有 LSM capability hook 执行 capability 白名单约束。
- 默认允许同 profile ptrace，跨 profile ptrace 需要显式授权。
- 为被拒绝的操作输出结构化内核日志。
- 为策略判断提供单元测试，并为可运行路径提供 initramfs 回归测试。

M1 不包含：

- 用户态策略语法和解析器。
- 运行时策略替换。
- `securityfs` 或其他策略控制文件系统。
- 文件、挂载、网络、信号、IPC、namespace 或资源限制规则。
- Linux AppArmor ABI 兼容。
- 通用审计子系统。

## 命名与位置

Cargo 包命名为 `aster-apparmor`，Rust 代码通过 `aster_apparmor` 引用，目录为 `kernel/core/comps/apparmor`，与现有 workspace 组件路径一致。LSM 模块名保持为 `apparmor`。

这符合 Asterinas 现有约定：Cargo 包使用 `aster-` 前缀，目录和模块使用功能名称。

## 组件模型

`aster-apparmor` 负责策略数据和纯策略判断。它不能依赖 `aster-core`，因为 `aster-core` 会依赖并使用该组件。

组件提供以下概念：

- `Profile`：不可变的名称、模式和 capability 规则。
- `ProfileMode`：`Enforce` 或 `Complain`。
- `CapabilityRules`：以 Linux capability 位图表示的白名单。
- `TaskLabel`：非空的 profile 引用。
- 按解析后的精确可执行文件路径查找 profile。

Capability 标识通过稳定的 Linux 数值跨越组件边界。Asterinas 内部 `Capability` 类型到该数值的转换保留在 core LSM 模块中，从而避免依赖环。

M1 使用编译期静态 profile。表中包含：

- `kernel/default`：初始具名 profile，允许当前支持的全部 capability，保证保留该 profile 的任务维持现有启动行为。
- `apparmor/m1-capability-test`：仅匹配 initramfs 测试程序 `/test/security/lsm/apparmor`，拒绝测试所使用的 capability。

静态表是 M1 的策略来源，不是公开的运行时配置接口。后续策略加载里程碑可以替换查找实现，而无需修改任务标签或 LSM 执行层。

## 标签状态与并发

每个 `PosixThread` 始终拥有一个 `TaskLabel`；不存在 `None`，也不存在隐式 unconfined 状态。标签由现有内核同步原语保护，并引用不可变的静态 profile。

`PosixThreadBuilder` 使用 `kernel/default` 初始化新线程。因此，包括初始任务在内，所有完成构造的任务始终带有标签。

AppArmor task-clone hook 在子任务加入进程任务集合、PID 表或调度器可见状态之前，将父任务标签复制给子任务。线程 clone 和进程 clone 两条路径都必须调用该 hook。

## Exec 转换

Profile 匹配使用原始请求执行文件解析后的绝对路径。加载解释器时，不得用解释器路径替代脚本自身的可执行文件身份。

Exec 转换按以下顺序进行：

1. 按现有 exec 流程解析可执行文件并加载新程序。
2. 在旧进程映像仍可恢复时，保留解析后的可执行文件身份。
3. 执行现有不可逆的 exec 提交。
4. 仅在新进程映像成功安装后提交准备好的 AppArmor 标签。

如果静态 profile 表没有匹配项，任务保留当前标签。这是标签继承，不是回退到 unconfined 标签。因此，受约束任务不能通过执行一个未匹配的二进制文件逃逸约束。

Core 在不可逆边界之前准备可执行文件身份，并在新映像安装成功后调用不可失败的 LSM task-exec 提交 hook。因此，exec 失败时，旧进程映像不会错误地携带新可执行文件的 profile。

## Capability 执行语义

现有 capability LSM 继续负责凭据和 user namespace 的 capability 语义。AppArmor 只施加额外限制，绝不授予 capability。

对于 capability 请求：

1. 现有 capability 检查必须允许该请求。
2. 当前 AppArmor profile 的 capability 白名单也必须允许该请求。
3. 如果 AppArmor 在 enforce 模式下拒绝，返回 `EPERM`。
4. 如果 AppArmor 在 complain 模式下拒绝，输出拒绝记录，但允许请求继续。

最终权限取交集：普通凭据与 AppArmor profile 必须同时授权该操作。

## Ptrace 执行语义

M1 在现有 ptrace LSM 路径中增加最小 AppArmor 检查：

- tracer 与 tracee 使用同一 profile 时，AppArmor 允许跟踪。
- M1 固定拒绝跟踪不同 profile 的任务，不提供可配置 ptrace 规则。
- 对跨 profile 请求，enforce 模式返回 `EPERM`。
- complain 模式记录拒绝，但允许现有 ptrace 检查继续执行。

现有 Yama 和 capability 检查仍然有效。AppArmor 不能覆盖它们的拒绝结果。

## 审计记录

M1 使用现有内核日志器，不新增审计框架。每次 AppArmor 拒绝至少输出一条包含以下字段的结构化 warning：

- `apparmor="DENIED"`
- 操作类型（`capable` 或 `ptrace`）
- profile 名称
- 任务标识
- 请求的 capability 数值或目标任务标识
- profile 模式

Enforce 和 complain 模式都会输出日志。最终结果需要区分该拒绝是实际执行，还是仅被报告。

## LSM 注册

Core 中的 AppArmor LSM 模块实现现有 capability、ptrace hook，以及 clone 和 exec 所需的任务生命周期 hook。

将 `apparmor` 加入已知 LSM 模块列表和默认可选模块列表。具名的宽松默认 profile 保证启用模块不会改变无关的启动行为。现有 `lsm=` 模块选择行为仍然具有最终决定权。

## 错误处理

- Capability 和 ptrace 策略拒绝使用 `EPERM`。
- 可执行文件身份必须在现有不可逆 exec 边界之前准备完成。
- Exec 提交阶段不分配内存且不能失败。
- 如果未来 clone hook 引入可失败操作，失败必须发生在子任务被发布或运行之前。
- 精确路径未匹配 profile 时继承当前 profile。
- 对于超出范围的 capability 数值，纯策略层直接拒绝，不得越界访问位图。

## 测试策略

实现遵循 red-green-refactor。

组件单元测试覆盖：

- 允许和拒绝的 capability 位。
- 超出范围的 capability 标识。
- Enforce 与 complain 的判断结果。
- 固定的同 profile 允许、跨 profile 拒绝 ptrace 判断。
- 精确路径匹配和未匹配路径继承。

位于 `/test/security/lsm/apparmor` 的 initramfs 回归测试覆盖：

- Exec 后附加 profile。
- 限制 profile 下受 capability 保护的操作返回 `EPERM`。
- Fork 后出现相同拒绝，证明标签得到继承。
- Profile 保留的 capability 能够到达底层系统调用，而不是被 AppArmor 拒绝。

仅在必要时扩展现有 LSM 模块选择回归测试，使其识别 `apparmor` 是可选择的可选模块。

## 验收标准

满足以下条件时，M1 完成：

- 启用新组件和 AppArmor LSM 后，内核能够构建。
- 每个 POSIX 任务始终具有具名 AppArmor 标签。
- 测试可执行文件仅在成功 exec 后获得 `apparmor/m1-capability-test`。
- Fork 和 clone 的子任务在运行前继承父任务标签。
- 不允许的 capability 返回 `EPERM` 并输出结构化拒绝记录。
- Complain 模式输出相同记录，但不执行拒绝。
- AppArmor 不会授予被现有凭据检查拒绝的 capability。
- AppArmor 不拒绝同 profile ptrace，但拒绝跨 profile ptrace。
- 现有 LSM 模块选择和无关启动行为保持不变。
- 相关组件测试和 initramfs 回归测试通过。

## 预计修改文件

- `kernel/core/comps/apparmor/Cargo.toml`
- `kernel/core/comps/apparmor/src/lib.rs`
- `kernel/core/src/security/lsm/hooks/mod.rs`
- `kernel/core/src/security/lsm/modules/mod.rs`
- `kernel/core/src/security/lsm/modules/apparmor.rs`
- `kernel/core/src/process/posix_thread/mod.rs`
- `kernel/core/src/process/posix_thread/builder.rs`
- `kernel/core/src/process/clone.rs`
- `kernel/core/src/process/execve.rs`
- `test/initramfs/src/regression/security/lsm/apparmor.c`
- `test/initramfs/src/regression/security/lsm/Makefile`

只有在现有 `aster-apparmor` workspace 引用不完整时，才修改 workspace manifest。
