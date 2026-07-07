# 回顾性分析与移植计划

> 基于对 40 个 RISC-V port commit 的逐一分析，结合上游代码现状和社区调研。
> 撰写日期：2026-07-07

## 1. 代码现状盘点

### 1.1 Git 拓扑

```
origin/main ─── 30e061b85 (refactor asid allocate)
                    │
                    ├── 40个 RISC-V port commit
                    │   (c23790c2c → 0c31459ae)
                    │
               local/main @ 0c31459ae

worktree-chip-knowledge @ origin/main (没有 RISC-V 代码)
```

**关键发现**：
- `origin/main` 在 fork 点 (`30e061b85`) 之后**没有任何新 commit**——上游没有前进。这意味着 merge/rebase 不会产生冲突（已 dry-run 验证）。
- 两个 stash (`stash@{0}`, `stash@{1}`) 是旧的 WIP session 残留，内容已被后续 commit 覆盖，可以删除。

### 1.2 修改的文件（kernel/OS 层面）

| 文件 | 改动性质 | 是否需要向上游提交 |
|------|---------|-----------------|
| `ostd/src/arch/riscv/boot/boot.S` | **重写**：66行→450行 | ✅ 核心提交 |
| `ostd/src/arch/riscv/boot/mod.rs` | **扩展**：+KERNEL_PT_READY, +DTB fallback, +diagnostic markers | ✅ 核心提交 |
| `ostd/src/arch/riscv/mod.rs` | +FPU enable, +late_init_on_bsp 实现 | ✅ |
| `ostd/src/arch/riscv/mm/mod.rs` | +KERNEL_PT_READY 集成（paddr_to_vaddr 中的条件判断） | 上游可能已有更好方案 |
| `ostd/src/arch/riscv/serial.rs` | 重写为 legacy SBI putchar | ✅ |
| `ostd/src/arch/riscv/qemu.rs` | QEMU exit 适配 | ✅ 小改动 |
| `ostd/src/lib.rs` | 2处改动：fill_early_info 提前+ feature gate 重排 | ⚠️ 需小心合并 |
| `ostd/src/mm/kspace/mod.rs` | paddr_to_vaddr 中加 identity 映射回退 + KERNEL_PT_READY 标记 | ⚠️ 需重审 |
| `ostd/src/panic.rs` | riscv64 跳过 _Unwind_Backtrace | ✅ |
| `ostd/src/mm/io.rs` | 小改动 | ⚠️ |
| `kernel/src/thread/oops.rs` | riscv64 跳过 print_stack_trace | ✅ |
| `kernel/src/lib.rs` | 小改动 | ⚠️ |

### 1.3 Commit 分类

| 类别 | 数量 | 说明 |
|------|------|------|
| **A. 基础设置** | 2 | OSDK scheme、辅助脚本、文档结构 |
| **B. 启动汇编修复** | 5 | SBI timer、signed constant、DTB 验证、MMU 入口 |
| **C. MMU/页表实验** | 10 | Sv39/Sv48 反复切换、1GiB→2MiB、KERNEL_PT_READY |
| **D. SBI/串口修复** | 2 | legacy SBI putchar、替换 sbi-rt |
| **E. Panic/FPU 修复** | 3 | Unwind skip、FPU Initial |
| **F. Session 总结** | 15 | checkpoint commit，每次调试会话的状态记录 |
| **G. 额外 Bug 修复** | 3 | a1 寄存器保存、Sv39 gigapage 修复、duplicate panic skip |

**问题**：类别 C 的 10 个 commit 中有大量"试了 Sv48 → 换回 Sv39 → 又试 Sv48 → 再换回 Sv39"的来回。这些在最终版本中应该被 squash 掉，只保留最终正确的决策链。

---

## 2. 上游 RISC-V 已有代码 vs 我们的改动

### 2.1 上游已有的东西

`origin/main` 上已经有 30 个 RISC-V 相关 commit，包括：
- 基础的 RISC-V 平台框架（`arch/riscv/mod.rs`, `mm/mod.rs`, `boot/mod.rs`, 等）
- 最小的启动汇编（66 行 boot.S，基本的页表设置）
- `paddr_to_vaddr` = `pa + LINEAR_MAPPING_BASE_VADDR`
- FPU 默认关闭 (`FS::Off`)
- 已注释掉的 `init_cvm_guest` 和 `interrupts_ack`

### 2.2 我们的改动 vs 上游的差异点

| 上游做法 | 我们的做法 | 为什么不同 |
|---------|-----------|----------|
| 66行 boot.S，基本 Sv39 | 450行 boot.S，完整4级页表+identity+DTB验证+SBI timer watchdog | 上游的版本不够健壮，无法在 QEMU 8.2.2/9.2.4/10.0.0 上稳定启动 |
| `sbi-rt` crate | legacy SBI inline asm | OpenSBI 1.5.1 的 `sbi-rt` 二进制接口不兼容 |
| FPU 默认关闭 | FPU 开 `FS::Initial` | 避免 lazy FPU 陷阱 |
| paddr_to_vaddr 始终用 linear offset | 启动早期用 identity，KERNEL_PT_READY 后切 linear | 启动页表不保证 linear 映射（QEMU 9.2.4 Sv48 bug） |
| `_Unwind_Backtrace` 无保护 | riscv64 跳过 | OpenSBI 1.5.1 上触发 illegal instruction |

