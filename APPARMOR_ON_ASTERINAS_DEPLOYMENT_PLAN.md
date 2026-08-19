# AppArmor 在 Asterinas（星绽）上的开发与部署规划

> 状态：拟议方案，尚未进入实现。  
> 调研日期：2026-08-19。  
> Asterinas 基线：`main@34a8a2d72829897d8cab3b4cb9be8ceb69dac584`，工作树分析开始时与 `origin/main` 一致。  
> 调研边界：本文没有读取或复用工作区中此前的 AppArmor issue/规划文档；结论来自当前源码和上游官方资料。

## 1. 结论先行

当前星绽不能通过“注册一个 AppArmor 模块”就获得 AppArmor。它现有的是一个很窄的 LSM 调度骨架：只定义了 `capability` 和 `alien_access` 两类 hook，已有模块只有 mandatory capability 与 optional Yama。AppArmor 所依赖的 task/cred 标签、exec 和文件生命周期 hook、路径策略、策略装载 ABI、securityfs、审计、socket 标签等均不存在。

但这项工作是可行的。星绽已经具备可复用的关键落点：per-thread credentials、集中的 exec 主流程、`Path = Mount + Dentry`、文件句柄、mmap backing file、Socket trait、mount namespace、伪文件系统模式，以及 initramfs/QEMU 回归测试链。正确路线不是照搬 Linux 内部对象，而是：

1. 用星绽原生、安全 Rust 实现 AppArmor 的可观察语义；
2. 先做一个不对外宣称兼容的最小机制骨架；
3. 逐步补齐 exec、file、审计与策略控制面；
4. 以“只广告实际实现功能”的 AppArmor feature ABI 接入未修改的 `apparmor_parser`；
5. 先交付 root policy namespace 下、含最小 ptrace 防护的可安装子集，再按需求加入 signal、完整 ptrace、AppArmor mount 规则、network 等功能；不过 legacy mount 的基础 `CAP_SYS_ADMIN` 缺口必须在文件策略之前修复，不能等到 mount 规则阶段。

首个值得做隔离系统试点的子集应包含：

- `lsm=apparmor` 选择、默认 `unconfined`；
- profile/label 生命周期与 fork/clone/exec 继承或转换；
- 用户态 policy load/replace/remove、feature negotiation；
- exec attachment，至少 `ix` 与无 fallback 的 `px`；
- 文件 `r/w/a/m/l/k` 及 create/delete/rename/link/truncate、FD 传递和 mmap 闭环；
- capability 规则；
- 对受约束服务生效的最小 ptrace 双向规则；
- legacy mount/umount/pivot_root 的传统权限基线已收口，且试点服务不得持有 `CAP_SYS_ADMIN`；
- enforce/complain；
- 可检测丢失、结构化、可消费的审计；
- profile/current-label/status 查询；
- pinned parser、initramfs/NixOS 早期装载和失败策略。

在此之前，任何阶段都只能称为“AppArmor 机制骨架”或“AppArmor 语义子集”，不能称为完整 AppArmor。

## 2. 先纠正几个容易误导规划的前提

### 2.1 “已有 LSM，所以主要工作是策略引擎”——不成立

