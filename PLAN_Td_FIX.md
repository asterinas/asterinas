# QEMU RISC-V 移植调试完整计划

## 问题根因分析

Td 不是 QEMU 特有的 PMA 限制问题，而是**两个独立的页表缺陷**导致的。修复后 QEMU 应该可以完成完整 boot。

### Bug 1: FRAME_METADATA_BASE_VADDR 没有 L4 页表条目

```
FRAME_METADATA_BASE_VADDR = 0xFFFFE00000000000
Sv48 VPN 分解: VPN[3]=448, VPN[2]=0, VPN[1]=0, VPN[0]=0
```

boot.S 当前只设置了三个 L4 条目:
- L4[0]   → boot_idpt (identity mapping)
- L4[256] → boot_idpt (linear mapping, paddr_to_vaddr)
- L4[511] → boot_l3pt (kernel high-half)

L4[448] 是 ZERO。当 `boot_pt.map_base_page(FRAME_METADATA_BASE_VADDR, ...)` 被调用时，页表 walk 第一步就找不到 L4[448] → 触发 Page Fault (T)。

**修复**: 添加 `L4[448] → boot_meta_l3pt`（一个新的零初始化 L3 页表），让 `map_base_page` 可以通过 `alloc_child` 动态分配 L2/L1 表。

### Bug 2: LINEAR_MAPPING 只覆盖 PA 0..4GiB

boot_idpt 只有 4 个条目（通过 boot_l2_id_0..3 覆盖 PA 0..4GiB），用 2MiB megapages。

QEMU 配置了 8GB 内存，`alloc_meta_frames` 调用 `early_alloc(128MB)` 时可能返回 PA > 4GiB。之后 `paddr_to_vaddr(start_paddr)` 需要通过 LINEAR_MAPPING 访问该物理地址来初始化 meta slots，但 boot_idpt[4+] 是 ZERO → Page Fault。

**修复**: 将 boot_idpt 从 4×2MiB 子表改为 512×1GiB gigapages，覆盖 PA 0..512GiB。Sv48 强制要求支持 gigapages，QEMU 8.2.2 也支持。

### Td 故障发生时间线

```
meta::init()
  → alloc_meta_frames(tot_nr_frames)
      → early_alloc(128MB) 
          → 可能返回 PA > 4GiB (QEMU 8GB 内存)
      → paddr_to_vaddr(start_paddr) 通过 LINEAR_MAPPING 访问
          → boot_idpt[VPN[2]] = 0 (因为 PA > 4GiB)
              → PAGE FAULT → T (此处实际是 Td，d=13=Load Page Fault)
  → boot_pt.map_base_page(FRAME_METADATA_BASE_VADDR=0xFFFFE00000000000, ...)
      → L4[448] = 0 → PAGE FAULT
```

两个 bug 都会触发 Td，取决于执行顺序。修复后 Td 消失。

## 修改方案

### 1. boot.S 页表修改

#### 1a. boot_idpt: 1GiB gigapages 替代 2MiB megapages

删除 boot_l2_id_0..3（4 × 4KB，节省 16KB），直接用 .rept 在 boot_idpt 中生成 512 个 1GiB leaf entries：

```asm
.balign 4096
boot_idpt:
    .set i, 0
    .rept 512
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
```

删除运行时的 boot_idpt 条目填充代码（原先的 4 个 `sd t0, N(s2)` 指令块）。

#### 1b. 添加 L4[448] → boot_meta_l3pt

```asm
# L4[448] → boot_meta_l3pt (FRAME_METADATA region)
lla    t0, boot_meta_l3pt
slli   t0, t0, 32; srli   t0, t0, 32
srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
ori    t0, t0, PTE_V
li     t2, 448 * 8
add    t2, t1, t2
sd     t0, 0(t2)
```

#### 1c. 添加 boot_meta_l3pt 页表页

```asm
.balign 4096
boot_meta_l3pt:
    .zero 512 * PTE_SIZE
```

#### 1d. 删除不再需要的代码

- 四个 boot_l2_id_* 表（~100 行）
- boot_idpt 运行时填充代码
- L3[448-455] 清零代码（不再需要，meta 用独立 L4 条目）

### 2. QEMU 自测后的后续评估

QEMU 自测成功/失败后评估是否需要修改 meta.rs 的 QEMU workaround（目前使用 0x80200000..0x80201000 作为 meta segment placeholder）。

### 3. 不需要改的文件

- `PagingConsts`: 保持 NR_LEVELS=4, ADDRESS_WIDTH=48 (Sv48) ✓
- `PageTableEntry`: 实现正确，无需修改
- `boot_pt.rs`: map_base_page 逻辑正确
- `add_temp_linear_mapping`: riscv64 skip 保持，boot_idpt 已覆盖所有 PA
- `meta.rs`: QEMU workaround 可以在 QEMU 测试通过后清理

## 修改文件列表

只需修改 **一个文件**: `ostd/src/arch/riscv/boot/boot.S`

- 删除 boot_l2_id_0..3 四个表
- 删除运行时 boot_idpt 填充代码
- 删除 L3[448-455] 清零代码
- 重写 boot_idpt 为 512×1GiB leaves
- 添加 L4[448] → boot_meta_l3pt
- 添加 boot_meta_l3pt 页表页

## 预期结果

QEMU 8.2.2: AEFGDJBWXVCRabcdefgh → 完整 boot 不再 Td
QEMU 9.2.4: 同上
QEMU 10.0.2: 同上（无 PMA 限制问题）

真机 (SiFive P550): 原本就能工作，1GiB gigapages 在真实硬件上完全支持
