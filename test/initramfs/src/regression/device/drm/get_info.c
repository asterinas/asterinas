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
	drmVersionPtr version = TEST_RES(drmGetVersion(card_fd),
					 _ret != NULL && _ret->name != NULL &&
						 _ret->date != NULL &&
						 _ret->desc != NULL);
	if (version == NULL || version->name == NULL || version->date == NULL ||
	    version->desc == NULL) {
		if (version != NULL) {
			drmFreeVersion(version);
		}
		goto out;
	}

	TEST_RES(strlen(version->name), _ret > 0 && version->name_len == _ret);
	TEST_RES(strlen(version->date), _ret > 0 && version->date_len == _ret);
	TEST_RES(strlen(version->desc), _ret > 0 && version->desc_len == _ret);
	drmFreeVersion(version);

out:;
}
END_TEST()

FN_TEST(capabilities)
{
	uint64_t prime = 0;
	uint64_t timestamp_monotonic = 0;
	uint64_t syncobj = 0;
	uint64_t syncobj_timeline = 0;
	uint64_t invalid_cap = 0;

	TEST_RES(drmGetCap(card_fd, DRM_CAP_PRIME, &prime),
		 (prime & ~(DRM_PRIME_CAP_IMPORT | DRM_PRIME_CAP_EXPORT)) == 0);

	TEST_RES(drmGetCap(card_fd, DRM_CAP_TIMESTAMP_MONOTONIC,
			   &timestamp_monotonic),
		 timestamp_monotonic == 1);

	TEST_RES(drmGetCap(card_fd, DRM_CAP_SYNCOBJ, &syncobj),
		 is_boolean_drm_cap(syncobj));

	TEST_RES(drmGetCap(card_fd, DRM_CAP_SYNCOBJ_TIMELINE,
			   &syncobj_timeline),
		 is_boolean_drm_cap(syncobj_timeline) &&
			 (!syncobj_timeline || syncobj));

	TEST_ERRNO(drmGetCap(card_fd, UINT64_MAX, &invalid_cap), EINVAL);
}
END_TEST()

FN_SETUP(close_card)
{
	CHECK(close(card_fd));
}
END_SETUP()