---

## 3. 真正的问题是什么？

回顾整个 git 历史，**核心难点不是技术本身，而是反馈循环的失效**。

### 3.1 历史中的主要阻塞事件

| 事件 | 症状 | 耗时 | 根因 |
|------|------|------|------|
| `sbi-rt` ecall 无限循环 | 启动后无输出 | 多个 debug session | OpenSBI 1.5.1 的二进制接口版本检查不匹配 sbi-rt crate |
| Sv48 1 GiB leaf PTE | 页表设置后无声 | 多个 session | QEMU 9.2.4/10.0.0 TLB walker bug |
| 1 GiB gigapage in Sv39 | Rust 入口后 page fault | 多个 session | QEMU 8.2.2 不支持 Sv39 gigapage（可选特性） |
| `fill_early_info` 调用时机 | T5 寄存器被破坏 | 反复测试 | memory_regions 在 early allocator 之前被读取 |
| `a1` 被 SBI ecall 破坏 | DTB fallback 路径失败 | 真机测试受阻 | SBI spec: ecall 返回 `sbiret {a0, a1}` |
| FPU 未启用 | float 指令触发 illegal_instruction | QEMU 启动后崩溃 | `FS::Off` 依赖未实现的 lazy FPU 陷阱 |

**共性**：所有这些问题的诊断都因为缺乏有效反馈而极度困难。每次都需要反复的"猜 → 构建 → 跑 QEMU → 看 marker → 猜错 → 重来"。

### 3.2 为什么「换页表模式」不是解决方案

你在对话中明确指出不认可「尝试换不同页表」的做法，这是对的。回顾历史，Sv39↔Sv48 的来回切换（10个 commit）本质上是**在试探 QEMU 的 bug 边界**，而不是在解决 Asterinas 的内核问题。最终确认的是：

- **Sv39 works**（QEMU 和理论上的真机）
- **Sv48 在 QEMU 9.x/10.x 上有 TLB walker bug**（不是我们的代码问题）
- **Sv39 的 1 GiB gigapage 是可选特性**（部分 QEMU 版本/硬件不支持）

这三个事实是可重用的知识。真正影响真机启动的未知数不是页表模式，而是：
1. **EIC7700 的投机执行 + OpenSBI PMP 保护状态**
2. **`meta::init()` 内部的崩溃原因**（对真机 DTB 返回的 memory regions 的处理）
3. **真机 UART (`0x50900000`) 的初始化**

---

## 4. 接下来应该做什么

### 4.1 第一阶段：整理 Commit 和分支（当前任务）

**目标**：把 40 个 commit 整理成一个干净的、有意义的、可读的历史。

**方案 A（rebase + squash）：保留最小 commit 集**

```
c23790c2c  porting: add Milk-V Megrez scheme, scripts, and docs
fb8cf161b  fix: rebuild board ELF after QEMU run
5a58e51fa  riscv/boot: add SBI timer watchdog and fix phys/virt address handling
0c998bb39  riscv/boot: use cold reboot (type 1) and fix signed-constant bugs
c1ed6df3e  riscv/boot: fix SBI_SET_TIMER clobber and DTB validation
8c6a576bc  riscv/boot: fix MMU setup and DTB parsing for QEMU 8.2.2
4d36d831a  riscv/boot: fix QEMU 8.2.2 ecall bug and add UART diagnostic marker
bdf9aa266  riscv/boot: add identity-mapped Rust entry point
0b748e0a4  riscv/boot: implement Sv39 page tables with identity DTB access
6c164cefc  riscv/boot: use 2 MiB leaves instead of 1 GiB gigapages
62202d091  riscv/boot: call fill_early_info before memory region access
4cf19c2a2  riscv/boot: use Sv39-only to reach kernel panic handler
  ^^^ squash 09c62f0c9, 0511a7d5c, 4f338b3a1 into this
  (the Sv48 experiments were exploratory — keep the final decision only)
1229357c9  riscv/serial: use legacy SBI putchar instead of sbi-rt
76ba59335  riscv: replace all sbi-rt calls with legacy SBI inline assembly
389adb42c  riscv: enable FPU by setting FS::Initial instead of FS::Off
808b70ca5  riscv/panic: skip _Unwind_Backtrace on riscv64
29bcb4e25  riscv/boot: preserve DTB pointer across SBI ecall
0838da20d  riscv/boot: use non-leaf pointer for kernel mapping in Sv39
```

**15 个 session summary commit → 全部 squash 到对应的 fix commit 中**。