星绽的 `LsmModule` 当前只继承两个 hook trait（[lsm/mod.rs](F:/App_Armor/asterinas/kernel/core/src/security/lsm/mod.rs:33)、[hooks/mod.rs](F:/App_Armor/asterinas/kernel/core/src/security/lsm/hooks/mod.rs:14)）。最初引入 LSM 的提交也明确写着 hook 面暂时只聚焦 ptrace。Linux AppArmor 则注册 cred、task、file、path、mount、exec、socket、procattr、audit 等大量 hook，并声明 cred/file/task/sock 四类安全 blob（[Linux AppArmor lsm.c](https://github.com/torvalds/linux/blob/master/security/apparmor/lsm.c)）。因此本项目首先是在扩展星绽的安全中介面，其次才是写策略匹配器。

### 2.2 “AppArmor 就是路径 allowlist”——不完整

AppArmor 是 task-centered MAC：profile 附着在任务标签上，任务在 exec 时发生 attachment/transition；文件路径只是重要对象之一。它还包含 capability、signal、ptrace、mount、network、Unix socket、rlimit、change_profile、policy namespace 等规则。未命中 profile 的任务默认 unconfined；要产生 DAC 之外的限制，必须从用户态加载 policy（[Linux 内核官方 AppArmor 文档](https://docs.kernel.org/admin-guide/LSM/apparmor.html)）。

### 2.3 “能在 open 时检查路径，就完成了 file confinement”——错误

当前 `O_CREAT` 会先创建 dentry，`O_TRUNC` 也可能在后置检查前产生副作用；文件打开后还存在 read/write、mmap/mprotect、lock、继承 FD、SCM_RIGHTS 传递和 exec 后继续持有 FD 等路径。只加一个 `file_open` 检查会留下可绕过面。现有绝对路径函数还明确记录 deleted/O_TMPFILE 会被错误渲染（[resolver.rs](F:/App_Armor/asterinas/kernel/core/src/fs/vfs/path/resolver.rs:201)）。

### 2.4 “可以整体把 Linux AppArmor 翻译成 Rust”——不建议

Linux 实现与 Linux LSM blob、VFS、RCU、audit、securityfs 和 socket 内部结构深度耦合，而星绽没有这些同构接口。Linux AppArmor 文件标记为 `GPL-2.0-only`，星绽 `kernel/` 当前使用 MPL-2.0 且要求全安全 Rust（[AGENTS.md](F:/App_Armor/asterinas/AGENTS.md:3)）。这不等于本文给出法律结论，但直接复制或近似翻译会触发实质性的许可证与维护者审批风险。建议复现规范、ABI、测试向量和可观察行为，不复制 Linux C 实现；项目开始前仍应得到维护者/法律确认。

## 3. AppArmor 的结构与功能要求

### 3.1 上下游结构

```text
文本 profile
    │
    ▼
apparmor_parser ──按 feature/policy ABI 编译──► 二进制 policy blob/cache
    │
    ▼
/sys/kernel/security/apparmor/{.load,.replace,.remove,features,profiles,...}
    │
    ▼
root/child policy namespace → profile/ruleset → task label
    │
    ▼
LSM hooks（exec/file/capability/task/mount/socket/...）
    │
    ├── allow
    ├── enforce deny
    └── complain allow + audit
```

上游文本策略由 `apparmor_parser` 编译成与具体机器架构无关的二进制策略，内核负责严格验证、解包并安装；load/replace/remove 的标准入口位于 apparmorfs/securityfs（[策略语言手册](https://gitlab.com/apparmor/apparmor/-/raw/master/parser/apparmor.d.pod)、[parser 手册](https://gitlab.com/apparmor/apparmor/-/raw/master/parser/apparmor_parser.pod)、[apparmorfs.c](https://github.com/torvalds/linux/blob/master/security/apparmor/apparmorfs.c)）。

### 3.2 内核核心对象

- **Policy namespace**：拥有 unconfined profile、profile 集合、层级可见性和 revision；它不是 Linux user/mount namespace 的别名（[policy_ns.h](https://github.com/torvalds/linux/blob/master/security/apparmor/include/policy_ns.h)）。
- **Profile/ruleset**：profile 名、attachment、mode、路径标志、规则 DFA/accept table、capability 和各安全类规则（[policy.h](https://github.com/torvalds/linux/blob/master/security/apparmor/include/policy.h)）。file 不是只有路径和权限位：同一 DFA state 还会依据 object UID 选择 owner/non-owner permissions，hard-link 需要同时检查 source/target 与 permission subset（[file.c](https://raw.githubusercontent.com/torvalds/linux/master/security/apparmor/file.c)）。
- **Label**：任务的安全身份；上游还支持 compound/stacked label、stale label 与 profile replacement（[label.h](https://github.com/torvalds/linux/blob/master/security/apparmor/include/label.h)）。首版星绽不需要照搬全部复杂度。
- **Object context**：至少 file/socket 等对象要保存创建者或打开时的安全上下文，否则 exec transition、继承和 FD/peer 传递会绕过当前 profile。
- **Decision/audit**：所有 hook 都应走同一原始规则判定和模式处理路径，但两者不能合并成一个简单三态。Complain 只把上游定义为可学习的 policy denial 转成“允许并产生 ALLOWED 审计”；explicit deny、no_new_privs、ABI/输入错误等并不因此统一放行。具体分类必须以 pinned 上游行为为准（[audit.c](https://github.com/torvalds/linux/blob/master/security/apparmor/audit.c)、[策略语言手册](https://gitlab.com/apparmor/apparmor/-/raw/master/parser/apparmor.d.pod)）。
- **Atomic policy update**：错误 blob 必须整体拒绝；replace/remove 时，正在运行的 task、已打开 file 和 socket 不能悬空或混用半个 generation。

### 3.3 功能层级

| 层级 | 功能 | 是否属于首个可安装版本 |
|---|---|---:|
| 基础机制 | module selection、unconfined、label/profile、enforce/complain、统一 decision | 是 |
| 用户态 ABI | features、load/replace/remove、profiles/current、严格 blob 校验 | 是 |
| Domain | exec attachment、inherit/transition、no_new_privs/unsafe-exec 约束 | 是，先做受限子集 |
| File | 路径匹配、open/permission/receive、mutation、mmap/mprotect/lock | 是 |
| Capability | profile 对已通过 common capability 的操作再收窄 | 是 |
| Task | 最小 ptrace；随后 signal、完整 ptrace、rlimit | ptrace 最小闭环进入首个安全试点，其余下一版本 |
| AppArmor Mount | mount/remount/bind/move/umount/pivot_root 规则 | 后续版本；传统 `CAP_SYS_ADMIN` 基线必须先完成 |
| Network | family/type/protocol、bind/connect/listen/accept/send/recv | 后续版本 |
| Unix peer | 本端/对端地址、peer label、双向 send/receive | 后续版本，成本高 |
| 高级 domain | `cx/ux`、fallback、change_profile/onexec、hats、stacking | 按需求追加 |
| 高级平台 | 层级 policy namespace、userns、DBus、mqueue、io_uring、prompt | 当前暂缓或被底层能力阻塞 |

### 3.4 ABI 与版本要求

内核必须通过 `features/` 诚实暴露支持集，parser 应针对该 feature ABI 编译。这里有一个边界：parser 可能依据 feature ABI 改写、降级或省略源码规则，内核只能验证收到的 blob，无法证明源码里没有被丢弃的语义。因此部署链还必须固定 parser 参数、policy ABI 和警告即失败策略，并做“源码 → parser → blob → load”的负向测试。策略二进制本身是非可信输入，长度、偏移、版本、DFA 状态和转换表都要 fail closed，并配合 fuzz（[policy_unpack.c](https://github.com/torvalds/linux/blob/master/security/apparmor/policy_unpack.c)）。

截至 2026-08-19，上游最新标签是 5.0.2；但上游明确说明 5.0 是通向 5.1 的短生命周期 bridge release。4.1.7 是同年发布的 4.x 用户态 bugfix 版本（[官方 releases](https://gitlab.com/apparmor/apparmor/-/releases)、[官方 tags](https://gitlab.com/apparmor/apparmor/-/tags)）。因此本文建议：

- M0 先用 4.1.7 做首个兼容性 spike，并固定 tag/commit；
- 同时记录 5.0.2 的差异，不把“latest”写成兼容承诺；
- 到真正进入发行版前重新评估 5.1；
- 内核只接受经测试的 policy blob 版本，未知版本明确拒绝。

这是工程建议，不是已验证的最终版本选择；M0 的未修改 parser 互操作测试才是最终依据。

## 4. 星绽当前结构与可复用基础设施

### 4.1 LSM 与启动

- `security::init()` 在基础 FS 初始化后调用（[init.rs](F:/App_Armor/asterinas/kernel/core/src/init.rs:46)）。
- capability 是 mandatory；Yama 是默认 optional；支持 Linux 风格 `lsm=`、legacy `security=`、顺序、去重和 exclusive module（[modules/mod.rs](F:/App_Armor/asterinas/kernel/core/src/security/lsm/modules/mod.rs:39)）。
- `LsmFlags::LEGACY_MAJOR | EXCLUSIVE` 已可表示 AppArmor 的 major/exclusive 属性。
- hook dispatcher 顺序调用模块并在第一个错误处短路，适合 capability 先验证持有权限、AppArmor 再收窄。
- 但只有 `on_capable` 和 `on_alien_access` 两种 hook；全仓没有 AppArmor 模块。

值得注意的是，`on_capable` 也不是所有 capability 决策的统一入口。UID/GID/securebits 等凭据代码仍有多处直接读取 effective capset（[credentials_.rs](F:/App_Armor/asterinas/kernel/core/src/process/credentials/credentials_.rs:146)）。因此 capability 功能开发前必须完成全部调用点审计和路由收口。

### 4.2 Task、credentials 与 exec

- `PosixThread` 持有 per-thread credentials（[posix_thread/mod.rs](F:/App_Armor/asterinas/kernel/core/src/process/posix_thread/mod.rs:50)）。
- credentials 当前只有 UID/GID、groups、capsets、securebits、no_new_privs；`ExecCred` 也没有 LSM label（[credentials_.rs](F:/App_Armor/asterinas/kernel/core/src/process/credentials/credentials_.rs:17)）。
- fork/clone 的两条主要分支都通过 `Credentials::new_from` 深复制，手工 `Clone` 是天然的 label 继承点（[clone.rs 第一处分支](F:/App_Armor/asterinas/kernel/core/src/process/clone.rs:441)、[第二处分支](F:/App_Armor/asterinas/kernel/core/src/process/clone.rs:577)、[credentials_.rs](F:/App_Armor/asterinas/kernel/core/src/process/credentials/credentials_.rs:766)）。
- `do_execve` 有清楚的解析、prepare、point-of-no-return 和 commit 边界（[execve.rs](F:/App_Armor/asterinas/kernel/core/src/process/execve.rs:36)）。
- 首个 init task 走独立装载路径，绕过普通 `do_execve`（[init_proc.rs](F:/App_Armor/asterinas/kernel/core/src/process/process/init_proc.rs:95)）。v1 明确让 PID 1/早期 loader 保持 unconfined，只要求初始化出有效 label；策略必须在受保护服务第一次 exec 前载入。若未来要求约束 PID 1，需另选内核预装 policy 或两阶段 init/re-exec，不能假装普通 attachment 已覆盖它。
- shebang 递归最终转成 ELF；要做正确 transition，需保留原请求文件、脚本和解释器链，不能只看最终 ELF。
- 当前 `NOEXEC` 没有成为 exec 强制条件，不能拿它当 AppArmor exec 基础。

结论：label 应直接进入 credentials 生命周期。首版不要先造一个支持任意 LSM blob 的工厂；在第二个确实需要 cred blob 的 LSM 出现前，最小的 AppArmor label 字段更易审计。

### 4.3 VFS 与路径

- `Path` 保存 mount 与 dentry，是对象基础；subject-visible/chroot-relative 视图还必须结合 `PathResolver` 的 root/cwd，hook 只拿 `Path` 不足以生成 policy path（[path/mod.rs](F:/App_Armor/asterinas/kernel/core/src/fs/vfs/path/mod.rs:42)、[resolver.rs](F:/App_Armor/asterinas/kernel/core/src/fs/vfs/path/resolver.rs:133)）。
- `Inode::check_permission` 集中实现 DAC，但没有 MAC hook（[inode.rs](F:/App_Armor/asterinas/kernel/core/src/fs/vfs/fs_apis/inode.rs:577)）。
- `InodeHandle::new` 做打开时 DAC；随后 read/write 只依赖已保存的 access mode，没有 LSM recheck（[inode_handle.rs](F:/App_Armor/asterinas/kernel/core/src/fs/file/inode_handle.rs:38)）。
- create、delete、rename、link、chmod/chown、truncate、mount 等操作多数有集中实现点，具备放置“副作用前 hook”的条件。
- 现有 inode 两个通用 extension slot 已被事件与锁上下文使用，不能假设有空闲 security slot。
- 路径显示 API对 deleted/O_TMPFILE 存在已知错误；安全匹配 API必须显式表示 reachable/disconnected/deleted，而不是直接复用展示字符串。

### 4.4 Signal、ptrace、mount 与 network

- `alien_access` 已集中覆盖 ptrace/proc mem/maps/pidfd_getfd 等路径，context 有 accessor、target 和 read/attach mode；这是最接近可直接扩展的 task hook。但当前 ptrace LSM 判断只发生在建立 trace 关系时，后续 PEEK/POKE/SETREGS/CONT 复用既有关系；若 tracer 在目标 exec/replace 前已经附着，单纯阻断新 attach 仍可绕过新 profile。
- signal 权限函数在 self、相同 UID、SIGCONT session 等条件下提前返回；AppArmor hook 必须作为独立合取检查放在这些 DAC/传统权限结果之后，不能只放进 `CAP_KILL` fallback（[kill.rs](F:/App_Armor/asterinas/kernel/core/src/process/kill.rs:165)）。
- mount/remount/bind/move/umount/pivot_root 操作集中，但没有相应 LSM hook。更严重的是 legacy mount/umount/pivot_root 路径未见 `CAP_SYS_ADMIN` 检查，而现代 mount API 才调用该 capability（[mount.rs](F:/App_Armor/asterinas/kernel/core/src/syscall/mount.rs:17)、[fsopen.rs](F:/App_Armor/asterinas/kernel/core/src/syscall/fsopen.rs:39)）。这是 AppArmor mount 之前要先处理的基础权限缺口。
- Socket trait 集中了 bind/connect/listen/accept/send/recv，AF_UNIX 地址也已建模；但没有 socket creator/peer label、统一 socket security context、net namespace 或 socket LSM hook。
- Unix socket 已支持 SCM_RIGHTS，这同时意味着 file confinement 必须考虑 `file_receive`，不能只检查打开者。

### 4.5 控制面、审计、用户态与测试

- 全仓没有 securityfs、apparmorfs、policy load/replace/remove、`/proc/.../attr/current` 或 AppArmor audit。
- 当前 logger 面向 console；没有可检测丢失的 audit ring、丢失计数、audit netlink 或可持久消费接口。源码只有标准 netlink protocol 的 `AUDIT = 9` 枚举值，这不等于 audit family 可用。
- Yama 的 `/proc/sys/kernel/yama/ptrace_scope` 证明伪文件读写模式可复用，但不等于 AppArmor securityfs ABI 已有基础。
- initramfs 启动脚本挂载 sysfs/proc/cgroup2/configfs，尚未挂载 securityfs（[test init](F:/App_Armor/asterinas/test/initramfs/src/init:22)）。
- security regression 已覆盖 capability、module selection、Yama 和 namespace，适合新增 AppArmor 用例（[run_test.sh](F:/App_Armor/asterinas/test/initramfs/src/regression/security/run_test.sh:7)）。
- 可复用验证命令包括 `make check`、`make test`、`make ktest`、`make run_kernel AUTO_TEST=regression`（[AGENTS.md](F:/App_Armor/asterinas/AGENTS.md:30)）。

## 5. 功能可行性矩阵

这里的“可行”表示在补齐所列基础设施后能在当前架构上原生实现，不表示现在已经支持。

| 功能 | 当前基础 | 判断 | 最早阶段 | 关键条件/边界 |
|---|---|---|---|---|
| 模块选择 | `lsm=`、`security=`、exclusive flags 已有 | 直接可做 | M1 | AppArmor opt-in；capability 仍 mandatory |
| unconfined/profile/label | credentials 生命周期可复用 | 可做 | M1-M2 | label 随 clone；init 特殊路径；先只支持单 profile label |
| enforce/complain | 无统一 decision/audit | 可做 | M2 | 必须与结构化审计一起交付 |
| capability | hook/context 已有但覆盖不全 | 优先做 MVP | M2 | 先审计并收口直接 capset 旁路；补 capget 语义 |
| exec attachment/`ix` | exec prepare/commit 清楚 | 可做，中高成本 | M3A-M3B | 先有 subject-visible 安全路径，再处理 script/interpreter、no_new_privs、FD 语义 |
| `px/cx/ux`、fallback | 缺 onexec/previous/secure-exec | 可做但后移 | M5B/M7 | 首个版本只广告实际实现的 transition |
| 基础 file MAC | Path/VFS 集中但无 hook | 可做，高成本 | M3A-M4B | 安全路径 API、owner/non-owner、hard-link pair/subset、副作用前 hook、DFA/权限映射；缺一则不广告 file |
| file FD/mmap | file 与 mapping 基础已有 | 必须与 file 同闭环 | M4B | 通用 receive hook、file context、read/write、mmap/mprotect/lock；先冻结上游旧 FD 语义 |
| ptrace | alien-access hook 已有，但既有 trace 关系的后续命令不重检 | 中等成本；安全试点前必须有生命周期闭环 | M3B/M5B | 覆盖 attach-before-exec、TRACEME、后续命令及 replace；完整规则后续补 |
| signal | 中央权限函数已有 | 可做，中等成本 | M7 | 不能被 self/UID 早返回绕过；send/receive 双向 |
| rlimit | rlimit 代码集中 | 可做，中等成本 | M7 | 增加 task_setrlimit 类 hook |
| policy load/replace/remove | 完全缺失 | 可做，高安全风险 | M5A | securityfs、严格 decoder、原子 transaction、权限检查 |
| features/profiles/current | 完全缺失 | 可做 | M5A-M5B | feature 必须诚实；current label 接口与权限 |
| 可检测丢失的 audit transport | 只有 console | 可做但属新基础设施 | M5B | sequence、lost counter、rate limit、用户态 consumer |
| mount/pivot_root | VFS 操作集中；legacy 权限基线缺失 | 基线修复可直接做，AppArmor 规则高成本 | M2 基线/M8 规则 | M2 先补 `CAP_SYS_ADMIN` 并禁止试点服务持有该能力；M8 再做所有变更前 hook |
| 基础 network | Socket trait/地址已有 | 可做，高成本 | M9 | socket label、操作 context、审计；无 netns 限制语义 |
| Unix peer mediation | AF_UNIX/SCM_RIGHTS 已有 | 可做，成本很高 | M9 | peer label、accept/clone、双向规则、SO_PEERSEC 类接口 |
| change_profile/onexec | procattr 与 task state 缺失 | 后续可做 | M7+ | previous/onexec、transition 权限、libapparmor 接口 |
| root policy namespace | 可作为 AppArmor 内部对象实现 | 可做 | M2 | 不放进 `NsProxy` |
| 层级 namespace/stacking | user namespace 仅 initial；无 compound label | 暂缓 | M10 | 容器委派、可见性、label stack 都要先成熟 |
| DBus | 内核不可直接观察完整消息语义 | 当前不作为内核交付 | M10 | 需要 D-Bus daemon/libapparmor 协作 |
| mqueue | 未见 POSIX mqueue 子系统 | 当前阻塞 | M10 | 先有底层对象与 syscall |
| io_uring | 未见可用 io_uring 子系统 | 当前阻塞 | M10 | 先实现底层功能与对象生命周期 |
| userns rule | 仅 initial user namespace | 当前阻塞 | M10 | 先完成 user namespace 创建/所有权/祖先语义 |
| prompt/kill/default_allow 等模式 | 无通知/会话/可消费 audit | 暂缓 | M10 | 有真实产品需求再做 |

## 6. 三条实现路线与选择

### 路线 A：星绽原生 Rust 语义实现 + 官方 ABI 子集（推荐）

围绕星绽现有 `Credentials`、`Path`、`InodeHandle`、`ProgramToLoad`、`VmMapping`、`Socket` 和 mount 拓扑实现 typed hook context；内核策略对象原生 Rust；对外只实现并广告一个经过验证的 AppArmor feature ABI 子集。

优点：安全 Rust、符合星绽对象生命周期、可逐步验收、长期维护成本最低。缺点：policy blob decoder、DFA 语义和路径边界仍是大工程。

### 路线 B：自定义 AppArmor-like 文本/loader（仅允许作一次性测试夹具）

它能更快展示 deny，但不能使用标准 parser/profile 生态，也容易形成第二套永远删不掉的 policy 语言。M1-M4B 可以在 ktest/regression 构建中直接构造只读 `ProfileSpec`，但不得暴露为用户 ABI，也不得创建假的 `features/`。M5B 到来后删除该测试注入路径。

### 路线 C：整体移植 Linux `security/apparmor`（不推荐）

星绽与 Linux 的对象、hook、RCU、audit 和 FS 控制面不相容；上游数十个、且随配置变化的 hook 相对当前两个 hook 的差距说明“机械翻译”不会减少核心工作。它还引入 GPL-2.0-only 与现有 MPL-2.0 仓库政策的审批风险。

## 7. 渐进开发计划

依赖主线如下：

```text
M0 契约冻结
  ↓
M1 可启动骨架
  ↓
M2 task/profile/capability 机制闭环 + legacy mount 权限基线
  ↓
M3A 安全路径/matcher 契约 → M3B exec domain
  ↓
M4A 安全路径/pre-hook → M4B file/FD/mmap 闭环
  ↓
M5A blob/securityfs → M5B parser/audit/procattr/ptrace
  ↓
M6 打包、早期装载与隔离实验性试点
  ├── M7 task/扩展 domain 子集
  ├── M8 mount
  └── M9 network/Unix
          ↓
        M10 高级功能（按真实需求）
```

工期是规划推测，不是已验证事实。以下阶段数字统一使用人周；日历估算另按“2 名内核开发者 + 1 名用户态/测试开发者、平均 2.0-2.5 FTE 有效并行、另有安全评审”换算。安全路径→exec/file→decoder→启动编排存在串行关键路径，不能简单用人周除以三。

### M0：兼容契约、威胁边界与基线测试（2-4 人周）

**目标**：先固定“实现什么”，避免边写内核边追逐上游 moving target。

**工作**：

- 固定 Asterinas commit、Linux AppArmor 参考 commit、parser tag/commit；首个 spike 候选为 AppArmor 4.1.7。
- 建立一个 pinned Linux AppArmor oracle VM/container：相同 policy source、parser 参数和 workload 同时跑 Linux 与 Asterinas，比较 syscall 结果/errno、current label、replace/remove、旧 FD、ptrace 生命周期与审计语义字段。
- 用未修改 parser 对一个最小 capability/file profile 生成 blob，记录 binary policy 版本、feature 文件和 cache 行为。
- 在当前 Asterinas NixOS 用户态实际运行 pinned parser 的离线编译路径，核实动态库、文件系统探测和 syscall 依赖。若不能运行，v1 必须明确采用构建机预编译 cache；不能把运行时编译留成未验证分支。
- 追踪 pinned parser/libapparmor 对 securityfs 与 procattr/selfattr 的实际探测顺序，M0 单选一个精确兼容接口；后续里程碑不保留“二选一”。
- 固定 parser 参数、policy ABI、feature 文件和“任何警告均使构建失败”的规则；加入源码含未支持 class 时 parser 必须非零退出且旧 policy 不变的端到端负向样例。
- 明确首版 feature manifest：root namespace、capability、domain attachment、受限 exec、file、最小 ptrace、enforce/complain；其他均不广告。
- 为每个计划广告的 feature/permission/transition、同一 class 内 qualifier 及非法组合生成 golden blob corpus；file corpus 必须覆盖 owner/non-owner accept entry、source/target hard-link pair 与 permission subset。一个最小 profile 只用于冒烟，不能作为 ABI 冻结依据。
- 验证 feature 位的真实粒度能否表达这个子集。若一个 advertised 位同时允许 parser 生成未实现语义，M0 不能通过：要么实现整个 feature group，要么选择更低 policy ABI，要么取消“未修改 parser 兼容”目标。
- 写出首个试点的攻击者模型；至少把“同 UID 或传统 Yama 仍允许的 tracer 接管受约束服务”、attach-before-exec、`PTRACE_TRACEME`、profile replace 后的既有 trace 关系，以及利用 legacy bind/move mount 改写路径视图列为必须阻断的路径。
- 明确许可证策略：原生实现、允许参考的规范/测试、禁止直接复制的代码边界。
- 完成 security-sensitive call-site 清单：capability 旁路、legacy mount 权限、NOEXEC、signal 早返回、create/truncate 副作用点。
- 为现有行为留下基线回归与性能数据；测量 open/read/exec/capability 的 p50/p99、分配、锁竞争和内存后，冻结各阶段预算，不先拍数字。
- 规定主线同步策略：每个里程碑开始和结束都在新的 Asterinas main 基线上重跑源码清单与回归；发行时重新固定最终 commit/feature matrix，而不是让 M0 commit 冻结数月不动。

**验收**：有一份冻结的兼容矩阵和 golden blob corpus；pinned parser 的 Asterinas 运行或构建机预编译路线已经单选并实测；未知 blob 规则/版本由内核 fail load，源码中未支持规则由固定 parser pipeline fail build；上述基础缺口有明确 owner，不存在“以后再看”的绕过项。许可证/维护者批准是无固定 SLA 的 M0 exit dependency，不计入 2-4 人周。M0 结束时依据 decoder spike 重估后续阶段，而不是沿用初始工期。

### M1：最小可启动骨架（2-3 人周）

**目标**：`lsm=apparmor` 能启动，所有任务默认 unconfined，系统行为不变；这是最小 runnable skeleton，不是安全交付。

**主要改动位置**：

- [modules/mod.rs](F:/App_Armor/asterinas/kernel/core/src/security/lsm/modules/mod.rs:31)：注册 `AppArmorLsm`，flags 为 `LEGACY_MAJOR | EXCLUSIVE`。
- 为 active LSM 增加一次性的显式初始化生命周期（最小可为 `LsmModule::init()` 默认空实现），由 `security::init()` 调用；AppArmor 在此创建 root store。不能只靠当前“打印模块名”的 init，也不能把安全初始化隐含在任意首次访问的 lazy 路径。
- `kernel/core/src/security/lsm/modules/apparmor/`：只放当前需要的 `label/profile/decision/audit`，不预建 network/mount 空壳。
- [credentials_.rs](F:/App_Armor/asterinas/kernel/core/src/process/credentials/credentials_.rs:17)：加入最小 `AppArmorLabel`，默认 unconfined，并在手工 `Clone` 中继承。
- [init_proc.rs](F:/App_Armor/asterinas/kernel/core/src/process/process/init_proc.rs:95)：显式初始化 init label。
- `test/initramfs/src/regression/security/lsm/`：模块选择与 unconfined 回归。

**最小设计**：

- label 暂时只有 `Unconfined | ProfileId`；不做 compound/stacked label。
- 定义 `SubjectLabel` 对 POSIX task 与 kernel/system task 的行为。v1 可让 system task 显式使用 `SystemUnconfined`，但任何“没有 current POSIX task”的路径都必须得到这个显式主体，不能因 `None` 自动跳过 hook或 `unwrap`。
- 一个 root store；不加入通用 LSM blob registry、factory 或动态模块。
- 原始规则判定返回原因明确的 allow/deny；独立 mode 层只转换上游允许 complain 放行的 policy denial。explicit deny、输入/ABI 错误和 no_new_privs 等不得落入统一的 complain-allow 分支；unconfined 走极短路径。
- 正式构建没有自定义 policy 语言，也没有内置强制 profile；ktest 可直接构造只读 profile fixture。
- 初始 PID 1 credential 另持有一个不可伪造、fork/clone 不继承、仅同一 bootstrap task 跨 exec 保留的一次性 `PolicyBootstrapAuthority`。它不是 label，也不能由 UID/capability 重新获得；M6 freeze 成功后立即销毁。不要把任意 `unconfined + CAP_MAC_ADMIN` task 当作 loader 身份。

**验收**：

- `lsm=apparmor` 日志显示 capability + AppArmor；`lsm=yama,apparmor` 可共存。
- 未加载 policy 时回归测试与基线一致。
- fork/clone 的 label 继承有 ktest。
- 初始 PID 1 的 bootstrap authority 跨其 bootstrap exec 保留，但 fork/clone 子任务拿不到；没有用户态接口可以重新铸造。
- kernel task 发起的 VFS/exec 辅助操作有 system-label 测试，不存在“缺 task 即静默放行”的隐式分支。
- 不出现伪造的 `/sys/kernel/security/apparmor/features`。

### M2：task/profile 核心、capability MVP 与 mount 权限基线（4-6 人周）

**目标**：证明 label → rule → decision → mode → audit → syscall result 的完整链路。

**工作**：

- 建立 root policy store、monotonic `ProfileId`、policy revision 与原子 snapshot replacement。
- 首版用一个全局 `RwLock<Arc<PolicySnapshot>>` 保证正确性；label 保存不复用的 ID。只有基准证明它是瓶颈时才换 lock-free/RCU 风格读取。
- 在写代码前冻结更新语义：replace 保留 ID，使运行任务的后续决策立即使用新 profile；remove 使引用该 ID 的任务变为 unconfined，并产生高优先级审计；同名重新 load 分配新 ID，不会重新捕获已 unconfined 的任务。这与上游可观察语义保持一致，并分别覆盖 task、已打开 file 和未来 socket context。
- 审计并收口所有 security-relevant 直接 capability 判断；凡 AppArmor capability feature 覆盖的路径都必须统一经过 hook。只要仍存在无法表达的旁路，就不得在 v1 广告 capability feature，也不得把 capability 写入产品声明。
- 增加 capget 类 hook/过滤，使读取其他 task 的 effective/permitted 集合也按 profile 收窄；当前直接读取目标 capset 的 [capget.rs](F:/App_Armor/asterinas/kernel/core/src/syscall/capget.rs:44) 必须有回归。
- capability AppArmor rule 在 mandatory capability 通过后再收窄。
- 独立修复 legacy mount/umount/pivot_root 的 `CAP_SYS_ADMIN` 检查，并覆盖 bind/move/remount；这是传统权限基线，不广告为 AppArmor mount mediation。M8 之前，任何试点 required service profile 都不得授予 `CAP_SYS_ADMIN`。
- enforce 对缺少 allow 返回 `EACCES/EPERM`；complain 只对该类可学习 denial 放行并产生稳定 `ALLOWED`。另有 explicit deny 用例验证 pinned 上游定义的非统一转换行为。
- 初期 audit 用固定字段 console 记录，仅作为机制测试；字段至少含 operation/profile/class/requested/pid/comm/error/mode/revision。
- ktest/regression 构建可注入一份只读 profile fixture：允许 `CAP_SETGID`，但不授予 `CAP_SYS_CHROOT`；另建 explicit-deny fixture，不混淆两种语义。

**验收**：

- enforce 下 `setgroups()` 成功，`chroot("/")` 在修改 root 前失败并恰好产生一条 `DENIED`。
- complain 下“缺少 allow”的同一 chroot 成功并产生 `ALLOWED`；explicit deny、no_new_privs 和无效输入各按冻结的上游行为验收，不能因 complain 全部成功。
- DAC 已拒绝的操作不会被 AppArmor allow 提权；AppArmor 只会进一步收窄。
- SETUID/SETGID/SETPCAP 等原直接 capset 路径和 capget 目标读取均有 allow/deny 测试；未全部收口则 M2 不通过。
- 无 `CAP_SYS_ADMIN` 的任务不能经 legacy mount/umount/pivot_root 改变拓扑；拒绝发生在副作用前。该回归未通过则不得进入路径/file 阶段。
- invalid profile snapshot 不会替换当前 generation。

此阶段完成后只能称为“AppArmor 语义机制 MVP”。

### M3A：subject-visible 安全路径与 matcher 契约（5-7 人周）

**目标**：先给 exec attachment 和后续 file rule 一个可信、并发语义明确的共同输入；完成后强制重估 M3B/M4A。

**工作**：

- 安全路径 context 同时携带 `Path` 与主体的 `PathResolver` root/cwd 视图，并携带规则选择所需的 subject fsuid 与 object UID，明确 mount namespace、chroot、reachable/disconnected/deleted。
- 定义 rename/mount topology 下的锁或 generation snapshot 契约；不能在逐级回溯时拼出跨 generation 路径。
- 用 pinned parser 在外部生成 matcher/DFA 测试向量，再以版本化、测试专用 fixture 注入 ktest；向量包括 owner/non-owner accept entry 与 hard-link source/target/subset。这只验证 matcher 语义，不冒充 M5A 的真实 blob decoder。
- 在 M0 冻结的 feature tree 上验证粒度：若某个 feature 位会让 parser 生成尚未实现的 transition/permission，就必须实现该整组、选择更低 ABI，或放弃“未修改 parser 兼容”声明，不能靠文档拆细。

**验收**：同一对象在 chroot/mount namespace/rename 并发下得到确定结果；deleted/O_TMPFILE 不误匹配普通路径；owner/non-owner 分支不会因漏传 UID 退化为普通 allow；matcher 对 pinned Linux AppArmor 测试向量给出相同 accept/deny。

### M3B：exec domain、label transition 与既有 tracer 门禁（5-7 人周）

**目标**：从“所有测试任务手工带 label”进化到由可执行文件 attachment 驱动的 task-centered MAC。

**工作**：

- 新增 typed exec prepare/commit hook；decision 必须在 point-of-no-return 之前完成，label 只在成功 commit 时切换。
- 扩展 exec 凭据准备结果，使待提交 label 与 UID/GID/capability 变换同一生命周期提交。
- 保留原请求 executable、脚本、解释器链和最终 ELF 的区分。
- 首期实现 attachment 与 `ix`；在进入可安装版本前补无 fallback `px`。`cx/ux`、fallback、大写 secure-exec 后移。
- 定义 no_new_privs、setuid/file-capability exec 与 profile transition 的组合规则；no_new_privs 下只允许经证明不扩大权限的 transition。
- PID 1/早期 loader 在 v1 明确保持 unconfined；独立 init 路径只负责产生有效初始 label。受保护服务必须在 policy load 后首次 exec 才 attachment。
- exec 准备阶段检查既有 tracer/tracee 关系：在新 label 下不被允许时，按 M0 冻结的上游语义拒绝 transition 或安全解除关系；不能先切换 profile 再让旧 tracer 继续 POKE/SETREGS。`PTRACE_TRACEME` 与 attach-before-exec 必须进入同一差分矩阵。
- 先建立上游差分矩阵，再定义 exec 后旧 FD 在 read/write/mmap/ioctl/SCM_RIGHTS 等操作上的兼容行为；不要把“即时撤销所有旧 FD”误称为标准 AppArmor。

**验收**：

- 无匹配 profile 仍是 unconfined。
- ELF attachment、fork 继承、exec transition、失败 exec 不换 label均通过；label 与 setuid/file-cap credential 必须原子提交。
- attachment best-match 不确定时拒绝 policy/exec；无 fallback `px` 的目标 profile 不存在时拒绝；no_new_privs 不得通过 transition 获得权限超集。
- shebang：脚本与解释器组合测试不会错误地只按最终 ELF 决策。
- 冷启动验证 PID 1/loader 是明确的 unconfined，而目标服务在首次 exec 前 policy 已加载并正确 attachment。
- tracer 在 exec 前已附着或目标调用 `PTRACE_TRACEME` 时，未经新旧双方规则许可不能完成可被旧 tracer 控制的受限 transition。
- `/proc` 或内核测试接口能观察 current label；标准 procattr 留到 M5B。

### M4A：file mutation 的副作用前 hook（5-7 人周）

**目标**：在 M3A 安全路径契约上覆盖 file mutation，并先证明 hook 位置正确，再扩展 file 对象生命周期；完成后重新估算 M4B。

**工作**：

- 将 M3A context 扩展到 parent/basename、source/destination、subject fsuid/object UID 等 mutation 输入；修复或隔离现有 O_TMPFILE/deleted 错误。
- hook 必须放在副作用前：create、truncate、unlink、rename、link、chmod/chown、mknod 等不能先改 VFS 后检查。
- 接入最小的编译后 DFA/规则结构和 permission 映射；owner/non-owner 选择和 hard-link 的 source/target pair + permission subset 必须按 pinned 上游语义实现。不要在内核解析 AppArmor 文本。
- 覆盖 mount namespace/chroot、symlink/hardlink/rename、bind mount、deleted file、O_TMPFILE。

**验收**：

- 有“DAC 允许但 MAC 拒绝”的 open/create/truncate/rename 端到端用例。
- create/truncate 被拒绝时没有残留对象或数据变化。
- owner/non-owner 的 allow/deny 结果与 object UID 一致；hard-link 只有 source/target 与 subset 检查全部通过才产生副作用。
- disconnected/deleted 语义要么按已广告 flag 正确处理，要么 fail closed；不能错误匹配普通路径。
- 路径原型和 rename 并发测试完成后，重新估算 M4B 与 M5A，不把初始区间当承诺。

### M4B：file 对象、旧 FD 与 mmap 闭环（9-13 人周）

**目标**：形成第一个真正有安全价值的文件资源中介闭环，同时明确哪些旧资源语义兼容上游、哪些是星绽扩展。

**工作**：

- 实现 file open、permission、lock、mmap、mprotect；在 `InodeHandle`/其 common state 保存最小 open context。
- `file_receive` hook 放在通用 FD 安装前，对所有 `FileLike` 调用；inode-backed file 执行路径规则，socket/匿名/伪文件可按当前已广告 class 选择 no-op 或专用规则，但调用点不能缺失。
- 映射 exec、read/write/append、memory-map-exec、link/lock 的 permission 位与 policy accept table，并保留 owner/non-owner accept entry；decoder 或 matcher 无法表达其中任一项时，整个 file feature 不得广告。
- 针对 profile transition、profile replacement、fork 继承和 SCM_RIGHTS，逐项记录 pinned Linux AppArmor 在 read/write/mmap/ioctl/receive 上的行为。兼容上游“已打开资源可能保留部分访问”的地方不得宣传即时撤销；若选择更强语义，标成 Asterinas 扩展并做兼容开关/说明。
- unconfined fast path 不分配内存；confined path 的路径构造、分配和锁竞争遵守 M0/M3A 实测后冻结的预算。普通 allow 默认不打 audit，除非策略要求。

**验收**：

- r/w/x/append/link/lock/mmap/mprotect 有 DAC-allow/MAC-deny 闭环。
- open-before-transition、profile replace、forked FD、SCM_RIGHTS 对每种已广告操作都有明确且与兼容矩阵一致的结果；验收目标是“没有缺失的中介调用点和未声明行为”，不是笼统承诺撤销所有旧 FD。
- 通用 FD receive 测试证明 socket、匿名 file 和伪文件也经过 hook dispatcher，即使当前规则结果是明确 no-op。
- 以小/中/大 policy corpus 跑 open/read/mmap microbench，p50/p99、内存和锁竞争均在冻结预算内；否则不进入 decoder/control-plane 阶段。

### M5A：policy blob decoder 与 securityfs transaction（8-12 人周）

**目标**：建立最小标准控制面和可严格验证的 policy transaction；decoder 原型完成后再次重估 M5B。

**工作**：

- 实现满足所需节点的最小 securityfs，不扩展成无需求的通用配置框架。
- 实现 `/sys/kernel/security/apparmor/{features,profiles,.load,.replace,.remove}` 及 policy revision；权限至少由适当的 MAC/admin capability 和 AppArmor policy 管理规则共同保护。
- 冻结 bootstrap 授权：首次装载要求初始 PID 1 持有 M1 的一次性 `PolicyBootstrapAuthority`，同时通过 `CAP_MAC_ADMIN`；不能用“任意 unconfined root”代替身份。root policy 生效后，更新同时通过 capability 与 policy-management 规则。实现一个显式、单向、仅重启可解除的 policy freeze；freeze 后 `.load/.replace/.remove` 全部拒绝并销毁 bootstrap authority。freeze、维护重启和 emergency boot 路径必须各有测试。
- 只实现 M0 冻结、由 pinned parser 产生的 blob 版本；未知版本、未知 blob class、越界字段、非法 DFA 全部拒绝。
- policy load/replace/remove 是 transaction：失败不改变旧 snapshot；replace 保留 ID，remove 使旧 label 变为 unconfined，同名 reload 使用新 ID。
- 保留上游 remove 后旧 task 变 unconfined 的兼容语义，但把它标为部署层 fail-open 风险：M6 试点在服务启动前执行全局 policy freeze，因此运行期不允许任何 load/replace/remove；兼容语义只在未 freeze 的测试/维护启动中暴露。
- 对 blob decoder 做 corpus、截断、随机偏移、超大计数、非法状态转换和并发 replace fuzz。

**验收**：

- 直接提交 malformed/未知 blob 不 panic、不越界、不部分安装，旧 generation 保持可用。
- fork/clone 的 task 即使是 UID 0 且有 `CAP_MAC_ADMIN`，也不能获得 bootstrap authority；authority 被消费或 freeze 后不可恢复。
- 未 freeze 时，replace 对运行 task 的后续判定立即生效；remove 后 task 变 unconfined并审计；同名 reload 不会重新附着这些 task。freeze 后三种更新均稳定拒绝且旧 snapshot 不变。
- task、打开 file 在并发 replace/remove 下无 use-after-free 或半更新；旧 FD 的权限变化严格遵循 M4B 冻结的兼容矩阵。

### M5B：parser 端到端、可检测丢失的审计、procattr 与最小 ptrace（7-11 人周）

**目标**：删除测试 profile 注入，未修改的 pinned `apparmor_parser` 能管理策略，并满足受约束服务的最小攻击者模型；从本阶段结束才称为“AppArmor-compatible subset”。

**工作**：

- 实现 M0 已由实际调用追踪选定的精确 procattr/selfattr 接口；非授权任务不能读取/修改他人敏感 label。
- 将 console proof 升级为可检测丢失的 audit ring/reader：sequence、timestamp、lost counter、rate limit、阻塞/非阻塞读取语义。有限 ring 不能宣称无损可靠；Linux audit netlink 兼容可以后移，但安全试点不能只依赖易丢失 console。
- 完成无 fallback `px`；目标 profile 缺失必须拒绝。
- 基于现有 alien-access 实现最小 ptrace 双向限制，使传统 DAC/Yama 即使允许，也不能由未获 profile 许可的 tracer 接管受约束服务。检查不能只放在 attach：`PTRACE_TRACEME`、attach-before-exec、所有后续读写寄存器/内存与控制命令，以及 profile replace 后的既有关系都必须按当前 labels 重验或按 M0 冻结的上游行为安全解除。
- 固定 parser 参数和“警告即失败”；对 signal/network/mount 等未支持源码 class 做 parser 非零退出、旧 kernel policy 不变的端到端测试。内核只负责验证实际收到的 blob，不声称能看见 parser 已省略的源码规则。

**验收**：

- 未修改的 pinned parser 完成 add/replace/remove；`profiles` 与 `current` 一致。
- features 只广告实际实现项；每个未支持源码 class 都有 source→parser→blob/load 的负向测试，pipeline 失败且旧 policy 保持不变。
- 同 UID、父子关系或 Yama 仍允许的 tracer，在 AppArmor profile 未许可时无法附加/读取受约束服务；exec 前已附着、`PTRACE_TRACEME` 和 replace 后继续 POKE/SETREGS/CONT 的用例也无法绕过。
- audit transport 能通过 sequence/lost counter 检测丢失；complain/enforce/explicit deny 产生与 pinned 上游分类一致的记录。
- 同一 profile/workload 的 Linux oracle 差分覆盖 path、exec、capability、ptrace、replace/remove 与旧 FD；所有差异要么修复，要么写入明确的 Asterinas 扩展/限制清单。仅能 load blob 不构成兼容验收。

### M6：initramfs/NixOS 打包、部署与隔离实验性试点（4-6 人周）

**目标**：把内核功能变成可重复安装、可回滚、可观测的实验性系统能力。星绽仓库当前明确说明 Asterinas NixOS 尚不适合生产使用（[distro README](F:/App_Armor/asterinas/book/src/distro/README.md:14)），所以本阶段不能对外称生产部署；真正生产使用另受整个平台 readiness 与风险接受门禁约束。

**交付物**：

- pinned `apparmor_parser` 构建/维护软件包；需要 change_profile/DBus 前不强行引入完整 libapparmor 工具集。
- `/etc/apparmor.d/` 最小 policy 包与固定 ABI/feature 文件。
- 固定 parser 命令行、policy ABI、feature 文件和 warning-as-error 策略；构建产物记录这些输入的哈希。
- v1 冷启动只加载构建阶段用 pinned parser 生成并验证的 policy cache，不在 initramfs 现场编译；隔离试点不做 live policy update，策略更新生成新 cache 并走维护重启。
- initramfs 早期挂载 securityfs，在启动普通服务前加载已验证 policy cache。
- last-known-good policy/cache、原子启动项选择、版本兼容检查和回滚脚本。
- 一个最小状态命令读取 features/profiles/current/audit lost count；以后再评估完整 `aa-status` 兼容。
- 一个版本化的 required-profile manifest，明确映射 `launcher profile → 无 fallback px → target profile → executable/service`。stage-1 loader 只有在 manifest 全部满足并 freeze 后才 handoff 到 stage-2 init；systemd 场景用显式 unit dependency 表达相同门禁。
- 一个极小的受监督 launcher：它的第一项动作是核对自己的 current label；不匹配立即退出，绝不执行目标。匹配后只能通过已冻结 policy 中的无 fallback `px` 执行目标；目标 profile 缺失或 attachment 不等于 manifest 预期时，exec 在 commit 前失败。服务镜像因此不会先以 unconfined 身份运行再被监测纠正。
- v1 试点进入 stage-2 前执行全局单向 policy freeze：运行中不允许 load/replace/remove。更新、删除或改名都生成新 cache，先停依赖服务，再经维护重启和重新 exec 生效。这个部署约束不冒充标准 AppArmor ABI 语义。
- audit consumer 的权限、持久化、轮转、lost-count 健康检查和背压策略；ring 满时允许丢失但必须递增计数并告警，不能阻塞安全热路径造成系统死锁。

**启动顺序**：

1. kernel 以 `lsm=yama,apparmor`（capability 隐式 mandatory）启动；开发期保持 AppArmor opt-in。
2. AppArmor 初始化 root namespace 与 unconfined label；PID 1/早期 loader 在 v1 明确保持 unconfined。
3. initramfs 挂载 securityfs。
4. loader 读取 kernel features，校验与构建阶段固定 ABI/feature/hash 一致，并只装入已验证 cache。
5. kernel 原子装入 baseline policy；此时尚未创建任何其他用户 task，只有持一次性 bootstrap authority 的 PID 1 能管理 policy。
6. 核对 required manifest 后立即执行全局 policy freeze并销毁 authority；audit ring 负责缓存此前记录。
7. 启动 audit consumer，并持续监测 policy revision、profile 存在性和服务 current label；任何不一致立即停止对应服务并将试点标为 unhealthy。
8. 由受监督 launcher 核对自身 label，再经无 fallback `px` 首次 exec 被约束服务；任何 expected-profile 不匹配都在目标镜像运行前失败。若未来要求约束 PID 1，另立内核预装 policy 或两阶段 init/re-exec 方案。

**推出顺序**：

- 开发：只跑测试 profile，enforce/complain 双模式对照。
- CI：每条规则有 allow、缺少 allow、explicit deny，用例必须同时断言 syscall 结果和 audit；不支持的源码 class 必须让 parser pipeline 失败。
- staging：先 complain 收集真实访问，再人工审查；不能把日志自动转换成无限 allow。
- canary：只 enforce 一个边界清楚的服务，保留 last-known-good policy。
- 隔离实验性试点：扩大到少量非生产服务；任何 audit loss、未加载 required profile、launcher/target current label 不匹配或 feature 不匹配均 fail-stop/告警。生产环境必须另过 Asterinas 平台 readiness 门禁。

**回滚**：切换到 last-known-good kernel/policy-cache 启动项并重启；freeze 生效后不尝试 live replace/remove。只有应急启动才从 boot entry 移除 AppArmor。回滚路径必须在发布前实测，而不是文档声明。

**验收**：先限 x86_64；从空构建环境生成 kernel、parser、policy cache 和 initramfs，冷启动可重复；PID 1 的 unconfined 状态、bootstrap authority 的唯一性/销毁，以及 launcher→target 的精确 labels 均被断言；错误 attachment 的目标镜像零指令执行。策略损坏、parser/kernel ABI 不匹配、required manifest/profile 缺失、freeze 后的更新尝试、audit consumer 中断都有明确且已测试的 fail-stop 行为。required service 没有 `CAP_SYS_ADMIN`；legacy bind/move mount 与 attach-before-exec 均不能绕过 file/exec policy。RISC-V/LoongArch 只有编译通过时不得出现在发布支持声明中。

### M7：task mediation 与扩展 domain 子集（4-8 人周）

- ptrace：在 M5B 最小阻断基础上补齐 `trace/tracedby/read/readby`、审计和星绽现有 ptrace 边界。
- signal：hook 必须覆盖 `kill()` 位于 `check_signal_perm` 之外的 self 快路径以及 UID/session 分支，检查 sender send 与 target receive；同时明确 kernel-generated signal 的 system-subject/豁免清单，不能把所有内核信号错误套用用户 task rule。
- rlimit：在统一修改点增加规则。
- 补 `cx/ux`、fallback、大写 secure-exec、change_profile/onexec；hats/stacking 只有真实用户态需求时再加入。
- 只有出现真实的不中断更新需求后，才设计“先停依赖服务、更新 transaction、再 exec 并持续验证”的 live policy 编排；在此之前保留 M6 的 boot-time freeze。
- 每项独立广告 feature，可单独回滚，不绑成一个大 PR。

### M8：AppArmor mount 与 pivot_root mediation（4-8 人周）

- 复核 M2 已完成的 legacy mount/umount/pivot_root `CAP_SYS_ADMIN` 基线；本阶段不再把它当作尚未完成的前置项。
- 为 new/bind/remount/change-type/move/umount/pivot_root 建立强类型 context，所有检查在拓扑变更前。
- 规则包含源、目标、fstype、flags/options；不把 pivot_root 简化成普通 mount allow。
- 覆盖 mount namespace、chroot、bind/move、传播与并发拓扑变化。

### M9：network 与 Unix socket（8-14 人周）

- socket create/post-create、socketpair、bind/connect/listen/accept/send/recv、sockopt/shutdown hooks；accepted/socketpair 对象在失败清理时也不能遗留 label。
- 保存 creator label、accepted socket label 与 peer label；定义 profile replace 后解析规则。
- 基础 INET 先只实现当前上游主线兼容的粗粒度 family/type/protocol 子集。IP/port endpoint 中介若另有产品需求，应标成 Asterinas 扩展，不能冒充当前上游非 UNIX AppArmor 语义。
- Unix socket 再加入 pathname/abstract/unnamed、本端/对端地址、peer label 与双向 send/receive。
- SO_PEERSEC/secctx、getsockname/getpeername 的暴露语义分别验收；若 feature ABI 不能准确表达更窄子集，该 feature 不得广告。
- 无 net namespace 是明确限制；不能把容器网络隔离承诺写进该阶段。

### M10：高级功能，按真实需求排队

层级 policy namespace、compound/stacked labels、hats、DBus、mqueue、io_uring、userns、prompt/kill/default_allow 等不进入此前里程碑。它们只有在底层子系统存在、用户态消费者明确、威胁模型说明价值后才立项。

## 8. 首个可安装版本的明确边界

M6 的产品声明应写成类似：

> Asterinas AppArmor-compatible subset v1：支持 root policy namespace、pinned AppArmor parser pipeline、profile load/replace/remove（测试/维护启动；隔离试点启动后 freeze）、exec attachment/ix/px、文件规则子集、capability、最小 ptrace、enforce/complain、profile/current 查询和 Asterinas audit transport。未支持的 feature 不会被广告；固定 parser pipeline 会把未支持的源码 class 当成构建失败，内核会拒绝未知或非法 blob。

不能写“完整支持 AppArmor”，也不能宣称可直接运行发行版全部 `/etc/apparmor.d`。发行版 profile 常包含 abstractions、mount/network/DBus/signal/完整 ptrace 等组合规则；必须由兼容性测试逐份确认。

## 9. 测试与发布门禁

每个阶段至少满足：

- `make check`；
- 纯决策/decoder 的 `make test` 或 ktest；
- label/exec/VFS 竞态的 `make ktest`；
- `make run_kernel AUTO_TEST=regression` 的用户态 allow/deny/audit 闭环；
- pinned Linux AppArmor oracle 的同策略/同 workload 差分测试；
- x86_64 运行验证，RISC-V/LoongArch 至少编译，直到其 QEMU 安全回归纳入 CI；
- kernel 新代码保持 safe Rust，不把 policy decoder 的边界校验下沉成不必要的 unsafe；
- security-sensitive hook 的评审必须检查全部调用者和副作用顺序；
- 每个被广告 feature 至少有一条 allow、一条缺少 allow、一条 explicit deny、一条 invalid-policy 测试；
- decoder/profile replace/file transition 需要并发与 fuzz 门禁。
- M3A 后每个阶段都重跑已冻结的安全热路径性能门禁，不能只检查功能正确。

高价值回归集合：

- DAC deny + AppArmor allow 仍 deny；DAC allow + AppArmor deny 被拒绝；
- fork/clone 继承、成功/失败 exec、init、setuid/file-cap/no_new_privs；
- shebang/解释器；
- create/truncate/rename/link/unlink；
- owner/non-owner、hard-link source/target pair 与 permission subset；
- open-before-transition、forked FD、SCM_RIGHTS、mmap/mprotect；
- symlink/hardlink、deleted/O_TMPFILE、chroot、mount namespace/bind mount；
- invalid/partial/oversized blob；
- load/replace/remove 与 task/file 并发；freeze 后三种更新均拒绝；新 cache 的维护重启与回滚；
- attach-before-exec、`PTRACE_TRACEME`、profile replace 后继续执行全部 ptrace 命令；
- complain 对可学习 denial 的 ALLOWED audit、enforce 的 DENIED audit、explicit deny/输入错误的独立分类，以及 audit loss 计数。

## 10. 主要风险与控制

| 风险 | 后果 | 控制 |
|---|---|---|
| 过度声明兼容性 | 用户相信未被中介的资源已受保护 | feature 只广告真实能力；产品名明确为 subset |
| hook 在副作用之后 | 即使返回错误也已创建/截断/改拓扑 | 全部 mutation 做 pre-hook，测试状态未改变 |
| 路径错误或 rename 竞态 | 规则错配/绕过 | typed mediated path；deleted/disconnected 显式语义；并发测试 |
| FD/socket 旧资源语义不清 | 与上游不兼容或误宣称即时撤销 | M0/M4B 差分矩阵；object context；通用 file_receive；扩展语义单独声明 |
| policy blob decoder 缺陷 | kernel panic、越界或策略绕过 | strict bounds、fail closed、fuzz、原子安装 |
| parser 根据 features 省略源码规则，或同一 class qualifier 未实现 | 内核无法发现已丢失的安全意图，owner/link 规则退化为更宽 allow | 固定 parser 参数/ABI；warning-as-error；owner/non-owner 与 link pair/subset corpus；不能完整表达则不广告整个 feature |
| audit 丢失 | complain 无法用于学习，拒绝不可追踪 | bounded ring、sequence、lost counter、rate limit、consumer health；只承诺可检测丢失 |
| policy 装载失败、attachment 不匹配或 profile 被 remove 后静默 unconfined | 服务先以错误身份执行或运行中失去保护 | 一次性 bootstrap authority；服务前全局 freeze；launcher + 无 fallback `px` 原子门禁；监测只作二次防线 |
| legacy mount 或既有 ptrace 关系绕过 path/exec | 攻击者改写路径视图或跨 transition 注入 | M2 `CAP_SYS_ADMIN` 基线；required service 禁该能力；M3B/M5B 覆盖 trace 生命周期 |
| 上游 ABI 漂移 | parser 升级后静默改变语义 | pin tag+feature file+blob versions；升级矩阵，不跟随 latest |
| 热路径锁/分配 | 显著性能回退 | unconfined fast path、allow 无分配；先测后优化 snapshot 读取 |
| 许可证边界不清 | 无法合并或发行 | M0 完成维护者/法律确认；原生实现、不复制 GPL C 代码 |

## 11. 资源与时间判断

**推测**：M0-M6 合计约 51-76 人周；M0 外部许可证/维护者审批时间另计。按上述团队与串行关键路径，隔离实验性可安装版本约需 11-17 个日历月。M0、M3A、M4A 和 M5A 是强制重估点；此前数字不能当交付承诺。真正生产使用的时间无法从 AppArmor 子项目单独估算，因为还取决于 Asterinas NixOS 平台 readiness。M7-M9 另需约 16-30 人周。

建议角色：

- LSM/domain owner：label、policy store、decision、exec、ABI；
- VFS owner：安全路径、file/mmap/mount hook 与竞态；
- userspace/test owner：parser 打包、policy、initramfs、audit consumer、回归/fuzz；
- 独立 security reviewer：每阶段检查绕过面、更新原子性和失败行为。

如果只有一名开发者，应保留阶段顺序，先停在 M2 或 M4A 形成可验证研究原型，不应同时铺开 file、policy ABI、mount 和 network。

## 12. 官方上游依据

以下 `master` 链接用于展示调研日看到的当前实现，不是开发期的规范性版本锚点。M0 必须把 Linux/AppArmor 参考源码保存为 commit/tag permalink 与哈希，后续兼容结论只针对该冻结基线。

- [Linux 内核 AppArmor 管理文档](https://docs.kernel.org/admin-guide/LSM/apparmor.html)：task-centered MAC、unconfined 默认、用户态加载要求。
- [Linux AppArmor LSM hooks](https://github.com/torvalds/linux/blob/master/security/apparmor/lsm.c)：实际 hook 面、blob 类型、major/exclusive 初始化。
- [AppArmor domain.c](https://github.com/torvalds/linux/blob/master/security/apparmor/domain.c)：exec attachment/transition。
- [AppArmor file.c](https://raw.githubusercontent.com/torvalds/linux/master/security/apparmor/file.c)：owner/non-owner permission 选择与 hard-link pair/subset 检查。
- [AppArmor apparmorfs.c](https://github.com/torvalds/linux/blob/master/security/apparmor/apparmorfs.c)：securityfs、features、load/replace/remove、profiles。
- [AppArmor policy.h](https://github.com/torvalds/linux/blob/master/security/apparmor/include/policy.h)、[policy_ns.h](https://github.com/torvalds/linux/blob/master/security/apparmor/include/policy_ns.h)、[label.h](https://github.com/torvalds/linux/blob/master/security/apparmor/include/label.h)：profile、namespace、label 数据模型。
- [AppArmor policy_unpack.c](https://github.com/torvalds/linux/blob/master/security/apparmor/policy_unpack.c)：二进制策略验证与解包攻击面。
- [AppArmor 策略语言手册](https://gitlab.com/apparmor/apparmor/-/raw/master/parser/apparmor.d.pod)、[parser 手册](https://gitlab.com/apparmor/apparmor/-/raw/master/parser/apparmor_parser.pod)：规则类别、编译、cache、feature/policy ABI。
- [libapparmor kernel interface](https://gitlab.com/apparmor/apparmor/-/raw/master/libraries/libapparmor/src/kernel.c)：用户态实际使用的 securityfs/procattr 接口。
- [AppArmor FAQ](https://gitlab.com/apparmor/apparmor/-/wikis/FAQ)：profile replace/remove 对运行任务和已打开资源的可观察边界。
- [AppArmor 官方 releases](https://gitlab.com/apparmor/apparmor/-/releases)、[tags](https://gitlab.com/apparmor/apparmor/-/tags)：截至调研日的版本状态和 5.0 bridge release 说明。

## 13. 下一步决策点

进入实现前只需确认一个架构决策：是否接受“星绽原生安全 Rust实现、首个公开版本只兼容诚实声明的 AppArmor feature 子集、M1-M4B 不对外承诺标准 loader、M5B 再以未修改 parser pipeline 作为兼容门槛”。

确认后，第一批实际工作应严格限制在 M0：冻结版本/feature 契约、生成兼容测试 blob、完成 hook/旁路清单和许可证确认。不要先创建一整套 AppArmor 模块空目录或为未来 network/mount 写抽象。
