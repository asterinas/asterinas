// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <linux/kd.h>
#include <linux/vt.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include "../../common/test.h"

#define VT_DENYREL 0
#define VT_ALLOWREL 1

#define VT1_NUM 61
#define VT2_NUM 62

#define VT1_PATH "/dev/tty61"
#define VT2_PATH "/dev/tty62"

#define SLEEP_TIME_US 1000 * 1000

static int vt1_fd;

static const struct vt_mode auto_mode = {
	.mode = VT_AUTO,
	.waitv = 0,
	.relsig = 0,
	.acqsig = 0,
	.frsig = 0,
};
static const struct vt_mode process_mode = {
	.mode = VT_PROCESS,
	.waitv = 0,
	.relsig = SIGUSR1,
	.acqsig = SIGUSR2,
	.frsig = 0,
};

static volatile int sigusr1_received;
static volatile int sigusr2_received;

static void sigusr1_handler(int sig, siginfo_t *info, void *ctx)
{
	(void)sig;
	(void)info;
	(void)ctx;
	sigusr1_received = 1;
}

static void sigusr2_handler(int sig, siginfo_t *info, void *ctx)
{
	(void)sig;
	(void)info;
	(void)ctx;
	sigusr2_received = 1;
}

static int get_active_vt(int fd)
{
	struct vt_stat state;

	if (ioctl(fd, VT_GETSTATE, &state) < 0)
		return -1;

	return state.v_active;
}

// Retry VT_WAITACTIVE on EINTR to avoid signal interruptions breaking tests.
static int vt_waitactive(int fd, int vt)
{
	int ret;

	do {
		ret = ioctl(fd, VT_WAITACTIVE, vt);
	} while (ret == -1 && errno == EINTR);

	return ret;
}

FN_SETUP(setup_signals)
{
	struct sigaction sa;

	memset(&sa, 0, sizeof(sa));
	sa.sa_sigaction = sigusr1_handler;
	sa.sa_flags = SA_SIGINFO;
	sigemptyset(&sa.sa_mask);
	CHECK(sigaction(SIGUSR1, &sa, NULL));

	memset(&sa, 0, sizeof(sa));
	sa.sa_sigaction = sigusr2_handler;
	sa.sa_flags = SA_SIGINFO;
	sigemptyset(&sa.sa_mask);
	CHECK(sigaction(SIGUSR2, &sa, NULL));
}
END_SETUP()

FN_SETUP(open_vt1)
{
	vt1_fd = CHECK(open(VT1_PATH, O_RDWR));
}
END_SETUP()

FN_TEST(vt_index_validation)
{
	// Test VT_ACTIVATE rejects invalid VT indices
	TEST_ERRNO(ioctl(vt1_fd, VT_ACTIVATE, 0), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_ACTIVATE, 64), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_ACTIVATE, -1), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_ACTIVATE, 257), ENXIO);

	// Test VT_WAITACTIVE rejects invalid VT indices
	TEST_ERRNO(ioctl(vt1_fd, VT_WAITACTIVE, 0), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_WAITACTIVE, 64), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_WAITACTIVE, -1), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_WAITACTIVE, 257), ENXIO);

	// Test VT_DISALLOCATE rejects invalid VT indices
	TEST_ERRNO(ioctl(vt1_fd, VT_DISALLOCATE, 64), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_DISALLOCATE, -1), ENXIO);
	TEST_ERRNO(ioctl(vt1_fd, VT_DISALLOCATE, 257), ENXIO);
}
END_TEST()

FN_TEST(kd_and_keyboard_mode_validation)
{
	char kb_type;
	int kb_mode, gfx_mode;

	// Test keyboard type query
	TEST_RES(ioctl(vt1_fd, KDGKBTYPE, &kb_type), kb_type == KB_101);

	// Test keyboard mode get/set with valid and invalid values
	TEST_SUCC(ioctl(vt1_fd, KDGKBMODE, &kb_mode));
	TEST_SUCC(ioctl(vt1_fd, KDSKBMODE, kb_mode));
	TEST_ERRNO(ioctl(vt1_fd, KDSKBMODE, 0x7fffffff), EINVAL);

	// Test graphics mode get/set with valid and invalid values
	TEST_SUCC(ioctl(vt1_fd, KDGETMODE, &gfx_mode));
	TEST_SUCC(ioctl(vt1_fd, KDSETMODE, gfx_mode));
	TEST_ERRNO(ioctl(vt1_fd, KDSETMODE, 0x7fffffff), EINVAL);
}
END_TEST()

