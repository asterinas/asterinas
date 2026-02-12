// SPDX-License-Identifier: MPL-2.0

#include "pseudo_file_create.h"

static int read_fdinfo_mnt_id(int fd)
{
	char path[64], line[256];
	FILE *f;

	fdinfo_path(fd, path, sizeof(path));
	f = fopen(path, "r");
	if (!f)
		return -1;

	int mnt_id = -1;
	while (fgets(line, sizeof(line), f)) {
		if (CHECK(sscanf(line, "mnt_id:\t%d", &mnt_id)) == 1)
			break;
	}
	CHECK(fclose(f));

	return mnt_id;
}

FN_TEST(pseudo_mount)
{
	int anon[] = { epoll_fd, event_fd, timer_fd, signal_fd, inotify_fd };

	struct fd_group {
		int *fds;
		int nr;
		int mnt_id;
	};

	struct fd_group groups[] = {
		{ pipe_1, 2, -1 },  { sock, 2, -1 },	{ anon, 5, -1 },
		{ &mem_fd, 1, -1 }, { &pid_fd, 1, -1 },
	};

	for (int i = 0; i < 5; i++) {
		int base = TEST_SUCC(read_fdinfo_mnt_id(groups[i].fds[0]));
		for (int j = 1; j < groups[i].nr; j++) {
			TEST_RES(read_fdinfo_mnt_id(groups[i].fds[j]),
				 _ret == base);
		}
		groups[i].mnt_id = base;
	}

	for (int i = 0; i < 5; i++) {
		for (int j = i + 1; j < 5; j++) {
			TEST_RES(0, groups[i].mnt_id != groups[j].mnt_id);
		}
	}
}
END_TEST()

#include "pseudo_file_cleanup.h"
