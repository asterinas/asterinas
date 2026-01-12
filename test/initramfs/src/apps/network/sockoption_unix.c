// SPDX-License-Identifier: MPL-2.0

/*
 * UNIX stream socket-related socket options.
 */

#include <stdlib.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/un.h>
#include <stdbool.h>
#include <sys/wait.h>
#include "../test.h"

static int sk_unbound;
static int sk_listen;
static int sk_connected;
static int sk_accepted;
static int sk_tcp;
static struct sockaddr_un addr = { .sun_family = AF_UNIX,
				   .sun_path = "/tmp/sock_option_test" };

FN_SETUP(create_sockets)
{
	sk_unbound = CHECK(socket(PF_UNIX, SOCK_STREAM, 0));

	sk_listen = CHECK(socket(PF_UNIX, SOCK_STREAM, 0));
	CHECK(bind(sk_listen, (struct sockaddr *)&addr, sizeof(addr)));
	CHECK(listen(sk_listen, 3));

	sk_connected = CHECK(socket(PF_UNIX, SOCK_STREAM, 0));
	CHECK(connect(sk_connected, (struct sockaddr *)&addr, sizeof(addr)));

	sk_accepted = CHECK(accept(sk_listen, NULL, NULL));

	sk_tcp = CHECK(socket(AF_INET, SOCK_STREAM, 0));
}
END_SETUP()

FN_TEST(priority)
{
	int val = 0;
	socklen_t len = sizeof(val);

	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_PRIORITY, &val, &len),
		 val == 0);

	val = -1;
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_PRIORITY, &val, len));
	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_PRIORITY, &val, &len),
		 val == -1);

	val = 100;
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_PRIORITY, &val, len));
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_PRIORITY, &val, &len),
		 val == 100);
}
END_TEST()

FN_TEST(acceptconn)
{
	int val = 0;
	socklen_t len = sizeof(val);

	TEST_ERRNO(setsockopt(sk_unbound, SOL_SOCKET, SO_ACCEPTCONN, &val, len),
		   ENOPROTOOPT);

	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_ACCEPTCONN, &val, &len),
		 val == 0 && len == 4);
	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_ACCEPTCONN, &val, &len),
		 val == 1 && len == 4);
	TEST_RES(getsockopt(sk_accepted, SOL_SOCKET, SO_ACCEPTCONN, &val, &len),
		 val == 0 && len == 4);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_ACCEPTCONN, &val,
			    &len),
		 val == 0 && len == 4);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_ACCEPTCONN, &val, &len),
		 val == 0 && len == 4);
}
END_TEST()

FN_TEST(pass_cred)
{
	int val = 0;
	socklen_t len = sizeof(val);

	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 0);
	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 0);
	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 0);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 0);

	val = 100;
	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_PASSCRED, &val, len));
	TEST_SUCC(setsockopt(sk_unbound, SOL_SOCKET, SO_PASSCRED, &val, len));
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_PASSCRED, &val, len));
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_PASSCRED, &val, len));

	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 1);
	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 1);
	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 1);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_PASSCRED, &val, &len),
		 len == 4 && val == 1);
}
END_TEST()

