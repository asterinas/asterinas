.section .text.entry
.global _start
_start:
    li t3, 0x10000000
    li t4, 'A'
    li t5, '\n'
1:
    sb t4, 0(t3)
    sb t5, 0(t3)
    j 1b
