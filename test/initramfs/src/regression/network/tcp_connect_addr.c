// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#include "../common/test.h"

// Regression for the SyzOxide report "assertion failed: addr.is_unicast()"
// at smoltcp/src/iface/route.rs:150, triggered by TCP connect to 224.0.0.2.
// Use finite calls and verify errno, socket state, and subsequent TCP I/O.

union ip_sockaddr {
	struct sockaddr sa;
	struct sockaddr_in v4;
	struct sockaddr_in6 v6;
};

static socklen_t address_len(int family)
{
	return family == AF_INET ? sizeof(struct sockaddr_in) :
				   sizeof(struct sockaddr_in6);
}

static union ip_sockaddr address(int family, const char *text, in_port_t port)
{
	union ip_sockaddr addr = { 0 };

	if (family == AF_INET) {
		addr.v4.sin_family = AF_INET;
		addr.v4.sin_port = port;
		CHECK_WITH(inet_pton(family, text, &addr.v4.sin_addr),
			   _ret == 1);
	} else {
		addr.v6.sin6_family = AF_INET6;
		addr.v6.sin6_port = port;
		CHECK_WITH(inet_pton(family, text, &addr.v6.sin6_addr),
			   _ret == 1);
	}
	return addr;
}

static in_port_t port_of(const union ip_sockaddr *addr)
{
	return addr->sa.sa_family == AF_INET ? addr->v4.sin_port :
					       addr->v6.sin6_port;
}

static int same_endpoint(const union ip_sockaddr *a, const union ip_sockaddr *b)
{
	if (a->sa.sa_family != b->sa.sa_family || port_of(a) != port_of(b))
		return 0;
	if (a->sa.sa_family == AF_INET)
		return a->v4.sin_addr.s_addr == b->v4.sin_addr.s_addr;
	return memcmp(&a->v6.sin6_addr, &b->v6.sin6_addr,
		      sizeof(a->v6.sin6_addr)) == 0;
}

static union ip_sockaddr local_endpoint(int fd, int family)
{
	union ip_sockaddr addr = { 0 };
	socklen_t len = sizeof(addr);

	CHECK_WITH(getsockname(fd, &addr.sa, &len),
		   _ret == 0 && len == address_len(family));
	return addr;
}

static void wait_ready(int fd, short events)
{
	struct pollfd pfd = { .fd = fd, .events = events };

	CHECK_WITH(poll(&pfd, 1, 5000),
		   _ret == 1 && (pfd.revents & events) == events &&
			   !(pfd.revents & (POLLERR | POLLNVAL)));
}

static void set_nonblocking(int fd)
{
	int flags = CHECK(fcntl(fd, F_GETFL));

	CHECK(fcntl(fd, F_SETFL, flags | O_NONBLOCK));
}

static void no_pending_error(int fd)
{
	int error = -1;
	socklen_t len = sizeof(error);

	CHECK_WITH(getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &len),
		   _ret == 0 && len == sizeof(error) && error == 0);
}

static void exchange_byte(int sender, int receiver, char value)
{
	char received = 0;

	wait_ready(sender, POLLOUT);
	CHECK_WITH(send(sender, &value, 1, 0), _ret == 1);
	wait_ready(receiver, POLLIN);
	CHECK_WITH(recv(receiver, &received, 1, 0),
		   _ret == 1 && received == value);
}

