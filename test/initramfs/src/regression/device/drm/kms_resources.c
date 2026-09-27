// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include "common.h"

static int card_fd = -1;
static drmModeResPtr resources;
static drmModePlaneResPtr legacy_planes;
static drmModePlaneResPtr universal_planes;

static int has_required_resources(void)
{
	return resources->count_crtcs > 0 && resources->count_encoders > 0 &&
			       resources->count_connectors > 0 &&
			       resources->min_width <= resources->max_width &&
			       resources->min_height <= resources->max_height &&
			       resources->max_width > 0 &&
			       resources->max_height > 0 ?
		       0 :
		       -1;
}

static int has_universal_planes(void)
{
	return universal_planes->count_planes > legacy_planes->count_planes ?
		       0 :
		       -1;
}

static int id_set_contains(const uint32_t *set, uint32_t set_count,
			   const uint32_t *subset, uint32_t subset_count)
{
	for (uint32_t i = 0; i < subset_count; i++) {
		if (!drm_id_in_array(set, set_count, subset[i])) {
			return 0;
		}
	}

	return 1;
}

static int id_sets_are_equal(const uint32_t *first, uint32_t first_count,
			     const uint32_t *second, uint32_t second_count)
{
	return first_count == second_count &&
	       id_set_contains(first, first_count, second, second_count);
}

FN_SETUP(load_kms_resources)
{
	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
	resources = get_kms_resources_or_skip(card_fd);
	CHECK_WITH(has_required_resources(), _ret == 0);

	legacy_planes =
		CHECK_WITH(drmModeGetPlaneResources(card_fd), _ret != NULL);
	CHECK(drmSetClientCap(card_fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1));
	universal_planes =
		CHECK_WITH(drmModeGetPlaneResources(card_fd), _ret != NULL);
	CHECK_WITH(has_universal_planes(), _ret == 0);
}
END_SETUP()

FN_TEST(universal_plane_capability)
{
	TEST_RES(id_set_contains(universal_planes->planes,
				 universal_planes->count_planes,
				 legacy_planes->planes,
				 legacy_planes->count_planes),
		 _ret);

	int default_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	drmModePlaneResPtr default_planes =
		TEST_RES(drmModeGetPlaneResources(default_fd), _ret != NULL);
	TEST_RES(id_sets_are_equal(
			 legacy_planes->planes, legacy_planes->count_planes,
			 default_planes->planes, default_planes->count_planes),
		 _ret);
	drmModeFreePlaneResources(default_planes);
	TEST_SUCC(close(default_fd));
}
END_TEST()

FN_TEST(resource_topology)
{
	drmModeCrtcPtr crtc =
		TEST_RES(drmModeGetCrtc(card_fd, resources->crtcs[0]),
			 _ret != NULL && _ret->crtc_id == resources->crtcs[0]);
	drmModeFreeCrtc(crtc);

	drmModeEncoderPtr encoder = TEST_RES(
		drmModeGetEncoder(card_fd, resources->encoders[0]),
		_ret != NULL && _ret->encoder_id == resources->encoders[0] &&
			_ret->possible_crtcs != 0);
	drmModeFreeEncoder(encoder);

	drmModeConnectorPtr connector = TEST_RES(
		drmModeGetConnector(card_fd, resources->connectors[0]),
		_ret != NULL &&
			_ret->connector_id == resources->connectors[0] &&
			_ret->count_encoders > 0 &&
			drm_id_in_array(resources->encoders,
					resources->count_encoders,
					_ret->encoders[0]));
	drmModeFreeConnector(connector);

	drmModePlanePtr plane = TEST_RES(
		drmModeGetPlane(card_fd, universal_planes->planes[0]),
		_ret != NULL && _ret->plane_id == universal_planes->planes[0] &&
			_ret->possible_crtcs != 0 && _ret->count_formats > 0);
	drmModeFreePlane(plane);
}
END_TEST()

FN_TEST(array_pointer_semantics)
{
	struct drm_mode_get_plane_res plane_args = { 0 };
	uint32_t first_plane = 0;

	TEST_RES(ioctl(card_fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &plane_args),
		 _ret == 0 && plane_args.count_planes ==
				      universal_planes->count_planes);

	uint32_t total_planes = plane_args.count_planes;
	plane_args.count_planes = 1;
	TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_GETPLANERESOURCES,
			 &plane_args),
		   EFAULT);

	plane_args.plane_id_ptr = (uintptr_t)&first_plane;
	TEST_RES(ioctl(card_fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &plane_args),
		 _ret == 0 && plane_args.count_planes == total_planes &&
			 drm_id_in_array(universal_planes->planes,
					 universal_planes->count_planes,
					 first_plane));

	struct drm_mode_card_res resource_args = { 0 };
	TEST_SUCC(ioctl(card_fd, DRM_IOCTL_MODE_GETRESOURCES, &resource_args));
	resource_args.count_crtcs = 1;
	resource_args.count_connectors = 0;
	resource_args.count_encoders = 0;
	resource_args.count_fbs = 0;
	TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_GETRESOURCES, &resource_args),
		   EFAULT);
}
END_TEST()

FN_TEST(invalid_object_ids)
{
	TEST_ERRNO(drmModeGetCrtc(card_fd, UINT32_MAX), ENOENT);
	TEST_ERRNO(drmModeGetEncoder(card_fd, UINT32_MAX), ENOENT);
	TEST_ERRNO(drmModeGetConnector(card_fd, UINT32_MAX), ENOENT);
	TEST_ERRNO(drmModeGetPlane(card_fd, UINT32_MAX), ENOENT);
}
END_TEST()

FN_SETUP(release_kms_resources)
{
	drmModeFreePlaneResources(universal_planes);
	drmModeFreePlaneResources(legacy_planes);
	drmModeFreeResources(resources);
	CHECK(close(card_fd));
}
END_SETUP()
