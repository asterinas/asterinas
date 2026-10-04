# SPDX-License-Identifier: MPL-2.0

guest_hello: guest_hello.S
	as --32 $< -o guest_hello.o
	ld -m elf_i386 -Ttext=0 --oformat=binary guest_hello.o -o $@
