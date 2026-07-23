// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include "common.h"

static int card_fd = -1;
static int render_fd = -1;
static int skip_render_node;

FN_SETUP(open_nodes)
{
	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);

	CHECK(should_skip_drm_render_node(card_fd, &skip_render_node));
	if (!skip_render_node) {
		render_fd = open_drm_node_or_skip(DRM_RENDER_DEVICE);
	}
}
END_SETUP()

FN_TEST(render_node_restrictions)
{
	/* simpledrm does not expose a render node. */
	SKIP_TEST_IF(skip_render_node);

	TEST_ERRNO(set_drm_client_cap(render_fd, DRM_CLIENT_CAP_ATOMIC, 1),
		   EACCES);
	TEST_ERRNO(ioctl(render_fd, DRM_IOCTL_SET_MASTER), EACCES);
	TEST_ERRNO(ioctl(render_fd, DRM_IOCTL_DROP_MASTER), EACCES);
}
END_TEST()

FN_TEST(primary_client_caps)
{
	int unconfigured_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));

	/*
	 * WRITEBACK_CONNECTORS is only valid after enabling ATOMIC on the same
	 * file description. Use a fresh fd so the failure is not hidden by state
	 * set by earlier ioctls on card_fd.
	 */
	TEST_ERRNO(set_drm_client_cap(unconfigured_fd,
				      DRM_CLIENT_CAP_WRITEBACK_CONNECTORS, 1),
		   EINVAL);
	TEST_SUCC(close(unconfigured_fd));

	TEST_SUCC(set_drm_client_cap(card_fd, DRM_CLIENT_CAP_ATOMIC, 0));
	TEST_SUCC(set_drm_client_cap(card_fd, DRM_CLIENT_CAP_ATOMIC, 1));
	TEST_SUCC(set_drm_client_cap(card_fd, DRM_CLIENT_CAP_ATOMIC, 2));
	TEST_ERRNO(set_drm_client_cap(card_fd, DRM_CLIENT_CAP_ATOMIC, 3),
		   EINVAL);

	TEST_SUCC(set_drm_client_cap(card_fd,
				     DRM_CLIENT_CAP_WRITEBACK_CONNECTORS, 0));
	TEST_SUCC(set_drm_client_cap(card_fd,
				     DRM_CLIENT_CAP_WRITEBACK_CONNECTORS, 1));
	TEST_ERRNO(set_drm_client_cap(card_fd,
				      DRM_CLIENT_CAP_WRITEBACK_CONNECTORS, 2),
		   EINVAL);
}
END_TEST()

FN_TEST(primary_master)
{
	int second_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));

	/*
	 * The first primary-node open owns DRM master. A second primary fd cannot
	 * become master until the original owner drops it.
	 */
	TEST_ERRNO(ioctl(second_fd, DRM_IOCTL_SET_MASTER), EBUSY);
	TEST_SUCC(ioctl(card_fd, DRM_IOCTL_DROP_MASTER));
	TEST_SUCC(ioctl(second_fd, DRM_IOCTL_SET_MASTER));
	TEST_SUCC(ioctl(second_fd, DRM_IOCTL_DROP_MASTER));

	TEST_SUCC(close(second_fd));
}
END_TEST()

FN_SETUP(close_nodes)
{
	if (!skip_render_node) {
		CHECK(close(render_fd));
	}
	CHECK(close(card_fd));
}
END_SETUP()