FN_TEST(peer_cred)
{
	struct cred {
		pid_t pid;
		uid_t uid;
		gid_t gid;
	};

	struct cred ucred = {};
	socklen_t len = sizeof(ucred);

	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_PEERCRED, &ucred, len),
		   ENOPROTOOPT);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_PEERCRED, &ucred, &len),
		 ucred.pid == 0 && ucred.uid == -1 && ucred.gid == -1);

	TEST_ERRNO(setsockopt(sk_unbound, SOL_SOCKET, SO_PEERCRED, &ucred, len),
		   ENOPROTOOPT);
	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_PEERCRED, &ucred, &len),
		 ucred.pid == 0 && ucred.uid == -1 && ucred.gid == -1);

	pid_t pid = getpid();
	uid_t uid = geteuid();
	gid_t gid = getegid();

	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_PEERCRED, &ucred, &len),
		 ucred.pid == pid && ucred.uid == uid && ucred.gid == gid);
	TEST_RES(getsockopt(sk_accepted, SOL_SOCKET, SO_PEERCRED, &ucred, &len),
		 ucred.pid == pid && ucred.uid == uid && ucred.gid == gid);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_PEERCRED, &ucred,
			    &len),
		 ucred.pid == pid && ucred.uid == uid && ucred.gid == gid);

	// Test listen socket
	int child_pid = TEST_SUCC(fork());
	if (child_pid == 0) {
		// Child process
		int sk_connect_new = CHECK(socket(AF_UNIX, SOCK_STREAM, 0));
		CHECK(connect(sk_connect_new, (struct sockaddr *)&addr,
			      sizeof(addr)));
		CHECK_WITH(getsockopt(sk_connect_new, SOL_SOCKET, SO_PEERCRED,
				      &ucred, &len),
			   ucred.pid == pid && ucred.uid == uid &&
				   ucred.gid == gid);
		CHECK(close(sk_connect_new));
		exit(0);
	}

	int sk_accepted_new = TEST_SUCC(accept(sk_listen, NULL, NULL));
	TEST_RES(getsockopt(sk_accepted_new, SOL_SOCKET, SO_PEERCRED, &ucred,
			    &len),
		 ucred.pid == child_pid && ucred.uid == uid &&
			 ucred.gid == gid);
	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_PEERCRED, &ucred, &len),
		 ucred.pid == pid && ucred.uid == uid && ucred.gid == gid);

	int status = 0;
	TEST_RES(wait4(child_pid, &status, 0, NULL),
		 _ret == child_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
	TEST_SUCC(close(sk_accepted_new));
}
END_TEST()

bool is_groups_equal(gid_t *g1, gid_t *g2, int groups)
{
	for (int i = 0; i < groups; i++) {
		if (g1[i] != g2[i]) {
			return false;
		}
	}
	return true;
}

FN_TEST(peer_groups)
{
	int groups = TEST_SUCC(getgroups(0, NULL));
	gid_t *buffer = (gid_t *)malloc(groups * sizeof(gid_t));
	TEST_SUCC(getgroups(groups, buffer));

	TEST_ERRNO(setsockopt(sk_unbound, SOL_SOCKET, SO_PEERGROUPS, buffer,
			      groups),
		   EINVAL);

	gid_t small_buffer[1];
	socklen_t buffer_len = 1;

	TEST_ERRNO(getsockopt(sk_tcp, SOL_SOCKET, SO_PEERGROUPS, small_buffer,
			      &buffer_len),
		   ENODATA);
	TEST_ERRNO(getsockopt(sk_unbound, SOL_SOCKET, SO_PEERGROUPS,
			      small_buffer, &buffer_len),
		   ENODATA);
	buffer_len = 1;
	TEST_ERRNO(getsockopt(sk_listen, SOL_SOCKET, SO_PEERGROUPS,
			      small_buffer, &buffer_len),
		   ERANGE);
	buffer_len = 1;
	TEST_ERRNO(getsockopt(sk_accepted, SOL_SOCKET, SO_PEERGROUPS,
			      small_buffer, &buffer_len),
		   ERANGE);

	gid_t big_buffer[100];

	buffer_len = sizeof(big_buffer);
	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_PEERGROUPS, big_buffer,
			    &buffer_len),
		 buffer_len == groups * sizeof(gid_t) &&
			 is_groups_equal(big_buffer, buffer, groups));

	buffer_len = sizeof(big_buffer);
	TEST_ERRNO(getsockopt(sk_connected, SOL_SOCKET, SO_PEERGROUPS, NULL,
			      &buffer_len),
		   EFAULT);

	buffer_len = sizeof(big_buffer);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_PEERGROUPS, big_buffer,
			    &buffer_len),
		 buffer_len == groups * sizeof(gid_t) &&
			 is_groups_equal(big_buffer, buffer, groups));

	int fildes[2];

	buffer_len = sizeof(big_buffer);
	TEST_SUCC(socketpair(PF_UNIX, SOCK_STREAM, 0, fildes));
	TEST_RES(getsockopt(fildes[0], SOL_SOCKET, SO_PEERGROUPS, big_buffer,
			    &buffer_len),
		 buffer_len == groups * sizeof(gid_t) &&
			 is_groups_equal(big_buffer, buffer, groups));

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));
	free(buffer);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sk_unbound));
	CHECK(close(sk_listen));
	CHECK(close(sk_accepted));
	CHECK(close(sk_connected));
	CHECK(close(sk_tcp));
	CHECK(unlink(addr.sun_path));
}
END_SETUP()
