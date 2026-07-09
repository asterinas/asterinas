#  Asterinas RISC-V 移植 — 阶段盘点与策略反思

> ## TL;DR

我们用了约 50 个 commit，把 Asterinas 的 RISC-V 启动从 "QEMU 上完全跑不通" 推到了 "QEMU 上稳定到达 `meta::init()` 内部的 frame metadata 映射阶段"。**真机尚未测试**（上次测的是旧代码，已不具代表性）。

关键教训：**不要用汇编做 Rust 该做的事**。我们曾把 boot.S 膨胀到 560 行（手工预建 80 个 4K 页表），最终缩减回 369 行（只留 1 个 L3），但仍在解决动态页表创建的 TLB 一致性问题。

<details>
<summary>当前 QEMU 输出（点击展开）</summary>

```
AEFGDJBWXV C R ... Td:e
```
Boot.S 全部通过 → Rust 入口 → DTB 解析 → meta 映射开始 → 某个 FRAME_METADATA VA 的 Load Page Fault。

</details>

---

## 1. 目标

把 [Asterinas](https://github.com/asterinas/asterinas) 移植到 **Milk-V Megrez** 开发板。

| 项目 | 信息 |
|------|------|
| SoC | ESWIN EIC7700X |
| CPU | SiFive P550 × 4（RV64GC + bitmanip + cache-management） |
| RAM | 16 GB LPDDR5 |
| 启动方式 | U-Boot → `booti` → S-mode kernel |
| SBI 实现 | OpenSBI（版本未知） |
| 调试接口 | FTDI USB-UART @ 115200 8N1 |
| 本地 QEMU | `qemu-system-riscv64` 8.2.2（`-machine virt`, `-m 8G`） |

---

## 2. 已完成的工程

### 2.1 启动链路

```
U-Boot booti 0x80200000 - 0xf0000000
  └─ _start (boot.S, 369 行)
       ├─ DTB 验证（0xf0000000 → a1 fallback）
       ├─ trap handler + safety timer
       ├─ Sv48 四级页表
       │    ├─ L4[0]:   512 GiB identity gigapages
       │    ├─ L4[256]: shared with L4[0] → linear mapping
       │    ├─ L4[448]: → boot_meta_l3pt (4 KiB)
       │    └─ L4[511]: → kernel 2 MiB leaves
       ├─ MMU 开启（satp ← Sv48 | root_ppn）
       ├─ VMA 切换（sp / gp / stvec / pc）
       └─ jr riscv_boot (Rust)
            ├─ Fdt::from_ptr → fill_early_info
            ├─ init_early_allocator (bump allocator)
            ├─ serial::init + logger::init
            ├─ cpu::init_on_bsp + FPU 启用
            ├─ meta::init → map_base_page loop ← 当前断点
            ├─ allocator::init
            ├─ kspace::init_kernel_page_table
            └─ … panic handler（理论上）
```

### 2.2 已解决的 14 个 bug

见 [`retrospective-analysis.md`](./retrospective-analysis.md) 完整列表。

### 2.3 诊断系统

- **boot.S marker**：每个里程碑一个 SBI_PUTCHAR 字符（`AEFGDJBWVXV`）。**无声故障时唯一的信息源。**
- **`_early_trap`**：非 timer 中断 → 输出 `T` + scause 低 nibble + `:` + stval VPN[3] nibble。
- **UART 兼容性**：早期用 SBI ecall（不依赖 UART 基地址），Rust 入口用 MMIO 写 `0x10000000`（QEMU 专有）。

### 2.4 构建与部署

```bash
# QEMU 构建
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme riscv --target-arch riscv64

# 真机构建
cargo osdk build --scheme milkv-megrez --target-arch riscv64
python3 porting/scripts/mkimage.py target/…/aster-nix-osdk-bin aster-nix.board.booti

# 部署
/mnt/c/Windows/System32/OpenSSH/scp.exe aster-nix.board.booti anjie@192.168.100.2:/tmp/
# 在板子上：sudo cp /tmp/aster-nix.board.booti /boot/
# U-Boot: ext4load … 0x80200000 /aster-nix.board.booti && booti 0x80200000 - 0xf0000000
```

---

## 3. 当前瓶颈：FRAME_METADATA 动态页表的 TLB 一致性

### 3.1 症状

`boot_pt::map_base_page()` 成功将大量 meta slot 页面映射到 `0xffffe000_00000000+` 区域，`sfence_vma_all()` 执行后，后续代码（`Segment::from_unused` → `get_slot` → `&*frame_to_meta_ptr`）读取已映射 VA 时触发 Load Page Fault。

### 3.2 根因分析

`map_base_page` 在 MMU 开启后，首次访问 FRAME_METADATA VA 时:

1. Walk L4[448] → present（boot.S 写入，MMU 开启前）✅
2. Walk L3[0] → absent → `alloc_child()` 分配新 L3 页 → write + `sfence.vma` → present ✅
3. Walk L2[0] → absent → `alloc_child()` 分配新 L2 页 → write + `sfence.vma` → **可能 stale**
4. Walk L1[0] → absent → `alloc_child()` 分配新 L1 页 → write + `sfence.vma`
5. Write leaf PTE → 完成

**问题在第 3 步**：`sfence.vma` 在 QEMU 8.2.2 上可能不能可靠地清除刚写入的非叶 PTE 的 TLB 缓存。后续对同一 L3 区域的访问（不同的 L2 index）会复用已 present 的 L3 条目，不会再触发 absent 路径和 sfence——但硬件 walker 的 TLB 可能仍然缓存了旧状态。

### 3.3 为什么这很难修

- **QEMU 8.2.2 的 TLB 模拟精度有限**。真机 SiFive P550 可能有不同行为。
- **sfence.vma 按 RISC-V 规范应该 works**，但 QEMU 的实现可能不符合规范。
- **x86 没有这个问题**：x86 的 `invlpg` + CR3 reload 强制刷新。RISC-V 的 `sfence.vma` 语义更弱。

### 3.4 可能的解决方案（按推荐程度排序）

| 方案 | 做法 | 优点 | 缺点 |
|------|------|------|------|
| **A: L3→L2 pre-link** | 在 boot.S 中只需多建 1 个 L2 表 + 1 个 L3[0]→L2 非叶 PTE（代码增加 ~15 行，无额外数据段） | 最小改动，从根上消除问题层级 | 仍是汇编解决问题 |
| **B: 升级 QEMU** | 测试 QEMU 9.x/10.x | 可能行为不同 | QEMU 9.x/10.x 已知有 Sv48 TLB walker 其他 bug |
| **C: 纯 Rust 修复** | map_base_page 中每写一个 leaf PTE 后 flush 整个 TLB | 不改 boot.S | 性能极差，可能仍然不够 |

---

## 4. 真机测试：状态与风险

### 4.1 当前状态

- **上次真机测试**：早期代码版本（Sv48?），输出 `AEFGD` 后无声。
- **当前代码未测试**。
- **已知真机差异**：

| 差异项 | QEMU `virt` | Milk-V Megrez |
|--------|-------------|---------------|
| UART 基地址 | `0x10000000` (NS16550) | `0x50900000` (dw-apb-uart) |
| DTB 内容 | QEMU 自动生成 | 真实板级 DTB |
| 中断控制器 | QEMU PLIC | SiFive PLIC |
| SBI | OpenSBI 1.3 (QEMU 内置) | 板载 OpenSBI（**版本未知**） |
| 内存布局 | 连续 8 GiB | 有 NPU 预留区（~6 GiB 可用） |
| 投机执行 | 不模拟 | EIC7700 P550 有已知 bus error quirk |

### 4.2 真机最可能的故障模式（概率排序）

1. **MMU 开启后 trap → `_early_trap` 输出 `T[x]:[y]`**（约 70%）— 可诊断
2. **EIC7700 投机执行 bus error**（约 20%）— 如果 OpenSBI < 1.8
3. **DTB 解析 panic**（约 10%）— `memory_regions` 或 `reserved-memory` 节点格式差异

---

## 5. 工程反思：什么做对了，什么做错了

### 5.1 做对的事

- **Marker 系统**：在完全无声的环境下是唯一的调试信息来源，简洁但信息密度高。
- **社区调研驱动**：OpenBSD/Xen/OpenSBI 的调研直接关联到具体问题。
- **写了文档**：`porting/` 下的文档体系 让知识可传递。
- **继承了上游设计**：`boot_pt::map_base_page` 的 lazy allocation 模式是对的，我们只需要让它 work。

### 5.2 做错的事

- **过早、过多依赖汇编**。FRAME_METADATA L1 表预建的 320 KiB `.data` 段是技术债的典型：短期解决了 Td，长期压住了真正的 TLB 一致性问题。
- **没有每次调试前确认 QEMU 版本**，导致一些修复尝试假设了错误的 QEMU 行为。
- **commit 历史缺乏纪律**：15 个 "final session summary"、10 个 Sv39↔Sv48 来回切换。
- **诊断 marker 没有用 feature gate 控制**，散落在各文件中。

---

## 6. 建议的下一步

### 现在就可以做（不需真机）

1. **加一个** `L3[0]→L2` 的 pre-link（方向 A，**~15 行汇编 + 1 个 4K 表**）— 这是把 TLB 一致性问题的范围从 "L3/L2/L1 三级" 缩小到 "仅 L2→L1"。**预计需要 30 分钟。**

2. 如果方案 A 不行，升级 QEMU 到 9.x/10.x 对比测试。

### 需要真机

3. **在 U-Boot 中运行 `sbi info`** — 确认 OpenSBI 版本和 PMP 保护状态。
4. **构建 booti 镜像 → scp 部署 → 收集输出**。如果 `_early_trap` 触发，scause/stval 会精确定位问题。
5. 如果完全无声 → 检查 OpenSBI PMP 配置。

### 长期

6. **整理 commit 历史** — interactive rebase 为 ~15 个语义清晰的 commit。
7. **向上游提交** — 参考上游已有的 RISC-V PR #924 和 Issue #1954。

---

<details>
<summary>相关资源（点击展开）</summary>

- [OpenBSD RISC-V Port Final Report](https://github.com/MengshiLi/openbsd-riscv-notes)
- [OpenSBI EIC7700 补丁 (Bo Gan, v6)](https://patchwork.ozlabs.org/project/opensbi/cover/20251218104243.562667-1-ganboing@gmail.com/)
- [Xen RISC-V trap CSR dump](https://patchew.org/Xen/f6f7ec863e92ade433f23ae0061391d2ef731f41.1768579139.git.oleksii.kurochko@gmail.com/)
- [上游 Asterinas RISC-V Issue #1954](https://github.com/asterinas/asterinas/issues/1954)
- 本仓库 `porting/debugging-strategy.md` — 完整调试策略
- 本仓库 `porting/retrospective-analysis.md` — 40-commit 详细分析
- 本仓库 `PLAN_Td_FIX.md` — Td page fault 根因分析

</details>
