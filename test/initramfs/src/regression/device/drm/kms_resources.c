// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include "common.h"

static int card_fd = -1;
static drmModeResPtr resources;
static drmModePlaneResPtr legacy_plane_resources;
static drmModePlaneResPtr plane_resources;

/* Values exposed by the standard immutable plane "type" property. */
enum drm_test_plane_type {
	DRM_TEST_PLANE_TYPE_OVERLAY,
	DRM_TEST_PLANE_TYPE_PRIMARY,
	DRM_TEST_PLANE_TYPE_CURSOR,
};

static uint32_t index_mask(uint32_t count)
{
	return count >= 32 ? UINT32_MAX : (1U << count) - 1;
}

static int ids_are_unique(const uint32_t *ids, uint32_t count)
{
	for (uint32_t i = 0; i < count; i++) {
		if (ids[i] == 0) {
			return 0;
		}
		for (uint32_t j = i + 1; j < count; j++) {
			if (ids[i] == ids[j]) {
				return 0;
			}
		}
	}

	return 1;
}

static int id_sets_are_disjoint(const uint32_t *first, uint32_t first_count,
				const uint32_t *second, uint32_t second_count)
{
	for (uint32_t i = 0; i < first_count; i++) {
		if (drm_id_in_array(second, second_count, first[i])) {
			return 0;
		}
	}

	return 1;
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

static int get_plane_type(uint32_t plane_id, uint64_t *plane_type)
{
	drmModeObjectPropertiesPtr properties = drmModeObjectGetProperties(
		card_fd, plane_id, DRM_MODE_OBJECT_PLANE);
	int ret = -1;

	if (!properties) {
		return -1;
	}

	for (uint32_t i = 0; i < properties->count_props; i++) {
		drmModePropertyPtr property =
			drmModeGetProperty(card_fd, properties->props[i]);

		if (!property) {
			goto out;
		}
		if (strcmp(property->name, "type") == 0) {
			uint32_t type = property->flags &
					(DRM_MODE_PROP_LEGACY_TYPE |
					 DRM_MODE_PROP_EXTENDED_TYPE);

			if (type != DRM_MODE_PROP_ENUM ||
			    properties->prop_values[i] >
				    DRM_TEST_PLANE_TYPE_CURSOR) {
				errno = EPROTO;
				drmModeFreeProperty(property);
				goto out;
			}

			*plane_type = properties->prop_values[i];
			drmModeFreeProperty(property);
			ret = 0;
			goto out;
		}
		drmModeFreeProperty(property);
	}

	errno = EPROTO;
out:
	drmModeFreeObjectProperties(properties);
	return ret;
}

FN_SETUP(load_kms_resources)
{
	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
	resources = get_kms_resources_or_skip(card_fd);
	legacy_plane_resources =
		CHECK_WITH(drmModeGetPlaneResources(card_fd), _ret != NULL);
	CHECK(drmSetClientCap(card_fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1));
	plane_resources =
		CHECK_WITH(drmModeGetPlaneResources(card_fd), _ret != NULL);
}
END_SETUP()

FN_TEST(universal_plane_capability)
{
	int default_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));

	TEST_RES(0, ids_are_unique(legacy_plane_resources->planes,
				   legacy_plane_resources->count_planes));
	TEST_RES(0, ids_are_unique(plane_resources->planes,
				   plane_resources->count_planes));
	TEST_RES(0, id_set_contains(plane_resources->planes,
				    plane_resources->count_planes,
				    legacy_plane_resources->planes,
				    legacy_plane_resources->count_planes));
	for (uint32_t i = 0; i < plane_resources->count_planes; i++) {
		uint32_t plane_id = plane_resources->planes[i];
		uint64_t plane_type = UINT64_MAX;
		int is_legacy = drm_id_in_array(
			legacy_plane_resources->planes,
			legacy_plane_resources->count_planes, plane_id);
		int ret = get_plane_type(plane_id, &plane_type);

		TEST_RES(0, ret == 0);
		if (ret == 0) {
			TEST_RES(0, is_legacy == (plane_type ==
						  DRM_TEST_PLANE_TYPE_OVERLAY));
		}
	}

	if (default_fd >= 0) {
		drmModePlaneResPtr default_planes = TEST_RES(
			drmModeGetPlaneResources(default_fd), _ret != NULL);

		if (default_planes) {
			TEST_RES(0,
				 ids_are_unique(default_planes->planes,
						default_planes->count_planes));
			TEST_RES(0,
				 id_sets_are_equal(
					 legacy_plane_resources->planes,
					 legacy_plane_resources->count_planes,
					 default_planes->planes,
					 default_planes->count_planes));
			drmModeFreePlaneResources(default_planes);
		}
		TEST_SUCC(close(default_fd));
	}
}
END_TEST()

