// SPDX-License-Identifier: MPL-2.0

#include <err.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/eventfd.h>
#include <unistd.h>

int main()
{
	int efd;
	uint64_t u;
	ssize_t s;

	uint64_t values[] = { 11, 222, 3333 };
	size_t length = sizeof(values) / sizeof(values[0]);

	efd = eventfd(0, 0);
	if (efd == -1)
		err(EXIT_FAILURE, "eventfd");

	switch (fork()) {
	case 0:
		for (size_t j = 0; j < length; j++) {
			printf("Child writing %ld to efd\n", values[j]);
			u = values[j]; /* strtoull() allows various bases */
			s = write(efd, &u, sizeof(uint64_t));
			if (s != sizeof(uint64_t))
				err(EXIT_FAILURE, "write");
		}

		printf("Child completed write loop\n");

		exit(EXIT_SUCCESS);

	default:
		sleep(2);

		printf("Parent about to read\n");
		s = read(efd, &u, sizeof(uint64_t));
		if (s != sizeof(uint64_t))
			err(EXIT_FAILURE, "read");
		printf("Parent read %" PRIu64 " (%#" PRIx64 ") from efd\n", u,
		       u);
		if (u != 11 + 222 + 3333) {
			err(EXIT_FAILURE, "read eventfd");
			exit(EXIT_FAILURE);
		}
		exit(EXIT_SUCCESS);

	case -1:
		err(EXIT_FAILURE, "fork");
	}
}