static int check_connect_case(int family, int is_bound, int nonblocking,
			      int use_unspecified)
{
	const char *loopback = family == AF_INET ? "127.0.0.1" : "::1";
	const char *unspecified = family == AF_INET ? "0.0.0.0" : "::";
	const char *v4_invalid[] = {
		"224.0.0.2",	   "224.0.0.1",	      "239.255.255.250",
		"255.255.255.255", "127.255.255.255",
	};
	const char *v6_invalid[] = {
		"ff02::1",
		"ff02::2",
		"ff0e::1234",
		// These aliases must not bypass TCP's group-address validation.
		"::ffff:224.0.0.2",
		"::ffff:239.255.255.250",
		"::ffff:255.255.255.255",
		"::ffff:127.255.255.255",
	};
	const char **invalid = family == AF_INET ? v4_invalid : v6_invalid;
	size_t count = family == AF_INET ?
			       sizeof(v4_invalid) / sizeof(*invalid) :
			       sizeof(v6_invalid) / sizeof(*invalid);
	socklen_t len = address_len(family);
	union ip_sockaddr listen_addr = address(family, loopback, 0);
	int listener = CHECK(socket(family, SOCK_STREAM, 0));

	set_nonblocking(listener);
	CHECK(bind(listener, &listen_addr.sa, len));
	CHECK(listen(listener, 1));
	listen_addr = local_endpoint(listener, family);

	int client = CHECK(socket(family, SOCK_STREAM, 0));
	struct timeval timeout = { .tv_sec = 5 };

	if (nonblocking)
		set_nonblocking(client);
	CHECK(setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			 sizeof(timeout)));
	if (is_bound) {
		union ip_sockaddr bind_addr = address(family, loopback, 0);

		CHECK(bind(client, &bind_addr.sa, len));
	}
	union ip_sockaddr before = local_endpoint(client, family);

	for (size_t i = 0; i < count; i++) {
		union ip_sockaddr remote =
			address(family, invalid[i], htons(20004));
		union ip_sockaddr peer = { 0 };
		socklen_t peer_len = sizeof(peer);

		// SO_BROADCAST must not make a TCP group destination valid.
		int enabled = 1;
		CHECK(setsockopt(client, SOL_SOCKET, SO_BROADCAST, &enabled,
				 sizeof(enabled)));
		errno = 0;
		int result = connect(client, &remote.sa, len);
		int error = errno;

		fprintf(stderr,
			"connect(%s:20004) bound=%d nonblock=%d retry_any=%d: ret=%d errno=%d\n",
			invalid[i], is_bound, nonblocking, use_unspecified,
			result, error);
		CHECK_WITH(result, _ret == -1 && error == ENETUNREACH);
		union ip_sockaddr after = local_endpoint(client, family);
		CHECK_WITH(same_endpoint(&before, &after), _ret == 1);
		CHECK_WITH(getpeername(client, &peer.sa, &peer_len),
			   _ret == -1 && errno == ENOTCONN);
		no_pending_error(client);
	}

	// The same descriptor must still support a normal connection after errors.
	union ip_sockaddr remote =
		address(family, use_unspecified ? unspecified : loopback,
			port_of(&listen_addr));
	CHECK_WITH(connect(client, &remote.sa, len),
		   _ret == 0 ||
			   (nonblocking && _ret == -1 && errno == EINPROGRESS));
	wait_ready(client, POLLOUT);
	no_pending_error(client);
	wait_ready(listener, POLLIN);
	int accepted = CHECK(accept(listener, NULL, NULL));

	union ip_sockaddr peer = { 0 };
	socklen_t peer_len = sizeof(peer);
	CHECK_WITH(getpeername(client, &peer.sa, &peer_len),
		   _ret == 0 && peer_len == len &&
			   same_endpoint(&peer, &listen_addr));
	union ip_sockaddr after = local_endpoint(client, family);
	if (is_bound)
		CHECK_WITH(same_endpoint(&before, &after), _ret == 1);
	else
		CHECK_WITH(port_of(&after), _ret != 0);

	// Switch only after exercising the nonblocking connect path.
	int flags = CHECK(fcntl(client, F_GETFL));
	CHECK(fcntl(client, F_SETFL, flags & ~O_NONBLOCK));
	exchange_byte(client, accepted, 'q');
	exchange_byte(accepted, client, 'r');
	CHECK(close(accepted));
	CHECK(close(client));
	CHECK(close(listener));
	return 0;
}

static int check_state_errors(int family)
{
	const char *loopback = family == AF_INET ? "127.0.0.1" : "::1";
	const char *multicast = family == AF_INET ? "224.0.0.2" : "ff02::1";
	union ip_sockaddr local = address(family, loopback, 0);
	union ip_sockaddr invalid = address(family, multicast, htons(20004));
	socklen_t len = address_len(family);
	int listener = CHECK(socket(family, SOCK_STREAM, 0));
	int client = CHECK(socket(family, SOCK_STREAM, 0));
	struct timeval timeout = { .tv_sec = 5 };

	CHECK(setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			 sizeof(timeout)));
	CHECK(bind(listener, &local.sa, len));
	CHECK(listen(listener, 1));
	local = local_endpoint(listener, family);

	// State errors retain precedence over destination validation.
	CHECK_WITH(connect(listener, &invalid.sa, len),
		   _ret == -1 && errno == EISCONN);
	CHECK(connect(client, &local.sa, len));
	wait_ready(listener, POLLIN);
	int accepted = CHECK(accept(listener, NULL, NULL));
	CHECK_WITH(connect(client, &invalid.sa, len),
		   _ret == -1 && errno == EISCONN);
	exchange_byte(client, accepted, 's');
	exchange_byte(accepted, client, 't');
	CHECK(close(accepted));
	CHECK(close(client));
	CHECK(close(listener));
	return 0;
}

FN_SETUP(deadline)
{
	// A broken connect must not stall the regression suite indefinitely.
	alarm(180);
}
END_SETUP()

FN_TEST(tcp_destination_validation_and_recovery)
{
	const int families[] = { AF_INET, AF_INET6 };

	// IPv6 TCP loopback is supported; do not silently skip IPv6 failures.
	for (size_t i = 0; i < sizeof(families) / sizeof(*families); i++)
		for (int bound = 0; bound < 2; bound++)
			for (int nonblocking = 0; nonblocking < 2;
			     nonblocking++)
				for (int unspecified = 0; unspecified < 2;
				     unspecified++)
					TEST_RES(check_connect_case(
							 families[i], bound,
							 nonblocking,
							 unspecified),
						 _ret == 0);
}
END_TEST()

FN_TEST(tcp_connect_state_errors)
{
	TEST_RES(check_state_errors(AF_INET), _ret == 0);
	TEST_RES(check_state_errors(AF_INET6), _ret == 0);
}
END_TEST()
