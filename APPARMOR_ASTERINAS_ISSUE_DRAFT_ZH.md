# 建议提交的 Asterinas Issue

- **标题：** <code>Add an AppArmor-compatible LSM subset to Asterinas</code>
- **初始类型：** 设计提案，而不是版本发布承诺
- **模板：** Feature Request；如果直接粘贴下面准备好的正文，也可以使用 Blank Issue
- **建议标签：** <code>C-design-proposal</code>（由维护者或机器人完成初步分类后添加）

如果 Feature Request 模板自动添加了 <code>C-feature-request</code>，请评论：

~~~text
@boterinas label -C-feature-request C-design-proposal
~~~

请维护者判断这项跨子系统设计是否应升级到正式 RFC 流程；不要预先在初始标题中加入 <code>[RFC]</code>。

---

## 概要

本 Issue 提议在 Asterinas 上增加一个兼容 AppArmor 的 Linux Security Module（LSM）功能子集，并以 Asterinas 原生的安全 Rust 实现。

本提案并不是要翻译 Linux 的 <code>security/apparmor</code> 实现，也不宣称实现完整的 AppArmor 兼容性。目标是在 Asterinas 自身的任务、凭据、VFS、exec 和组件模型之上，重现一组经过审慎限定、外部可观察的 AppArmor 语义，同时只公开那些 Asterinas 已经完整实现的官方用户态 ABI 功能。

在开始实现之前，本 Issue 希望社区就以下事项达成共识：

1. 初始兼容范围和威胁模型边界；
2. <code>aster-core</code> 中的通用 LSM 基础设施与 AppArmor 高层组件之间的接口边界；
3. 分阶段实现与验证计划；
4. 能够评审这项跨子系统工作的维护者和代码所有者。

## 动机

AppArmor 是一种以任务为中心的强制访问控制系统。安全标签通过配置文件（profile）关联到任务，并对 exec、文件访问、capability 使用和 ptrace 等操作进行仲裁。通常情况下，策略由 <code>apparmor_parser</code> 编译，再通过内核的 AppArmor 文件系统接口加载。

支持一个定义清晰的 AppArmor 子集，可以为 Asterinas 带来：

- 一种兼容 Linux 的应用程序约束机制；
- 与现有 AppArmor 策略工具链中一个固定版本子集的兼容能力；
- capability 和 Yama 之外，Asterinas LSM 框架的一个端到端使用者；
- 一个可复用的测试平台，用于验证任务、exec、VFS、策略更新和审计仲裁。

与那些要求兼容 Linux 内部 C 数据结构或辅助接口的方案不同，本提案面向一个固定版本的用户态策略 ABI，以及外部可观察的系统调用和安全行为。内部实现仍然可以自由使用 Asterinas 原生的 Rust 类型和组件接口。

在把实现工作列入版本路线图之前，必须明确一个具体的首个试点工作负载。Issue 作者应说明该工作负载是一个 Asterinas NixOS 服务、一个 OCI/容器运行时 AppArmor 配置文件，还是其他可复现的应用程序，并链接目前只能在非约束状态下运行的命令或测试。

近期目标是一个实验性的、仅支持 x86_64 的系统试点。本提案并不宣称 Asterinas 或 Asterinas NixOS 已经可以用于生产环境。

## 当前状态

当前的 LSM 框架有意保持得很小：

