// SPDX-License-Identifier: MPL-2.0

#include <elf.h>
#include <stddef.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/fcntl.h>

#include "../../common/test.h"

struct custom_elf {
	Elf64_Ehdr ehdr;
	Elf64_Phdr phdr[3]; // See `push_interp` and `pop_interp` below.
	char buf[128];
};

static struct custom_elf elf;

#define BASE_ADDRESS 0x10000
#define PAGE_SIZE 0x1000

#define EXE_PATH "/tmp/my_exe"

#define UD2_INSTR \
	"\x0f\x0b" // "ud2" in x86-64. TODO: Support other architectures.

FN_SETUP(init_exec)
{
	elf.ehdr.e_ident[EI_MAG0] = ELFMAG0;
	elf.ehdr.e_ident[EI_MAG1] = ELFMAG1;
	elf.ehdr.e_ident[EI_MAG2] = ELFMAG2;
	elf.ehdr.e_ident[EI_MAG3] = ELFMAG3;
	elf.ehdr.e_ident[EI_CLASS] = ELFCLASS64;
	elf.ehdr.e_ident[EI_DATA] = ELFDATA2LSB;
	elf.ehdr.e_ident[EI_VERSION] = EV_CURRENT;

	elf.ehdr.e_type = ET_EXEC;
	elf.ehdr.e_machine = EM_X86_64;
	elf.ehdr.e_version = EV_CURRENT;
	elf.ehdr.e_entry = BASE_ADDRESS + offsetof(struct custom_elf, buf);

	elf.ehdr.e_phoff = offsetof(struct custom_elf, phdr);
	elf.ehdr.e_phentsize = sizeof(Elf64_Phdr);
	elf.ehdr.e_phnum = 1;

	elf.phdr[0].p_type = PT_LOAD;
	elf.phdr[0].p_flags = PF_R | PF_W | PF_X;
	elf.phdr[0].p_offset = 0;
	elf.phdr[0].p_vaddr = BASE_ADDRESS;
	elf.phdr[0].p_filesz = PAGE_SIZE;
	elf.phdr[0].p_memsz = PAGE_SIZE;
}
END_SETUP()

static void write_all(const char *filename, void *buf, size_t len)
{
	int fd;

	fd = CHECK(open(filename, O_WRONLY | O_CREAT | O_TRUNC, 0755));
	CHECK_WITH(write(fd, buf, len), _ret == len);
	CHECK(close(fd));
}

static int do_execve(void)
{
	write_all(EXE_PATH, &elf, sizeof(elf));

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wnonnull"
	return execve(EXE_PATH, NULL, NULL);
#pragma GCC diagnostic pop
}

static int do_execve_good(void)
{
	pid_t pid;
	int status;

	memcpy(elf.buf, UD2_INSTR, sizeof(UD2_INSTR));

	pid = CHECK(fork());
	if (pid == 0) {
		CHECK(do_execve());
		exit(EXIT_FAILURE);
	}

	CHECK_WITH(wait(&status), _ret == pid);
	if (!WIFSIGNALED(status) || WTERMSIG(status) != SIGILL)
		return -1;

	return 0;
}

FN_TEST(good_exec)
{
	// First of all, verify that `elf` is a good ELF.
	TEST_RES(do_execve_good(), _ret == 0);
}
END_TEST()

FN_TEST(bad_magic)
{
	int i;

	for (i = 0; i < SELFMAG; ++i) {
		++elf.ehdr.e_ident[i];
		TEST_ERRNO(do_execve(), ENOEXEC);
		--elf.ehdr.e_ident[i];
	}
}
END_TEST()

FN_TEST(bad_machine)
{
	++elf.ehdr.e_machine;
	TEST_ERRNO(do_execve(), ENOEXEC);
	--elf.ehdr.e_machine;
}
END_TEST()

FN_TEST(bad_type)
{
	long old;

	old = elf.ehdr.e_type;
	elf.ehdr.e_type = ET_CORE;
	TEST_ERRNO(do_execve(), ENOEXEC);
	elf.ehdr.e_type = old;
}
END_TEST()

FN_TEST(bad_phoff)
{
	long old;

	old = elf.ehdr.e_phoff;
	elf.ehdr.e_phoff = sizeof(elf);
	TEST_ERRNO(do_execve(), ENOEXEC);
	elf.ehdr.e_phoff = old;
}
END_TEST()

FN_TEST(bad_phentsize)
{
	++elf.ehdr.e_phentsize;
	TEST_ERRNO(do_execve(), ENOEXEC);
	--elf.ehdr.e_phentsize;

	--elf.ehdr.e_phentsize;
	TEST_ERRNO(do_execve(), ENOEXEC);
	++elf.ehdr.e_phentsize;
}
END_TEST()

FN_TEST(bad_phnum)
{
	long old;

	old = elf.ehdr.e_phnum;
	elf.ehdr.e_phnum = 0;
	TEST_ERRNO(do_execve(), ENOEXEC);
	elf.ehdr.e_phnum = old;
}
END_TEST()