FN_TEST(vt_setmode_edge_cases)
{
	struct vt_mode old_mode, invalid_mode, process_mode, new_mode;

	TEST_SUCC(ioctl(vt1_fd, VT_GETMODE, &old_mode));

	// Test invalid mode value is rejected
	invalid_mode = old_mode;
	invalid_mode.mode = 0x7f;
	TEST_ERRNO(ioctl(vt1_fd, VT_SETMODE, &invalid_mode), EINVAL);

	// Test invalid `relsig` is accepted
	invalid_mode = old_mode;
	invalid_mode.relsig = 256;
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &invalid_mode));

	// Test invalid `acqsig` is accepted
	invalid_mode = old_mode;
	invalid_mode.acqsig = 256;
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &invalid_mode));

	// Test multiple invalid fields are clamped to valid ranges
	invalid_mode = old_mode;
	invalid_mode.waitv = 2;
	invalid_mode.relsig = 256;
	invalid_mode.acqsig = 256;
	invalid_mode.frsig = 256;
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &invalid_mode));
	TEST_SUCC(ioctl(vt1_fd, VT_GETMODE, &new_mode));

	// Test forced-release signal is exposed as `0`
	TEST_RES(new_mode.frsig, _ret == 0);

	// FIXME: Asterinas converts `struct vt_mode` fields when applying VT_SETMODE:
	// - `waitv` is treated as a boolean; any non-zero value is exposed as `1`.
	// - Out-of-range `relsig`/`acqsig` are treated as no signal.
#ifdef __asterinas__
	TEST_RES(new_mode.waitv, _ret == 1);
	TEST_RES(new_mode.relsig, _ret == 0);
	TEST_RES(new_mode.acqsig, _ret == 0);
#else
	TEST_RES(new_mode.waitv, _ret == 2);
	TEST_RES(new_mode.relsig, _ret == 256);
	TEST_RES(new_mode.acqsig, _ret == 256);
#endif

	// Test switching to VT_PROCESS mode with valid signals
	process_mode = old_mode;
	process_mode.mode = VT_PROCESS;
	process_mode.relsig = SIGUSR1;
	process_mode.acqsig = SIGUSR2;
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &process_mode));

	// Verify VT_PROCESS mode signals are properly set
	TEST_SUCC(ioctl(vt1_fd, VT_GETMODE, &new_mode));
	TEST_RES(new_mode.mode, _ret == VT_PROCESS);
	TEST_RES(new_mode.relsig, _ret == SIGUSR1);
	TEST_RES(new_mode.acqsig, _ret == SIGUSR2);

	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &old_mode));
}
END_TEST()

FN_TEST(vt_openqry_reflects_opened_vt)
{
	int next_vt, next_vt_fd, next_vt_after_open, next_vt_after_close;
	char next_vt_path[32];

	// Get the next available VT
	TEST_SUCC(ioctl(vt1_fd, VT_OPENQRY, &next_vt));
	SKIP_TEST_IF(next_vt == -1);
	TEST_RES(next_vt, _ret >= 1 && _ret <= 63);

	// Open the VT and verify the next available VT changes
	snprintf(next_vt_path, sizeof(next_vt_path), "/dev/tty%d", next_vt);
	next_vt_fd = TEST_SUCC(open(next_vt_path, O_RDWR));
	TEST_SUCC(ioctl(vt1_fd, VT_OPENQRY, &next_vt_after_open));
	TEST_RES(next_vt_after_open, _ret == -1 || _ret != next_vt);

	// Close the VT and verify it becomes available again
	TEST_SUCC(close(next_vt_fd));
	TEST_SUCC(ioctl(vt1_fd, VT_OPENQRY, &next_vt_after_close));
	TEST_RES(next_vt_after_close, _ret == next_vt);
}
END_TEST()

FN_TEST(vt_reldisp_without_pending_switch)
{
	struct vt_mode old_mode;

	TEST_SUCC(ioctl(vt1_fd, VT_GETMODE, &old_mode));

	// In VT_AUTO mode, VT_ACKACQ should fail without pending switch
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &auto_mode));
	TEST_ERRNO(ioctl(vt1_fd, VT_RELDISP, VT_ACKACQ), EINVAL);

	// In VT_PROCESS mode, VT_ACKACQ succeeds but
	// VT_DENYREL/VT_ALLOWREL fail without pending switch
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &process_mode));
	TEST_SUCC(ioctl(vt1_fd, VT_RELDISP, VT_ACKACQ));
	TEST_ERRNO(ioctl(vt1_fd, VT_RELDISP, VT_DENYREL), EINVAL);
	TEST_ERRNO(ioctl(vt1_fd, VT_RELDISP, VT_ALLOWREL), EINVAL);

	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &old_mode));
}
END_TEST()

