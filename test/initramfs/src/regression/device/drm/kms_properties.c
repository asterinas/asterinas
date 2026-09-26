// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include "common.h"

struct observed_property {
	char name[DRM_PROP_NAME_LEN];
	uint32_t id;
};

static int card_fd = -1;
static int atomic_enabled;
static uint32_t plane_count;
static uint32_t visible_property_count;
static drmModeResPtr resources;
static struct observed_property *observed_properties;
static size_t observed_property_count;

static const char *const standard_property_names[] = {
	"type",
	"IN_FORMATS",
	"SRC_X",
	"SRC_Y",
	"SRC_W",
	"SRC_H",
	"CRTC_X",
	"CRTC_Y",
	"CRTC_W",
	"CRTC_H",
	"FB_ID",
	"CRTC_ID",
	"ACTIVE",
	"MODE_ID",
	"EDID",
	"PATH",
	"HDR_OUTPUT_METADATA",
	"DPMS",
	"link-status",
	"non-desktop",
	"TILE",
};

static int validate_blob(uint32_t blob_id, uint32_t *length)
{
	drmModePropertyBlobPtr blob = drmModeGetPropertyBlob(card_fd, blob_id);

	if (!blob) {
		return -1;
	}
	if (blob->id != blob_id || (blob->length != 0 && !blob->data)) {
		errno = EPROTO;
		drmModeFreePropertyBlob(blob);
		return -1;
	}

	*length = blob->length;
	drmModeFreePropertyBlob(blob);
	return 0;
}

static int validate_property_value(const drmModePropertyPtr property,
				   uint32_t type, uint64_t value)
{
	if (type == DRM_MODE_PROP_RANGE &&
	    (value < property->values[0] || value > property->values[1])) {
		return 0;
	}
	if (type == DRM_MODE_PROP_SIGNED_RANGE &&
	    ((int64_t)value < (int64_t)property->values[0] ||
	     (int64_t)value > (int64_t)property->values[1])) {
		return 0;
	}
	if (type == DRM_MODE_PROP_ENUM) {
		for (int i = 0; i < property->count_values; i++) {
			if (property->values[i] == value) {
				return 1;
			}
		}
		return 0;
	}
	if (type == DRM_MODE_PROP_OBJECT && value != 0) {
		uint32_t object_type = property->values[0];

		if (value > UINT32_MAX) {
			return 0;
		}
		if (object_type == DRM_MODE_OBJECT_CRTC) {
			return drm_id_in_array(resources->crtcs,
					       resources->count_crtcs,
					       (uint32_t)value);
		}
		if (object_type == DRM_MODE_OBJECT_FB) {
			return drm_id_in_array(resources->fbs,
					       resources->count_fbs,
					       (uint32_t)value);
		}
	}

	return 1;
}

static int is_standard_property_name(const char *name)
{
	for (size_t i = 0; i < sizeof(standard_property_names) /
				       sizeof(standard_property_names[0]);
	     i++) {
		if (strcmp(name, standard_property_names[i]) == 0) {
			return 1;
		}
	}

	return 0;
}

static int remember_standard_property(const char *name, uint32_t id)
{
	struct observed_property *new_properties;

	if (!is_standard_property_name(name)) {
		return 0;
	}
	for (size_t i = 0; i < observed_property_count; i++) {
		if (strcmp(observed_properties[i].name, name) == 0) {
			if (observed_properties[i].id != id) {
				errno = EPROTO;
				return -1;
			}
			return 0;
		}
	}

	new_properties = realloc(observed_properties,
				 (observed_property_count + 1) *
					 sizeof(*observed_properties));
	if (!new_properties) {
		errno = ENOMEM;
		return -1;
	}
	observed_properties = new_properties;
	memcpy(observed_properties[observed_property_count].name, name,
	       DRM_PROP_NAME_LEN);
	observed_properties[observed_property_count].id = id;
	observed_property_count++;
	return 0;
}

