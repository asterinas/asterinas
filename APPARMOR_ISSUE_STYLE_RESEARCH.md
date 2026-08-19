# Asterinas AppArmor issue 写作风格调研

> 调研日期：2026-08-19  
> 本地 Asterinas 基线：`34a8a2d72829897d8cab3b4cb9be8ceb69dac584`  
> 资料范围：仅 Asterinas 官方仓库文档、issue 模板、GitHub issue 和 CODEOWNERS 成员的公开评论；没有读取或参考工作区内的旧 AppArmor issue 规划。

## 结论

不建议把第一个公开 issue 定题为 **Deploy AppArmor on Asterinas**。这会让人以为只是打包安装或已承诺完整 AppArmor 兼容性，与实际的内核跨子系统开发不符。

按当前社区流程，最稳妥的做法是两步：

1. 先提交一个设计 issue：**`Add an AppArmor-compatible LSM subset to Asterinas`**，标签用 `C-design-proposal`。它用于确认需求、Linux 兼容边界、架构、安全不变量、最小范围，并请维护者决定是否要走正式 RFC。
2. 设计建立共识后，再建一个实施跟踪 issue：**`Tracking issue for AppArmor-compatible policy enforcement`**，标签用 `C-tracking-issue`，只跟踪已经同意的子 issue/PR 和验收状态。

如果当前只准备发一个 issue，应发第一个设计 issue，而不是预先宣称为 tracking issue。也不要现在批量创建空子 issue；等架构和首个垂直切片被接受后，再按“一个独立可测能力/一个独立设计决策”逐个拆分。

## 已验证的社区规则

### Issue 模板与标签

