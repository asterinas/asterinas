/* SPDX-License-Identifier: MPL-2.0 */

#ifndef SECCOMP_FD_PASS_H
#define SECCOMP_FD_PASS_H

#include "seccomp_common.h"
#include <sys/socket.h>

static inline void send_fd(int socket, int fd)
{
	char byte = 0;
	struct iovec iov = { .iov_base = &byte, .iov_len = 1 };
	char control[CMSG_SPACE(sizeof(int))] = { 0 };
	struct msghdr msg = { .msg_iov = &iov,
			      .msg_iovlen = 1,
			      .msg_control = control,
			      .msg_controllen = sizeof(control) };
	struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
	cmsg->cmsg_level = SOL_SOCKET;
	cmsg->cmsg_type = SCM_RIGHTS;
	cmsg->cmsg_len = CMSG_LEN(sizeof(int));
	memcpy(CMSG_DATA(cmsg), &fd, sizeof(fd));
	CHECK_WITH(sendmsg(socket, &msg, 0), _ret == 1);
}

static inline int receive_fd(int socket)
{
	char byte;
	struct iovec iov = { .iov_base = &byte, .iov_len = 1 };
	char control[CMSG_SPACE(sizeof(int))] = { 0 };
	struct msghdr msg = { .msg_iov = &iov,
			      .msg_iovlen = 1,
			      .msg_control = control,
			      .msg_controllen = sizeof(control) };
	CHECK_WITH(recvmsg(socket, &msg, 0),
		   _ret == 1 && !(msg.msg_flags & MSG_CTRUNC));
	struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
	CHECK_WITH(cmsg != NULL && cmsg->cmsg_level == SOL_SOCKET &&
			   cmsg->cmsg_type == SCM_RIGHTS &&
			   cmsg->cmsg_len == CMSG_LEN(sizeof(int)),
		   _ret);
	int fd;
	memcpy(&fd, CMSG_DATA(cmsg), sizeof(fd));
	return fd;
}

#endif
