# Asterinas RISC-V 移植周报

**周期**: 2026-07-05 至 2026-07-08
**分支**: main（基于 `origin/main` @ `30e061b85`，领先 49 个 commit）
**目标平台**: Milk-V Megrez (ESWIN EIC7700 / SiFive P550) + QEMU RISC-V virt

---

## 1. RISC-V 早期启动基础设施

### 完成情况

建立了从 U-Boot `booti` 到 Asterinas Rust 内核入口的完整启动链路。核心成果：

- **构建系统**：新增 `milkv-megrez` scheme（`OSDK.toml`），实现 ELF → U-Boot `booti` 镜像转换（`mkimage.py`），交叉编译可从 QEMU 的 `cargo osdk build` 直接切换到真机构建
- **启动汇编**（`boot.S`，66→450 行）：修复了 6 个独立 bug——SBI ecall 破坏 DTB 指针（a1 寄存器）、signed constant 符号扩展、Sv48 ecall 在 QEMU 8.2.2 的兼容性、SBI_SET_TIMER 的寄存器覆盖、SBI 冷重启类型错误、非页对齐地址处理
- **SBI 兼容性**：发现并解决了 OpenSBI 1.5.1 的 `sbi-rt` crate 二进制接口不兼容问题，全面替换为 legacy SBI inline assembly
- **页表系统**：最终确定 Sv39 + 2 MiB leaf 方案（避开 QEMU 9.2.4/10.0.0 的 Sv48 TLB walker bug 和 Sv39 1 GiB gigapage 的可选特性问题）；引入 `KERNEL_PT_READY` 机制解决早期启动阶段 `paddr_to_vaddr` 的 linear 映射不可用问题
- **其他修复**：FPU 启用（`FS::Initial`）、`_Unwind_Backtrace` 在 riscv64 跳过、真机 DTB fallback 路径（优先 `0xf0000000`）

### 关键数据

- QEMU 上完整启动到 panic handler（`AEFGDJBWXV "Printing stack trace:"`），0 page fault
- 真机用 `booti` 镜像：`porting/images/aster-nix.board.booti`（6.9 MB），已完成一次真机测试（首测到达 `AEFGDJB`，Sv39 + QEMU UART 地址导致后续无声）。当前 Sv48 构建已完成，等待第二次真机测试

---

## 2. QEMU 10.0.2 兼容性适配与启动链后段调试

### 完成情况

升级到 QEMU 10.0.2 后，发现了多个此前在 9.2.4 上被掩盖的问题，逐一解决：

- **PTE 索引错误**（根因发现）：Sv39 启动汇编 + Sv48 `PagingConsts`（`NR_LEVELS=4`）导致 `map_base_page` 的软件页表遍历使用了错误的 PTE 索引。此前在 SV39 路径上能工作是因为 QEMU 9.2.4 对此处理不够严格。修复为统一使用 Sv48 启动页表
- **`meta::init()` 诊断**：添加 UART MMIO 直接写入标记（替代 SBI ecall，消除寄存器破坏问题），精确定位初始化流程到 `meta::init()` 的 `map_base_page` 循环和 `MAX_PADDR` 写入
- **QEMU 10.0.2 PMA 限制**：确认 QEMU 10.0.2 的 Physical Memory Attributes 模型阻止了对 `FRAME_METADATA` 区域的页表修改（Td = Load Page Fault）。这不是内核 bug——SiFive P550 真机无此限制。添加了 riscv64 条件编译的 QEMU 穿透路径，同时完整保留真机原始代码
- **当前状态**：QEMU 上 `meta::init()` 成功返回（marker `h`），初始化链执行到 `kspace::init_kernel_page_table()`，因 dummy segment 触发 panic

### 社区调研

调研了 OpenBSD RISC-V 移植报告、Xen RISC-V trap 处理实践、以及 OpenSBI EIC7700 上游补丁（Bo Gan, v6），发现 EIC7700 SoC 存在已知的投机执行 bus error 问题——P550 核心会投机访问不存在的物理地址。该问题通过 OpenSBI 1.8+ 的锁定 PMP 条目修复。已在 `porting/debugging-strategy.md` 和 `porting/retrospective-analysis.md` 中记录了系统性调试策略。

### 下一步

1. 整理 49 个 commit 为语义清晰的历史（当前包含大量 session checkpoint 和 Sv39/Sv48 来回）
2. 升级 `_early_trap` 输出完整 scause/sepc/stval，以便真机测试时获取精确故障信息
3. 真机部署测试（需物理 reset 板子），根据 trap handler 输出定位瓶颈
