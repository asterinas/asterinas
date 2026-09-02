// SPDX-License-Identifier: MPL-2.0

#include "../../common/capability.h"
#include "common.h"

#include <sys/wait.h>

static int get_drm_magic(int fd, uint32_t *magic)
{
	struct drm_auth auth = { 0 };
	int ret = ioctl(fd, DRM_IOCTL_GET_MAGIC, &auth);

	*magic = auth.magic;
	return ret;
}

static int auth_drm_magic(int fd, uint32_t magic)
{
	struct drm_auth auth = {
		.magic = magic,
	};

	return ioctl(fd, DRM_IOCTL_AUTH_MAGIC, &auth);
}

FN_SETUP(check_primary_node_available)
{
	int fd = open_drm_node_or_skip(DRM_CARD_DEVICE);

	CHECK(close(fd));
}
END_SETUP()

FN_TEST(master_ownership)
{
	int first_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int second_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));

	TEST_SUCC(ioctl(first_fd, DRM_IOCTL_SET_MASTER));
	TEST_ERRNO(ioctl(second_fd, DRM_IOCTL_SET_MASTER), EBUSY);
	TEST_ERRNO(ioctl(second_fd, DRM_IOCTL_DROP_MASTER), EINVAL);

	TEST_SUCC(ioctl(first_fd, DRM_IOCTL_DROP_MASTER));
	TEST_ERRNO(ioctl(first_fd, DRM_IOCTL_DROP_MASTER), EINVAL);
	TEST_SUCC(ioctl(second_fd, DRM_IOCTL_SET_MASTER));
	TEST_ERRNO(ioctl(first_fd, DRM_IOCTL_SET_MASTER), EBUSY);
	TEST_SUCC(ioctl(second_fd, DRM_IOCTL_DROP_MASTER));
	TEST_SUCC(ioctl(first_fd, DRM_IOCTL_SET_MASTER));

	TEST_SUCC(close(second_fd));
	TEST_SUCC(close(first_fd));
}
END_TEST()

FN_TEST(magic_authentication)
{
	int master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int client_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	uint32_t first_magic = 0;
	uint32_t second_magic = 0;

	TEST_SUCC(get_drm_magic(client_fd, &first_magic));
	TEST_SUCC(get_drm_magic(client_fd, &second_magic));
	TEST_RES(first_magic, first_magic != 0 && first_magic == second_magic);

	TEST_ERRNO(auth_drm_magic(client_fd, first_magic), EACCES);
	TEST_ERRNO(auth_drm_magic(master_fd, 0), EINVAL);
	TEST_SUCC(auth_drm_magic(master_fd, first_magic));
	TEST_ERRNO(auth_drm_magic(master_fd, first_magic), EINVAL);

	TEST_SUCC(close(client_fd));
	TEST_SUCC(close(master_fd));
}
END_TEST()

FN_TEST(magic_ids_are_unique_per_client)
{
	int master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int first_client_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int second_client_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	uint32_t first_magic = 0;
	uint32_t second_magic = 0;

	TEST_SUCC(get_drm_magic(first_client_fd, &first_magic));
	TEST_SUCC(get_drm_magic(second_client_fd, &second_magic));
	TEST_RES(first_magic, first_magic != 0 && first_magic != second_magic);
	TEST_RES(second_magic, second_magic != 0);

	TEST_SUCC(auth_drm_magic(master_fd, first_magic));
	TEST_SUCC(auth_drm_magic(master_fd, second_magic));

	TEST_SUCC(close(second_client_fd));
	TEST_SUCC(close(first_client_fd));
	TEST_SUCC(close(master_fd));
}
END_TEST()

FN_TEST(pending_magic_removed_on_close)
{
	int master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int client_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	uint32_t magic = 0;

	TEST_SUCC(get_drm_magic(client_fd, &magic));
	TEST_SUCC(close(client_fd));
	TEST_ERRNO(auth_drm_magic(master_fd, magic), EINVAL);

	TEST_SUCC(close(master_fd));
}
END_TEST()

