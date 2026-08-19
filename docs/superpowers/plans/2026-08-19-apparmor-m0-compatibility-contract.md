# AppArmor M0 Compatibility Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task by task.

**Goal:** 在不修改内核执行路径的前提下，冻结首个 AppArmor 兼容子集，并产出可复现的策略语料与 Linux 对照结果，为 M1 的 unconfined 骨架提供稳定契约。

**Architecture:** M0 只建设仓库外缘的兼容性夹具和文档。官方 <code>apparmor_parser</code> 在宿主或参考 Linux 虚拟机中生成二进制策略；Linux 参考环境给出外部可观察结果；Asterinas 尚不加载或执行这些策略。M1 以后只消费 M0 已冻结的输入和预期，不反向修改契约来迁就实现。

**Tech Stack:** POSIX shell、官方 AppArmor parser、Linux 参考虚拟机、Asterinas Book、现有 Make/Docker 开发流程。

**Spec:** <code>APPARMOR_ASTERINAS_COMMUNITY_ISSUE_EN.md</code>

## Global Constraints

- 先取得维护者对组件布局、capability-first、评审责任和 RFC 要求的明确意见；未达成一致前不修改 LSM 核心。
- M0 不增加 AppArmor 内核模块、hook、securityfs 节点或策略加载入口。
- 只使用未修改的官方 parser；内核不解析源策略文本。
- 不复制或机械翻译 Linux/AppArmor 的 GPL 实现代码。
- 二进制策略从第一天起按不可信输入处理。
- 首轮运行目标为 x86_64；其他架构只要求测试夹具可移植，不宣称已支持。
- 首个契约只覆盖 unconfined 基线和 capability 垂直切片所需的最小 ABI。
- 每项 feature 只有在对应语义完整且有差分证据时才能公布。
- M2 需要消费真实的官方 capability blob：先用 typed policy snapshot 验证决策内核，再增加 capability 子集的严格解码和一次性加载；M5 负责动态替换、查询及完整控制面，不能让 M2 依赖硬编码策略来冒充 ABI 兼容。

---

## Task 1: 通过社区设计门槛

**External artifact:** GitHub design-proposal issue

1. 提交英文设计 Issue，但不要同时创建 M0–M6 的空子 Issue。
2. 请求维护者明确回答以下四项：
   - 通用 LSM seam 位于 <code>aster-core</code>，AppArmor 实现位于 <code>kernel/comps/apparmor</code>；
   - capability 是否作为第一个垂直切片；
   - 是否需要正式 RFC；
   - 通用 LSM、exec/VFS、用户态 ABI 分别由谁评审。
3. 若维护者要求 RFC，先完成 RFC；Issue 路线图继续作为实现跟踪入口。
4. 仅在架构与 M0 边界得到确认后进入 Task 2。

**Exit gate:** Issue 中存在可引用的维护者结论，不以“无人反对”代替批准。

---

## Task 2: 建立版本与 ABI 决策记录

**Files:**

- Create: <code>tools/apparmor/README.md</code>
- Create: <code>tools/apparmor/reference-versions.env</code>
- Create: <code>book/src/kernel/linux-compatibility/apparmor.md</code>
- Modify: <code>book/src/SUMMARY.md</code>

1. 从获批后的最新 Asterinas main 创建独立分支，并记录精确 commit：

~~~bash
git fetch origin main
git switch -c apparmor-m0 origin/main
git rev-parse HEAD
~~~

2. 在 <code>reference-versions.env</code> 中记录：Asterinas commit、Linux tag、AppArmor parser tag、目标架构和策略 feature ABI。所有值必须是精确 tag、commit 或 feature 集，不能使用 <code>latest</code>。
3. 以 Asterinas 当前 kselftest 所用 Linux 版本为首选 Linux 基线；对 AppArmor 4.1 稳定系列和 5.0 系列各编译同一份最小策略。选择规则是：满足 capability 契约的更小 feature ABI 优先，而不是版本号更新者优先。
4. 在兼容性页面记录选择依据、未选择版本及原因、许可证边界和升级条件。
5. 将页面加入 Book 目录。

**Verification:**

~~~bash
rg "latest|master|main" tools/apparmor/reference-versions.env
make docs
~~~

