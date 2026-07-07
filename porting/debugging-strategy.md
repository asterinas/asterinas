# RISC-V 真机启动调试策略

> 基于社区实践（OpenBSD RISC-V 移植、Xen RISC-V trap 处理、OpenSBI EIC7700 补丁）
> 整理的系统性调试方案。

## 1. 当前状态

### 1.1 已达成的里程碑

| 里程碑 | 状态 | 说明 |
|--------|------|------|
| QEMU Sv39 完整启动 | ✅ | `AEFGDJBWXV` → `[Ostd panic]`，0 page fault |
| QEMU Sv48 | ❌ | QEMU 9.2.4/10.0.0 TLB walker 有 bug（非 kernel 问题） |
| 真机 Megrez 启动 | ❓ | 未确认，等待物理 reset 后测试 |

### 1.2 已解决的构建/环境问题

见 `porting/issues/01-11-*.md`，包括：
- VDSO、initramfs 占位、全局分配器冲突（需 `OSDK_LOCAL_DEV=1`）
- `let_chains` 不稳定特性、U-Boot autoboot 配置
- SBI `sbi-rt` 二进制接口不兼容 → 改用 legacy SBI inline assembly
- FPU 未启用 → `FS::Initial` 替代 `FS::Off`
- `_Unwind_Backtrace` 在 riscv64 触发 illegal instruction → 跳过

### 1.3 核心未解决问题

**真机（Milk-V Megrez / ESWIN EIC7700）上 Asterinas 启动到 `AEFGD` 后挂起**。
下一个标记 `H`（Sv48 SATP 验证）或 Sv39 的对应标记从未出现。

---

## 2. 这个问题为什么难？

### 2.1 观察性悬崖

```
A  → OK，我们能输出
E  → OK，trap handler 安装成功
F  → OK，satp 写入 0
G  → OK，satp 读出成功
D  → OK，页表项写入
   ——— 悬崖 ———
?  → 不知道发生了什么。页表结构错误？PTE 位缺失？TLB 失效？
      唯一反馈 = 没有反馈。
```

### 2.2 组合爆炸

| 维度 | 可能取值 |
|------|---------|
| 页表模式 | Sv39, Sv48 |
| 叶子大小 | 1 GiB, 2 MiB |
| 非叶子 PTE 位 | PTE_V, PTE_V\|A, PTE_V\|A\|D |
| 映射模式 | identity, high-half, linear 转换时机 |
| 执行环境 | QEMU 9.2.4, QEMU 10.0.0, 真机 SiFive P550 |

QEMU 和真机行为**不同**——QEMU 上成功的配置在真机上可能挂；真机上挂了
在 QEMU 上无法复现。

### 2.3 反馈周期长

```
修改代码 → 交叉编译 → scp 到 board → cp 到 /boot → 物理 reset (人工)
→ U-Boot → booti → 观察输出（可能只有几个字母）→ 分析...
```
一个循环 10-20 分钟，有效信息可能只有 0 bit。

---

## 3. 社区调研：关键发现

### 3.1 EIC7700 投机执行 Bug（OpenSBI 上游补丁）

