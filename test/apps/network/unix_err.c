// SPDX-License-Identifier: MPL-2.0

#include <sys/socket.h>
#include <sys/un.h>
#include <sys/poll.h>
#include <fcntl.h>
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
	TEST_SUCC(unlink("/tmp/R0"));

	MAKE_TEST("/tmp/R1", 8, 9, 8, 8, "/tmp/R1");
	TEST_SUCC(unlink("/tmp/R1"));

	MAKE_TEST("/tmp/R2", 6, 6, 8, 7, "/tmp/R");
	TEST_SUCC(unlink("/tmp/R"));

	MAKE_TEST("/tmp/R3", 7, 7, 8, 8, "/tmp/R3");
	TEST_SUCC(unlink("/tmp/R3"));

	MAKE_TEST("/tmp/R4", 7, 7, 7, 8, "/tmp/R4");
	TEST_SUCC(unlink("/tmp/R4"));

	MAKE_TEST("/tmp/R5", 7, 7, 6, 8, "/tmp/R");
	TEST_SUCC(unlink("/tmp/R5"));

	MAKE_TEST("/tmp/R6", 7, 7, 0, 8, "");
	TEST_SUCC(unlink("/tmp/R6"));

	MAKE_TEST(LONG_PATH, 107, 107, 108, 108, LONG_PATH);
	TEST_SUCC(unlink(LONG_PATH));

	MAKE_TEST(LONG_PATH "a", 108, 108, 108, 109, LONG_PATH "a");
	TEST_SUCC(unlink(LONG_PATH "a"));

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

#define UNIX_ADDR(path) \
	((struct sockaddr_un){ .sun_family = AF_UNIX, .sun_path = path })

#define UNNAMED_ADDR UNIX_ADDR("")
#define UNNAMED_ADDRLEN PATH_OFFSET

#define BOUND_ADDR UNIX_ADDR("//tmp/B0")
#define BOUND_ADDRLEN (PATH_OFFSET + 9)

#define LISTEN_ADDR UNIX_ADDR("/tmp//L0")
#define LISTEN_ADDRLEN (PATH_OFFSET + 9)

#define LISTEN_ADDR2 UNIX_ADDR("/tmp/L0")
#define LISTEN_ADDRLEN2 (PATH_OFFSET + 8)

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

	CHECK(connect(sk_connected, (struct sockaddr *)&LISTEN_ADDR2,
		      LISTEN_ADDRLEN2));
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

FN_TEST(listen)
{
	TEST_ERRNO(listen(sk_unbound, 10), EINVAL);

	TEST_SUCC(listen(sk_listen, 10));

	TEST_ERRNO(listen(sk_connected, 10), EINVAL);

	TEST_ERRNO(listen(sk_accepted, 10), EINVAL);
}
END_TEST()

FN_TEST(ns_path)
{
	int fd;

	fd = TEST_SUCC(creat("/tmp/.good", 0644));
	TEST_ERRNO(bind(sk_unbound, (struct sockaddr *)&UNIX_ADDR("/tmp/.good"),
			sizeof(struct sockaddr)),
		   EADDRINUSE);
	TEST_ERRNO(connect(sk_unbound,
			   (struct sockaddr *)&UNIX_ADDR("/tmp/.good"),
			   sizeof(struct sockaddr)),
		   ECONNREFUSED);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink("/tmp/.good"));

	fd = TEST_SUCC(creat("/tmp/.bad", 0000));
	TEST_ERRNO(bind(sk_unbound, (struct sockaddr *)&UNIX_ADDR("/tmp/.bad"),
			sizeof(struct sockaddr)),
		   EADDRINUSE);
	TEST_ERRNO(connect(sk_unbound,
			   (struct sockaddr *)&UNIX_ADDR("/tmp/.bad"),
			   sizeof(struct sockaddr)),
		   EACCES);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink("/tmp/.bad"));
}
END_TEST()

FN_TEST(ns_abs)
{
	int sk, sk2;
	struct sockaddr_un addr;
	socklen_t addrlen;

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_STREAM, 0));

	TEST_SUCC(bind(sk, (struct sockaddr *)&UNIX_ADDR(""), PATH_OFFSET));
	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk, (struct sockaddr *)&addr, &addrlen),
		 addrlen == PATH_OFFSET + 6 && addr.sun_path[0] == '\0');

	sk2 = TEST_SUCC(socket(PF_UNIX, SOCK_STREAM, 0));

	TEST_ERRNO(bind(sk2, (struct sockaddr *)&addr, addrlen), EADDRINUSE);
	TEST_ERRNO(connect(sk2, (struct sockaddr *)&addr, addrlen),
		   ECONNREFUSED);
	TEST_SUCC(listen(sk, 1));
	TEST_SUCC(connect(sk2, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(close(sk));
	TEST_SUCC(close(sk2));

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_STREAM, 0));
	TEST_ERRNO(connect(sk, (struct sockaddr *)&addr, addrlen),
		   ECONNREFUSED);
	TEST_SUCC(bind(sk, (struct sockaddr *)&addr, addrlen));
	TEST_SUCC(close(sk));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sk_unbound));

	CHECK(close(sk_bound));

	CHECK(close(sk_listen));

	CHECK(close(sk_connected));

	CHECK(close(sk_accepted));

	CHECK(unlink(BOUND_ADDR.sun_path));

	CHECK(unlink(LISTEN_ADDR.sun_path));
}
END_SETUP()