Expected: 第一条命令无输出；Book 构建成功。

**Commit:**

~~~bash
git add tools/apparmor book/src/kernel/linux-compatibility/apparmor.md book/src/SUMMARY.md
git commit -m "Document the initial AppArmor compatibility baseline"
~~~

---

## Task 3: 建立最小策略语料库

**Files:**

- Create: <code>tools/apparmor/profiles/unconfined-baseline.apparmor</code>
- Create: <code>tools/apparmor/profiles/capability-allow.apparmor</code>
- Create: <code>tools/apparmor/profiles/capability-deny.apparmor</code>
- Create: <code>tools/apparmor/profiles/capability-complain.apparmor</code>
- Create: <code>tools/apparmor/profiles/invalid-truncated.apparmor</code>
- Create: <code>tools/apparmor/corpus.tsv</code>

1. 先定义 <code>corpus.tsv</code> 的固定列：用例名、源策略、预期 parser 结果、预期运行模式、预期判定、预期 errno、预期审计类别。
2. 只加入五类输入：unconfined、allow、deny、complain 和一个必然无效的输入。不要在 M0 提前加入 exec、file、namespace 或策略替换语法。
3. allow、deny、complain 必须针对同一个 capability 操作，以便后续只比较策略模式，而不是比较不同系统调用。
4. 无效输入应在 parser 阶段或二进制验证阶段确定性失败，并在矩阵中注明失败层级。

**Verification:**

~~~bash
test "$(find tools/apparmor/profiles -maxdepth 1 -type f | wc -l)" -eq 5
test "$(cut -f1 tools/apparmor/corpus.tsv | sort | uniq -d | wc -l)" -eq 0
~~~

Expected: 两条命令退出状态均为 0。

**Commit:**

~~~bash
git add tools/apparmor/profiles tools/apparmor/corpus.tsv
git commit -m "Add the minimal AppArmor policy corpus"
~~~

---

## Task 4: 生成可复现的 parser 产物

**Files:**

- Create: <code>tools/apparmor/generate-corpus.sh</code>
- Create: <code>tools/apparmor/check-corpus.sh</code>
- Modify: <code>tools/apparmor/README.md</code>

1. 先实现 <code>check-corpus.sh</code>，使其在缺少 blob、manifest、parser 版本或校验和不匹配时失败。
2. 运行检查并确认失败：

~~~bash
tools/apparmor/check-corpus.sh
~~~

Expected: 非零退出，并指出缺失的生成产物。

3. 实现最小生成脚本，接口固定为：

~~~text
generate-corpus.sh PARSER FEATURE_DIR OUTPUT_DIR
~~~

脚本只做四件事：校验 parser 版本、编译有效策略、确认无效输入失败、写出包含 SHA-256 的 manifest。输出目录不得提交仓库；仓库只提交源策略、期望值和生成方法。
4. 使用固定工具链生成两次，并比较 manifest：

~~~bash
tools/apparmor/generate-corpus.sh "$PARSER" "$FEATURE_DIR" /tmp/aa-corpus-a
tools/apparmor/generate-corpus.sh "$PARSER" "$FEATURE_DIR" /tmp/aa-corpus-b
diff -u /tmp/aa-corpus-a/manifest.tsv /tmp/aa-corpus-b/manifest.tsv
tools/apparmor/check-corpus.sh /tmp/aa-corpus-a
~~~

Expected: 两次 manifest 完全一致，检查通过。

5. README 只记录依赖、命令、输出格式和失败含义，不包装 parser 为新的策略语言。

**Commit:**

~~~bash
git add tools/apparmor
git commit -m "Add reproducible AppArmor policy corpus generation"
~~~

---

## Task 5: 建立 Linux 外部行为对照

**Files:**

- Create: <code>tools/apparmor/oracle/capability-probe.c</code>
- Create: <code>tools/apparmor/run-linux-oracle.sh</code>
- Create: <code>tools/apparmor/check-oracle.sh</code>
- Modify: <code>tools/apparmor/corpus.tsv</code>
- Modify: <code>tools/apparmor/README.md</code>

