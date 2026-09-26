// SPDX-License-Identifier: MPL-2.0

#include <sys/mman.h>
#include <xf86drmMode.h>

#include "../../common/test.h"
#include "common.h"

enum {
	DUMB_WIDTH = 13,
	DUMB_HEIGHT = 257,
	DUMB_BPP = 15,
};

static int card_fd = -1;

static int validate_created_buffer(const struct drm_mode_create_dumb *create,
				   size_t page_size)
{
	uint64_t min_pitch = ((uint64_t)create->width * create->bpp + 7) / 8;
	uint64_t min_size = (uint64_t)create->pitch * create->height;

	return create->handle != 0 && create->pitch >= min_pitch &&
			       create->size >= min_size &&
			       create->size > page_size &&
			       create->size <= SIZE_MAX ?
		       0 :
		       -1;
}

static int validate_mmap_offset(const struct drm_mode_map_dumb *map,
				size_t page_size)
{
	return map->offset != 0 && map->offset % page_size == 0 ? 0 : -1;
}

static int create_and_map_dumb(struct drm_mode_create_dumb *create,
			       struct drm_mode_map_dumb *map, size_t page_size)
{
	uint64_t size;
	int create_ret = drmModeCreateDumbBuffer(card_fd, create->width,
						 create->height, create->bpp,
						 create->flags, &create->handle,
						 &create->pitch, &size);
	if (create_ret != 0) {
		return -1;
	}
	create->size = size;

	if (validate_created_buffer(create, page_size) != 0) {
		goto destroy;
	}

	map->handle = create->handle;
	uint64_t offset;
	int map_ret = drmModeMapDumbBuffer(card_fd, map->handle, &offset);
	if (map_ret != 0) {
		goto destroy;
	}
	map->offset = offset;

	if (validate_mmap_offset(map, page_size) != 0) {
		goto destroy;
	}

	return 0;

destroy:
	drmModeDestroyDumbBuffer(card_fd, create->handle);
	return -1;
}

static int verify_zeroed_buffer(const uint8_t *buffer, size_t size)
{
	for (size_t i = 0; i < size; i++) {
		if (buffer[i] != 0) {
			return -1;
		}
	}

	return 0;
}

static int write_and_verify_pattern(uint8_t *buffer, size_t size)
{
	for (size_t i = 0; i < size; i++) {
		buffer[i] = (uint8_t)(i * 37 + 11);
	}

	for (size_t i = 0; i < size; i++) {
		if (buffer[i] != (uint8_t)(i * 37 + 11)) {
			return -1;
		}
	}

	return 0;
}

FN_SETUP(open_card)
{
	card_fd = open_drm_node_or_skip(DRM_CARD_DEVICE);
}
END_SETUP()

FN_TEST(create_map_destroy)
{
	long page_size = TEST_RES(sysconf(_SC_PAGESIZE), _ret > 0);

	struct drm_mode_create_dumb create = {
		.width = DUMB_WIDTH,
		.height = DUMB_HEIGHT,
		.bpp = DUMB_BPP,
	};
	struct drm_mode_map_dumb map = { 0 };
	TEST_RES(create_and_map_dumb(&create, &map, (size_t)page_size),
		 _ret == 0);

	size_t map_size = (size_t)create.size;
	uint8_t *buffer = TEST_RES(mmap(NULL, map_size, PROT_READ | PROT_WRITE,
					MAP_SHARED, card_fd, (off_t)map.offset),
				   _ret != MAP_FAILED);
	TEST_RES(verify_zeroed_buffer(buffer, map_size), _ret == 0);
	TEST_RES(write_and_verify_pattern(buffer, map_size), _ret == 0);
	TEST_RES(munmap(buffer, map_size), _ret == 0);
	TEST_RES(drmModeDestroyDumbBuffer(card_fd, create.handle), _ret == 0);
}
END_TEST()

FN_TEST(validate_mapping_and_handle_errors)
{
	long page_size = TEST_RES(sysconf(_SC_PAGESIZE), _ret > 0);

	struct drm_mode_create_dumb create = {
		.width = DUMB_WIDTH,
		.height = DUMB_HEIGHT,
		.bpp = DUMB_BPP,
	};
	struct drm_mode_map_dumb map = { 0 };
	TEST_RES(create_and_map_dumb(&create, &map, (size_t)page_size),
		 _ret == 0);

	size_t map_size = (size_t)create.size;
	size_t inner_size = map_size - (size_t)page_size;
	uint8_t *inner = TEST_RES(
		mmap(NULL, inner_size, PROT_READ | PROT_WRITE, MAP_SHARED,
		     card_fd, (off_t)(map.offset + (uint64_t)page_size)),
		_ret != MAP_FAILED);
	TEST_RES(verify_zeroed_buffer(inner, inner_size), _ret == 0);
	TEST_RES(munmap(inner, inner_size), _ret == 0);

	TEST_ERRNO(mmap(NULL, map_size + (size_t)page_size,
			PROT_READ | PROT_WRITE, MAP_SHARED, card_fd,
			(off_t)map.offset),
		   EINVAL);

	TEST_ERRNO(mmap(NULL, map_size, PROT_READ | PROT_WRITE, MAP_PRIVATE,
			card_fd, (off_t)map.offset),
		   EINVAL);

	int other_fd = TEST_RES(open(DRM_CARD_DEVICE, O_RDWR), _ret >= 0);
	TEST_ERRNO(mmap(NULL, map_size, PROT_READ | PROT_WRITE, MAP_SHARED,
			other_fd, (off_t)map.offset),
		   EACCES);
	TEST_RES(close(other_fd), _ret == 0);

	struct drm_gem_close invalid_close = {
		.handle = UINT32_MAX,
	};
	/*
	 * Keep invalid requests on the generic ioctl path so the exact test
	 * payload reaches the kernel without typed-wrapper preprocessing.
	 */
	TEST_ERRNO(drmIoctl(card_fd, DRM_IOCTL_GEM_CLOSE, &invalid_close),
		   EINVAL);

	struct drm_mode_destroy_dumb invalid_destroy = {
		.handle = UINT32_MAX,
	};
	TEST_ERRNO(drmIoctl(card_fd, DRM_IOCTL_MODE_DESTROY_DUMB,
			    &invalid_destroy),
		   EINVAL);

	TEST_RES(drmModeDestroyDumbBuffer(card_fd, create.handle), _ret == 0);
	TEST_ERRNO(mmap(NULL, map_size, PROT_READ | PROT_WRITE, MAP_SHARED,
			card_fd, (off_t)map.offset),
		   EINVAL);
}
END_TEST()

FN_TEST(reject_pitch_overflow)
{
	/* Invalid uAPI payloads intentionally use drmIoctl; see above. */
	struct drm_mode_create_dumb create = {
		.width = UINT32_MAX,
		.height = 1,
		.bpp = UINT32_MAX,
	};
	TEST_ERRNO(drmIoctl(card_fd, DRM_IOCTL_MODE_CREATE_DUMB, &create),
		   EINVAL);
}
END_TEST()

FN_TEST(reject_size_overflow)
{
	/* Invalid uAPI payloads intentionally use drmIoctl; see above. */
	struct drm_mode_create_dumb create = {
		.width = 0x10000,
		.height = 0x10000,
		.bpp = 32,
	};
	TEST_ERRNO(drmIoctl(card_fd, DRM_IOCTL_MODE_CREATE_DUMB, &create),
		   EINVAL);
}
END_TEST()

FN_SETUP(close_card)
{
	CHECK(close(card_fd));
}
END_SETUP()