- 官方 [Tracking Issue 模板](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/.github/ISSUE_TEMPLATE/tracking_issue.md) 默认标签是 `C-tracking-issue`，最小结构是 `Description` / `Current Status` / `Additional Information`。
- 官方 [Feature Request 模板](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/.github/ISSUE_TEMPLATE/feature_request.md) 要求说明功能、问题/需求、建议实现、备选方案和补充背景。
- 当前标签目录中，`C-design-proposal` 的官方描述是 “Design proposal”，`C-tracking-issue` 是 “Track the progress of a specific task or feature”；`C-feature-request` 只表示建议新功能。可在 [Asterinas labels](https://github.com/asterinas/asterinas/labels) 核验。
- [Boterinas 文档](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/boterinas.md) 和 [`triagebot.toml`](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/triagebot.toml) 显示，社区使用 `C-*` 标记 issue 类别，使用 `O-*` 标记 PR 类别；用户可通过 `@boterinas label ...` 调整允许的类别标签。
- 与 AppArmor 最接近的 [seccomp 设计 issue #3648](https://github.com/asterinas/asterinas/issues/3648) 最终使用 `C-design-proposal`；作者在[评论](https://github.com/asterinas/asterinas/issues/3648#issuecomment-5079305498)中去掉 `C-feature-request` 并改为 `C-design-proposal`。这是 AppArmor 第一个 issue 最直接的风格先例。

### 何时需要 RFC

[RFC-0001](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/rfcs/0001-rfc-process.md) 明确区分了三类情况：

- 显著改变 roadmap、项目级规范或重大架构时需要 RFC；
- 影响只限于一个子项目/模块的非平凡设计，可先在 GitHub Issues 提交 design proposal；
- 标准 Linux syscall 或设备驱动这类成熟模式通常不需要 RFC。

**推断，不是已有维护者结论：** AppArmor 虽然是 Linux 成熟功能，但路线 A 需要同时改动 LSM、task credentials/label、exec、VFS、securityfs、audit 和用户态 policy ABI，不像一个独立 syscall。因此它至少应先走 design-proposal/socialization，并在 issue 中显式请维护者判定是否升级为正式 RFC，而不应由提交者自行假定。

RFC 文档还建议在正式 RFC 之前先通过 GitHub Discussions、Pre-RFC 或与 code owner 沟通来评估兴趣和收集反馈。

### 实施应当如何拆分

[Asterinas 可维护性贡献指南](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/coding-guidelines/for-maintainability/process.md) 要求：

- 一个 commit 只包含一个逻辑变更；
- 准备性重构与功能改动分开；
- PR 聚焦单一主题，请求评审前通过 CI。

相同原则应影射到 AppArmor 子 issue：一个子 issue 对应一项可独立评审、可独立测试、合并后不破坏现有行为的能力，而不是把所有 hook 堆在一个 PR 里。

### 评审者关注的内容

根据根 [CODEOWNERS](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/CODEOWNERS)，根维护者是 `@tatetian`，内核顶层 code owners 是 `@StevenJiang1110` 和 `@lrh2000`；目前没有单独的 security/LSM code owner 规则。

在内核参数设计 issue #2982 中，根 CODEOWNER `@tatetian` 的[完整评审评论](https://github.com/asterinas/asterinas/issues/2982#issuecomment-3988533438) 特别指出：

- 不能只讲 Asterinas 内部架构，还要说清如何忠实实现 Linux 的可观察行为；
- 设计需要考虑默认值、错误输入、初始化顺序等边界语义；
- proposal 应包含文档计划和 CI 测试计划。

这对 AppArmor 直接意味着：issue 不能只画 `SubjectLabel -> DecisionEngine` 的内部图；还必须列出具体支持的 feature/policy ABI、`load/replace/remove`、exec/file/capability 结果、错误码、继承/跨 `execve` 语义、不支持输入的 fail-closed 行为及差分测试计划。

## 代表性 issue 及共性

| Issue | 类型/状态（2026-08-19） | 值得借鉴的写法 |
|---|---|---|
| [#3648 Add seccomp support with strict and classic-BPF filter modes](https://github.com/asterinas/asterinas/issues/3648) | `C-design-proposal`，closed | 与 AppArmor 最近的安全子系统先例；列出 Linux ABI、状态继承、权限、分阶段实施、non-goals 和 expected validation。 |
| [#3696 Hypervisor OSTD PR and Test Plan](https://github.com/asterinas/asterinas/issues/3696) | `C-tracking-issue`，open | 用表格列 PR 顺序、合并后的独立能力和测试环境；每个 PR 都有 `Atomic scope` 和 `Test plan`，最后用 `After PR5` 明确延后功能。 |
| [#3586 Tracking issue for adding AArch64 support](https://github.com/asterinas/asterinas/issues/3586) | `C-tracking-issue`，open | 先列准备依赖，再限定“minimal main PR”，后续能力逐个增加，并明确 SMP/CVM/IOMMU 超出当前 tracker 范围。 |
| [#3479 Tracking issue for Asterinas 0.19.0 release](https://github.com/asterinas/asterinas/issues/3479) | `C-tracking-issue`，open | 根维护者撰写；明确多 PR 大功能与小功能/修复的边界，并说“先建专用 issue 写计划并建立共识”才进入 release tracker。 |
| [#3343 Improve virtio-fs syscall coverage, CI validation, and benchmarking](https://github.com/asterinas/asterinas/issues/3343) | `C-tracking-issue`，open | 先 correctness/CI，再 benchmark，再扩展 features；每个可执行项目链接独立 issue/PR，并说明不只是 syscall 返回成功，还要维持 Linux 语义。 |
| [#3239 Fully implement initial random seed generation](https://github.com/asterinas/asterinas/issues/3239) | `C-tracking-issue`，open | 先用源码链接、可复现命令和 panic 输出证明现状；将任务标记为 necessary/optional/requires discussion，明确有争议的语义应拆独立 issue。 |
| [#3207 Add Ext4 Filesystem Support](https://github.com/asterinas/asterinas/issues/3207) | `C-tracking-issue`，open | 从只读、extents 的最小目标开始，按 foundation/metadata/data/VFS 分阶段，并明定详细架构和首 PR 范围放到单独 Design Issue。 |
| [#3728 Tracking issue for optimizing UDP bandwidth over virtio-net](https://github.com/asterinas/asterinas/issues/3728) | `C-tracking-issue`，open | 不只列优化项；先给 Linux/Asterinas 基线、各原型结果和未解释的回归，让验收可量化。 |
| [#1847 Tracking issue for Intel TDX production support](https://github.com/asterinas/asterinas/issues/1847) | `C-tracking-issue`，open | 按 SMP、attestation、performance、bugs、security audit 分类，任务均链接独立 issue/PR；表明安全相关 tracker 需要单列 security audit，不能只跟踪功能完成。 |

这些 issue 的共性不是“写得很长”，而是：

1. 开头快速说清用户需求和当前缺口；
2. 有证据的 Current Status，包括源码链接、运行命令、当前结果或基线；
3. 从一个可运行/可测的最小切片开始，不在第一 PR 追求完整功能；
4. 把依赖、实施顺序、独立能力和测试环境写清楚；
5. 复杂或有争议的决策拆为专用 design issue，已确认的工作才进 tracker；
6. 在 parent tracker 中用 issue/PR 链接和 checkbox 跟踪，而不是把每个实现细节写成不可独立验收的巨型任务。

## AppArmor 设计 issue 应该写什么

### 推荐定题

**Title**

```text
Add an AppArmor-compatible LSM subset to Asterinas
```

**Initial label**

```text
C-design-proposal
```

如果使用 Feature Request 模板创建后自动带上 `C-feature-request`，可仿照 #3648 留言：

```text
@boterinas label -C-feature-request C-design-proposal
```

不建议提交者自行添加 P0/P1 或承诺纳入某个 release；[#3479](https://github.com/asterinas/asterinas/issues/3479) 把 release 功能列表定义为开发者和评审者的双向“社会契约”。在没有 lead reviewer 和已同意范围前，这个承诺并不成立。

### 推荐正文结构

可以沿用 Feature Request 模板，再增加 seccomp #3648 使用的 `Non-goals` 和 `Expected Validation`：

1. **Feature Description / Summary**  
   一段话说清：在 Asterinas 中用 safe Rust 原生实现 AppArmor 可观察语义，只暴露已完整实现并测试的官方 ABI 子集。
2. **Problem or Need**  
   说明实际用户/工作负载，不要只说“Linux 有，所以 Asterinas 也要有”。例如：需要对 Asterinas NixOS 中的服务实施强制访问控制，并使用现有 AppArmor 策略工具链的可验证子集。
3. **Current Gap**  
   用当前 Asterinas 源码链接说明已有 LSM 框架和 hook，以及缺失的 task label、file/exec mediation、policy control plane、audit/procattr 等。只写已验证事实，推断单独标注。
4. **Compatibility Contract**  
   列出要对齐的三个可观察面：官方 parser/policy blob、securityfs/procattr 控制 ABI、syscall 决策/错误码/审计。明确未广告的 feature 不在兼容声明中。
5. **Suggested Design**  
   仅保留经过当前评审所需的核心模块：task label/profile、immutable policy snapshot、decision engine、typed hooks、strict blob decoder、audit。不在 issue 中预先定义没有用户的泛化框架。
6. **Security Invariants**  
   显式列出 fail-closed policy input、副作用前检查、label 在 fork/clone/exec 中的原子性、不能由未授权任务管理 policy、禁用时不改变现有行为等。
7. **Implementation Staging**  
   只写里程碑和每阶段可观察产出，不要把几十个函数当作 checklist。首个实施切片应是“可启动的 unconfined 骨架”，第一个安全闭环才是 label/profile/capability/decision/audit。
8. **Expected Validation / Acceptance Criteria**  
   验收用用户可观察结果表达，包括 Linux 差分测试，而不是“类型写完”或“模块能编译”。
9. **Non-goals**  
   明确 v1 不做的内容，防止“AppArmor support”被理解为全部上游功能和任意发行版 profile 都可用。
10. **Alternatives, Risks, and Open Questions**  
    说明为什么不整体移植 Linux C 实现，为什么不伪造自定义 policy ABI，以及许可证、feature ABI 粒度、路径语义、PID 1/bootstrap 等需要社区决策的问题。
11. **Documentation and CI Plan**  
    列出 Linux-compatibility 文档、supported/unsupported matrix、回归测试、Linux oracle 差分测试和启动集成测试。
12. **References**  
    链接 Asterinas 现有 LSM 源码、Linux/AppArmor 官方 ABI 文档、固定版本/提交的 parser 和差分测试基线。

### 总 issue 的验收条件应该怎样写

以下是路线 A 的建议验收集，是针对 AppArmor 的设计建议，不是 Asterinas 已经公布的硬性模板：

- [ ] 在 AppArmor 未启用或所有任务为 `unconfined` 时，现有回归测试不发生行为变化。
- [ ] 固定版本的官方 `apparmor_parser` 能根据 Asterinas 广告的 feature ABI 产生 policy cache，内核能原子加载它。
- [ ] 实现并文档化约定的 `load/replace/remove` 、profile attachment 和 current-label 语义。
- [ ] 对广告的 capability、exec 和 file 权限，enforce 模式拒绝操作，complain 模式放行可学习拒绝，两者均产生可区分审计记录。
- [ ] 所有已广告权限均覆盖其安全相关入口；未完成的 feature 不广告，未知/损坏 policy blob 整体拒绝且不留下部分状态。
- [ ] label 在 fork/clone/exec 和 policy replace/remove 时的语义有回归测试，且不出现部分更新可观察窗口。
- [ ] 每个对外声明支持的 ABI/权限都有与固定 Linux AppArmor 对照环境的差分用例，比较 syscall 结果、label、policy lifecycle 和审计语义。
- [ ] 完成 supported/unsupported matrix、安全模型、管理接口和部署/回滚文档。
- [ ] 一个受控 x86_64 试点镜像能在保护服务首次 `execve` 之前装载必需 profile，验证失败时停止启动受保护服务。

注意：“能编译”、“可以启动”、“parser 返回成功”均不是安全功能的充分验收条件。

### 建议明写的 non-goals

- 不移植或逐行翻译 Linux `security/apparmor` C 实现。
- v1 不宣称完整 AppArmor 兼容，只声明文档化、广告并测试的 ABI 子集。
- v1 不保证未经筛选的 Ubuntu/SUSE/其他发行版 profile 可直接使用。
- 首个实施切片不包括 network、mount、signal、D-Bus、mqueue、io_uring 等后续类别；需要时单独提案。
- 首个试点不承诺生产级支持、全架构支持或无条件策略热更新。
- 不暴露一套伪装成 AppArmor 的自定义用户 ABI；测试专用的内核构造器不算用户面兼容性。

## 如何拆子 issue

设计建立共识后，建议在 parent tracker 中按下列边界创建子 issue：

1. 独立的架构/兼容性决策，例如 policy ABI 版本和 feature advertisement。
2. 必要的准备重构，例如共用 LSM hook 扩展；不与 AppArmor 行为改动混在一个 PR。
3. 一个端到端安全切片，例如 `label -> capability decision -> errno -> audit`。
4. 一个独立对外 ABI，例如严格 policy blob load/replace/remove。
5. 一个独立的集成/部署产出，例如 initramfs policy cache 和 fail-closed service gate。

建议的 tracker 顶层顺序是：

```text
contract/baseline
  -> bootable unconfined skeleton
  -> first capability vertical slice
  -> path + exec domain
  -> file mediation closure
  -> official policy/control ABI
  -> audit + procattr + minimal anti-takeover controls
  -> x86_64 experimental system pilot
```

不建议按“一个 hook 一个 issue”机械拆分；一个独立 hook 通常不产生可安全验收的用户能力。也不建议在设计评审前创建七个空 issue；先建第一个已同意切片，其他保留为 parent 中的未链接项目即可。

## 一个合格 parent tracker 的最小格式

下面不是完整 AppArmor 提案，而是设计获得共识后用来建 tracker 的最小骨架：

```markdown
### Description

This issue tracks the implementation of the approved AppArmor-compatible
LSM subset described in <design issue/RFC>. The supported compatibility
contract is <versioned matrix>; unlisted AppArmor features are not claimed.

### Current Status

- Existing Asterinas infrastructure: <source links>
- Missing prerequisites: <source links>
- Pinned Linux/AppArmor oracle: <version/commit>

### Implementation Plan

- [ ] Contract and golden compatibility corpus: #...
- [ ] Bootable unconfined LSM skeleton: #...
- [ ] Capability enforcement vertical slice: #...
- [ ] Path and exec-domain mediation: #...
- [ ] File mediation closure: #...
- [ ] Policy/control ABI and audit: #...
- [ ] x86_64 experimental pilot: #...

### Acceptance Criteria

- <observable behavior and differential tests>
- <fail-closed unsupported input behavior>
- <documentation and CI gates>

### Non-goals

- <explicitly unsupported feature classes and deployment claims>

### Additional Information

- Design: #... / RFC-....
- Compatibility matrix: ...
- Test environment: ...
```

最终风格应学 #3696 的“每阶段独立能力 + atomic scope + test plan”，学 #3586 的“minimal first + after main PR + outside scope”，学 #3648 的“Linux ABI + lifecycle + non-goals + validation”；不需要复制它们的篇幅。

