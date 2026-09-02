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
	size_t copied_name_len;
	size_t copied_date_len;
	size_t copied_desc_len;

	TEST_SUCC(get_drm_version(card_fd, &version, name, date, desc,
				  sizeof(name)));

	copied_name_len = version.name_len < sizeof(name) ? version.name_len :
							    sizeof(name);
	copied_date_len = version.date_len < sizeof(date) ? version.date_len :
							    sizeof(date);
	copied_desc_len = version.desc_len < sizeof(desc) ? version.desc_len :
							    sizeof(desc);

	TEST_RES(strnlen(name, sizeof(name)),
		 _ret > 0 && _ret == copied_name_len);
	TEST_RES(strnlen(date, sizeof(date)), _ret == copied_date_len);
	TEST_RES(strnlen(desc, sizeof(desc)), _ret == copied_desc_len);
}
END_TEST()

FN_TEST(capabilities)
{
	uint64_t prime = 0;
	uint64_t timestamp_monotonic = 0;
	uint64_t syncobj = 0;
	uint64_t syncobj_timeline = 0;
	uint64_t invalid_cap = 0;

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_PRIME, &prime));
	TEST_RES(prime,
		 (prime & ~(DRM_PRIME_CAP_IMPORT | DRM_PRIME_CAP_EXPORT)) == 0);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_TIMESTAMP_MONOTONIC,
			      &timestamp_monotonic));
	TEST_RES(timestamp_monotonic, timestamp_monotonic == 1);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_SYNCOBJ, &syncobj));
	TEST_RES(is_boolean_drm_cap(syncobj), _ret);

	TEST_SUCC(get_drm_cap(card_fd, DRM_CAP_SYNCOBJ_TIMELINE,
			      &syncobj_timeline));
	TEST_RES(is_boolean_drm_cap(syncobj_timeline), _ret);
	TEST_RES(syncobj_timeline, !syncobj_timeline || syncobj);

	TEST_ERRNO(get_drm_cap(card_fd, UINT64_MAX, &invalid_cap), EINVAL);
}
END_TEST()

FN_SETUP(close_card)
{
	CHECK(close(card_fd));
}
END_SETUP()
