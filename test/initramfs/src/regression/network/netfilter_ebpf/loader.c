// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

#include <netinet/in.h>

#ifndef SYS_bpf
#define SYS_bpf 321
#endif

#define BPF_PROG_LOAD 5
#define BPF_LINK_CREATE 28

#define BPF_PROG_TYPE_NETFILTER 45
#define BPF_ATTACH_TYPE_NETFILTER 45

// Asterinas-private hook number for the UDP send hook.
#define AST_HOOK_UDP_SEND 0x1000

struct bpf_prog_load_attr {
	uint32_t prog_type;
	uint32_t insn_cnt;
	uint64_t insns;
	uint64_t license;
	uint32_t log_level;
	uint32_t log_size;
	uint64_t log_buf;
	uint32_t kern_version;
	uint32_t prog_flags;
	char prog_name[16];
	uint32_t prog_ifindex;
	uint32_t expected_attach_type;
};

struct bpf_link_create_attr {
	uint32_t prog_fd;
	uint32_t target_fd_or_ifindex;
	uint32_t attach_type;
	uint32_t flags;
	uint32_t nf_pf;
	uint32_t nf_hooknum;
	int32_t nf_priority;
	uint32_t nf_flags;
};

static int bpf(int cmd, void *attr, unsigned int size)
{
	return syscall(SYS_bpf, cmd, attr, size);
}

static unsigned char *read_file(const char *path, size_t *len)
{
	int fd = open(path, O_RDONLY);
	if (fd < 0) {
		perror(path);
		return NULL;
	}

	off_t size = lseek(fd, 0, SEEK_END);
	if (size < 0) {
		perror("lseek");
		close(fd);
		return NULL;
	}
	if (lseek(fd, 0, SEEK_SET) < 0) {
		perror("lseek");
		close(fd);
		return NULL;
	}

	unsigned char *buffer = malloc((size_t)size);
	if (!buffer) {
		perror("malloc");
		close(fd);
		return NULL;
	}

	size_t offset = 0;
	while (offset < (size_t)size) {
		ssize_t chunk = read(fd, buffer + offset, (size_t)size - offset);
		if (chunk < 0) {
			if (errno == EINTR) {
				continue;
			}
			perror("read");
			free(buffer);
			close(fd);
			return NULL;
		}
		if (chunk == 0) {
			break;
		}
		offset += (size_t)chunk;
	}

	close(fd);
	*len = offset;
	return buffer;
}

static void usage(const char *prog)
{
	fprintf(stderr, "usage: %s --prog FILE [--hook N]\n", prog);
}

int main(int argc, char **argv)
{
	const char *prog_path = NULL;
	uint32_t hook_num = AST_HOOK_UDP_SEND;

	for (int i = 1; i < argc; ++i) {
		if (strcmp(argv[i], "--prog") == 0 && i + 1 < argc) {
			prog_path = argv[++i];
			continue;
		}
		if (strcmp(argv[i], "--hook") == 0 && i + 1 < argc) {
			hook_num = (uint32_t)strtoul(argv[++i], NULL, 0);
			continue;
		}
		usage(argv[0]);
		return 2;
	}

	if (!prog_path) {
		usage(argv[0]);
		return 2;
	}

	size_t prog_size = 0;
	unsigned char *bytecode = read_file(prog_path, &prog_size);
	if (!bytecode) {
		return 1;
	}
	if ((prog_size % 8) != 0) {
		fprintf(stderr, "%s: size must be a multiple of 8 bytes\n", prog_path);
		free(bytecode);
		return 1;
	}

	struct bpf_prog_load_attr load_attr = {
		.prog_type = BPF_PROG_TYPE_NETFILTER,
		.insn_cnt = (uint32_t)(prog_size / 8),
		.insns = (uint64_t)(uintptr_t)bytecode,
	};
	int prog_fd = bpf(BPF_PROG_LOAD, &load_attr, sizeof(load_attr));
	if (prog_fd < 0) {
		perror("BPF_PROG_LOAD");
		free(bytecode);
		return 1;
	}

	struct bpf_link_create_attr link_attr = {
		.prog_fd = (uint32_t)prog_fd,
		.attach_type = BPF_ATTACH_TYPE_NETFILTER,
		.nf_hooknum = hook_num,
	};
	int link_fd = bpf(BPF_LINK_CREATE, &link_attr, sizeof(link_attr));
	if (link_fd < 0) {
		perror("BPF_LINK_CREATE");
		close(prog_fd);
		free(bytecode);
		return 1;
	}

	printf("loaded %s: pid=%d prog_fd=%d link_fd=%d hook=%u\n", prog_path,
	       (int)getpid(), prog_fd, link_fd, hook_num);
	fflush(stdout);

	free(bytecode);

	for (;;) {
		pause();
	}
}
