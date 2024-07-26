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

static int sk_unbound;
static int sk_bound;
static int sk_listen;
static int sk_connected;
static int sk_accepted;

#define UNNAMED_ADDR \
	((struct sockaddr_un){ .sun_family = AF_UNIX, .sun_path = "" })
#define UNNAMED_ADDRLEN PATH_OFFSET

#define BOUND_ADDR \
	((struct sockaddr_un){ .sun_family = AF_UNIX, .sun_path = "//tmp/B0" })
#define BOUND_ADDRLEN (PATH_OFFSET + 9)

#define LISTEN_ADDR \
	((struct sockaddr_un){ .sun_family = AF_UNIX, .sun_path = "/tmp//L0" })
#define LISTEN_ADDRLEN (PATH_OFFSET + 9)

FN_SETUP(unbound)
{
	sk_unbound = CHECK(socket(PF_UNIX, SOCK_STREAM, 0));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound = CHECK(socket(PF_UNIX, SOCK_STREAM, 0));

	CHECK(bind(sk_bound, (struct sockaddr *)&BOUND_ADDR, BOUND_ADDRLEN));
}
END_SETUP()

FN_SETUP(listen)
{
	sk_listen = CHECK(socket(PF_UNIX, SOCK_STREAM, 0));

	CHECK(bind(sk_listen, (struct sockaddr *)&LISTEN_ADDR, LISTEN_ADDRLEN));

	CHECK(listen(sk_listen, 1));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected = CHECK(socket(PF_UNIX, SOCK_STREAM, 0));

	CHECK(connect(sk_connected, (struct sockaddr *)&LISTEN_ADDR,
		      LISTEN_ADDRLEN));
}
END_SETUP()

FN_SETUP(accepted)
{
	sk_accepted = CHECK(accept(sk_listen, NULL, NULL));
}
END_SETUP()

FN_TEST(getsockname)
{
	struct sockaddr_un addr;
	socklen_t addrlen;

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_unbound, (struct sockaddr *)&addr, &addrlen),
		 addrlen == UNNAMED_ADDRLEN &&
			 memcmp(&addr, &UNNAMED_ADDR, UNNAMED_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_bound, (struct sockaddr *)&addr, &addrlen),
		 addrlen == BOUND_ADDRLEN &&
			 memcmp(&addr, &BOUND_ADDR, BOUND_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_listen, (struct sockaddr *)&addr, &addrlen),
		 addrlen == LISTEN_ADDRLEN &&
			 memcmp(&addr, &LISTEN_ADDR, LISTEN_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_connected, (struct sockaddr *)&addr, &addrlen),
		 addrlen == UNNAMED_ADDRLEN &&
			 memcmp(&addr, &UNNAMED_ADDR, UNNAMED_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_accepted, (struct sockaddr *)&addr, &addrlen),
		 addrlen == LISTEN_ADDRLEN &&
			 memcmp(&addr, &LISTEN_ADDR, LISTEN_ADDRLEN) == 0);
}
END_TEST()

FN_TEST(getpeername)
{
	struct sockaddr_un addr;
	socklen_t addrlen;

	addrlen = sizeof(addr);
	TEST_ERRNO(getpeername(sk_unbound, (struct sockaddr *)&addr, &addrlen),
		   ENOTCONN);

	addrlen = sizeof(addr);
	TEST_ERRNO(getpeername(sk_bound, (struct sockaddr *)&addr, &addrlen),
		   ENOTCONN);

	addrlen = sizeof(addr);
	TEST_ERRNO(getpeername(sk_listen, (struct sockaddr *)&addr, &addrlen),
		   ENOTCONN);

	addrlen = sizeof(addr);
	TEST_RES(getpeername(sk_connected, (struct sockaddr *)&addr, &addrlen),
		 addrlen == LISTEN_ADDRLEN &&
			 memcmp(&addr, &LISTEN_ADDR, LISTEN_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getpeername(sk_accepted, (struct sockaddr *)&addr, &addrlen),
		 addrlen == UNNAMED_ADDRLEN &&
			 memcmp(&addr, &UNNAMED_ADDR, UNNAMED_ADDRLEN) == 0);
}
END_TEST()

FN_TEST(connect)
{
	TEST_ERRNO(connect(sk_unbound, (struct sockaddr *)&BOUND_ADDR,
			   BOUND_ADDRLEN),
		   ECONNREFUSED);

	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&BOUND_ADDR,
			   BOUND_ADDRLEN),
		   ECONNREFUSED);

	TEST_ERRNO(connect(sk_listen, (struct sockaddr *)&LISTEN_ADDR,
			   LISTEN_ADDRLEN),
		   EINVAL);

	TEST_ERRNO(connect(sk_connected, (struct sockaddr *)&LISTEN_ADDR,
			   LISTEN_ADDRLEN),
		   EISCONN);

	TEST_ERRNO(connect(sk_connected, (struct sockaddr *)&LISTEN_ADDR,
			   LISTEN_ADDRLEN),
		   EISCONN);
}
END_TEST()