**来源**: [Bo Gan, OpenSBI v6 patch series — Initial ESWIN/EIC7700 support](https://patchwork.ozlabs.org/project/opensbi/cover/20251218104243.562667-1-ganboing@gmail.com/)

ESWIN EIC7700 SoC 的 **SiFive P550 核心** 有一个已被充分记录的硬件 quirk：
投机执行和硬件预取器会访问不存在的物理地址，触发总线错误。

```
典型错误:
  bus error of cause event: 9, accrued: 0x220,
  physical address: 0x24ffffffa0
```

**根本原因**：EIC7700 的双 die 内存映射。单 die 配置下，远端 die 的区域变成"洞"，
CPU 的投机引擎不知道，仍然会尝试访问。

**危险区域（Die 0 单 die 配置）**：

| 地址范围 | 描述 |
|---------|------|
| `0x1a00_0000 – 0x1a40_0000` | Die 0 L3 零设备（可缓存，可能触发 bus error） |
| `0x3a00_0000 – 0x3a40_0000` | Die 1 L3 零设备（可缓存，单 die 下不存在） |
| Memory Port 中的空洞 | 远端 die 的缓存内存 + 间隙 |

**OpenSBI 修复方案**：用**锁定的 PMP（Physical Memory Protection）条目**物理阻断
这些危险区域。补丁已合并到 OpenSBI 1.8+。

**对你的影响**：
- Asterinas 运行在 S-mode，依赖下层 OpenSBI 配置正确的 PMP
- 如果板子上的 OpenSBI 版本 < 1.8，这些区域没有被保护
- MMU 开启后 CPU 投机取指可能触发 bus error，表现为**无声挂起**

验证方法 — 在 U-Boot 中运行：

```
sbi info
```

查看 OpenSBI 版本。

**EIC7700 完整内存映射**：

```
0x0000_0000 – 0x2000_0000    Die 0: P550 内部
0x2000_0000 – 0x4000_0000    Die 1: P550 内部
0x4000_0000 – 0x6000_0000    Die 0: 低 MMIO (System Port 0)
0x6000_0000 – 0x8000_0000    Die 1: 低 MMIO (System Port 0)
0x8000_0000 – 0x10_8000_0000 Die 0: 缓存 DRAM (Memory Port)
0x20_0000_0000 – 0x30_0000_0000 Die 1: 缓存 DRAM
0x40_0000_0000 – 0x60_0000_0000 交错内存（缓存）
0x80_0000_0000 – 0xa0_0000_0000 Die 0: 高 MMIO (System Port 1)
0xa0_0000_0000 – 0xc0_0000_0000 Die 1: 高 MMIO (System Port 1)
0xc0_0000_0000 – 0xd0_0000_0000 Die 0: 非缓存 DRAM
0xe0_0000_0000 – 0xf0_0000_0000 Die 1: 非缓存 DRAM
0x100_0000_0000 – 0x120_0000_0000 交错内存（非缓存）
```

### 3.2 "MMU 开启时刻"是所有 RISC-V OS 移植的标准难关

**来源**: [OpenBSD RISC-V Port — Final Report, SJSU](https://github.com/MengshiLi/openbsd-riscv-notes)

> "When the MMU is enabled, the program counter immediately points to an
> unmapped virtual address, causing a fault before the kernel can relocate
> itself."

**标准解决方案**（也是你已经在做的）：
1. 开启 MMU 之前插入 identity-mapped GigaPage
2. 确保当前物理地址在 MMU 开启后也作为虚拟地址有效

**OpenBSD 的额外步骤（你可能缺失的）**：
- 跳转到虚拟地址空间**之后立即删除临时 identity 映射**
  ```asm
  sd x0, (s8)        // 移除 identity PTE
  sfence.vma         // 刷新 TLB
  ```
- 在 Sv39 模式下，VPN[2] = bits [38:30] 用来索引 L1 页表

### 3.3 Trap Handler 应该输出完整 CSR 状态

**来源**: [Xen RISC-V: dump GPRs and CSRs on unexpected traps](https://patchew.org/Xen/f6f7ec863e92ade433f23ae0061391d2ef731f41.1768579139.git.oleksii.kurochko@gmail.com/)
和 [Tock OS improved RISC-V panic dump](https://github.com/tock/tock/pull/1575)

社区标准做法是在 unexpected trap 时 dump：

| CSR | 目的 |
|-----|------|
| `scause` | 为什么 trap（page fault / illegal inst / bus error） |
| `sepc` | 哪条指令触发了 trap |
| `stval` | 故障地址（page fault）或错误指令（illegal inst） |
| `satp` | 当前页表根地址和模式 |

再配合关键 GPR（ra, sp, a0-a7, t0-t6），可以在没有 JTAG 的情况下诊断绝大多数
启动问题。你需要的就是串口输出。

### 3.4 U-Boot 调试技巧

**来源**: [Milk-V Community](https://community.milkv.io/t/how-to-build-customized-uboot-so-that-i-can-run-privileged-instructions/3811)

在 U-Boot 中可以通过 `md` 命令 dump 内存，验证 kernel 是否被正确加载：

```
=> md 0x80200000 0x40     # dump 前 256 字节
=> md 0xf0000000 0x40     # dump DTB 头部，验证 magic = 0xd00dfeed
```

### 3.5 OpenBSD RISC-V 移植的 Sv39 页表结构参考

| 层级 | 条目数 | 每项覆盖 | 说明 |
|------|--------|---------|------|
| L1 (PGD) | 512 | 1 GiB (GigaPage) | VPN[2] = bits [38:30] |
| L2 (PMD) | 512 | 2 MiB (MegaPage) | VPN[1] = bits [29:21] |
| L3 (PTE) | 512 | 4 KiB | VPN[0] = bits [20:12] |

---

## 4. 调试策略

### 策略 1：升级 trap handler 的信息输出（最高优先级）

当前 `_early_trap` 只输出 `T` + scause 低 4 位。**这不足以诊断问题。**

**改造目标**：输出完整的 scause, sepc, stval（用 hex 编码）。

改造后的预期输出格式：
```
T[C]E[ADDR]V[ADDR]
```
其中：
- `T` = 标记"trap 发生"
- `C` = scause 完整 16 进制值（16 个 hex digit）
- `E` = sepc（16 个 hex digit）
- `V` = stval（16 个 hex digit）

**诊断能力对比**：

| Trap 类型 | scause | stval 含义 | 能告诉你的 |
|-----------|--------|-----------|-----------|
| Page Fault (Fetch) | 0x0c | 故障虚拟地址 | 页表翻译失败 |
| Page Fault (Load) | 0x0d | 故障虚拟地址 | load 到了未映射地址 |
| Page Fault (Store) | 0x0f | 故障虚拟地址 | store 到了未映射地址 |
| Illegal Instruction | 0x02 | 错误指令本身 | 编译问题 / CSR 权限 |
| Bus Error | ? | 物理地址 | EIC7700 投机执行问题 |

**实现参考**（RISC-V 汇编）：

需要一个 hex 输出的辅助宏：

```asm
// 输出 t0 的 16 个 hex digit 到串口
.macro PUTHEX reg
    li    t3, 60          // 从 bit 60 开始（每 4 bit 一个 hex digit）
1:
    srl   t4, \reg, t3
    andi  t4, t4, 0xf
    addi  t4, t4, '0'
    li    t5, '9'
    ble   t4, t5, 2f
    addi  t4, t4, 'a' - '0' - 10
2:
    mv    a0, t4
    li    a7, 0x01        // SBI putchar
    ecall
    addi  t3, t3, -4
    bgez  t3, 1b
.endm
```

然后在 trap handler 中：

```asm
_early_trap:
    // 检查 timer interrupt
    csrr   t0, scause
    li     t1, 0x8000000000000005
    beq    t0, t1, .Ltimer_reset

    // 输出 'T'
    SBI_PUTCHAR 'T'

    // 输出 scause (t0 中已有)
    PUTHEX t0

    // 输出 'E'
    SBI_PUTCHAR 'E'

    // 输出 sepc
    csrr t0, sepc
    PUTHEX t0

    // 输出 'V'
    SBI_PUTCHAR 'V'

    // 输出 stval
    csrr t0, stval
    PUTHEX t0

    // 等待 board reset
1:  j 1b
```

### 策略 2：缩小 QEMU-真机的语义差距

#### 2a. 用真实 Megrez DTB 在 QEMU 中测试

```bash
./porting/scripts/qemu_run_megrez_dtb.sh 45
```

这用 QEMU 的 `-dtb` 参数加载真实的 Megrez DTB，配合匹配的 CPU 特性。
可以提前暴露 DTB 解析、内存布局、timebase 频率等问题。

#### 2b. QEMU MMU trace（最强大的 QEMU 诊断工具）

```bash
qemu-system-riscv64 \
    -machine virt -cpu rv64,zba=true,zbb=true -m 4G \
    -nographic -bios default \
    -dtb porting/images/eic7700-milkv-megrez.dtb \
    -kernel target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
    -d mmu,cpu,int -D /tmp/qemu-mmu-trace.log
```

`-d mmu` 输出每次页表遍历的细节：哪个 VA 翻译失败、在哪一级失败、
PTW 访问了哪个物理地址。

#### 2c. 匹配 CPU 特性

QEMU 的 CPU 标志应尽量匹配 Megrez 的 SiFive P550：
```
rv64imafdch_zicsr_zifencei_zba_zbb_sscofpmf
```
目前 `OSDK.toml` 的 riscv scheme 用的是 `rv64,zba=true,zbb=true`，
缺少 `v`（向量扩展，这应该没问题）和 `h`（Hypervisor 扩展，OpenSBI 需要）。

### 策略 3：确认 OpenSBI PMP 保护

在 U-Boot 中：

```
=> sbi info
```

如果 OpenSBI 版本 < 1.8，PMP 可能没有保护 EIC7700 的危险区域。
这意味着页表再正确，CPU 投机执行也可能触发 bus error。

**如果 OpenSBI 版本不够新**：考虑升级 OpenSBI，或者至少意识到：
板子上的 bus error 可能不是你的页表问题，而是 OpenSBI 没有做好保护。

### 策略 4：构建「硬件探针」内核

在完整 Asterinas 启动之前，用一个极简内核逐步验证：

| 阶段 | 测试内容 | 关键观察 |
|------|---------|---------|
| 1 | U-Boot 加载确认 | `md 0x80200000` 对比 booti 文件 |
| 2 | SBI putchar 可用 | 连续输出字母，验证串口工作 |
| 3 | 构建页表但不开启 MMU | SBI 调用 dump 页表内容 |
| 4 | 开启 MMU + 跳转 | 当前 AEFGDHB 序列 |
| 5 | 开启 MMU 后读 satp | 验证页表未被破坏 |
| 6 | Trap 测试 | 故意触发 page fault，验证 handler 输出 |

每通过一阶段，问题一定出在下一阶段。每阶段只比上一阶段多做一件事。

### 策略 5：自动化真机测试循环

`serial_boot_asterinas.py` 已经可以：
- 自动等待 U-Boot 提示符
- 发送 booti 命令
- 捕获输出
- 检测 marker 序列

**可改进的地方**：
- 增加超时检测和自动判断
- 当检测到挂起时，自动提示"请物理 reset"
- 自动对比 marker 序列和目标序列
- 保存每次测试的完整日志用于对比

人工只需要：1) 按 reset 按钮；2) 运行脚本；3) 看结果。

---

## 5. QEMU vs 真机差异完整清单

| 差异项 | QEMU `virt` | Milk-V Megrez | 风险评估 |
|--------|-------------|---------------|---------|
| UART 基地址 | `0x10000000` (NS16550) | `0x50900000` (snps,dw-apb-uart) | 早期 boot 用 SBI putchar 不受影响；后续 UART 驱动需适配 |
| DTB 来源 | QEMU 自动生成 | `/boot` 分区中的真实 DTB | 内存布局、CPU 拓扑、保留内存完全不同 |
| 中断控制器 | QEMU virt PLIC | SiFive PLIC（可能有 ESWIN quirk） | 中断初始化可能失败 |
| MMU 行为 | QEMU 模拟（有 Sv48 bug） | 真实 SiFive P550 MMU | PTE A/D 位处理、TLB 行为可能不同 |
| SBI 实现 | OpenSBI 1.5.1 | 板载 OpenSBI（版本未知） | 版本 < 1.8 缺少 EIC7700 PMP 保护 |
| 启动入口 | OpenSBI → kernel | U-Boot `booti` → kernel | hartid 传递方式（已在代码中处理，从 a0 获取） |
| 投机执行 | QEMU 不模拟 | EIC7700 有已知 bus error 问题 | **可能是真机无声挂起的根因** |
| CPU 数量 | 可配置 | 4 × P550 + E21 | SMP 初始化差异 |
| 物理内存布局 | QEMU 连续的 | NPU 占用 6 GB 默认 | 可用内存量不同 |

---

## 6. 推荐的执行顺序

```
1. [ ] 升级 _early_trap — 输出完整 scause/sepc/stval
      文件: ostd/src/arch/riscv/boot/boot.S

2. [ ] 在 U-Boot 中确认 OpenSBI 版本
      命令: sbi info

3. [ ] QEMU 中用真实 Megrez DTB + MMU trace 测试
      脚本: qemu_run_megrez_dtb.sh + qemu -d mmu,cpu,int

4. [ ] 真机测试升级后的 trap handler
      部署 aster-nix.board.booti，收集输出

5. [ ] 如果（4）中 trap handler 被触发
      → 根据 scause/sepc/stval 值定位具体问题

6. [ ] 如果（4）中仍然完全无声
      → 可能是 EIC7700 投机执行 bus error
      → 检查 OpenSBI 版本，考虑升级或手动配置 PMP

7. [ ] 根据（5）或（6）的结果决定下一步
```

---

## 7. 参考资源

| 资源 | 链接 |
|------|------|
| OpenSBI EIC7700 补丁 (Bo Gan, v6) | https://patchwork.ozlabs.org/project/opensbi/cover/20251218104243.562667-1-ganboing@gmail.com/ |
| EIC7700 UART Boot 文档 | https://github.com/ganboing/EIC770x-Docs/blob/main/p550/bootchain/UART-Boot.md |
| EIC7700 SoC Technical Reference | https://github.com/eswincomputing/EIC7700X-SoC-Technical-Reference-Manual |
| OpenBSD RISC-V 移植报告 | https://github.com/MengshiLi/openbsd-riscv-notes |
| Xen RISC-V CSR dump 实现 | https://patchew.org/Xen/f6f7ec863e92ade433f23ae0061391d2ef731f41.1768579139.git.oleksii.kurochko@gmail.com/ |
| Tock OS RISC-V panic dump 改进 | https://github.com/tock/tock/pull/1575 |
| Milk-V Megrez 社区 | https://community.milkv.io/c/megrez/ |
| Asterinas RISC-V 支持 Issue | https://github.com/asterinas/asterinas/issues/1954 |
| Asterinas RISC-V refactor PR | https://github.com/asterinas/asterinas/pull/924 |
