// SPDX-License-Identifier: MPL-2.0

#include <elf.h>
#include <stddef.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/fcntl.h>

#include "../test.h"

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

FN_SETUP(init)
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

FN_TEST(good)
{
	pid_t pid;
	int status;

	// First of all, verify that `elf` is a good ELF.

	memcpy(elf.buf, UD2_INSTR, sizeof(UD2_INSTR));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(do_execve());
		exit(EXIT_FAILURE);
	}

	TEST_RES(wait(&status), _ret == pid && WIFSIGNALED(status) &&
					WTERMSIG(status) == SIGILL);
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

FN_SETUP(cleanup)
{
	CHECK(unlink(EXE_PATH));
}
END_SETUP()
