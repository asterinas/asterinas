// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include "common.h"

static int card_fd = -1;
static drmModeResPtr resources;
static drmModePlaneResPtr planes;

enum {
	DRM_TEST_PLANE_TYPE_CURSOR = 2,
};

static uint32_t property_type(uint32_t flags)
{
	return flags &
	       (DRM_MODE_PROP_LEGACY_TYPE | DRM_MODE_PROP_EXTENDED_TYPE);
}

static int has_property_test_objects(void)
{
	return resources->count_crtcs > 0 && planes->count_planes > 0 ? 0 : -1;
}

static int validate_in_formats_blob(uint64_t value)
{
	if (value == 0 || value > UINT32_MAX) {
		errno = EPROTO;
		return -1;
	}

	drmModePropertyBlobPtr blob =
		drmModeGetPropertyBlob(card_fd, (uint32_t)value);
	if (!blob) {
		return -1;
	}

	int ret = 0;
	if (blob->id != value ||
	    blob->length < sizeof(struct drm_format_modifier_blob) ||
	    !blob->data) {
		errno = EPROTO;
		ret = -1;
	}
	drmModeFreePropertyBlob(blob);
	return ret;
}

static int validate_plane_properties(uint32_t plane_id)
{
	drmModeObjectPropertiesPtr properties = drmModeObjectGetProperties(
		card_fd, plane_id, DRM_MODE_OBJECT_PLANE);
	if (!properties) {
		return -1;
	}

	int found_type = 0;
	int found_in_formats = 0;
	int ret = -1;

	if (properties->count_props != 2) {
		errno = EPROTO;
		goto out;
	}

	for (uint32_t i = 0; i < properties->count_props; i++) {
		drmModePropertyPtr property =
			drmModeGetProperty(card_fd, properties->props[i]);
		if (!property) {
			goto out;
		}

		if (strcmp(property->name, "type") == 0) {
			if (found_type ||
			    property_type(property->flags) !=
				    DRM_MODE_PROP_ENUM ||
			    property->count_values != 3 ||
			    property->count_enums != 3 ||
			    properties->prop_values[i] >
				    DRM_TEST_PLANE_TYPE_CURSOR) {
				errno = EPROTO;
				drmModeFreeProperty(property);
				goto out;
			}
			found_type = 1;
		} else if (strcmp(property->name, "IN_FORMATS") == 0) {
			if (found_in_formats ||
			    property_type(property->flags) !=
				    DRM_MODE_PROP_BLOB ||
			    validate_in_formats_blob(
				    properties->prop_values[i]) < 0) {
				drmModeFreeProperty(property);
				goto out;
			}
			found_in_formats = 1;
		} else {
			errno = EPROTO;
			drmModeFreeProperty(property);
			goto out;
		}

		drmModeFreeProperty(property);
	}

	if (!found_type || !found_in_formats) {
		errno = EPROTO;
		goto out;
	}

	ret = 0;
out:
	drmModeFreeObjectProperties(properties);
	return ret;
}

FN_SETUP(load_kms_resources)
{
	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
	resources = get_kms_resources_or_skip(card_fd);
	CHECK(drmSetClientCap(card_fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1));
	planes = CHECK_WITH(drmModeGetPlaneResources(card_fd), _ret != NULL);
	CHECK_WITH(has_property_test_objects(), _ret == 0);
}
END_SETUP()

FN_TEST(plane_properties_and_blob)
{
	TEST_RES(validate_plane_properties(planes->planes[0]), _ret == 0);
}
END_TEST()

FN_TEST(invalid_property_objects)
{
	TEST_ERRNO(drmModeObjectGetProperties(card_fd, UINT32_MAX,
					      DRM_MODE_OBJECT_PLANE),
		   ENOENT);
	TEST_ERRNO(drmModeObjectGetProperties(card_fd, resources->crtcs[0],
					      DRM_MODE_OBJECT_CONNECTOR),
		   ENOENT);
	TEST_ERRNO(drmModeGetProperty(card_fd, UINT32_MAX), ENOENT);
	TEST_ERRNO(drmModeGetPropertyBlob(card_fd, UINT32_MAX), ENOENT);
}
END_TEST()

FN_SETUP(release_kms_resources)
{
	drmModeFreePlaneResources(planes);
	drmModeFreeResources(resources);
	CHECK(close(card_fd));
}
END_SETUP()
