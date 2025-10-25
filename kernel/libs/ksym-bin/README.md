# ksym-bin 说明

本目录提供从标准输入读取的 kallsyms 风格符号列表（例如 /proc/kallsyms）生成压缩二进制 blob 的工具，以及对应的零拷贝读取格式约定。

本文档描述：
- KallsymsBlob 元数据之间的关系
- 压缩与 token 选择规则
- 符号排序与查找
- 二进制文件布局与对齐（适配零拷贝读取）
- 限制与注意事项

## KallsymsBlob 元数据关系

KallsymsBlob 是构建期的数据容器，用于收集符号并压缩、序列化。关键字段：

- token_table: Vec<u8>
  - 所有 token 字符串的拼接字节。
- token_index: Vec<u32>
  - 每个 token 在 token_table 中的起始偏移（按字节），与 token 的 id 一一对应。
- token_map: HashMap<String, u16>
  - 构建期使用的字典（不会写入 blob），用于从 token 文本得到 token id（0..N-1）。
- kallsyms_names: Vec<u8>
  - 压缩后的所有符号名字节串，顺序拼接。
- kallsyms_offsets: Vec<u32>
  - 每个符号在 `kallsyms_names` 中的起始偏移（按字节），与地址序下的符号一一对应。
- kallsyms_seqs_of_names: Vec<u32>
  - “名字序 → 地址序”的索引映射。用于对名字做二分时，定位到地址序下的具体条目。
- kallsyms_addresses: Vec<u64>
  - 每个符号的虚拟地址，按升序排列（即“地址序”）。
- kallsyms_num_syms: usize
  - 符号数量（同时也会以 u64 形式写入 blob 头）。

关系示意：
- 地址序下的第 i 个符号：
  - 地址 = `kallsyms_addresses[i]`
  - 名称压缩片段起始 = `kallsyms_offsets[i]`，每条名称均以类型和前缀长度字段开头，长度为 `TY_LEN=1`、`LENGTH_BYTES=2`（小端，低字节在前），即PREFIX_LEN = TY_LEN + LENGTH_BYTES，
    实际片段范围 = `kallsyms_names[kallsyms_offsets[i] + PREFIX_LEN .. kallsyms_offsets[i] + PREFIX_LEN + entry_len]`，其中 `entry_len` 来自该条目前置的 2 字节小端长度。
- 名字序查找：在 `kallsyms_seqs_of_names` 上二分比较（通过展开名称），命中 `mid` 后得到地址序索引 `seq = kallsyms_seqs_of_names[mid]`。

## 压缩与 token 选择规则

- Token 候选采用“固定前缀启发式”：只统计每个符号名的前缀，长度集合由实现配置（当前默认包含若干离散长度，如 10、24、31、…、2000）。
  - 若符号长度短于集合中的最小长度（当前为 10），会把“整名”也计入候选（仅用于名字开头）。
  - 候选排序采用加权策略，按 频次×长度 由高到低选取，最多保留 512 个 token；并避免选择是已选 token 前缀的候选（减少冗余）。
- 符号压缩采用“单前缀 token + 原始剩余”：
  - 仅在名称开头尝试匹配一个 token（按候选长度从长到短）；
  - 命中后写入 token 编码，剩余部分以原始字节追加；若未命中，则整名以原始字节写入。
- 名称条目为“定长前缀 + 负载”的记录格式：
  - 每个名称条目1字节类型 + 2 字节长度（小端，低字节在前）作为前缀，表示“负载”的总字节数；
  - 若使用 token，负载以 `0xFF <id> 0xFF`（1 字节 id）或 `0xFF <id_hi> <id_lo> 0xFF`（2 字节 id）开头，随后跟随名称剩余原始字节；
  - 若未使用 token，负载即为整名的原始字节。
- Token 编码中使用的分隔符常量：`TOKEN_MARKER = 0xFF`。

备注：读取输入时仅保留文本符号类型 T/t，并进行 Rust demangle。编码阶段会把符号类型字符（T/t）作为前缀放在压缩名称的最前面。

## 符号排序与查找

- 存储顺序为“地址序”。构建过程中：
  - `kallsyms_addresses` 与 `kallsyms_offsets` 按地址升序排列；
  - 另外构建一个“名字序 → 地址序”的 `kallsyms_seqs_of_names` 用于名字二分。
