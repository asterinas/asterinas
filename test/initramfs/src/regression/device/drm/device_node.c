// SPDX-License-Identifier: MPL-2.0

#include <sys/stat.h>
#include <sys/sysmacros.h>

#include "../../common/test.h"
#include "common.h"

static int has_render_node;

FN_SETUP(check_nodes_available)
{
	int card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
	int render_fd = open_optional_drm_node(DRM_RENDER_DEVICE);

	has_render_node = render_fd >= 0;
	if (has_render_node) {
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

		if (!has_render_node && is_drm_render_node(node)) {
			continue;
		}

		TEST_RES(stat(node->path, &stat_buf),
			 S_ISCHR(stat_buf.st_mode) &&
				 stat_buf.st_rdev ==
					 makedev(node->major, node->minor));
	}
}
END_TEST()
