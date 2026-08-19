## 动机和试点

AppArmor 可以为 Asterinas 提供兼容 Linux 的应用约束机制，并成为 capability 和 Yama 之外，LSM 框架的第二个端到端使用者。

首个 QEMU 试点保持最小：一个程序在单个 profile 下完成一项允许操作，并在执行一项选定的 capability 操作时被拒绝；任务标签、errno 和审计结果都可验证。未选择 AppArmor 时，同一镜像保持 unconfined 基线。真实 NixOS 服务或容器工作负载留到 M5 由社区选定。

## Asterinas 当前基础

| 已有 | 缺少 |
| --- | --- |
| [<code>lsm=</code>/<code>security=</code> 选择](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/modules/mod.rs)、强制 capability 和可选 Yama | 面向高层 LSM 组件的公开注册/生命周期 seam |
| [capability 和 alien-access hook](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/hooks/mod.rs) | AppArmor 任务标签及 exec/file hook |
| 每线程凭据和集中的 exec prepare/commit | 二进制策略解码器和 AppArmor/securityfs 节点 |
| <code>Path = Mount + Dentry</code>、通用 VFS 路径、文件句柄和 mmap 后备文件 | AppArmor 审计传输和兼容性测试 |
| 启动参数和 QEMU 回归测试 | — |

[内核 crate 规范](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/coding-guidelines/for-maintainability/rust-specific/crates-and-modules.md)要求高层子系统默认位于 <code>aster-core</code> 之外；低层需要高层行为时，由低层定义接口。

## 建议架构

~~~text
kernel/core/src/security/lsm/          kernel/comps/apparmor/
--------------------------------      ------------------------------
通用 hook 上下文                 ───▶ 标签和 profile
注册/生命周期 seam                    规则匹配和决策
最小不透明 task/file 状态              ABI 解码器和策略快照
clone/exec 回调                        审计和 hook adapter
                │
                └──── 由 kernel/src 装配；distro/ 和测试提供用户态支持
~~~

~~~text
内核操作 → 通用 hook → AppArmor 标签/规则
         → enforce 或 complain → 审计 → allow 或 errno
~~~

AppArmor 始终是附加限制，不能覆盖 DAC 或 capability 的拒绝。内核代码继续使用安全 Rust，不复制或机械翻译仅采用 GPL 许可的 Linux/AppArmor 实现代码。第一个 seam 改动不迁移 capability 或 Yama。

| Asterinas 自带 capability LSM | AppArmor capability 规则 |
| --- | --- |
| 检查任务凭据是否包含所需 capability | 检查任务当前 profile 是否允许使用该 capability |
| 必须通过的基础权限检查 | 额外的强制访问控制限制 |

两者都允许时操作才能继续。AppArmor 不能授予任务缺少的 capability；任务拥有 capability 也不能绕过 AppArmor 的拒绝。

<details>
<summary>为什么不把 AppArmor 全部放入 aster-core？</summary>

全部放入核心可以避免跨 crate seam，但会把策略和 ABI 复杂性带入核心，也不符合 #3601 的方向。核心只保留通用 hook，可以让 AppArmor 独立替换和评审。

</details>

## 分阶段实现路线图

| 里程碑 | 交付物 | 退出门槛 |
| --- | --- | --- |
| **M1——可运行的 capability 约束** | 固定契约；增加 LSM seam 和 AppArmor 组件；解码/加载 capability-only 官方 blob；在 exec 时附着一个 profile；实现标签继承、模式、审计、capability allow/deny、禁止 unconfined 回退和最小 ptrace 安全检查。 | 在 QEMU 中，固定 parser 的策略可以加载，目标在新镜像运行前进入 profile，允许操作成功，选定 capability 操作按照 Linux 对照返回 errno 和审计记录；未选择 AppArmor 时基线不变。 |
| **M2——文件生命周期** | 在副作用前仲裁 open、create、mutation、描述符传递、映射和必要限定符。 | 已公布文件功能组不存在未仲裁的操作或限定符。 |
| **M3——exec 转换** | 在 M1 精确路径附着基础上增加原子 <code>ix</code>、非回退 <code>px</code>、跨 profile 转换和更完整的 ptrace 规则。 | 新镜像只在目标标签提交后运行，绝不回退 unconfined。 |
| **M4——策略控制面** | 补齐所需 securityfs ABI、原子 replace/remove、状态查询和审计传输。 | 固定工具链可管理有效策略；无效变更原子失败；标签可查询。 |
| **M5——系统试点** | 在内核外构建缓存、早期加载，并通过非回退转换约束一个经确认的工作负载。 | 行为可在 QEMU 复现，并明确标记为实验性。 |

希望维护者重点评审 LSM/组件划分和 capability-first M1。

<details>
<summary>参考资料</summary>

- [Linux 内核 AppArmor 文档](https://docs.kernel.org/admin-guide/LSM/apparmor.html)
- [AppArmor 用户态项目和 parser](https://gitlab.com/apparmor/apparmor)
- [Asterinas LSM 框架](https://github.com/asterinas/asterinas/tree/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm)
- [Asterinas 内核 crate 规范](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/coding-guidelines/for-maintainability/rust-specific/crates-and-modules.md)
- [Asterinas RFC 流程](https://github.com/asterinas/asterinas/blob/main/book/src/rfcs/0001-rfc-process.md)
- [Asterinas 内核组件提案 #3601](https://github.com/asterinas/asterinas/issues/3601)

</details>
