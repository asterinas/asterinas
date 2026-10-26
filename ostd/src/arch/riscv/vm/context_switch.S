/* SPDX-License-Identifier: MPL-2.0 */

.text

.option push
.option norelax

# sd t0, offset(sp): 把 t0 的 8 字节写到地址 sp + offset。
# ld t0, offset(sp): 从地址 sp + offset 读取 8 字节到 t0。

.global __vcpu_run
.balign 4
.type __vcpu_run, @function
__vcpu_run:
    # a0 = regs_ptr, a1 = csrs_ptr
    addi sp, sp, -{host_frame_size}

    # 保存返回地址、宿主专用寄存器和 ABI 要求保留的寄存器
    sd ra,  {host_ra}(sp)
    sd gp,  {host_gp}(sp)
    sd tp,  {host_tp}(sp)
    sd s0,  {host_s0}(sp)
    sd s1,  {host_s1}(sp)
    sd s2,  {host_s2}(sp)
    sd s3,  {host_s3}(sp)
    sd s4,  {host_s4}(sp)
    sd s5,  {host_s5}(sp)
    sd s6,  {host_s6}(sp)
    sd s7,  {host_s7}(sp)
    sd s8,  {host_s8}(sp)
    sd s9,  {host_s9}(sp)
    sd s10, {host_s10}(sp)
    sd s11, {host_s11}(sp)
    sd a0,  {host_regs}(sp)
    sd a1,  {host_csrs}(sp)

    csrr t0, sstatus
    sd t0, {host_sstatus}(sp)
    csrr t0, sepc
    sd t0, {host_sepc}(sp)
    csrr t0, stvec
    sd t0, {host_stvec}(sp)
    csrr t0, sscratch
    sd t0, {host_sscratch}(sp)
    csrr t0, 0x600                       # hstatus
    sd t0, {host_hstatus}(sp)

    # 配置 HS 执行 sret 所需的 guest 恢复状态
    ld t0, {csrs_sepc}(a1)
    csrw sepc, t0
    ld t0, {csrs_hstatus}(a1)            # assert SPV = 1
    csrw 0x600, t0
    ld t0, {csrs_sstatus}(a1)            # assert SIE = 0, SPP = 1
    csrw sstatus, t0

    # lla 取得标签地址；四字节对齐使 stvec 使用 Direct 模式。
    lla t0, __vm_exit_handler
    csrw stvec, t0
    csrw sscratch, sp                    # 保存宿主栈帧地址。

    # t0 暂时充当 regs_ptr，必须最后才装入 guest 的 t0。
    mv t0, a0
    ld ra,  {guest_ra}(t0)
    ld gp,  {guest_gp}(t0)
    ld tp,  {guest_tp}(t0)
    ld t1,  {guest_t1}(t0)
    ld t2,  {guest_t2}(t0)
    ld s0,  {guest_s0}(t0)
    ld s1,  {guest_s1}(t0)
    ld a0,  {guest_a0}(t0)
    ld a1,  {guest_a1}(t0)
    ld a2,  {guest_a2}(t0)
    ld a3,  {guest_a3}(t0)
    ld a4,  {guest_a4}(t0)
    ld a5,  {guest_a5}(t0)
    ld a6,  {guest_a6}(t0)
    ld a7,  {guest_a7}(t0)
    ld s2,  {guest_s2}(t0)
    ld s3,  {guest_s3}(t0)
    ld s4,  {guest_s4}(t0)
    ld s5,  {guest_s5}(t0)
    ld s6,  {guest_s6}(t0)
    ld s7,  {guest_s7}(t0)
    ld s8,  {guest_s8}(t0)
    ld s9,  {guest_s9}(t0)
    ld s10, {guest_s10}(t0)
    ld s11, {guest_s11}(t0)
    ld t3,  {guest_t3}(t0)
    ld t4,  {guest_t4}(t0)
    ld t5,  {guest_t5}(t0)
    ld t6,  {guest_t6}(t0)
    ld sp,  {guest_sp}(t0)
    ld t0,  {guest_t0}(t0)
    sret

