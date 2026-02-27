// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <elf.h>
#include <signal.h>
#include <sys/ptrace.h>
#include <sys/user.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

#define TARGET "/test/process/ptrace/debuggee"
#define FUNC_NAME "hello_world"

// Finds the runtime address of the symbol `name` in the target process,
// by parsing the non-PIE ELF file.
unsigned long find_symbol_runtime_addr(pid_t pid, const char *file,
				       const char *name)
{
	int fd = CHECK(open(file, O_RDONLY));

	// Read and validate the ELF header.
	Elf64_Ehdr eh = { 0 };
	// PIE is not supported for simplicity.
	CHECK_WITH(read(fd, &eh, sizeof(eh)),
		   _ret == sizeof(eh) && eh.e_type != ET_DYN);
	CHECK_WITH(memcmp(eh.e_ident, ELFMAG, SELFMAG),
		   _ret == 0 && eh.e_shentsize == sizeof(Elf64_Shdr));
	CHECK(lseek(fd, eh.e_shoff, SEEK_SET));

	// Find `.symtab`.
	Elf64_Shdr sh = { 0 }, symtab = { 0 }, strtab = { 0 };
	for (int i = 0; i < eh.e_shnum; i++) {
		CHECK_WITH(read(fd, &sh, sizeof(sh)), _ret == sizeof(sh));
		if (sh.sh_type == SHT_SYMTAB)
			symtab = sh;
	}
	CHECK_WITH(0, symtab.sh_offset != 0 && symtab.sh_link < eh.e_shnum);

	// Use `.symtab`'s `sh_link` to locate the associated string table.
	unsigned long long strtab_hdr_off =
		eh.e_shoff +
		(unsigned long long)symtab.sh_link * eh.e_shentsize;
	CHECK_WITH(0, strtab_hdr_off >= eh.e_shoff);
	CHECK(lseek(fd, strtab_hdr_off, SEEK_SET));
	CHECK_WITH(read(fd, &strtab, sizeof(strtab)), _ret == sizeof(strtab));
	CHECK_WITH(0, strtab.sh_type == SHT_STRTAB && strtab.sh_offset != 0);

	// Read all symbol name strings.
	CHECK(lseek(fd, strtab.sh_offset, SEEK_SET));
	char *strs = CHECK_WITH(malloc(strtab.sh_size), _ret != NULL);
	CHECK_WITH(read(fd, strs, strtab.sh_size), _ret == strtab.sh_size);

	// Iterate symbols and return the matched symbol value.
	CHECK(lseek(fd, symtab.sh_offset, SEEK_SET));
	Elf64_Sym sym;
	for (int i = 0; i < symtab.sh_size / sizeof(sym); i++) {
		CHECK_WITH(read(fd, &sym, sizeof(sym)), _ret == sizeof(sym));
		if (sym.st_name < strtab.sh_size &&
		    strcmp(strs + sym.st_name, name) == 0) {
			free(strs);
			CHECK(close(fd));
			return sym.st_value;
		}
	}

	free(strs);
	CHECK(close(fd));
	exit(1);
}

// Reads a byte at `addr` in the tracee process.
char read_tracee_byte(pid_t pid, unsigned long addr)
{
	char path[64], byte = 0;
	CHECK(snprintf(path, sizeof(path), "/proc/%d/mem", pid));

	int fd = CHECK(open(path, O_RDONLY));
	CHECK_WITH(pread(fd, &byte, 1, addr), _ret == 1);
	CHECK(close(fd));
	return byte;
}

// Writes a byte at `addr` in the tracee process.
void write_tracee_byte(pid_t pid, unsigned long addr, char byte)
{
	char path[64];
	CHECK(snprintf(path, sizeof(path), "/proc/%d/mem", pid));

	int fd = CHECK(open(path, O_RDWR));
	CHECK_WITH(pwrite(fd, &byte, 1, addr), _ret == 1);
	CHECK(close(fd));
}

FN_TEST(ptrace_debugger)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		char *const argv[] = { TARGET, NULL };
		char *const envp[] = { NULL };
		CHECK(execve(TARGET, argv, envp));
		exit(-1);
	}

	int status;
	siginfo_t si;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGTRAP);
	TEST_RES(ptrace(PTRACE_GETSIGINFO, pid, 0, &si),
		 _ret == 0 && si.si_signo == SIGTRAP && si.si_code == SI_USER &&
			 si.si_pid == pid && si.si_uid == getuid());

	unsigned long runtime_addr =
		find_symbol_runtime_addr(pid, TARGET, FUNC_NAME);
	char orig = read_tracee_byte(pid, runtime_addr);

	// Insert a breakpoint by writing `INT3` (0xCC).
	write_tracee_byte(pid, runtime_addr, 0xCC);
	TEST_RES(ptrace(PTRACE_CONT, pid, 0, 0), _ret == 0);

	for (int i = 0; i < 5; i++) {
		// The tracee should hit the breakpoint.
		TEST_RES(waitpid(pid, &status, 0),
			 _ret == pid && WIFSTOPPED(status) &&
				 WSTOPSIG(status) == SIGTRAP);
		TEST_RES(ptrace(PTRACE_GETSIGINFO, pid, 0, &si),
			 _ret == 0 && si.si_signo == SIGTRAP &&
				 si.si_code == SI_KERNEL);

		struct user_regs_struct regs;
		TEST_RES(ptrace(PTRACE_GETREGS, pid, 0, &regs),
			 regs.rip == runtime_addr + 1);

		// Restore the original instruction, and then single-step.
		regs.rip -= 1;
		TEST_SUCC(ptrace(PTRACE_SETREGS, pid, 0, &regs));
		write_tracee_byte(pid, runtime_addr, orig);
		TEST_SUCC(ptrace(PTRACE_SINGLESTEP, pid, 0, 0));

		// The tracee should be stopped after single-step. Re-insert the breakpoint.
		TEST_RES(waitpid(pid, &status, 0),
			 _ret == pid && WIFSTOPPED(status) &&
				 WSTOPSIG(status) == SIGTRAP);
		TEST_RES(ptrace(PTRACE_GETSIGINFO, pid, 0, &si),
			 _ret == 0 && si.si_signo == SIGTRAP &&
				 si.si_code == TRAP_TRACE);
		write_tracee_byte(pid, runtime_addr, 0xCC);
		TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	}

	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFEXITED(status) &&
						   WEXITSTATUS(status) == 233);
}
END_TEST()