static unsigned int push_interp(const char *interpreter_path)
{
	unsigned int i;

	i = CHECK_WITH(elf.ehdr.e_phnum++,
		       _ret < sizeof(elf.phdr) / sizeof(elf.phdr[0]));
	elf.phdr[i].p_type = PT_INTERP;
	elf.phdr[i].p_offset = offsetof(struct custom_elf, buf);
	elf.phdr[i].p_filesz = strlen(interpreter_path) + 1;

	strncpy(elf.buf, interpreter_path, sizeof(elf.buf) - 1);

	return i;
}

static void pop_interp(void)
{
	CHECK_WITH(--elf.ehdr.e_phnum, _ret >= 1);
}

FN_TEST(interp_too_long)
{
	unsigned int i;

	i = push_interp("/dev/zero");
	elf.phdr[i].p_filesz = 0x1000000;
	TEST_ERRNO(do_execve(), ENOEXEC);
	pop_interp();
}
END_TEST()

FN_TEST(interp_missing_nul)
{
	unsigned int i;

	i = push_interp("/dev/zero");
	--elf.phdr[i].p_filesz;
	TEST_ERRNO(do_execve(), ENOEXEC);
	pop_interp();
}
END_TEST()

FN_TEST(interp_trunc_eof)
{
	unsigned int i;

	i = push_interp("/dev/zero");
	elf.phdr[i].p_offset = sizeof(elf) - 1;
	TEST_ERRNO(do_execve(), EIO);
	pop_interp();
}
END_TEST()

FN_TEST(interp_overflow_end)
{
	unsigned int i;
	int j;

	i = push_interp("/dev/zero");
	elf.phdr[i].p_offset = ~(Elf64_Off)0 - 10;

	for (j = 9; j <= 12; ++j) {
		elf.phdr[i].p_filesz = j;
		TEST_ERRNO(do_execve(), EINVAL);
	}

	pop_interp();
}
END_TEST()

FN_TEST(interp_doesnt_exist)
{
	push_interp("/tmp/file_doesnt_exist");
	TEST_ERRNO(do_execve(), ENOENT);
	pop_interp();
}
END_TEST()

FN_TEST(interp_bad_perm)
{
	push_interp("/dev/zero");
	TEST_ERRNO(do_execve(), EACCES);
	pop_interp();
}
END_TEST()

FN_TEST(interp_bad_format)
{
	int fd;

	push_interp("/tmp/my_lib");

	fd = TEST_SUCC(open("/tmp/my_lib", O_WRONLY | O_CREAT | O_TRUNC, 0755));
	TEST_RES(write(fd, "#!", 2), _ret == 2);
	TEST_SUCC(close(fd));

	TEST_ERRNO(do_execve(), EIO);

	TEST_SUCC(truncate("/tmp/my_lib", PAGE_SIZE));
	TEST_ERRNO(do_execve(), ELIBBAD);

	push_interp("/tmp/my_lib");
	TEST_ERRNO(do_execve(), ELIBBAD);
	pop_interp();

	TEST_SUCC(unlink("/tmp/my_lib"));

	pop_interp();
}
END_TEST()

// FIXME: Linux drops the old MM before creating new mappings. Any failures
// during the creation of new mappings result in a fatal signal, causing the
// error code to be lost. Asterinas now attempts to return these error codes
// to the user space.
#ifdef __asterinas__

static int do_execve_fatal(void)
{
	return do_execve();
}

#else /* __asterinas__ */

#include <sys/ptrace.h>
#include <sys/user.h>
#include <sys/syscall.h>

static int do_execve_fatal(void)
{
	pid_t pid;
	int status;
	struct user_regs_struct regs;

	write_all(EXE_PATH, &elf, sizeof(elf));

	pid = CHECK(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME));
		CHECK(raise(SIGSTOP));
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wnonnull"
		CHECK(execve(EXE_PATH, NULL, NULL));
#pragma GCC diagnostic pop
		exit(EXIT_FAILURE);
	}

	CHECK_WITH(wait(&status), _ret == pid && WIFSTOPPED(status) &&
					  WSTOPSIG(status) == SIGSTOP);

	// Wait until `execve` starts.
	CHECK(ptrace(PTRACE_SYSCALL, pid, NULL, NULL));
	CHECK_WITH(wait(&status), _ret == pid && WIFSTOPPED(status) &&
					  WSTOPSIG(status) == SIGTRAP);

	// Wait until `execve` completes.
	CHECK(ptrace(PTRACE_SYSCALL, pid, NULL, NULL));
	CHECK_WITH(wait(&status), _ret == pid && WIFSTOPPED(status) &&
					  WSTOPSIG(status) == SIGTRAP);

	// Get `execve` results.
	CHECK_WITH(ptrace(PTRACE_GETREGS, pid, NULL, &regs),
		   _ret >= 0 && regs.orig_rax == __NR_execve);

	CHECK(ptrace(PTRACE_DETACH, pid, NULL, NULL));
	CHECK_WITH(wait(&status), _ret == pid && WIFSIGNALED(status) &&
					  WTERMSIG(status) == SIGSEGV);

	errno = -regs.rax;
	return errno == 0 ? 0 : -1;
}

#endif /* __asterinas__ */

