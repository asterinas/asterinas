// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include "common.h"

static int card_fd = -1;

FN_SETUP(open_card)
{
	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
}
END_SETUP()

FN_TEST(version)
{
	struct drm_version version;
	char name[DRM_FIELD_LEN];
	char date[DRM_FIELD_LEN];
	char desc[DRM_FIELD_LEN];

	TEST_SUCC(get_drm_version(card_fd, &version, name, date, desc,
				  sizeof(name)));
	TEST_RES(strlen(name), _ret != 0 && _ret == version.name_len);
	TEST_RES(strlen(date), _ret != 0 && _ret == version.date_len);
	TEST_RES(strlen(desc), _ret != 0 && _ret == version.desc_len);
}
END_TEST()

FN_TEST(capabilities)
{
	uint64_t dumb_buffer = 0;
	uint64_t preferred_depth = 0;
	uint64_t prime = 0;
	uint64_t timestamp_monotonic = 0;
	uint64_t cursor_width = 0;
	uint64_t cursor_height = 0;
	uint64_t syncobj = 0;
	uint64_t syncobj_timeline = 0;
	uint64_t invalid_cap = 0;

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_DUMB_BUFFER, &dumb_buffer));
	TEST_RES(is_boolean_drm_cap(dumb_buffer), _ret);

	/*
	 * These capability checks intentionally avoid pinning simpledrm's exact
	 * policy values. The test validates each value's ABI shape instead:
	 * boolean capabilities are 0/1, dimensions are in a sane positive range,
	 * and bitmask capabilities do not report undefined bits.
	 */
	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_DUMB_PREFERRED_DEPTH,
			      &preferred_depth));
	TEST_RES(preferred_depth,
		 preferred_depth >= 16 && preferred_depth <= 32);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_PRIME, &prime));
	TEST_RES(prime,
		 (prime & ~(DRM_PRIME_CAP_IMPORT | DRM_PRIME_CAP_EXPORT)) == 0);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_TIMESTAMP_MONOTONIC,
			      &timestamp_monotonic));
	TEST_RES(is_boolean_drm_cap(timestamp_monotonic), _ret);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_CURSOR_WIDTH, &cursor_width));
	TEST_RES(cursor_width, cursor_width > 0 && cursor_width <= 4096);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_CURSOR_HEIGHT, &cursor_height));
	TEST_RES(cursor_height, cursor_height > 0 && cursor_height <= 4096);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_SYNCOBJ, &syncobj));
	TEST_RES(is_boolean_drm_cap(syncobj), _ret);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_SYNCOBJ_TIMELINE,
			      &syncobj_timeline));
	TEST_RES(is_boolean_drm_cap(syncobj_timeline), _ret);

	TEST_ERRNO(get_drm_cap(card_fd, UINT64_MAX, &invalid_cap), EINVAL);
}
END_TEST()

FN_SETUP(close_card)
{
	CHECK(close(card_fd));
}
END_SETUP()