FN_TEST(raw_array_pointer_semantics)
{
	struct drm_mode_get_plane_res plane_args = { 0 };
	struct drm_mode_card_res resource_args = { 0 };
	uint32_t first_plane = 0;

	TEST_SUCC(
		ioctl(card_fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &plane_args));
	TEST_RES(0, plane_args.count_planes == plane_resources->count_planes);

	if (plane_args.count_planes > 0) {
		uint32_t total_planes = plane_args.count_planes;

		plane_args.count_planes = 1;
		plane_args.plane_id_ptr = 0;
		TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_GETPLANERESOURCES,
				 &plane_args),
			   EFAULT);

		plane_args.count_planes = 1;
		plane_args.plane_id_ptr = (uintptr_t)&first_plane;
		TEST_SUCC(ioctl(card_fd, DRM_IOCTL_MODE_GETPLANERESOURCES,
				&plane_args));
		TEST_RES(0, plane_args.count_planes == total_planes);
		TEST_RES(0, drm_id_in_array(plane_resources->planes,
					    plane_resources->count_planes,
					    first_plane));
	}

	TEST_SUCC(ioctl(card_fd, DRM_IOCTL_MODE_GETRESOURCES, &resource_args));
	if (resource_args.count_crtcs > 0) {
		resource_args.count_crtcs = 1;
		resource_args.crtc_id_ptr = 0;
		resource_args.count_connectors = 0;
		resource_args.count_encoders = 0;
		resource_args.count_fbs = 0;
		TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_GETRESOURCES,
				 &resource_args),
			   EFAULT);
	}
}
END_TEST()

FN_TEST(resource_limits)
{
	TEST_RES(0, resources->min_width <= resources->max_width);
	TEST_RES(0, resources->min_height <= resources->max_height);
	TEST_RES(0, resources->max_width > 0 && resources->max_height > 0);
}
END_TEST()

FN_TEST(resource_ids_are_global_and_unique)
{
	TEST_RES(0, ids_are_unique(resources->fbs, resources->count_fbs));
	TEST_RES(0, ids_are_unique(resources->crtcs, resources->count_crtcs));
	TEST_RES(0, ids_are_unique(resources->connectors,
				   resources->count_connectors));
	TEST_RES(0, ids_are_unique(resources->encoders,
				   resources->count_encoders));

	TEST_RES(0, id_sets_are_disjoint(resources->fbs, resources->count_fbs,
					 resources->crtcs,
					 resources->count_crtcs));
	TEST_RES(0, id_sets_are_disjoint(resources->fbs, resources->count_fbs,
					 resources->connectors,
					 resources->count_connectors));
	TEST_RES(0, id_sets_are_disjoint(resources->fbs, resources->count_fbs,
					 resources->encoders,
					 resources->count_encoders));
	TEST_RES(0,
		 id_sets_are_disjoint(resources->crtcs, resources->count_crtcs,
				      resources->connectors,
				      resources->count_connectors));
	TEST_RES(0, id_sets_are_disjoint(
			    resources->crtcs, resources->count_crtcs,
			    resources->encoders, resources->count_encoders));
	TEST_RES(0, id_sets_are_disjoint(
			    resources->connectors, resources->count_connectors,
			    resources->encoders, resources->count_encoders));
}
END_TEST()

FN_TEST(crtc_metadata)
{
	for (int i = 0; i < resources->count_crtcs; i++) {
		drmModeCrtcPtr crtc =
			TEST_RES(drmModeGetCrtc(card_fd, resources->crtcs[i]),
				 _ret != NULL);

		if (!crtc) {
			continue;
		}
		TEST_RES(0, crtc->crtc_id == resources->crtcs[i]);
		TEST_RES(0, crtc->mode_valid == 0 || crtc->mode_valid == 1);
		if (crtc->mode_valid) {
			TEST_RES(0, crtc->mode.hdisplay > 0 &&
					    crtc->mode.vdisplay > 0);
			TEST_RES(0, memchr(crtc->mode.name, '\0',
					   sizeof(crtc->mode.name)) != NULL);
		}
		drmModeFreeCrtc(crtc);
	}

	drmModeCrtcPtr invalid =
		TEST_ERRNO(drmModeGetCrtc(card_fd, UINT32_MAX), ENOENT);
	if (invalid) {
		drmModeFreeCrtc(invalid);
	}
}
END_TEST()