FN_TEST(vt_reldisp_ackacq_completes_switch)
{
	struct vt_mode old_mode;
	int vt2_fd;

	TEST_SUCC(ioctl(vt1_fd, VT_GETMODE, &old_mode));
	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));

	// Activate VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	// Set VT1 to process mode
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &process_mode));

	// Request a VT2 switch
	sigusr1_received = 0;
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	// Sleep to allow signal handler to run
	usleep(SLEEP_TIME_US);
	TEST_RES(sigusr1_received, _ret == 1);

	// Test VT_ACKACQ completes the switch immediately
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);
	TEST_SUCC(ioctl(vt1_fd, VT_RELDISP, VT_ACKACQ));
	TEST_SUCC(vt_waitactive(vt1_fd, VT2_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT2_NUM);

	// VT_ACKACQ without a pending switch is a no-op
	TEST_SUCC(ioctl(vt1_fd, VT_RELDISP, VT_ACKACQ));

	// Switch back to VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &old_mode));
	TEST_SUCC(close(vt2_fd));
}
END_TEST()

FN_TEST(vt_reldisp_ackacq_on_inactive_process_vt)
{
	struct vt_mode old_mode;
	int vt2_fd;

	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));

	// Activate VT1 so VT2 is inactive
	TEST_SUCC(ioctl(vt2_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt2_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt2_fd), _ret == VT1_NUM);

	// Set VT2 to process mode while inactive
	TEST_SUCC(ioctl(vt2_fd, VT_GETMODE, &old_mode));
	TEST_SUCC(ioctl(vt2_fd, VT_SETMODE, &process_mode));

	// Test VT_ACKACQ on inactive VT succeeds, but VT_ALLOWREL fails
	TEST_SUCC(ioctl(vt2_fd, VT_RELDISP, VT_ACKACQ));
	TEST_ERRNO(ioctl(vt2_fd, VT_RELDISP, VT_ALLOWREL), EINVAL);

	TEST_SUCC(ioctl(vt2_fd, VT_SETMODE, &old_mode));
	TEST_SUCC(close(vt2_fd));
}
END_TEST()

FN_TEST(vt_reldisp_nonstandard_value_treated_as_allow_release)
{
	struct vt_mode old_mode;
	int vt2_fd;

	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));
	TEST_SUCC(ioctl(vt1_fd, VT_GETMODE, &old_mode));

	// Activate VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	// Set VT1 to process mode
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &process_mode));

	// Request a VT2 switch
	sigusr1_received = 0;
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	// Sleep to allow signal handler to run
	usleep(SLEEP_TIME_US);
	TEST_RES(sigusr1_received, _ret == 1);
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	// Test a non-standard VT_RELDISP value is treated as VT_ALLOWREL
	TEST_SUCC(ioctl(vt1_fd, VT_RELDISP, 123));
	TEST_SUCC(vt_waitactive(vt1_fd, VT2_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT2_NUM);

	// Switch back to VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &old_mode));
	TEST_SUCC(close(vt2_fd));
}
END_TEST()

FN_TEST(vt_switch_in_process_mode)
{
	struct vt_mode old_mode;
	int vt2_fd;

	TEST_SUCC(ioctl(vt1_fd, VT_GETMODE, &old_mode));
	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));

	// Activate VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	// Set VT1 to process mode
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &process_mode));

	// Request a VT2 switch
	sigusr1_received = 0;
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	// Sleep to allow signal handler to run
	usleep(SLEEP_TIME_US);
	TEST_RES(sigusr1_received, _ret == 1);

	// Test denying the switch
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);
	TEST_SUCC(ioctl(vt1_fd, VT_RELDISP, VT_DENYREL));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);
	TEST_ERRNO(ioctl(vt1_fd, VT_RELDISP, VT_ALLOWREL), EINVAL);

	// Request a VT2 switch
	sigusr1_received = 0;
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	// Sleep to allow signal handler to run
	usleep(SLEEP_TIME_US);
	TEST_RES(sigusr1_received, _ret == 1);
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	// Test allowing the switch
	TEST_SUCC(ioctl(vt1_fd, VT_RELDISP, VT_ALLOWREL));
	TEST_SUCC(vt_waitactive(vt1_fd, VT2_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT2_NUM);
	TEST_ERRNO(ioctl(vt1_fd, VT_RELDISP, VT_ALLOWREL), EINVAL);

	// Switch back to VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	TEST_SUCC(close(vt2_fd));
	TEST_SUCC(ioctl(vt1_fd, VT_SETMODE, &old_mode));
}
END_TEST()

FN_TEST(vt_activate_ignores_switch_rejection)
{
	int vt2_fd;
	int old_mode;

	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));

	// Activate VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	// Set KD_GRAPHICS mode which can reject VT switches
	TEST_SUCC(ioctl(vt1_fd, KDGETMODE, &old_mode));
	TEST_SUCC(ioctl(vt1_fd, KDSETMODE, KD_GRAPHICS));

	// Test VT_ACTIVATE succeeds even though the switch may be rejected
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	TEST_SUCC(ioctl(vt1_fd, KDSETMODE, old_mode));
	TEST_SUCC(close(vt2_fd));
}
END_TEST()

