path="/home/anjie/asterinas/ostd/src/arch/riscv/boot/bsp_boot.S"
with open(path) as f:
    s=f.read()

old=""".Ldtb_ok:
    # Set up the Sv48 page table."""
new=""".Ldtb_ok:
    # Early debug: send a single character to the dw-apb-uart at 0x50900000.
    # U-Boot has already initialized the UART, so we can just write the THR.
    li     t3, 0x50900000
    li     t4, 0x41          # 'A'
    sb     t4, 0(t3)

    # Set up the Sv48 page table."""
if old not in s:
    print("old block 1 not found")
    exit(1)
s=s.replace(old,new)

old2="""bsp_flush_tlb:
    sfence.vma"""
new2="""bsp_flush_tlb:
    li     t3, 0x50900000
    li     t4, 0x42          # 'B'
    sb     t4, 0(t3)
    sfence.vma"""
if old2 not in s:
    print("old block 2 not found")
    exit(1)
s=s.replace(old2,new2)

old3="""bsp_boot_virt:
    # Initialize GP to the CPU-local storage's base address."""
new3="""bsp_boot_virt:
    li     t3, 0x50900000
    li     t4, 0x43          # 'C'
    sb     t4, 0(t3)
    # Initialize GP to the CPU-local storage's base address."""
if old3 not in s:
    print("old block 3 not found")
    exit(1)
s=s.replace(old3,new3)

with open(path,"w") as f:
    f.write(s)
print("patched")