FN_TEST(encoder_topology)
{
	uint32_t crtc_mask = index_mask(resources->count_crtcs);
	uint32_t encoder_mask = index_mask(resources->count_encoders);

	for (int i = 0; i < resources->count_encoders; i++) {
		drmModeEncoderPtr encoder = TEST_RES(
			drmModeGetEncoder(card_fd, resources->encoders[i]),
			_ret != NULL);

		if (!encoder) {
			continue;
		}
		TEST_RES(0, encoder->encoder_id == resources->encoders[i]);
		TEST_RES(0, encoder->crtc_id == 0 ||
				    drm_id_in_array(resources->crtcs,
						    resources->count_crtcs,
						    encoder->crtc_id));
		TEST_RES(0, (encoder->possible_crtcs & ~crtc_mask) == 0);
		TEST_RES(0, (encoder->possible_clones & ~encoder_mask) == 0);
		drmModeFreeEncoder(encoder);
	}

	drmModeEncoderPtr invalid =
		TEST_ERRNO(drmModeGetEncoder(card_fd, UINT32_MAX), ENOENT);
	if (invalid) {
		drmModeFreeEncoder(invalid);
	}
}
END_TEST()

FN_TEST(connector_topology_and_modes)
{
	for (int i = 0; i < resources->count_connectors; i++) {
		drmModeConnectorPtr connector = TEST_RES(
			drmModeGetConnector(card_fd, resources->connectors[i]),
			_ret != NULL);

		if (!connector) {
			continue;
		}
		TEST_RES(0,
			 connector->connector_id == resources->connectors[i]);
		TEST_RES(0, connector->connection >= DRM_MODE_CONNECTED &&
				    connector->connection <=
					    DRM_MODE_UNKNOWNCONNECTION);
		TEST_RES(0, connector->encoder_id == 0 ||
				    drm_id_in_array(resources->encoders,
						    resources->count_encoders,
						    connector->encoder_id));

		for (int j = 0; j < connector->count_encoders; j++) {
			TEST_RES(0, drm_id_in_array(resources->encoders,
						    resources->count_encoders,
						    connector->encoders[j]));
		}
		for (int j = 0; j < connector->count_modes; j++) {
			const drmModeModeInfo *mode = &connector->modes[j];

			TEST_RES(0, mode->hdisplay > 0 && mode->vdisplay > 0);
			TEST_RES(0, mode->htotal >= mode->hdisplay &&
					    mode->vtotal >= mode->vdisplay);
			TEST_RES(0, mode->name[0] != '\0' &&
					    memchr(mode->name, '\0',
						   sizeof(mode->name)) != NULL);
		}

		drmModeFreeConnector(connector);
	}

	drmModeConnectorPtr invalid =
		TEST_ERRNO(drmModeGetConnector(card_fd, UINT32_MAX), ENOENT);
	if (invalid) {
		drmModeFreeConnector(invalid);
	}
}
END_TEST()

FN_TEST(plane_formats_and_crtcs)
{
	uint32_t valid_crtcs = index_mask(resources->count_crtcs);

	for (uint32_t i = 0; i < plane_resources->count_planes; i++) {
		TEST_RES(0, plane_resources->planes[i] != 0);
		for (uint32_t j = i + 1; j < plane_resources->count_planes;
		     j++) {
			TEST_RES(0, plane_resources->planes[i] !=
					    plane_resources->planes[j]);
		}

		drmModePlanePtr plane = TEST_RES(
			drmModeGetPlane(card_fd, plane_resources->planes[i]),
			_ret != NULL);
		if (!plane) {
			continue;
		}
		TEST_RES(0, plane->plane_id == plane_resources->planes[i]);
		TEST_RES(0, plane->count_formats > 0);
		TEST_RES(0, plane->gamma_size == 0);
		TEST_RES(0, (plane->possible_crtcs & ~valid_crtcs) == 0);
		TEST_RES(0, resources->count_crtcs == 0 ||
				    plane->possible_crtcs != 0);
		TEST_RES(0, plane->crtc_id == 0 ||
				    drm_id_in_array(resources->crtcs,
						    resources->count_crtcs,
						    plane->crtc_id));

		for (uint32_t j = 0; j < plane->count_formats; j++) {
			TEST_RES(0, plane->formats[j] != 0);
			for (uint32_t k = j + 1; k < plane->count_formats;
			     k++) {
				TEST_RES(0, plane->formats[j] !=
						    plane->formats[k]);
			}
		}

		drmModeFreePlane(plane);
	}

	drmModePlanePtr invalid =
		TEST_ERRNO(drmModeGetPlane(card_fd, UINT32_MAX), ENOENT);
	if (invalid) {
		drmModeFreePlane(invalid);
	}
}
END_TEST()

FN_SETUP(release_kms_resources)
{
	drmModeFreePlaneResources(plane_resources);
	drmModeFreePlaneResources(legacy_plane_resources);
	drmModeFreeResources(resources);
	CHECK(close(card_fd));
}
END_SETUP()