1. 选择一个在参考 VM 中可隔离、无持久副作用、能稳定触发指定 capability 的系统调用。选择结果写入 README，并说明为什么其普通权限前置条件可控。
2. 先实现单个 C probe。输出仅包含：操作名、返回值和 errno；每个策略用例在独立进程中运行。
3. 先实现 <code>check-oracle.sh</code>，要求每个有效策略用例都有：任务标签、返回值、errno、enforce/complain 结果和审计事件分类。
4. 在缺少对照结果时运行检查并确认失败。
5. 实现 <code>run-linux-oracle.sh</code>：加载生成策略、执行 probe、读取任务标签、收集对应审计事件、卸载策略，并写出稳定 TSV。脚本必须在开始前验证自己运行在固定参考内核和 parser 版本上。
6. 运行全部用例两次并比较规范化结果：

~~~bash
tools/apparmor/run-linux-oracle.sh /tmp/aa-oracle-a
tools/apparmor/run-linux-oracle.sh /tmp/aa-oracle-b
diff -u /tmp/aa-oracle-a/results.tsv /tmp/aa-oracle-b/results.tsv
tools/apparmor/check-oracle.sh /tmp/aa-oracle-a/results.tsv
~~~

Expected: allow 成功；deny 返回已记录的 Linux errno；complain 允许操作但产生对应审计；两次结果一致。

7. 将实际 Linux 结果写回 <code>corpus.tsv</code>，不凭记忆填写 errno 或审计字段。

**Commit:**

~~~bash
git add tools/apparmor
git commit -m "Add the Linux oracle for the AppArmor capability slice"
~~~

---

## Task 6: 冻结 M0 契约并评审

**Files:**

- Modify: <code>book/src/kernel/linux-compatibility/apparmor.md</code>
- Modify: <code>tools/apparmor/README.md</code>

1. 在兼容性页面中形成四张表：支持的 ABI、明确拒绝的 ABI、Linux 对照结果、后续里程碑依赖。
2. 明确记录 M1 可以依赖的最小契约：unconfined 默认标签、clone 标签继承、模块选择行为，以及 M2 所需的 capability 输入/输出。
3. 运行最终检查：

~~~bash
tools/apparmor/check-corpus.sh /tmp/aa-corpus-a
tools/apparmor/check-oracle.sh /tmp/aa-oracle-a/results.tsv
make docs
make check
git status --short
~~~

Expected: 所有检查通过；工作区只包含有意提交的 M0 改动。

4. 发起一个仅包含 M0 文档和工具的 PR。评审通过前不开始修改 <code>aster-core</code>。

**Exit gate:** 版本/ABI 固定、语料可重复生成、Linux 对照可重复运行、兼容矩阵获评审。

---

## M0 之后的开发顺序

后续每个阶段单独编写实现计划，不在 M0 创建未来脚手架：

1. **M1 — unconfined 骨架：** 只增加高层 LSM 注册 seam、AppArmor 空组件、任务初始化和 clone 继承；证明 <code>lsm=apparmor</code> 能启动且不改变基线行为。
2. **M2 — capability 垂直切片：** 先实现最小 label/profile/rule/mode/audit 决策链，再加入 capability ABI 子集的严格 blob 解码和一次性加载，复用 M0 Linux oracle 验证 allow、deny、complain 和错误策略；不在这里实现动态替换或完整 securityfs。
3. **M3 — exec 域：** 在 prepare/commit 边界实现原子附着、<code>ix</code>、非回退 <code>px</code> 和必要 ptrace 检查。
4. **M4 — 文件生命周期：** 按对象生命周期补齐 open/create/mutation/FD transfer/mmap 和已公布限定符；每增加一个功能组都先做 Linux 差分测试。
5. **M5 — 策略控制面：** 在已按功能组扩展的严格解码器之上，补齐固定 parser 所需的 securityfs/策略 ABI、原子替换、状态查询和审计传输。
6. **M6 — 系统试点：** 在内核外生成缓存并早期加载，对社区选定的真实工作负载执行非回退约束，保持实验性标记。

每个阶段只有在自己的退出条件满足并合入后，才开始下一阶段。M1–M6 可以拆成多个小 PR，但不能跨阶段把尚未验证的 ABI 一并暴露。