.global __vm_exit_handler
.balign 4
__vm_exit_handler:
    # 陷入 HS 不会自动切换 sp。交换后：
    # sp = 宿主栈帧地址，sscratch = guest 原来的 sp。
    csrrw sp, sscratch, sp

    # 先保护 guest t0，再借用 t0 取得 regs_ptr。
    sd t0, {host_guest_t0}(sp)
    ld t0, {host_regs}(sp)

    # guest 的所有可写 GPR 都要保存，包括 caller-saved 寄存器。
    sd zero, {guest_zero}(t0)
    sd ra,  {guest_ra}(t0)
    sd gp,  {guest_gp}(t0)
    sd tp,  {guest_tp}(t0)
    sd t1,  {guest_t1}(t0)
    sd t2,  {guest_t2}(t0)
    sd s0,  {guest_s0}(t0)
    sd s1,  {guest_s1}(t0)
    sd a0,  {guest_a0}(t0)
    sd a1,  {guest_a1}(t0)
    sd a2,  {guest_a2}(t0)
    sd a3,  {guest_a3}(t0)
    sd a4,  {guest_a4}(t0)
    sd a5,  {guest_a5}(t0)
    sd a6,  {guest_a6}(t0)
    sd a7,  {guest_a7}(t0)
    sd s2,  {guest_s2}(t0)
    sd s3,  {guest_s3}(t0)
    sd s4,  {guest_s4}(t0)
    sd s5,  {guest_s5}(t0)
    sd s6,  {guest_s6}(t0)
    sd s7,  {guest_s7}(t0)
    sd s8,  {guest_s8}(t0)
    sd s9,  {guest_s9}(t0)
    sd s10, {guest_s10}(t0)
    sd s11, {guest_s11}(t0)
    sd t3,  {guest_t3}(t0)
    sd t4,  {guest_t4}(t0)
    sd t5,  {guest_t5}(t0)
    sd t6,  {guest_t6}(t0)

    # t1 的 guest 值已保存，现在可以用它搬运暂存的 t0 和 sp。
    ld t1, {host_guest_t0}(sp)
    sd t1, {guest_t0}(t0)
    csrr t1, sscratch
    sd t1, {guest_sp}(t0)

    # 保存下一次进入 guest 要用的状态；这不是 vsstatus/vsepc。
    ld t0, {host_csrs}(sp)
    csrr t1, sepc
    sd t1, {csrs_sepc}(t0)
    csrr t1, sstatus
    sd t1, {csrs_sstatus}(t0)
    csrr t1, 0x600                       # hstatus
    sd t1, {csrs_hstatus}(t0)

    # 恢复宿主的运行环境。这里仍不能调用 Rust 或开启 IRQ。
    ld gp, {host_gp}(sp)
    ld tp, {host_tp}(sp)
    ld t0, {host_sscratch}(sp)
    csrw sscratch, t0
    ld t0, {host_stvec}(sp)
    csrw stvec, t0
    ld t0, {host_sepc}(sp)
    csrw sepc, t0
    ld t0, {host_hstatus}(sp)
    csrw 0x600, t0
    ld t0, {host_sstatus}(sp)
    csrw sstatus, t0                     # 保存时 SIE = 0。

    ld s0,  {host_s0}(sp)
    ld s1,  {host_s1}(sp)
    ld s2,  {host_s2}(sp)
    ld s3,  {host_s3}(sp)
    ld s4,  {host_s4}(sp)
    ld s5,  {host_s5}(sp)
    ld s6,  {host_s6}(sp)
    ld s7,  {host_s7}(sp)
    ld s8,  {host_s8}(sp)
    ld s9,  {host_s9}(sp)
    ld s10, {host_s10}(sp)
    ld s11, {host_s11}(sp)
    ld ra,  {host_ra}(sp)
    addi sp, sp, {host_frame_size}
    ret                                 # 跳转到 ra，返回 Rust 调用者。

.size __vcpu_run, .-__vcpu_run
.option pop
