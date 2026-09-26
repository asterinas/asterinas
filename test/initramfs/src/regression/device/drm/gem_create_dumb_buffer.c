// SPDX-License-Identifier: MPL-2.0

#include <sys/mman.h>

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
	uint64_t min_size = min_pitch * create->height;
	uint64_t expected_size =
		(min_size + page_size - 1) / page_size * page_size;

	return create->handle != 0 && create->pitch == min_pitch &&
			       create->size == expected_size &&
			       create->size <= SIZE_MAX ?
		       0 :
		       -1;
}

static int validate_mmap_offset(const struct drm_mode_map_dumb *map,
				size_t page_size)
{
	return map->offset != 0 && map->offset % page_size == 0 ? 0 : -1;
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
	if (page_size <= 0) {
		goto out;
	}

	struct drm_mode_create_dumb create = {
		.width = DUMB_WIDTH,
		.height = DUMB_HEIGHT,
		.bpp = DUMB_BPP,
	};
	struct drm_mode_map_dumb map = { 0 };
	size_t map_size = 0;
	int create_ret = TEST_RES(
		ioctl(card_fd, DRM_IOCTL_MODE_CREATE_DUMB, &create), _ret == 0);
	if (create_ret != 0) {
		goto out;
	}
	int validate_ret = TEST_RES(
		validate_created_buffer(&create, (size_t)page_size), _ret == 0);
	if (validate_ret != 0) {
		goto destroy;
	}

	map.handle = create.handle;
	int map_ret = TEST_RES(ioctl(card_fd, DRM_IOCTL_MODE_MAP_DUMB, &map),
			       _ret == 0);
	if (map_ret != 0) {
		goto destroy;
	}
	int validate_map_ret = TEST_RES(
		validate_mmap_offset(&map, (size_t)page_size), _ret == 0);
	if (validate_map_ret != 0) {
		goto destroy;
	}

	map_size = (size_t)create.size;
	uint8_t *buffer = TEST_RES(mmap(NULL, map_size, PROT_READ | PROT_WRITE,
					MAP_SHARED, card_fd, (off_t)map.offset),
				   _ret != MAP_FAILED);
	if (buffer == MAP_FAILED) {
		goto destroy;
	}
	TEST_RES(verify_zeroed_buffer(buffer, map_size), _ret == 0);
	TEST_RES(write_and_verify_pattern(buffer, map_size), _ret == 0);
	TEST_RES(munmap(buffer, map_size), _ret == 0);

destroy:;
	struct drm_mode_destroy_dumb destroy = {
		.handle = create.handle,
	};
	TEST_RES(ioctl(card_fd, DRM_IOCTL_MODE_DESTROY_DUMB, &destroy),
		 _ret == 0);

out:;
}
END_TEST()

FN_TEST(validate_mapping_and_handle_errors)
{
	long page_size = TEST_RES(sysconf(_SC_PAGESIZE), _ret > 0);
	if (page_size <= 0) {
		goto out;
	}

	struct drm_mode_create_dumb create = {
		.width = DUMB_WIDTH,
		.height = DUMB_HEIGHT,
		.bpp = DUMB_BPP,
	};
	struct drm_mode_map_dumb map = { 0 };
	size_t map_size = 0;
	int create_ret = TEST_RES(
		ioctl(card_fd, DRM_IOCTL_MODE_CREATE_DUMB, &create), _ret == 0);
	if (create_ret != 0) {
		goto out;
	}
	int validate_ret = TEST_RES(
		validate_created_buffer(&create, (size_t)page_size), _ret == 0);
	if (validate_ret != 0) {
		goto destroy;
	}

	map.handle = create.handle;
	int map_ret = TEST_RES(ioctl(card_fd, DRM_IOCTL_MODE_MAP_DUMB, &map),
			       _ret == 0);
	if (map_ret != 0) {
		goto destroy;
	}
	int validate_map_ret = TEST_RES(
		validate_mmap_offset(&map, (size_t)page_size), _ret == 0);
	if (validate_map_ret != 0) {
		goto destroy;
	}

	map_size = (size_t)create.size;
	TEST_RES(map_size, _ret > (size_t)page_size);
	if (map_size <= (size_t)page_size) {
		goto destroy;
	}

	size_t inner_size = map_size - (size_t)page_size;
	uint8_t *inner = TEST_RES(
		mmap(NULL, inner_size, PROT_READ | PROT_WRITE, MAP_SHARED,
		     card_fd, (off_t)(map.offset + (uint64_t)page_size)),
		_ret != MAP_FAILED);
	if (inner != MAP_FAILED) {
		TEST_RES(verify_zeroed_buffer(inner, inner_size), _ret == 0);
		TEST_RES(munmap(inner, inner_size), _ret == 0);
	}

	void *oversized = TEST_ERRNO(mmap(NULL, map_size + (size_t)page_size,
					  PROT_READ | PROT_WRITE, MAP_SHARED,
					  card_fd, (off_t)map.offset),
				     EINVAL);
	if (oversized != MAP_FAILED) {
		TEST_RES(munmap(oversized, map_size + (size_t)page_size),
			 _ret == 0);
	}

	void *private =
		TEST_ERRNO(mmap(NULL, map_size, PROT_READ | PROT_WRITE,
				MAP_PRIVATE, card_fd, (off_t)map.offset),
			   EINVAL);
	if (private != MAP_FAILED) {
		TEST_RES(munmap(private, map_size), _ret == 0);
	}

	int other_fd = TEST_RES(open(DRM_CARD_DEVICE, O_RDWR), _ret >= 0);
	if (other_fd >= 0) {
		void *foreign = TEST_ERRNO(
			mmap(NULL, map_size, PROT_READ | PROT_WRITE, MAP_SHARED,
			     other_fd, (off_t)map.offset),
			EACCES);
		if (foreign != MAP_FAILED) {
			TEST_RES(munmap(foreign, map_size), _ret == 0);
		}
		TEST_RES(close(other_fd), _ret == 0);
	}

	struct drm_gem_close invalid_close = {
		.handle = UINT32_MAX,
	};
	TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_GEM_CLOSE, &invalid_close), EINVAL);

	struct drm_mode_destroy_dumb invalid_destroy = {
		.handle = UINT32_MAX,
	};
	TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_DESTROY_DUMB,
			 &invalid_destroy),
		   EINVAL);

destroy:;
	struct drm_mode_destroy_dumb destroy = {
		.handle = create.handle,
	};
	int destroy_ret =
		TEST_RES(ioctl(card_fd, DRM_IOCTL_MODE_DESTROY_DUMB, &destroy),
			 _ret == 0);
	if (destroy_ret == 0 && map.offset != 0) {
		void *destroyed =
			TEST_ERRNO(mmap(NULL, map_size, PROT_READ | PROT_WRITE,
					MAP_SHARED, card_fd, (off_t)map.offset),
				   EINVAL);
		if (destroyed != MAP_FAILED) {
			TEST_RES(munmap(destroyed, map_size), _ret == 0);
		}
	}

out:;
}
END_TEST()

FN_TEST(reject_oversized_buffer)
{
	struct drm_mode_create_dumb create = {
		.width = 4096,
		.height = 1U << 20,
		.bpp = 32,
	};
	TEST_ERRNO(ioctl(card_fd, DRM_IOCTL_MODE_CREATE_DUMB, &create), EINVAL);
}
END_TEST()

FN_SETUP(close_card)
{
	CHECK(close(card_fd));
}
END_SETUP()