FN_TEST(unaglined_offset)
{
	++elf.phdr[0].p_offset;
	TEST_ERRNO(do_execve_fatal(), EINVAL);
	--elf.phdr[0].p_offset;
}
END_TEST()

FN_TEST(unaligned_vaddr)
{
	++elf.phdr[0].p_vaddr;
	TEST_ERRNO(do_execve_fatal(), EINVAL);
	--elf.phdr[0].p_vaddr;
}
END_TEST()

FN_TEST(overflow_offset_plus_filesz)
{
	long old;

	old = elf.phdr[0].p_offset;

	elf.phdr[0].p_offset = (~(Elf64_Off)0 & ~(PAGE_SIZE - 1)) +
			       (elf.phdr[0].p_offset & (PAGE_SIZE - 1));
	TEST_ERRNO(do_execve_fatal(), EINVAL);

	elf.phdr[0].p_offset -= PAGE_SIZE;
	TEST_ERRNO(do_execve_fatal(), EOVERFLOW);

	elf.phdr[0].p_offset = old;
}
END_TEST()

FN_TEST(overflow_vaddr_plus_memsz)
{
	int i;
	long old;

	old = elf.phdr[0].p_memsz;

	elf.phdr[0].p_memsz = ~(Elf64_Xword)0 - elf.phdr[0].p_vaddr;
	for (i = 0; i < 3; ++i) {
		TEST_ERRNO(do_execve_fatal(), ENOMEM);
		++elf.phdr[0].p_memsz;
	}

	elf.phdr[0].p_memsz = old;
}
END_TEST()

FN_TEST(underflow_vaddr)
{
	long old;

	old = elf.phdr[0].p_vaddr;
	elf.phdr[0].p_vaddr = PAGE_SIZE;
	TEST_ERRNO(do_execve_fatal(), EPERM);
	elf.phdr[0].p_vaddr = old;
}
END_TEST()

FN_TEST(memsz_larger_than_filesz)
{
	// It's okay for `p_memsz` to be larger than `p_filesz`.
	// However, the trailing part must be zeroed out. This is
	// an example of when zeroing fails.
	elf.phdr[0].p_filesz += PAGE_SIZE - 1;
	elf.phdr[0].p_memsz += PAGE_SIZE;
	// FIXME: This fails in Linux. However, Asterinas inserts
	// zero pages at the end of private mappings, so it will
	// succeed. See
	// <https://github.com/asterinas/asterinas/blob/9c4f644bd9287da1815a13115fbdfa914d8426f0/kernel/src/vm/vmar/vm_mapping.rs#L443-L447>.
#ifndef __asterinas__
	TEST_ERRNO(do_execve_fatal(), EFAULT);
#endif
	elf.phdr[0].p_memsz -= PAGE_SIZE;
	elf.phdr[0].p_filesz -= PAGE_SIZE - 1;
}
END_TEST()

FN_TEST(filesz_larger_than_memsz)
{
	--elf.phdr[0].p_memsz;
	TEST_ERRNO(do_execve_fatal(), EINVAL);
	++elf.phdr[0].p_memsz;
}
END_TEST()

// ==========================
// Below are tests for ET_DYN
// ==========================

FN_SETUP(init_dyn)
{
	elf.ehdr.e_type = ET_DYN;
}
END_SETUP()

FN_TEST(good_dyn)
{
	// First of all, verify that `elf` is a good ELF.
	TEST_RES(do_execve_good(), _ret == 0);
}
END_TEST()

FN_TEST(bad_align)
{
	long old;

	old = elf.phdr[0].p_align;

	// 2048 is smaller than PAGE_SIZE.
	elf.phdr[0].p_align = 2048;
	TEST_RES(do_execve_good(), _ret == 0);

	// 2047 is not a power of two.
	elf.phdr[0].p_align = 2047;
	TEST_RES(do_execve_good(), _ret == 0);

	elf.phdr[0].p_align = old;
}
END_TEST()

FN_TEST(large_align)
{
	long old;

	old = elf.phdr[0].p_align;

	elf.phdr[0].p_align = 1ul << 21;
	TEST_RES(do_execve_good(), _ret == 0);

	elf.phdr[0].p_align = 1ul << 42;
	TEST_RES(do_execve_good(), _ret == 0);

	elf.phdr[0].p_align = 1ul << 63;
	// FIXME: p_align exceeds the size of the user address space.
	// What does this mean? As far as Linux is concerned, it tries
	// to map to a zero address due to [1], [2], and [3]. This
	// does not seem to make much sense.
	// [1]: https://elixir.bootlin.com/linux/v6.19-rc2/source/fs/binfmt_elf.c#L1172
	// [2]: https://elixir.bootlin.com/linux/v6.19-rc2/source/fs/binfmt_elf.c#L1185
	// [3]: https://elixir.bootlin.com/linux/v6.19-rc2/source/fs/binfmt_elf.c#L1188
#ifdef __asterinas__
	TEST_ERRNO(do_execve_fatal(), ENOMEM);
#else
	TEST_ERRNO(do_execve_fatal(), EPERM);
#endif

	elf.phdr[0].p_align = old;
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(EXE_PATH));
}
END_SETUP()