FN_TEST(master_context_survives_reacquisition)
{
	int master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int client_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	uint32_t magic = 0;

	TEST_SUCC(get_drm_magic(client_fd, &magic));
	TEST_SUCC(ioctl(master_fd, DRM_IOCTL_DROP_MASTER));
	TEST_SUCC(ioctl(master_fd, DRM_IOCTL_SET_MASTER));
	TEST_SUCC(auth_drm_magic(master_fd, magic));

	TEST_SUCC(close(client_fd));
	TEST_SUCC(close(master_fd));
}
END_TEST()

FN_TEST(first_time_master_replaces_inherited_context)
{
	int old_master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int new_master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	uint32_t old_magic = 0;

	TEST_SUCC(get_drm_magic(new_master_fd, &old_magic));
	TEST_SUCC(ioctl(old_master_fd, DRM_IOCTL_DROP_MASTER));
	TEST_SUCC(ioctl(new_master_fd, DRM_IOCTL_SET_MASTER));
	TEST_ERRNO(auth_drm_magic(new_master_fd, old_magic), EINVAL);

	/* Closing the new master must not release old_magic in its new context. */
	TEST_SUCC(close(new_master_fd));
	TEST_SUCC(ioctl(old_master_fd, DRM_IOCTL_SET_MASTER));
	TEST_SUCC(close(old_master_fd));
}
END_TEST()

FN_TEST(master_context_is_isolated_during_handoff)
{
	int old_master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int client_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	int new_master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	uint32_t old_magic = 0;

	TEST_SUCC(get_drm_magic(client_fd, &old_magic));
	TEST_SUCC(ioctl(old_master_fd, DRM_IOCTL_DROP_MASTER));
	TEST_SUCC(ioctl(new_master_fd, DRM_IOCTL_SET_MASTER));

	TEST_ERRNO(auth_drm_magic(old_master_fd, old_magic), EACCES);
	TEST_ERRNO(auth_drm_magic(new_master_fd, old_magic), EINVAL);

	TEST_SUCC(ioctl(new_master_fd, DRM_IOCTL_DROP_MASTER));
	TEST_SUCC(ioctl(old_master_fd, DRM_IOCTL_SET_MASTER));
	TEST_SUCC(auth_drm_magic(old_master_fd, old_magic));

	TEST_SUCC(close(new_master_fd));
	TEST_SUCC(close(client_fd));
	TEST_SUCC(close(old_master_fd));
}
END_TEST()

FN_TEST(master_control_requires_owner_or_sys_admin)
{
	int master_fd = TEST_SUCC(open(DRM_CARD_DEVICE, O_RDWR));
	pid_t child = TEST_SUCC(fork());
	int status;

	if (child == 0) {
		drop_capability(CAP_SYS_ADMIN);

		errno = 0;
		CHECK_WITH(ioctl(master_fd, DRM_IOCTL_SET_MASTER),
			   _ret == -1 && errno == EACCES);
		errno = 0;
		CHECK_WITH(ioctl(master_fd, DRM_IOCTL_DROP_MASTER),
			   _ret == -1 && errno == EACCES);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(child, &status, 0),
		 _ret == child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_SUCC(ioctl(master_fd, DRM_IOCTL_DROP_MASTER));
	TEST_SUCC(close(master_fd));
}
END_TEST()

FN_TEST(master_owner_can_control_without_sys_admin)
{
	pid_t child = TEST_SUCC(fork());
	int status;

	if (child == 0) {
		int master_fd = CHECK(open(DRM_CARD_DEVICE, O_RDWR));

		drop_capability(CAP_SYS_ADMIN);
		CHECK(ioctl(master_fd, DRM_IOCTL_DROP_MASTER));
		CHECK(ioctl(master_fd, DRM_IOCTL_SET_MASTER));
		CHECK(close(master_fd));
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(child, &status, 0),
		 _ret == child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