FN_TEST(vt_cannot_disallocate_while_open)
{
	int vt2_fd;

	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));
	TEST_SUCC(close(vt2_fd));

	// Test VT2 cannot be deallocated while active
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT2_NUM));
	TEST_ERRNO(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM), EBUSY);

	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));

	// Test VT2 cannot be deallocated while open
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_ERRNO(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM), EBUSY);

	// Test closing the file allows deallocation
	TEST_SUCC(close(vt2_fd));
	// Sleep to ensure kernel has processed the close
	usleep(SLEEP_TIME_US);
	TEST_SUCC(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM));

	// Test reopening resets open file tracking
	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT2_NUM));
	TEST_ERRNO(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM), EBUSY);

	// Test closing the file allows deallocation
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_SUCC(close(vt2_fd));
	// Sleep to ensure kernel has processed the close
	usleep(SLEEP_TIME_US);
	TEST_SUCC(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM));
}
END_TEST()

FN_TEST(vt_disallocate_resets_vt_state)
{
	struct vt_mode mode;
	int gfx_mode;
	int kb_mode;
	int vt2_fd;

	// Activate VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));

	// Set VT2 to process mode
	mode = process_mode;
	mode.waitv = 1;
	TEST_SUCC(ioctl(vt2_fd, VT_SETMODE, &mode));
	TEST_SUCC(ioctl(vt2_fd, VT_GETMODE, &mode));
	TEST_RES(mode.mode, _ret == VT_PROCESS);
	TEST_RES(mode.waitv, _ret == 1);
	TEST_RES(mode.relsig, _ret == SIGUSR1);
	TEST_RES(mode.acqsig, _ret == SIGUSR2);

	// Move VT2 away from its default graphics and keyboard modes
	TEST_SUCC(ioctl(vt2_fd, KDSETMODE, KD_GRAPHICS));
	TEST_SUCC(ioctl(vt2_fd, KDGETMODE, &gfx_mode));
	TEST_RES(gfx_mode, _ret == KD_GRAPHICS);
	TEST_SUCC(ioctl(vt2_fd, KDSKBMODE, K_XLATE));
	TEST_SUCC(ioctl(vt2_fd, KDGKBMODE, &kb_mode));
	TEST_RES(kb_mode, _ret == K_XLATE);

	// Deallocate VT2
	TEST_SUCC(close(vt2_fd));
	// Sleep to ensure kernel has processed the close
	usleep(SLEEP_TIME_US);
	TEST_SUCC(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM));

	// Reopen and verify mode behavior matches deallocation result
	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));
	TEST_SUCC(ioctl(vt2_fd, VT_GETMODE, &mode));
	TEST_RES(mode.mode, _ret == VT_AUTO);
	TEST_RES(mode.waitv, _ret == 0);
	TEST_RES(mode.relsig, _ret == 0);
	TEST_RES(mode.acqsig, _ret == 0);
	TEST_RES(mode.frsig, _ret == 0);
	TEST_SUCC(ioctl(vt2_fd, KDGETMODE, &gfx_mode));
	TEST_RES(gfx_mode, _ret == KD_TEXT);
	TEST_SUCC(ioctl(vt2_fd, KDGKBMODE, &kb_mode));
	TEST_RES(kb_mode, _ret == K_UNICODE);

	TEST_SUCC(close(vt2_fd));
}
END_TEST()

FN_TEST(tty0_keeps_vt_busy_across_switches)
{
	int vt2_fd, tty0_fd;

	vt2_fd = TEST_SUCC(open(VT2_PATH, O_RDWR));

	// Activate VT2 and open `/dev/tty0` which should point to VT2
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT2_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT2_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT2_NUM);
	tty0_fd = TEST_SUCC(open("/dev/tty0", O_RDWR));

	// Switch to VT1
	TEST_SUCC(ioctl(vt1_fd, VT_ACTIVATE, VT1_NUM));
	TEST_SUCC(vt_waitactive(vt1_fd, VT1_NUM));
	TEST_RES(get_active_vt(vt1_fd), _ret == VT1_NUM);

	// Test VT2 is busy due to `tty0_fd` reference
	TEST_SUCC(close(vt2_fd));
	// Sleep to ensure kernel has processed the close
	usleep(SLEEP_TIME_US);
	TEST_ERRNO(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM), EBUSY);

	// Close `tty0_fd` and verify VT2 can now be deallocated
	TEST_SUCC(close(tty0_fd));
	// Sleep to ensure kernel has processed the close
	usleep(SLEEP_TIME_US);
	TEST_SUCC(ioctl(vt1_fd, VT_DISALLOCATE, VT2_NUM));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(vt1_fd));
}
END_SETUP()
