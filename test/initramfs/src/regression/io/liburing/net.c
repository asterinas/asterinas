// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <arpa/inet.h>
#include <errno.h>
#include <liburing.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#define QUEUE_DEPTH 8
static const char CLIENT_MESSAGE[] = "Asterinas io_uring client message";
static const char SERVER_MESSAGE[] = "Asterinas io_uring server response";

#define CLIENT_PORT_START 40000
#define CLIENT_PORT_END 40100

enum {
	CLIENT_CONNECT_USER_DATA = 0x201,
	CLIENT_SENDMSG_USER_DATA = 0x202,
	CLIENT_RECVMSG_USER_DATA = 0x203,
	SERVER_ACCEPT_USER_DATA = 0x204,
	SERVER_RECVMSG_USER_DATA = 0x205,
	SERVER_SENDMSG_USER_DATA = 0x206,
};

static void fail_errno(const char *message)
{
	perror(message);
	exit(EXIT_FAILURE);
}

static void fail_uring(const char *message, int ret)
{
	errno = -ret;
	perror(message);
	exit(EXIT_FAILURE);
}

static void fail_message(const char *message)
{
	fprintf(stderr, "%s\n", message);
	exit(EXIT_FAILURE);
}

static void init_ring(struct io_uring *ring)
{
	int ret = io_uring_queue_init(QUEUE_DEPTH, ring, 0);
	if (ret < 0) {
		fail_uring("io_uring_queue_init", ret);
	}
}

static struct io_uring_sqe *get_sqe(struct io_uring *ring,
				    const char *operation)
{
	struct io_uring_sqe *sqe = io_uring_get_sqe(ring);
	if (sqe == NULL) {
		fprintf(stderr, "%s: failed to get an SQE\n", operation);
		exit(EXIT_FAILURE);
	}

	return sqe;
}

static void submit_one(struct io_uring *ring, const char *operation)
{
	int ret = io_uring_submit(ring);
	if (ret < 0) {
		fail_uring(operation, ret);
	}
	if (ret != 1) {
		fprintf(stderr, "%s: submitted %d SQEs, expected 1\n",
			operation, ret);
		exit(EXIT_FAILURE);
	}
}

static int wait_cqe_res_user_data(struct io_uring *ring, const char *operation,
				  unsigned long long expected_user_data)
{
	struct io_uring_cqe *cqe;
	int ret = io_uring_wait_cqe(ring, &cqe);
	if (ret < 0) {
		fail_uring(operation, ret);
	}

	if ((unsigned long long)cqe->user_data != expected_user_data) {
		fprintf(stderr, "%s: got user_data %llu, expected %llu\n",
			operation, (unsigned long long)cqe->user_data,
			expected_user_data);
		exit(EXIT_FAILURE);
	}

	int res = cqe->res;
	io_uring_cqe_seen(ring, cqe);
	if (res < 0) {
		fail_uring(operation, res);
	}

	return res;
}

static void expect_cqe_res_user_data(struct io_uring *ring,
				     const char *operation,
				     unsigned long long expected_user_data,
				     int expected_res)
{
	int res = wait_cqe_res_user_data(ring, operation, expected_user_data);
	if (res != expected_res) {
		fprintf(stderr, "%s: got CQE result %d, expected %d\n",
			operation, res, expected_res);
		exit(EXIT_FAILURE);
	}
}

static void sendmsg_all(struct io_uring *ring, int fd, const char *buffer,
			size_t len, const char *operation,
			unsigned long long user_data)
{
	size_t total_len = 0;
	while (total_len < len) {
		struct iovec iov = {
			.iov_base = (void *)(buffer + total_len),
			.iov_len = len - total_len,
		};
		struct msghdr msg = {
			.msg_iov = &iov,
			.msg_iovlen = 1,
		};

		struct io_uring_sqe *sqe = get_sqe(ring, operation);
		io_uring_prep_sendmsg(sqe, fd, &msg, 0);
		sqe->user_data = user_data;
		submit_one(ring, operation);

		int sent_len =
			wait_cqe_res_user_data(ring, operation, user_data);
		if (sent_len <= 0) {
			fail_message("io_uring sendmsg returned zero");
		}
		total_len += sent_len;
	}
}

static void recvmsg_exact(struct io_uring *ring, int fd, char *buffer,
			  size_t len, const char *operation,
			  unsigned long long user_data)
{
	size_t total_len = 0;
	while (total_len < len) {
		struct iovec iov = {
			.iov_base = buffer + total_len,
			.iov_len = len - total_len,
		};
		struct msghdr msg = {
			.msg_iov = &iov,
			.msg_iovlen = 1,
		};

		struct io_uring_sqe *sqe = get_sqe(ring, operation);
		io_uring_prep_recvmsg(sqe, fd, &msg, 0);
		sqe->user_data = user_data;
		submit_one(ring, operation);

		int recv_len =
			wait_cqe_res_user_data(ring, operation, user_data);
		if (recv_len <= 0) {
			fail_message("io_uring recvmsg reached EOF");
		}
		total_len += recv_len;
	}
}

