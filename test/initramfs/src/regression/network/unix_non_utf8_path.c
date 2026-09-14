/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE

#include <sys/socket.h>
#include <sys/un.h>
#include <stddef.h>
#include <unistd.h>

#include "../common/test.h"

#define PATH_OFFSET offsetof(struct sockaddr_un, sun_path)

FN_TEST(non_utf8_nul_terminated_path)
{
	int sk;
	socklen_t addrlen;
	struct sockaddr_un addr;
	char path[sizeof(addr.sun_path) + 1];

	// 107 bytes of content + NUL: "/tmp/" + 'a' * 100 + 0xff + 0xff.
	memset(path, 'a', sizeof(addr.sun_path));
	memcpy(path, "/tmp/", 5);
	path[sizeof(addr.sun_path) - 3] = (char)0xff;
	path[sizeof(addr.sun_path) - 2] = (char)0xff;
	path[sizeof(addr.sun_path) - 1] = '\0';

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_STREAM, 0));

	memset(&addr, 0, sizeof(addr));
	addr.sun_family = AF_UNIX;
	memcpy(addr.sun_path, path, sizeof(addr.sun_path));

	TEST_SUCC(bind(sk, (struct sockaddr *)&addr,
		       PATH_OFFSET + sizeof(addr.sun_path)));

	memset(&addr, 0, sizeof(addr));
	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk, (struct sockaddr *)&addr, &addrlen),
		 addrlen == PATH_OFFSET + sizeof(addr.sun_path) &&
			 0 == memcmp(addr.sun_path, path,
				     sizeof(addr.sun_path)));

	TEST_SUCC(close(sk));
	TEST_SUCC(unlink(path));
}
END_TEST()