- [<code>LsmModule</code>](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/mod.rs#L31-L48)
  目前只组合了 capability 和 alien-access 两类 hook trait。
- [当前 hook 集合](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/hooks/mod.rs#L14-L28)
  只包含上述两类 hook。
- [模块注册机制](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/modules/mod.rs#L31-L42)
  是一个私有的静态列表，其中仅包含 capability 和 Yama。

Asterinas 已经具备一些有用的实现接入点：每线程凭据、集中的 exec prepare/commit 路径、<code>Path = Mount + Dentry</code>、通用 VFS 操作路径、文件句柄、mmap 后备文件、启动参数以及 QEMU 回归测试。但它目前还没有 AppArmor 任务标签、exec/file hook、策略 blob 解码器、AppArmor securityfs 节点或 AppArmor 审计传输机制。

[当前内核组织规范](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/coding-guidelines/for-maintainability/rust-specific/crates-and-modules.md)
指出，新的内核子系统默认应作为独立 crate 放在 <code>aster-core</code> 之外。因此建议采用以下代码布局：

- <code>aster-core</code>：通用 LSM 接口、hook 上下文、注册机制，以及已注册 LSM 所需的最小不透明任务/文件安全状态；
- <code>kernel/comps/apparmor</code>：AppArmor 标签、配置文件、规则匹配、决策、策略存储、ABI 解码器和 AppArmor 专用 hook 实现；
- <code>distro/</code> 和 <code>test/</code>：固定版本的 parser 打包、策略缓存、早期加载器、审计消费程序和端到端回归测试。

由于现有 <code>LsmModule</code> 和静态模块列表都属于 <code>aster-core</code> 私有实现，这种拆分需要增加一个小型组件注册接口。它不应顺带触发 capability 或 Yama 的无关迁移。

## 设计原则

1. **原生安全 Rust。** <code>kernel/</code> 下的代码继续保持为安全 Rust。设计可以参考 Linux 已有文档记录的 ABI 和外部可观察行为，但不得复制或机械翻译仅以 GPL-2.0 授权的实现代码。
2. **官方 ABI 子集。** <code>features/</code> 只公布已经端到端实现并验证的行为。如果某个 feature bit 的粒度比当前实现更粗，则应完整实现对应功能组、选择更低版本的 ABI，或者不公布该功能。
3. **内核中不实现策略文本解析器。** 使用固定版本且未经修改的 <code>apparmor_parser</code> 编译源策略。内核只接受固定版本的二进制策略 ABI，并在安装前验证所有不可信字段。
4. **唯一决策路径。** 类型化 hook 上下文统一进入一个 AppArmor 决策引擎；原始规则结果产生后，再应用执行模式和结构化审计。
5. **原子的生命周期变更。** exec 标签转换和策略替换必须原子提交。被拒绝的操作不得遗留 VFS 或策略副作用。
6. **渐进式评审。** 每个阶段拆分为聚焦的 Issue/PR，并附带用户可见测试。本 Issue 并不是要用一个大型实现 PR 完成全部工作。

## 初始兼容目标

首个可安装子集应包含：

- 支持通过 <code>lsm=apparmor</code> 选择 AppArmor，并默认处于 unconfined 状态；
- 一个根策略命名空间，以及每个任务只关联单个配置文件的标签；
- 标签在 fork/clone 时继承，并在 exec 时完成附着或转换；
- enforce 和 complain 模式，以及结构化、可检测丢失的审计记录；
- 在常规 capability 检查之后执行 capability 仲裁；
- exec 附着、<code>ix</code> 和不允许回退的 <code>px</code>；
- 被公布 ABI 中的文件权限，包括 owner/non-owner 条目、硬链接成对检查和权限子集语义、修改操作的事前检查、继承或接收的文件描述符，以及 mmap/mprotect；
- 阻止 tracer 在受保护的 exec 或配置文件替换后继续保留控制权所需的最小双向 ptrace 检查；
- 针对一个固定 parser/策略 ABI，提供官方 <code>features</code>、<code>profiles</code>、load/replace/remove 和 current-label 接口；
- 构建时生成策略缓存，并在早期 initramfs 阶段完成加载。

准确的 feature manifest 和 parser 版本应由下面的兼容性验证阶段得出，而不是作为本提案的预设前提。

## 建议路线图

相关设计获批后，每个复选项都应拆成一个或多个独立 Issue 和聚焦的 PR。

- [ ] **冻结兼容性契约和威胁模型**
  - 固定一个 Asterinas commit、一个 Linux AppArmor 参考 commit、parser tag、策略 ABI 和 feature ABI。
  - 为每一种已公布的权限、转换、限定符和无效组合生成黄金策略 blob 语料库。
  - 建立一个固定版本的 Linux 差分对照基准，用于比较系统调用结果/errno、标签、审计分类、策略替换和已打开资源的行为。
  - 与维护者确认实现和许可证边界。

- [ ] **增加一个可启动的 unconfined AppArmor 组件骨架**
  - 向 LSM 核心增加注册高层组件所需的最小接口。
  - 使用 <code>lsm=apparmor</code> 启动，同时让所有任务保持 unconfined，并保持基线行为不变。
  - 增加明确的初始化测试和任务标签继承测试。

- [ ] **完成第一个 capability 垂直切片**
  - 实现“标签 → 配置文件 → 规则 → 决策 → 模式 → 审计 → 系统调用结果”的完整链路。
  - 在公布 capability 功能之前，审计所有安全相关的 capability 检查，并将它们统一路由到 LSM hook。
  - 在依赖路径约束之前，为旧有 mount/umount/pivot-root 路径补回缺失的 <code>CAP_SYS_ADMIN</code> 基线检查。

- [ ] **增加主体可见路径和 exec 域**
  - 定义 mount namespace、chroot、rename 竞态、已删除路径和 <code>O_TMPFILE</code> 的路径语义。
  - 增加 exec prepare/commit hook、配置文件附着、<code>ix</code> 和不允许回退的 <code>px</code>。
  - 覆盖脚本/解释器、set-id/文件 capability、<code>no_new_privs</code> 和现有 ptrace 关系。

- [ ] **闭合文件仲裁的完整生命周期**
  - 在产生副作用之前放置 create/truncate/rename/link/unlink 和元数据 hook。
  - 覆盖 open/read/write/append/lock、继承的文件描述符、SCM_RIGHTS、mmap/mprotect、owner 限定符以及硬链接权限子集检查。
  - 如果所选 ABI 中仍有某项操作或限定符能够绕过仲裁，就不得公布 file 功能。

- [ ] **增加官方策略控制面**
  - 实现固定版本 parser 所使用的最小 securityfs/AppArmor 节点集合。
  - 严格解码固定版本的策略 blob 格式，并以原子方式安装。
  - 增加 profiles/current-label 状态查询、可检测丢失的审计读取器以及最小 ptrace 生命周期检查。
  - 在固定 parser 流水线中拒绝不受支持的源规则类别，并由内核拒绝未知或无效 blob。

- [ ] **打包并验证一个实验性系统试点**
  - 使用固定版本 parser 构建策略缓存，而不是在 initramfs 中编译策略。
  - 赋予初始加载器一次性引导权限，加载并验证所有必需配置文件，然后在启动其他用户任务之前冻结策略更新。
  - 通过受约束的启动器和不允许回退的 <code>px</code> 启动受保护服务，确保配置文件附着不匹配时，目标程序镜像绝不会在 unconfined 状态下执行。
  - 提供一个最近已知可用的启动项、审计消费程序、健康检查以及经过测试的维护重启回滚路径。

## 安全不变量

只有以下条件全部成立，才能宣布初始试点成功：

- AppArmor 的 allow 结果绝不能覆盖普通 DAC/capability 的 deny 结果；
- 格式错误或不受支持的策略必须被拒绝，并且不能只替换当前有效策略快照的一部分；
- 被拒绝的 VFS 修改操作不得产生任何对象、数据、元数据或拓扑结构变更；
- 旧有 mount 操作不能利用缺失的普通 capability 检查绕过路径策略；在 AppArmor mount 仲裁尚未实现之前，受保护的试点服务不得获得 <code>CAP_SYS_ADMIN</code>；
- attach-before-exec、<code>PTRACE_TRACEME</code> 和策略替换后的 ptrace 命令，都不能继续对受约束任务保持未授权控制；
- 如果必需配置文件缺失，或者启动器/目标标签错误，系统必须在受保护目标程序镜像执行之前失败；
- 已公布的文件行为必须包含同一 ABI 中的 owner/non-owner 和硬链接条件，而不能把这些条件当作无条件允许；
- 审计丢失必须能通过序列号/丢失计数器检测出来；设计不得声称容量有限的环形缓冲区永不丢失数据。

## 验证

每个实现阶段都应运行以下命令中与本阶段相关的最小集合：

~~~bash
make check
make test
make ktest
make run_kernel AUTO_TEST=regression
~~~

验证应测试用户可见行为，而不是内部常量。对于每项已公布功能，至少应包含一个允许案例、一个因缺少 allow 规则而拒绝的案例、一个显式 deny 案例，以及一个格式错误或不受支持的策略输入案例。凡是宣称兼容的地方，还应使用同一策略和工作负载与固定版本的 Linux AppArmor 对照基准进行比较。

首个运行时目标是 x86_64。在 RISC-V 和 LoongArch 的安全回归套件进入 CI 之前，不应在支持声明中包含这两种架构。

## 文档和 CI 计划

实现工作应更新 Asterinas Book，记录：

- 已支持和不支持的 AppArmor 功能及策略 ABI 矩阵；
- 启动参数以及准确的 parser/缓存构建契约；
- 策略管理权限、引导/冻结行为和审计字段；
- 实验性部署、健康检查、维护更新和回滚流程；
- 与固定版本 Linux AppArmor 对照基准之间的明确差异。

CI 应将 unconfined 基线、策略输入负面语料库、已公布功能的回归测试以及 x86_64 启动/部署测试充分分开，以便确定具体是哪一项契约失败。在其他架构加入运行时安全测试之前，它们只进行编译检查。

## 缺点和替代方案

本提案会带来长期的兼容性和安全评审负担。LSM hook 会影响高频且安全敏感的执行路径，策略解码器需要处理恶意输入；固定的 AppArmor ABI 升级时还必须重新验证兼容性。只有在社区就具体试点工作负载以及维护者/评审者责任归属达成一致时，这些成本才是合理的。

考虑过的替代方案：

- **把 Linux <code>security/apparmor</code> 翻译成 Rust。** 这种做法不符合 Asterinas 的对象模型以及安全 Rust/组件规范，还会产生严重的许可证和维护问题。
- **增加一种自定义的类 AppArmor 策略语言。** 它可以快速演示一次拒绝操作，但不能与官方 parser 或配置文件生态互操作。在官方解码器完成之前，它只能作为测试夹具使用，绝不能成为面向用户的 ABI。
- **只增加通用 LSM hook，而不提供 AppArmor 使用者。** 某些 hook 工作确实必要，但不应在存在具体使用者和用户可见测试之前，提前加入推测性的 hook 族和 blob 注册表。

## 初始子集不包含的目标

- 完整兼容每个发行版在 <code>/etc/apparmor.d</code> 下提供的配置文件；
- 兼容 Linux AppArmor 的内部实现细节；
- 自定义的类 AppArmor 策略语言或内核文本解析器；
- 复合/堆叠标签、分层策略命名空间、hat，以及任意 <code>change_profile</code>/onexec 行为；
- AppArmor 的 signal、rlimit、mount-rule、network、Unix-peer、DBus、mqueue、io_uring、prompt 或 kill 仲裁；
- Linux audit-netlink 线级协议兼容；首个试点使用明确属于 Asterinas、并且能够检测丢失的审计传输机制；
- 首个系统试点中的在线策略更新；
- 关于 Asterinas NixOS 已可用于生产环境的声明。

只有在初始 hook、对象生命周期和实际用户需求已经存在后，才应另行提议这些功能。

## 向维护者提出的待决问题

1. 根据当前内核 crate 规范，在 <code>aster-core</code> 中提供通用 LSM 接口，同时把 AppArmor 实现为高层组件，是否是正确的代码布局？
2. 对于由核心持有的组件任务/文件安全状态，以及 clone/exec 生命周期回调，最小可接受的数据表示是什么？
3. 建议的初始兼容边界是否足够小，能够接受评审，同时又没有在 AppArmor feature ABI 的功能粒度问题上作出不诚实的兼容声明？
4. 哪些维护者或代码所有者能够评审通用 LSM、exec/VFS 和用户态策略部分？
5. 在首席开发者和评审者明确同意承担责任之前，本提案是否应保持不分配任何发布版本？
6. 第一个部署试点具体采用哪个服务或容器工作负载？哪个外部可观察的约束测试可以作为成功标准？

## 协作方式

在架构达成共识后，本 Issue 可以转为总跟踪 Issue。路线图中的每一项都应链接到专门的子 Issue/PR。准备性重构应与功能行为分开提交，每个 PR 应只表示一项逻辑变更，并拥有自己的回归测试覆盖。

目前不提出发布版本目标。把这项工作加入某个版本计划，应当是具名实现者和评审者之间另行作出的社区协作承诺。

在发布本提案之前，作者应增加一个简短段落，说明自己的项目背景和具体投入承诺，例如计划负责兼容性验证、LSM 骨架或后续阶段中的哪一部分。

## 参考资料

- [Linux 内核 AppArmor 文档](https://docs.kernel.org/admin-guide/LSM/apparmor.html)
- [Linux AppArmor LSM hook](https://github.com/torvalds/linux/tree/master/security/apparmor)
- [AppArmor parser 和用户态项目](https://gitlab.com/apparmor/apparmor)
- [Asterinas 内核组件提案 #3601](https://github.com/asterinas/asterinas/issues/3601)
- [Asterinas RISC-V IOMMU 跟踪 Issue 结构 #3474](https://github.com/asterinas/asterinas/issues/3474)
- [Asterinas seccomp 设计提案 #3648](https://github.com/asterinas/asterinas/issues/3648)
- [Asterinas Issue 模板](https://github.com/asterinas/asterinas/tree/main/.github/ISSUE_TEMPLATE)
- [Asterinas 贡献规范](https://github.com/asterinas/asterinas/tree/main/book/src/to-contribute/coding-guidelines)
