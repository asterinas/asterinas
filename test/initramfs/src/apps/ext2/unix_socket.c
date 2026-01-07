// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/un.h>
#include "../test.h"

#define SERVER_ADDRESS "/ext2/my_unix_server"

FN_TEST(ext2_unix_socket)
{
	int sk_server = TEST_SUCC(socket(AF_UNIX, SOCK_STREAM, 0));
	struct sockaddr_un server_addr;

	memset(&server_addr, 0, sizeof(server_addr));
	server_addr.sun_family = AF_UNIX;
	strncpy(server_addr.sun_path, SERVER_ADDRESS,
		sizeof(server_addr.sun_path) - 1);

	TEST_SUCC(bind(sk_server, (struct sockaddr *)&server_addr,
		       sizeof(server_addr)));
	TEST_SUCC(listen(sk_server, 3));

	if (TEST_SUCC(fork()) == 0) {
		int sk_client = CHECK(socket(AF_UNIX, SOCK_STREAM, 0));
		CHECK(connect(sk_client, (struct sockaddr *)&server_addr,
			      sizeof(server_addr)));

		char buf[1];
		CHECK_WITH(read(sk_client, buf, sizeof(buf)),
			   _ret == 1 && buf[0] == 'a');
		buf[0] = 'b';
		CHECK_WITH(write(sk_client, buf, sizeof(buf)), _ret == 1);
		close(sk_client);

		exit(EXIT_SUCCESS);
	}

	int sk_accepted = TEST_SUCC(accept(sk_server, NULL, NULL));
	char buf[1] = { 'a' };
	TEST_RES(write(sk_accepted, buf, sizeof(buf)), _ret == 1);
	TEST_RES(read(sk_accepted, buf, sizeof(buf)),
		 _ret == 1 && buf[0] == 'b');

	TEST_SUCC(close(sk_accepted));
	TEST_SUCC(close(sk_server));
	TEST_SUCC(unlink(SERVER_ADDRESS));
}
END_TEST()