结果是约 18 个语义清晰的 commit，每个解决了**一个具体问题**，有明确的 commit message 解释"为什么"。

**方案 B（merge）：保留原始历史作为证据**

如果原始历史有保留价值（debug 记录），可以创建一个 `riscv-port-history` 分支保存完整历史，然后在 `main` 上 squash merge。

### 4.2 第二阶段：补足诊断信息

在整理完 commit 后，在当前功能代码中添加关键诊断点：

1. **升级 `_early_trap`**（boot.S）：输出完整 scause/sepc/stval，参考 `porting/debugging-strategy.md` 中的汇编实现
2. **在 `meta::init()` 内部加 marker**：精确定位崩溃位置（info!() 前？alloc_meta_frames 中？）
3. **在 QEMU 中用 `-d mmu,cpu,int` 验证**

这些诊断点应该是**永久性的**（用 `#[cfg(debug_assertions)]` 或 feature gate 控制），不是临时 marker。

### 4.3 第三阶段：真机测试

1. **确认 OpenSBI 版本**：在 U-Boot 中 `sbi info`，确认是否 ≥ 1.8（有 EIC7700 PMP 补丁）
2. **部署 + 收集完整输出**：用 `serial_boot_asterinas.py` 捕获
3. **分析故障点**：如果 `_early_trap` 被触发，scause/sepc/stval 会精确告诉我们是 page fault、illegal instruction、还是 bus error
4. **根据结果决定下一步**：
   - Page fault → 页表或 DTB memory region 问题
   - Illegal instruction → 编译或 CSR 权限问题
   - Bus error → EIC7700 投机执行 + 需要 PMP 保护
   - 完全无声 → 串口问题或更早期的崩溃（SBI 本身的问题）

### 4.4 第四阶段：向上游提交

等 QEMU 上完整启动（到 panic handler）、真机上至少到 `meta::init()` 之后，可以考虑向上游提交。需要：

1. 将 diagnostic marker 改为 `#[cfg(debug_assertions)]` 或 `trace` level log
2. 确保所有 `#[cfg(target_arch = "riscv64")]` 改动不影响 x86 构建
3. 从 upstream 接受的 PR/Issue 看他们的 RISC-V 支持进展（已有 PR #924 和 Issue #1954）

---

## 5. 关于真机启动的可能性的判断

### 5.1 你问：当前进展是否支持真机正常启动？

**答：基础路径已通，但大概率会卡在某个点上。**

理由是：

**已确认能穿过的最难关卡**（QEMU 上验证）：
- OpenSBI → S-mode 入口 ✅
- DTB 解析 ✅
- Sv39 页表构建 + MMU 开启 ✅
- Identity → VMA 映射切换 ✅
- Rust 入口 `riscv_boot` + `Fdt::from_ptr()` ✅
- FPU 启用 + `fill_early_info` + `serial::init` + `logger::init` + `cpu::init_on_bsp` ✅
- `meta::init()` 入口 ✅

**真机上的未知数**：
- DTB 内容不同（memory regions 布局、CPU 数量、UART 基地址）
- EIC7700 的投机执行（如果 OpenSBI PMP 不够新）
- IO 设备初始化（PLIC、timer、UART MMIO）

**真机最可能通过到哪儿？**
- 乐观：到 `meta::init()` 内部的 `info!()` 输出
- 中等：在 `init_kernel_page_table()` 或 `late_init_on_bsp` 中崩溃
- 悲观：MMU 开启后立刻 trap（EIC7700 投机执行触发 bus error）

但无论哪种情况，升级后的 `_early_trap` 会告诉我们确切原因。

---

## 6. 建议的执行顺序

```
现在（第一阶段）:
  1. 清理 stash（两个都是旧 WIP，删除）
  2. 整理 commit 历史（squash session summaries，消除 Sv39↔Sv48 来回）
  3. 升级 _early_trap 输出 scause/sepc/stval
  4. 在 meta::init() 中添加诊断 marker

然后（第二阶段）:
  5. QEMU 中用真实 Megrez DTB 测试（-d mmu trace）
  6. 真机部署测试
  7. 根据 trap handler 输出定位问题

最终（第三阶段）:
  8. 清理诊断代码，准备向上游提交
  9. 文档化所有发现和解决方案
```

---

## 7. 风险与未知数

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| 上游 RISC-V 代码和我们冲突 | 低（目前无分歧） | 中 | 定期 fetch origin/main，尽早发现 |
| EIC7700 PMP 问题 | 中 | 高（真机无法启动） | 先确认 OpenSBI 版本 |
| 真机 DTB 导致 memory region 解析失败 | 中 | 中 | QEMU 中用真实 DTB 预测试 |
| meta::init() 内存分配在真机上崩溃 | 中 | 中 | 加诊断 marker 精确定位 |
| KERNEL_PT_READY 的 identity 回退路径不安全 | 低 | 高（安全问题） | 上游提交前重审 |
| diagnostic marker 太多使代码不整洁 | 低 | 低 | 用 feature gate 控制 |
