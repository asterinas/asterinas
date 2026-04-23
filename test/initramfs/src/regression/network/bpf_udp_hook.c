// SPDX-License-Identifier: MPL-2.0

// Verify that BPF_PROG_LOAD + BPF_LINK_CREATE attach an eBPF program to the
// Asterinas UDP send netfilter hook, and that closing the link detaches it.
//
// The program loaded here is two eBPF instructions:
//   mov64 r0, 0
//   exit
// which returns 0 — mapped by the kernel to NF_DROP for the netfilter attach
// type. Once attached, UDP sends succeed at the syscall layer but are silently
// dropped before hitting the interface; recvfrom on the receiving socket must
// therefore block. After the link FD is closed the hook is removed and traffic
// flows again.

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>

#include "../common/test.h"

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

// `mov64 r0, 0; exit`.
static const uint8_t DROP_ALL_BYTECODE[] = {
	0xb7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
};

static int bpf(int cmd, void *attr, unsigned int size)
{
	return syscall(SYS_bpf, cmd, attr, size);
}

static int sender;
static int receiver;
static struct sockaddr_in receiver_addr;

FN_SETUP(sockets)
{
	sender = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
	receiver = CHECK(socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	memset(&receiver_addr, 0, sizeof(receiver_addr));
	receiver_addr.sin_family = AF_INET;
	receiver_addr.sin_port = htons(9753);
	CHECK(inet_aton("127.0.0.1", &receiver_addr.sin_addr));
	CHECK(bind(receiver, (struct sockaddr *)&receiver_addr,
		   sizeof(receiver_addr)));
}
END_SETUP()

static int send_msg(const char *msg)
{
	return sendto(sender, msg, strlen(msg), 0,
		      (struct sockaddr *)&receiver_addr, sizeof(receiver_addr));
}

// Wait up to `timeout_ms` for the receiver to become readable, then read.
// Returns the number of bytes read, or -1 with errno=EAGAIN on timeout.
static int recv_msg_timeout(char *buf, int len, int timeout_ms)
{
	struct pollfd pfd = { .fd = receiver, .events = POLLIN };
	int r = poll(&pfd, 1, timeout_ms);
	if (r == 0) {
		errno = EAGAIN;
		return -1;
	}
	if (r < 0) {
		return -1;
	}
	return recvfrom(receiver, buf, len, 0, NULL, NULL);
}

static int recv_msg(char *buf, int len)
{
	return recv_msg_timeout(buf, len, 500);
}

FN_TEST(bpf_udp_hook_drops_then_restores)
{
	char buf[64];

	// Baseline: no hook installed, traffic flows.
	TEST_RES(send_msg("pre"), _ret == 3);
	TEST_RES(recv_msg(buf, sizeof(buf)),
		 _ret == 3 && memcmp(buf, "pre", 3) == 0);

	// Load a drop-all eBPF program.
	struct bpf_prog_load_attr load_attr = {
		.prog_type = BPF_PROG_TYPE_NETFILTER,
		.insn_cnt = sizeof(DROP_ALL_BYTECODE) / 8,
		.insns = (uint64_t)(uintptr_t)DROP_ALL_BYTECODE,
	};
	int prog_fd = TEST_RES(
		bpf(BPF_PROG_LOAD, &load_attr, sizeof(load_attr)), _ret >= 0);
	SKIP_TEST_IF(prog_fd < 0);

	// Attach it to the UDP send hook.
	struct bpf_link_create_attr link_attr = {
		.prog_fd = prog_fd,
		.attach_type = BPF_ATTACH_TYPE_NETFILTER,
		.nf_hooknum = AST_HOOK_UDP_SEND,
	};
	int link_fd = TEST_RES(
		bpf(BPF_LINK_CREATE, &link_attr, sizeof(link_attr)), _ret >= 0);
	SKIP_TEST_IF(link_fd < 0);

	// Drain anything that slipped through before the hook was attached.
	while (recv_msg_timeout(buf, sizeof(buf), 0) > 0) {
	}

	// With the drop-all hook attached, sendto still succeeds but the
	// packet is swallowed; recvfrom must time out.
	TEST_RES(send_msg("blocked"), _ret == 7);
	TEST_ERRNO(recv_msg(buf, sizeof(buf)), EAGAIN);

	// Detach by closing the link FD.
	TEST_SUCC(close(link_fd));

	// Traffic flows again.
	TEST_RES(send_msg("post"), _ret == 4);
	TEST_RES(recv_msg(buf, sizeof(buf)),
		 _ret == 4 && memcmp(buf, "post", 4) == 0);

	TEST_SUCC(close(prog_fd));
}
END_TEST()

FN_TEST(bpf_rejects_unknown_prog_type)
{
	struct bpf_prog_load_attr load_attr = {
		.prog_type = 1, // socket filter — unsupported in phase 1.
		.insn_cnt = sizeof(DROP_ALL_BYTECODE) / 8,
		.insns = (uint64_t)(uintptr_t)DROP_ALL_BYTECODE,
	};
	TEST_ERRNO(bpf(BPF_PROG_LOAD, &load_attr, sizeof(load_attr)), EINVAL);
}
END_TEST()
