#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

static int sk_unbound;
static int sk_bound;
static int sk_connected;

static struct sockaddr_in sk_addr;

#define C_PORT htons(0x1234)

static int setup_unbound(void)
{
	sk_unbound = socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
	if (sk_unbound < 0) {
		perror("socket");
		return -1;
	}

	return 0;
}

static int setup_bound(void)
{
	int err;

	sk_bound = socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
	if (sk_bound < 0) {
		perror("socket");
		return -1;
	}

	sk_addr.sin_port = C_PORT;
	err = bind(sk_bound, (struct sockaddr *)&sk_addr, sizeof(sk_addr));
	if (err < 0) {
		perror("bind");
		goto err;
	}

	return 0;
err:
	close(sk_unbound);
	return -1;
}

static int setup_connected(void)
{
	int err;

	sk_connected = socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
	if (sk_connected < 0) {
		perror("socket");
		return -1;
	}

	sk_addr.sin_port = C_PORT;
	err = connect(sk_connected, (struct sockaddr *)&sk_addr,
		      sizeof(sk_addr));
	if (err < 0 && errno != EINPROGRESS) {
		perror("bind");
		goto err;
	}

	return 0;
err:
	close(sk_connected);
	return -1;
}

static void do_setup(void)
{
	int err;

	sk_addr.sin_family = AF_INET;
	sk_addr.sin_port = htons(8080);
	if (inet_aton("127.0.0.1", &sk_addr.sin_addr) < 0) {
		fprintf(stderr, "inet_aton cannot parse 127.0.0.1\n");
		exit(EXIT_FAILURE);
	}

#define CHECK(func)                                   \
	err = func();                                 \
	if (err < 0) {                                \
		fprintf(stderr, #func "() failed\n"); \
		exit(EXIT_FAILURE);                   \
	}

	CHECK(setup_unbound);
	CHECK(setup_bound);
	CHECK(setup_connected);

#undef CHECK
}

#define TEST_AND(err, cond, func, ...)                                       \
	errno = 0;                                                           \
	(void)func(sk, __VA_ARGS__);                                         \
	if (errno != (err)) {                                                \
		tests_failed++;                                              \
		fprintf(stderr,                                              \
			"%s: " #func " failed [got %s, but expected %s]\n",  \
			__func__, strerror(errno), strerror(err));           \
	} else if (!(cond)) {                                                \
		tests_failed++;                                              \
		fprintf(stderr,                                              \
			"%s: " #func " failed [got %s, but " #cond           \
			" is false]\n",                                      \
			__func__, strerror(errno));                          \
	} else {                                                             \
		tests_passed++;                                              \
		fprintf(stderr, "%s: " #func " passed [got %s]\n", __func__, \
			strerror(errno));                                    \
	}

#define TEST(err, func, ...) TEST_AND(err, 1, func, __VA_ARGS__)

#define START_TESTS(type)                               \
	static int test_##type(void)                    \
	{                                               \
		int tests_passed = 0, tests_failed = 0; \
		int sk = sk_##type;

#define END_TESTS(type)                                                   \
	fprintf(stderr, "%s summary: %d tests passed, %d tests failed\n", \
		__func__, tests_passed, tests_failed);                    \
	return tests_failed;                                              \
	}

static struct sockaddr_in saddr = { .sin_port = 0xbeef };
#define psaddr ((struct sockaddr *)&saddr)
#define IN_LEN sizeof(struct sockaddr_in)

START_TESTS(unbound)
{
	socklen_t alen = IN_LEN;
	TEST_AND(0, alen == IN_LEN && saddr.sin_port == 0, getsockname, psaddr,
		 &alen);

	TEST(ENOTCONN, getpeername, psaddr, &alen);
}
END_TESTS()

START_TESTS(bound)
{
	socklen_t alen = IN_LEN;
	TEST_AND(0, alen == IN_LEN && saddr.sin_port == C_PORT, getsockname,
		 psaddr, &alen);

	TEST(ENOTCONN, getpeername, psaddr, &alen);
}
END_TESTS()

START_TESTS(connected)
{
	socklen_t alen = IN_LEN;
	saddr.sin_port = C_PORT;
	TEST_AND(0, alen == IN_LEN && saddr.sin_port != C_PORT, getsockname,
		 psaddr, &alen);

	TEST_AND(0, alen == IN_LEN && saddr.sin_port == C_PORT, getpeername,
		 psaddr, &alen);
}
END_TESTS()

static int do_tests(void)
{
	int tests_failed = 0;

	tests_failed += test_unbound();

	tests_failed += test_bound();

	tests_failed += test_connected();

	if (tests_failed == 0) {
		fprintf(stderr, "All tests passed!\n");
		return 0;
	} else {
		fprintf(stderr, "Some tests failed..\n");
		return -1;
	}
}

int main()
{
	signal(SIGPIPE, SIG_IGN);

	do_setup();

	return do_tests();
}