static int create_listener(struct sockaddr_in *server_addr)
{
	int listen_fd = socket(AF_INET, SOCK_STREAM, 0);
	if (listen_fd < 0) {
		fail_errno("socket");
	}

	int enable = 1;
	if (setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &enable,
		       sizeof(enable)) < 0) {
		fail_errno("setsockopt");
	}

	memset(server_addr, 0, sizeof(*server_addr));
	server_addr->sin_family = AF_INET;
	server_addr->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	server_addr->sin_port = 0;

	if (bind(listen_fd, (struct sockaddr *)server_addr,
		 sizeof(*server_addr)) < 0) {
		fail_errno("bind");
	}
	if (listen(listen_fd, 8) < 0) {
		fail_errno("listen");
	}

	socklen_t addr_len = sizeof(*server_addr);
	if (getsockname(listen_fd, (struct sockaddr *)server_addr, &addr_len) <
	    0) {
		fail_errno("getsockname");
	}

	printf("[io_uring:net] listening on 127.0.0.1:%u\n",
	       ntohs(server_addr->sin_port));
	return listen_fd;
}

static void bind_client_socket(int sock)
{
	struct sockaddr_in client_addr;
	memset(&client_addr, 0, sizeof(client_addr));
	client_addr.sin_family = AF_INET;
	client_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

	for (int port = CLIENT_PORT_START; port < CLIENT_PORT_END; port++) {
		client_addr.sin_port = htons(port);
		if (bind(sock, (struct sockaddr *)&client_addr,
			 sizeof(client_addr)) == 0) {
			printf("[io_uring:net] client bound to 127.0.0.1:%d\n",
			       port);
			return;
		}
		if (errno != EADDRINUSE) {
			fail_errno("client bind");
		}
	}

	fail_message("no client port is available");
}

static void run_network_client(struct sockaddr_in server_addr)
{
	alarm(30);

	struct io_uring ring;
	init_ring(&ring);

	int sock = socket(AF_INET, SOCK_STREAM, 0);
	if (sock < 0) {
		fail_errno("client socket");
	}
	bind_client_socket(sock);

	struct io_uring_sqe *sqe = get_sqe(&ring, "io_uring connect");
	io_uring_prep_connect(sqe, sock, (struct sockaddr *)&server_addr,
			      sizeof(server_addr));
	sqe->user_data = CLIENT_CONNECT_USER_DATA;
	submit_one(&ring, "io_uring connect submit");
	expect_cqe_res_user_data(&ring, "io_uring connect",
				 CLIENT_CONNECT_USER_DATA, 0);
	printf("[io_uring:net] client connected\n");

	sendmsg_all(&ring, sock, CLIENT_MESSAGE, sizeof(CLIENT_MESSAGE),
		    "io_uring client sendmsg", CLIENT_SENDMSG_USER_DATA);

	char response[sizeof(SERVER_MESSAGE)] = {};
	recvmsg_exact(&ring, sock, response, sizeof(response),
		      "io_uring client recvmsg", CLIENT_RECVMSG_USER_DATA);

	if (memcmp(response, SERVER_MESSAGE, sizeof(SERVER_MESSAGE)) != 0) {
		fail_message("io_uring client received unexpected data");
	}
	printf("[io_uring:net] client exchange verified\n");

	if (close(sock) < 0) {
		fail_errno("client close");
	}
	io_uring_queue_exit(&ring);
	exit(EXIT_SUCCESS);
}

static void test_network_io(void)
{
	struct sockaddr_in server_addr;
	int listen_fd = create_listener(&server_addr);

	pid_t child = fork();
	if (child < 0) {
		fail_errno("fork");
	}
	if (child == 0) {
		if (close(listen_fd) < 0) {
			fail_errno("child close listen socket");
		}
		run_network_client(server_addr);
	}

	struct io_uring ring;
	init_ring(&ring);

	struct sockaddr_in peer_addr;
	socklen_t peer_addr_len = sizeof(peer_addr);
	struct io_uring_sqe *sqe = get_sqe(&ring, "io_uring accept");
	io_uring_prep_accept(sqe, listen_fd, (struct sockaddr *)&peer_addr,
			     &peer_addr_len, 0);
	sqe->user_data = SERVER_ACCEPT_USER_DATA;
	submit_one(&ring, "io_uring accept submit");
	int accepted_fd = wait_cqe_res_user_data(&ring, "io_uring accept",
						 SERVER_ACCEPT_USER_DATA);
	printf("[io_uring:net] server accepted connection\n");

	char request[sizeof(CLIENT_MESSAGE)] = {};
	recvmsg_exact(&ring, accepted_fd, request, sizeof(request),
		      "io_uring server recvmsg", SERVER_RECVMSG_USER_DATA);

	if (memcmp(request, CLIENT_MESSAGE, sizeof(CLIENT_MESSAGE)) != 0) {
		fail_message("io_uring server received unexpected data");
	}

	sendmsg_all(&ring, accepted_fd, SERVER_MESSAGE, sizeof(SERVER_MESSAGE),
		    "io_uring server sendmsg", SERVER_SENDMSG_USER_DATA);
	printf("[io_uring:net] server exchange verified\n");

	int status;
	if (waitpid(child, &status, 0) != child) {
		fail_errno("waitpid");
	}
	if (!WIFEXITED(status) || WEXITSTATUS(status) != EXIT_SUCCESS) {
		fail_message("io_uring client exited with failure");
	}

	if (close(accepted_fd) < 0) {
		fail_errno("close accepted socket");
	}
	if (close(listen_fd) < 0) {
		fail_errno("close listen socket");
	}

	io_uring_queue_exit(&ring);
}

int main(void)
{
	setbuf(stdout, NULL);
	alarm(30);
	printf("[io_uring:net] starting\n");

	test_network_io();

	printf("[io_uring:net] PASS\n");
	return EXIT_SUCCESS;
}
