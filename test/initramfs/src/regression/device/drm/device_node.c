// SPDX-License-Identifier: MPL-2.0

#include <poll.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>

#include "../../common/test.h"
#include "common.h"

static int skip_render_node;

FN_SETUP(check_nodes_available)
{
	int card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);

	CHECK(should_skip_drm_render_node(card_fd, &skip_render_node));
	if (!skip_render_node) {
		int render_fd = open_drm_node_or_skip(DRM_RENDER_DEVICE);

		CHECK(close(render_fd));
	}

	CHECK(close(card_fd));
}
END_SETUP()

FN_TEST(rdev)
{
	size_t node_count = 0;
	const struct drm_node *nodes = drm_test_nodes(&node_count);

	for (size_t i = 0; i < node_count; i++) {
		const struct drm_node *node = &nodes[i];
		struct stat stat_buf;

		if (skip_render_node && is_drm_render_node(node)) {
			continue;
		}

		TEST_RES(stat(node->path, &stat_buf),
			 S_ISCHR(stat_buf.st_mode) &&
				 stat_buf.st_rdev ==
					 makedev(node->major, node->minor));
	}
}
END_TEST()

FN_TEST(file_ops)
{
	char buf = 0;
	size_t node_count = 0;
	const struct drm_node *nodes = drm_test_nodes(&node_count);

	for (size_t i = 0; i < node_count; i++) {
		if (skip_render_node && is_drm_render_node(&nodes[i])) {
			continue;
		}

		int fd = TEST_SUCC(open(nodes[i].path, O_RDWR));
		struct pollfd poll_fd = {
			.fd = fd,
			.events = POLLIN | POLLOUT,
		};

		TEST_ERRNO(read(fd, &buf, sizeof(buf)), EINVAL);
		TEST_ERRNO(write(fd, &buf, sizeof(buf)), EINVAL);
		TEST_RES(poll(&poll_fd, 1, 0),
			 _ret == 1 && poll_fd.revents == (POLLIN | POLLOUT));
		TEST_SUCC(close(fd));
	}
}
END_TEST()
