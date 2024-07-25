// SPDX-License-Identifier: MPL-2.0

#include <sys/socket.h>
#include <sys/un.h>
#include <sys/poll.h>
#include <unistd.h>
#include <stddef.h>

#include "test.h"

#define PATH_OFFSET offsetof(struct sockaddr_un, sun_path)

FN_TEST(socket_addresses)
{
	int sk;
	socklen_t addrlen;
	struct sockaddr_un addr;

#define MIN(a, b) ((a) < (b) ? (a) : (b))

#define MAKE_TEST(path, path_copy_len, path_len_to_kernel, path_buf_len,       \
		  path_len_from_kernel, path_from_kernel)                      \
	sk = TEST_SUCC(socket(PF_UNIX, SOCK_STREAM, 0));                       \
                                                                               \
	memset(&addr, 0, sizeof(addr));                                        \
	addr.sun_family = AF_UNIX;                                             \
	memcpy(addr.sun_path, path, path_copy_len);                            \
                                                                               \
	TEST_SUCC(bind(sk, (struct sockaddr *)&addr,                           \
		       PATH_OFFSET + path_len_to_kernel));                     \
                                                                               \
	memset(&addr, 0, sizeof(addr));                                        \
                                                                               \
	addrlen = path_buf_len + PATH_OFFSET;                                  \
	TEST_RES(                                                              \
		getsockname(sk, (struct sockaddr *)&addr, &addrlen),           \
		addrlen == PATH_OFFSET + path_len_from_kernel &&               \
			0 == memcmp(addr.sun_path, path_from_kernel,           \
				    MIN(path_buf_len, path_len_from_kernel))); \
                                                                               \
	TEST_SUCC(close(sk));

#define LONG_PATH \
	"/tmp/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
	_Static_assert(sizeof(LONG_PATH) == sizeof(addr.sun_path),
		       "LONG_PATH has a wrong length");

	MAKE_TEST("/tmp/R0", 8, 8, 8, 8, "/tmp/R0");

	MAKE_TEST("/tmp/R1", 8, 9, 8, 8, "/tmp/R1");

	MAKE_TEST("/tmp/R2", 6, 6, 8, 7, "/tmp/R");

	MAKE_TEST("/tmp/R3", 7, 7, 8, 8, "/tmp/R3");

	MAKE_TEST("/tmp/R4", 7, 7, 7, 8, "/tmp/R4");

	MAKE_TEST("/tmp/R5", 7, 7, 6, 8, "/tmp/R");

	MAKE_TEST("/tmp/R6", 7, 7, 0, 8, "");

	MAKE_TEST(LONG_PATH, 107, 107, 108, 108, LONG_PATH);

	MAKE_TEST(LONG_PATH "a", 108, 108, 108, 109, LONG_PATH "a");

#undef LONG_PATH
#undef MAKE_TEST

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_STREAM, 0));

	TEST_ERRNO(bind(sk, (struct sockaddr *)&addr, -1), EINVAL);
	TEST_ERRNO(bind(sk, (struct sockaddr *)&addr, PATH_OFFSET - 1), EINVAL);
	TEST_ERRNO(bind(sk, (struct sockaddr *)&addr, sizeof(addr) + 1),
		   EINVAL);

	TEST_SUCC(close(sk));
}
END_TEST()