- 查找：
  - 地址 → 符号：在 `kallsyms_addresses` 上二分得到 i；为处理“同址符号别名（alias）”，会向前回溯到该地址的第一个符号，然后向后搜索下一个“地址不同”的符号以确定大小；若找不到则以文本段结束地址作为上界。名称解码基于 `kallsyms_offsets[i]` 与记录内的长度前缀来截取片段再展开。
  - 名称 → 地址：在名字序空间二分，每步根据 `kallsyms_seqs_of_names[mid]` 取回地址序索引，再用长度前缀解码对应名称进行比较；命中后返回对应地址。 

## 二进制文件布局与对齐

所有数值均为小端序列化。为满足零拷贝读取时的对齐要求（u64 按 8 字节对齐，u32 按 4 字节对齐），在写入各段之前会插入适当的 padding。内核加载该 bin 时会将其映射到 4K 对齐的页起始地址；在此前提下，下面的对齐能保证 `from_raw_parts` 的对齐安全。

布局顺序（括号中为对齐要求）：

1) num_syms: u64
2) addresses[num_syms]: u64[]  (align 8)
3) offsets[num_syms]: u32[]    (align 4)
4) seqs[num_syms]: u32[]       (align 4)
5) names:                      (align 8)
   - names_len: u64
   - names_bytes[names_len]: u8[]
     - 重复的“名称条目记录”：`[type: u8] [len: u16(le)] [payload: u8[len]]`
6) token_table:                (align 8)
   - token_table_len: u64
   - token_table_bytes[token_table_len]: u8[]
7) token_index:                (align 8 for len, then align 4 for array)
   - token_index_len: u64
   - token_index[token_index_len]: u32[] (align 4)

说明：
- 对齐通过在写入每个段前对 `Vec<u8>` 进行 padding 实现。
- `kallsyms_offsets` 使用 u32 存储，要求 `kallsyms_names.len() < 4GiB`。

## 零拷贝读取（KallsymsMapped）

- 零拷贝视图通过 `from_blob(&blob, stext, etext)` 直接把上述各段解释为切片，同时记住文本段边界用于地址查找：
  - `&[u64]` 对应 addresses，
  - `&[u32]` 对应 offsets、seqs、token_index，
  - `&[u8]`  对应 names、token_table。
- 由于段起始已按 8/4 字节对齐，再加上整体 4K 页对齐映射，`from_raw_parts` 的对齐要求满足，避免拷贝。

## 限制与注意事项

- token 数量上限 512；token id 为 u16。
- 每个名称条目的压缩后长度字段为 u16（小端），单条记录最大 65535 字节。
- `kallsyms_offsets` 为 u32，限制 `kallsyms_names` 总长度 < 4GiB。
- 顶层整数（num_syms、addresses、offsets、seqs、names_len、token_table_len、token_index_len 和 token_index 内容）均以小端写入；名称条目长度同为小端。
- 压缩仅在名称开头使用一个 token，其余部分为原始字节；若需要更高压缩比，可扩展为混合策略（前缀+后缀等）。

## 生成与使用

- 生成：
  - 将 nm -n -C {ELF} 作为标准输入（仅保留 T/t），执行本工具二进制，例如：
    - `nm -n -C {ELF} | cargo run -p ksym-bin --bin gen_ksym --features demangle > kallsyms.bin`
- 读取（消费者侧）：
  - 使用 `ksym_bin::KallsymsMapped::from_blob(&blob, stext, etext)` 零拷贝解析；
  - 可调用 `lookup_address`、`lookup_name`、或 `dump_all_symbols()` 获取数据（dump 输出形如：`<addr_hex> <type_char> <name>`）。


## Tests
```
cargo test --bin gen_ksym --features="demangle"
```

## 示例

示例（简化三条符号）：
- 输入：
  - 0000000000001000 T _start
  - 0000000000001100 T do_fork
  - 0000000000001200 T cpu_startup_entry
- 生成的 blob 中：
  - addresses = [0x1000, 0x1100, 0x1200]
  - offsets 指向 `kallsyms_names` 中三段压缩名称的起始位置；
  - seqs 反映名字序到地址序的对应关系；
  - names/token_table/token_index 按上述布局与对齐写入。

以上即当前实现对应的元数据关系与二进制布局说明。