static int validate_property(uint32_t property_id, uint64_t value)
{
	drmModePropertyPtr property = drmModeGetProperty(card_fd, property_id);
	uint32_t blob_length;
	uint32_t type;
	int ret = -1;

	if (!property) {
		return -1;
	}
	if (property->prop_id != property_id || property->name[0] == '\0' ||
	    !memchr(property->name, '\0', sizeof(property->name))) {
		errno = EPROTO;
		goto out;
	}

	type = property->flags &
	       (DRM_MODE_PROP_LEGACY_TYPE | DRM_MODE_PROP_EXTENDED_TYPE);
	if (type != 0 && type != DRM_MODE_PROP_RANGE &&
	    type != DRM_MODE_PROP_ENUM && type != DRM_MODE_PROP_BLOB &&
	    type != DRM_MODE_PROP_BITMASK && type != DRM_MODE_PROP_OBJECT &&
	    type != DRM_MODE_PROP_SIGNED_RANGE) {
		errno = EPROTO;
		goto out;
	}
	if ((type == DRM_MODE_PROP_RANGE ||
	     type == DRM_MODE_PROP_SIGNED_RANGE) &&
	    property->count_values != 2) {
		errno = EPROTO;
		goto out;
	}
	if (type == DRM_MODE_PROP_OBJECT && property->count_values != 1) {
		errno = EPROTO;
		goto out;
	}
	if ((type == DRM_MODE_PROP_ENUM || type == DRM_MODE_PROP_BITMASK) &&
	    property->count_values != property->count_enums) {
		errno = EPROTO;
		goto out;
	}
	for (int i = 0; i < property->count_enums; i++) {
		if (!memchr(property->enums[i].name, '\0',
			    sizeof(property->enums[i].name))) {
			errno = EPROTO;
			goto out;
		}
	}
	if (remember_standard_property(property->name, property_id) < 0) {
		goto out;
	}
	if (!validate_property_value(property, type, value)) {
		errno = EPROTO;
		goto out;
	}
	if (type == DRM_MODE_PROP_BLOB && value != 0) {
		if (value > UINT32_MAX ||
		    validate_blob((uint32_t)value, &blob_length) < 0) {
			goto out;
		}
		if (strcmp(property->name, "IN_FORMATS") == 0 &&
		    blob_length == 0) {
			errno = EPROTO;
			goto out;
		}
	}

	ret = 0;
out:
	drmModeFreeProperty(property);
	return ret;
}

static int validate_object(uint32_t object_id, uint32_t object_type)
{
	drmModeObjectPropertiesPtr properties =
		drmModeObjectGetProperties(card_fd, object_id, object_type);
	int ret = -1;

	if (!properties) {
		return -1;
	}
	for (uint32_t i = 0; i < properties->count_props; i++) {
		for (uint32_t j = i + 1; j < properties->count_props; j++) {
			if (properties->props[i] == properties->props[j]) {
				errno = EPROTO;
				goto out;
			}
		}
		if (validate_property(properties->props[i],
				      properties->prop_values[i]) < 0) {
			goto out;
		}
	}
	visible_property_count += properties->count_props;
	ret = 0;
out:
	drmModeFreeObjectProperties(properties);
	return ret;
}

static int validate_planes(void)
{
	drmModePlaneResPtr planes = drmModeGetPlaneResources(card_fd);
	int ret = -1;

	if (!planes) {
		return -1;
	}
	plane_count = planes->count_planes;
	for (uint32_t i = 0; i < planes->count_planes; i++) {
		if (validate_object(planes->planes[i], DRM_MODE_OBJECT_PLANE) <
		    0) {
			goto out;
		}
	}

	ret = 0;
out:
	drmModeFreePlaneResources(planes);
	return ret;
}

FN_SETUP(load_kms_resources)
{
	int ret;

	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
	resources = get_kms_resources_or_skip(card_fd);
	CHECK(drmSetClientCap(card_fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1));
	ret = CHECK_WITH(drmSetClientCap(card_fd, DRM_CLIENT_CAP_ATOMIC, 1),
			 _ret == 0 || errno == EOPNOTSUPP);
	atomic_enabled = ret == 0;
}
END_SETUP()

FN_TEST(object_properties_and_blobs)
{
	for (int i = 0; i < resources->count_crtcs; i++) {
		TEST_SUCC(validate_object(resources->crtcs[i],
					  DRM_MODE_OBJECT_CRTC));
	}
	for (int i = 0; i < resources->count_connectors; i++) {
		TEST_SUCC(validate_object(resources->connectors[i],
					  DRM_MODE_OBJECT_CONNECTOR));
	}
	TEST_SUCC(validate_planes());
	if (atomic_enabled && (resources->count_crtcs ||
			       resources->count_connectors || plane_count)) {
		TEST_RES(0, visible_property_count > 0);
	}
}
END_TEST()

FN_TEST(invalid_property_objects)
{
	drmModeObjectPropertiesPtr object =
		TEST_ERRNO(drmModeObjectGetProperties(card_fd, UINT32_MAX,
						      DRM_MODE_OBJECT_CRTC),
			   ENOENT);
	if (object) {
		drmModeFreeObjectProperties(object);
	}

	drmModePropertyPtr property =
		TEST_ERRNO(drmModeGetProperty(card_fd, UINT32_MAX), ENOENT);
	if (property) {
		drmModeFreeProperty(property);
	}

	drmModePropertyBlobPtr blob =
		TEST_ERRNO(drmModeGetPropertyBlob(card_fd, UINT32_MAX), ENOENT);
	if (blob) {
		drmModeFreePropertyBlob(blob);
	}

	if (resources->count_crtcs > 0) {
		object = TEST_ERRNO(
			drmModeObjectGetProperties(card_fd, resources->crtcs[0],
						   DRM_MODE_OBJECT_CONNECTOR),
			EINVAL);
		if (object) {
			drmModeFreeObjectProperties(object);
		}
	}
}
END_TEST()

FN_SETUP(release_kms_resources)
{
	free(observed_properties);
	drmModeFreeResources(resources);
	CHECK(close(card_fd));
}
END_SETUP()
